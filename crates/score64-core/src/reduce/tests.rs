//! Monophonising one voice's notes, and the decisions it records.

use std::collections::BTreeMap;

use proptest::prelude::*;

use super::*;
use crate::profile::{ProfileName, builtin};
use crate::provenance::{ContentHash, SourceRef};
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
    assert_eq!(top.decisions[0].confidence.get(), 0.5);
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
    assert_eq!(d.id.as_str(), "mono-trim-v1-r0");
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
fn a_drop_never_leaves_a_note_cut_short() {
    let n = name();
    let rules = MonoRules {
        min_rows: NonZeroU32::new(2).unwrap(),
        ..MonoRules::default()
    };
    // G, one row after E and more confident, drops E; C is trimmed to meet G, not E: no silent row before G, and one
    // decision about C.
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
        [(0, 5, "C-4".into()), (5, 4, "G-4".into())]
    );
    let ids: Vec<_> = r.decisions.iter().map(|d| d.id.as_str()).collect();
    assert_eq!(ids, ["mono-overlap-v1-r4-p52", "mono-trim-v1-r0"]);
    assert!(matches!(
        r.decisions[1].proposal,
        Proposal::TrimNote { row: 0, length, .. } if length.get() == 5
    ));
    assert_eq!(r.notes[0].decision.as_ref(), Some(&r.decisions[1].id));
    // Had C ended before G on its own, it would have got its whole length back and no decision.
    let r = monophonise(
        [
            note(0, 5, "C-4", 0.9),
            note(4, 8, "E-4", 0.3),
            note(5, 4, "G-4", 0.9),
        ],
        &at(&n),
        &rules,
    );
    assert_eq!(
        shape(&r.notes),
        [(0, 5, "C-4".into()), (5, 4, "G-4".into())]
    );
    assert_eq!(r.notes[0].decision, None);
    assert_eq!(reasons(&r), [DropReason::Overlap]);
    assert_eq!(r.decisions.len(), 1);
}

#[test]
fn a_gap_of_exactly_the_minimum_is_trimmed_not_dropped() {
    let n = name();
    let rules = MonoRules {
        min_rows: NonZeroU32::new(2).unwrap(),
        ..MonoRules::default()
    };
    let r = monophonise(
        [note(0, 4, "C-4", 0.5), note(2, 4, "E-4", 0.9)],
        &at(&n),
        &rules,
    );
    assert_eq!(
        shape(&r.notes),
        [(0, 2, "C-4".into()), (2, 4, "E-4".into())]
    );
    assert!(reasons(&r).is_empty());
    // And with the more confident note first, so the later one is checked against the note before it.
    let r = monophonise(
        [note(0, 4, "C-4", 0.9), note(2, 4, "E-4", 0.5)],
        &at(&n),
        &rules,
    );
    assert_eq!(
        shape(&r.notes),
        [(0, 2, "C-4".into()), (2, 4, "E-4".into())]
    );
    assert!(reasons(&r).is_empty());
}

