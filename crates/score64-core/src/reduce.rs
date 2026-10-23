//! Reduction rules (`docs/TAD.md` §3.2): from transcribed notes to lines the SID can play, with every judgement call
//! recorded as a [`Decision`] (provenance `rules`).
//!
//! [`monophonise`] (#40) makes one voice's notes a single line: ghosts and too-short notes out, one note per row (the
//! skyline), overlaps trimmed, ties that lost their note restarted. [`choose_lead`] (#39) picks the part that
//! carries the melody, bar by bar. [`fold_octaves`] (#41) moves a line's bars by whole octaves into its roles'
//! ranges. [`fold_drums`] and [`interleave_drums`] (#43) put drum hits on a voice of their own or into another line.
//! [`find_loop`] (#44) proposes where a subtune loops back to.
//! [`detect_chords`] and [`arpeggiate`] (#42) find the harmony and play it as arpeggios.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::num::{NonZeroU8, NonZeroU32};

use serde::{Deserialize, Serialize};

use crate::decide::Rank;
use crate::provenance::{Confidence, Decision, DecisionId, DropReason, Proposal, Provenance};
use crate::score::{Attack, Note, SubtuneName, Voice};

pub use crate::decide::Unapplied;
mod chords;
mod lead;
mod looping;
pub use looping::{Candidate, FIND_LOOP, LoopChoice, LoopRules, find_loop};
mod octave;
pub use chords::{
    Arpeggio, CHORDS, Chord, ChordQuality, ChordSpan, MIN_SCORE, arpeggiate, detect_chords,
};
pub use lead::{Bars, CHOOSE_LEAD, LeadChoice, LeadRules, PartSalience, Salience, choose_lead};
pub use octave::{FOLD_OCTAVES, fold_octaves};
mod drums;
pub use drums::{
    DrumKind, DrumKit, DrumSound, FOLD_DRUMS, INTERLEAVE_DRUMS, InterleaveRules, OnClash,
    fold_drums, interleave_drums,
};

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
#[serde(default, deny_unknown_fields)]
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
    /// Every change the rule made to the input, in the order it made them.
    pub decisions: Vec<Decision>,
    /// Override decisions the rule was given but couldn't apply, and why, by id.
    pub unapplied: Vec<(DecisionId, Unapplied)>,
}

/// How sure the rule is of a skyline choice: it is the arrangement's real judgement call.
const SKYLINE_CONFIDENCE: f32 = 0.5;

/// Make `notes` one monophonic line for the voice in `at`, following `rules`:
///
/// 1. Notes less confident than `ghost_below` are dropped as ghosts, and notes shorter than `min_rows` as too short.
/// 2. Of the notes starting on one row, the skyline keeps the highest (or lowest); another of the same pitch is a
///    repeat, merged into the longest of them; the rest are dropped.
/// 3. A note starting less than `min_rows` after another, or before it, is dropped when the other is more confident
///    (the more confident notes are placed first; the decision is as sure as the other note was surer). Then a note running into the next is trimmed to end where the next
///    starts. So every drop is caused by a note that stays, and a note that trimming can keep is kept.
/// 4. A `Legato` note whose predecessor no longer ends on its row starts fresh.
///
/// Every drop, trim and restart is a decision. A trimmed or restarted note records its decision's id unless it
/// already records one (an earlier rule's); the decision names the note by row and pitch either way, and by source
/// span when it has one. Decision ids name what they are about, so they stay put when other notes change:
/// `mono-trim-v<voice>-r<row>`, `mono-tie-v<voice>-r<row>`, and `mono-<reason>-v<voice>-r<row>-p<semitone>` for a
/// drop (`ghost`, `short`, `repeat`, `skyline` or `overlap`), with `-2`, `-3`, ... for further drops of one kind on
/// one row and pitch (those suffixes follow the drops' order, so another drop of the same kind at the same place can
/// renumber them). They are unique within the subtune when it is monophonised once per voice, all its notes in one
/// call. The result does not depend on the order of `notes`.
pub fn monophonise(
    notes: impl IntoIterator<Item = Note>,
    at: &Placement,
    rules: &MonoRules,
) -> Reduced {
    monophonise_with(notes, at, rules, &[])
}

