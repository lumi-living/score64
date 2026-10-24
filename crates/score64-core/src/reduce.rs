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
use crate::decide::{Claims, lost_to, strength};
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
/// the rules' own, and any of another kind, are left alone. "Stronger" is `resolve`'s order throughout: the higher
/// rank, then of one rank the lower id. Drops and keeps apply one at a time, the strongest first, each to the line
/// the stronger ones made; trims and fresh starts then apply to the line as built. A weaker drop or keep never makes a
/// stronger decision unapplied (#217).
///
/// - A `drop_note` takes its note (same row, pitch and length) out before the rule starts, as if it had never been
///   there, so the rule's other choices follow from the line without it. It takes the note from its source span when
///   it names one; of identical notes otherwise, the one the rule would drop. It gives way (`overruled` or
///   `outranked`) to a stronger trim or fresh start of the same note, even one that then can't apply: those are
///   separate slots, so `resolve` keeps all three, and a stronger decision about the note means it should stay. It
///   gives way, too, to a stronger trim or fresh start of another note that taking its note out would make
///   unapplied: another note stays and pushes that one off the line (even a trim that then can't apply), or ends a
///   trimmed note before the length of a trim that fits.
/// - A `trim_note` sets its note's length once the line is built. It only shortens: one longer than the rule left
///   the note is unapplied (`lengthens`). Shorter than `min_rows` is a person's call: an unreviewed model's is
///   unapplied (`out_of_range`).
/// - A `retrigger` gives its note a fresh start.
/// - A `keep_note` keeps a note on its row and pitch that the rule would drop (ADR-014 §8). It applies with the
///   stronger drops and keeps in force, and the rule runs with the kept notes in place, so the notes around them give
///   way by the rule's own logic:
///   - **drops:** a keep is met by any note the drops leave on its row and pitch. So to keep a shorter repeat, keep
///     the row and pitch and drop the longer ones, whatever their ids; to bring back a longer repeat a drop took, drop
///     the shorter ones too. A drop names a length, so of twins of one length it takes one only. Only when the drops
///     would take every note there do they compete with the keep: when stronger drops have taken them all, the keep
///     loses to the strongest; a weaker drop that would leave the keep no note there (the last, or the only one long
///     enough for a model's keep) loses to it (`overruled` or `outranked`).
///   - **ghost, too short:** of a kept row and pitch where no note passes step 1, the first (the longest) stays.
///     Shorter than `min_rows` is a person's call: an unreviewed model's keep of it is unapplied (`out_of_range`).
///   - **skyline:** the kept pitch is the one its row keeps, in its longest note as for any repeats, and the row's
///     other notes are dropped (`repeat` or `skyline`). A skyline drop a keep forces is certain, unless the line
///     without the keeps drops that note as a skyline too: then it is as sure as the skyline always is.
///   - **overlap:** kept notes are placed first, so a note closer than `min_rows` to one is dropped (`overlap`) even
///     when it is the surer. When a kept note is among the neighbours it is too close to, the drop follows from the
///     keep, so it is certain, unless the line without the keeps drops the note as an overlap too: then it has that
///     drop's confidence. A drop caused only by notes that aren't kept has the usual gap. Then notes running into a
///     kept note are trimmed, and a kept note running into the next is too, as the rule trims any note.
///
///   A keep that can't keep its note is unapplied and changes nothing: an unreviewed model's keep of a note that is
///   too short (`out_of_range`), or one whose note a stronger keep's pushes out, on its row or closer than `min_rows`
///   (`overruled` or `outranked`). So is one that would take out the note of a stronger trim or fresh start, even one
///   that then can't apply (as a drop gives way to it), or end a trimmed note before the length of a stronger trim
///   that fits (ADR-008: a model's keep never takes out a note a person trimmed or restarted, nor cuts a trimmed note
///   short). A fresh start names no length, so a kept note may still end a restarted note sooner. A keep that the
///   line with the stronger drops and keeps already meets changes no note; when it is the weakest of them and no
///   other keep is in force, it changes no decision either. Its note still stays against weaker keeps and drops, and,
///   like every kept note, is placed first. A kept note records its keep's id when the keep saved it, from a drop
///   (it kept the drop from taking the last note there) or from the rule (the line as it ends up, with only the
///   stronger keeps, doesn't have it), unless it already records one; a trim or fresh start in force on it then
///   leaves that id.
/// - Of several decisions for one slot (as when `resolve` hasn't run), the strongest claims it, whether or not it then
///   applies, and the others are `overruled` or `outranked`: the result is the one `resolve` and then this rule give.
///
/// A note an override names that isn't there (a different note is in its place, or the rule dropped it) is reported
/// as `no_such_note`. The rule records no decision of its own for what an override did. A note an override changed
/// records the override's id in place of this rule's; an id from an earlier rule stays. The order of `overrides`
/// doesn't matter.
///
/// The line is kept run by run (notes each less than `min_rows` rows after the one before). A keep that changes the
/// line places its run again; a drop places again only the notes its change reaches, note by note. So the cost grows
/// most with keeps on notes so dense that one run spans the subtune.
pub fn monophonise_with(
    notes: impl IntoIterator<Item = Note>,
    at: &Placement,
    rules: &MonoRules,
    overrides: &[Decision],
) -> Reduced {
    monophonise_in(notes, at, rules, overrides, false)
}

