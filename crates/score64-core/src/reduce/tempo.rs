//! Tempo decisions (#167): no rule proposes a tempo, so a model's or a person's `frames_per_row` decision is applied
//! to the subtune directly.

use crate::decide::{Rank, Unapplied};
use crate::provenance::{Decision, DecisionId, Proposal};
use crate::score::{Subtune, TempoChange};

/// Apply the resolved `frames_per_row` decisions for `subtune` that a model or a person made: each sets the tempo
/// from its bar's first row, as the subtune's starting tempo at bar 0 and as a tempo change after that, replacing
/// one already on that row. A decision past the subtune's end is unapplied (`past_end`) and returned; the rules'
/// own, and any of another kind, are left alone.
pub fn apply_tempo(subtune: &mut Subtune, overrides: &[Decision]) -> Vec<(DecisionId, Unapplied)> {
    let mut unapplied = Vec::new();
    let rows = u32::from(subtune.rows_per_bar.get());
    for d in overrides
        .iter()
        .filter(|d| d.subtune == subtune.name && d.rank() != Rank::Rules)
    {
        let Proposal::FramesPerRow { frames_per_row } = d.proposal else {
            continue;
        };
        let row = d.bar.saturating_mul(rows);
        if row >= subtune.length.get() {
            unapplied.push((d.id.clone(), Unapplied::PastEnd));
        } else if row == 0 {
            subtune.frames_per_row = frames_per_row;
        } else {
            subtune.tempo.retain(|t| t.row != row);
            subtune.tempo.push(TempoChange {
                row,
                frames_per_row,
            });
        }
    }
    subtune.tempo.sort_by_key(|t| t.row);
    unapplied
}

#[cfg(test)]
mod tests;
