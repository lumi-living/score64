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
    assert_eq!(resolve([a.clone(), b.clone()]), resolve([b, a.clone()]));
    // Same id, rank and confidence, different proposals: still one answer.
    let c = decision("same", 0, octave(-2), rules());
    assert_eq!(resolve([a.clone(), c.clone()]), resolve([c, a]));
}

#[test]
fn a_persons_choice_holds_over_later_weaker_ones() {
    let lead = |id: &str, bar, provenance| {
        decision(
            id,
            bar,
            Proposal::LeadSource {
                source: ContentHash([bar as u8; 32]),
            },
            provenance,
        )
    };
    let r = resolve([
        lead("rules-early", 0, rules()),
        lead("person", 2, Provenance::Manual),
        lead("model-later", 8, model()),
        lead("rules-later", 12, rules()),
        lead("person-again", 16, Provenance::Manual),
    ]);
    // The rules hold until the person; the person holds over the later model and rules, until the person again.
    assert_eq!(ids(&r), ["rules-early", "person", "person-again"]);
    let lost: Vec<_> = r
        .overruled
        .iter()
        .map(|(d, by)| (d.id.as_str(), by.as_str()))
        .collect();
    assert_eq!(lost, [("model-later", "person"), ("rules-later", "person")]);
    // A model's later choice does take over from the rules'.
    let r = resolve([lead("rules-early", 0, rules()), lead("model", 4, model())]);
    assert_eq!(ids(&r), ["rules-early", "model"]);
}

#[test]
fn tempos_hold_over_time_apart_from_leads() {
    let tempo = |id: &str, bar, f, provenance| {
        decision(
            id,
            bar,
            Proposal::FramesPerRow {
                frames_per_row: crate::score::FramesPerRow::new(f).unwrap(),
            },
            provenance,
        )
    };
    let lead = decision(
        "lead-rules",
        4,
        Proposal::LeadSource {
            source: ContentHash([3; 32]),
        },
        rules(),
    );
    // A person's tempo at bar 0 holds over the rules' at bar 4; the rules' lead at bar 4 is another kind.
    let r = resolve([
        tempo("tempo-person", 0, 6, Provenance::Manual),
        tempo("tempo-rules", 4, 5, rules()),
        lead,
    ]);
    assert_eq!(ids(&r), ["lead-rules", "tempo-person"]);
    assert_eq!(r.overruled[0].0.id.as_str(), "tempo-rules");
    // Two people, then a model: the model loses to the later person, which is in force.
    let r = resolve([
        tempo("p1", 0, 6, Provenance::Manual),
        tempo("p2", 2, 5, Provenance::Manual),
        tempo("m", 4, 4, model()),
    ]);
    assert_eq!(r.overruled[0].1.as_str(), "p2");
}

#[test]
fn roles_hold_per_voice() {
    let role = |id: &str, bar, v, provenance| {
        decision(
            id,
            bar,
            Proposal::VoiceRole {
                voice: voice(v),
                role: Role::Bass,
            },
            provenance,
        )
    };
    // A person's role for voice 1 doesn't overrule the rules' role for voice 2 later.
    let r = resolve([
        role("v1-person", 0, 1, Provenance::Manual),
        role("v2-rules", 4, 2, rules()),
        role("v1-rules", 4, 1, rules()),
    ]);
    assert_eq!(ids(&r), ["v1-person", "v2-rules"]);
}