/// [`monophonise_with`] placing the whole line again, by the rule's own steps, for every drop and keep: the reference
/// the windowed runs must agree with.
#[cfg(test)]
fn monophonise_whole(
    notes: impl IntoIterator<Item = Note>,
    at: &Placement,
    rules: &MonoRules,
    overrides: &[Decision],
) -> Reduced {
    monophonise_in(notes, at, rules, overrides, true)
}

fn monophonise_in(
    notes: impl IntoIterator<Item = Note>,
    at: &Placement,
    rules: &MonoRules,
    overrides: &[Decision],
    whole: bool,
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
    let mut won = Claims::default();
    let mut notes: Vec<Note> = notes.into_iter().collect();
    notes.sort_by(|a, b| canonical(a, b, rules.skyline));

    // The trims and fresh starts that will hold their slots (the first of each slot), strongest first. A weaker drop
    // gives way to one of its note, and a weaker keep may not make one unapplied.
    let mut held: Vec<&Decision> = Vec::new();
    for (i, d) in mine.iter().enumerate() {
        let first = i == 0 || mine[i - 1].slot() != d.slot();
        if first
            && matches!(
                d.proposal,
                Proposal::TrimNote { .. } | Proposal::Retrigger { .. }
            )
        {
            held.push(d);
        }
    }
    held.sort_by(|a, b| strength(a, b));

    // 0. Drops and keeps in force, one at a time, strongest first, each on the line the stronger ones made: a drop
    // takes its note out before the rule runs, a keep keeps its note in place. Neither may make a stronger trim or
    // fresh start unapplied.
    let mut order: Vec<&Decision> = mine
        .iter()
        .copied()
        .filter(|d| {
            matches!(
                d.proposal,
                Proposal::DropNote { .. } | Proposal::KeepNote { .. }
            )
        })
        .collect();
    order.sort_by(|a, b| strength(a, b));
    let applied = apply_in_order(&notes, &order, &held, &mut won, rules, at, whole);
    unapplied.extend(applied.refused);
    let dropping = applied.dropping;
    let saved = applied.saved;
    let mut kept = applied.kept;

    // The line with every keep that applied in force, their notes placed first. The line without any (with the drops
    // that apply) says how sure a drop they force is.
    let notes = without(&notes, &dropping);
    let base = (!kept.is_empty()).then(|| {
        let mut scratch = Log::new(at, MONOPHONISE, "mono");
        place(
            notes.clone(),
            &mut BTreeMap::new(),
            rules,
            &mut scratch,
            &mut Vec::new(),
            None,
        );
        Baseline::of(&scratch.decisions)
    });
    let mut line = place(
        notes,
        &mut kept,
        rules,
        &mut log,
        &mut unapplied,
        base.as_ref(),
    );
    // A note a keep saved records it, in place of any id this rule would give it; an earlier rule's id stays.
    for n in &mut line {
        if saved.contains(&(n.row, n.pitch))
            && let Some(d) = kept.get(&(n.row, n.pitch))
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
                if let Some(why) = won.claim(d) {
                    unapplied.push((d.id.clone(), why));
                    continue;
                }
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
                if let Some(why) = won.claim(d) {
                    unapplied.push((d.id.clone(), why));
                    continue;
                }
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

/// What applying drops and keeps one by one gives: the drops that took their note, by its index; the keeps that
/// applied, by row and pitch, which the rule places first; the rows and pitches whose note a keep saved, from a drop
/// (whose last note there it kept) or from the rule (the line as it ends up, with only the stronger keeps, wouldn't
/// have the note); and
/// the drops and keeps that couldn't apply, with why.
struct Applied<'d> {
    dropping: BTreeMap<usize, &'d Decision>,
    kept: BTreeMap<(u32, Pitch), &'d Decision>,
    saved: BTreeSet<(u32, Pitch)>,
    refused: Vec<(DecisionId, Unapplied)>,
}

/// Apply `order`'s drops and keeps to `all` (in canonical order), strongest first, each on the line the stronger ones
/// made, claiming their slots in `won`.
///
/// - A drop gives way to a stronger trim or fresh start of its note (`held`, strongest first), and to a stronger keep
///   of its row and pitch that would then keep no note there (the last, or the only one long enough for a model).
///   Otherwise it takes its note out, unless that would make a stronger trim or fresh start unapplied: take its note
///   off the line (even one that then can't apply), or end a trimmed note before the length of a trim that fits there.
/// - A keep whose row and pitch's notes stronger drops have all taken loses to the strongest of them. A keep the line
///   already meets changes no note; one that can't keep its note, or that would make a stronger trim or fresh start
///   unapplied as a drop may not, is refused. A stronger keep's note needs no other check: kept notes are placed
///   first, the strongest keep's first, and a drop only takes a note out.
///
/// A keep's trial places its run again ([`Runs`]); a drop's follows its change note by note, and checks the trims and
/// fresh starts it could touch, and a keep only on its own row and pitch.
fn apply_in_order<'d>(
    all: &[Note],
    order: &[&'d Decision],
    held: &[&'d Decision],
    won: &mut Claims<'d>,
    rules: &MonoRules,
    at: &Placement,
    whole: bool,
) -> Applied<'d> {
    let mut out = Applied {
        dropping: BTreeMap::new(),
        kept: BTreeMap::new(),
        saved: BTreeSet::new(),
        refused: Vec::new(),
    };
    // The note each drop names.
    let mut note_of: Vec<Option<usize>> = Vec::with_capacity(order.len());
    for &d in order {
        let Proposal::DropNote {
            row,
            pitch,
            length,
            source,
            ..
        } = d.proposal
        else {
            note_of.push(None);
            continue;
        };
        // The one from its source span when it names one; otherwise the last of its row, pitch and length in
        // canonical order, which is the one the rule would drop of identical twins. Drops of one row, pitch and length
        // share a slot, so no two take one note.
        let found = on_row(all, row)
            .filter(|&i| {
                let n = &all[i];
                n.pitch == pitch && n.length == length && (source.is_none() || n.source == source)
            })
            .last();
        note_of.push(found);
    }
    // The trims and fresh starts by their note, strongest first.
    let mut held_at: BTreeMap<(u32, Pitch), Vec<&'d Decision>> = BTreeMap::new();
    for &h in held {
        if let Some(note) = about(h) {
            held_at.entry(note).or_default().push(h);
        }
    }
    let mut runs = Runs::new(all, rules, at, whole);
    for (&d, &found) in order.iter().zip(&note_of) {
        if let Some(why) = won.claim(d) {
            out.refused.push((d.id.clone(), why));
            continue;
        }
        match d.proposal {
            Proposal::DropNote { row, pitch, .. } => {
                if let Some(h) = held_over(held, d, row, pitch) {
                    out.refused.push((d.id.clone(), lost_to(d, h)));
                    continue;
                }
                let Some(i) = found else {
                    out.refused.push((d.id.clone(), Unapplied::NoSuchNote));
                    continue;
                };
                let k = runs.run_of(row).expect("a note's row is in its run");
                let next = runs.without(k, i, &out.dropping, &out.kept);
                // A stronger keep of its row and pitch, which must still keep a note there.
                let lost = out
                    .kept
                    .get(&(row, pitch))
                    .copied()
                    .filter(|_| !runs.view(k, Some(&next)).has((row, pitch)));
                let undone = runs.undoes(&held_at, d, k, &next, &out.dropping, &out.kept);
                if let Some(by) = lost.into_iter().chain(undone).min_by(|a, b| strength(a, b)) {
                    out.refused.push((d.id.clone(), lost_to(d, by)));
                    if lost.is_some() {
                        // The keep saved its note from the drop.
                        out.saved.insert((row, pitch));
                    }
                    continue;
                }
                runs.commit(k, next);
                out.dropping.insert(i, d);
            }
            Proposal::KeepNote { row, pitch, .. } => {
                let here = (row, pitch);
                // Its row and pitch's notes, longest first, and those still there.
                let there: Vec<usize> = on_row(all, row)
                    .filter(|&i| all[i].pitch == pitch)
                    .collect();
                if there.is_empty() {
                    out.refused.push((d.id.clone(), Unapplied::NoSuchNote));
                    continue;
                }
                if there.iter().all(|i| out.dropping.contains_key(i)) {
                    let by = there
                        .iter()
                        .map(|i| out.dropping[i])
                        .min_by(|a, b| strength(a, b))
                        .expect("a note there");
                    out.refused.push((d.id.clone(), lost_to(d, by)));
                    continue;
                }
                let k = runs.run_of(row).expect("a note's row is in its run");
                if !runs.has(k, here, &out.dropping, &out.kept) {
                    let mut with = out.kept.clone();
                    with.insert(here, d);
                    let (line, why) = runs.build(k, &out.dropping, None, &with);
                    let next = Patch::Line(line);
                    if !runs.view(k, Some(&next)).has(here) {
                        // Only it can fail: the stronger keeps' notes are placed before its own.
                        let why = why
                            .into_iter()
                            .next()
                            .expect("a keep that can't keep its note says why");
                        out.refused.push(why);
                        continue;
                    }
                    if let Some(h) = runs.undoes(&held_at, d, k, &next, &out.dropping, &out.kept) {
                        out.refused.push((d.id.clone(), lost_to(d, h)));
                        continue;
                    }
                    runs.commit(k, next);
                }
                out.kept.insert(here, d);
            }
            _ => unreachable!("only drops and keeps"),
        }
    }
    // A keep saved its note from the rule when the line as it ends up, with only the stronger keeps, wouldn't have the
    // note. A keep the line met when it applied may be what keeps the note now: weaker drops may since have come.
    for (&here, &d) in &out.kept {
        let k = runs.run_of(here.0).expect("a note's row is in its run");
        let stronger: Kept = out
            .kept
            .iter()
            .filter(|(_, o)| strength(o, d).is_lt())
            .map(|(&at, &o)| (at, o))
            .collect();
        let line = runs.build(k, &out.dropping, None, &stronger).0;
        if !(View {
            line: &line,
            delta: None,
        })
        .has(all, here)
        {
            out.saved.insert(here);
        }
    }
    out
}

/// The strongest trim or fresh start of `held` (strongest first) about the note on `row` and `pitch`, if it is
/// stronger than `d`.
fn held_over<'d>(
    held: &[&'d Decision],
    d: &Decision,
    row: u32,
    pitch: Pitch,
) -> Option<&'d Decision> {
    held.iter()
        .take_while(|h| strength(h, d).is_lt())
        .find(|h| about(h) == Some((row, pitch)))
        .copied()
}

