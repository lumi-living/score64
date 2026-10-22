//! Monophonising one voice's notes, and the decisions it records.

use std::collections::BTreeMap;

use proptest::prelude::*;

use super::*;
use crate::profile::{ProfileName, builtin};
use crate::score::{FramesPerRow, Pitch, PresetId, Role, Roles, Schema, Song, Subtune, VoiceLine};
use crate::strategies;
use crate::validate::{Context, InstrumentFacts, validate};

fn name() -> SubtuneName {
    SubtuneName::new("main").unwrap()
}

fn at(subtune: &SubtuneName) -> Placement<'_> {
    Placement {
        subtune,
        voice: Voice::new(1).unwrap(),
        rows_per_bar: NonZeroU8::new(16).unwrap(),
    }
}

fn note(row: u32, length: u32, pitch: &str, confidence: f32) -> Note {
    Note {
        row,
        length: NonZeroU32::new(length).unwrap(),
        pitch: pitch.parse().unwrap(),
        attack: Attack::Fresh,
        instrument: None,
        source: None,
        confidence: Confidence::new(confidence).unwrap(),
        decision: None,
    }
}

fn legato(mut n: Note) -> Note {
    n.attack = Attack::Legato;
    n
}

/// (row, length, pitch) of each note.
fn shape(notes: &[Note]) -> Vec<(u32, u32, String)> {
    notes
        .iter()
        .map(|n| (n.row, n.length.get(), n.pitch.to_string()))
        .collect()
}

fn proposals(r: &Reduced) -> Vec<&Proposal> {
    r.decisions.iter().map(|d| &d.proposal).collect()
}

fn reasons(r: &Reduced) -> Vec<DropReason> {
    r.decisions
        .iter()
        .filter_map(|d| match d.proposal {
            Proposal::DropNote { reason, .. } => Some(reason),
            _ => None,
        })
        .collect()
}

#[test]
fn a_clean_line_is_left_alone() {
    let n = name();
    let notes = vec![
        note(0, 4, "C-4", 0.9),
        note(4, 2, "E-4", 0.9),
        legato(note(6, 2, "G-4", 0.9)),
    ];
    let r = monophonise(notes.clone(), &at(&n), &MonoRules::default());
    assert_eq!(r.notes, notes);
    assert!(r.decisions.is_empty());
}

#[test]
fn ghosts_and_short_notes_are_dropped() {
    let n = name();
    let rules = MonoRules {
        min_rows: NonZeroU32::new(2).unwrap(),
        ..MonoRules::default()
    };
    let r = monophonise(
        [
            note(0, 4, "C-4", 0.9),
            note(4, 4, "D-4", 0.1),
            note(8, 1, "E-4", 0.9),
        ],
        &at(&n),
        &rules,
    );
    assert_eq!(shape(&r.notes), [(0, 4, "C-4".into())]);
    assert_eq!(reasons(&r), [DropReason::Ghost, DropReason::TooShort]);
    // A ghost's drop is as sure as the note was unsure.
    assert!((r.decisions[0].confidence.get() - 0.9).abs() < 1e-6);
}

#[test]
fn the_skyline_keeps_the_top_or_the_bottom_of_a_chord() {
    let n = name();
    let chord = || {
        [
            note(0, 4, "C-4", 0.9),
            note(0, 4, "G-4", 0.9),
            note(0, 4, "E-4", 0.9),
        ]
    };
    let top = monophonise(chord(), &at(&n), &MonoRules::default());
    assert_eq!(shape(&top.notes), [(0, 4, "G-4".into())]);
    assert_eq!(reasons(&top), [DropReason::Skyline, DropReason::Skyline]);
    assert_eq!(top.decisions[0].confidence.get(), SKYLINE_CONFIDENCE);
    let bass = MonoRules {
        skyline: Skyline::Lowest,
        ..MonoRules::default()
    };
    let bottom = monophonise(chord(), &at(&n), &bass);
    assert_eq!(shape(&bottom.notes), [(0, 4, "C-4".into())]);
}

