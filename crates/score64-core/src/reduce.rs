//! Reduction rules (`docs/TAD.md` §3.2): from transcribed notes to lines the SID can play, with every judgement call
//! recorded as a [`Decision`] (provenance `rules`).
//!
//! [`monophonise`] (#40) makes one voice's notes a single line: ghosts and too-short notes out, one note per row (the
//! skyline), overlaps trimmed, ties that lost their note restarted.

use std::cmp::Ordering;
use std::collections::BTreeSet;
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
/// span when it has one. Decision ids name what they are about, so they stay put when other notes change:
/// `mono-trim-v<voice>-r<row>`, `mono-tie-v<voice>-r<row>` and `mono-drop-v<voice>-r<row>-p<semitone>`, with `-2`,
/// `-3`, ... for further drops of the same pitch on the same row. They are unique within the subtune for one call
/// per voice. The result does not depend on the order of `notes`.
pub fn monophonise(
    notes: impl IntoIterator<Item = Note>,
    at: &Placement,
    rules: &MonoRules,
) -> Reduced {
    let mut log = Log {
        at,
        decisions: Vec::new(),
        ids: BTreeSet::new(),
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

    // 3. Overlaps. Each note on the line keeps its length from before trimming, and this rule's trim of it, so that
    // the trim can be undone when the note after it is dropped: the trim was only for that note.
    let mut line: Vec<OnLine> = Vec::with_capacity(heads.len());
    'notes: for note in heads {
        while let Some(prev) = line.last_mut() {
            if prev.note.row + prev.note.length.get() <= note.row {
                break;
            }
            let gap = note.row - prev.note.row; // at least 1: rows are distinct and in order
            if gap >= rules.min_rows.get() {
                let length = NonZeroU32::new(gap).expect("at least one row");
                let id = log.record(
                    format!("mono-trim-v{}-r{}", at.voice.get(), prev.note.row),
                    prev.note.row,
                    Proposal::TrimNote {
                        voice: at.voice,
                        row: prev.note.row,
                        pitch: prev.note.pitch,
                        source: prev.note.source,
                        length,
                    },
                    Confidence::CERTAIN,
                );
                prev.note.length = length;
                if prev.note.decision.is_none() {
                    prev.note.decision = Some(id.clone());
                }
                prev.trim = Some(id);
                break;
            }
            if prev.note.confidence < note.confidence {
                // The note on top was never trimmed: only a note pushed after it could have done that.
                let gone = line.pop().expect("just looked");
                log.drop(&gone.note, DropReason::Overlap, Confidence::CERTAIN);
                if let Some(below) = line.last_mut() {
                    below.untrim(&mut log);
                }
            } else {
                log.drop(&note, DropReason::Overlap, Confidence::CERTAIN);
                continue 'notes;
            }
        }
        line.push(OnLine {
            length: note.length,
            note,
            trim: None,
        });
    }
    let mut line: Vec<Note> = line.into_iter().map(|l| l.note).collect();

    // 4. Ties that lost the note they were tied to.
    let mut end = None;
    for note in &mut line {
        if note.attack == Attack::Legato && end != Some(note.row) {
            let id = log.record(
                format!("mono-tie-v{}-r{}", at.voice.get(), note.row),
                note.row,
                Proposal::Retrigger {
                    voice: at.voice,
                    row: note.row,
                    pitch: note.pitch,
                    source: note.source,
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

/// A note on the line being built, with its length before this rule trimmed it and the trim's decision.
struct OnLine {
    note: Note,
    length: NonZeroU32,
    trim: Option<DecisionId>,
}

impl OnLine {
    /// Undo this rule's trim, if any: back to the note's own length, without the decision.
    fn untrim(&mut self, log: &mut Log) {
        if let Some(id) = self.trim.take() {
            log.forget(&id);
            self.note.length = self.length;
            if self.note.decision.as_ref() == Some(&id) {
                self.note.decision = None;
            }
        }
    }
}

/// The decisions one call makes.
struct Log<'a> {
    at: &'a Placement<'a>,
    decisions: Vec<Decision>,
    ids: BTreeSet<DecisionId>,
}

impl Log<'_> {
    /// Record a decision under `id`, or `id-2`, `id-3`, ... if that is taken.
    fn record(
        &mut self,
        id: String,
        row: u32,
        proposal: Proposal,
        confidence: Confidence,
    ) -> DecisionId {
        let id = (1..)
            .map(|k| {
                if k == 1 {
                    id.clone()
                } else {
                    format!("{id}-{k}")
                }
            })
            .map(|s| DecisionId::new(s).expect("a valid id"))
            .find(|id| !self.ids.contains(id))
            .expect("a free id");
        self.ids.insert(id.clone());
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
            format!(
                "mono-drop-v{}-r{}-p{}",
                self.at.voice.get(),
                note.row,
                note.pitch.semitone()
            ),
            note.row,
            Proposal::DropNote {
                voice: self.at.voice,
                row: note.row,
                pitch: note.pitch,
                length: note.length,
                source: note.source,
                reason,
            },
            confidence,
        );
    }

    /// Take back the decision `id`.
    fn forget(&mut self, id: &DecisionId) {
        self.decisions.retain(|d| &d.id != id);
        self.ids.remove(id);
    }
}

#[cfg(test)]
mod tests;