/// The note a trim or fresh start is about.
fn about(d: &Decision) -> Option<(u32, Pitch)> {
    match d.proposal {
        Proposal::TrimNote { row, pitch, .. } | Proposal::Retrigger { row, pitch, .. } => {
            Some((row, pitch))
        }
        _ => None,
    }
}

type Taking<'d> = BTreeMap<usize, &'d Decision>;
type Kept<'d> = BTreeMap<(u32, Pitch), &'d Decision>;

/// One run's line as steps 1 to 3 leave it: the note each row keeps (by its index, after the ghosts, the too-short
/// notes and the skyline), and the rows whose note is placed.
#[derive(Clone, Debug, Default)]
struct RunLine {
    heads: BTreeMap<u32, usize>,
    placed: BTreeSet<u32>,
}

/// A change to a run's line: the rows whose note changed (`None` when none is left), and the rows whose placing
/// flipped.
#[derive(Debug, Default)]
struct Delta {
    heads: BTreeMap<u32, Option<usize>>,
    flips: BTreeSet<u32>,
}

/// A run's line as a trial would leave it: placed again, or changed by a delta.
enum Patch {
    Line(RunLine),
    Delta(Delta),
}

/// A run's line, with a delta on it if any.
#[derive(Clone, Copy)]
struct View<'r> {
    line: &'r RunLine,
    delta: Option<&'r Delta>,
}