#[test]
fn repeats_merge_into_the_longest() {
    let n = name();
    let r = monophonise(
        [
            note(0, 2, "C-4", 0.9),
            note(0, 6, "C-4", 0.5),
            note(0, 2, "C-4", 0.9),
        ],
        &at(&n),
        &MonoRules::default(),
    );
    assert_eq!(shape(&r.notes), [(0, 6, "C-4".into())]);
    assert_eq!(reasons(&r), [DropReason::Repeat, DropReason::Repeat]);
}

#[test]
fn overlaps_are_trimmed_and_recorded() {
    // An end that rounded up past the next start (#138): trimmed to meet it, and the note records the decision.
    let n = name();
    let r = monophonise(
        [note(0, 5, "C-4", 0.9), note(4, 4, "E-4", 0.9)],
        &at(&n),
        &MonoRules::default(),
    );
    assert_eq!(
        shape(&r.notes),
        [(0, 4, "C-4".into()), (4, 4, "E-4".into())]
    );
    let d = &r.decisions[0];
    assert_eq!(
        d.proposal,
        Proposal::TrimNote {
            voice: Voice::new(1).unwrap(),
            row: 0,
            pitch: "C-4".parse().unwrap(),
            source: None,
            length: NonZeroU32::new(4).unwrap(),
        }
    );
    assert_eq!(r.notes[0].decision.as_ref(), Some(&d.id));
    assert_eq!(d.id.as_str(), "mono-v1-0");
    assert_eq!((d.subtune.as_str(), d.bar), ("main", 0));
    assert_eq!(
        d.provenance,
        Provenance::Rules {
            rule: "monophonise@1".into()
        }
    );
    assert!(!d.needs_review(), "rules decisions need no review");
}

#[test]
fn an_overlap_too_close_to_trim_drops_the_less_confident_note() {
    let n = name();
    let rules = MonoRules {
        min_rows: NonZeroU32::new(2).unwrap(),
        ..MonoRules::default()
    };
    // One row apart: trimming would leave one row, under the minimum of two.
    let r = monophonise(
        [note(0, 4, "C-4", 0.5), note(1, 4, "E-4", 0.9)],
        &at(&n),
        &rules,
    );
    assert_eq!(shape(&r.notes), [(1, 4, "E-4".into())]);
    assert_eq!(reasons(&r), [DropReason::Overlap]);
    // On a tie the earlier note stays.
    let r = monophonise(
        [note(0, 4, "C-4", 0.9), note(1, 4, "E-4", 0.9)],
        &at(&n),
        &rules,
    );
    assert_eq!(shape(&r.notes), [(0, 4, "C-4".into())]);
}

#[test]
fn a_trimmed_note_that_is_dropped_later_keeps_only_its_drop() {
    let n = name();
    let rules = MonoRules {
        min_rows: NonZeroU32::new(2).unwrap(),
        ..MonoRules::default()
    };
    // C is trimmed to meet E at row 4; then G, one row after E and more confident, drops E. C (ending at 4) no longer
    // overlaps anything, so its trim stays; E's drop is the only decision about E.
    let r = monophonise(
        [
            note(0, 8, "C-4", 0.9),
            note(4, 8, "E-4", 0.3),
            note(5, 4, "G-4", 0.9),
        ],
        &at(&n),
        &rules,
    );
    assert_eq!(
        shape(&r.notes),
        [(0, 4, "C-4".into()), (5, 4, "G-4".into())]
    );
    let about_e: Vec<_> = proposals(&r)
        .into_iter()
        .filter(|p| {
            matches!(
                p,
                Proposal::TrimNote { row: 4, .. } | Proposal::DropNote { row: 4, .. }
            )
        })
        .collect();
    assert_eq!(about_e.len(), 1);
    assert!(matches!(
        about_e[0],
        Proposal::DropNote {
            reason: DropReason::Overlap,
            ..
        }
    ));
}

