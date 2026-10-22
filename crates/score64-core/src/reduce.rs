//! Reduction rules (`docs/TAD.md` §3.2): from transcribed notes to lines the SID can play, with every judgement call
//! recorded as a [`Decision`] (provenance `rules`).
//!
//! [`monophonise`] (#40) makes one voice's notes a single line: ghosts and too-short notes out, one note per row (the
//! skyline), overlaps trimmed, ties that lost their note restarted.

use std::cmp::Ordering;
use std::num::{NonZeroU8, NonZeroU32};

use serde::{Deserialize, Serialize};

use crate::provenance::{Confidence, Decision, DecisionId, DropReason, Proposal, Provenance};
use crate::score::{Attack, Note, SubtuneName, Voice};

/// The rule name and version [`monophonise`] records in its decisions.
pub const MONOPHONISE: &str = "monophonise@1";

/// Which note of a chord the skyline keeps.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Skyline {
    /// The highest: a melody's top line.
    #[default]
    Highest,
    /// The lowest: a bass line.
    Lowest,
}

/// The settings of [`monophonise`].
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct MonoRules {
    /// Which note of a chord to keep.
    pub skyline: Skyline,
    /// Notes less sure than this are ghosts and are dropped.
    pub ghost_below: Confidence,
    /// The shortest note to keep, in rows; an overlap that would trim a note shorter drops one of the two instead.
    pub min_rows: NonZeroU32,
}

impl Default for MonoRules {
    /// The top line, ghosts below 0.2, notes of any length.
    fn default() -> Self {
        MonoRules {
            skyline: Skyline::Highest,
            ghost_below: Confidence::new(0.2).expect("in range"),
            min_rows: NonZeroU32::MIN,
        }
    }
}

/// Where the notes go, which the decisions record.
#[derive(Clone, Copy, Debug)]
pub struct Placement<'a> {
    /// The subtune.
    pub subtune: &'a SubtuneName,
    /// The voice.
    pub voice: Voice,
    /// Rows per bar, to give each decision its bar.
    pub rows_per_bar: NonZeroU8,
}

/// A reduced line and the decisions that made it.
#[derive(Clone, Debug, PartialEq)]
pub struct Reduced {
    /// The notes, in row order, one at a time.
    pub notes: Vec<Note>,
    /// Every change made to the input, in the order the rule made them.
    pub decisions: Vec<Decision>,
}

/// How sure the rule is of a skyline choice: it is the arrangement's real judgement call.
const SKYLINE_CONFIDENCE: f32 = 0.5;

