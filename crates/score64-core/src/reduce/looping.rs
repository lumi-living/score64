//! The loop finder (#44): where a subtune should jump back to when it reaches its end.
//!
//! A subtune plays to its last row and then carries on from `loop_row`. A good loop row is a bar start after which
//! the music once went the way the ending goes: the bars just before it sound like the subtune's last bars, so the
//! jump sounds like the music continuing. Bars are compared by chroma, the weight of each of the 12 pitch classes.

use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};

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
    /// The similarity a candidate needs, 0 to 1; with none that good the subtune loops to its start.
    pub min_similarity: f32,
}

impl Default for LoopRules {
    /// Two bars of context; a similarity of 0.8.
    fn default() -> Self {
        LoopRules {
            context: NonZeroU32::new(2).expect("non-zero"),
            min_similarity: 0.8,
        }
    }
}

/// Where [`find_loop`] would loop, and why.
#[derive(Clone, Debug, PartialEq)]
pub struct LoopChoice {
    /// The loop row: a bar start.
    pub row: u32,
    /// How alike the bars before it and the ending are, 0 to 1; 0 for a loop to the start that no bar earned.
    pub similarity: f32,
    /// Every candidate bar and its similarity, in bar order, for review.
    pub candidates: Vec<(u32, f32)>,
    /// The `LoopRow` decision (`loop`).
    pub decision: Decision,
}

/// Find the loop row of `subtune`.
///
/// Every bar `a` from 1 on whose `context` bars before it end before the subtune's last `context` bars start is a
/// candidate: its score is the cosine similarity of those bars' chroma (all voices, by rows sounding times
/// confidence, arpeggios by their chord tones) with the last bars'. The best candidate at or above `min_similarity`
/// wins, the earliest on a tie (the longest loop); with none, or a subtune too short to compare, the loop row is 0.
/// Candidates whose bars are silent are skipped. One [`Proposal::LoopRow`] decision proposes it, as sure as the
/// similarity; the subtune itself is not changed.
pub fn find_loop(subtune: &Subtune, rules: &LoopRules) -> LoopChoice {
    let rows = u32::from(subtune.rows_per_bar.get());
    let length = subtune.length.get();
    let bars = length.div_ceil(rows);
    let chroma = chroma_by_bar(subtune, bars);
    let context = rules.context.get();
    let ending = bars
        .checked_sub(context)
        .map(|from| window(&chroma, from, context));
    let mut candidates = Vec::new();
    if let Some(ending) = ending.filter(|e| norm(e) > 0.0) {
        for a in context.max(1)..=bars - context {
            let before = window(&chroma, a - context, context);
            if norm(&before) > 0.0 {
                candidates.push((a, cosine(&before, &ending)));
            }
        }
    }
    let threshold = rules.min_similarity.clamp(0.0, 1.0);
    let best = candidates
        .iter()
        .filter(|&&(_, s)| s >= threshold)
        // The highest; on a tie the earliest, which comes first.
        .fold(None::<(u32, f32)>, |acc, &(a, s)| match acc {
            Some((_, best)) if best >= s => acc,
            _ => Some((a, s)),
        });
    let (bar, similarity) = best.unwrap_or((0, 0.0));
    let row = bar * rows;
    LoopChoice {
        row,
        similarity,
        candidates,
        decision: Decision {
            id: DecisionId::new("loop").expect("a valid id"),
            subtune: subtune.name.clone(),
            bar,
            proposal: Proposal::LoopRow { row },
            provenance: Provenance::Rules {
                rule: FIND_LOOP.into(),
            },
            confidence: Confidence::new(similarity.clamp(0.0, 1.0)).expect("clamped"),
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
        notes.sort_by_key(|n| (n.row, n.pitch));
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
                let from = arp.row.max(n.row);
                let to = arp.row.saturating_add(arp.length.get()).min(end);
                add(row, from, n.pitch.semitone(), c);
                let share = c / arp.intervals.len().max(1) as f32;
                for &i in &arp.intervals {
                    add(from, to, n.pitch.semitone().saturating_add(i), share);
                }
                row = row.max(to);
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