/// [`monophonise`] with a model's or a person's decisions in force (#167): `overrides` are resolved decisions
/// ([`crate::decide::resolve`]'s `chosen`, after [`crate::decide::check`]). Those for this subtune and voice apply;
/// the rules' own, and any of another kind, are left alone.
///
/// - A `drop_note` takes its note (same row, pitch and length) out before the rule starts, as if it had never been
///   there, so the rule's other choices follow from the line without it.
/// - A `trim_note` sets its note's length once the line is built. It can make the note shorter, or as long as the gap
///   to the next note; longer would overlap it, and the override is unapplied (`overlaps`).
/// - A `retrigger` gives its note a fresh start.
///
/// A note an override names that isn't there (a different note is in its place, or the rule dropped it) is reported
/// as `no_such_note`. The rule records no decision of its own for what an override did, and a note an override
/// changed records the override's id instead of the rule's.
pub fn monophonise_with(
    notes: impl IntoIterator<Item = Note>,
    at: &Placement,
    rules: &MonoRules,
    overrides: &[Decision],
) -> Reduced {
    let mut log = Log::new(at, MONOPHONISE, "mono");
    let mut unapplied = Vec::new();
    let mine: Vec<&Decision> = overrides
        .iter()
        .filter(|d| &d.subtune == at.subtune && d.rank() != Rank::Rules)
        .filter(|d| match d.proposal {
            Proposal::DropNote { voice, .. }
            | Proposal::TrimNote { voice, .. }
            | Proposal::Retrigger { voice, .. } => voice == at.voice,
            _ => false,
        })
        .collect();
    let mut notes: Vec<Note> = notes.into_iter().collect();
    notes.sort_by(|a, b| canonical(a, b, rules.skyline));

    // 0. Drops in force: each takes out one note of its row, pitch and length, the first in canonical order.
    for d in &mine {
        if let Proposal::DropNote {
            row, pitch, length, ..
        } = d.proposal
        {
            match notes
                .iter()
                .position(|n| n.row == row && n.pitch == pitch && n.length == length)
            {
                Some(i) => {
                    notes.remove(i);
                }
                None => unapplied.push((d.id.clone(), Unapplied::NoSuchNote)),
            }
        }
    }

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

    // 3. Overlaps. Notes closer than `min_rows` to a more confident one are dropped: the most confident are placed
    // first, and a note joins the line only if it starts at least `min_rows` after the note before it and before the
    // note after it. Then each note is trimmed to end where the next starts. A drop is only ever caused by a note that
    // stays, and every note that can be kept by trimming is.
    let mut by_confidence = heads;
    by_confidence.sort_by(|a, b| {
        b.confidence
            .get()
            .total_cmp(&a.confidence.get())
            .then_with(|| canonical(a, b, rules.skyline))
    });
    let mut placed: BTreeMap<u32, Note> = BTreeMap::new();
    for note in by_confidence {
        let min = rules.min_rows.get();
        let before = placed
            .range(..note.row)
            .next_back()
            .filter(|&(&r, _)| note.row - r < min);
        let after = placed
            .range(note.row.saturating_add(1)..)
            .next()
            .filter(|&(&r, _)| r - note.row < min);
        // The confident neighbour it is too close to, if any: it stays, and this note goes.
        match before
            .into_iter()
            .chain(after)
            .map(|(_, n)| n.confidence.get())
            .reduce(f32::max)
        {
            None => {
                placed.insert(note.row, note);
            }
            Some(winner) => {
                // As sure as the neighbour was surer: a tie is a coin toss, settled by order.
                let sure = Confidence::new((winner - note.confidence.get()).clamp(0.0, 1.0))
                    .expect("clamped");
                log.drop(&note, DropReason::Overlap, sure);
            }
        }
    }
    let mut line: Vec<Note> = placed.into_values().collect();
    for i in 1..line.len() {
        let next = line[i].row;
        let prev = &mut line[i - 1];
        if prev.row.saturating_add(prev.length.get()) > next {
            let length = NonZeroU32::new(next - prev.row).expect("rows are distinct and in order");
            let id = log.record(
                log.id("trim", prev.row),
                prev.row,
                Proposal::TrimNote {
                    voice: at.voice,
                    row: prev.row,
                    pitch: prev.pitch,
                    length,
                    source: prev.source,
                },
                Confidence::CERTAIN,
            );
            prev.length = length;
            prev.decision.get_or_insert(id);
        }
    }

    // Trims and fresh starts in force, on the line as built. A note keeps an earlier rule's decision id, but one
    // this rule gave it gives way to the override's.
    let own =
        |id: &Option<DecisionId>, log: &Log| id.as_ref().is_none_or(|id| log.ids.contains(id));
    for d in &mine {
        match d.proposal {
            Proposal::TrimNote {
                row, pitch, length, ..
            } => {
                let Some(i) = line.iter().position(|n| n.row == row && n.pitch == pitch) else {
                    unapplied.push((d.id.clone(), Unapplied::NoSuchNote));
                    continue;
                };
                let room = line.get(i + 1).map_or(u32::MAX, |next| next.row - row);
                if length.get() > room {
                    unapplied.push((d.id.clone(), Unapplied::Overlaps));
                    continue;
                }
                let n = &mut line[i];
                n.length = length;
                if own(&n.decision, &log) {
                    n.decision = Some(d.id.clone());
                }
            }
            Proposal::Retrigger { row, pitch, .. } => {
                let Some(n) = line.iter_mut().find(|n| n.row == row && n.pitch == pitch) else {
                    unapplied.push((d.id.clone(), Unapplied::NoSuchNote));
                    continue;
                };
                n.attack = Attack::Fresh;
                if own(&n.decision, &log) {
                    n.decision = Some(d.id.clone());
                }
            }
            _ => {}
        }
    }
    // A trim in force replaces the rule's own trim of that note: the line's lengths are the overrides' now.
    log.decisions.retain(|r| match r.proposal {
        Proposal::TrimNote { row, pitch, .. } => !mine.iter().any(|d| {
            matches!(d.proposal, Proposal::TrimNote { row: r2, pitch: p2, .. } if r2 == row && p2 == pitch)
                && !unapplied.iter().any(|(id, _)| id == &d.id)
        }),
        _ => true,
    });

    // 4. Ties that lost the note they were tied to.
    let mut end = None;
    for note in &mut line {
        if note.attack == Attack::Legato && end != Some(note.row) {
            let id = log.record(
                log.id("tie", note.row),
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
        end = Some(note.row.saturating_add(note.length.get()));
    }

    Reduced {
        notes: line,
        decisions: log.decisions,
        unapplied,
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

/// The decisions one call of a rule makes, with ids `<prefix>-<kind>-v<voice>-r<row>` (and `-p<semitone>` for drops),
/// `-2`, `-3`, ... added when one is taken.
struct Log<'a> {
    at: &'a Placement<'a>,
    rule: &'static str,
    prefix: &'static str,
    decisions: Vec<Decision>,
    ids: BTreeSet<DecisionId>,
}

impl<'a> Log<'a> {
    fn new(at: &'a Placement<'a>, rule: &'static str, prefix: &'static str) -> Self {
        Log {
            at,
            rule,
            prefix,
            decisions: Vec::new(),
            ids: BTreeSet::new(),
        }
    }

    /// The id for a decision of `kind` about the note on `row`.
    fn id(&self, kind: &str, row: u32) -> String {
        format!("{}-{kind}-v{}-r{row}", self.prefix, self.at.voice.get())
    }

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
                rule: self.rule.into(),
            },
            confidence,
            reviewed: false,
        });
        id
    }

    fn drop(&mut self, note: &Note, reason: DropReason, confidence: Confidence) {
        let kind = match reason {
            DropReason::Ghost => "ghost",
            DropReason::TooShort => "short",
            DropReason::Repeat => "repeat",
            DropReason::Skyline => "skyline",
            DropReason::Overlap => "overlap",
            DropReason::Clash => "clash",
            DropReason::Unmapped => "unmapped",
            DropReason::Interleave => "interleave",
        };
        let id = format!("{}-p{}", self.id(kind, note.row), note.pitch.semitone());
        self.record(
            id,
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
}

#[cfg(test)]
mod tests;
