//! The loop finder (#44): where a subtune should jump back to when it reaches its end.
//!
//! A subtune plays to its last row and then carries on from `loop_row`. A good loop row is a bar start after which
//! the music once went the way the ending goes: the bars just before it sound like the subtune's last bars, so the
//! jump sounds like the music continuing. Bars are compared by chroma, the weight of each of the 12 pitch classes.

use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};

use crate::decide::{Rank, Unapplied};
use crate::provenance::{Confidence, Decision, DecisionId, Proposal, Provenance};
use crate::score::{Note, Subtune};

/// The rule name and version [`find_loop`] records in its decision.
pub const FIND_LOOP: &str = "find-loop@1";

/// The settings of [`find_loop`].
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct LoopRules {
    /// How many bars before a candidate are compared with the subtune's last bars.
    pub context: NonZeroU32,
    /// The similarity a candidate needs; with none that good the subtune loops to its start.
    pub min_similarity: Confidence,
}

impl Default for LoopRules {
    /// Two bars of context; a similarity of 0.8.
    fn default() -> Self {
        LoopRules {
            context: NonZeroU32::new(2).expect("non-zero"),
            min_similarity: Confidence::new(0.8).expect("in range"),
        }
    }
}

/// A bar [`find_loop`] considered, and how alike the bars before it and the ending are, 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Candidate {
    /// The bar.
    pub bar: u32,
    /// Its similarity.
    pub similarity: f32,
}

/// Where [`find_loop`] would loop, and why.
#[derive(Clone, Debug, PartialEq)]
pub struct LoopChoice {
    /// The loop row: a bar start.
    pub row: u32,
    /// How alike the bars before it and the ending are, 0 to 1; 0 for a loop to the start that no bar earned.
    pub similarity: f32,
    /// Every candidate and its similarity, in bar order, for review.
    pub candidates: Vec<Candidate>,
    /// The rule's `LoopRow` decision (`loop`), made whether or not an override is in force.
    pub decision: Decision,
    /// The decision whose row this is: the rule's, or the override in force.
    pub by: DecisionId,
    /// Overrides that couldn't apply, and why, by id.
    pub unapplied: Vec<(DecisionId, Unapplied)>,
}

/// Find the loop row of `subtune`.
///
/// The ending is the last `context` whole bars, up to the last bar with music in it: a part bar at the end, and
/// silent bars before it, are left out. Every bar `a` from 1 on whose `context` bars before it end before the ending starts is a
/// candidate, scored by the cosine similarity of those bars' chroma (all voices, by rows sounding times confidence,
/// arpeggios by their chord tones) with the ending's; candidates whose bars are silent are skipped. The best
/// candidate at or above `min_similarity` wins, the earliest on a tie (the longest loop); with none, or a subtune too
/// short to compare, the loop row is 0.
///
/// One [`Proposal::LoopRow`] decision proposes it, as sure as the similarity, or half as sure when the subtune ends
/// inside a bar or after silence (the jump then comes from music the comparison did not see). The subtune itself is
/// not changed, and tempo is not compared.
pub fn find_loop(subtune: &Subtune, rules: &LoopRules) -> LoopChoice {
    find_loop_with(subtune, rules, &[])
}

/// [`find_loop`] with a model's or a person's resolved `loop_row` decision in force (#167): its row replaces the
/// rule's choice, and `by` names it. The rule still scores the candidates and makes its own decision, for review. An
/// override past the subtune's end is unapplied (`past_end`); with more than one for the subtune, the first applies
/// and the rest are `outranked` (resolve keeps one per subtune, so that only happens when it hasn't run).
pub fn find_loop_with(subtune: &Subtune, rules: &LoopRules, overrides: &[Decision]) -> LoopChoice {
    let mut choice = rule_loop(subtune, rules);
    for d in overrides
        .iter()
        .filter(|d| d.subtune == subtune.name && d.rank() != Rank::Rules)
    {
        let Proposal::LoopRow { row } = d.proposal else {
            continue;
        };
        if row >= subtune.length.get() {
            choice.unapplied.push((d.id.clone(), Unapplied::PastEnd));
        } else if choice.by != choice.decision.id {
            choice.unapplied.push((d.id.clone(), Unapplied::Outranked));
        } else {
            choice.row = row;
            choice.similarity = choice
                .candidates
                .iter()
                .find(|c| c.bar * u32::from(subtune.rows_per_bar.get()) == row)
                .map_or(0.0, |c| c.similarity);
            choice.by = d.id.clone();
        }
    }
    choice
}

