//! Lead selection (#39): which transcribed part carries the melody, bar by bar, scored by salience and overridable
//! by decisions.

use std::num::{NonZeroU8, NonZeroU32};

use serde::{Deserialize, Serialize};

use crate::provenance::{Confidence, ContentHash, Decision, DecisionId, Proposal, Provenance};
use crate::score::{Note, Subtune, SubtuneName};
use crate::transcription::QuantisedPart;

/// The rule name and version [`choose_lead`] records in its decisions.
pub const CHOOSE_LEAD: &str = "choose-lead@1";

/// The settings of [`choose_lead`]: how much each kind of salience counts, and how steady the lead is. A weight that
/// is not a positive, finite number counts as 0.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct LeadRules {
    /// Weight of pitch: leads sit in the middle of the range, around C-5.
    pub pitch: f32,
    /// Weight of density: leads have about one note a beat, neither held pads nor busy hi-hats.
    pub density: f32,
    /// Weight of continuity: leads are single lines that move in steps, neither repeating one pitch nor leaping.
    pub continuity: f32,
    /// Weight of the transcriber's confidence.
    pub confidence: f32,
    /// The density that scores best, in notes per row (0.25: one note a beat at four rows a beat).
    pub notes_per_row: f32,
    /// How many bars in a row a part must score best before the lead moves to it.
    pub min_run: NonZeroU32,
}

impl Default for LeadRules {
    /// Pitch 0.3, density 0.2, continuity 0.3, confidence 0.2; a note every four rows; two bars to change lead.
    /// Starting points, to tune against real material.
    fn default() -> Self {
        LeadRules {
            pitch: 0.3,
            density: 0.2,
            continuity: 0.3,
            confidence: 0.2,
            notes_per_row: 0.25,
            min_run: NonZeroU32::new(2).expect("non-zero"),
        }
    }
}

/// The bars a lead is chosen over: a subtune's name, bar length and length.
#[derive(Clone, Copy, Debug)]
pub struct Bars<'a> {
    /// The subtune, which decisions name.
    pub subtune: &'a SubtuneName,
    /// Rows per bar.
    pub rows_per_bar: NonZeroU8,
    /// The subtune's length, in rows; notes past it are not heard and do not count.
    pub length: NonZeroU32,
}

impl<'a> From<&'a Subtune> for Bars<'a> {
    fn from(s: &'a Subtune) -> Self {
        Bars {
            subtune: &s.name,
            rows_per_bar: s.rows_per_bar,
            length: s.length,
        }
    }
}

/// How lead-like a part is in one bar. Each score is 0 to 1; all are 0 in a bar where none of the part's notes sound.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Salience {
    /// Mean pitch of the notes sounding in the bar: 1 at C-5, falling to 0 three octaves either side.
    pub pitch: f32,
    /// Notes starting in the bar against [`LeadRules::notes_per_row`]: 1 at that many, less for fewer or more.
    pub density: f32,
    /// The share of sounding rows with one note only, averaged with how step-wise the line moves (steps up to a
    /// fourth score 1, leaps less, a repeated pitch 0).
    pub continuity: f32,
    /// Mean confidence of the notes sounding in the bar.
    pub confidence: f32,
    /// The weighted mean of the four; above 0 whenever a note sounds.
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

/// Why an override decision was not applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Unapplied {
    /// Its source is not one of the parts.
    UnknownSource,
    /// Its bar is past the end of the subtune.
    PastEnd,
    /// A model's choice where a person's is in force.
    Overruled,
}

/// What [`choose_lead`] chose.
#[derive(Clone, Debug, PartialEq)]
pub struct LeadChoice {
    /// The lead's source in each bar; `None` until some part sounds.
    pub leads: Vec<Option<ContentHash>>,
    /// Every part's salience, in source order, so a reviewer can see why.
    pub salience: Vec<PartSalience>,
    /// A `LeadSource` decision at each bar where the rules start a new lead (`lead-b<bar>`).
    pub decisions: Vec<Decision>,
    /// Override decisions that were not applied, and why, by id.
    pub unapplied: Vec<(DecisionId, Unapplied)>,
}

