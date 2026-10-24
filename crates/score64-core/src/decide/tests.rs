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
        Proposal::KeepNote {
            voice: voice(1),
            row: 4,
            pitch: p,
        },
    ];
    let decisions: Vec<_> = kinds
        .iter()
        .enumerate()
        .map(|(i, k)| decision(&format!("d{i}"), 0, k.clone(), rules()))
        .collect();
    // Ten kinds, ten slots: even the trim, drop, retrigger and keep of one note are about different things.
    let r = resolve(decisions.clone());
    assert_eq!(r.chosen.len(), 10);
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

fn keep_of(pitch: &str) -> Proposal {
    Proposal::KeepNote {
        voice: voice(1),
        row: 4,
        pitch: pitch.parse().unwrap(),
    }
}

fn drop_of(pitch: &str, length: u32, reason: DropReason) -> Proposal {
    Proposal::DropNote {
        voice: voice(1),
        row: 4,
        pitch: pitch.parse().unwrap(),
        length: NonZeroU32::new(length).unwrap(),
        source: None,
        reason,
    }
}

fn lost(r: &Resolution) -> Vec<(&str, &str)> {
    r.overruled
        .iter()
        .map(|(d, by)| (d.id.as_str(), by.as_str()))
        .collect()
}

/// ADR-014 §8: a keep and the drops of notes on its row and pitch are separate slots. Whether they conflict depends on
/// the notes there, so the reduction that applies them settles it; `resolve` keeps the strongest of each.
#[test]
fn a_keep_and_the_drops_of_its_note_are_separate_slots() {
    // A person's keep and the rules' drop, a model's keep and a person's drop: both are chosen.
    let r = resolve([
        decision(
            "rules-drop",
            0,
            drop_of("C-4", 2, DropReason::Ghost),
            rules(),
        ),
        decision("keep", 0, keep_of("C-4"), Provenance::Manual),
    ]);
    assert_eq!(ids(&r), ["rules-drop", "keep"]);
    assert!(r.overruled.is_empty());
    let r = resolve([
        decision("model-keep", 0, keep_of("C-4"), model()),
        decision(
            "drop",
            0,
            drop_of("C-4", 2, DropReason::Overlap),
            Provenance::Manual,
        ),
    ]);
    assert_eq!(ids(&r), ["drop", "model-keep"]);
    assert!(r.overruled.is_empty());
    // Keeps of one note compete with each other, and drops with the drops of their note's length, whatever the
    // other kind holds there.
    let r = resolve([
        decision("short", 0, drop_of("C-4", 1, DropReason::Repeat), rules()),
        decision("long", 0, drop_of("C-4", 4, DropReason::Ghost), model()),
        decision(
            "p-long",
            0,
            drop_of("C-4", 4, DropReason::Overlap),
            Provenance::Manual,
        ),
        decision("keep", 0, keep_of("C-4"), Provenance::Manual),
        decision("model-keep", 0, keep_of("C-4"), model()),
    ]);
    assert_eq!(ids(&r), ["short", "p-long", "keep"]);
    assert_eq!(
        sorted(lost(&r)),
        [("long", "p-long"), ("model-keep", "keep")]
    );
    // Of one rank, the lower id.
    let r = resolve([
        decision("b-keep", 0, keep_of("C-4"), Provenance::Manual),
        decision("a-keep", 0, keep_of("C-4"), Provenance::Manual),
    ]);
    assert_eq!(lost(&r), [("b-keep", "a-keep")]);
    // A keep of another pitch, row, voice or subtune is another slot.
    let elsewhere = |id: &str, f: &dyn Fn(&mut Decision)| {
        let mut d = decision(id, 0, keep_of("C-4"), Provenance::Manual);
        f(&mut d);
        d
    };
    let r = resolve([
        elsewhere("keep", &|_| {}),
        elsewhere("pitch", &|d| d.proposal = keep_of("D-4")),
        elsewhere("row", &|d| {
            d.proposal = Proposal::KeepNote {
                voice: voice(1),
                row: 5,
                pitch: "C-4".parse().unwrap(),
            }
        }),
        elsewhere("voice", &|d| {
            d.proposal = Proposal::KeepNote {
                voice: voice(2),
                row: 4,
                pitch: "C-4".parse().unwrap(),
            }
        }),
        elsewhere("subtune", &|d| {
            d.subtune = SubtuneName::new("title").unwrap()
        }),
    ]);
    assert_eq!(r.chosen.len(), 5);
    assert!(r.overruled.is_empty());
}