impl View<'_> {
    fn head(&self, row: u32) -> Option<usize> {
        match self.delta.and_then(|d| d.heads.get(&row)) {
            Some(&h) => h,
            None => self.line.heads.get(&row).copied(),
        }
    }

    fn placed(&self, row: u32) -> bool {
        self.line.placed.contains(&row) != self.delta.is_some_and(|d| d.flips.contains(&row))
    }

    fn flipped(&self, row: &u32) -> bool {
        self.delta.is_some_and(|d| d.flips.contains(row))
    }

    /// Whether a note on `here` is placed.
    fn has(&self, all: &[Note], here: (u32, Pitch)) -> bool {
        self.placed(here.0) && self.head(here.0).is_some_and(|h| all[h].pitch == here.1)
    }

    /// The first placed row after `row`.
    fn next_placed(&self, row: u32) -> Option<u32> {
        let stays = self
            .line
            .placed
            .range(row.saturating_add(1)..)
            .find(|r| !self.flipped(r));
        let comes = self.delta.and_then(|d| {
            d.flips
                .range(row.saturating_add(1)..)
                .find(|r| !self.line.placed.contains(r))
        });
        stays.into_iter().chain(comes).min().copied()
    }

    /// The first placed row.
    fn first_placed(&self) -> Option<u32> {
        let stays = self.line.placed.iter().find(|r| !self.flipped(r));
        let comes = self
            .delta
            .and_then(|d| d.flips.iter().find(|r| !self.line.placed.contains(r)));
        stays.into_iter().chain(comes).min().copied()
    }

    /// The last placed row before `row`, or of all with `None`.
    fn last_placed(&self, before: Option<u32>) -> Option<u32> {
        let end = before.unwrap_or(u32::MAX);
        let stays = self
            .line
            .placed
            .range(..end)
            .rev()
            .find(|r| !self.flipped(r));
        let comes = self.delta.and_then(|d| {
            d.flips
                .range(..end)
                .rev()
                .find(|r| !self.line.placed.contains(r))
        });
        stays.into_iter().chain(comes).max().copied()
    }

    /// The rows with a note within `reach` rows of `row`, other than `row`.
    fn near(&self, row: u32, reach: u32) -> Vec<u32> {
        let range = row.saturating_sub(reach)..=row.saturating_add(reach);
        let mut rows: BTreeSet<u32> = self
            .line
            .heads
            .range(range.clone())
            .map(|(&r, _)| r)
            .collect();
        if let Some(d) = self.delta {
            for (&r, h) in d.heads.range(range) {
                match h {
                    Some(_) => rows.insert(r),
                    None => rows.remove(&r),
                };
            }
        }
        rows.remove(&row);
        rows.into_iter().collect()
    }
}

/// Where a row's note comes in step 3's order: kept notes first, the strongest keep's first; then the surer; then
/// in canonical order.
#[derive(Clone, Copy, Debug)]
struct Turn<'d> {
    keep: Option<&'d Decision>,
    confidence: f32,
    index: usize,
}

impl<'d> Turn<'d> {
    fn of(all: &[Note], index: usize, keeps: &Kept<'d>) -> Self {
        let n = &all[index];
        Turn {
            keep: keeps.get(&(n.row, n.pitch)).copied(),
            confidence: n.confidence.get(),
            index,
        }
    }
}

impl Ord for Turn<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self.keep, other.keep) {
            (Some(a), Some(b)) => strength(a, b),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => other.confidence.total_cmp(&self.confidence),
        }
        .then(self.index.cmp(&other.index))
    }
}

impl PartialOrd for Turn<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Turn<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Turn<'_> {}

