//! Which decision wins its slot.

use std::num::NonZeroU32;

use proptest::prelude::*;

use super::*;
use crate::provenance::{Confidence, ContentHash, DropReason};
use crate::score::Role;
use crate::strategies;

fn subtune() -> SubtuneName {
    SubtuneName::new("main").unwrap()
}

fn voice(n: u8) -> Voice {
    Voice::new(n).unwrap()
}

fn decision(id: &str, bar: u32, proposal: Proposal, provenance: Provenance) -> Decision {
    Decision {
        id: DecisionId::new(id).unwrap(),
        subtune: subtune(),
        bar,
        proposal,
        provenance,
        confidence: Confidence::new(0.7).unwrap(),
        reviewed: false,
    }
}

fn rules() -> Provenance {
    Provenance::Rules {
        rule: "any@1".into(),
    }
}

fn model() -> Provenance {
    Provenance::Llm {
        model: "any".into(),
        prompt: ContentHash([1; 32]),
    }
}

fn octave(shift: i8) -> Proposal {
    Proposal::Octave {
        voice: voice(1),
        shift,
    }
}

fn ids(r: &Resolution) -> Vec<&str> {
    r.chosen.iter().map(|d| d.id.as_str()).collect()
}

#[test]
fn a_person_beats_a_model_beats_the_rules() {
    let r = resolve([
        decision("by-rules", 0, octave(-1), rules()),
        decision("by-model", 0, octave(1), model()),
        decision("by-person", 0, octave(2), Provenance::Manual),
    ]);
    assert_eq!(ids(&r), ["by-person"]);
    let lost: Vec<_> = r
        .overruled
        .iter()
        .map(|(d, by)| (d.id.as_str(), by.as_str()))
        .collect();
    assert_eq!(lost, [("by-model", "by-person"), ("by-rules", "by-person")]);
    // Without the person, the model wins.
    let r = resolve([
        decision("by-rules", 0, octave(-1), rules()),
        decision("by-model", 0, octave(1), model()),
    ]);
    assert_eq!(ids(&r), ["by-model"]);
}

#[test]
fn a_reviewed_models_decision_counts_as_a_persons() {
    let mut accepted = decision("accepted", 0, octave(1), model());
    accepted.reviewed = true;
    assert_eq!(accepted.rank(), Rank::Person);
    // Of two of one rank, the lower id.
    let r = resolve([
        decision("z-person", 0, octave(2), Provenance::Manual),
        accepted,
    ]);
    assert_eq!(ids(&r), ["accepted"]);
}

#[test]
fn different_slots_all_apply() {
    let r = resolve([
        decision("bar0", 0, octave(1), rules()),
        decision("bar1", 1, octave(1), rules()),
        decision(
            "voice2",
            0,
            Proposal::Octave {
                voice: voice(2),
                shift: 1,
            },
            rules(),
        ),
        decision(
            "role",
            0,
            Proposal::VoiceRole {
                voice: voice(1),
                role: Role::Lead,
            },
            rules(),
        ),
    ]);
    assert_eq!(r.chosen.len(), 4);
    assert!(r.overruled.is_empty());
}

#[test]
fn every_kind_has_its_slot() {
    let p = "C-4".parse().unwrap();
    let length = NonZeroU32::new(2).unwrap();
    let kinds = [
        Proposal::VoiceRole {
            voice: voice(1),
            role: Role::Bass,
        },
        Proposal::LeadSource {
            source: ContentHash([2; 32]),
        },
        octave(1),
        Proposal::Chord {
            row: 4,
            root: 0,
            intervals: vec![0, 4, 7],
        },
        Proposal::LoopRow { row: 16 },
        Proposal::FramesPerRow {
            frames_per_row: crate::score::FramesPerRow::new(6).unwrap(),
        },
        Proposal::TrimNote {
            voice: voice(1),
            row: 4,
            pitch: p,
            length,
            source: None,
        },
        Proposal::DropNote {
            voice: voice(1),
            row: 4,
            pitch: p,
            length,
            source: None,
            reason: DropReason::Ghost,
        },
        Proposal::Retrigger {
            voice: voice(1),
            row: 4,
            pitch: p,
            source: None,
        },
    ];
    let decisions: Vec<_> = kinds
        .iter()
        .enumerate()
        .map(|(i, k)| decision(&format!("d{i}"), 0, k.clone(), rules()))
        .collect();
    // Nine kinds, nine slots: even the trim, drop and retrigger of one note are about different things.
    let r = resolve(decisions.clone());
    assert_eq!(r.chosen.len(), 9);
    // The loop row is the subtune's, whatever bar a decision names; the chord is its row's.
    let mut other_bar = decisions[4].clone();
    other_bar.bar = 3;
    other_bar.id = DecisionId::new("later").unwrap();
    assert_eq!(other_bar.slot(), decisions[4].slot());
    let mut chord_bar = decisions[3].clone();
    chord_bar.bar = 7;
    assert_eq!(chord_bar.slot(), decisions[3].slot());
    // Another subtune's is another slot.
    let mut elsewhere = decisions[4].clone();
    elsewhere.subtune = SubtuneName::new("title").unwrap();
    assert_ne!(elsewhere.slot(), decisions[4].slot());
}

#[test]
fn duplicates_resolve_the_same_whatever_their_order() {
    let a = decision("same", 0, octave(1), rules());
    let mut b = a.clone();
    b.confidence = Confidence::new(0.2).unwrap();
    let one = resolve([a.clone(), b.clone()]);
    let two = resolve([b, a]);
    assert_eq!(one, two);
}

proptest! {
    /// One decision per slot; every input is chosen or overruled; a person's decision is never overruled by anything
    /// weaker; and the order of the input does not matter.
    #[test]
    fn resolution_is_a_partition_by_slot(
        decisions in proptest::collection::vec(strategies::decision(), 0..24),
        reverse in any::<bool>(),
    ) {
        let r = resolve(decisions.clone());
        prop_assert_eq!(r.chosen.len() + r.overruled.len(), decisions.len());
        let mut slots: Vec<_> = r.chosen.iter().map(Decision::slot).collect();
        let before = slots.len();
        slots.dedup();
        prop_assert_eq!(slots.len(), before, "one per slot, in slot order");
        prop_assert!(r.chosen.windows(2).all(|w| w[0].slot() < w[1].slot()));
        for (lost, by) in &r.overruled {
            let winner = r.chosen.iter().find(|d| &d.id == by && d.slot() == lost.slot());
            prop_assert!(winner.is_some(), "{by:?} did not win {lost:?}'s slot");
            let winner = winner.unwrap();
            prop_assert!(
                (winner.rank(), std::cmp::Reverse(&winner.id)) >= (lost.rank(), std::cmp::Reverse(&lost.id)),
                "{winner:?} should not beat {lost:?}"
            );
        }
        let mut flipped = decisions;
        if reverse {
            flipped.reverse();
        }
        let again = resolve(flipped);
        prop_assert_eq!(&again.chosen, &r.chosen);
        prop_assert_eq!(again.overruled.len(), r.overruled.len());
    }
}
