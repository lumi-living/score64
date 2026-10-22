//! Which decision wins (#45): manual decisions override the LLM's, which override the rules' (`docs/TAD.md` §3.3).
//!
//! Decisions come from the rules (`reduce`), from a model and from a person (`decisions.toml`). Two decisions about
//! the same thing share a [`Slot`]; [`resolve`] keeps one per slot and says which lost, and to what.

use crate::provenance::{Decision, DecisionId, Proposal, Provenance};
use crate::score::{Pitch, SubtuneName, Voice};

/// What a decision is about: two decisions with the same slot compete, and only one applies.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Slot {
    /// A voice's role from a bar.
    VoiceRole {
        /// The subtune.
        subtune: SubtuneName,
        /// The bar.
        bar: u32,
        /// The voice.
        voice: Voice,
    },
    /// The lead's source from a bar.
    LeadSource {
        /// The subtune.
        subtune: SubtuneName,
        /// The bar.
        bar: u32,
    },
    /// A voice's octave shift in a bar.
    Octave {
        /// The subtune.
        subtune: SubtuneName,
        /// The bar.
        bar: u32,
        /// The voice.
        voice: Voice,
    },
    /// The chord from a row.
    Chord {
        /// The subtune.
        subtune: SubtuneName,
        /// The row.
        row: u32,
    },
    /// The subtune's loop row.
    LoopRow {
        /// The subtune.
        subtune: SubtuneName,
    },
    /// The tempo from a bar.
    FramesPerRow {
        /// The subtune.
        subtune: SubtuneName,
        /// The bar.
        bar: u32,
    },
    /// A note's trim.
    TrimNote {
        /// The subtune.
        subtune: SubtuneName,
        /// The voice.
        voice: Voice,
        /// The note's row.
        row: u32,
        /// Its pitch.
        pitch: Pitch,
    },
    /// Whether a note is dropped.
    DropNote {
        /// The subtune.
        subtune: SubtuneName,
        /// The voice.
        voice: Voice,
        /// The note's row.
        row: u32,
        /// Its pitch.
        pitch: Pitch,
    },
    /// A note's fresh start.
    Retrigger {
        /// The subtune.
        subtune: SubtuneName,
        /// The voice.
        voice: Voice,
        /// The note's row.
        row: u32,
        /// Its pitch.
        pitch: Pitch,
    },
}

impl Decision {
    /// What this decision is about.
    pub fn slot(&self) -> Slot {
        let subtune = self.subtune.clone();
        let bar = self.bar;
        match self.proposal {
            Proposal::VoiceRole { voice, .. } => Slot::VoiceRole {
                subtune,
                bar,
                voice,
            },
            Proposal::LeadSource { .. } => Slot::LeadSource { subtune, bar },
            Proposal::Octave { voice, .. } => Slot::Octave {
                subtune,
                bar,
                voice,
            },
            Proposal::Chord { row, .. } => Slot::Chord { subtune, row },
            Proposal::LoopRow { .. } => Slot::LoopRow { subtune },
            Proposal::FramesPerRow { .. } => Slot::FramesPerRow { subtune, bar },
            Proposal::TrimNote {
                voice, row, pitch, ..
            } => Slot::TrimNote {
                subtune,
                voice,
                row,
                pitch,
            },
            Proposal::DropNote {
                voice, row, pitch, ..
            } => Slot::DropNote {
                subtune,
                voice,
                row,
                pitch,
            },
            Proposal::Retrigger {
                voice, row, pitch, ..
            } => Slot::Retrigger {
                subtune,
                voice,
                row,
                pitch,
            },
        }
    }

    /// How strongly this decision claims its slot: a person's (or a model's a person has accepted, ADR-008) over a
    /// model's over the rules'.
    pub fn rank(&self) -> Rank {
        match self.provenance {
            Provenance::Manual => Rank::Person,
            Provenance::Llm { .. } if self.reviewed => Rank::Person,
            Provenance::Llm { .. } => Rank::Model,
            Provenance::Rules { .. } => Rank::Rules,
        }
    }
}

/// Who stands behind a decision, weakest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Rank {
    /// A reduction rule.
    Rules,
    /// A model, not yet reviewed.
    Model,
    /// A person, or a model's decision they accepted.
    Person,
}

/// The decisions that apply, and those that lost.
#[derive(Clone, Debug, PartialEq)]
pub struct Resolution {
    /// One decision per slot, in slot order.
    pub chosen: Vec<Decision>,
    /// Every decision that lost its slot, with the id of the one that won it, in slot order.
    pub overruled: Vec<(Decision, DecisionId)>,
}

/// Keep one decision per slot: the highest-ranked, and of one rank the lowest id, so the order of `decisions` does
/// not matter.
pub fn resolve(decisions: impl IntoIterator<Item = Decision>) -> Resolution {
    let mut all: Vec<(Slot, Decision)> = decisions.into_iter().map(|d| (d.slot(), d)).collect();
    // By slot, then the strongest claim first: the higher rank, then the lower id; then the surer, so even
    // duplicates of one id sort the same way.
    all.sort_by(|(sa, a), (sb, b)| {
        sa.cmp(sb)
            .then(b.rank().cmp(&a.rank()))
            .then_with(|| a.id.cmp(&b.id))
            .then_with(|| b.confidence.get().total_cmp(&a.confidence.get()))
    });
    let mut chosen: Vec<(Slot, Decision)> = Vec::new();
    let mut overruled = Vec::new();
    for (slot, d) in all {
        match chosen.last() {
            Some((s, winner)) if *s == slot => overruled.push((d, winner.id.clone())),
            _ => chosen.push((slot, d)),
        }
    }
    Resolution {
        chosen: chosen.into_iter().map(|(_, d)| d).collect(),
        overruled,
    }
}

#[cfg(test)]
mod tests;
