//! Built-in listening cases: the fixture piece "Sixteen Steps" (`fixtures/_tools/composition.py`) as a score, with the
//! fixture's instruments (`fixtures/_tools/make_song.py`), and small scores that exercise one export behaviour each.

use std::collections::BTreeMap;
use std::num::{NonZeroU8, NonZeroU32};

use score64_core::profile::ProfileName;
use score64_core::{
    AdaptiveSet, ArpEvent, Attack, Confidence, FramesPerRow, Note, Pitch, PresetId, Role, Roles,
    Schema, Song, Subtune, SubtuneName, Switch, TempoChange, Voice, VoiceLine,
};
use score64_formats::gt2::export::Library;
use score64_formats::gt2::{Ins, Instrument, Table};

/// A score to listen to, with what is special about it.
pub struct Case {
    /// File-name stem.
    pub name: &'static str,
    /// The score.
    pub song: Song,
    /// Adaptive sets to export it with (timing marks).
    pub adaptive: Vec<AdaptiveSet>,
    /// What to listen for in this case, in Markdown, beyond what the timeline shows.
    pub notes: &'static str,
}

/// Every built-in case's name.
pub const NAMES: [&str; 6] = [
    "sixteen-steps",
    "sixteen-steps-adaptive",
    "combat-filtered",
    "ties-keyoffs-arp",
    "loop-tempo",
    "bass-arps",
];

/// The built-in case called `name`.
pub fn case(name: &str) -> Option<Case> {
    Some(match name {
        "sixteen-steps" => Case {
            name: "sixteen-steps",
            song: sixteen_steps(),
            adaptive: vec![],
            notes: "The fixture piece, written as a score and exported. `main`: lead (pulse sweep, delayed vibrato), \
                chords as arpeggios over root, third, fifth and octave, and a filtered bass. `combat`: a quieter lead, \
                a plucked bass with a key-off after every note, and drums on voice 3, the voice built for sound \
                effects. **`combat` is unfiltered here:** the fixture's own song filters voices 1 and 2 with pattern \
                command `AXY`, which the score can't express yet (#187), so on the 6581 its effects are less \
                prominent than the original's. `combat-filtered` has the filter. `main` isn't meant for effects (voice \
                3 is the filtered bass), so its `+fx` files only check that nothing breaks.",
        },
        "sixteen-steps-adaptive" => {
            let mut song = sixteen_steps();
            let set = AdaptiveSet::with_switch(
                "layers",
                vec![name_of("main"), name_of("combat")],
                Switch::EveryBar,
            )
            .expect("two layers");
            set.place_marks(&mut song);
            Case {
                name: "sixteen-steps-adaptive",
                song,
                adaptive: vec![set],
                notes: "`sixteen-steps` with `main` and `combat` as an adaptive set switching every bar, so a timing \
                    mark (`DXY`, `$10`/`$11`) is written on every bar line and the song is packed with `-H1`. **It \
                    should sound exactly like `sixteen-steps`:** a mark writes the timing-mark byte, not the master \
                    volume. A volume dip or pump on bar lines means the marks are read as volume commands.",
            }
        }
        "combat-filtered" => {
            let mut song = sixteen_steps();
            let mut combat = song.subtunes.remove(1);
            combat.voices[0].instrument = preset("lead-combat-filtered");
            Case {
                name: "combat-filtered",
                song: score(vec![combat]),
                adaptive: vec![],
                notes: "`combat` with the fixture's filter on voices 1 and 2 (low-pass, resonance 2, cutoff `$38`), \
                    carried by the lead instrument's own filter table. **A/B against `sixteen-steps-combat` on the \
                    6581, with effects:** here the effects should stand out as in the original fixture song.",
            }
        }
        "ties-keyoffs-arp" => Case {
            name: "ties-keyoffs-arp",
            song: ties_keyoffs_arp(),
            adaptive: vec![],
            notes: "A short score, 5 frames a row (GoatTracker's default is 6, so a missing tempo command shows). \
                Voice 1 ties C-4 into E-4, gets a key-off, then plays G-4. Voice 2 holds C-3 under a major arpeggio. \
                Voice 3 is free, so the `+fx` files put the effects over unchanged music.",
        },
        "loop-tempo" => Case {
            name: "loop-tempo",
            song: loop_tempo(),
            adaptive: vec![],
            notes: "Loops to a **silent** row 16, with a faster tempo from row 24. The last note runs to the end and \
                **must stop at the jump back**, from the key-off the export writes on the loop row. Every pass after \
                the first must keep the loop row's slower tempo, because the export sets it there. **Known:** the \
                gaps are not fully silent. The oscillator keeps running after a key-off and leaks a faint tone at \
                the note's pitch (#186).",
        },
        "bass-arps" => Case {
            name: "bass-arps",
            song: bass_arps(),
            adaptive: vec![],
            notes: "`sixteen-steps`' `main` with **no arp voice**: its chords go on the bass instead, through \
                `reduce::arpeggiate_on_bass` (#164). The bass (voice 3) keeps its notes on beats 1 and 3 and plays \
                the bar's chord as an arpeggio on beats 2 and 4, rooted near the bass note before it. **The voicing \
                matches `sixteen-steps`':** each arpeggio plays the intervals of the arp voice's chord over it \
                (root, third, fifth and octave), not the rule's plain triad, so the A/B changes only the voice. \
                **Listen for:** the harmony still coming through with one voice fewer; the bass line still clear \
                on the strong beats; no clicks or dropouts where the arpeggio starts and stops. A/B against \
                `sixteen-steps-main`, which has the chords on a voice of their own.",
        },
        _ => return None,
    })
}