#[test]
fn ties_that_lost_their_note_start_fresh() {
    let n = name();
    // The tie at row 4 follows a ghost, which is dropped; the tie at row 10 follows a gap.
    let r = monophonise(
        [
            note(0, 4, "C-4", 0.1),
            legato(note(4, 4, "D-4", 0.9)),
            legato(note(10, 2, "E-4", 0.9)),
            legato(note(12, 2, "F-4", 0.9)),
        ],
        &at(&n),
        &MonoRules::default(),
    );
    let attacks: Vec<_> = r.notes.iter().map(|n| n.attack).collect();
    assert_eq!(attacks, [Attack::Fresh, Attack::Fresh, Attack::Legato]);
    let restarted: Vec<_> = proposals(&r)
        .into_iter()
        .filter_map(|p| match p {
            Proposal::Retrigger { row, .. } => Some(*row),
            _ => None,
        })
        .collect();
    assert_eq!(restarted, [4, 10]);
}

#[test]
fn decisions_name_the_bar_and_keep_an_earlier_rules_id() {
    let n = name();
    let mut early = note(32, 6, "C-4", 0.9);
    early.decision = Some(DecisionId::new("octave-1").unwrap());
    let r = monophonise(
        [early, note(36, 2, "D-4", 0.9)],
        &at(&n),
        &MonoRules::default(),
    );
    assert_eq!(r.decisions[0].bar, 2);
    assert_eq!(r.notes[0].decision.as_ref().unwrap().as_str(), "octave-1");
}

#[test]
fn decisions_round_trip_through_json() {
    let n = name();
    let r = monophonise(
        [
            note(0, 5, "C-4", 0.9),
            note(0, 5, "E-4", 0.9),
            note(4, 4, "G-4", 0.9),
            note(8, 2, "F-4", 0.1),
        ],
        &at(&n),
        &MonoRules::default(),
    );
    for d in &r.decisions {
        let json = serde_json::to_string(d).unwrap();
        assert_eq!(
            &serde_json::from_str::<Decision>(&json).unwrap(),
            d,
            "{json}"
        );
    }
    // Ghosts go first, then the chord.
    let json: Vec<_> = r
        .decisions
        .iter()
        .map(|d| serde_json::to_string(&d.proposal).unwrap())
        .collect();
    assert_eq!(
        json,
        [
            r#"{"change":"drop_note","voice":1,"row":8,"pitch":"F-4","reason":"ghost"}"#,
            r#"{"change":"drop_note","voice":1,"row":0,"pitch":"C-4","reason":"skyline"}"#,
            r#"{"change":"trim_note","voice":1,"row":0,"pitch":"E-4","length":4}"#,
        ]
    );
}

/// The song and context a reduced line is checked in: one subtune of 300 rows under `full`, a lead line on voice 1
/// with an instrument fitted to it.
fn checked(notes: Vec<Note>) -> (Song, Context) {
    let song = Song {
        schema: Schema::default(),
        profile: ProfileName::new(builtin::FULL).unwrap(),
        subtunes: vec![Subtune {
            name: name(),
            profile: None,
            frames_per_row: FramesPerRow::new(6).unwrap(),
            tempo: vec![],
            rows_per_bar: NonZeroU8::new(16).unwrap(),
            length: NonZeroU32::new(300).unwrap(),
            loop_row: 0,
            marks: vec![],
            voices: vec![VoiceLine {
                voice: Voice::new(1).unwrap(),
                roles: Roles::constant(Role::Lead),
                instrument: PresetId("lead".into()),
                notes,
                arps: vec![],
            }],
        }],
    };
    let facts = InstrumentFacts {
        filter: false,
        pulse_at_start: true,
        gate_timer: 2,
        hard_restart: true,
        gate_off: true,
    };
    let ctx = Context {
        instruments: BTreeMap::from([(PresetId("lead".into()), facts)]),
        ..Context::default()
    };
    (song, ctx)
}

