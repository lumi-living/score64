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
    let unapplied = apply_tempo(
        &mut s,
        &[
            tempo("start", 0, 5, Provenance::Manual),
            tempo("faster", 3, 3, Provenance::Manual),
            tempo("replace", 2, 7, Provenance::Manual),
        ],
    );
    assert!(unapplied.is_empty());
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
        unapplied,
        [(DecisionId::new("past").unwrap(), Unapplied::PastEnd)]
    );
    assert_eq!(s, subtune());
}
