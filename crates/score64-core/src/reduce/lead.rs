//! Lead selection (#39): which transcribed part carries the melody, bar by bar, scored by salience and overridable
//! by decisions.

use std::num::NonZeroU8;

use serde::{Deserialize, Serialize};

use crate::provenance::{Confidence, ContentHash, Decision, DecisionId, Proposal, Provenance};
use crate::score::{Note, SubtuneName};
use crate::transcription::QuantisedPart;

/// The rule name and version [`choose_lead`] records in its decisions.
pub const CHOOSE_LEAD: &str = "choose-lead@1";

/// The settings of [`choose_lead`]: how much each kind of salience counts, and how steady the lead is.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct LeadRules {
    /// Weight of pitch: leads sit high.
    pub pitch: f32,
    /// Weight of density: leads have about one note a beat, neither held pads nor busy hi-hats.
    pub density: f32,
    /// Weight of continuity: leads are single lines that move in steps, neither repeating one pitch nor leaping.
    pub continuity: f32,
    /// Weight of the transcriber's confidence.
    pub confidence: f32,
    /// The density that scores best, in notes per bar.
    pub notes_per_bar: f32,
    /// How many bars in a row a part must score best before the lead moves to it.
    pub min_run: u32,
}

impl Default for LeadRules {
    /// Pitch 0.3, density 0.2, continuity 0.3, confidence 0.2; four notes a bar; two bars to change lead. Starting
    /// points, to tune against real material.
    fn default() -> Self {
        LeadRules {
            pitch: 0.3,
            density: 0.2,
            continuity: 0.3,
            confidence: 0.2,
            notes_per_bar: 4.0,
            min_run: 2,
        }
    }
}

/// How lead-like a part is in one bar. Each score is 0 to 1; all are 0 in a bar where the part has no notes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Salience {
    /// Mean pitch, from C-3 (0) to C-7 (1).
    pub pitch: f32,
    /// Notes in the bar against [`LeadRules::notes_per_bar`]: 1 at that many, less for fewer or more.
    pub density: f32,
    /// The share of sounding rows with one note only, averaged with how step-wise the line moves (steps up to a
    /// fourth score 1, leaps less, repeated pitches a half).
    pub continuity: f32,
    /// Mean confidence.
    pub confidence: f32,
    /// The weighted mean of the four.
    pub total: f32,
}

/// One part's salience, bar by bar.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct PartSalience {
    /// The part, by source.
    pub source: ContentHash,
    /// Its salience in each bar.
    pub bars: Vec<Salience>,
}

/// What [`choose_lead`] chose.
#[derive(Clone, Debug, PartialEq)]
pub struct LeadChoice {
    /// The lead's source in each bar; `None` until some part has notes.
    pub leads: Vec<Option<ContentHash>>,
    /// Every part's salience, in source order, so a reviewer can see why.
    pub salience: Vec<PartSalience>,
    /// A `LeadSource` decision at each bar where the rules start a new lead (`lead-b<bar>`).
    pub decisions: Vec<Decision>,
    /// Override decisions naming a source that is not one of the parts; not applied.
    pub unknown_overrides: Vec<DecisionId>,
}