fn preset(s: &str) -> PresetId {
    PresetId(s.into())
}

fn name_of(s: &str) -> SubtuneName {
    SubtuneName::new(s).expect("a valid subtune name")
}

/// A wavetable absolute note for MIDI pitch `midi` (`$80` + semitones from C-0).
fn abs(midi: u8) -> u8 {
    0x80 + midi - 12
}

fn table(rows: &[(u8, u8)]) -> Table {
    Table {
        rows: rows.to_vec(),
    }
}

/// An instrument with the fixture's defaults: gate timer 2, first frame `$09`, and its own tables.
fn ins(
    name: &str,
    ad: u8,
    sr: u8,
    wave: &[(u8, u8)],
    pulse: &[(u8, u8)],
    filter: &[(u8, u8)],
    vibrato: Option<(u8, u8, u8)>,
) -> Ins {
    let mut i = Instrument::from_bytes(&[0; 25]);
    i.ad = ad;
    i.sr = sr;
    i.wave = 1;
    i.pulse = u8::from(!pulse.is_empty());
    i.filter = u8::from(!filter.is_empty());
    i.gate_timer = 2;
    i.first_wave = 0x09;
    let mut speed = Table::default();
    if let Some((left, right, delay)) = vibrato {
        speed.rows.push((left, right));
        i.vibrato = 1;
        i.vibrato_delay = delay;
    }
    i.set_name(name);
    Ins {
        instrument: i,
        wave: table(wave),
        pulse: table(pulse),
        filter: table(filter),
        speed,
    }
}

