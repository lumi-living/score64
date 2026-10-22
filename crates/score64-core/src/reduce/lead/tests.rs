//! Choosing the lead part bar by bar, and how overrides win.

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

/// `n` bars of 16 rows of the subtune `subtune`.
fn bars(subtune: &SubtuneName, n: u32) -> Bars<'_> {
    Bars {
        subtune,
        rows_per_bar: NonZeroU8::new(16).unwrap(),
        length: NonZeroU32::new(n * 16).unwrap(),
    }
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

/// A singable line over `bars`: four quarter notes a bar from `base`, in steps.
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

/// Hi-hats on every `every` rows, at C-7.
fn hats(bars: std::ops::Range<u32>, every: u32) -> QuantisedPart {
    part(
        HATS,
        bars.flat_map(|b| (0..16 / every).map(move |i| note(b * 16 + i * every, 1, 84)))
            .collect(),
    )
}

fn choose(parts: &[QuantisedPart], n: u32, overrides: &[Decision]) -> LeadChoice {
    choose_lead(parts, bars(&name(), n), overrides, &LeadRules::default())
}

fn sal(c: &LeadChoice, source: ContentHash, bar: usize) -> Salience {
    c.salience.iter().find(|s| s.source == source).unwrap().bars[bar]
}

#[test]
fn a_singable_line_beats_bass_and_hats() {
    // Hats on sixteenths and on eighths, around a melody low in the range (C-4) and a bass.
    for every in [1, 2] {
        let parts = [bass(0..4), hats(0..4, every), melody(MELODY, 0..4, 48)];
        let c = choose(&parts, 4, &[]);
        assert_eq!(c.leads, vec![Some(MELODY); 4], "hats every {every}");
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
        // The margin over the best rival, relative to the lead's own salience.
        let own = sal(&c, MELODY, 0).total;
        let rival = sal(&c, HATS, 0).total.max(sal(&c, BASS, 0).total);
        assert!((d.confidence.get() - (own - rival) / own).abs() < 1e-5);
    }
}

#[test]
fn each_score_does_its_part() {
    let c = choose(
        &[melody(MELODY, 0..1, 58), hats(0..1, 1), bass(0..1)],
        1,
        &[],
    );
    let (m, h, b) = (sal(&c, MELODY, 0), sal(&c, HATS, 0), sal(&c, BASS, 0));
    // Pitch peaks at C-5: the melody (around B-4) is near it; the hats (C-7) and bass (C-2 to G-2) are far.
    assert!(
        m.pitch > 0.9 && h.pitch < 0.4 && b.pitch < 0.25,
        "{m:?} {h:?} {b:?}"
    );
    // Density: four notes a bar is right; sixteen, or two, is not.
    assert_eq!(m.density, 1.0);
    assert_eq!(h.density, 0.25);
    assert_eq!(b.density, 0.5);
    // Continuity: the melody moves in steps; the hats repeat one pitch (no step credit); the bass leaps a fifth.
    assert_eq!(m.continuity, 1.0);
    assert_eq!(h.continuity, 0.5);
    assert!(b.continuity < 1.0 && b.continuity > 0.5);
    assert!((m.confidence - 0.9).abs() < 1e-6);
    // Overlapping notes lower continuity: a chord is not a line.
    let chord = part(SOLO, vec![note(0, 16, 60), note(0, 16, 64)]);
    let c = choose(&[chord], 1, &[]);
    assert!(sal(&c, SOLO, 0).continuity <= 0.5);
    // A note held over a bar line sounds in the next bar too.
    let held = part(SOLO, vec![note(8, 16, 60)]);
    let c = choose(&[held], 2, &[]);
    assert!(sal(&c, SOLO, 1).total > 0.0);
    assert_eq!(sal(&c, SOLO, 1).density, 0.0, "but starts nothing there");
}

#[test]
fn weights_change_the_mix_and_bad_ones_count_as_zero() {
    let parts = [melody(MELODY, 0..1, 58)];
    let only = |r: LeadRules| sal(&choose_lead(&parts, bars(&name(), 1), &[], &r), MELODY, 0);
    let pitch_only = LeadRules {
        pitch: 1.0,
        density: 0.0,
        continuity: f32::NAN,
        confidence: -1.0,
        ..LeadRules::default()
    };
    let s = only(pitch_only);
    assert!((s.total - s.pitch).abs() < 1e-6, "{s:?}");
    let confidence_only = LeadRules {
        pitch: 0.0,
        density: 0.0,
        continuity: 0.0,
        confidence: 2.0,
        ..LeadRules::default()
    };
    let s = only(confidence_only);
    assert!((s.total - 0.9).abs() < 1e-6, "{s:?}");
    // No weight at all: still above zero, so a part that plays is not silence.
    let none = LeadRules {
        pitch: 0.0,
        density: 0.0,
        continuity: 0.0,
        confidence: 0.0,
        ..LeadRules::default()
    };
    assert!(only(none).total > 0.0);
}