/// Choose the lead part of each bar of `bars`.
///
/// Each bar's best part is the one with the highest salience (the lowest source hash on a tie, so the order of
/// `parts` does not matter). The lead moves to a new part only when that part is best for `min_run` bars in a row (or
/// to the end), and straight away when there is no lead yet; a bar where no part sounds keeps the lead, and so does a
/// one-bar rest of the lead while another part plays.
///
/// `overrides` are `LeadSource` decisions for this subtune; the rules' own are ignored. A person's (`Manual`) is in
/// force from its bar until their next; a model's (`Llm`) likewise, but only where no person's is in force, so
/// re-running a model never undoes a person's choice (TAD §3.3). On the same bar and rank, the lower id wins. The rules
/// choose only before the first override in force.
///
/// Each rules decision's confidence is how clearly the lead beat the best other part over its run, counting the bars
/// where it sounds: `(own - other) / own` of their mean saliences, and 1 with no rival.
pub fn choose_lead(
    parts: &[QuantisedPart],
    bars: Bars,
    overrides: &[Decision],
    rules: &LeadRules,
) -> LeadChoice {
    let rows = u32::from(bars.rows_per_bar.get());
    let count = bars.length.get().div_ceil(rows) as usize;
    let mut parts: Vec<&QuantisedPart> = parts.iter().collect();
    parts.sort_by_key(|p| p.source);
    let salience: Vec<PartSalience> = parts
        .iter()
        .map(|p| {
            // Each bar's sounding notes, found in one pass over the part.
            let mut by_bar: Vec<Vec<&Note>> = vec![Vec::new(); count];
            for n in &p.notes {
                let first = n.row / rows;
                let last = n.row.saturating_add(n.length.get() - 1) / rows;
                for b in first..=last.min(count as u32 - 1) {
                    if let Some(bar) = by_bar.get_mut(b as usize) {
                        bar.push(n);
                    }
                }
            }
            PartSalience {
                source: p.source,
                bars: by_bar
                    .iter()
                    .enumerate()
                    .map(|(b, notes)| score(notes, b as u32 * rows, rows, rules))
                    .collect(),
            }
        })
        .collect();
    let best: Vec<Option<ContentHash>> = (0..count)
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

    let (pinned, unapplied) = in_force(overrides, bars, &salience, count);
    let mut leads: Vec<Option<ContentHash>> = Vec::with_capacity(count);
    let mut starts: Vec<usize> = Vec::new(); // bars where the rules started a lead
    let mut lead = None;
    let run = rules.min_run.get() as usize;
    for b in 0..count {
        if let Some(source) = pinned[b] {
            lead = Some(source);
        } else if pinned[..b].iter().all(Option::is_none)
            && let Some(candidate) = best[b]
        {
            let holds = best[b..(b + run).min(count)]
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
            let next = starts.get(i + 1).copied().unwrap_or(count);
            let end = (b + 1..next).find(|&x| pinned[x].is_some()).unwrap_or(next);
            let source = leads[b].expect("a rules start has a lead");
            Decision {
                id: DecisionId::new(format!("lead-b{b}")).expect("a valid id"),
                subtune: bars.subtune.clone(),
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
        unapplied,
    }
}

/// A weight as used: 0 unless positive and finite.
fn weight(w: f32) -> f32 {
    if w.is_finite() && w > 0.0 { w } else { 0.0 }
}

/// The salience of a part's `notes` sounding in the bar of `rows` rows from row `start`.
fn score(notes: &[&Note], start: u32, rows: u32, rules: &LeadRules) -> Salience {
    if notes.is_empty() {
        return Salience::default();
    }
    let count = notes.len() as f32;
    let mean_pitch = notes
        .iter()
        .map(|n| f32::from(n.pitch.semitone()))
        .sum::<f32>()
        / count;
    let pitch = (1.0 - (mean_pitch - 60.0).abs() / 36.0).clamp(0.0, 1.0);
    let mut starting: Vec<&Note> = notes.iter().copied().filter(|n| n.row >= start).collect();
    starting.sort_by_key(|n| (n.row, n.pitch));
    let ideal = (weight(rules.notes_per_row) * rows as f32).max(1.0);
    let started = starting.len() as f32;
    let density = started.min(ideal) / started.max(ideal);
    // Rows of the bar with at least one note sounding, and with exactly one.
    let (mut sounding, mut single) = (0u32, 0u32);
    for row in start..start.saturating_add(rows) {
        let here = notes
            .iter()
            .filter(|n| n.row <= row && row < n.row.saturating_add(n.length.get()))
            .count();
        sounding += u32::from(here > 0);
        single += u32::from(here == 1);
    }
    let lines = single as f32 / sounding.max(1) as f32;
    // Melodies move in steps: up to a fourth scores 1, leaps less, and a repeated pitch (drums, ostinati) nothing.
    let steps: Vec<f32> = starting
        .windows(2)
        .map(
            |w| match w[1].pitch.semitone().abs_diff(w[0].pitch.semitone()) {
                0 => 0.0,
                1..=5 => 1.0,
                d => 1.0 / (1.0 + f32::from(d - 5) / 7.0),
            },
        )
        .collect();
    let smooth = if steps.is_empty() {
        1.0
    } else {
        steps.iter().sum::<f32>() / steps.len() as f32
    };
    let continuity = (lines + smooth) / 2.0;
    let confidence = notes.iter().map(|n| n.confidence.get()).sum::<f32>() / count;
    let w = [
        rules.pitch,
        rules.density,
        rules.continuity,
        rules.confidence,
    ]
    .map(weight);
    let sum = w.iter().sum::<f32>();
    let total = if sum > 0.0 {
        (w[0] * pitch + w[1] * density + w[2] * continuity + w[3] * confidence) / sum
    } else {
        0.0
    };
    Salience {
        pitch,
        density,
        continuity,
        confidence,
        // Above zero whenever a note sounds, so a part that plays is never taken for silence.
        total: total.max(f32::MIN_POSITIVE),
    }
}

/// The override in force in each bar, and the overrides not applied.
fn in_force(
    overrides: &[Decision],
    bars: Bars,
    salience: &[PartSalience],
    count: usize,
) -> (Vec<Option<ContentHash>>, Vec<(DecisionId, Unapplied)>) {
    // Each rank's own choice per bar, by the lowest id on a tie.
    let mut by_rank: [Vec<Option<(&DecisionId, ContentHash)>>; 2] =
        [vec![None; count], vec![None; count]];
    let mut unapplied = Vec::new();
    for d in overrides {
        let Proposal::LeadSource { source } = d.proposal else {
            continue;
        };
        let rank = match d.provenance {
            Provenance::Manual => 0,
            Provenance::Llm { .. } => 1,
            Provenance::Rules { .. } => continue,
        };
        if &d.subtune != bars.subtune {
            continue;
        }
        if !salience.iter().any(|s| s.source == source) {
            unapplied.push((d.id.clone(), Unapplied::UnknownSource));
            continue;
        }
        let Some(slot) = by_rank[rank].get_mut(d.bar as usize) else {
            unapplied.push((d.id.clone(), Unapplied::PastEnd));
            continue;
        };
        if slot.is_none_or(|(id, _)| &d.id < id) {
            *slot = Some((&d.id, source));
        }
    }
    // Carry each rank's choice forward from its bar; a person's, once made, is in force over any model's.
    let carried = |choices: &[Option<(&DecisionId, ContentHash)>]| -> Vec<Option<ContentHash>> {
        let mut now = None;
        choices
            .iter()
            .map(|c| {
                if let Some((_, s)) = c {
                    now = Some(*s);
                }
                now
            })
            .collect()
    };
    let person = carried(&by_rank[0]);
    let model = carried(&by_rank[1]);
    for (b, choice) in by_rank[1].iter().enumerate() {
        if let Some((id, _)) = choice
            && person[b].is_some()
        {
            unapplied.push(((*id).clone(), Unapplied::Overruled));
        }
    }
    unapplied.sort_by(|a, b| a.0.cmp(&b.0));
    unapplied.dedup_by(|a, b| a.0 == b.0);
    let pinned = person.iter().zip(&model).map(|(p, m)| p.or(*m)).collect();
    (pinned, unapplied)
}

/// How clearly `source` led over `bars`, counting the bars where it sounds: `(own - other) / own` of the mean
/// saliences, where `other` is the best other part's; 1 when no other part sounds there.
fn margin(
    salience: &[PartSalience],
    source: ContentHash,
    bars: std::ops::Range<usize>,
) -> Confidence {
    let own_part = salience.iter().find(|s| s.source == source);
    let playing: Vec<usize> = bars
        .filter(|&b| own_part.is_some_and(|s| s.bars[b].total > 0.0))
        .collect();
    let n = playing.len().max(1) as f32;
    let mean = |s: &PartSalience| playing.iter().map(|&b| s.bars[b].total).sum::<f32>() / n;
    let own = own_part.map(mean).unwrap_or(0.0);
    let other = salience
        .iter()
        .filter(|s| s.source != source)
        .map(mean)
        .fold(0.0, f32::max);
    let m = if other <= 0.0 {
        1.0
    } else if own <= 0.0 {
        0.0
    } else {
        (own - other) / own
    };
    Confidence::new(m.clamp(0.0, 1.0)).expect("clamped")
}

#[cfg(test)]
mod tests;