#[test]
fn every_slot_field_tells_decisions_apart() {
    let p = |s: &str| s.parse().unwrap();
    let len = |n| NonZeroU32::new(n).unwrap();
    let pairs = [
        (
            Proposal::VoiceRole {
                voice: voice(1),
                role: Role::Bass,
            },
            Proposal::VoiceRole {
                voice: voice(2),
                role: Role::Bass,
            },
        ),
        (
            Proposal::Retrigger {
                voice: voice(1),
                row: 4,
                pitch: p("C-4"),
                source: None,
            },
            Proposal::Retrigger {
                voice: voice(1),
                row: 4,
                pitch: p("D-4"),
                source: None,
            },
        ),
        (
            Proposal::TrimNote {
                voice: voice(1),
                row: 4,
                pitch: p("C-4"),
                length: len(2),
                source: None,
            },
            Proposal::TrimNote {
                voice: voice(1),
                row: 5,
                pitch: p("C-4"),
                length: len(2),
                source: None,
            },
        ),
        (
            Proposal::TrimNote {
                voice: voice(1),
                row: 4,
                pitch: p("C-4"),
                length: len(2),
                source: None,
            },
            Proposal::TrimNote {
                voice: voice(2),
                row: 4,
                pitch: p("C-4"),
                length: len(2),
                source: None,
            },
        ),
        (
            // Two repeats on one row, told apart by their lengths: the rules drop both.
            Proposal::DropNote {
                voice: voice(1),
                row: 4,
                pitch: p("C-4"),
                length: len(1),
                source: None,
                reason: DropReason::Repeat,
            },
            Proposal::DropNote {
                voice: voice(1),
                row: 4,
                pitch: p("C-4"),
                length: len(2),
                source: None,
                reason: DropReason::Repeat,
            },
        ),
        (
            octave(1),
            Proposal::Octave {
                voice: voice(2),
                shift: 1,
            },
        ),
    ];
    for (a, b) in pairs {
        let r = resolve([
            decision("a", 0, a.clone(), rules()),
            decision("b", 0, b.clone(), rules()),
        ]);
        assert_eq!(r.chosen.len(), 2, "{a:?} and {b:?} should not compete");
    }
}

/// Decisions that compete: two subtunes, a few bars, rows, voices, pitches and ids, every rank.
fn competing() -> impl Strategy<Value = Decision> {
    let voice = (1u8..=2).prop_map(|v| Voice::new(v).unwrap());
    let pitch = prop_oneof![Just("C-4"), Just("E-4")]
        .prop_map(|p| p.parse::<crate::score::Pitch>().unwrap());
    let length = (1u32..3).prop_map(|n| NonZeroU32::new(n).unwrap());
    let proposal = prop_oneof![
        (voice.clone(), Just(Role::Lead))
            .prop_map(|(voice, role)| Proposal::VoiceRole { voice, role }),
        (0u8..2).prop_map(|s| Proposal::LeadSource {
            source: ContentHash([s; 32])
        }),
        (voice.clone(), -1i8..=1).prop_map(|(voice, shift)| Proposal::Octave { voice, shift }),
        (0u32..3).prop_map(|row| Proposal::Chord {
            row,
            root: 0,
            intervals: vec![0, 4, 7]
        }),
        (0u32..3).prop_map(|row| Proposal::LoopRow { row }),
        (3u8..5).prop_map(|f| Proposal::FramesPerRow {
            frames_per_row: crate::score::FramesPerRow::new(f).unwrap()
        }),
        (voice.clone(), 0u32..3, pitch.clone(), length.clone()).prop_map(
            |(voice, row, pitch, length)| {
                Proposal::TrimNote {
                    voice,
                    row,
                    pitch,
                    length,
                    source: None,
                }
            }
        ),
        (voice.clone(), 0u32..3, pitch.clone(), length).prop_map(|(voice, row, pitch, length)| {
            Proposal::DropNote {
                voice,
                row,
                pitch,
                length,
                source: None,
                reason: DropReason::Repeat,
            }
        }),
        (voice, 0u32..3, pitch).prop_map(|(voice, row, pitch)| Proposal::Retrigger {
            voice,
            row,
            pitch,
            source: None
        }),
    ];
    let provenance = prop_oneof![Just(rules()), Just(model()), Just(Provenance::Manual)];
    (
        prop_oneof![Just("a"), Just("b"), Just("c")],
        prop_oneof![Just("main"), Just("title")],
        0u32..3,
        proposal,
        provenance,
        any::<bool>(),
        prop_oneof![Just(0.5f32), Just(0.9)],
    )
        .prop_map(
            |(id, sub, bar, proposal, provenance, reviewed, c)| Decision {
                id: DecisionId::new(id).unwrap(),
                subtune: SubtuneName::new(sub).unwrap(),
                bar,
                proposal,
                provenance,
                confidence: Confidence::new(c).unwrap(),
                reviewed,
            },
        )
}