/// Steps 1 and 2 on one row: the note it keeps, of `present` (its notes' indices, in canonical order), with `keeps`
/// in force; why a keep there can't apply goes to `why`.
fn head_of_row(
    all: &[Note],
    present: &[usize],
    keeps: &Kept<'_>,
    row: u32,
    rules: &MonoRules,
    why: &mut Vec<(DecisionId, Unapplied)>,
) -> Option<usize> {
    let passes = |n: &Note| n.confidence >= rules.ghost_below && n.length >= rules.min_rows;
    let mut valid: Vec<(Pitch, &Decision)> = Vec::new();
    let mut rescue: BTreeSet<Pitch> = BTreeSet::new();
    for (&(_, pitch), &d) in keeps.range((row, Pitch::MIN)..=(row, Pitch::MAX)) {
        let mut there = present
            .iter()
            .map(|&i| &all[i])
            .filter(|n| n.pitch == pitch);
        let Some(first) = there.clone().next() else {
            continue;
        };
        if !there.any(passes) {
            if first.length < rules.min_rows && d.rank() < Rank::Person {
                why.push((d.id.clone(), Unapplied::OutOfRange));
                continue;
            }
            rescue.insert(pitch);
        }
        valid.push((pitch, d));
    }
    let passed: Vec<usize> = present
        .iter()
        .copied()
        .filter(|&i| passes(&all[i]) || rescue.contains(&all[i].pitch))
        .collect();
    let keep_of = |p: Pitch| valid.iter().find(|(q, _)| *q == p).map(|(_, d)| *d);
    let pick = passed
        .iter()
        .filter_map(|&i| keep_of(all[i].pitch).map(|d| (i, d)))
        .min_by(|a, b| strength(a.1, b.1));
    match pick {
        Some((i, winner)) => {
            for &(p, d) in &valid {
                if p != all[i].pitch && passed.iter().any(|&j| all[j].pitch == p) {
                    why.push((d.id.clone(), lost_to(d, winner)));
                }
            }
            Some(i)
        }
        None => passed.first().copied(),
    }
}

/// The line as the decisions applied so far leave it, run by run. Notes closer than `min_rows` only ever meet within
/// a run (notes each less than `min_rows` rows after the one before), so each run is placed on its own. A keep's
/// trial places its run again; a drop's follows on from the row its note leaves, note by note, only as far as that
/// changes which notes are placed.
struct Runs<'a, 'p> {
    all: &'a [Note],
    /// Each run's first and last rows, and the indices of its notes.
    bounds: Vec<(u32, u32, std::ops::Range<usize>)>,
    /// Each run's line, once placed.
    lines: Vec<Option<RunLine>>,
    rules: &'a MonoRules,
    at: &'a Placement<'p>,
    /// One run of every note, placed again in full by the rule's own steps for every trial: the reference the
    /// windowed runs must agree with.
    whole: bool,
}

impl<'a, 'p> Runs<'a, 'p> {
    fn new(all: &'a [Note], rules: &'a MonoRules, at: &'a Placement<'p>, whole: bool) -> Self {
        let min = rules.min_rows.get();
        let mut bounds: Vec<(u32, u32, std::ops::Range<usize>)> = Vec::new();
        for (i, n) in all.iter().enumerate() {
            match bounds.last_mut() {
                Some((_, last, range)) if whole || n.row - *last < min => {
                    *last = n.row;
                    range.end = i + 1;
                }
                _ => bounds.push((n.row, n.row, i..i + 1)),
            }
        }
        let lines = vec![None; bounds.len()];
        Runs {
            all,
            bounds,
            lines,
            rules,
            at,
            whole,
        }
    }

    /// The run whose rows span `row`, if any.
    fn run_of(&self, row: u32) -> Option<usize> {
        let k = self.bounds.partition_point(|b| b.0 <= row).checked_sub(1)?;
        (row <= self.bounds[k].1).then_some(k)
    }

    /// Run `k` placed with the notes `taking` (and the note at `also`, if any) leaves and its keeps of `keeps`, and why
    /// any of those keeps can't apply. A keep with no note left, which a trial may leave, keeps nothing.
    fn build(
        &self,
        k: usize,
        taking: &Taking,
        also: Option<usize>,
        keeps: &Kept<'_>,
    ) -> (RunLine, Vec<(DecisionId, Unapplied)>) {
        let (start, end, range) = &self.bounds[k];
        let present: Vec<usize> = range
            .clone()
            .filter(|&i| Some(i) != also && !taking.contains_key(&i))
            .collect();
        let keeps: Kept = keeps
            .range((*start, Pitch::MIN)..=(*end, Pitch::MAX))
            .filter(|&(&(row, pitch), _)| {
                present
                    .iter()
                    .any(|&i| (self.all[i].row, self.all[i].pitch) == (row, pitch))
            })
            .map(|(&at, &d)| (at, d))
            .collect();
        let mut why = Vec::new();
        let mut line = RunLine::default();
        if self.whole {
            // The rule's own steps.
            let notes = present.iter().map(|&i| self.all[i].clone()).collect();
            let mut keeps = keeps;
            let mut scratch = Log::new(self.at, MONOPHONISE, "mono");
            for n in place(notes, &mut keeps, self.rules, &mut scratch, &mut why, None) {
                let i = on_row(self.all, n.row)
                    .find(|&i| self.all[i] == n)
                    .expect("a placed note is one of them");
                line.heads.insert(n.row, i);
                line.placed.insert(n.row);
            }
            return (line, why);
        }
        // Steps 1 and 2, row by row.
        let mut from = 0;
        while from < present.len() {
            let row = self.all[present[from]].row;
            let to = from + present[from..].partition_point(|&i| self.all[i].row == row);
            if let Some(h) = head_of_row(
                self.all,
                &present[from..to],
                &keeps,
                row,
                self.rules,
                &mut why,
            ) {
                line.heads.insert(row, h);
            }
            from = to;
        }
        // Step 3: in order, a note is placed unless it is too close to one placed before it.
        let min = self.rules.min_rows.get();
        let mut order: Vec<(Turn, u32)> = line
            .heads
            .iter()
            .map(|(&row, &h)| (Turn::of(self.all, h, &keeps), row))
            .collect();
        order.sort();
        for (rank, row) in order {
            let before = line
                .placed
                .range(..row)
                .next_back()
                .filter(|&&r| row - r < min);
            let after = line
                .placed
                .range(row.saturating_add(1)..)
                .next()
                .filter(|&&r| r - row < min);
            if before.is_none() && after.is_none() {
                line.placed.insert(row);
            } else if let Some(d) = rank.keep {
                let by = before
                    .into_iter()
                    .chain(after)
                    .filter_map(|r| {
                        let h = self.all[line.heads[r]].pitch;
                        keeps.get(&(*r, h)).copied()
                    })
                    .min_by(|a, b| strength(a, b))
                    .expect("only kept notes are placed before a kept one");
                why.push((d.id.clone(), lost_to(d, by)));
            }
        }
        (line, why)
    }

