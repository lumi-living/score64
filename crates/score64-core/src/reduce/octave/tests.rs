//! Folding lines into their roles' ranges, a bar at a time.

use std::collections::BTreeMap;
use std::num::{NonZeroU8, NonZeroU32};

use proptest::prelude::*;

use super::*;
use crate::profile::{ProfileName, builtin};
use crate::provenance::DecisionId;
use crate::score::{
    Attack, FramesPerRow, PresetId, RoleSpan, Schema, Song, Subtune, SubtuneName, Voice, VoiceLine,
};
use crate::strategies;
use crate::validate::{Context, InstrumentFacts, validate};

fn name() -> SubtuneName {
    SubtuneName::new("main").unwrap()
}

fn at(subtune: &SubtuneName) -> Placement<'_> {
    Placement {
        subtune,
        voice: Voice::new(2).unwrap(),
        rows_per_bar: NonZeroU8::new(16).unwrap(),
    }
}

fn note(row: u32, pitch: &str) -> Note {
    Note {
        row,
        length: NonZeroU32::new(4).unwrap(),
        pitch: pitch.parse().unwrap(),
        attack: Attack::Fresh,
        instrument: None,
        source: None,
        confidence: Confidence::CERTAIN,
        decision: None,
    }
}

fn pitches(r: &Reduced) -> Vec<String> {
    r.notes.iter().map(|n| n.pitch.to_string()).collect()
}

fn shifts(r: &Reduced) -> Vec<(u32, i8)> {
    r.decisions
        .iter()
        .map(|d| match d.proposal {
            Proposal::Octave { shift, .. } => (d.bar, shift),
            ref p => panic!("{p:?}"),
        })
        .collect()
}

#[test]
fn a_bass_line_an_octave_too_high_comes_down() {
    // Default bass range: C-1 to G-3. This line sits an octave above it.
    let n = name();
    let line = vec![note(0, "C-4"), note(4, "G-4"), note(8, "E-4")];
    let r = fold_octaves(
        line,
        &at(&n),
        &Roles::constant(Role::Bass),
        &RoleRanges::default(),
    );
    assert_eq!(pitches(&r), ["C-3", "G-3", "E-3"]);
    assert_eq!(shifts(&r), [(0, -1)]);
    let d = &r.decisions[0];
    assert_eq!(d.id.as_str(), "octave-v2-b0");
    assert_eq!(d.confidence, Confidence::CERTAIN);
    assert_eq!(
        d.provenance,
        Provenance::Rules {
            rule: "fold-octaves@1".into()
        }
    );
    assert!(r.notes.iter().all(|n| n.decision.as_ref() == Some(&d.id)));
}

#[test]
fn lines_in_range_are_left_alone() {
    let n = name();
    let line = vec![note(0, "C-4"), note(16, "E-5")];
    let r = fold_octaves(
        line.clone(),
        &at(&n),
        &Roles::constant(Role::Lead),
        &RoleRanges::default(),
    );
    assert_eq!(r.notes, line);
    assert!(r.decisions.is_empty());
}

#[test]
fn each_bar_gets_its_own_shift_and_keeps_its_shape() {
    // Lead range C-3 to C-6. Bar 0 is two octaves low; bar 1 fits only where it is (E-5 two octaves up is E-7, out);
    // bar 2 is an octave high.
    let n = name();
    let line = vec![
        note(0, "C-1"),
        note(4, "E-1"),
        note(16, "E-5"),
        note(32, "C-7"),
        note(36, "G-6"),
    ];
    let r = fold_octaves(
        line,
        &at(&n),
        &Roles::constant(Role::Lead),
        &RoleRanges::default(),
    );
    assert_eq!(pitches(&r), ["C-3", "E-3", "E-5", "C-6", "G-5"]);
    assert_eq!(shifts(&r), [(0, 2), (2, -1)]);
}