#[test]
fn decision_ids_name_what_they_are_about() {
    // Ids depend on the voice, row and pitch, not on how many decisions came before: adding a note elsewhere leaves
    // them alone. Repeats of a drop on one row and pitch get -2, -3.
    let n = name();
    let base = vec![
        note(0, 5, "C-4", 0.9),
        note(4, 4, "E-4", 0.9),
        note(8, 2, "G-4", 0.9),
        note(8, 2, "G-4", 0.8),
        note(8, 1, "G-4", 0.7),
    ];
    let ids = |notes: Vec<Note>| -> Vec<String> {
        let mut v: Vec<_> = monophonise(notes, &at(&n), &MonoRules::default())
            .decisions
            .into_iter()
            .map(|d| d.id.as_str().to_owned())
            .collect();
        v.sort();
        v
    };
    let before = ids(base.clone());
    assert_eq!(
        before,
        [
            "mono-repeat-v1-r8-p55",
            "mono-repeat-v1-r8-p55-2",
            "mono-trim-v1-r0"
        ]
    );
    let mut more = base;
    more.insert(0, note(64, 2, "A-4", 0.1)); // a ghost far away
    let after = ids(more);
    assert!(before.iter().all(|id| after.contains(id)), "{after:?}");
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
    // The restarted notes record their decisions.
    let recorded: Vec<_> = r
        .notes
        .iter()
        .map(|n| n.decision.as_ref().map(|d| d.as_str()))
        .collect();
    assert_eq!(
        recorded,
        [Some("mono-tie-v1-r4"), Some("mono-tie-v1-r10"), None]
    );
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
            r#"{"change":"drop_note","voice":1,"row":8,"pitch":"F-4","length":2,"reason":"ghost"}"#,
            r#"{"change":"drop_note","voice":1,"row":0,"pitch":"C-4","length":5,"reason":"skyline"}"#,
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

/// A note soup as a transcription gives it: every note from a source span, some with their own instrument.
fn transcribed_soup() -> impl Strategy<Value = Vec<Note>> {
    strategies::note_soup().prop_flat_map(|soup| {
        let n = soup.len();
        (
            Just(soup),
            proptest::collection::vec(
                (
                    strategies::source_ref(),
                    proptest::option::weighted(0.3, strategies::preset()),
                ),
                n,
            ),
        )
            .prop_map(|(mut soup, extra)| {
                for (note, (source, instrument)) in soup.iter_mut().zip(extra) {
                    note.source = Some(source);
                    note.instrument = instrument;
                }
                soup
            })
    })
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
    fn every_note_is_kept_or_dropped_by_a_decision(
        soup in prop_oneof![strategies::note_soup(), transcribed_soup()],
        rules in any_rules(),
    ) {
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
        soup in prop_oneof![strategies::note_soup(), transcribed_soup()],
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

#[test]
fn notes_that_differ_only_in_source_or_instrument_reduce_the_same_in_any_order() {
    // Two transcriptions of one note: everything equal but where they came from, or their instrument. Whichever
    // order they come in, the same one is kept.
    let n = name();
    let source = |start| SourceRef::new(ContentHash([1; 32]), start, start + 100);
    let mut a = note(0, 4, "C-4", 0.9);
    let mut b = a.clone();
    a.source = source(0);
    b.source = source(50);
    let one = monophonise([a.clone(), b.clone()], &at(&n), &MonoRules::default());
    let two = monophonise([b, a], &at(&n), &MonoRules::default());
    assert_eq!(one, two);
    let mut c = note(0, 4, "C-4", 0.9);
    let mut d = c.clone();
    c.instrument = Some(PresetId("kick".into()));
    d.instrument = Some(PresetId("snare".into()));
    let one = monophonise([c.clone(), d.clone()], &at(&n), &MonoRules::default());
    let two = monophonise([d, c], &at(&n), &MonoRules::default());
    assert_eq!(one, two);
}

#[test]
fn drops_do_not_cascade() {
    // E would drop C, and G would drop E; C must survive, trimmed to meet G, because only G stays.
    let n = name();
    let rules = MonoRules {
        min_rows: NonZeroU32::new(2).unwrap(),
        ..MonoRules::default()
    };
    let r = monophonise(
        [
            note(0, 10, "C-4", 0.5),
            note(1, 10, "E-4", 0.6),
            note(2, 4, "G-4", 0.9),
        ],
        &at(&n),
        &rules,
    );
    assert_eq!(
        shape(&r.notes),
        [(0, 2, "C-4".into()), (2, 4, "G-4".into())]
    );
    assert_eq!(reasons(&r), [DropReason::Overlap]);
}

#[test]
fn proposals_carry_the_notes_source() {
    let n = name();
    let src = |start| SourceRef::new(ContentHash([3; 32]), start, start + 10);
    let with = |mut nt: Note, start| {
        nt.source = src(start);
        nt
    };
    let r = monophonise(
        [
            with(note(0, 6, "C-4", 0.9), 0),            // trimmed by D
            with(note(4, 4, "D-4", 0.1), 40),           // a ghost
            with(note(4, 4, "E-4", 0.9), 41),           // trims C
            with(legato(note(10, 2, "F-4", 0.9)), 100), // a tie after a gap
        ],
        &at(&n),
        &MonoRules::default(),
    );
    for d in &r.decisions {
        let source = match &d.proposal {
            Proposal::TrimNote { source, .. }
            | Proposal::DropNote { source, .. }
            | Proposal::Retrigger { source, .. } => *source,
            p => panic!("{p:?}"),
        };
        assert!(source.is_some(), "{:?} lost its source", d.proposal);
    }
    assert_eq!(r.decisions.len(), 3);
}

#[test]
fn a_restarted_note_keeps_an_earlier_rules_id() {
    let n = name();
    let mut tie = legato(note(8, 2, "C-4", 0.9));
    tie.decision = Some(DecisionId::new("octave-v1-b0").unwrap());
    let r = monophonise([tie], &at(&n), &MonoRules::default());
    assert_eq!(r.notes[0].attack, Attack::Fresh);
    assert_eq!(
        r.notes[0].decision.as_ref().unwrap().as_str(),
        "octave-v1-b0"
    );
}

#[test]
fn every_tie_break_makes_the_order_irrelevant() {
    // Pairs of notes equal in all but one field: confidence, attack or an earlier decision. Either order, one result.
    let n = name();
    let base = note(0, 4, "C-4", 0.9);
    let mut pairs = Vec::new();
    let mut other = base.clone();
    other.confidence = Confidence::new(0.5).unwrap();
    pairs.push(other);
    pairs.push(legato(base.clone()));
    let mut other = base.clone();
    other.decision = Some(DecisionId::new("octave-v1-b0").unwrap());
    pairs.push(other);
    for other in pairs {
        let one = monophonise(
            [base.clone(), other.clone()],
            &at(&n),
            &MonoRules::default(),
        );
        let two = monophonise(
            [other.clone(), base.clone()],
            &at(&n),
            &MonoRules::default(),
        );
        assert_eq!(one, two, "{other:?}");
    }
}

#[test]
fn notes_at_the_thresholds_are_kept() {
    // Exactly as confident as the ghost threshold, and exactly the minimum length.
    let n = name();
    let rules = MonoRules {
        ghost_below: Confidence::new(0.5).unwrap(),
        min_rows: NonZeroU32::new(2).unwrap(),
        ..MonoRules::default()
    };
    let r = monophonise(
        [note(0, 2, "C-4", 0.5), note(4, 2, "D-4", 0.9)],
        &at(&n),
        &rules,
    );
    assert_eq!(r.notes.len(), 2);
    assert!(r.decisions.is_empty());
}

#[test]
fn rules_read_with_defaults() {
    let rules: MonoRules = serde_json::from_str(r#"{"skyline":"lowest"}"#).unwrap();
    assert_eq!(rules.skyline, Skyline::Lowest);
    assert_eq!(rules.min_rows, MonoRules::default().min_rows);
}