    fn fresh(&mut self, k: usize, taking: &Taking, keeps: &Kept<'_>) {
        if self.lines[k].is_none() {
            self.lines[k] = Some(self.build(k, taking, None, keeps).0);
        }
    }

    /// Run `k` as it is, or as `patch` would leave it.
    fn view<'s>(&'s self, k: usize, patch: Option<&'s Patch>) -> RunView<'s> {
        let view = match patch {
            Some(Patch::Line(line)) => View { line, delta: None },
            Some(Patch::Delta(delta)) => View {
                line: self.lines[k].as_ref().expect("fresh"),
                delta: Some(delta),
            },
            None => View {
                line: self.lines[k].as_ref().expect("fresh"),
                delta: None,
            },
        };
        RunView {
            all: self.all,
            view,
        }
    }

    fn commit(&mut self, k: usize, patch: Patch) {
        match patch {
            Patch::Line(line) => self.lines[k] = Some(line),
            Patch::Delta(delta) => {
                let line = self.lines[k].as_mut().expect("fresh");
                for (row, h) in delta.heads {
                    match h {
                        Some(h) => line.heads.insert(row, h),
                        None => line.heads.remove(&row),
                    };
                }
                for row in delta.flips {
                    if !line.placed.remove(&row) {
                        line.placed.insert(row);
                    }
                }
            }
        }
    }

    /// Whether the line has a note on `here`, in run `k`.
    fn has(&mut self, k: usize, here: (u32, Pitch), taking: &Taking, keeps: &Kept<'_>) -> bool {
        self.fresh(k, taking, keeps);
        self.view(k, None).has(here)
    }

    /// Run `k` without the note at `i`. Taking a note out changes at most the note its row keeps; from there, a note
    /// whose placing changes can change those it is too close to that come after it in step 3's order, and only
    /// those are placed again, in that order.
    fn without(&mut self, k: usize, i: usize, taking: &Taking, keeps: &Kept<'_>) -> Patch {
        if self.whole {
            return Patch::Line(self.build(k, taking, Some(i), keeps).0);
        }
        self.fresh(k, taking, keeps);
        let all = self.all;
        let line = self.lines[k].as_ref().expect("fresh");
        let row = all[i].row;
        let present: Vec<usize> = on_row(all, row)
            .filter(|&j| j != i && !taking.contains_key(&j))
            .collect();
        let new = head_of_row(all, &present, keeps, row, self.rules, &mut Vec::new());
        let mut delta = Delta::default();
        if new == line.heads.get(&row).copied() {
            return Patch::Delta(delta);
        }
        delta.heads.insert(row, new);
        let reach = self.rules.min_rows.get() - 1;
        let rank = |delta: &Delta, q: u32| {
            View {
                line,
                delta: Some(delta),
            }
            .head(q)
            .map(|h| Turn::of(all, h, keeps))
        };
        let mut queue: BTreeSet<(Turn, u32)> = BTreeSet::new();
        if new.is_none() && line.placed.contains(&row) {
            delta.flips.insert(row);
        }
        let mut first = vec![row];
        first.extend(
            View {
                line,
                delta: Some(&delta),
            }
            .near(row, reach),
        );
        for q in first {
            if let Some(r) = rank(&delta, q) {
                queue.insert((r, q));
            }
        }
        while let Some((r, q)) = queue.pop_first() {
            let view = View {
                line,
                delta: Some(&delta),
            };
            let near = view.near(q, reach);
            let free = !near
                .iter()
                .any(|&p| view.placed(p) && rank(&delta, p).is_some_and(|rp| rp < r));
            if free != view.placed(q) {
                if !delta.flips.remove(&q) {
                    delta.flips.insert(q);
                }
                for p in near {
                    if let Some(rp) = rank(&delta, p).filter(|rp| *rp > r) {
                        queue.insert((rp, p));
                    }
                }
            }
        }
        Patch::Delta(delta)
    }

    /// The last note on the line before run `k`, by row and pitch.
    fn last_before(&mut self, k: usize, taking: &Taking, keeps: &Kept<'_>) -> Option<(u32, Pitch)> {
        for j in (0..k).rev() {
            self.fresh(j, taking, keeps);
            let view = self.view(j, None).view;
            if let Some(row) = view.last_placed(None) {
                let h = view.head(row).expect("a placed row has its note");
                return Some((row, self.all[h].pitch));
            }
        }
        None
    }