#[test]
fn the_lead_moves_only_when_another_part_holds_it() {
    // The melody plays bars 0–5; a higher solo takes over from bar 3, and plays one stray bar (1) before that.
    let mut solo = melody(SOLO, 3..6, 60);
    solo.notes.extend(melody(SOLO, 1..2, 60).notes);
    let parts = [melody(MELODY, 0..6, 48), solo];
    let c = choose(&parts, 6, &[]);
    assert_eq!(
        c.leads,
        [
            Some(MELODY),
            Some(MELODY),
            Some(MELODY),
            Some(SOLO),
            Some(SOLO),
            Some(SOLO)
        ]
    );
    assert_eq!(
        c.decisions.iter().map(|d| d.bar).collect::<Vec<_>>(),
        [0, 3]
    );
    let rules = LeadRules {
        min_run: NonZeroU32::new(1).unwrap(),
        ..LeadRules::default()
    };
    let c = choose_lead(&parts, bars(&name(), 6), &[], &rules);
    assert_eq!(
        c.leads[1],
        Some(SOLO),
        "with no run needed, the stray bar flips it"
    );
}

#[test]
fn ties_go_to_the_lowest_source() {
    let parts = [melody(SOLO, 0..2, 60), melody(MELODY, 0..2, 60)];
    let c = choose(&parts, 2, &[]);
    assert_eq!(c.leads, [Some(MELODY), Some(MELODY)]);
}

#[test]
fn silence_and_no_rival() {
    // Bar 1 is silent: the lead stays. The solo has bar 2 alone, to the end, so it takes over at once.
    let parts = [melody(MELODY, 0..1, 60), melody(SOLO, 2..3, 60)];
    let c = choose(&parts, 3, &[]);
    assert_eq!(c.leads, [Some(MELODY), Some(MELODY), Some(SOLO)]);
    assert_eq!(sal(&c, MELODY, 1), Salience::default());
    // Neither had a rival where it played.
    assert!(
        c.decisions
            .iter()
            .all(|d| d.confidence == Confidence::CERTAIN)
    );
}