proptest! {
    /// One decision per slot; every input is chosen or overruled; a person's decision is never overruled by anything
    /// weaker; and the order of the input does not matter.
    #[test]
    fn resolution_is_a_partition_by_slot(
        decisions in prop_oneof![
            proptest::collection::vec(strategies::decision(), 0..24),
            proptest::collection::vec(competing(), 0..24),
        ],
        seed in any::<u64>(),
    ) {
        let r = resolve(decisions.clone());
        prop_assert_eq!(r.chosen.len() + r.overruled.len(), decisions.len());
        let mut slots: Vec<_> = r.chosen.iter().map(Decision::slot).collect();
        let before = slots.len();
        slots.dedup();
        prop_assert_eq!(slots.len(), before, "one per slot, in slot order");
        prop_assert!(r.chosen.windows(2).all(|w| w[0].slot() < w[1].slot()));
        // The decision that beat each loser: in its slot or, for the kinds that hold from a bar, of its kind at the
        // same bar or earlier.
        let beat = |lost: &Decision, by: &DecisionId| {
            let same_slot = r.chosen.iter().find(|d| &d.id == by && d.slot() == lost.slot());
            same_slot.or_else(|| {
                // The latest of its kind at or before the loser's bar with that id.
                r.chosen.iter().rev().find(|d| {
                    &d.id == by
                        && match (d.slot().holds_from_bar(), lost.slot().holds_from_bar()) {
                            (Some((ws, wk, wv, wb)), Some((ls, lk, lv, lb))) => {
                                (ws, wk, wv) == (ls, lk, lv) && wb <= lb
                            }
                            _ => false,
                        }
                })
            })
        };
        // For the kinds that hold from a bar: what is in force at a bar is the latest chosen decision of the kind at
        // or before it. A chosen one is never weaker than what was in force before it; a loser not beaten in its own
        // slot names exactly that decision.
        let in_force_at = |lost: &Decision| {
            let (ls, lk, lv, lb) = lost.slot().holds_from_bar()?;
            r.chosen
                .iter()
                .filter(|d| {
                    d.slot().holds_from_bar().is_some_and(|(s, k, v, b)| (&s, k, v) == (&ls, lk, lv) && b <= lb)
                        && d.slot() != lost.slot()
                })
                .max_by_key(|d| d.bar)
        };
        for d in &r.chosen {
            if let Some(before) = in_force_at(d).filter(|b| b.bar < d.bar) {
                prop_assert!(before.rank() <= d.rank(), "{d:?} chosen though {before:?} was stronger and in force");
            }
        }
        for (lost, by) in &r.overruled {
            let same_slot = r.chosen.iter().any(|d| &d.id == by && d.slot() == lost.slot());
            if !same_slot {
                let in_force = in_force_at(lost);
                prop_assert!(in_force.is_some_and(|f| &f.id == by), "{lost:?} names {by:?}, not what is in force");
            }
        }
        for (lost, by) in &r.overruled {
            let winner = beat(lost, by);
            prop_assert!(winner.is_some(), "{by:?} did not win over {lost:?}");
            let winner = winner.unwrap();
            prop_assert!(winner.rank() >= lost.rank());
            // A person's decision is overruled only by another person's.
            if lost.rank() == Rank::Person {
                prop_assert_eq!(winner.rank(), Rank::Person);
            }
        }
        let mut shuffled = decisions;
        let mut x = seed | 1;
        for i in (1..shuffled.len()).rev() {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            shuffled.swap(i, (x % (i as u64 + 1)) as usize);
        }
        prop_assert_eq!(resolve(shuffled), r);
    }
}
