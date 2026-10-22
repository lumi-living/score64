//! Choosing the lead part bar by bar, and how overrides win.

use std::num::NonZeroU32;

use proptest::prelude::*;

use super::*;
use crate::score::{Attack, Pitch};
use crate::strategies;
use crate::transcription::QuantiseReport;

const MELODY: ContentHash = ContentHash([1; 32]);
const BASS: ContentHash = ContentHash([2; 32]);
const HATS: ContentHash = ContentHash([3; 32]);
const SOLO: ContentHash = ContentHash([4; 32]);

fn name() -> SubtuneName {
    SubtuneName::new("main").unwrap()
}

fn bar() -> NonZeroU8 {
    NonZeroU8::new(16).unwrap()
}

fn note(row: u32, length: u32, semitone: u8) -> Note {
    Note {
        row,
        length: NonZeroU32::new(length).unwrap(),
        pitch: Pitch::new(semitone).unwrap(),
        attack: Attack::Fresh,
        instrument: None,
        source: None,
        confidence: Confidence::new(0.9).unwrap(),
        decision: None,
    }
}

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

/// A singable line over `bars` bars: four quarter notes a bar around C-5, in small steps.
fn melody(source: ContentHash, bars: std::ops::Range<u32>, base: u8) -> QuantisedPart {
    let notes = bars
        .flat_map(|b| (0..4).map(move |q| note(b * 16 + q * 4, 4, base + [0, 2, 4, 2][q as usize])))
        .collect();
    part(source, notes)
}

/// A low bass: two half notes a bar around C-2.
fn bass(bars: std::ops::Range<u32>) -> QuantisedPart {
    part(
        BASS,
        bars.flat_map(|b| [note(b * 16, 8, 24), note(b * 16 + 8, 8, 31)])
            .collect(),
    )
}

/// Busy hi-hats: sixteen short notes a bar, high.
fn hats(bars: std::ops::Range<u32>) -> QuantisedPart {
    part(
        HATS,
        bars.flat_map(|b| (0..16).map(move |r| note(b * 16 + r, 1, 84)))
            .collect(),
    )
}

fn leads(c: &LeadChoice) -> Vec<Option<ContentHash>> {
    c.leads.clone()
}

#[test]
fn a_singable_line_beats_bass_and_hats() {
    let parts = [bass(0..4), hats(0..4), melody(MELODY, 0..4, 60)];
    let c = choose_lead(&parts, &name(), bar(), &[], &LeadRules::default());
    assert_eq!(leads(&c), vec![Some(MELODY); 4]);
    // One decision, where the rules chose it, with a clear margin.
    assert_eq!(c.decisions.len(), 1);
    let d = &c.decisions[0];
    assert_eq!(d.id.as_str(), "lead-b0");
    assert_eq!(d.proposal, Proposal::LeadSource { source: MELODY });
    assert_eq!(
        d.provenance,
        Provenance::Rules {
            rule: "choose-lead@1".into()
        }
    );
    assert!(d.confidence.get() > 0.0, "{:?}", d.confidence);
    // The scores say why: the melody's density is ideal and it moves in small steps.
    let m = c.salience.iter().find(|s| s.source == MELODY).unwrap();
    assert_eq!(m.bars[0].density, 1.0);
    let h = c.salience.iter().find(|s| s.source == HATS).unwrap();
    assert!(h.bars[0].density < 0.3);
}

#[test]
fn the_lead_moves_only_when_another_part_holds_it() {
    // The melody plays bars 0–5; a higher solo takes over from bar 3, and plays one stray bar (1) before that.
    let mut solo = melody(SOLO, 3..6, 72);
    solo.notes.extend(melody(SOLO, 1..2, 72).notes);
    let parts = [melody(MELODY, 0..6, 60), solo];
    let c = choose_lead(&parts, &name(), bar(), &[], &LeadRules::default());
    assert_eq!(
        leads(&c),
        [
            Some(MELODY),
            Some(MELODY),
            Some(MELODY),
            Some(SOLO),
            Some(SOLO),
            Some(SOLO)
        ]
    );
    let starts: Vec<_> = c.decisions.iter().map(|d| d.bar).collect();
    assert_eq!(starts, [0, 3]);
    // With no run needed, the stray bar flips it.
    let rules = LeadRules {
        min_run: 1,
        ..LeadRules::default()
    };
    let c = choose_lead(&parts, &name(), bar(), &[], &rules);
    assert_eq!(c.leads[1], Some(SOLO));
}

#[test]
fn silence_keeps_the_lead_and_silent_bars_score_nothing() {
    let parts = [melody(MELODY, 0..1, 60), melody(SOLO, 2..3, 60)];
    let c = choose_lead(&parts, &name(), bar(), &[], &LeadRules::default());
    // Bar 1 is silent: the lead stays. Bar 2 is the solo's alone, to the end, so it takes over at once.
    assert_eq!(c.leads[0], Some(MELODY));
    assert_eq!(c.leads[1], Some(MELODY));
    assert_eq!(c.leads[2], Some(SOLO));
    let m = c.salience.iter().find(|s| s.source == MELODY).unwrap();
    assert_eq!(m.bars[1], Salience::default());
}

fn override_(id: &str, bar: u32, source: ContentHash, provenance: Provenance) -> Decision {
    Decision {
        id: DecisionId::new(id).unwrap(),
        subtune: name(),
        bar,
        proposal: Proposal::LeadSource { source },
        provenance,
        confidence: Confidence::CERTAIN,
        reviewed: true,
    }
}