#[test]
fn a_bar_moves_only_to_bring_notes_in_and_then_follows_the_bar_before() {
    let n = name();
    // Bar 0 (C-7, G#7) only fits two octaves down. Bar 1's C-7 fits one, two or three octaves down: it follows bar 0
    // (two), keeping the line's shape across the bar line. Bar 2's E-5 already fits, and would still fit two octaves
    // down: it stays, because a bar moves only to bring notes in, not to follow the bar before.
    let line = vec![
        note(0, "C-7"),
        note(4, "G#7"),
        note(16, "C-7"),
        note(32, "E-5"),
    ];
    let r = fold_octaves(
        line,
        &at(&n),
        &Roles::constant(Role::Lead),
        &RoleRanges::default(),
    );
    assert_eq!(pitches(&r), ["C-5", "G#5", "C-5", "E-5"]);
    assert_eq!(shifts(&r), [(0, -2), (1, -2)]);
    assert!(
        r.decisions
            .iter()
            .all(|d| d.confidence == Confidence::CERTAIN)
    );
    // After a bar of rest, the shift is not carried over: the smallest that fits is taken.
    let line = vec![note(0, "C-7"), note(4, "G#7"), note(32, "C-7")];
    let r = fold_octaves(
        line,
        &at(&n),
        &Roles::constant(Role::Lead),
        &RoleRanges::default(),
    );
    assert_eq!(shifts(&r), [(0, -2), (2, -1)]);
}

#[test]
fn notes_that_cannot_fit_stay_and_lower_the_confidence() {
    // C-2 to C-6 can't both fit a lead's three octaves: the best shift fits one of the two.
    let n = name();
    let line = vec![note(0, "C-2"), note(4, "C-6")];
    let r = fold_octaves(
        line,
        &at(&n),
        &Roles::constant(Role::Lead),
        &RoleRanges::default(),
    );
    // Shifting up fits C-3 and pushes C-6 to C-7 (out); shifting nothing keeps C-2 out. Both leave one out: no shift.
    assert!(r.decisions.is_empty(), "{:?}", r.decisions);
    // Three notes an octave high and one far too low: down an octave fits the three, and C-1 goes out of reach.
    let line = vec![
        note(0, "C-1"),
        note(4, "A-6"),
        note(8, "B-6"),
        note(12, "C-7"),
    ];
    let r = fold_octaves(
        line,
        &at(&n),
        &Roles::constant(Role::Lead),
        &RoleRanges::default(),
    );
    assert_eq!(shifts(&r), [(0, -1)]);
    assert_eq!(r.decisions[0].confidence, Confidence::new(0.75).unwrap());
}

#[test]
fn drums_and_texture_never_move_and_roles_follow_the_row() {
    // Bass for bar 0, drums from bar 1: only bar 0 folds.
    let n = name();
    let roles = Roles::new(vec![
        RoleSpan {
            row: 0,
            role: Role::Bass,
        },
        RoleSpan {
            row: 16,
            role: Role::Drums,
        },
    ])
    .unwrap();
    let line = vec![note(0, "C-5"), note(16, "C-7"), note(20, "C-0")];
    let r = fold_octaves(line, &at(&n), &roles, &RoleRanges::default());
    assert_eq!(pitches(&r), ["C-3", "C-7", "C-0"]);
    assert_eq!(shifts(&r), [(0, -2)]);
}

#[test]
fn shifts_never_leave_goattrackers_range() {
    // G#7 is the top. Bass ends at G-3, so four octaves down (G#3) is still a semitone out: it takes five.
    let n = name();
    let r = fold_octaves(
        vec![note(0, "G#7")],
        &at(&n),
        &Roles::constant(Role::Bass),
        &RoleRanges::default(),
    );
    assert_eq!(pitches(&r), ["G#2"]);
    assert!(r.notes.iter().all(|n| n.pitch <= Pitch::MAX));
}

#[test]
fn continuity_holds_only_within_one_role() {
    // Bar 0 is a lead that must come down two octaves. Bar 1 is a harmony part (C-2 to C-5) whose C-7 fits two or
    // three octaves down; a new role does not follow the lead's shift, so it takes the smallest that fits.
    let n = name();
    let roles = Roles::new(vec![
        RoleSpan {
            row: 0,
            role: Role::Lead,
        },
        RoleSpan {
            row: 16,
            role: Role::Harmony,
        },
    ])
    .unwrap();
    let line = vec![note(0, "C-7"), note(4, "G#7"), note(16, "C-7")];
    let r = fold_octaves(line, &at(&n), &roles, &RoleRanges::default());
    assert_eq!(shifts(&r), [(0, -2), (1, -2)]);
    let same = fold_octaves(
        vec![note(0, "C-7"), note(4, "G#7"), note(16, "C-6")],
        &at(&n),
        &roles,
        &RoleRanges::default(),
    );
    // C-6 in harmony fits one or two octaves down (C-5, C-4): the smallest, one, not the lead's two.
    assert_eq!(shifts(&same), [(0, -2), (1, -1)]);
}

