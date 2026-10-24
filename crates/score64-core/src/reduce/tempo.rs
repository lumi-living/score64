//! Tempo decisions (#167): no rule proposes a tempo, so a model's or a person's `frames_per_row` decision is applied
//! to the subtune directly.

use std::collections::BTreeMap;

use crate::decide::{Rank, Unapplied, by_strength, lost_to};
use crate::provenance::{Decision, DecisionId, Proposal};
use crate::score::{Subtune, TempoChange};

/// What [`apply_tempo`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TempoApplied {
    /// The decision now behind each row's tempo, by row, so the change keeps its provenance.
    pub applied: Vec<(u32, DecisionId)>,
    /// Decisions that couldn't apply, and why, sorted by id.
    pub unapplied: Vec<(DecisionId, Unapplied)>,
}

/// Apply the resolved `frames_per_row` decisions for `subtune` that a model or a person made: each sets the tempo
/// from its bar's first row, as the subtune's starting tempo at bar 0 and as a tempo change after that, replacing
/// one already on that row. A decision that sets the tempo already playing there adds no change (each change is a
/// command in the exported song), but still counts as applied. Changes already in the subtune are left as they are,
/// even when one comes to repeat a tempo a decision set before it.
///
/// A decision past the subtune's end is unapplied (`past_end`). Of several for one bar, the strongest wins (the
/// lowest id on a tie, as `resolve` would); the others are `overruled` or `outranked`. The rules' own decisions, and
/// any of another kind, are left alone. Rule 9 checks the result like any tempo: a gate timer must stay below it.
pub fn apply_tempo(subtune: &mut Subtune, overrides: &[Decision]) -> TempoApplied {
    let mut out = TempoApplied::default();
    let rows = u32::from(subtune.rows_per_bar.get());
    let mut by_bar: BTreeMap<u32, Vec<&Decision>> = BTreeMap::new();
    for d in overrides
        .iter()
        .filter(|d| d.subtune == subtune.name && d.rank() != Rank::Rules)
        .filter(|d| matches!(d.proposal, Proposal::FramesPerRow { .. }))
    {
        by_bar.entry(d.bar).or_default().push(d);
    }
    let mut set = Vec::new();
    for (bar, candidates) in by_bar {
        let ranked = by_strength(candidates);
        let row = bar.saturating_mul(rows);
        if row >= subtune.length.get() {
            out.unapplied
                .extend(ranked.iter().map(|d| (d.id.clone(), Unapplied::PastEnd)));
            continue;
        }
        let winner = ranked[0];
        out.unapplied.extend(
            ranked[1..]
                .iter()
                .map(|d| (d.id.clone(), lost_to(d, winner))),
        );
        let Proposal::FramesPerRow { frames_per_row } = winner.proposal else {
            unreachable!("filtered");
        };
        subtune.tempo.retain(|t| t.row != row);
        if row == 0 {
            subtune.frames_per_row = frames_per_row;
        } else {
            subtune.tempo.push(TempoChange {
                row,
                frames_per_row,
            });
            set.push(row);
        }
        out.applied.push((row, winner.id.clone()));
    }
    subtune.tempo.sort_by_key(|t| t.row);
    // A change a decision made that only repeats the tempo before it is no change at all.
    let mut playing = subtune.frames_per_row;
    subtune.tempo.retain(|t| {
        let keep = !(set.contains(&t.row) && t.frames_per_row == playing);
        if keep {
            playing = t.frames_per_row;
        }
        keep
    });
    out.unapplied.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

#[cfg(test)]
mod tests;
