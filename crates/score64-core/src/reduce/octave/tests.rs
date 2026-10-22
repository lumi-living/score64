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
        &Roles::constant(Role::Bass),
        &RoleRanges::default(),
        &at(&n),
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
        &Roles::constant(Role::Lead),
        &RoleRanges::default(),
        &at(&n),
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
        &Roles::constant(Role::Lead),
        &RoleRanges::default(),
        &at(&n),
    );
    assert_eq!(pitches(&r), ["C-3", "E-3", "E-5", "C-6", "G-5"]);
    assert_eq!(shifts(&r), [(0, 2), (2, -1)]);
}

#[test]
fn a_tie_keeps_the_previous_bars_shift() {
    // Bar 0 must come down an octave. Bar 1's C-5 fits at 0 and at -1: it follows bar 0 down, so the line keeps its
    // shape across the bar line.
    let n = name();
    let line = vec![note(0, "C-7"), note(16, "C-5")];
    let r = fold_octaves(
        line,
        &Roles::constant(Role::Lead),
        &RoleRanges::default(),
        &at(&n),
    );
    assert_eq!(pitches(&r), ["C-6", "C-4"]);
    assert_eq!(shifts(&r), [(0, -1), (1, -1)]);
}

#[test]
fn notes_that_cannot_fit_stay_and_lower_the_confidence() {
    // C-2 to C-6 can't both fit a lead's three octaves: the best shift fits one of the two.
    let n = name();
    let line = vec![note(0, "C-2"), note(4, "C-6")];
    let r = fold_octaves(
        line,
        &Roles::constant(Role::Lead),
        &RoleRanges::default(),
        &at(&n),
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
        &Roles::constant(Role::Lead),
        &RoleRanges::default(),
        &at(&n),
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
    let r = fold_octaves(line, &roles, &RoleRanges::default(), &at(&n));
    assert_eq!(pitches(&r), ["C-3", "C-7", "C-0"]);
    assert_eq!(shifts(&r), [(0, -2)]);
}

#[test]
fn shifts_never_leave_goattrackers_range() {
    // G#7 is the top. Bass ends at G-3, so four octaves down (G#3) is still a semitone out: it takes five.
    let n = name();
    let r = fold_octaves(
        vec![note(0, "G#7")],
        &Roles::constant(Role::Bass),
        &RoleRanges::default(),
        &at(&n),
    );
    assert_eq!(pitches(&r), ["G#2"]);
    assert!(r.notes.iter().all(|n| n.pitch <= Pitch::MAX));
}

#[test]
fn an_earlier_rules_decision_id_is_kept() {
    let n = name();
    let mut first = note(0, "C-5");
    first.decision = Some(DecisionId::new("mono-v2-0").unwrap());
    let r = fold_octaves(
        vec![first, note(4, "D-5")],
        &Roles::constant(Role::Bass),
        &RoleRanges::default(),
        &at(&n),
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
        if folds(role) {
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
    ) {
        let n = name();
        let ranges = RoleRanges::default();
        let r = fold_octaves(line.clone(), &roles, &ranges, &at(&n));
        prop_assert_eq!(r.notes.len(), line.len());
        for (a, b) in line.iter().zip(&r.notes) {
            prop_assert_eq!((a.row, a.length, a.attack, &a.instrument, a.source, a.confidence),
                (b.row, b.length, b.attack, &b.instrument, b.source, b.confidence));
            let moved = i16::from(b.pitch.semitone()) - i16::from(a.pitch.semitone());
            prop_assert_eq!(moved % 12, 0);
            prop_assert!(moved == 0 || folds(roles.at(a.row)), "drums and texture never move");
        }
        let before = outside_per_bar(&line, &roles, &ranges);
        let after = outside_per_bar(&r.notes, &roles, &ranges);
        for (bar, out) in &after {
            prop_assert!(*out <= before[bar], "bar {bar}: {out} out after, {} before", before[bar]);
            // No other shift of this bar's folded notes would leave fewer out.
            let folded: Vec<&Note> = line
                .iter()
                .filter(|n| n.row / 16 == *bar && folds(roles.at(n.row)))
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
        // One decision per shifted bar, each naming this voice.
        for d in &r.decisions {
            let ok = matches!(d.proposal, Proposal::Octave { voice, shift } if voice == at(&n).voice && shift != 0);
            prop_assert!(ok);
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