#[test]
fn of_two_equal_shifts_the_downward_one_is_taken() {
    // C-2 and C-6 around a C-3 to C-5 range: up an octave fits C-3 only, down an octave fits C-5 only. A tie: the
    // rule takes the lower shift.
    let n = name();
    let ranges = RoleRanges {
        lead: crate::validate::PitchRange {
            low: "C-3".parse().unwrap(),
            high: "C-5".parse().unwrap(),
        },
        ..RoleRanges::default()
    };
    let r = fold_octaves(
        vec![note(0, "C-2"), note(4, "C-6")],
        &at(&n),
        &Roles::constant(Role::Lead),
        &ranges,
    );
    assert_eq!(shifts(&r), [(0, -1)]);
    assert_eq!(pitches(&r), ["C-1", "C-5"]);
}

#[test]
fn bars_follow_the_subtunes_bar_length() {
    // Twelve rows a bar: rows 0 and 11 are bar 0, row 12 is bar 1.
    let n = name();
    let place = Placement {
        rows_per_bar: NonZeroU8::new(12).unwrap(),
        ..at(&n)
    };
    let line = vec![note(0, "C-5"), note(11, "D-5"), note(12, "C-2")];
    let r = fold_octaves(
        line,
        &place,
        &Roles::constant(Role::Bass),
        &RoleRanges::default(),
    );
    assert_eq!(pitches(&r), ["C-3", "D-3", "C-2"]);
    assert_eq!(shifts(&r), [(0, -2)]);
}

#[test]
fn full_range_notes_in_a_bar_do_not_count_towards_the_confidence() {
    // Bass then drums inside one bar: of the two bass notes only one can fit, so the confidence is a half, whatever
    // the drum note does.
    let n = name();
    let roles = Roles::new(vec![
        RoleSpan {
            row: 0,
            role: Role::Bass,
        },
        RoleSpan {
            row: 8,
            role: Role::Drums,
        },
    ])
    .unwrap();
    let line = vec![note(0, "C-1"), note(4, "C-6"), note(8, "C-7")];
    let r = fold_octaves(line, &at(&n), &roles, &RoleRanges::default());
    assert_eq!(
        pitches(&r),
        ["C-1", "C-6", "C-7"],
        "both bass shifts leave one out: no move"
    );
    // C-5 and G#7 are 32 semitones apart, one more than the bass range spans: two octaves down fits C-5 alone.
    let line = vec![note(0, "C-5"), note(4, "G#7"), note(8, "C-7")];
    let r = fold_octaves(line, &at(&n), &roles, &RoleRanges::default());
    assert_eq!(pitches(&r), ["C-3", "G#5", "C-7"]);
    assert_eq!(r.decisions[0].confidence, Confidence::new(0.5).unwrap());
}

#[test]
fn a_narrowed_drum_range_folds_drums_too() {
    // The exemption follows the ranges, as rule 6 does: a project that narrows the drums' range gets them folded.
    let n = name();
    let mut ranges = RoleRanges::default();
    ranges.drums = ranges.bass;
    let r = fold_octaves(
        vec![note(0, "C-6")],
        &at(&n),
        &Roles::constant(Role::Drums),
        &ranges,
    );
    assert_eq!(pitches(&r), ["C-3"]);
}

#[test]
fn unsorted_input_folds_by_bar() {
    // Rows 0, 20, 4: bar 0 is still one bar, with one decision.
    let n = name();
    let line = vec![note(0, "C-5"), note(20, "C-2"), note(4, "D-5")];
    let r = fold_octaves(
        line,
        &at(&n),
        &Roles::constant(Role::Bass),
        &RoleRanges::default(),
    );
    assert_eq!(
        r.notes.iter().map(|n| n.row).collect::<Vec<_>>(),
        [0, 4, 20]
    );
    assert_eq!(shifts(&r), [(0, -2)]);
}

#[test]
fn an_earlier_rules_decision_id_is_kept() {
    let n = name();
    let mut first = note(0, "C-5");
    first.decision = Some(DecisionId::new("mono-v2-0").unwrap());
    let r = fold_octaves(
        vec![first, note(4, "D-5")],
        &at(&n),
        &Roles::constant(Role::Bass),
        &RoleRanges::default(),
    );
    assert_eq!(r.notes[0].decision.as_ref().unwrap().as_str(), "mono-v2-0");
    assert_eq!(
        r.notes[1].decision.as_ref().unwrap().as_str(),
        "octave-v2-b0"
    );
}