    /// How long the note on `row` and `pitch` sounds on the line, once the rule ends it where the next starts, with run
    /// `k` as `patch` would leave it, if given; `None` when it isn't there.
    fn sound(
        &mut self,
        row: u32,
        pitch: Pitch,
        patch: Option<(usize, &Patch)>,
        taking: &Taking,
        keeps: &Kept<'_>,
    ) -> Option<NonZeroU32> {
        let k = self.run_of(row)?;
        self.fresh(k, taking, keeps);
        let of = |j: usize| patch.filter(|(p, _)| *p == j).map(|(_, patch)| patch);
        let (length, next) = {
            let view = self.view(k, of(k)).view;
            let h = view.head(row)?;
            if !view.placed(row) || self.all[h].pitch != pitch {
                return None;
            }
            (self.all[h].length, view.next_placed(row))
        };
        let next = next.or_else(|| {
            (k + 1..self.bounds.len()).find_map(|j| {
                self.fresh(j, taking, keeps);
                self.view(j, of(j)).view.first_placed()
            })
        });
        let end = next.map_or(length.get(), |r| length.get().min(r - row));
        Some(NonZeroU32::new(end).expect("rows are distinct and in order"))
    }

    /// The strongest trim or fresh start of `held_at` stronger than `d` that run `k` as `patch` would leave it makes
    /// unapplied: one whose note it takes off the line (even one that then can't apply), or a trim that fits whose
    /// note it ends before the trim's length. Only notes the patch can touch are looked at: those on a row it changes
    /// and the placed notes before them, which a change ends; for a run placed again, all its notes and the note
    /// before it.
    fn undoes<'d>(
        &mut self,
        held_at: &BTreeMap<(u32, Pitch), Vec<&'d Decision>>,
        d: &Decision,
        k: usize,
        patch: &Patch,
        taking: &Taking,
        keeps: &Kept<'_>,
    ) -> Option<&'d Decision> {
        let mut rows: BTreeSet<u32> = BTreeSet::new();
        let mut before_run = false;
        match patch {
            Patch::Line(_) => {
                let (start, end, _) = self.bounds[k];
                rows.extend(
                    held_at
                        .range((start, Pitch::MIN)..=(end, Pitch::MAX))
                        .map(|(&(r, _), _)| r),
                );
                before_run = true;
            }
            Patch::Delta(delta) => {
                let changed: BTreeSet<u32> =
                    delta.heads.keys().chain(&delta.flips).copied().collect();
                for &f in &changed {
                    rows.insert(f);
                    for view in [self.view(k, None).view, self.view(k, Some(patch)).view] {
                        match view.last_placed(Some(f)) {
                            Some(p) => {
                                rows.insert(p);
                            }
                            None => before_run = true,
                        }
                    }
                }
            }
        }
        let mut notes: Vec<(u32, Pitch)> = Vec::new();
        for r in rows {
            notes.extend(
                held_at
                    .range((r, Pitch::MIN)..=(r, Pitch::MAX))
                    .map(|(&at, _)| at),
            );
        }
        if before_run && let Some(prev) = self.last_before(k, taking, keeps) {
            notes.push(prev);
        }
        let mut undone: Option<&'d Decision> = None;
        for (row, pitch) in notes {
            let Some(hs) = held_at.get(&(row, pitch)) else {
                continue;
            };
            let before = self.sound(row, pitch, None, taking, keeps);
            let after = self.sound(row, pitch, Some((k, patch)), taking, keeps);
            for &h in hs.iter().take_while(|h| strength(h, d).is_lt()) {
                let fits = |l: Option<NonZeroU32>| match h.proposal {
                    Proposal::TrimNote { length, .. } => l.is_some_and(|l| length <= l),
                    _ => l.is_some(),
                };
                if (before.is_some() && after.is_none()) || (fits(before) && !fits(after)) {
                    if undone.is_none_or(|u| strength(h, u).is_lt()) {
                        undone = Some(h);
                    }
                    break;
                }
            }
        }
        undone
    }
}

/// A run's view with the notes it indexes.
struct RunView<'s> {
    all: &'s [Note],
    view: View<'s>,
}

impl RunView<'_> {
    fn has(&self, here: (u32, Pitch)) -> bool {
        self.view.has(self.all, here)
    }
}

/// The indices of the notes on `row`, of `notes` in canonical order (by row first).
fn on_row(notes: &[Note], row: u32) -> impl Iterator<Item = usize> + Clone + '_ {
    let start = notes.partition_point(|n| n.row < row);
    (start..notes.len()).take_while(move |&i| notes[i].row == row)
}

/// The notes left when `dropping` takes its notes out, in canonical order.
fn without(notes: &[Note], dropping: &BTreeMap<usize, &Decision>) -> Vec<Note> {
    notes
        .iter()
        .enumerate()
        .filter(|(i, _)| !dropping.contains_key(i))
        .map(|(_, n)| n.clone())
        .collect()
}

/// What the line without the keeps drops by skyline (by row and pitch), and how sure each overlap drop is.
struct Baseline {
    skylines: BTreeSet<(u32, Pitch)>,
    overlaps: BTreeMap<(u32, Pitch), Confidence>,
}