/// The fixture's instruments by preset id. The chord voice is a plain sawtooth: the export writes the arpeggio
/// programs itself.
pub fn library() -> Library {
    let lead = |name, ad, sr, pulse: &[(u8, u8)], filter: &[(u8, u8)]| {
        ins(
            name,
            ad,
            sr,
            &[(0x41, 0), (0xFF, 0)],
            pulse,
            filter,
            Some((0x03, 0x18, 0x18)),
        )
    };
    let combat_pulse = [(0x82, 0x80), (0x10, 0x18), (0x10, 0xE8), (0xFF, 2)];
    BTreeMap::from([
        (
            preset("lead"),
            lead(
                "Lead PWM",
                0x09,
                0x8A,
                &[(0x84, 0x00), (0x20, 0x30), (0x20, 0xD0), (0xFF, 2)],
                &[],
            ),
        ),
        (
            preset("lead-combat"),
            lead("Lead combat", 0x06, 0x5A, &combat_pulse, &[]),
        ),
        // The fixture's combat filter program (make_song.py's `bass_filter`, started there by pattern command A):
        // low-pass, resonance 2, voices 1 and 2, cutoff $38. Here the lead carries it in its own filter table.
        (
            preset("lead-combat-filtered"),
            lead(
                "Lead combat filt",
                0x06,
                0x5A,
                &combat_pulse,
                &[(0x90, 0x23), (0x00, 0x38), (0xFF, 0x00)],
            ),
        ),
        (
            preset("arp"),
            ins(
                "Arp saw",
                0x06,
                0x8B,
                &[(0x21, 0), (0xFF, 0)],
                &[],
                &[],
                None,
            ),
        ),
        (
            preset("bass-filtered"),
            ins(
                "Bass filtered",
                0x05,
                0xC9,
                &[(0x41, 0), (0xFF, 0)],
                &[(0x86, 0), (0xFF, 0)],
                &[
                    (0x90, 0x84),
                    (0x00, 0x28),
                    (0x30, 0x01),
                    (0x30, 0xFF),
                    (0xFF, 3),
                ],
                None,
            ),
        ),
        (
            preset("bass-pluck"),
            ins(
                "Bass pluck",
                0x0A,
                0x00,
                &[(0x41, 0), (0xFF, 0)],
                &[(0x83, 0), (0xFF, 0)],
                &[],
                None,
            ),
        ),
        (
            preset("kick"),
            ins(
                "Kick",
                0x00,
                0xF8,
                &[
                    (0x81, abs(76)),
                    (0x41, abs(52)),
                    (0x41, abs(48)),
                    (0x41, abs(44)),
                    (0x40, abs(40)),
                    (0xFF, 0),
                ],
                &[(0x88, 0), (0xFF, 0)],
                &[],
                None,
            ),
        ),
        (
            preset("snare"),
            ins(
                "Snare",
                0x00,
                0xF8,
                &[
                    (0x81, abs(88)),
                    (0x41, abs(54)),
                    (0x41, abs(48)),
                    (0x80, abs(88)),
                    (0x80, abs(85)),
                    (0xFF, 0),
                ],
                &[(0x88, 0), (0xFF, 0)],
                &[],
                None,
            ),
        ),
        (
            preset("hat"),
            ins(
                "Hat",
                0x00,
                0x84,
                &[(0x81, abs(100)), (0x80, abs(100)), (0xFF, 0)],
                &[],
                &[],
                None,
            ),
        ),
        (
            preset("pulse"),
            ins(
                "Test pulse",
                0x09,
                0xA0,
                &[(0x41, 0), (0xFF, 0)],
                &[(0x88, 0), (0xFF, 0)],
                &[],
                None,
            ),
        ),
    ])
}

fn note(row: u32, length: u32, midi: u8, attack: Attack, instrument: Option<&str>) -> Note {
    Note {
        row,
        length: NonZeroU32::new(length).expect("non-zero"),
        pitch: Pitch::from_midi(midi).expect("a playable pitch"),
        attack,
        instrument: instrument.map(preset),
        source: None,
        confidence: Confidence::new(1.0).expect("in range"),
        decision: None,
    }
}

fn line(voice: u8, instrument: &str, notes: Vec<Note>) -> VoiceLine {
    VoiceLine {
        voice: Voice::new(voice).expect("1-3"),
        roles: Roles::constant(Role::Lead),
        instrument: preset(instrument),
        notes,
        arps: vec![],
    }
}

fn subtune(name: &str, frames_per_row: u8, length: u32, voices: Vec<VoiceLine>) -> Subtune {
    Subtune {
        name: name_of(name),
        profile: None,
        frames_per_row: FramesPerRow::new(frames_per_row).expect("3-127"),
        tempo: vec![],
        rows_per_bar: NonZeroU8::new(16).expect("non-zero"),
        length: NonZeroU32::new(length).expect("non-zero"),
        loop_row: 0,
        marks: vec![],
        voices,
    }
}

fn score(subtunes: Vec<Subtune>) -> Song {
    Song {
        schema: Schema::default(),
        profile: ProfileName::new("full").expect("built in"),
        subtunes,
    }
}

