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
    // As sure as E was surer than C.
    assert!((r.decisions[0].confidence.get() - 0.4).abs() < 1e-6);
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
        // Step 3's promises: kept notes are at least the minimum long, and an overlap drop sits too close to a kept
        // note at least as confident.
        for kept in &r.notes {
            prop_assert!(kept.length >= rules.min_rows, "{kept:?}");
        }
        for d in &r.decisions {
            if let Proposal::DropNote { row, pitch, length, reason: DropReason::Overlap, .. } = d.proposal {
                let conf = soup
                    .iter()
                    .filter(|o| o.row == row && o.pitch == pitch && o.length == length)
                    .map(|o| o.confidence.get())
                    .fold(f32::INFINITY, f32::min);
                let near = r.notes.iter().any(|k| {
                    k.row.abs_diff(row) < rules.min_rows.get() && k.confidence.get() >= conf
                });
                prop_assert!(near, "overlap drop at {row} has no confident neighbour");
            }
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

#[test]
fn ids_and_bars_name_the_voice_and_the_bar_edges() {
    // Voice 3, a tie on row 16 (the first row of bar 1) and a too-short note on row 15 (the last of bar 0).
    let n = name();
    let place = Placement {
        voice: Voice::new(3).unwrap(),
        ..at(&n)
    };
    let rules = MonoRules {
        min_rows: NonZeroU32::new(2).unwrap(),
        ..MonoRules::default()
    };
    let r = monophonise(
        [note(15, 1, "C-4", 0.9), legato(note(16, 4, "D-4", 0.9))],
        &place,
        &rules,
    );
    let got: Vec<_> = r.decisions.iter().map(|d| (d.id.as_str(), d.bar)).collect();
    assert_eq!(got, [("mono-short-v3-r15-p48", 0), ("mono-tie-v3-r16", 1)]);
}

/// A person's decision about voice 1's note at `row` and `pitch`.
fn person(id: &str, proposal: Proposal) -> Decision {
    let row = match proposal {
        Proposal::DropNote { row, .. }
        | Proposal::TrimNote { row, .. }
        | Proposal::Retrigger { row, .. } => row,
        _ => 0,
    };
    Decision {
        id: DecisionId::new(id).unwrap(),
        subtune: name(),
        bar: row / 16,
        proposal,
        provenance: Provenance::Manual,
        confidence: Confidence::CERTAIN,
        reviewed: false,
    }
}

fn v1() -> Voice {
    Voice::new(1).unwrap()
}

fn drop_of(row: u32, length: u32, pitch: &str) -> Proposal {
    Proposal::DropNote {
        voice: v1(),
        row,
        pitch: pitch.parse().unwrap(),
        length: NonZeroU32::new(length).unwrap(),
        source: None,
        reason: DropReason::Overlap,
    }
}

fn trim_of(row: u32, pitch: &str, length: u32) -> Proposal {
    Proposal::TrimNote {
        voice: v1(),
        row,
        pitch: pitch.parse().unwrap(),
        length: NonZeroU32::new(length).unwrap(),
        source: None,
    }
}

fn retrigger_of(row: u32, pitch: &str) -> Proposal {
    Proposal::Retrigger {
        voice: v1(),
        row,
        pitch: pitch.parse().unwrap(),
        source: None,
    }
}

fn unapplied(r: &Reduced) -> Vec<(&str, Unapplied)> {
    r.unapplied
        .iter()
        .map(|(id, w)| (id.as_str(), *w))
        .collect()
}

#[test]
fn a_forced_drop_takes_the_note_out_before_the_rule_runs() {
    let n = name();
    // Without overrides the confident C-4 wins the overlap and E-4 is dropped.
    let soup = vec![
        note(0, 8, "C-4", 0.9),
        note(2, 8, "E-4", 0.5),
        note(8, 4, "G-4", 0.8),
    ];
    let rules = MonoRules {
        min_rows: NonZeroU32::new(4).unwrap(),
        ..MonoRules::default()
    };
    let plain = monophonise(soup.clone(), &at(&n), &rules);
    assert_eq!(
        shape(&plain.notes),
        [(0, 8, "C-4".into()), (8, 4, "G-4".into())]
    );
    // A person drops C-4: E-4 has nothing to lose to, and keeps its place, trimmed where G-4 starts.
    let r = monophonise_with(
        soup,
        &at(&n),
        &rules,
        &[person("no-c", drop_of(0, 8, "C-4"))],
    );
    assert_eq!(
        shape(&r.notes),
        [(2, 6, "E-4".into()), (8, 4, "G-4".into())]
    );
    assert!(r.unapplied.is_empty());
    // The rule records nothing about C-4; the drop is the person's.
    assert!(
        r.decisions
            .iter()
            .all(|d| !matches!(d.proposal, Proposal::DropNote { row: 0, .. }))
    );
    let (song, ctx) = checked(r.notes);
    assert!(validate(&song, &ctx).iter().all(|p| !p.is_error()));
}

#[test]
fn trims_and_fresh_starts_in_force_set_the_note() {
    let n = name();
    let soup = vec![
        note(0, 4, "C-4", 0.9),
        legato(note(4, 4, "D-4", 0.9)),
        note(12, 4, "E-4", 0.9),
    ];
    let r = monophonise_with(
        soup.clone(),
        &at(&n),
        &MonoRules::default(),
        &[
            person("short-c", trim_of(0, "C-4", 2)),
            person("long-d", trim_of(4, "D-4", 8)),
            person("long-e", trim_of(12, "E-4", 100)),
        ],
    );
    // C-4 shortened, so D-4's tie lost its note and the rule restarts it. A trim only shortens: D-4 into the gap,
    // and the last note past its own end (or the subtune's), are refused.
    assert_eq!(
        shape(&r.notes),
        [
            (0, 2, "C-4".into()),
            (4, 4, "D-4".into()),
            (12, 4, "E-4".into())
        ]
    );
    assert_eq!(
        unapplied(&r),
        [
            ("long-d", Unapplied::Lengthens),
            ("long-e", Unapplied::Lengthens)
        ]
    );
    assert_eq!(
        r.notes[0].decision.as_ref().map(|d| d.as_str()),
        Some("short-c")
    );
    assert_eq!(r.notes[1].attack, Attack::Fresh);
    assert!(
        proposals(&r)
            .iter()
            .any(|p| matches!(p, Proposal::Retrigger { row: 4, .. }))
    );
    // A fresh start in force: D-4 starts fresh with the person's id, and the rule records no tie of its own.
    let r = monophonise_with(
        vec![note(0, 2, "C-4", 0.9), legato(note(4, 4, "D-4", 0.9))],
        &at(&n),
        &MonoRules::default(),
        &[person("fresh-d", retrigger_of(4, "D-4"))],
    );
    assert_eq!(r.notes[1].attack, Attack::Fresh);
    assert_eq!(
        r.notes[1].decision.as_ref().map(|d| d.as_str()),
        Some("fresh-d")
    );
    assert!(r.decisions.is_empty(), "{:?}", r.decisions);
}

#[test]
fn a_rule_trim_gives_way_to_a_trim_in_force() {
    let n = name();
    // The rule trims C-4 to 4, where D-4 starts; the person wants 3.
    let soup = vec![note(0, 8, "C-4", 0.9), note(4, 4, "D-4", 0.9)];
    let plain = monophonise(soup.clone(), &at(&n), &MonoRules::default());
    assert!(
        proposals(&plain)
            .iter()
            .any(|p| matches!(p, Proposal::TrimNote { row: 0, .. }))
    );
    let r = monophonise_with(
        soup,
        &at(&n),
        &MonoRules::default(),
        &[person("c3", trim_of(0, "C-4", 3))],
    );
    assert_eq!(shape(&r.notes)[0], (0, 3, "C-4".into()));
    assert!(
        proposals(&r)
            .iter()
            .all(|p| !matches!(p, Proposal::TrimNote { .. })),
        "the rule's trim is gone"
    );
    assert_eq!(r.notes[0].decision.as_ref().map(|d| d.as_str()), Some("c3"));
}

#[test]
fn overrides_that_cant_apply_say_why() {
    let n = name();
    let soup = vec![note(0, 4, "C-4", 0.9), note(4, 4, "D-4", 0.9)];
    let r = monophonise_with(
        soup.clone(),
        &at(&n),
        &MonoRules::default(),
        &[
            person("gone", drop_of(0, 2, "C-4")),      // no C-4 of length 2
            person("into-d", trim_of(0, "C-4", 5)),    // would overlap D-4 at row 4
            person("absent", trim_of(8, "E-4", 2)),    // no such note
            person("nowhere", retrigger_of(2, "C-4")), // no such note
        ],
    );
    assert_eq!(
        unapplied(&r),
        [
            ("absent", Unapplied::NoSuchNote),
            ("gone", Unapplied::NoSuchNote),
            ("into-d", Unapplied::Lengthens),
            ("nowhere", Unapplied::NoSuchNote),
        ]
    );
    assert_eq!(
        r.notes,
        monophonise(soup, &at(&n), &MonoRules::default()).notes
    );
}

#[test]
fn only_this_voice_and_subtune_and_not_the_rules_own() {
    let n = name();
    let soup = vec![note(0, 4, "C-4", 0.9)];
    let mut rules_drop = person("by-rules", drop_of(0, 4, "C-4"));
    rules_drop.provenance = Provenance::Rules {
        rule: MONOPHONISE.into(),
    };
    let mut elsewhere = person("other-subtune", drop_of(0, 4, "C-4"));
    elsewhere.subtune = SubtuneName::new("combat").unwrap();
    let other_voice = person(
        "voice-2",
        Proposal::DropNote {
            voice: Voice::new(2).unwrap(),
            row: 0,
            pitch: "C-4".parse().unwrap(),
            length: NonZeroU32::new(4).unwrap(),
            source: None,
            reason: DropReason::Overlap,
        },
    );
    let r = monophonise_with(
        soup,
        &at(&n),
        &MonoRules::default(),
        &[rules_drop, elsewhere, other_voice],
    );
    assert_eq!(r.notes.len(), 1);
    assert!(r.unapplied.is_empty());
    // A model's decision counts like a person's here; resolve has already chosen between them.
    let mut by_model = person("by-model", drop_of(0, 4, "C-4"));
    by_model.provenance = Provenance::Llm {
        model: "any".into(),
        prompt: ContentHash([1; 32]),
    };
    let r = monophonise_with(
        vec![note(0, 4, "C-4", 0.9)],
        &at(&n),
        &MonoRules::default(),
        &[by_model],
    );
    assert!(r.notes.is_empty());
}

proptest! {
    /// Overrides drawn from the notes themselves (drops, trims of any length, fresh starts) still reduce to a line
    /// that validates with no errors, and every override either applied or says why not.
    #[test]
    fn any_overrides_still_give_a_valid_line(
        soup in strategies::note_soup(),
        rules in any_rules(),
        picks in proptest::collection::vec((any::<proptest::sample::Index>(), 0u8..3, 1u32..12), 0..6),
    ) {
        prop_assume!(!soup.is_empty());
        let n = name();
        let overrides: Vec<Decision> = picks
            .iter()
            .enumerate()
            .map(|(k, (i, kind, len))| {
                let o = &soup[i.index(soup.len())];
                let pitch = o.pitch.to_string();
                let p = match kind {
                    0 => drop_of(o.row, o.length.get(), &pitch),
                    1 => trim_of(o.row, &pitch, *len),
                    _ => retrigger_of(o.row, &pitch),
                };
                person(&format!("o{k}"), p)
            })
            .collect();
        let r = monophonise_with(soup, &at(&n), &rules, &overrides);
        let (song, ctx) = checked(r.notes.clone());
        let errors: Vec<_> = validate(&song, &ctx).into_iter().filter(|p| p.is_error()).collect();
        prop_assert!(errors.is_empty(), "{errors:#?}");
        prop_assert!(r.unapplied.len() <= overrides.len());
        for w in r.notes.windows(2) {
            prop_assert!(w[0].row + w[0].length.get() <= w[1].row, "overlap {w:?}");
        }
    }
}

#[test]
fn a_forced_drop_of_identical_twins_takes_the_one_the_rule_would() {
    let n = name();
    // Two C-4s of one length on one row: the rule keeps the fresh, confident one and drops the other as a repeat.
    let mut twin = legato(note(0, 4, "C-4", 0.5));
    twin.instrument = Some(PresetId("other".into()));
    let soup = vec![note(0, 4, "C-4", 0.9), twin];
    let plain = monophonise(soup.clone(), &at(&n), &MonoRules::default());
    let r = monophonise_with(
        soup.clone(),
        &at(&n),
        &MonoRules::default(),
        &[person("agree", drop_of(0, 4, "C-4"))],
    );
    // Agreeing with the rule changes nothing in the line.
    assert_eq!(r.notes, plain.notes);
    // With a source span, the drop takes exactly that note.
    let span = |k: u8| SourceRef::new(ContentHash([k; 32]), 0, 1).unwrap();
    let mut a = note(0, 4, "C-4", 0.9);
    a.source = Some(span(1));
    let mut b = note(0, 4, "C-4", 0.5);
    b.source = Some(span(2));
    let mut by_span = drop_of(0, 4, "C-4");
    if let Proposal::DropNote { source, .. } = &mut by_span {
        *source = Some(span(1));
    }
    let r = monophonise_with(
        vec![a, b],
        &at(&n),
        &MonoRules::default(),
        &[person("first", by_span)],
    );
    assert_eq!(r.notes.len(), 1);
    assert_eq!(r.notes[0].source, Some(span(2)));
}

#[test]
fn a_models_drop_gives_way_to_a_persons_trim_or_fresh_start() {
    let n = name();
    let mut model_drop = person("model-drop", drop_of(0, 8, "C-4"));
    model_drop.provenance = Provenance::Llm {
        model: "any".into(),
        prompt: ContentHash([1; 32]),
    };
    let soup = vec![note(0, 8, "C-4", 0.9), note(8, 4, "D-4", 0.9)];
    let r = monophonise_with(
        soup,
        &at(&n),
        &MonoRules::default(),
        &[model_drop, person("short-c", trim_of(0, "C-4", 4))],
    );
    assert_eq!(
        shape(&r.notes),
        [(0, 4, "C-4".into()), (8, 4, "D-4".into())]
    );
    assert_eq!(unapplied(&r), [("model-drop", Unapplied::Overruled)]);
}

#[test]
fn the_rules_trim_stays_where_no_trim_applied() {
    let n = name();
    // The rule trims both C-4 (to 4) and E-4 (to 4); only C-4 has a trim in force, and a second trim for E-4 can't
    // apply (it lengthens).
    let soup = vec![
        note(0, 8, "C-4", 0.9),
        note(4, 8, "E-4", 0.9),
        note(8, 4, "G-4", 0.9),
    ];
    let r = monophonise_with(
        soup,
        &at(&n),
        &MonoRules::default(),
        &[
            person("c2", trim_of(0, "C-4", 2)),
            person("e6", trim_of(4, "E-4", 6)),
        ],
    );
    assert_eq!(unapplied(&r), [("e6", Unapplied::Lengthens)]);
    let trims: Vec<u32> = r
        .decisions
        .iter()
        .filter_map(|d| match d.proposal {
            Proposal::TrimNote { row, .. } => Some(row),
            _ => None,
        })
        .collect();
    assert_eq!(trims, [4], "only E-4 keeps the rule's trim");
    assert_eq!(r.notes[0].decision.as_ref().map(|d| d.as_str()), Some("c2"));
    assert_eq!(
        r.notes[1].decision.as_ref().map(|d| d.as_str()),
        Some("mono-trim-v1-r4")
    );
    // A trim to the rule's own length replaces the rule's trim, and the note records the override.
    let soup = vec![note(0, 8, "C-4", 0.9), note(4, 4, "E-4", 0.9)];
    let r = monophonise_with(
        soup,
        &at(&n),
        &MonoRules::default(),
        &[person("c4", trim_of(0, "C-4", 4))],
    );
    assert!(r.decisions.is_empty(), "{:?}", r.decisions);
    assert_eq!(r.notes[0].decision.as_ref().map(|d| d.as_str()), Some("c4"));
}

#[test]
fn an_earlier_rules_id_and_unchanged_notes_keep_what_they_record() {
    let n = name();
    let mut folded = note(0, 8, "C-4", 0.9);
    folded.decision = Some(DecisionId::new("octave-v1-b0").unwrap());
    let r = monophonise_with(
        vec![folded, note(12, 4, "D-4", 0.9)],
        &at(&n),
        &MonoRules::default(),
        &[
            person("c2", trim_of(0, "C-4", 2)),
            // Already fresh, and already 4 long: these change nothing, so D-4 records nothing.
            person("fresh-d", retrigger_of(12, "D-4")),
            person("same-d", trim_of(12, "D-4", 4)),
        ],
    );
    assert_eq!(shape(&r.notes)[0], (0, 2, "C-4".into()));
    assert_eq!(
        r.notes[0].decision.as_ref().map(|d| d.as_str()),
        Some("octave-v1-b0")
    );
    assert_eq!(r.notes[1].decision, None);
    assert!(r.unapplied.is_empty());
}

#[test]
fn below_the_minimum_is_a_persons_call() {
    let n = name();
    let rules = MonoRules {
        min_rows: NonZeroU32::new(4).unwrap(),
        ..MonoRules::default()
    };
    let soup = vec![note(0, 8, "C-4", 0.9), note(8, 4, "D-4", 0.9)];
    let mut by_model = person("model", trim_of(0, "C-4", 1));
    by_model.provenance = Provenance::Llm {
        model: "any".into(),
        prompt: ContentHash([1; 32]),
    };
    let r = monophonise_with(soup.clone(), &at(&n), &rules, &[by_model.clone()]);
    assert_eq!(unapplied(&r), [("model", Unapplied::OutOfRange)]);
    // Reviewed, it is the person's.
    by_model.reviewed = true;
    let r = monophonise_with(soup.clone(), &at(&n), &rules, &[by_model]);
    assert_eq!(shape(&r.notes)[0], (0, 1, "C-4".into()));
    let r = monophonise_with(
        soup,
        &at(&n),
        &rules,
        &[person("mine", trim_of(0, "C-4", 1))],
    );
    assert_eq!(shape(&r.notes)[0], (0, 1, "C-4".into()));
}

proptest! {
    /// Overrides in any order give the same result, and every id a note records is a decision of this call, an
    /// override that applied, or one the note came in with.
    #[test]
    fn overrides_in_any_order_and_ids_that_exist(
        soup in strategies::note_soup(),
        rules in any_rules(),
        picks in proptest::collection::vec((any::<proptest::sample::Index>(), 0u8..3, 1u32..12, any::<bool>()), 0..6),
        seed in any::<u64>(),
    ) {
        prop_assume!(!soup.is_empty());
        let n = name();
        let overrides: Vec<Decision> = picks
            .iter()
            .enumerate()
            .map(|(k, (i, kind, len, model))| {
                let o = &soup[i.index(soup.len())];
                let pitch = o.pitch.to_string();
                let p = match kind {
                    0 => drop_of(o.row, o.length.get(), &pitch),
                    1 => trim_of(o.row, &pitch, *len),
                    _ => retrigger_of(o.row, &pitch),
                };
                let mut d = person(&format!("o{k}"), p);
                if *model {
                    d.provenance = Provenance::Llm { model: "any".into(), prompt: ContentHash([1; 32]) };
                }
                d
            })
            .collect();
        let r = monophonise_with(soup.clone(), &at(&n), &rules, &overrides);
        let mut shuffled = overrides.clone();
        let mut state = seed;
        for i in (1..shuffled.len()).rev() {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            shuffled.swap(i, (state >> 33) as usize % (i + 1));
        }
        let again = monophonise_with(soup, &at(&n), &rules, &shuffled);
        prop_assert_eq!(&r.notes, &again.notes);
        prop_assert_eq!(&r.decisions, &again.decisions);
        let mut a = r.unapplied.clone();
        let mut b = again.unapplied.clone();
        a.sort_by(|x, y| x.0.cmp(&y.0));
        b.sort_by(|x, y| x.0.cmp(&y.0));
        prop_assert_eq!(a, b);
        let refused: Vec<&DecisionId> = r.unapplied.iter().map(|(id, _)| id).collect();
        for kept in &r.notes {
            if let Some(id) = &kept.decision {
                let ok = r.decisions.iter().any(|d| &d.id == id)
                    || overrides.iter().any(|d| &d.id == id && !refused.contains(&id));
                prop_assert!(ok, "{id:?} is neither a decision of the call nor an applied override");
            }
        }
    }
}

fn as_model(mut d: Decision) -> Decision {
    d.provenance = Provenance::Llm {
        model: "any".into(),
        prompt: ContentHash([1; 32]),
    };
    d
}

#[test]
fn of_several_decisions_for_one_slot_the_strongest_wins_in_any_order() {
    let n = name();
    let soup = vec![note(0, 8, "C-4", 0.9), note(8, 4, "D-4", 0.9)];
    // A person's trim to 4 beats a model's trim to 2, whatever their order.
    for overrides in [
        vec![
            as_model(person("m-trim", trim_of(0, "C-4", 2))),
            person("p-trim", trim_of(0, "C-4", 4)),
        ],
        vec![
            person("p-trim", trim_of(0, "C-4", 4)),
            as_model(person("m-trim", trim_of(0, "C-4", 2))),
        ],
    ] {
        let r = monophonise_with(soup.clone(), &at(&n), &MonoRules::default(), &overrides);
        assert_eq!(shape(&r.notes)[0], (0, 4, "C-4".into()));
        assert_eq!(
            r.notes[0].decision.as_ref().map(|d| d.as_str()),
            Some("p-trim")
        );
        assert_eq!(unapplied(&r), [("m-trim", Unapplied::Overruled)]);
    }
    // Two drops of one slot over identical twins take one twin, not both.
    let twins = vec![note(0, 4, "C-4", 0.9), note(0, 4, "C-4", 0.5)];
    let r = monophonise_with(
        twins,
        &at(&n),
        &MonoRules::default(),
        &[
            person("p-drop", drop_of(0, 4, "C-4")),
            as_model(person("m-drop", drop_of(0, 4, "C-4"))),
        ],
    );
    assert_eq!(r.notes.len(), 1);
    assert_eq!(unapplied(&r), [("m-drop", Unapplied::Overruled)]);
    // A second fresh start of a note by the same rank is outranked.
    let r = monophonise_with(
        vec![note(0, 4, "C-4", 0.9), legato(note(4, 4, "D-4", 0.9))],
        &at(&n),
        &MonoRules::default(),
        &[
            person("b-fresh", retrigger_of(4, "D-4")),
            person("a-fresh", retrigger_of(4, "D-4")),
        ],
    );
    assert_eq!(
        r.notes[1].decision.as_ref().map(|d| d.as_str()),
        Some("a-fresh"),
        "the lower id makes the tie fresh"
    );
    assert_eq!(unapplied(&r), [("b-fresh", Unapplied::Outranked)]);
}

#[test]
fn a_stronger_fresh_start_keeps_the_note_and_another_pitch_doesnt() {
    let n = name();
    let soup = vec![note(0, 4, "C-4", 0.9), note(0, 4, "E-4", 0.5)];
    let rules = MonoRules {
        skyline: Skyline::Lowest,
        ..MonoRules::default()
    };
    // A person's fresh start of C-4 keeps it against a model's drop.
    let r = monophonise_with(
        soup.clone(),
        &at(&n),
        &rules,
        &[
            as_model(person("m-drop", drop_of(0, 4, "C-4"))),
            person("p-fresh", retrigger_of(0, "C-4")),
        ],
    );
    assert_eq!(unapplied(&r), [("m-drop", Unapplied::Overruled)]);
    assert_eq!(r.notes[0].pitch.to_string(), "C-4");
    // A person's trim of E-4 on the same row says nothing about C-4: the model's drop of C-4 applies.
    let r = monophonise_with(
        soup,
        &at(&n),
        &rules,
        &[
            as_model(person("m-drop", drop_of(0, 4, "C-4"))),
            person("p-trim-e", trim_of(0, "E-4", 2)),
        ],
    );
    assert!(!unapplied(&r).contains(&("m-drop", Unapplied::Overruled)));
    assert!(r.notes.iter().all(|x| x.pitch.to_string() != "C-4"));
}

#[test]
fn the_strongest_keeps_its_slot_even_when_it_cant_apply() {
    let n = name();
    let soup = vec![note(0, 8, "C-4", 0.9), note(8, 4, "D-4", 0.9)];
    // A person's trim that would lengthen C-4 still holds the slot: the model's trim doesn't take it, and C-4 keeps
    // the rule's length, as `resolve` and then the rule would give.
    let overrides = [
        person("p-long", trim_of(0, "C-4", 10)),
        as_model(person("m-short", trim_of(0, "C-4", 2))),
    ];
    let r = monophonise_with(soup.clone(), &at(&n), &MonoRules::default(), &overrides);
    assert_eq!(shape(&r.notes)[0], (0, 8, "C-4".into()));
    assert_eq!(
        sorted(unapplied(&r)),
        [
            ("m-short", Unapplied::Overruled),
            ("p-long", Unapplied::Lengthens)
        ]
    );
    // A person's trim that would lengthen still keeps the note against a model's drop.
    let r = monophonise_with(
        soup,
        &at(&n),
        &MonoRules::default(),
        &[
            as_model(person("m-drop", drop_of(0, 8, "C-4"))),
            person("p-long", trim_of(0, "C-4", 10)),
        ],
    );
    assert_eq!(
        sorted(unapplied(&r)),
        [
            ("m-drop", Unapplied::Overruled),
            ("p-long", Unapplied::Lengthens)
        ]
    );
    assert_eq!(r.notes[0].pitch.to_string(), "C-4");
}

#[test]
fn slots_are_per_note_and_a_second_fresh_start_loses_even_on_a_fresh_note() {
    let n = name();
    // Two drops on one row, of different notes: both apply.
    let soup = vec![
        note(0, 4, "C-4", 0.9),
        note(0, 4, "E-4", 0.9),
        note(0, 2, "C-4", 0.9),
    ];
    let r = monophonise_with(
        soup,
        &at(&n),
        &MonoRules::default(),
        &[
            person("no-e", drop_of(0, 4, "E-4")),
            person("no-short-c", drop_of(0, 2, "C-4")),
        ],
    );
    assert!(r.unapplied.is_empty(), "{:?}", r.unapplied);
    assert_eq!(shape(&r.notes), [(0, 4, "C-4".into())]);
    // A note that's already fresh: the first fresh start claims the slot (changing nothing), the second is outranked.
    let r = monophonise_with(
        vec![note(0, 4, "C-4", 0.9)],
        &at(&n),
        &MonoRules::default(),
        &[
            person("a-fresh", retrigger_of(0, "C-4")),
            person("b-fresh", retrigger_of(0, "C-4")),
        ],
    );
    assert_eq!(unapplied(&r), [("b-fresh", Unapplied::Outranked)]);
}

proptest! {
    /// Applying the overrides directly gives what `resolve` and then the rule give.
    #[test]
    fn overrides_apply_as_resolve_would_choose(
        soup in strategies::note_soup(),
        rules in any_rules(),
        picks in proptest::collection::vec((any::<proptest::sample::Index>(), 0u8..3, 1u32..12, any::<bool>()), 0..6),
    ) {
        prop_assume!(!soup.is_empty());
        let n = name();
        let overrides: Vec<Decision> = picks
            .iter()
            .enumerate()
            .map(|(k, (i, kind, len, model))| {
                let o = &soup[i.index(soup.len())];
                let pitch = o.pitch.to_string();
                let p = match kind {
                    0 => drop_of(o.row, o.length.get(), &pitch),
                    1 => trim_of(o.row, &pitch, *len),
                    _ => retrigger_of(o.row, &pitch),
                };
                let d = person(&format!("o{k}"), p);
                if *model { as_model(d) } else { d }
            })
            .collect();
        let direct = monophonise_with(soup.clone(), &at(&n), &rules, &overrides);
        let chosen = crate::decide::resolve(overrides).chosen;
        let resolved = monophonise_with(soup, &at(&n), &rules, &chosen);
        prop_assert_eq!(direct.notes, resolved.notes);
        prop_assert_eq!(direct.decisions, resolved.decisions);
    }
}

fn sorted<T>(mut v: Vec<(&str, T)>) -> Vec<(&str, T)> {
    v.sort_by_key(|(id, _)| *id);
    v
}