fn any_rules() -> impl Strategy<Value = MonoRules> {
    (any::<bool>(), 0.0f32..0.6, 1u32..4).prop_map(|(low, ghost, min)| MonoRules {
        skyline: if low {
            Skyline::Lowest
        } else {
            Skyline::Highest
        },
        ghost_below: Confidence::new(ghost).unwrap(),
        min_rows: NonZeroU32::new(min).unwrap(),
    })
}

proptest! {
    /// Any note soup reduces to a line that validates with no errors.
    #[test]
    fn any_soup_reduces_to_a_valid_line(soup in strategies::note_soup(), rules in any_rules()) {
        let n = name();
        let r = monophonise(soup, &at(&n), &rules);
        let (song, ctx) = checked(r.notes);
        let errors: Vec<_> = validate(&song, &ctx).into_iter().filter(|p| p.is_error()).collect();
        prop_assert!(errors.is_empty(), "{errors:#?}");
    }

    /// Every input note is accounted for: kept (same row and pitch, no longer than it was) or dropped by a decision;
    /// every change is a decision, and every decision id a note records is one of them.
    #[test]
    fn every_note_is_kept_or_dropped_by_a_decision(soup in strategies::note_soup(), rules in any_rules()) {
        let n = name();
        let r = monophonise(soup.clone(), &at(&n), &rules);
        let dropped = reasons(&r).len();
        prop_assert_eq!(r.notes.len() + dropped, soup.len());
        let mut unused: Vec<&Note> = soup.iter().collect();
        for kept in &r.notes {
            // Its input note: same row, pitch and confidence, no shorter; one of the same length if there is one.
            let same = |o: &&Note| o.row == kept.row && o.pitch == kept.pitch && o.confidence == kept.confidence;
            let i = unused
                .iter()
                .position(|o| same(o) && o.length == kept.length)
                .or_else(|| unused.iter().position(|o| same(o) && o.length > kept.length));
            prop_assert!(i.is_some(), "{kept:?} has no input note");
            let original = unused.swap_remove(i.unwrap());
            let trimmed = original.length != kept.length;
            let restarted = original.attack != kept.attack;
            let trims = proposals(&r).iter().any(|p| matches!(p, Proposal::TrimNote { row, pitch, length, .. }
                if *row == kept.row && *pitch == kept.pitch && *length == kept.length));
            let restarts = proposals(&r).iter().any(|p| matches!(p, Proposal::Retrigger { row, pitch, .. }
                if *row == kept.row && *pitch == kept.pitch));
            prop_assert_eq!(trimmed, trims);
            prop_assert_eq!(restarted, restarts);
            prop_assert!(!restarted || kept.attack == Attack::Fresh);
        }
        for d in &r.decisions {
            prop_assert_eq!(&d.provenance, &Provenance::Rules { rule: MONOPHONISE.into() });
            prop_assert_eq!(d.subtune.as_str(), "main");
        }
        let ids: Vec<_> = r.decisions.iter().map(|d| &d.id).collect();
        let mut unique = ids.clone();
        unique.sort();
        unique.dedup();
        prop_assert_eq!(unique.len(), ids.len(), "ids are unique");
        for kept in &r.notes {
            if let Some(id) = &kept.decision {
                prop_assert!(ids.contains(&id), "{id:?} is not a decision of this call");
            }
        }
    }

    /// The input order does not matter.
    #[test]
    fn the_result_does_not_depend_on_the_input_order(
        soup in strategies::note_soup(),
        rules in any_rules(),
        seed in any::<u64>(),
    ) {
        let n = name();
        let mut shuffled = soup.clone();
        // A cheap deterministic shuffle.
        let mut x = seed | 1;
        for i in (1..shuffled.len()).rev() {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            shuffled.swap(i, (x % (i as u64 + 1)) as usize);
        }
        prop_assert_eq!(monophonise(soup, &at(&n), &rules), monophonise(shuffled, &at(&n), &rules));
    }
}

#[test]
fn pitch_is_a_goattracker_pitch() {
    // The helpers above lean on GoatTracker names.
    assert_eq!("G#7".parse::<Pitch>().unwrap(), Pitch::MAX);
}
