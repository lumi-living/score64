//! What to listen for, worked out from a score: when its ties, key-offs, arpeggios, tempo changes and loop come.

use std::fmt::Write as _;

use score64_core::{Attack, Subtune};

use super::render::PAL_HZ;

/// Most events listed per subtune; past that only the ones worth checking by ear are kept.
const MAX_EVENTS: usize = 32;

/// One thing that happens on a row.
struct Event {
    row: u32,
    voice: Option<u8>,
    what: String,
    /// A tie, key-off, arpeggio, tempo change or loop: always listed.
    check: bool,
    /// It happens after the jump back, not on the first pass.
    after_loop: bool,
}

fn event(row: u32, voice: Option<u8>, what: String, check: bool) -> Event {
    Event {
        row,
        voice,
        what,
        check,
        after_loop: false,
    }
}

/// Seconds from row 0 to the start of `row` on the first pass.
fn at(subtune: &Subtune, row: u32) -> f64 {
    subtune.frames(0, row) as f64 / PAL_HZ
}

/// The events of `subtune`'s first pass in row order, then what happens after the jump back.
fn events(subtune: &Subtune) -> Vec<Event> {
    let length = subtune.length.get();
    let mut out = Vec::new();
    for line in &subtune.voices {
        let v = Some(line.voice.get());
        let mut notes: Vec<_> = line.notes.iter().collect();
        notes.sort_by_key(|n| n.row);
        for (k, n) in notes.iter().enumerate() {
            out.push(match n.attack {
                Attack::Fresh => event(n.row, v, n.pitch.to_string(), false),
                Attack::Legato => event(
                    n.row,
                    v,
                    format!("**tie** into {}: pitch changes, no new attack", n.pitch),
                    true,
                ),
            });
            let end = n.row + n.length.get();
            let next = notes.get(k + 1).map(|m| m.row);
            if end < length && next != Some(end) {
                out.push(event(
                    end,
                    v,
                    "**key-off**: a clean release (the instrument's gate timer closes the gate a few frames early)"
                        .into(),
                    true,
                ));
            } else if end == length && !notes.iter().any(|m| m.row == subtune.loop_row) {
                out.push(Event {
                    after_loop: true,
                    ..event(
                        subtune.loop_row,
                        v,
                        "**key-off** on the loop row, so the last note doesn't sound on after the jump back".into(),
                        true,
                    )
                });
            }
        }
        for a in &line.arps {
            let steps: Vec<String> = a.intervals.iter().map(u8::to_string).collect();
            out.push(event(
                a.row,
                v,
                format!(
                    "**arpeggio** starts (+{} semitones, one step a frame): even, no stutter",
                    steps.join("/")
                ),
                true,
            ));
            let end = a.row + a.length.get();
            let what = if end == length {
                "**arpeggio** ends with the song: the loop row sets what plays next".to_string()
            } else {
                "**arpeggio** ends: a clean stop".to_string()
            };
            out.push(event(end, v, what, true));
        }
    }
    for t in &subtune.tempo {
        out.push(event(
            t.row,
            None,
            format!(
                "**tempo** changes to {} frames a row",
                t.frames_per_row.get()
            ),
            true,
        ));
    }
    out.push(event(
        length,
        None,
        format!(
            "**loop** back to row {}: no click, gap, stuck note or tempo jump",
            subtune.loop_row
        ),
        true,
    ));
    out.retain(|e| e.row <= length);
    out.sort_by_key(|e| (e.after_loop, e.row, e.voice));
    out
}

/// A Markdown timeline for `subtune`.
pub fn timeline(subtune: &Subtune) -> String {
    let length = subtune.length.get();
    let pass = at(subtune, length);
    let period = subtune.frames(subtune.loop_row, length) as f64 / PAL_HZ;
    let mut s = String::new();
    let _ = writeln!(
        s,
        "{} rows at {} frames a row{}. The first pass lasts **{pass:.2} s**; the song then loops to row {}, every **{period:.2} s**. \
         Times are from row 0, which sounds a few frames into the file (up to 0.15 s).\n",
        length,
        subtune.frames_per_row.get(),
        if subtune.tempo.is_empty() {
            ""
        } else {
            ", with tempo changes"
        },
        subtune.loop_row
    );
    let all = events(subtune);
    let shown: Vec<&Event> = if all.len() > MAX_EVENTS {
        all.iter().filter(|e| e.check).take(MAX_EVENTS).collect()
    } else {
        all.iter().collect()
    };
    let _ = writeln!(s, "| Time | Row | Voice | What |\n|---|---|---|---|");
    for e in &shown {
        let voice = e.voice.map_or("all".to_string(), |v| v.to_string());
        let time = if e.after_loop {
            "after the loop".to_string()
        } else {
            format!("{:.2} s", at(subtune, e.row))
        };
        let _ = writeln!(s, "| {time} | {} | {voice} | {} |", e.row, e.what);
    }
    if shown.len() < all.len() {
        let _ = writeln!(
            s,
            "\n({} more events, plain notes or later ones, aren't listed.)",
            all.len() - shown.len()
        );
    }
    s
}