/// What the rule itself chooses.
fn rule_loop(subtune: &Subtune, rules: &LoopRules) -> LoopChoice {
    let rows = u32::from(subtune.rows_per_bar.get());
    let length = subtune.length.get();
    // Whole bars only: the jump lands on a downbeat, so a part bar at the end can't be matched against one.
    let whole = length / rows;
    let chroma = chroma_by_bar(subtune, length.div_ceil(rows));
    // The ending is the last bars with music, so trailing silence (common at the end of a transcription) is skipped.
    let last = (0..whole)
        .rev()
        .find(|&b| chroma[b as usize].iter().any(|&w| w > 0.0))
        .map_or(0, |b| b + 1);
    let context = rules.context.get();
    let mut candidates = Vec::new();
    if let Some(from) = last.checked_sub(context) {
        let ending = window(&chroma, from, context);
        for a in context.max(1)..=from {
            let before = window(&chroma, a - context, context);
            if norm(&before) > 0.0 && norm(&ending) > 0.0 {
                candidates.push(Candidate {
                    bar: a,
                    similarity: cosine(&before, &ending),
                });
            }
        }
    }
    let threshold = rules.min_similarity.get();
    let best = candidates
        .iter()
        .filter(|c| c.similarity >= threshold)
        // The highest; on a tie the earliest, which comes first.
        .fold(None::<&Candidate>, |acc, c| match acc {
            Some(best) if best.similarity >= c.similarity => acc,
            _ => Some(c),
        });
    let (bar, similarity) = best.map_or((0, 0.0), |c| (c.bar, c.similarity));
    // A subtune that ends inside a bar or after silence jumps from somewhere the comparison didn't see: half as sure.
    let clean_end = length.is_multiple_of(rows) && last == whole;
    let confidence = if clean_end {
        similarity
    } else {
        similarity / 2.0
    };
    let row = bar * rows;
    let id = DecisionId::new("loop").expect("a valid id");
    LoopChoice {
        row,
        similarity,
        candidates,
        by: id.clone(),
        unapplied: Vec::new(),
        decision: Decision {
            id,
            subtune: subtune.name.clone(),
            bar,
            proposal: Proposal::LoopRow { row },
            provenance: Provenance::Rules {
                rule: FIND_LOOP.into(),
            },
            confidence: Confidence::new(confidence.clamp(0.0, 1.0)).expect("clamped"),
            reviewed: false,
        },
    }
}

/// Each bar's chroma, summed in voice and row order so the result does not depend on how the song lists its voices.
fn chroma_by_bar(subtune: &Subtune, bars: u32) -> Vec<[f32; 12]> {
    let rows = u32::from(subtune.rows_per_bar.get());
    let length = subtune.length.get();
    let mut chroma = vec![[0.0f32; 12]; bars as usize];
    let mut lines: Vec<_> = subtune.voices.iter().collect();
    lines.sort_by_key(|l| l.voice);
    let mut add = |from: u32, to: u32, semitone: u8, weight: f32| {
        let to = to.min(length);
        let mut row = from;
        while row < to {
            let bar = row / rows;
            let end = ((bar + 1) * rows).min(to);
            chroma[bar as usize][usize::from(semitone % 12)] += (end - row) as f32 * weight;
            row = end;
        }
    };
    for line in lines {
        let mut notes: Vec<&Note> = line.notes.iter().collect();
        notes.sort_by(|a, b| super::canonical(a, b, super::Skyline::Highest));
        let mut arps: Vec<_> = line.arps.iter().collect();
        arps.sort_by_key(|a| a.row);
        for n in &notes {
            let end = n.row.saturating_add(n.length.get());
            let c = n.confidence.get();
            // Under an arpeggio a note sounds as the chord; elsewhere as itself.
            let mut row = n.row;
            for arp in arps
                .iter()
                .filter(|a| a.row < end && n.row < a.row.saturating_add(a.length.get()))
            {
                // Overlapping arpeggios: the rows an earlier one covered are not counted again.
                let from = arp.row.max(row);
                let to = arp.row.saturating_add(arp.length.get()).min(end);
                if from >= to {
                    continue;
                }
                add(row, from, n.pitch.semitone(), c);
                let share = c / arp.intervals.len().max(1) as f32;
                for &i in &arp.intervals {
                    add(
                        from,
                        to,
                        ((u16::from(n.pitch.semitone()) + u16::from(i)) % 12) as u8,
                        share,
                    );
                }
                row = to;
            }
            add(row, end, n.pitch.semitone(), c);
        }
    }
    chroma
}

/// The chroma of `count` bars from `from`, one after another.
fn window(chroma: &[[f32; 12]], from: u32, count: u32) -> Vec<f32> {
    chroma[from as usize..(from + count) as usize]
        .iter()
        .flatten()
        .copied()
        .collect()
}

fn norm(v: &[f32]) -> f32 {
    v.iter().map(|x| x * x).sum::<f32>().sqrt()
}

/// The cosine similarity of two chroma windows: 1 for the same harmony, 0 for none shared.
fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    (dot / (norm(a) * norm(b))).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests;