// "Sixteen Steps" (fixtures/_tools/composition.py): (bar, row, length, MIDI pitch).
#[rustfmt::skip]
const MELODY: [(u32, u32, u32, u8); 28] = [
    (0, 0, 4, 69), (0, 4, 2, 72), (0, 6, 6, 76), (0, 12, 4, 74),
    (1, 0, 4, 72), (1, 4, 4, 69), (1, 8, 4, 65), (1, 12, 4, 69),
    (2, 0, 4, 67), (2, 4, 2, 72), (2, 6, 6, 76), (2, 12, 4, 79),
    (3, 0, 4, 77), (3, 4, 2, 76), (3, 6, 2, 74), (3, 8, 8, 71),
    (4, 0, 4, 69), (4, 4, 2, 72), (4, 6, 6, 76), (4, 12, 4, 81),
    (5, 0, 4, 79), (5, 4, 4, 77), (5, 8, 4, 76), (5, 12, 4, 72),
    (6, 0, 8, 71), (6, 8, 4, 68), (6, 12, 4, 71),
    (7, 0, 12, 76),
];
/// One chord per bar: root and whether it's major. Am F C G Am F E E.
const CHORDS: [(u8, bool); 8] = [
    (57, false),
    (53, true),
    (60, true),
    (55, true),
    (57, false),
    (53, true),
    (52, true),
    (52, true),
];
/// Bass: two half notes per bar.
const BASS: [(u8, u8); 8] = [
    (45, 45),
    (41, 41),
    (48, 48),
    (43, 43),
    (45, 45),
    (41, 41),
    (40, 40),
    (40, 44),
];
/// The drum loop, one bar: (row, instrument).
const DRUMS: [(u32, &str); 8] = [
    (0, "kick"),
    (2, "hat"),
    (4, "snare"),
    (6, "hat"),
    (8, "kick"),
    (10, "kick"),
    (12, "snare"),
    (14, "hat"),
];

fn sixteen_steps() -> Song {
    let melody = |instrument| {
        let notes = MELODY
            .iter()
            .map(|&(bar, row, len, midi)| note(bar * 16 + row, len, midi, Attack::Fresh, None))
            .collect();
        line(1, instrument, notes)
    };
    let mut chords = line(
        2,
        "arp",
        (0..8)
            .map(|bar| note(bar * 16, 16, CHORDS[bar as usize].0, Attack::Fresh, None))
            .collect(),
    );
    chords.arps = (0..8u32)
        .map(|bar| ArpEvent {
            row: bar * 16,
            length: NonZeroU32::new(16).expect("non-zero"),
            intervals: vec![0, if CHORDS[bar as usize].1 { 4 } else { 3 }, 7, 12],
            decision: None,
        })
        .collect();
    let bass = line(
        3,
        "bass-filtered",
        (0..8u32)
            .flat_map(|bar| {
                let (a, b) = BASS[bar as usize];
                [
                    note(bar * 16, 8, a, Attack::Fresh, None),
                    note(bar * 16 + 8, 8, b, Attack::Fresh, None),
                ]
            })
            .collect(),
    );
    let pluck = line(
        2,
        "bass-pluck",
        (0..8u32)
            .flat_map(|bar| {
                let root = CHORDS[bar as usize].0 - 12;
                [(0, 0), (4, 12), (8, 0), (12, 7)]
                    .map(|(row, up)| note(bar * 16 + row, 2, root + up, Attack::Fresh, None))
            })
            .collect(),
    );
    let drums = line(
        3,
        "kick",
        (0..8u32)
            .flat_map(|bar| {
                DRUMS.map(|(row, d)| note(bar * 16 + row, 1, 60, Attack::Fresh, Some(d)))
            })
            .collect(),
    );
    score(vec![
        subtune("main", 6, 128, vec![melody("lead"), chords, bass]),
        subtune("combat", 6, 128, vec![melody("lead-combat"), pluck, drums]),
    ])
}