#[cfg(test)]
mod tests {
    use std::num::{NonZeroU8, NonZeroU32};

    use score64_core::{
        Confidence, FramesPerRow, Note, PresetId, Role, Roles, SubtuneName, Voice, VoiceLine,
    };

    use super::*;

    fn note(row: u32, length: u32, attack: Attack) -> Note {
        Note {
            row,
            length: NonZeroU32::new(length).unwrap(),
            pitch: "C-4".parse().unwrap(),
            attack,
            instrument: None,
            source: None,
            confidence: Confidence::new(1.0).unwrap(),
            decision: None,
        }
    }

    #[test]
    fn ties_key_offs_and_the_loop_are_listed_with_their_times() {
        let s = Subtune {
            name: SubtuneName::new("m").unwrap(),
            profile: None,
            frames_per_row: FramesPerRow::new(5).unwrap(),
            tempo: vec![],
            rows_per_bar: NonZeroU8::new(16).unwrap(),
            length: NonZeroU32::new(32).unwrap(),
            loop_row: 16,
            marks: vec![],
            voices: vec![VoiceLine {
                voice: Voice::new(1).unwrap(),
                roles: Roles::constant(Role::Lead),
                instrument: PresetId("p".into()),
                notes: vec![
                    note(20, 4, Attack::Fresh),
                    note(24, 2, Attack::Legato),
                    note(28, 4, Attack::Fresh),
                ],
                arps: vec![],
            }],
        };
        let t = timeline(&s);
        assert!(t.contains("| 2.00 s | 20 | 1 | C-4 |"), "{t}");
        assert!(t.contains("| 24 | 1 | **tie**"), "{t}");
        assert!(t.contains("| 26 | 1 | **key-off**"), "{t}");
        assert!(
            t.contains("| after the loop | 16 | 1 | **key-off** on the loop row"),
            "the last note runs to the end: {t}"
        );
        assert!(
            t.contains("| 3.19 s | 32 | all | **loop** back to row 16"),
            "{t}"
        );
        assert!(t.contains("every **1.60 s**"), "{t}");
    }

    fn subtune(notes: Vec<Note>, loop_row: u32) -> Subtune {
        Subtune {
            name: SubtuneName::new("m").unwrap(),
            profile: None,
            frames_per_row: FramesPerRow::new(5).unwrap(),
            tempo: vec![],
            rows_per_bar: NonZeroU8::new(16).unwrap(),
            length: NonZeroU32::new(32).unwrap(),
            loop_row,
            marks: vec![],
            voices: vec![VoiceLine {
                voice: Voice::new(1).unwrap(),
                roles: Roles::constant(Role::Lead),
                instrument: PresetId("p".into()),
                notes,
                arps: vec![],
            }],
        }
    }

    #[test]
    fn back_to_back_notes_have_no_key_off_and_a_note_on_the_loop_row_needs_none() {
        // Notes straight into each other, the last running to the end, with a note on the loop row.
        let s = subtune(
            vec![
                note(0, 8, Attack::Fresh),
                note(8, 8, Attack::Fresh),
                note(16, 16, Attack::Fresh),
            ],
            0,
        );
        let t = timeline(&s);
        assert!(!t.contains("key-off"), "{t}");
        // The same with a silent loop row: the loop-row key-off appears.
        let s = subtune(vec![note(8, 24, Attack::Fresh)], 0);
        let t = timeline(&s);
        assert!(
            t.contains("| after the loop | 0 | 1 | **key-off** on the loop row"),
            "{t}"
        );
    }

    #[test]
    fn tempo_changes_and_arpeggios_are_timed_through_the_tempo_map() {
        let mut s = subtune(vec![note(0, 32, Attack::Fresh)], 0);
        s.tempo = vec![score64_core::TempoChange {
            row: 8,
            frames_per_row: FramesPerRow::new(10).unwrap(),
        }];
        s.voices[0].arps = vec![
            score64_core::ArpEvent {
                row: 12,
                length: NonZeroU32::new(4).unwrap(),
                intervals: vec![0, 4, 7],
                decision: None,
            },
            score64_core::ArpEvent {
                row: 24,
                length: NonZeroU32::new(8).unwrap(),
                intervals: vec![0, 3],
                decision: None,
            },
        ];
        let t = timeline(&s);
        // Rows 0-7 at 5 frames (40), then 10 frames a row: row 12 is 80 frames in, row 16 is 120.
        let secs = |frames: f64| format!("{:.2} s", frames / PAL_HZ);
        assert!(
            t.contains(&format!(
                "| {} | 8 | all | **tempo** changes to 10",
                secs(40.0)
            )),
            "{t}"
        );
        assert!(
            t.contains(&format!(
                "| {} | 12 | 1 | **arpeggio** starts (+0/4/7",
                secs(80.0)
            )),
            "{t}"
        );
        assert!(
            t.contains(&format!(
                "| {} | 16 | 1 | **arpeggio** ends: a clean stop",
                secs(120.0)
            )),
            "{t}"
        );
        assert!(
            t.contains("| 32 | 1 | **arpeggio** ends with the song"),
            "{t}"
        );
    }
}
