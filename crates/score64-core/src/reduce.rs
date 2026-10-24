//! Reduction rules (`docs/TAD.md` §3.2): from transcribed notes to lines the SID can play, with every judgement call
//! recorded as a [`Decision`] (provenance `rules`).
//!
//! [`monophonise`] (#40) makes one voice's notes a single line: ghosts and too-short notes out, one note per row (the
//! skyline), overlaps trimmed, ties that lost their note restarted. [`choose_lead`] (#39) picks the part that
//! carries the melody, bar by bar. [`fold_octaves`] (#41) moves a line's bars by whole octaves into its roles'
//! ranges. [`fold_drums`] and [`interleave_drums`] (#43) put drum hits on a voice of their own or into another line.
//! [`find_loop`] (#44) proposes where a subtune loops back to.
//! [`monophonise_with`], [`fold_octaves_with`] and [`find_loop_with`] apply a model's or a person's resolved
//! decisions in force (#167), as [`choose_lead`] does its overrides, and [`apply_tempo`] applies tempo decisions,
//! which no rule makes.
//! [`detect_chords`] and [`arpeggiate`] (#42) find the harmony and play it as arpeggios.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::num::{NonZeroU8, NonZeroU32};

use serde::{Deserialize, Serialize};

use crate::decide::Rank;
use crate::decide::{lost_to, strength};
use crate::provenance::{Confidence, Decision, DecisionId, DropReason, Proposal, Provenance};
use crate::score::{Attack, Note, Pitch, SubtuneName, Voice};

