//! Which decision wins (#45): manual decisions override the LLM's, which override the rules' (`docs/TAD.md` §3.3).
//!
//! Decisions come from the rules (`reduce`), from a model and from a person (`decisions.toml`). Two decisions about
//! the same thing share a [`Slot`]; [`resolve`] keeps one per slot and says which lost, and to what. [`check`] runs
//! first, so a decision that can't apply to the song never holds a slot (#167).

use std::collections::BTreeMap;
use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};

use crate::provenance::{ContentHash, Decision, DecisionId, Proposal, Provenance};
use crate::score::{Pitch, Song, SubtuneName, Voice};

/// Why a decision was not applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Unapplied {
    /// Its source is not one of the parts.
    UnknownSource,
    /// Its bar, or the row it names, is past the end of the subtune.
    PastEnd,
    /// A model's choice where a person's is in force.
    Overruled,
    /// Another override of the same rank on the same bar has the lower id, and wins.
    Outranked,
    /// Its subtune isn't in the song.
    UnknownSubtune,
    /// The row it names isn't in its bar.
    WrongBar,
    /// A value it proposes can't be played: an octave shift beyond ±7, a chord root above 11, a chord whose intervals
    /// don't start at 0 and rise, or one whose top note would be past G#7 even on its lowest root.
    OutOfRange,
    /// The note it names isn't there when the rule runs.
    NoSuchNote,
    /// The trim it proposes would make the note longer than the rule left it; a trim only shortens.
    Lengthens,
    /// The loop row it proposes falls inside a note, or on a tie, which can't play after the jump back.
    InsideNote,
}

/// Decisions competing for one slot, in the order a rule tries them: [`strength`]'s, as [`resolve`] would choose.
pub(crate) fn by_strength(mut ds: Vec<&Decision>) -> Vec<&Decision> {
    ds.sort_by(|a, b| strength(a, b));
    ds
}

/// The largest octave shift a decision may propose, either way: enough to cross GoatTracker's whole range.
pub const MAX_OCTAVE_SHIFT: i8 = 7;

/// Decisions checked against a song: those that can apply, and the others with why they can't. Whole decisions,
/// since an id is only unique within its subtune.
#[derive(Clone, Debug, PartialEq)]
pub struct Checked {
    /// The decisions that can apply, in input order.
    pub valid: Vec<Decision>,
    /// The others, in input order.
    pub unapplied: Vec<(Decision, Unapplied)>,
}

/// Check `decisions` against `song` before [`resolve`]: each must name a subtune of the song, a bar inside it, rows
/// inside both and inside its own bar, playable values, and, for a lead source, one of that subtune's parts in
/// `sources` (a subtune with no entry has none, so every lead source there is refused). A stronger decision that
/// can't apply would otherwise hold its slot and keep a valid weaker one out.
///
/// The rules make only decisions that pass, about notes that start inside the subtune; parts should be clipped to
/// the subtune before they are reduced, or a rule's decision about a note past the end comes back `past_end`. What
/// only a reduction can see is left to it: whether the note a `trim_note`, `drop_note`, `retrigger` or `keep_note`
/// names is there, and whether a trim fits.
pub fn check(
    decisions: impl IntoIterator<Item = Decision>,
    song: &Song,
    sources: &BTreeMap<SubtuneName, Vec<ContentHash>>,
) -> Checked {
    let mut checked = Checked {
        valid: Vec::new(),
        unapplied: Vec::new(),
    };
    for d in decisions {
        let parts = sources.get(&d.subtune).map_or(&[][..], Vec::as_slice);
        match why_not(&d, song, parts) {
            None => checked.valid.push(d),
            Some(why) => checked.unapplied.push((d, why)),
        }
    }
    checked
}