/// Choose the lead part of each bar of `subtune`.
///
/// Each bar's best part is the one with the highest salience (the lowest source hash on a tie, so the order of
/// `parts` does not matter). The lead moves to a new part only when that part is best for `min_run` bars in a row (or
/// to the end), and straight away when there is no lead yet; a bar where no part has notes keeps the lead.
///
/// `overrides` are `LeadSource` decisions by a person or a model for this subtune (others are ignored). From an
/// override's bar on, its source is the lead until the next override; a person's beats a model's on the same bar,
/// as TAD §3.3 orders them. The rules make decisions only where they choose.
pub fn choose_lead(
    parts: &[QuantisedPart],
    subtune: &SubtuneName,
    rows_per_bar: NonZeroU8,
    overrides: &[Decision],
    rules: &LeadRules,
) -> LeadChoice {
    let rows = u32::from(rows_per_bar.get());
    let mut parts: Vec<&QuantisedPart> = parts.iter().collect();
    parts.sort_by_key(|p| p.source);
    let bars = parts
        .iter()
        .flat_map(|p| &p.notes)
        .map(|n| (n.row + n.length.get()).div_ceil(rows))
        .max()
        .unwrap_or(0);
    let salience: Vec<PartSalience> = parts
        .iter()
        .map(|p| PartSalience {
            source: p.source,
            bars: (0..bars)
                .map(|b| score(&p.notes, b * rows, rows, rules))
                .collect(),
        })
        .collect();
    let best: Vec<Option<ContentHash>> = (0..bars as usize)
        .map(|b| {
            salience
                .iter()
                .filter(|s| s.bars[b].total > 0.0)
                // Highest total; on a tie the first, which has the lowest source.
                .fold(None::<&PartSalience>, |acc, s| match acc {
                    Some(a) if a.bars[b].total >= s.bars[b].total => Some(a),
                    _ => Some(s),
                })
                .map(|s| s.source)
        })
        .collect();

    let (pinned, unknown_overrides) = overrides_by_bar(overrides, subtune, &salience, bars);
    let mut leads: Vec<Option<ContentHash>> = Vec::with_capacity(bars as usize);
    let mut starts: Vec<usize> = Vec::new(); // bars where the rules started a lead
    let mut lead = None;
    let mut locked = false;
    for b in 0..bars as usize {
        if let Some(source) = pinned[b] {
            lead = Some(source);
            locked = true;
        } else if !locked && let Some(candidate) = best[b] {
            let run = rules.min_run.max(1) as usize;
            let holds = best[b..(b + run).min(best.len())]
                .iter()
                .all(|&x| x == Some(candidate));
            if lead.is_none() || (lead != Some(candidate) && holds) {
                lead = Some(candidate);
                starts.push(b);
            }
        }
        leads.push(lead);
    }

    let decisions = starts
        .iter()
        .enumerate()
        .map(|(i, &b)| {
            let end = starts.get(i + 1).copied().unwrap_or(leads.len());
            let end = (b + 1..end).find(|&x| pinned[x].is_some()).unwrap_or(end);
            let source = leads[b].expect("a rules start has a lead");
            Decision {
                id: DecisionId::new(format!("lead-b{b}")).expect("a valid id"),
                subtune: subtune.clone(),
                bar: b as u32,
                proposal: Proposal::LeadSource { source },
                provenance: Provenance::Rules {
                    rule: CHOOSE_LEAD.into(),
                },
                confidence: margin(&salience, source, b..end),
                reviewed: false,
            }
        })
        .collect();

    LeadChoice {
        leads,
        salience,
        decisions,
        unknown_overrides,
    }
}