#[test]
fn notes_past_the_end_do_not_count() {
    // One stray note far beyond a two-bar subtune: no extra bars, and no overflow.
    let mut p = melody(MELODY, 0..2, 60);
    p.notes.push(note(u32::MAX - 2, 5, 60));
    let c = choose(&[p], 2, &[]);
    assert_eq!(c.leads.len(), 2);
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

fn model() -> Provenance {
    Provenance::Llm {
        model: "any".into(),
        prompt: ContentHash([9; 32]),
    }
}

#[test]
fn a_persons_choice_holds_over_any_later_models() {
    let parts = [bass(0..6), melody(MELODY, 0..6, 60)];
    let overrides = [
        override_("model-early", 1, BASS, model()),
        override_("person", 2, BASS, Provenance::Manual),
        override_("model-same-bar", 2, MELODY, model()),
        override_("model-later", 4, MELODY, model()),
    ];
    let c = choose(&parts, 6, &overrides);
    assert_eq!(
        c.leads,
        [
            Some(MELODY),
            Some(BASS),
            Some(BASS),
            Some(BASS),
            Some(BASS),
            Some(BASS)
        ]
    );
    // The rules decided bar 0 only; the model was overruled from bar 2 on.
    assert_eq!(c.decisions.iter().map(|d| d.bar).collect::<Vec<_>>(), [0]);
    assert_eq!(
        c.unapplied,
        [
            (
                DecisionId::new("model-later").unwrap(),
                Unapplied::Overruled
            ),
            (
                DecisionId::new("model-same-bar").unwrap(),
                Unapplied::Overruled
            ),
        ]
    );
    // A person's later choice replaces their earlier one.
    let overrides = [
        override_("first", 1, BASS, Provenance::Manual),
        override_("second", 3, MELODY, Provenance::Manual),
    ];
    let c = choose(&parts, 6, &overrides);
    assert_eq!(
        c.leads[1..],
        [
            Some(BASS),
            Some(BASS),
            Some(MELODY),
            Some(MELODY),
            Some(MELODY)
        ]
    );
}

#[test]
fn on_one_bar_and_rank_the_lower_id_wins_in_any_order() {
    let parts = [bass(0..2), melody(MELODY, 0..2, 60)];
    let a = override_("a-bass", 0, BASS, Provenance::Manual);
    let b = override_("b-melody", 0, MELODY, Provenance::Manual);
    for overrides in [[a.clone(), b.clone()], [b, a]] {
        let c = choose(&parts, 2, &overrides);
        assert_eq!(c.leads, [Some(BASS), Some(BASS)]);
    }
}

#[test]
fn overrides_not_applied_are_reported() {
    let parts = [melody(MELODY, 0..2, 60)];
    let mut elsewhere = override_("elsewhere", 0, MELODY, Provenance::Manual);
    elsewhere.subtune = SubtuneName::new("title").unwrap();
    let rules_made = override_(
        "rules",
        0,
        MELODY,
        Provenance::Rules {
            rule: "choose-lead@1".into(),
        },
    );
    let overrides = [
        override_("ghost-part", 1, SOLO, Provenance::Manual),
        override_("too-late", 5, MELODY, Provenance::Manual),
        elsewhere,
        rules_made,
    ];
    let c = choose(&parts, 2, &overrides);
    assert_eq!(c.leads, [Some(MELODY), Some(MELODY)]);
    assert_eq!(
        c.unapplied,
        [
            (
                DecisionId::new("ghost-part").unwrap(),
                Unapplied::UnknownSource
            ),
            (DecisionId::new("too-late").unwrap(), Unapplied::PastEnd),
        ]
    );
    assert_eq!(c.decisions.len(), 1, "the rules still decide");
}

#[test]
fn rules_round_trip_and_read_with_defaults() {
    let r = LeadRules::default();
    let json = serde_json::to_string(&r).unwrap();
    assert_eq!(serde_json::from_str::<LeadRules>(&json).unwrap(), r);
    let partial: LeadRules = serde_json::from_str(r#"{"min_run":4}"#).unwrap();
    assert_eq!(partial.min_run.get(), 4);
    assert_eq!(partial.pitch, r.pitch);
    assert!(serde_json::from_str::<LeadRules>(r#"{"min_run":0}"#).is_err());
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
    /// Every bar's lead is one of the parts (none only before any part sounds); rules decisions sit where the lead
    /// changes to their source, and every change has one; part order does not matter.
    #[test]
    fn leads_are_parts_and_decisions_mark_changes(parts in any_parts(), reverse in any::<bool>(), run in 1u32..4) {
        let rules = LeadRules { min_run: NonZeroU32::new(run).unwrap(), ..LeadRules::default() };
        let n = name();
        let c = choose_lead(&parts, bars(&n, 20), &[], &rules);
        prop_assert_eq!(c.leads.len(), 20);
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
        for b in 1..c.leads.len() {
            if c.leads[b] != c.leads[b - 1] {
                prop_assert!(c.decisions.iter().any(|d| d.bar as usize == b));
            }
        }
        let mut flipped = parts.clone();
        if reverse {
            flipped.reverse();
        }
        prop_assert_eq!(choose_lead(&flipped, bars(&n, 20), &[], &rules), c);
    }

    /// A person's override is obeyed from its bar to the end (with no later one), whatever a model says.
    #[test]
    fn a_persons_override_is_obeyed(
        parts in any_parts(),
        at_bar in 0u32..20,
        model_bar in 0u32..20,
        pick in any::<prop::sample::Index>(),
        other in any::<prop::sample::Index>(),
    ) {
        prop_assume!(!parts.is_empty());
        let source = parts[pick.index(parts.len())].source;
        let rival = parts[other.index(parts.len())].source;
        let overrides = [
            override_("person", at_bar, source, Provenance::Manual),
            override_("model", model_bar, rival, model()),
        ];
        let c = choose(&parts, 20, &overrides);
        for (b, lead) in c.leads.iter().enumerate() {
            if b as u32 >= at_bar {
                prop_assert_eq!(*lead, Some(source));
            } else if b as u32 >= model_bar {
                prop_assert_eq!(*lead, Some(rival));
            }
        }
        prop_assert!(c.decisions.iter().all(|d| d.bar < at_bar.min(model_bar)));
    }
}