/// Make `notes` one monophonic line for the voice in `at`, following `rules`:
///
/// 1. Notes less confident than `ghost_below` are dropped as ghosts, and notes shorter than `min_rows` as too short.
/// 2. Of the notes starting on one row, the skyline keeps the highest (or lowest); another of the same pitch is a
///    repeat, merged into the longest of them; the rest are dropped.
/// 3. A note running into the next is trimmed to end where the next starts. If that would leave it shorter than
///    `min_rows`, the less confident of the two is dropped instead (the later one on a tie).
/// 4. A `Legato` note whose predecessor no longer ends on its row starts fresh.
///
/// Every drop, trim and restart is a decision. A trimmed or restarted note records its decision's id unless it
/// already records one (an earlier rule's); the decision names the note by row and pitch either way, and by source
/// span when it has one. Decision ids (`mono-v<voice>-<n>`) are unique within one call. The result does not depend
/// on the order of `notes`.
pub fn monophonise(
    notes: impl IntoIterator<Item = Note>,
    at: &Placement,
    rules: &MonoRules,
) -> Reduced {
    let mut log = Log {
        at,
        decisions: Vec::new(),
        next: 0,
    };
    let mut notes: Vec<Note> = notes.into_iter().collect();
    notes.sort_by(|a, b| canonical(a, b, rules.skyline));

    // 1. Ghosts and too-short notes.
    let mut kept = Vec::with_capacity(notes.len());
    for note in notes {
        if note.confidence < rules.ghost_below {
            let sure = Confidence::new(1.0 - note.confidence.get()).expect("in range");
            log.drop(&note, DropReason::Ghost, sure);
        } else if note.length < rules.min_rows {
            log.drop(&note, DropReason::TooShort, Confidence::CERTAIN);
        } else {
            kept.push(note);
        }
    }

    // 2. One note per row: the first in canonical order is the skyline's pick, and the longest of its pitch.
    let mut heads: Vec<Note> = Vec::with_capacity(kept.len());
    for note in kept {
        match heads.last() {
            Some(head) if head.row == note.row => {
                let (reason, sure) = if head.pitch == note.pitch {
                    (DropReason::Repeat, Confidence::CERTAIN)
                } else {
                    (
                        DropReason::Skyline,
                        Confidence::new(SKYLINE_CONFIDENCE).expect("in range"),
                    )
                };
                log.drop(&note, reason, sure);
            }
            _ => heads.push(note),
        }
    }

    // 3. Overlaps.
    let mut line: Vec<Note> = Vec::with_capacity(heads.len());
    'notes: for note in heads {
        while let Some(prev) = line.last_mut() {
            if prev.row + prev.length.get() <= note.row {
                break;
            }
            let gap = note.row - prev.row; // at least 1: rows are distinct and in order
            if gap >= rules.min_rows.get() {
                let length = NonZeroU32::new(gap).expect("at least one row");
                let id = log.record(
                    prev.row,
                    Proposal::TrimNote {
                        voice: at.voice,
                        row: prev.row,
                        pitch: prev.pitch,
                        source: prev.source,
                        length,
                    },
                    Confidence::CERTAIN,
                );
                prev.length = length;
                prev.decision.get_or_insert(id);
                break;
            }
            if prev.confidence < note.confidence {
                let prev = line.pop().expect("just looked");
                log.unrecord(&prev);
                log.drop(&prev, DropReason::Overlap, Confidence::CERTAIN);
            } else {
                log.drop(&note, DropReason::Overlap, Confidence::CERTAIN);
                continue 'notes;
            }
        }
        line.push(note);
    }

    // 4. Ties that lost the note they were tied to.
    let mut end = None;
    for note in &mut line {
        if note.attack == Attack::Legato && end != Some(note.row) {
            let id = log.record(
                note.row,
                Proposal::Retrigger {
                    voice: at.voice,
                    row: note.row,
                    pitch: note.pitch,
                },
                Confidence::CERTAIN,
            );
            note.attack = Attack::Fresh;
            note.decision.get_or_insert(id);
        }
        end = Some(note.row + note.length.get());
    }

    Reduced {
        notes: line,
        decisions: log.decisions,
    }
}

/// The order the rule works in: by row; then the skyline's preference; then longer, more confident, fresh before
/// legato; then by source, instrument and decision, so that no two different notes tie.
fn canonical(a: &Note, b: &Note, skyline: Skyline) -> Ordering {
    let pitch = match skyline {
        Skyline::Highest => b.pitch.cmp(&a.pitch),
        Skyline::Lowest => a.pitch.cmp(&b.pitch),
    };
    let source = |n: &Note| n.source.map(|s| (s.content(), s.start_us(), s.end_us()));
    a.row
        .cmp(&b.row)
        .then(pitch)
        .then(b.length.cmp(&a.length))
        .then(b.confidence.get().total_cmp(&a.confidence.get()))
        .then((a.attack == Attack::Legato).cmp(&(b.attack == Attack::Legato)))
        .then_with(|| source(a).cmp(&source(b)))
        .then_with(|| a.instrument.cmp(&b.instrument))
        .then_with(|| a.decision.cmp(&b.decision))
}

/// The decisions one call makes.
struct Log<'a> {
    at: &'a Placement<'a>,
    decisions: Vec<Decision>,
    next: u32,
}

impl Log<'_> {
    fn record(&mut self, row: u32, proposal: Proposal, confidence: Confidence) -> DecisionId {
        let id = DecisionId::new(format!("mono-v{}-{}", self.at.voice.get(), self.next))
            .expect("a valid id");
        self.next += 1;
        self.decisions.push(Decision {
            id: id.clone(),
            subtune: self.at.subtune.clone(),
            bar: row / u32::from(self.at.rows_per_bar.get()),
            proposal,
            provenance: Provenance::Rules {
                rule: MONOPHONISE.into(),
            },
            confidence,
            reviewed: false,
        });
        id
    }

    fn drop(&mut self, note: &Note, reason: DropReason, confidence: Confidence) {
        self.record(
            note.row,
            Proposal::DropNote {
                voice: self.at.voice,
                row: note.row,
                pitch: note.pitch,
                source: note.source,
                reason,
            },
            confidence,
        );
    }

    /// Forget this call's trim of `note`, which is about to be dropped after all.
    fn unrecord(&mut self, note: &Note) {
        self.decisions.retain(|d| {
            !matches!(d.proposal, Proposal::TrimNote { row, pitch, .. } if row == note.row && pitch == note.pitch)
        });
    }
}

#[cfg(test)]
mod tests;