/// Notes of each bar outside their role's range.
fn outside_per_bar(notes: &[Note], roles: &Roles, ranges: &RoleRanges) -> BTreeMap<u32, usize> {
    let mut out = BTreeMap::new();
    for n in notes {
        let role = roles.at(n.row);
        if folds(ranges, role) {
            *out.entry(n.row / 16).or_default() += usize::from(!ranges.of(role).contains(n.pitch));
        }
    }
    out
}

fn song_with(line: VoiceLine) -> (Song, Context) {
    let song = Song {
        schema: Schema::default(),
        profile: ProfileName::new(builtin::FULL).unwrap(),
        subtunes: vec![Subtune {
            name: name(),
            profile: None,
            frames_per_row: FramesPerRow::new(6).unwrap(),
            tempo: vec![],
            rows_per_bar: NonZeroU8::new(16).unwrap(),
            length: NonZeroU32::new(512).unwrap(),
            loop_row: 0,
            marks: vec![],
            voices: vec![line],
        }],
    };
    let facts = InstrumentFacts {
        filter: false,
        pulse_at_start: true,
        gate_timer: 2,
        hard_restart: true,
        gate_off: true,
    };
    let mut ctx = Context::default();
    for n in &song.subtunes[0].voices[0].notes {
        if let Some(i) = &n.instrument {
            ctx.instruments.insert(i.clone(), facts);
        }
    }
    ctx.instruments.insert(PresetId("line".into()), facts);
    (song, ctx)
}

proptest! {
    /// Folding changes only pitches, by whole octaves, keeps every pitch playable, keeps the line valid, never adds
    /// out-of-range notes to a bar, and picks each bar's best shift.
    #[test]
    fn folding_only_moves_pitches_and_never_makes_a_bar_worse(
        line in strategies::notes(),
        roles in strategies::roles(),
        reverse in any::<bool>(),
    ) {
        let n = name();
        let ranges = RoleRanges::default();
        // Fed in either order; the result is in row order.
        let input = if reverse { line.iter().rev().cloned().collect() } else { line.clone() };
        let r = fold_octaves(input, &at(&n), &roles, &ranges);
        prop_assert_eq!(r.notes.len(), line.len());
        for (a, b) in line.iter().zip(&r.notes) {
            prop_assert_eq!((a.row, a.length, a.attack, &a.instrument, a.source, a.confidence),
                (b.row, b.length, b.attack, &b.instrument, b.source, b.confidence));
            let moved = i16::from(b.pitch.semitone()) - i16::from(a.pitch.semitone());
            prop_assert_eq!(moved % 12, 0);
            prop_assert!(moved == 0 || folds(&ranges, roles.at(a.row)), "full-range roles never move");
        }
        let before = outside_per_bar(&line, &roles, &ranges);
        let after = outside_per_bar(&r.notes, &roles, &ranges);
        for (bar, out) in &after {
            prop_assert!(*out <= before[bar], "bar {bar}: {out} out after, {} before", before[bar]);
            // No other shift of this bar's folded notes would leave fewer out.
            let folded: Vec<&Note> = line
                .iter()
                .filter(|n| n.row / 16 == *bar && folds(&ranges, roles.at(n.row)))
                .collect();
            for k in -MAX_SHIFT..=MAX_SHIFT {
                let alt: Option<usize> = folded.iter().try_fold(0, |o, n| {
                    let p = shifted(n.pitch, k)?;
                    Some(o + usize::from(!ranges.of(roles.at(n.row)).contains(p)))
                });
                if let Some(alt) = alt {
                    prop_assert!(*out <= alt, "bar {bar}: shift {k} leaves {alt} out, the rule {out}");
                }
            }
        }
        // One decision per shifted bar, each naming this voice, with the shift its notes got.
        let mut bars: Vec<u32> = r.decisions.iter().map(|d| d.bar).collect();
        bars.dedup();
        prop_assert_eq!(bars.len(), r.decisions.len(), "one decision per bar");
        for d in &r.decisions {
            let Proposal::Octave { voice, shift } = d.proposal else {
                return Err(TestCaseError::fail(format!("{:?}", d.proposal)));
            };
            prop_assert!(voice == at(&n).voice && shift != 0);
            for (a, b) in line.iter().zip(&r.notes) {
                if a.row / 16 == d.bar && folds(&ranges, roles.at(a.row)) {
                    let moved = i16::from(b.pitch.semitone()) - i16::from(a.pitch.semitone());
                    prop_assert_eq!(moved, 12 * i16::from(shift));
                }
            }
        }
        let (song, ctx) = song_with(VoiceLine {
            voice: Voice::new(2).unwrap(),
            roles: roles.clone(),
            instrument: PresetId("line".into()),
            notes: r.notes,
            arps: vec![],
        });
        let errors: Vec<_> = validate(&song, &ctx)
            .into_iter()
            .filter(|p| p.is_error())
            .collect();
        prop_assert!(errors.is_empty(), "{errors:#?}");
    }
}