impl Baseline {
    fn of(decisions: &[Decision]) -> Self {
        let mut base = Baseline {
            skylines: BTreeSet::new(),
            overlaps: BTreeMap::new(),
        };
        for d in decisions {
            if let Proposal::DropNote {
                row, pitch, reason, ..
            } = d.proposal
            {
                match reason {
                    DropReason::Skyline => {
                        base.skylines.insert((row, pitch));
                    }
                    DropReason::Overlap => {
                        base.overlaps.insert((row, pitch), d.confidence);
                    }
                    _ => {}
                }
            }
        }
        base
    }
}

/// The keep in force on `n`'s row and pitch, if any.
fn kept_by<'d>(n: &Note, keeps: &BTreeMap<(u32, Pitch), &'d Decision>) -> Option<&'d Decision> {
    keeps.get(&(n.row, n.pitch)).copied()
}

/// Steps 1 to 3 of [`monophonise_with`], on `notes` in canonical order with `keeps` in force: the notes placed on the
/// line, by row and not yet trimmed. A keep that can't apply leaves `keeps`, and says why in `unapplied`. With keeps,
/// `base` is the line without them, which says how sure a drop they force is.
fn place(
    notes: Vec<Note>,
    keeps: &mut BTreeMap<(u32, Pitch), &Decision>,
    rules: &MonoRules,
    log: &mut Log,
    unapplied: &mut Vec<(DecisionId, Unapplied)>,
    base: Option<&Baseline>,
) -> Vec<Note> {
    // 1. Ghosts and too-short notes. A kept row and pitch where no note passes keeps its first note, the longest.
    let passes = |n: &Note| n.confidence >= rules.ghost_below && n.length >= rules.min_rows;
    let mut rescue: BTreeSet<(u32, Pitch)> = BTreeSet::new();
    keeps.retain(|&(row, pitch), d| {
        let mut there = on_row(&notes, row)
            .map(|i| &notes[i])
            .filter(|n| n.pitch == pitch);
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
    let mut passed = Vec::with_capacity(notes.len());
    for note in notes {
        if !passes(&note) && rescue.remove(&(note.row, note.pitch)) {
            passed.push(note);
        } else if note.confidence < rules.ghost_below {
            let sure = Confidence::new(1.0 - note.confidence.get()).expect("in range");
            log.drop(&note, DropReason::Ghost, sure);
        } else if note.length < rules.min_rows {
            log.drop(&note, DropReason::TooShort, Confidence::CERTAIN);
        } else {
            passed.push(note);
        }
    }

    // 2. One note per row: the first in canonical order is the skyline's pick, and the longest of its pitch. A kept
    // pitch on the row is picked instead, the strongest keep's if there are several; the other keeps there lose to it.
    let skyline = Confidence::new(SKYLINE_CONFIDENCE).expect("in range");
    let mut heads: Vec<Note> = Vec::with_capacity(passed.len());
    let mut passed = passed.into_iter().peekable();
    while let Some(first) = passed.next() {
        let row = first.row;
        let mut on_row = vec![first];
        while let Some(n) = passed.next_if(|n| n.row == row) {
            on_row.push(n);
        }
        // `min_by` takes the first of equals: of a kept pitch's notes, the longest.
        let pick = on_row
            .iter()
            .enumerate()
            .filter_map(|(i, n)| kept_by(n, keeps).map(|d| (i, d)))
            .min_by(|a, b| strength(a.1, b.1));
        let pick = match pick {
            Some((i, winner)) => {
                let pitch = on_row[i].pitch;
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
                // With keeps in force, a drop the line without them doesn't make follows from a keep (which picked the
                // row: the others' rows are as there).
                let sure = match base {
                    Some(base) if !base.skylines.contains(&(note.row, note.pitch)) => {
                        Confidence::CERTAIN
                    }
                    _ => skyline,
                };
                (DropReason::Skyline, sure)
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
    let (mut order, rest): (Vec<Note>, Vec<Note>) = by_confidence
        .into_iter()
        .partition(|n| kept_by(n, keeps).is_some());
    order.sort_by(|a, b| {
        strength(
            kept_by(a, keeps).expect("kept"),
            kept_by(b, keeps).expect("kept"),
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
                placed.insert(note.row, note);
            }
            Some(winner) => {
                // The strongest keep among the neighbours it is too close to.
                let by = near(&placed, note.row, min)
                    .filter_map(|n| kept_by(n, keeps))
                    .min_by(|a, b| strength(a, b));
                // A kept note too close to a stronger kept one, which is all that is placed before it.
                if let Some(d) = kept_by(&note, keeps) {
                    let by = by.expect("only kept notes are placed before a kept one");
                    unapplied.push((d.id.clone(), lost_to(d, by)));
                }
                let sure = match base {
                    // Next to a kept note, the drop follows from the keep, unless the line without the keeps makes
                    // it too.
                    Some(base) if by.is_some() => base
                        .overlaps
                        .get(&(note.row, note.pitch))
                        .copied()
                        .unwrap_or(Confidence::CERTAIN),
                    // As sure as the neighbour was surer: a tie is a coin toss, settled by order.
                    _ => Confidence::new((winner - note.confidence.get()).clamp(0.0, 1.0))
                        .expect("clamped"),
                };
                log.drop(&note, DropReason::Overlap, sure);
            }
        }
    }
    placed.into_values().collect()
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