/// The salience of `notes` in the bar of `rows` rows from row `start`.
fn score(notes: &[Note], start: u32, rows: u32, rules: &LeadRules) -> Salience {
    let mut bar: Vec<&Note> = notes
        .iter()
        .filter(|n| (start..start + rows).contains(&n.row))
        .collect();
    if bar.is_empty() {
        return Salience::default();
    }
    bar.sort_by_key(|n| (n.row, n.pitch));
    let count = bar.len() as f32;
    let mean_pitch = bar
        .iter()
        .map(|n| f32::from(n.pitch.semitone()))
        .sum::<f32>()
        / count;
    let pitch = ((mean_pitch - 36.0) / 48.0).clamp(0.0, 1.0);
    let ideal = rules.notes_per_bar.max(1.0);
    let density = count.min(ideal) / count.max(ideal);
    // Rows of the bar with at least one note sounding, and with exactly one.
    let (mut sounding, mut single) = (0u32, 0u32);
    for row in start..start + rows {
        let here = notes
            .iter()
            .filter(|n| n.row <= row && row < n.row + n.length.get())
            .count();
        sounding += u32::from(here > 0);
        single += u32::from(here == 1);
    }
    let lines = if sounding == 0 {
        1.0
    } else {
        single as f32 / sounding as f32
    };
    // Melodies move in steps: up to a fourth scores 1, leaps less, and a repeated pitch (drums, ostinati) a half.
    let steps: Vec<f32> = bar
        .windows(2)
        .map(|w| {
            let d = w[1].pitch.semitone().abs_diff(w[0].pitch.semitone());
            match d {
                0 => 0.5,
                1..=5 => 1.0,
                _ => 1.0 / (1.0 + f32::from(d - 5) / 7.0),
            }
        })
        .collect();
    let smooth = if steps.is_empty() {
        1.0
    } else {
        steps.iter().sum::<f32>() / steps.len() as f32
    };
    let continuity = (lines + smooth) / 2.0;
    let confidence = bar.iter().map(|n| n.confidence.get()).sum::<f32>() / count;
    let weights = rules.pitch + rules.density + rules.continuity + rules.confidence;
    let total = if weights > 0.0 {
        (rules.pitch * pitch
            + rules.density * density
            + rules.continuity * continuity
            + rules.confidence * confidence)
            / weights
    } else {
        0.0
    };
    Salience {
        pitch,
        density,
        continuity,
        confidence,
        // Above zero for any part with notes, so a part that plays is never taken for silence.
        total: total.max(f32::MIN_POSITIVE),
    }
}

/// Each bar's override, if any, and the overrides naming no part.
fn overrides_by_bar(
    overrides: &[Decision],
    subtune: &SubtuneName,
    salience: &[PartSalience],
    bars: u32,
) -> (Vec<Option<ContentHash>>, Vec<DecisionId>) {
    let mut pinned: Vec<Option<(u8, &DecisionId, ContentHash)>> = vec![None; bars as usize];
    let mut unknown = Vec::new();
    for d in overrides {
        let Proposal::LeadSource { source } = d.proposal else {
            continue;
        };
        // A person's choice outranks a model's; the rules' own decisions are not overrides.
        let rank = match d.provenance {
            Provenance::Manual => 2,
            Provenance::Llm { .. } => 1,
            Provenance::Rules { .. } => continue,
        };
        if &d.subtune != subtune {
            continue;
        }
        if !salience.iter().any(|s| s.source == source) {
            unknown.push(d.id.clone());
            continue;
        }
        let Some(slot) = pinned.get_mut(d.bar as usize) else {
            continue;
        };
        // Higher rank wins; on the same rank, the lower id, so the order of `overrides` does not matter.
        if slot
            .is_none_or(|(r, id, _)| (rank, std::cmp::Reverse(&d.id)) > (r, std::cmp::Reverse(id)))
        {
            *slot = Some((rank, &d.id, source));
        }
    }
    unknown.sort();
    (
        pinned.into_iter().map(|p| p.map(|(.., s)| s)).collect(),
        unknown,
    )
}

/// How clearly `source` led over `bars`: its mean salience minus the best other part's, from 0 to 1.
fn margin(
    salience: &[PartSalience],
    source: ContentHash,
    bars: std::ops::Range<usize>,
) -> Confidence {
    let n = bars.len().max(1) as f32;
    let mean = |s: &PartSalience| bars.clone().map(|b| s.bars[b].total).sum::<f32>() / n;
    let own = salience
        .iter()
        .find(|s| s.source == source)
        .map(mean)
        .unwrap_or(0.0);
    let other = salience
        .iter()
        .filter(|s| s.source != source)
        .map(mean)
        .fold(0.0, f32::max);
    Confidence::new((own - other).clamp(0.0, 1.0)).expect("clamped")
}

#[cfg(test)]
mod tests;