fn sorted<'a>(mut v: Vec<(&'a str, &'a str)>) -> Vec<(&'a str, &'a str)> {
    v.sort();
    v
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
        (voice.clone(), 0u32..3, pitch.clone()).prop_map(|(voice, row, pitch)| {
            Proposal::Retrigger {
                voice,
                row,
                pitch,
                source: None,
            }
        }),
        (voice, 0u32..3, pitch).prop_map(|(voice, row, pitch)| Proposal::KeepNote {
            voice,
            row,
            pitch
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
        // A keep and the drops of notes on its row and pitch are separate slots: each is chosen exactly when it is
        // the strongest of its own, whatever the other kind holds there.
        for d in &decisions {
            if !matches!(d.proposal, Proposal::KeepNote { .. } | Proposal::DropNote { .. }) {
                continue;
            }
            let strongest = decisions
                .iter()
                .filter(|o| o.slot() == d.slot())
                .min_by(|a, b| strength(a, b))
                .unwrap();
            prop_assert_eq!(r.chosen.contains(d), strongest == d, "{:?}", d);
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

/// "main": two and a half bars of 16 rows (rows 0–39), and "combat", one bar.
fn song() -> Song {
    use crate::score::{FramesPerRow, Schema, Subtune};
    let sub = |name: &str, length: u32| Subtune {
        name: SubtuneName::new(name).unwrap(),
        profile: None,
        frames_per_row: FramesPerRow::new(6).unwrap(),
        tempo: vec![],
        rows_per_bar: std::num::NonZeroU8::new(16).unwrap(),
        length: NonZeroU32::new(length).unwrap(),
        loop_row: 0,
        marks: vec![],
        voices: vec![],
    };
    Song {
        schema: Schema::default(),
        profile: crate::profile::ProfileName::new("full").unwrap(),
        subtunes: vec![sub("main", 40), sub("combat", 16)],
    }
}

/// Lead `lead` in "main" only.
fn parts(lead: ContentHash) -> BTreeMap<SubtuneName, Vec<ContentHash>> {
    BTreeMap::from([(subtune(), vec![lead])])
}

const LEAD: ContentHash = ContentHash([7; 32]);

fn why(d: Decision) -> Option<Unapplied> {
    let checked = check([d], &song(), &parts(LEAD));
    assert_eq!(checked.valid.len() + checked.unapplied.len(), 1);
    checked.unapplied.first().map(|(_, w)| *w)
}

fn trim(row: u32) -> Proposal {
    Proposal::TrimNote {
        voice: voice(1),
        row,
        pitch: "C-4".parse().unwrap(),
        length: NonZeroU32::new(2).unwrap(),
        source: None,
    }
}

fn dropping(row: u32) -> Proposal {
    Proposal::DropNote {
        voice: voice(1),
        row,
        pitch: "C-4".parse().unwrap(),
        length: NonZeroU32::new(2).unwrap(),
        source: None,
        reason: DropReason::Overlap,
    }
}

fn fresh(row: u32) -> Proposal {
    Proposal::Retrigger {
        voice: voice(1),
        row,
        pitch: "C-4".parse().unwrap(),
        source: None,
    }
}

fn keep(row: u32) -> Proposal {
    Proposal::KeepNote {
        voice: voice(1),
        row,
        pitch: "C-4".parse().unwrap(),
    }
}

fn chord(row: u32, root: u8, intervals: &[u8]) -> Proposal {
    Proposal::Chord {
        row,
        root,
        intervals: intervals.to_vec(),
    }
}

#[test]
fn check_keeps_what_can_apply() {
    for (bar, proposal) in [
        (2, octave(-7)),
        (0, octave(7)),
        (1, Proposal::LeadSource { source: LEAD }),
        (1, chord(20, 11, &[0, 3, 7])),
        // The highest a chord can reach: B's lowest root (11) plus 81 is G#7.
        (0, chord(0, 11, &[0, 4, 81])),
        (2, Proposal::LoopRow { row: 39 }),
        (2, trim(39)),
        (1, dropping(16)),
        (0, fresh(15)),
        (2, keep(39)),
        (1, keep(16)),
        (
            0,
            Proposal::VoiceRole {
                voice: voice(2),
                role: Role::Bass,
            },
        ),
    ] {
        let d = decision("d", bar, proposal.clone(), Provenance::Manual);
        assert_eq!(why(d), None, "{proposal:?} at bar {bar}");
    }
}

#[test]
fn check_says_why_a_decision_cant_apply() {
    let other = |s: &str| Decision {
        subtune: SubtuneName::new(s).unwrap(),
        ..decision("d", 0, octave(1), Provenance::Manual)
    };
    assert_eq!(why(other("ending")), Some(Unapplied::UnknownSubtune));
    for (bar, proposal, want) in [
        (3, octave(1), Unapplied::PastEnd),
        (2, trim(40), Unapplied::PastEnd),
        (0, trim(16), Unapplied::WrongBar),
        (0, dropping(16), Unapplied::WrongBar),
        (1, fresh(15), Unapplied::WrongBar),
        (2, dropping(40), Unapplied::PastEnd),
        (0, keep(16), Unapplied::WrongBar),
        (2, keep(31), Unapplied::WrongBar),
        (2, keep(40), Unapplied::PastEnd),
        (3, keep(48), Unapplied::PastEnd),
        (1, Proposal::LoopRow { row: 8 }, Unapplied::WrongBar),
        (0, chord(16, 0, &[0, 4, 7]), Unapplied::WrongBar),
        (2, chord(40, 0, &[0, 4, 7]), Unapplied::PastEnd),
        (0, octave(8), Unapplied::OutOfRange),
        (0, octave(-8), Unapplied::OutOfRange),
        (0, chord(0, 12, &[0, 4, 7]), Unapplied::OutOfRange),
        (0, chord(0, 0, &[4, 7]), Unapplied::OutOfRange),
        (0, chord(0, 0, &[0, 7, 4]), Unapplied::OutOfRange),
        (0, chord(0, 0, &[0, 4, 4]), Unapplied::OutOfRange),
        (0, chord(0, 0, &[]), Unapplied::OutOfRange),
        (0, chord(0, 11, &[0, 4, 82]), Unapplied::OutOfRange),
        (0, chord(0, 11, &[0, 4, 200]), Unapplied::OutOfRange),
        (
            0,
            Proposal::LeadSource {
                source: ContentHash([8; 32]),
            },
            Unapplied::UnknownSource,
        ),
    ] {
        let d = decision("d", bar, proposal.clone(), Provenance::Manual);
        assert_eq!(why(d), Some(want), "{proposal:?} at bar {bar}");
    }
}

#[test]
fn a_lead_source_must_be_one_of_its_own_subtunes_parts() {
    // LEAD is a part of "main", not of "combat", which has no parts listed at all.
    let lead_in = |s: &str| Decision {
        subtune: SubtuneName::new(s).unwrap(),
        ..decision(
            "lead-b0",
            0,
            Proposal::LeadSource { source: LEAD },
            Provenance::Manual,
        )
    };
    let checked = check([lead_in("main"), lead_in("combat")], &song(), &parts(LEAD));
    assert_eq!(checked.valid, [lead_in("main")]);
    // The same id in two subtunes: the report says which one.
    assert_eq!(
        checked.unapplied,
        [(lead_in("combat"), Unapplied::UnknownSource)]
    );
}

/// #168's review: a person's override that can't apply must not keep a model's valid one out of the slot.
#[test]
fn an_invalid_stronger_decision_no_longer_blocks_a_valid_weaker_one() {
    let decisions = [
        decision(
            "by-person",
            1,
            Proposal::LeadSource {
                source: ContentHash([9; 32]),
            },
            Provenance::Manual,
        ),
        decision(
            "by-model",
            1,
            Proposal::LeadSource { source: LEAD },
            model(),
        ),
    ];
    // Unchecked, the person's unknown source wins the slot.
    assert_eq!(ids(&resolve(decisions.clone())), ["by-person"]);
    let checked = check(decisions.clone(), &song(), &parts(LEAD));
    assert_eq!(
        checked.unapplied,
        [(decisions[0].clone(), Unapplied::UnknownSource)]
    );
    assert_eq!(ids(&resolve(checked.valid)), ["by-model"]);
}

#[test]
fn unapplied_reasons_keep_their_names() {
    let names: Vec<String> = [
        Unapplied::UnknownSource,
        Unapplied::PastEnd,
        Unapplied::Overruled,
        Unapplied::Outranked,
        Unapplied::UnknownSubtune,
        Unapplied::WrongBar,
        Unapplied::OutOfRange,
        Unapplied::NoSuchNote,
        Unapplied::Lengthens,
        Unapplied::InsideNote,
    ]
    .iter()
    .map(|u| serde_json::to_string(u).unwrap())
    .collect();
    assert_eq!(
        names,
        [
            "\"unknown_source\"",
            "\"past_end\"",
            "\"overruled\"",
            "\"outranked\"",
            "\"unknown_subtune\"",
            "\"wrong_bar\"",
            "\"out_of_range\"",
            "\"no_such_note\"",
            "\"lengthens\"",
            "\"inside_note\"",
        ]
    );
}

type Sources = BTreeMap<SubtuneName, Vec<ContentHash>>;

/// Decisions about `song`: random proposals placed in its own subtunes, bars and rows (some past the end), with lead
/// sources from a small pool, some of which are the subtunes' parts.
fn about(song: Song) -> impl Strategy<Value = (Song, Vec<Decision>, Sources)> {
    let names: Vec<SubtuneName> = song.subtunes.iter().map(|s| s.name.clone()).collect();
    let pool: Vec<ContentHash> = (1..=4).map(|k| ContentHash([k; 32])).collect();
    let n = names.len();
    (
        Just(song),
        prop::collection::vec(
            (strategies::decision(), 0..n, 0u32..8, 0u32..20, 0usize..4),
            0..12,
        ),
        prop::collection::vec(prop::collection::vec(0usize..4, 0..3), n),
    )
        .prop_map(move |(song, raw, owned)| {
            let decisions = raw
                .into_iter()
                .map(|(mut d, s, bar, offset, k)| {
                    d.subtune = names[s].clone();
                    d.bar = bar;
                    let row = bar * 16 + offset;
                    match &mut d.proposal {
                        Proposal::Chord { row: r, .. }
                        | Proposal::LoopRow { row: r }
                        | Proposal::TrimNote { row: r, .. }
                        | Proposal::DropNote { row: r, .. }
                        | Proposal::Retrigger { row: r, .. }
                        | Proposal::KeepNote { row: r, .. } => *r = row,
                        Proposal::LeadSource { source } => *source = pool[k],
                        _ => {}
                    }
                    d
                })
                .collect();
            let sources = names
                .iter()
                .zip(owned)
                .map(|(name, ks)| (name.clone(), ks.into_iter().map(|k| pool[k]).collect()))
                .collect();
            (song, decisions, sources)
        })
}

proptest! {
    /// Every decision comes back once, valid or with its reason, in input order.
    #[test]
    fn check_partitions_its_input((song, decisions, sources) in strategies::song().prop_flat_map(about)) {
        let checked = check(decisions.clone(), &song, &sources);
        let (mut v, mut u) = (0, 0);
        for d in &decisions {
            if checked.valid.get(v) == Some(d) {
                v += 1;
            } else if checked.unapplied.get(u).map(|(x, _)| x) == Some(d) {
                u += 1;
            } else {
                prop_assert!(false, "{:?} is missing or out of order", d.id);
            }
        }
        prop_assert_eq!((v, u), (checked.valid.len(), checked.unapplied.len()));
    }
}

/// Whatever the rules decide about a song passes `check`, so a review never shows a rule's own decision as one that
/// can't apply.
mod rules_pass_check {
    use std::num::NonZeroU8;

    use super::*;
    use crate::reduce::*;
    use crate::score::{Note, Roles};
    use crate::transcription::{QuantiseReport, QuantisedPart};
    use crate::validate::RoleRanges;

    fn part(source: ContentHash, notes: Vec<Note>) -> QuantisedPart {
        QuantisedPart {
            source,
            notes,
            report: QuantiseReport {
                placed: 0,
                mean_start_error: 0.0,
                max_start_error: 0.0,
                loose: 0,
                stretched: 0,
                dropped: vec![],
            },
        }
    }

    proptest! {
        #[test]
        fn rules_decisions_pass_check(
            song in strategies::song(),
            soup in strategies::note_soup(),
            min_rows in 1u32..4,
            beat in 1u8..8,
        ) {
            let mut all: Vec<Decision> = Vec::new();
            let mut sources = Sources::new();
            for st in &song.subtunes {
                // Parts clipped to the subtune: notes that start inside it.
                let soup: Vec<Note> = soup.iter().filter(|n| n.row < st.length.get()).cloned().collect();
                let mut parts = Vec::new();
                let mut every = Vec::new();
                for (i, line) in st.voices.iter().enumerate() {
                    let at = Placement { subtune: &st.name, voice: line.voice, rows_per_bar: st.rows_per_bar };
                    let mono = MonoRules { min_rows: NonZeroU32::new(min_rows).unwrap(), ..MonoRules::default() };
                    let mut input = line.notes.clone();
                    input.extend(soup.iter().cloned());
                    all.extend(monophonise(input.clone(), &at, &mono).decisions);
                    let lowest = MonoRules { skyline: Skyline::Lowest, ..mono };
                    all.extend(monophonise(input, &at, &lowest).decisions);
                    all.extend(fold_octaves(line.notes.clone(), &at, &line.roles, &RoleRanges::default()).decisions);
                    all.extend(
                        fold_octaves(soup.clone(), &at, &Roles::constant(Role::Lead), &RoleRanges::default()).decisions,
                    );
                    all.extend(fold_drums(soup.clone(), &at, &DrumKit::default()).decisions);
                    all.extend(
                        interleave_drums(line.notes.clone(), soup.clone(), &at, &DrumKit::default(), &InterleaveRules::default())
                            .decisions,
                    );
                    let source = ContentHash([i as u8 + 1; 32]);
                    sources.entry(st.name.clone()).or_default().push(source);
                    parts.push(part(source, line.notes.clone()));
                    every.extend(line.notes.iter().cloned());
                    every.extend(soup.iter().cloned());
                    let spans = detect_chords(&every, NonZeroU8::new(beat).unwrap(), st.length);
                    all.extend(arpeggiate(&spans, &at, RoleRanges::default().arp).decisions);
                }
                let soup_source = ContentHash([0xEE; 32]);
                sources.entry(st.name.clone()).or_default().push(soup_source);
                parts.push(part(soup_source, soup.clone()));
                all.extend(choose_lead(&parts, Bars::from(st), &[], &LeadRules::default()).decisions);
                all.push(find_loop(st, &LoopRules::default()).decision);
                let eager = LoopRules { context: NonZeroU32::new(1).unwrap(), min_similarity: Confidence::new(0.0).unwrap() };
                all.push(find_loop(st, &eager).decision);
            }
            let checked = check(all.clone(), &song, &sources);
            let refused: Vec<_> = checked.unapplied.iter().take(3).map(|(d, w)| (w, d.bar, &d.proposal)).collect();
            prop_assert!(checked.unapplied.is_empty(), "{} of {} refused: {:?}", checked.unapplied.len(), all.len(), refused);
        }
    }
}