/// Why `d` can't apply to `song`, if it can't.
fn why_not(d: &Decision, song: &Song, sources: &[ContentHash]) -> Option<Unapplied> {
    let Some((_, subtune)) = song.subtune(&d.subtune) else {
        return Some(Unapplied::UnknownSubtune);
    };
    let length = subtune.length.get();
    let rows_per_bar = u32::from(subtune.rows_per_bar.get());
    if d.bar >= length.div_ceil(rows_per_bar) {
        return Some(Unapplied::PastEnd);
    }
    // A row the proposal names must be in the subtune, and in the decision's bar.
    let in_bar = |row: u32| {
        if row >= length {
            Some(Unapplied::PastEnd)
        } else if row / rows_per_bar != d.bar {
            Some(Unapplied::WrongBar)
        } else {
            None
        }
    };
    match &d.proposal {
        Proposal::LeadSource { source } => {
            (!sources.contains(source)).then_some(Unapplied::UnknownSource)
        }
        Proposal::Octave { shift, .. } => (shift.unsigned_abs() > MAX_OCTAVE_SHIFT.unsigned_abs())
            .then_some(Unapplied::OutOfRange),
        Proposal::Chord {
            row,
            root,
            intervals,
        } => in_bar(*row).or_else(|| {
            let rising = intervals.first() == Some(&0) && intervals.windows(2).all(|w| w[0] < w[1]);
            // The lowest root of its pitch class must leave room for the top note, as `arpeggiate` needs.
            let top = intervals.last().map_or(0, |&t| u16::from(t));
            let fits = u16::from(*root) + top <= u16::from(Pitch::MAX.semitone());
            (*root > 11 || !rising || !fits).then_some(Unapplied::OutOfRange)
        }),
        // A loop row may be any row of the subtune; the decision's bar is the one it lands in.
        Proposal::LoopRow { row } => in_bar(*row),
        Proposal::TrimNote { row, .. }
        | Proposal::DropNote { row, .. }
        | Proposal::Retrigger { row, .. }
        | Proposal::KeepNote { row, .. } => in_bar(*row),
        Proposal::VoiceRole { .. } | Proposal::FramesPerRow { .. } => None,
    }
}

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
    /// The chord from a row (rows count from the subtune's start; [`check`] requires the row to be in the decision's
    /// bar).
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
    /// Whether a note on a row and pitch is kept. Keeps of one row and pitch compete with each other; the drops of
    /// notes there are slots of their own, which the reduction that applies them settles with the keep: see
    /// [`resolve`].
    KeepNote {
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
            Proposal::KeepNote { voice, row, pitch } => Slot::KeepNote {
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

/// The order decisions for one slot are tried in: the highest-ranked first, and of one rank the lowest id (ids compare
/// as text), then the surer, then by the rest of the decision, so the order they come in never matters. [`resolve`]
/// and the rules that apply decisions in force use it alike.
pub(crate) fn strength(a: &Decision, b: &Decision) -> std::cmp::Ordering {
    b.rank()
        .cmp(&a.rank())
        .then_with(|| a.id.cmp(&b.id))
        .then_with(|| b.confidence.get().total_cmp(&a.confidence.get()))
        .then_with(|| format!("{a:?}").cmp(&format!("{b:?}")))
}

/// Why `loser` didn't apply when `winner` took the slot: a weaker decision is overruled, an equal one outranked.
pub(crate) fn lost_to(loser: &Decision, winner: &Decision) -> Unapplied {
    if loser.rank() < winner.rank() {
        Unapplied::Overruled
    } else {
        Unapplied::Outranked
    }
}

/// The slots claimed so far, for a rule that applies decisions in force one by one, each slot's strongest first (in
/// [`strength`]'s order), as [`resolve`] chooses: the first to claim a slot holds it, whether or not it then applies.
#[derive(Debug, Default)]
pub(crate) struct Claims<'d>(BTreeMap<Slot, &'d Decision>);

impl<'d> Claims<'d> {
    /// Claim `d`'s slot. `None` when it was free, and `d` now holds it; otherwise why `d` lost to the one that does.
    pub(crate) fn claim(&mut self, d: &'d Decision) -> Option<Unapplied> {
        match self.0.entry(d.slot()) {
            std::collections::btree_map::Entry::Occupied(held) => Some(lost_to(d, held.get())),
            std::collections::btree_map::Entry::Vacant(free) => {
                free.insert(d);
                None
            }
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
///
/// A `keep_note` and the drops of notes on its row and pitch are separate slots, as a drop and a trim of one note
/// are, so both are chosen (ADR-014 §8). Whether they conflict depends on the notes there, which only the reduction
/// that applies them sees: a keep is met by any note left on its row and pitch, so it keeps the longest one no drop
/// takes out, and only when the drops take out every note there does the stronger of the keep and those drops win.
pub fn resolve(decisions: impl IntoIterator<Item = Decision>) -> Resolution {
    let mut all: Vec<(Slot, Decision)> = decisions.into_iter().map(|d| (d.slot(), d)).collect();
    all.sort_by(|(sa, a), (sb, b)| sa.cmp(sb).then_with(|| strength(a, b)));
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