/// A person's octave shift for voice 2 in `bar`.
fn forced(id: &str, bar: u32, shift: i8) -> Decision {
    Decision {
        id: DecisionId::new(id).unwrap(),
        subtune: name(),
        bar,
        proposal: Proposal::Octave {
            voice: Voice::new(2).unwrap(),
            shift,
        },
        provenance: Provenance::Manual,
        confidence: Confidence::CERTAIN,
        reviewed: false,
    }
}

#[test]
fn a_shift_in_force_replaces_the_rules_choice_for_its_bar() {
    // The rule would take this bass line down an octave in bar 0, and carry that into bar 1.
    let n = name();
    let line = vec![note(0, "C-4"), note(4, "G-4"), note(16, "D-4")];
    let bass = Roles::constant(Role::Bass);
    let plain = fold_octaves(line.clone(), &at(&n), &bass, &RoleRanges::default());
    assert_eq!(pitches(&plain), ["C-3", "G-3", "D-3"]);
    // A person keeps bar 0 where it is; bar 1 is still the rule's.
    let r = fold_octaves_with(
        line.clone(),
        &at(&n),
        &bass,
        &RoleRanges::default(),
        &[forced("keep", 0, 0)],
    );
    assert_eq!(pitches(&r), ["C-4", "G-4", "D-3"]);
    assert_eq!(shifts(&r), [(1, -1)], "no rule decision for bar 0");
    assert_eq!(
        r.notes[0].decision.as_ref().map(|d| d.as_str()),
        Some("keep")
    );
    assert!(r.unapplied.is_empty());
    // Two down in bar 0: the rule carries it into bar 1, where it fits as well as one down.
    let r = fold_octaves_with(
        line,
        &at(&n),
        &bass,
        &RoleRanges::default(),
        &[forced("low", 0, -2)],
    );
    assert_eq!(pitches(&r), ["C-2", "G-2", "D-2"]);
    assert_eq!(shifts(&r), [(1, -2)]);
}

#[test]
fn a_shift_that_cant_apply_says_why() {
    let n = name();
    let line = vec![note(0, "C-4"), note(16, "C-4")];
    let lead = Roles::constant(Role::Lead);
    let r = fold_octaves_with(
        line.clone(),
        &at(&n),
        &lead,
        &RoleRanges::default(),
        &[forced("too-high", 0, 5), forced("empty-bar", 3, 1)],
    );
    // C-4 five octaves up is past G#7; bar 3 has no notes.
    let reasons: Vec<_> = r
        .unapplied
        .iter()
        .map(|(id, w)| (id.as_str(), *w))
        .collect();
    assert_eq!(
        reasons,
        [
            ("empty-bar", Unapplied::NoSuchNote),
            ("too-high", Unapplied::OutOfRange)
        ]
    );
    assert_eq!(
        r.notes,
        fold_octaves(line, &at(&n), &lead, &RoleRanges::default()).notes
    );
}

#[test]
fn only_this_voices_and_not_the_rules_own() {
    let n = name();
    let line = vec![note(0, "C-4")];
    let bass = Roles::constant(Role::Bass);
    let mut by_rules = forced("by-rules", 0, 0);
    by_rules.provenance = Provenance::Rules {
        rule: FOLD_OCTAVES.into(),
    };
    let mut other_voice = forced("voice-1", 0, 0);
    other_voice.proposal = Proposal::Octave {
        voice: Voice::new(1).unwrap(),
        shift: 0,
    };
    let r = fold_octaves_with(
        line.clone(),
        &at(&n),
        &bass,
        &RoleRanges::default(),
        &[by_rules, other_voice],
    );
    assert_eq!(
        r,
        fold_octaves(line, &at(&n), &bass, &RoleRanges::default())
    );
}

