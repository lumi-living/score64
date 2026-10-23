//! Applying tempo decisions.

use std::num::{NonZeroU8, NonZeroU32};

use super::*;
use crate::provenance::{Confidence, Provenance};
use crate::score::{FramesPerRow, SubtuneName};

fn subtune() -> Subtune {
    Subtune {
        name: SubtuneName::new("main").unwrap(),
        profile: None,
        frames_per_row: FramesPerRow::new(6).unwrap(),
        tempo: vec![TempoChange {
            row: 32,
            frames_per_row: FramesPerRow::new(4).unwrap(),
        }],
        rows_per_bar: NonZeroU8::new(16).unwrap(),
        length: NonZeroU32::new(64).unwrap(),
        loop_row: 0,
        marks: vec![],
        voices: vec![],
    }
}

fn tempo(id: &str, bar: u32, frames: u8, provenance: Provenance) -> Decision {
    Decision {
        id: DecisionId::new(id).unwrap(),
        subtune: SubtuneName::new("main").unwrap(),
        bar,
        proposal: Proposal::FramesPerRow {
            frames_per_row: FramesPerRow::new(frames).unwrap(),
        },
        provenance,
        confidence: Confidence::CERTAIN,
        reviewed: false,
    }
}

fn changes(s: &Subtune) -> Vec<(u32, u8)> {
    s.tempo
        .iter()
        .map(|t| (t.row, t.frames_per_row.get()))
        .collect()
}

#[test]
fn a_tempo_decision_sets_the_tempo_from_its_bar() {
    let mut s = subtune();
    let done = apply_tempo(
        &mut s,
        &[
            tempo("start", 0, 5, Provenance::Manual),
            tempo("faster", 3, 3, Provenance::Manual),
            tempo("replace", 2, 7, Provenance::Manual),
        ],
    );
    assert!(done.unapplied.is_empty());
    let applied: Vec<_> = done
        .applied
        .iter()
        .map(|(r, id)| (*r, id.as_str()))
        .collect();
    assert_eq!(applied, [(0, "start"), (32, "replace"), (48, "faster")]);
    assert_eq!(s.frames_per_row.get(), 5);
    assert_eq!(changes(&s), [(32, 7), (48, 3)]);
    // The frames the subtune now takes follow: 16 rows at 5, 16 at 5, 16 at 7, 16 at 3.
    assert_eq!(s.frames(0, 64), 16 * (5 + 5 + 7 + 3));
}

#[test]
fn a_tempo_decision_that_cant_apply_and_the_rules_own() {
    let mut s = subtune();
    let mut elsewhere = tempo("other", 1, 3, Provenance::Manual);
    elsewhere.subtune = SubtuneName::new("combat").unwrap();
    let unapplied = apply_tempo(
        &mut s,
        &[
            tempo("past", 4, 3, Provenance::Manual),
            tempo(
                "rules",
                1,
                3,
                Provenance::Rules {
                    rule: "any@1".into(),
                },
            ),
            elsewhere,
        ],
    );
    assert_eq!(
        unapplied.unapplied,
        [(DecisionId::new("past").unwrap(), Unapplied::PastEnd)]
    );
    assert!(unapplied.applied.is_empty());
    assert_eq!(s, subtune());
}

fn model() -> Provenance {
    Provenance::Llm {
        model: "any".into(),
        prompt: crate::provenance::ContentHash([1; 32]),
    }
}