#[test]
fn overrides_hold_until_the_next_and_a_person_beats_a_model() {
    let parts = [bass(0..6), melody(MELODY, 0..6, 60)];
    let llm = Provenance::Llm {
        model: "any".into(),
        prompt: ContentHash([9; 32]),
    };
    let overrides = [
        override_("pick-bass", 2, BASS, Provenance::Manual),
        override_("model-melody", 2, MELODY, llm.clone()),
        override_("back", 4, MELODY, llm),
    ];
    let c = choose_lead(&parts, &name(), bar(), &overrides, &LeadRules::default());
    assert_eq!(
        leads(&c),
        [
            Some(MELODY),
            Some(MELODY),
            Some(BASS),
            Some(BASS),
            Some(MELODY),
            Some(MELODY)
        ]
    );
    // The rules decided bar 0 only; the rest was overridden.
    assert_eq!(c.decisions.iter().map(|d| d.bar).collect::<Vec<_>>(), [0]);
    assert!(c.unknown_overrides.is_empty());
}

#[test]
fn overrides_for_other_parts_subtunes_or_kinds_are_not_applied() {
    let parts = [melody(MELODY, 0..2, 60)];
    let mut other_subtune = override_("elsewhere", 0, MELODY, Provenance::Manual);
    other_subtune.subtune = SubtuneName::new("title").unwrap();
    let mut rules_made = override_("rules", 0, MELODY, Provenance::Manual);
    rules_made.provenance = Provenance::Rules {
        rule: "choose-lead@1".into(),
    };
    let overrides = [
        override_("ghost-part", 1, SOLO, Provenance::Manual),
        other_subtune,
        rules_made,
    ];
    let c = choose_lead(&parts, &name(), bar(), &overrides, &LeadRules::default());
    assert_eq!(leads(&c), [Some(MELODY), Some(MELODY)]);
    assert_eq!(
        c.unknown_overrides,
        [DecisionId::new("ghost-part").unwrap()]
    );
    assert_eq!(c.decisions.len(), 1, "the rules still decide");
}

#[test]
fn no_parts_no_bars() {
    let c = choose_lead(&[], &name(), bar(), &[], &LeadRules::default());
    assert!(c.leads.is_empty() && c.salience.is_empty() && c.decisions.is_empty());
}

#[test]
fn rules_round_trip_and_read_with_defaults() {
    let r = LeadRules::default();
    let json = serde_json::to_string(&r).unwrap();
    assert_eq!(serde_json::from_str::<LeadRules>(&json).unwrap(), r);
    let partial: LeadRules = serde_json::from_str(r#"{"min_run":4}"#).unwrap();
    assert_eq!(partial.min_run, 4);
    assert_eq!(partial.pitch, r.pitch);
}

fn any_parts() -> impl Strategy<Value = Vec<QuantisedPart>> {
    proptest::collection::vec((strategies::content_hash(), strategies::note_soup()), 0..4).prop_map(
        |parts| {
            let mut seen = std::collections::BTreeSet::new();
            parts
                .into_iter()
                .filter(|(s, _)| seen.insert(*s))
                .map(|(s, notes)| part(s, notes))
                .collect()
        },
    )
}

proptest! {
    /// Every bar's lead is one of the parts (none only before any part plays); rules decisions sit where the lead
    /// changes to their source; and the order of the parts does not matter.
    #[test]
    fn leads_are_parts_and_decisions_mark_changes(parts in any_parts(), reverse in any::<bool>(), run in 1u32..4) {
        let rules = LeadRules { min_run: run, ..LeadRules::default() };
        let c = choose_lead(&parts, &name(), bar(), &[], &rules);
        let sources: Vec<_> = parts.iter().map(|p| p.source).collect();
        for (b, lead) in c.leads.iter().enumerate() {
            match lead {
                Some(s) => prop_assert!(sources.contains(s)),
                None => prop_assert!(c.leads[..=b].iter().all(Option::is_none)),
            }
        }
        for d in &c.decisions {
            let b = d.bar as usize;
            let Proposal::LeadSource { source } = d.proposal else { unreachable!() };
            prop_assert_eq!(c.leads[b], Some(source));
            prop_assert!(b == 0 || c.leads[b - 1] != Some(source));
        }
        // Every change of lead has a decision.
        for b in 1..c.leads.len() {
            if c.leads[b] != c.leads[b - 1] {
                prop_assert!(c.decisions.iter().any(|d| d.bar as usize == b));
            }
        }
        let mut flipped = parts.clone();
        if reverse {
            flipped.reverse();
        }
        prop_assert_eq!(choose_lead(&flipped, &name(), bar(), &[], &rules), c);
    }

    /// A person's override is always obeyed, from its bar to the next.
    #[test]
    fn overrides_are_obeyed(parts in any_parts(), at_bar in 0u32..20, pick in any::<prop::sample::Index>()) {
        prop_assume!(!parts.is_empty());
        let source = parts[pick.index(parts.len())].source;
        let o = override_("pick", at_bar, source, Provenance::Manual);
        let c = choose_lead(&parts, &name(), bar(), &[o], &LeadRules::default());
        for (b, lead) in c.leads.iter().enumerate() {
            if b as u32 >= at_bar {
                prop_assert_eq!(*lead, Some(source));
            }
        }
        prop_assert!(c.decisions.iter().all(|d| d.bar < at_bar));
    }
}