fn by_model(mut d: Decision) -> Decision {
    d.provenance = Provenance::Llm {
        model: "any".into(),
        prompt: crate::provenance::ContentHash([1; 32]),
    };
    d
}

#[test]
fn of_several_shifts_for_a_bar_the_strongest_that_fits_wins_in_any_order() {
    let n = name();
    let line = vec![note(0, "C-4"), note(4, "G-4")];
    let bass = Roles::constant(Role::Bass);
    let run = |overrides: &[Decision]| {
        fold_octaves_with(
            line.clone(),
            &at(&n),
            &bass,
            &RoleRanges::default(),
            overrides,
        )
    };
    let reasons = |r: &Reduced| {
        r.unapplied
            .iter()
            .map(|(id, w)| (id.as_str().to_string(), *w))
            .collect::<Vec<_>>()
    };
    // A person's beats a model's whatever their order; of two people's, the lower id.
    for overrides in [
        vec![by_model(forced("a-model", 0, -2)), forced("b-person", 0, 0)],
        vec![forced("b-person", 0, 0), by_model(forced("a-model", 0, -2))],
    ] {
        let r = run(&overrides);
        assert_eq!(pitches(&r), ["C-4", "G-4"]);
        assert_eq!(reasons(&r), [("a-model".to_string(), Unapplied::Overruled)]);
    }
    for overrides in [
        vec![forced("a", 0, 0), forced("b", 0, -2)],
        vec![forced("b", 0, -2), forced("a", 0, 0)],
    ] {
        let r = run(&overrides);
        assert_eq!(pitches(&r), ["C-4", "G-4"]);
        assert_eq!(reasons(&r), [("b".to_string(), Unapplied::Outranked)]);
    }
    // The strongest can't apply (five octaves up is past G#7), so the next one does.
    let r = run(&[forced("a-high", 0, 5), forced("b-low", 0, -2)]);
    assert_eq!(pitches(&r), ["C-2", "G-2"]);
    assert_eq!(reasons(&r), [("a-high".to_string(), Unapplied::OutOfRange)]);
}

#[test]
fn a_shift_that_cant_apply_leaves_the_bar_to_the_rule() {
    let n = name();
    // One note could go up an octave, the other (G-7) can't: the whole bar's shift is refused, and the rule folds
    // the bar as it would have, down an octave into the bass range.
    let line = vec![note(0, "C-4"), note(4, "G-7")];
    let bass = Roles::constant(Role::Bass);
    let plain = fold_octaves(line.clone(), &at(&n), &bass, &RoleRanges::default());
    let r = fold_octaves_with(
        line,
        &at(&n),
        &bass,
        &RoleRanges::default(),
        &[forced("up", 0, 1)],
    );
    assert_eq!(
        r.unapplied,
        [(DecisionId::new("up").unwrap(), Unapplied::OutOfRange)]
    );
    assert_eq!(r.notes, plain.notes);
    assert_eq!(r.decisions, plain.decisions);
    // Another subtune's override is not this call's.
    let mut elsewhere = forced("elsewhere", 0, 2);
    elsewhere.subtune = SubtuneName::new("combat").unwrap();
    let line = vec![note(0, "C-4")];
    let r = fold_octaves_with(
        line.clone(),
        &at(&n),
        &bass,
        &RoleRanges::default(),
        &[elsewhere],
    );
    assert_eq!(
        r,
        fold_octaves(line, &at(&n), &bass, &RoleRanges::default())
    );
}

#[test]
fn a_shift_in_force_still_gives_a_valid_line() {
    let n = name();
    let mut line_notes = vec![note(0, "C-4"), note(4, "G-4"), note(16, "D-4")];
    for x in &mut line_notes {
        x.instrument = None;
    }
    let roles = Roles::constant(Role::Bass);
    let r = fold_octaves_with(
        line_notes,
        &at(&n),
        &roles,
        &RoleRanges::default(),
        &[forced("low", 0, -2)],
    );
    let (song, ctx) = song_with(VoiceLine {
        voice: Voice::new(2).unwrap(),
        roles,
        instrument: PresetId("line".into()),
        notes: r.notes,
        arps: vec![],
    });
    let errors: Vec<_> = validate(&song, &ctx)
        .into_iter()
        .filter(|p| p.is_error())
        .collect();
    assert!(errors.is_empty(), "{errors:#?}");
}