pub use crate::decide::Unapplied;
mod chords;
mod lead;
mod looping;
pub use looping::{Candidate, FIND_LOOP, LoopChoice, LoopRules, find_loop, find_loop_with};
mod tempo;
pub use tempo::{TempoApplied, apply_tempo};
mod octave;
pub use chords::{
    Arpeggio, CHORDS, Chord, ChordQuality, ChordSpan, MIN_SCORE, arpeggiate, detect_chords,
};
pub use lead::{Bars, CHOOSE_LEAD, LeadChoice, LeadRules, PartSalience, Salience, choose_lead};
pub use octave::{FOLD_OCTAVES, fold_octaves, fold_octaves_with};
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
    /// Override decisions the rule was given but couldn't apply, and why, sorted by id.
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
/// - A `trim_note` sets its note's length once the line is built. It only shortens: one longer than the rule left
///   the note is unapplied (`lengthens`). Shorter than `min_rows` is a person's call: an unreviewed model's is
///   unapplied (`out_of_range`).
/// - A `retrigger` gives its note a fresh start.
/// - A `keep_note` keeps a note on its row and pitch that the rule would drop (ADR-014 §8). The rule runs as usual
///   with the kept note in place, so the notes around it give way by the rule's own logic:
///   - **ghost, too short:** of a kept row and pitch where no note passes step 1, the first (the longest) stays.
///     Shorter than `min_rows` is a person's call: an unreviewed model's keep of it is unapplied (`out_of_range`).
///   - **skyline:** the kept note is the one its row keeps, and the row's other notes are dropped as usual (`repeat`
///     or `skyline`). A repeat isn't undone: repeats of one pitch on one row merge into the longest, which is the note
///     kept; to keep a shorter one, drop the longer ones too.
///   - **overlap:** kept notes are placed first, so a note closer than `min_rows` to one is dropped (`overlap`) even
///     when it is the surer, at the usual confidence, the gap to the note that stays (0 when the rule alone wouldn't
///     drop it). Then notes running into a kept note are trimmed, and a kept note running into the next is too, as
///     the rule trims any note.
///
///   Of two kept notes that can't both stay (on one row, or closer than `min_rows`), the stronger keep's stays and
///   the other's is dropped as the rule would, its keep `overruled` or `outranked`. A kept note records the keep's id
///   when the keep saved it, unless it already records one; a trim or fresh start in force on it then leaves that id.
///   Keeping a note the rule keeps anyway changes nothing.
/// - A `drop_note` gives way (`overruled`) to a stronger trim or fresh start of the same note, even one that then
///   can't apply: those are separate slots, so `resolve` keeps all three, and a stronger decision about the note
///   means it should stay. It takes the note from its source span when it names one; of identical notes otherwise,
///   the one the rule would drop.
/// - A `keep_note` and the drops of notes on its row and pitch are one slot, as for `resolve`: the strongest of them
///   all decides, and those of the other kind lose to it.
/// - Of several decisions for one slot (as when `resolve` hasn't run), the strongest claims it, whether or not it then
///   applies, and the others are `overruled` or `outranked`: the result is the one `resolve` and then this rule give.
///
/// A note an override names that isn't there (a different note is in its place, or the rule dropped it) is reported
/// as `no_such_note`. The rule records no decision of its own for what an override did. A note an override changed
/// records the override's id in place of this rule's; an id from an earlier rule stays. The order of `overrides`
/// doesn't matter.
pub fn monophonise_with(
    notes: impl IntoIterator<Item = Note>,
    at: &Placement,
    rules: &MonoRules,
    overrides: &[Decision],
) -> Reduced {
    let mut log = Log::new(at, MONOPHONISE, "mono");
    let mut unapplied = Vec::new();
    let mut mine: Vec<&Decision> = overrides
        .iter()
        .filter(|d| &d.subtune == at.subtune && d.rank() != Rank::Rules)
        .filter(|d| match d.proposal {
            Proposal::DropNote { voice, .. }
            | Proposal::TrimNote { voice, .. }
            | Proposal::Retrigger { voice, .. }
            | Proposal::KeepNote { voice, .. } => voice == at.voice,
            _ => false,
        })
        .collect();
    // In slot order, strongest first, so the order they come in never matters. Of several for one slot, the strongest
    // claims it and the rest lose to it, as `resolve` would choose.
    mine.sort_by(|a, b| a.slot().cmp(&b.slot()).then_with(|| strength(a, b)));
    let mut won: BTreeMap<crate::decide::Slot, &Decision> = BTreeMap::new();
    let mut notes: Vec<Note> = notes.into_iter().collect();
    notes.sort_by(|a, b| canonical(a, b, rules.skyline));

    // A keep and the drops of notes on its row and pitch are one slot: the strongest of them all decides, and those
    // of the other kind lose to it.
    let mut stays: BTreeMap<(u32, Pitch), &Decision> = BTreeMap::new();
    for d in &mine {
        if let Proposal::DropNote { row, pitch, .. } | Proposal::KeepNote { row, pitch, .. } =
            d.proposal
        {
            stays
                .entry((row, pitch))
                .and_modify(|best| {
                    if strength(d, best).is_lt() {
                        *best = d;
                    }
                })
                .or_insert(d);
        }
    }
    let is_keep = |d: &Decision| matches!(d.proposal, Proposal::KeepNote { .. });
    let beaten = |d: &Decision, row: u32, pitch: Pitch| {
        stays
            .get(&(row, pitch))
            .copied()
            .filter(|s| is_keep(s) != is_keep(d))
    };

    // 0. Drops in force. A drop, a trim and a fresh start of one note are separate slots, so `resolve` keeps all
    // three; a drop gives way when a stronger decision trims or restarts the note instead.
    for d in &mine {
        let Proposal::DropNote {
            row,
            pitch,
            length,
            source,
            ..
        } = d.proposal
        else {
            continue;
        };
        if let Some(w) = beaten(d, row, pitch).or_else(|| won.get(&d.slot()).copied()) {
            unapplied.push((d.id.clone(), lost_to(d, w)));
            continue;
        }
        // The strongest claims its slot whether or not it then applies, as `resolve` would.
        won.insert(d.slot(), d);
        let stronger = mine.iter().any(|o| {
            o.rank() > d.rank()
                && matches!(o.proposal,
                    Proposal::TrimNote { row: r, pitch: p, .. } | Proposal::Retrigger { row: r, pitch: p, .. }
                    if r == row && p == pitch)
        });
        if stronger {
            unapplied.push((d.id.clone(), Unapplied::Overruled));
            continue;
        }
        // Its note: the one from its source span when it names one; otherwise the last of its row, pitch and
        // length in canonical order, which is the one the rule would drop of identical twins.
        let found = notes.iter().rposition(|n| {
            n.row == row
                && n.pitch == pitch
                && n.length == length
                && (source.is_none() || n.source == source)
        });
        match found {
            Some(i) => {
                notes.remove(i);
            }
            None => unapplied.push((d.id.clone(), Unapplied::NoSuchNote)),
        }
    }

    // Keeps in force, by row and pitch: those that won their slot and have a note there. No drop of their note is in
    // force, since a keep that wins beats them all.
    let mut keeps: BTreeMap<(u32, Pitch), &Decision> = BTreeMap::new();
    for d in &mine {
        let Proposal::KeepNote { row, pitch, .. } = d.proposal else {
            continue;
        };
        if let Some(w) = beaten(d, row, pitch).or_else(|| won.get(&d.slot()).copied()) {
            unapplied.push((d.id.clone(), lost_to(d, w)));
            continue;
        }
        won.insert(d.slot(), d);
        if notes.iter().any(|n| n.row == row && n.pitch == pitch) {
            keeps.insert((row, pitch), d);
        } else {
            unapplied.push((d.id.clone(), Unapplied::NoSuchNote));
        }
    }
    fn kept_by<'d>(n: &Note, keeps: &BTreeMap<(u32, Pitch), &'d Decision>) -> Option<&'d Decision> {
        keeps.get(&(n.row, n.pitch)).copied()
    }
    // The kept notes the keep saved from a drop of the rule's, which record the keep's id.
    let mut saved: BTreeSet<(u32, Pitch)> = BTreeSet::new();

    // 1. Ghosts and too-short notes. A kept row and pitch where no note passes keeps its first note, the longest.
    let passes = |n: &Note| n.confidence >= rules.ghost_below && n.length >= rules.min_rows;
    let mut rescue: BTreeSet<(u32, Pitch)> = BTreeSet::new();
    keeps.retain(|&(row, pitch), d| {
        let mut there = notes.iter().filter(|n| n.row == row && n.pitch == pitch);
        let first = there.clone().next().expect("a keep in force has its note");
        if there.any(passes) {
            return true;
        }
        // Shorter than the rule allows is a person's call, not a model's, as for a trim.
        if first.length < rules.min_rows && d.rank() < Rank::Person {
            unapplied.push((d.id.clone(), Unapplied::OutOfRange));
            return false;
        }
        rescue.insert((row, pitch));
        true
    });
    let mut kept = Vec::with_capacity(notes.len());
    for note in notes {
        if !passes(&note) && rescue.remove(&(note.row, note.pitch)) {
            saved.insert((note.row, note.pitch));
            kept.push(note);
        } else if note.confidence < rules.ghost_below {
            let sure = Confidence::new(1.0 - note.confidence.get()).expect("in range");
            log.drop(&note, DropReason::Ghost, sure);
        } else if note.length < rules.min_rows {
            log.drop(&note, DropReason::TooShort, Confidence::CERTAIN);
        } else {
            kept.push(note);
        }
    }

    // 2. One note per row: the first in canonical order is the skyline's pick, and the longest of its pitch. A kept
    // pitch on the row is picked instead, the strongest keep's if there are several; the other keeps there lose to it.
    let mut heads: Vec<Note> = Vec::with_capacity(kept.len());
    let mut kept = kept.into_iter().peekable();
    while let Some(first) = kept.next() {
        let row = first.row;
        let mut on_row = vec![first];
        while let Some(n) = kept.next_if(|n| n.row == row) {
            on_row.push(n);
        }
        // `min_by` takes the first of equals: of a kept pitch's notes, the longest.
        let pick = on_row
            .iter()
            .enumerate()
            .filter_map(|(i, n)| kept_by(n, &keeps).map(|d| (i, d)))
            .min_by(|a, b| strength(a.1, b.1));
        let pick = match pick {
            Some((i, winner)) => {
                let pitch = on_row[i].pitch;
                if i != 0 {
                    saved.insert((row, pitch));
                }
                let others: BTreeSet<Pitch> = on_row
                    .iter()
                    .map(|n| n.pitch)
                    .filter(|&p| p != pitch)
                    .collect();
                for p in others {
                    if let Some(d) = keeps.remove(&(row, p)) {
                        unapplied.push((d.id.clone(), lost_to(d, winner)));
                    }
                }
                i
            }
            None => 0,
        };
        let pitch = on_row[pick].pitch;
        for (i, note) in on_row.into_iter().enumerate() {
            if i == pick {
                heads.push(note);
                continue;
            }
            let (reason, sure) = if note.pitch == pitch {
                (DropReason::Repeat, Confidence::CERTAIN)
            } else {
                (
                    DropReason::Skyline,
                    Confidence::new(SKYLINE_CONFIDENCE).expect("in range"),
                )
            };
            log.drop(&note, reason, sure);
        }
    }

    // 3. Overlaps. Notes closer than `min_rows` to a more confident one are dropped: the most confident are placed
    // first, and a note joins the line only if it starts at least `min_rows` after the note before it and before the
    // note after it. Then each note is trimmed to end where the next starts. A drop is only ever caused by a note that
    // stays, and every note that can be kept by trimming is. Kept notes are placed before the rest, the strongest keep
    // first.
    let min = rules.min_rows.get();
    let mut by_confidence = heads;
    by_confidence.sort_by(|a, b| {
        b.confidence
            .get()
            .total_cmp(&a.confidence.get())
            .then_with(|| canonical(a, b, rules.skyline))
    });
    // The rows the rule alone would place, to tell which kept notes their keep saves here.
    let mut plain: BTreeMap<u32, ()> = BTreeMap::new();
    if !keeps.is_empty() {
        for note in &by_confidence {
            if near(&plain, note.row, min).next().is_none() {
                plain.insert(note.row, ());
            }
        }
    }
    let (mut order, rest): (Vec<Note>, Vec<Note>) = by_confidence
        .into_iter()
        .partition(|n| kept_by(n, &keeps).is_some());
    order.sort_by(|a, b| {
        strength(
            kept_by(a, &keeps).expect("kept"),
            kept_by(b, &keeps).expect("kept"),
        )
    });
    order.extend(rest);
    let mut placed: BTreeMap<u32, Note> = BTreeMap::new();
    for note in order {
        // The confident neighbour it is too close to, if any: it stays, and this note goes.
        match near(&placed, note.row, min)
            .map(|n| n.confidence.get())
            .reduce(f32::max)
        {
            None => {
                if kept_by(&note, &keeps).is_some() && !plain.contains_key(&note.row) {
                    saved.insert((note.row, note.pitch));
                }
                placed.insert(note.row, note);
            }
            Some(winner) => {
                // A kept note too close to a stronger kept one, which is all that is placed before it.
                if let Some(d) = kept_by(&note, &keeps) {
                    let by = near(&placed, note.row, min)
                        .filter_map(|n| kept_by(n, &keeps))
                        .min_by(|a, b| strength(a, b))
                        .expect("only kept notes are placed before a kept one");
                    unapplied.push((d.id.clone(), lost_to(d, by)));
                }
                // As sure as the neighbour was surer: a tie is a coin toss, settled by order.
                let sure = Confidence::new((winner - note.confidence.get()).clamp(0.0, 1.0))
                    .expect("clamped");
                log.drop(&note, DropReason::Overlap, sure);
            }
        }
    }
    let mut line: Vec<Note> = placed.into_values().collect();
    // A note a keep saved records it, in place of any id this rule would give it; an earlier rule's id stays.
    for n in &mut line {
        if saved.contains(&(n.row, n.pitch))
            && let Some(d) = kept_by(n, &keeps)
        {
            n.decision.get_or_insert(d.id.clone());
        }
    }
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

    // Trims and fresh starts in force, on the line as built. A note an override changes records its id in place of
    // one this rule gave it; an earlier rule's id stays.
    let own =
        |id: &Option<DecisionId>, log: &Log| id.as_ref().is_none_or(|id| log.ids.contains(id));
    let mut trimmed = Vec::new();
    for d in &mine {
        match d.proposal {
            Proposal::TrimNote {
                row, pitch, length, ..
            } => {
                if let Some(w) = won.get(&d.slot()) {
                    unapplied.push((d.id.clone(), lost_to(d, w)));
                    continue;
                }
                won.insert(d.slot(), d);
                let Some(n) = line.iter_mut().find(|n| n.row == row && n.pitch == pitch) else {
                    unapplied.push((d.id.clone(), Unapplied::NoSuchNote));
                    continue;
                };
                // A trim only shortens: the rule has already ended the note where the next starts, if not before.
                if length > n.length {
                    unapplied.push((d.id.clone(), Unapplied::Lengthens));
                    continue;
                }
                // Shorter than the rule allows is a person's call, not a model's.
                if length < rules.min_rows && d.rank() < Rank::Person {
                    unapplied.push((d.id.clone(), Unapplied::OutOfRange));
                    continue;
                }
                trimmed.push((row, pitch));
                // The override takes the place of the rule's trim, so the note records it when it changes the
                // length or when the rule's trim had set it.
                let rules_trim = n.decision.is_some() && own(&n.decision, &log);
                if (n.length != length || rules_trim) && own(&n.decision, &log) {
                    n.decision = Some(d.id.clone());
                }
                n.length = length;
            }
            Proposal::Retrigger { row, pitch, .. } => {
                if let Some(w) = won.get(&d.slot()) {
                    unapplied.push((d.id.clone(), lost_to(d, w)));
                    continue;
                }
                won.insert(d.slot(), d);
                let Some(n) = line.iter_mut().find(|n| n.row == row && n.pitch == pitch) else {
                    unapplied.push((d.id.clone(), Unapplied::NoSuchNote));
                    continue;
                };
                if n.attack != Attack::Fresh {
                    n.attack = Attack::Fresh;
                    if own(&n.decision, &log) {
                        n.decision = Some(d.id.clone());
                    }
                }
            }
            _ => {}
        }
    }
    // A trim in force replaces the rule's own trim of that note. A note whose override didn't apply keeps the rule's
    // trim, and its id.
    log.decisions.retain(|r| match r.proposal {
        Proposal::TrimNote { row, pitch, .. } => !trimmed.contains(&(row, pitch)),
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

    unapplied.sort_by(|a, b| a.0.cmp(&b.0));
    Reduced {
        notes: line,
        decisions: log.decisions,
        unapplied,
    }
}

/// The notes in `placed` (by row) that a note on `row` would start less than `min` rows after or before: the one
/// before it and the one after it, when they are that close.
fn near<V>(placed: &BTreeMap<u32, V>, row: u32, min: u32) -> impl Iterator<Item = &V> {
    let before = placed
        .range(..row)
        .next_back()
        .filter(|&(&r, _)| row - r < min);
    let after = placed
        .range(row.saturating_add(1)..)
        .next()
        .filter(|&(&r, _)| r - row < min);
    before.into_iter().chain(after).map(|(_, v)| v)
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
