//! Which decision wins (#45): manual decisions override the LLM's, which override the rules' (`docs/TAD.md` §3.3).
//!
//! Decisions come from the rules (`reduce`), from a model and from a person (`decisions.toml`). Two decisions about
//! the same thing share a [`Slot`]; [`resolve`] keeps one per slot and says which lost, and to what.

use std::collections::BTreeMap;
use std::num::NonZeroU32;

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
    /// The chord from a row (rows count from the subtune's start, whatever bar the decision names).
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
    /// Whether a note is dropped. The length tells two repeats on one row apart.
    DropNote {
        /// The subtune.
        subtune: SubtuneName,
        /// The voice.
        voice: Voice,
        /// The note's row.
        row: u32,
        /// Its pitch.
        pitch: Pitch,
        /// Its length.
        length: NonZeroU32,
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
                voice,
                row,
                pitch,
                length,
                ..
            } => Slot::DropNote {
                subtune,
                voice,
                row,
                pitch,
                length,
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

impl Slot {
    /// For the kinds that hold from their bar until the next decision of the kind (voice role, lead source, tempo),
    /// what they share across bars: a stronger decision in force at a bar overrules a weaker one made there.
    fn holds_from_bar(&self) -> Option<(SubtuneName, u8, Option<Voice>, u32)> {
        match self {
            Slot::VoiceRole {
                subtune,
                bar,
                voice,
            } => Some((subtune.clone(), 0, Some(*voice), *bar)),
            Slot::LeadSource { subtune, bar } => Some((subtune.clone(), 1, None, *bar)),
            Slot::FramesPerRow { subtune, bar } => Some((subtune.clone(), 2, None, *bar)),
            _ => None,
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

/// Keep one decision per slot: the highest-ranked, and of one rank the lowest id (ids compare as text, so `lead-b10`
/// comes before `lead-b2`), then the surer, then by the rest of the decision, so the order of `decisions` never
/// matters.
///
/// Voice roles, lead sources and tempos hold from their bar until the next decision of their kind, as
/// `choose_lead` treats lead sources: a decision made at a bar where a stronger one is already in force (made at that
/// bar or earlier, for the same voice) is overruled by it. So a person's choice holds over any later model's or
/// rules', until the person decides again.
pub fn resolve(decisions: impl IntoIterator<Item = Decision>) -> Resolution {
    let mut all: Vec<(Slot, Decision)> = decisions.into_iter().map(|d| (d.slot(), d)).collect();
    all.sort_by(|(sa, a), (sb, b)| {
        sa.cmp(sb)
            .then(b.rank().cmp(&a.rank()))
            .then_with(|| a.id.cmp(&b.id))
            .then_with(|| b.confidence.get().total_cmp(&a.confidence.get()))
            .then_with(|| format!("{a:?}").cmp(&format!("{b:?}")))
    });
    // One pass in slot order. In a slot the strongest comes first and wins. For the kinds that hold from a bar,
    // slots sort by subtune and then bar, so each kind's bars come in order: one record per kind (and voice) of the
    // decision in force so far, which overrules anything weaker after it, so every loser names what actually beat it.
    let mut chosen: Vec<(Slot, Decision)> = Vec::new();
    let mut overruled = Vec::new();
    let mut in_force: BTreeMap<(SubtuneName, u8, Option<Voice>), (Rank, DecisionId)> =
        BTreeMap::new();
    for (slot, d) in all {
        let family = slot
            .holds_from_bar()
            .map(|(subtune, kind, voice, _)| (subtune, kind, voice));
        let force = family.as_ref().and_then(|f| in_force.get(f));
        if let Some((_, by)) = force.filter(|(rank, _)| *rank > d.rank()) {
            overruled.push((d, by.clone()));
        } else if let Some((_, winner)) = chosen.last().filter(|(s, _)| *s == slot) {
            overruled.push((d, winner.id.clone()));
        } else {
            if let Some(f) = family {
                in_force.insert(f, (d.rank(), d.id.clone()));
            }
            chosen.push((slot, d));
        }
    }
    overruled.sort_by(|(a, _), (b, _)| a.slot().cmp(&b.slot()).then_with(|| a.id.cmp(&b.id)));
    Resolution {
        chosen: chosen.into_iter().map(|(_, d)| d).collect(),
        overruled,
    }
}

#[cfg(test)]
mod tests;