#[test]
fn of_several_tempos_for_a_bar_the_strongest_wins_in_any_order() {
    for overrides in [
        vec![
            tempo("a-model", 1, 9, model()),
            tempo("b-person", 1, 8, Provenance::Manual),
        ],
        vec![
            tempo("b-person", 1, 8, Provenance::Manual),
            tempo("a-model", 1, 9, model()),
        ],
    ] {
        let mut s = subtune();
        let done = apply_tempo(&mut s, &overrides);
        assert_eq!(changes(&s), [(16, 8), (32, 4)]);
        assert_eq!(
            done.unapplied,
            [(DecisionId::new("a-model").unwrap(), Unapplied::Overruled)]
        );
    }
    let mut s = subtune();
    let done = apply_tempo(
        &mut s,
        &[
            tempo("b", 1, 9, Provenance::Manual),
            tempo("a", 1, 8, Provenance::Manual),
        ],
    );
    assert_eq!(changes(&s), [(16, 8), (32, 4)]);
    assert_eq!(
        done.unapplied,
        [(DecisionId::new("b").unwrap(), Unapplied::Outranked)]
    );
}

#[test]
fn a_tempo_already_playing_adds_no_change() {
    // 4 from row 32 already: a decision for 4 from row 48 is no change, and one for 6 at bar 1 (the start tempo) is
    // none either. Both count as applied.
    let mut s = subtune();
    let done = apply_tempo(
        &mut s,
        &[
            tempo("same", 3, 4, Provenance::Manual),
            tempo("start-again", 1, 6, Provenance::Manual),
        ],
    );
    assert_eq!(changes(&s), [(32, 4)]);
    assert_eq!(done.applied.len(), 2);
    // At bar 0, a stray change on row 0 can't outlive the new starting tempo.
    let mut s = subtune();
    s.tempo.insert(
        0,
        TempoChange {
            row: 0,
            frames_per_row: FramesPerRow::new(9).unwrap(),
        },
    );
    apply_tempo(&mut s, &[tempo("start", 0, 5, Provenance::Manual)]);
    assert_eq!((s.frames_per_row.get(), changes(&s)), (5, vec![(32, 4)]));
    assert_eq!(s.frames_per_row_at(0).get(), 5);
}

/// A tempo in force validates like any tempo: fine while every gate timer stays below it, and rule 9's problem when
/// one doesn't.
#[test]
fn a_tempo_in_force_is_checked_like_any_tempo() {
    use std::collections::BTreeMap;

    use crate::profile::{ProfileName, builtin};
    use crate::score::{Attack, Note, PresetId, Role, Roles, Schema, Song, Voice, VoiceLine};
    use crate::validate::{Context, InstrumentFacts, Rule, validate};

    let notes = (0..8)
        .map(|k| Note {
            row: k * 8,
            length: NonZeroU32::new(4).unwrap(),
            pitch: "C-4".parse().unwrap(),
            attack: Attack::Fresh,
            instrument: None,
            source: None,
            confidence: Confidence::CERTAIN,
            decision: None,
        })
        .collect();
    let mut s = subtune();
    s.voices = vec![VoiceLine {
        voice: Voice::new(1).unwrap(),
        roles: Roles::constant(Role::Lead),
        instrument: PresetId("lead".into()),
        notes,
        arps: vec![],
    }];
    let errors = |s: &Subtune, gate_timer: u8| {
        let song = Song {
            schema: Schema::default(),
            profile: ProfileName::new(builtin::FULL).unwrap(),
            subtunes: vec![s.clone()],
        };
        let facts = InstrumentFacts {
            filter: false,
            pulse_at_start: true,
            gate_timer,
            hard_restart: true,
            gate_off: true,
        };
        let ctx = Context {
            instruments: BTreeMap::from([(PresetId("lead".into()), facts)]),
            ..Context::default()
        };
        validate(&song, &ctx)
            .into_iter()
            .filter(|p| p.is_error())
            .map(|p| p.rule)
            .collect::<Vec<_>>()
    };
    apply_tempo(&mut s, &[tempo("fast", 1, 3, Provenance::Manual)]);
    assert_eq!(changes(&s), [(16, 3), (32, 4)]);
    assert_eq!(errors(&s, 2), [], "a gate timer of 2 is below 3");
    assert!(
        errors(&s, 3).contains(&Rule::HardRestart),
        "a gate timer of 3 is not"
    );
}