/// `sixteen-steps`' `main` without its arp voice, the chords put on the bass's weak beats, voiced as that arp voice
/// voices them.
fn bass_arps() -> Song {
    use score64_core::reduce::{
        BassArpRules, Chord, ChordQuality, ChordSpan, Placement, arpeggiate_on_bass,
    };
    let mut song = sixteen_steps();
    song.subtunes.truncate(1);
    let main = &mut song.subtunes[0];
    // The arp voice's own arpeggios, for their voicing.
    let voicing = main
        .voices
        .iter()
        .find(|v| v.voice.get() == 2)
        .expect("the arp voice")
        .arps
        .clone();
    main.voices.retain(|v| v.voice.get() != 2);
    let spans: Vec<ChordSpan> = (0..8u32)
        .map(|bar| {
            let (root, major) = CHORDS[bar as usize];
            ChordSpan {
                row: bar * 16,
                length: NonZeroU32::new(16).expect("non-zero"),
                chord: Chord {
                    root: root % 12,
                    quality: if major {
                        ChordQuality::Major
                    } else {
                        ChordQuality::Minor
                    },
                },
                confidence: Confidence::new(1.0).expect("in range"),
            }
        })
        .collect();
    let bass = main
        .voices
        .iter_mut()
        .find(|v| v.voice.get() == 3)
        .expect("the bass");
    let at = Placement {
        subtune: &main.name,
        voice: bass.voice,
        rows_per_bar: main.rows_per_bar,
    };
    let line = arpeggiate_on_bass(
        bass.notes.clone(),
        &spans,
        &at,
        &BassArpRules::default(),
        score64_core::validate::RoleRanges::default().bass,
    );
    bass.notes = line.notes;
    // The rule voices each chord as its quality's triad; give each arpeggio the arp voice's intervals over the same
    // chord instead (root, third, fifth and octave), so an A/B against `sixteen-steps` changes only the voice. The
    // rule's `Chord` decisions, which record the triad, aren't part of the score, so nothing else records it.
    bass.arps = line
        .arps
        .into_iter()
        .map(|mut arp| {
            arp.intervals = voicing
                .iter()
                .find(|v| v.row <= arp.row && arp.row < v.row + v.length.get())
                .expect("an arpeggio under every row of `sixteen-steps`' arp voice")
                .intervals
                .clone();
            arp
        })
        .collect();
    song
}

fn ties_keyoffs_arp() -> Song {
    let mut held = line(2, "pulse", vec![note(0, 16, 48, Attack::Fresh, None)]);
    held.arps = vec![ArpEvent {
        row: 4,
        length: NonZeroU32::new(8).expect("non-zero"),
        intervals: vec![0, 4, 7],
        decision: None,
    }];
    let lead = line(
        1,
        "pulse",
        vec![
            note(0, 4, 60, Attack::Fresh, None),
            note(4, 4, 64, Attack::Legato, None),
            note(12, 2, 67, Attack::Fresh, None),
        ],
    );
    score(vec![subtune("main", 5, 32, vec![lead, held])])
}

fn loop_tempo() -> Song {
    let mut s = subtune(
        "main",
        5,
        32,
        vec![line(
            1,
            "pulse",
            vec![
                note(20, 4, 60, Attack::Fresh, None),
                note(28, 4, 64, Attack::Fresh, None),
            ],
        )],
    );
    s.loop_row = 16;
    s.tempo = vec![TempoChange {
        row: 24,
        frames_per_row: FramesPerRow::new(4).expect("3-127"),
    }];
    score(vec![s])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn voice(song: &Song, v: u8) -> &VoiceLine {
        song.subtunes[0]
            .voices
            .iter()
            .find(|l| l.voice.get() == v)
            .expect("the voice")
    }

    /// Each `bass-arps` arpeggio plays `sixteen-steps`' arp voice's chord at its row: the same root and intervals.
    #[test]
    fn bass_arps_are_voiced_as_sixteen_steps() {
        let reference = sixteen_steps();
        let chords = voice(&reference, 2);
        let song = bass_arps();
        let bass = voice(&song, 3);
        // Beats 2 and 4 of all eight bars.
        let rows: Vec<u32> = bass.arps.iter().map(|a| a.row).collect();
        let weak: Vec<u32> = (0..8u32).flat_map(|b| [b * 16 + 4, b * 16 + 12]).collect();
        assert_eq!(rows, weak);
        for arp in &bass.arps {
            let covers =
                |row: u32, length: NonZeroU32| row <= arp.row && arp.row < row + length.get();
            let over = chords
                .arps
                .iter()
                .find(|a| covers(a.row, a.length))
                .expect("a chord at every row");
            assert_eq!(arp.intervals, over.intervals, "row {}", arp.row);
            assert_eq!(
                arp.intervals.last(),
                Some(&12),
                "row {}: with the octave",
                arp.row
            );
            let chord = chords
                .notes
                .iter()
                .find(|n| covers(n.row, n.length))
                .expect("a chord note at every row");
            let root = bass
                .notes
                .iter()
                .find(|n| n.row == arp.row)
                .expect("a root note under every arpeggio");
            assert_eq!(
                root.pitch.semitone() % 12,
                chord.pitch.semitone() % 12,
                "row {}: the same chord root",
                arp.row
            );
            assert_eq!(root.length, arp.length, "row {}", arp.row);
        }
    }
}
