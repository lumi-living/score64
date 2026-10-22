//! Drum folding (#43): transcribed drum hits onto a voice of their own, or interleaved with another line (a bass, say)
//! when no voice is free for drums.
//!
//! Hits arrive as notes whose pitch carries their General MIDI drum key ([`Pitch::from_midi`] of the GM number), as
//! a drum transcription or a MIDI drum track gives them. The sounds themselves (a noise snare, a triangle kick) are
//! instrument presets; this module refers to them by preset id through a project's [`DrumKit`].

use std::collections::BTreeMap;
use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};

use crate::provenance::{Confidence, DropReason, Proposal};
use crate::score::{Attack, Note, Pitch, PresetId};

use super::{Log, Placement, Reduced, Skyline, canonical};

/// The rule name and version [`fold_drums`] records in its decisions.
pub const FOLD_DRUMS: &str = "fold-drums@1";

/// The rule name and version [`interleave_drums`] records in its decisions.
pub const INTERLEAVE_DRUMS: &str = "interleave-drums@1";

/// How sure the rule is when a person's ear could reasonably go either way: which of a drum hit and a line note keeps
/// a shared row.
const CLASH_CONFIDENCE: f32 = 0.5;

/// What a drum hit is, by the General MIDI drum map. In priority order: when two hits share a row, the earlier kind
/// wins.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum DrumKind {
    /// Bass drum (GM 35, 36).
    Kick,
    /// Snare, rim shot and clap (GM 37–40).
    Snare,
    /// Hi-hats (GM 42, 44, 46) and cymbals (GM 49, 51, 52, 53, 55, 57, 59).
    Hat,
    /// Anything else: toms, percussion.
    Other,
}

impl DrumKind {
    /// The kind of a General MIDI drum note.
    pub const fn from_midi(note: u8) -> Self {
        match note {
            35 | 36 => DrumKind::Kick,
            37..=40 => DrumKind::Snare,
            42 | 44 | 46 | 49 | 51 | 52 | 53 | 55 | 57 | 59 => DrumKind::Hat,
            _ => DrumKind::Other,
        }
    }
}

/// One drum sound: the preset that plays it, the pitch it is played at, and how many rows it lasts at most. Several
/// kinds may share a preset at different pitches (one noise instrument for snare and hat, say).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct DrumSound {
    /// The instrument preset.
    pub instrument: PresetId,
    /// The pitch to play it at.
    pub pitch: Pitch,
    /// Its length, in rows, unless the next note comes sooner.
    pub rows: NonZeroU32,
}

/// A project's drum sounds, by kind. A kind without a sound is left out of the arrangement.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct DrumKit {
    /// Bass drum.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kick: Option<DrumSound>,
    /// Snare.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snare: Option<DrumSound>,
    /// Hi-hat and cymbals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hat: Option<DrumSound>,
    /// Everything else.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other: Option<DrumSound>,
}

impl DrumKit {
    /// The sound for `kind`, if the kit has one.
    pub fn sound(&self, kind: DrumKind) -> Option<&DrumSound> {
        match kind {
            DrumKind::Kick => self.kick.as_ref(),
            DrumKind::Snare => self.snare.as_ref(),
            DrumKind::Hat => self.hat.as_ref(),
            DrumKind::Other => self.other.as_ref(),
        }
    }
}

/// Which note keeps a row when a drum hit and a line note start on it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum OnClash {
    /// The drum hit.
    #[default]
    Drum,
    /// The line's note.
    Line,
}

/// The settings of [`interleave_drums`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct InterleaveRules {
    /// Keep hi-hats too. Off by default: a voice shared with a line can't afford them.
    pub hats: bool,
    /// Who keeps a row both start on.
    pub on_clash: OnClash,
}

/// Fold transcribed drum `hits` (GM drum keys, in any order) onto a voice of their own.
///
/// Each hit becomes a note with its kind's instrument and pitch from `kit`, lasting the sound's rows or until the next
/// hit. Of the hits on one row the highest-priority kind wins (kick, snare, hat, other), and of one kind the more
/// confident; the rest are dropped as [`DropReason::Clash`] (as sure as the winner was surer, for one kind; certain
/// across kinds, which follow a fixed priority). Hits whose kind has no sound in the kit are dropped as
/// [`DropReason::Unmapped`]. Decision ids are `drums-<reason>-v<voice>-r<row>-p<semitone>`. The result does not
/// depend on the order of `hits`.
pub fn fold_drums(hits: impl IntoIterator<Item = Note>, at: &Placement, kit: &DrumKit) -> Reduced {
    let mut log = Log::new(at, FOLD_DRUMS, "drums");
    let folded = fold(hits, kit, &mut log, |_| true);
    let mut notes: Vec<Note> = folded.into_iter().map(|(.., n)| n).collect();
    for i in 1..notes.len() {
        let next = notes[i].row;
        let prev = &mut notes[i - 1];
        if prev.length.get() > next - prev.row {
            prev.length = NonZeroU32::new(next - prev.row).expect("rows are distinct");
        }
    }
    Reduced {
        notes,
        decisions: log.decisions,
    }
}

/// Interleave transcribed drum `hits` (GM drum keys, in any order) with a monophonic `line` on one voice.
///
/// The hits are folded as [`fold_drums`] does, leaving hi-hats out unless `rules.hats` ([`DropReason::Interleave`]).
/// A hit cuts the line note sounding over it, which does not resume (a [`Proposal::TrimNote`] decision). Where a hit
/// and a line note start on one row, `rules.on_clash` picks the one that stays and the other is dropped
/// ([`DropReason::Clash`], at confidence 0.5: either could be right; a dropped hit is named as transcribed). Drums keep their instruments and last their
/// sound's rows or until the next note; a line note after a drum hit always starts fresh, a broken tie with a
/// [`Proposal::Retrigger`] decision. Of line notes doubling a row, the highest stays and the rest are dropped as
/// clashes (at 0.5). Ids
/// are `inter-trim-…`, `inter-tie-…` and `inter-<reason>-…`, as for [`fold_drums`].
pub fn interleave_drums(
    line: impl IntoIterator<Item = Note>,
    hits: impl IntoIterator<Item = Note>,
    at: &Placement,
    kit: &DrumKit,
    rules: &InterleaveRules,
) -> Reduced {
    let mut log = Log::new(at, INTERLEAVE_DRUMS, "inter");
    let mut line: Vec<Note> = line.into_iter().collect();
    // A total order, so a row the line doubles keeps the same note whatever the input order: the higher.
    line.sort_by(|a, b| canonical(a, b, Skyline::Highest));
    let sure = Confidence::new(CLASH_CONFIDENCE).expect("in range");
    // Each row's note, and whether it is a drum hit.
    let mut merged: BTreeMap<u32, (bool, Note)> = BTreeMap::new();
    for n in line {
        match merged.entry(n.row) {
            std::collections::btree_map::Entry::Occupied(_) => {
                log.drop(&n, DropReason::Clash, sure);
            }
            std::collections::btree_map::Entry::Vacant(slot) => {
                slot.insert((false, n));
            }
        }
    }
    for (_, raw, hit) in fold(hits, kit, &mut log, |kind| {
        rules.hats || kind != DrumKind::Hat
    }) {
        if merged.contains_key(&hit.row) {
            if rules.on_clash == OnClash::Line {
                // Named as it was transcribed, like every other dropped hit.
                log.drop(&raw, DropReason::Clash, sure);
                continue;
            }
            let (_, gone) = merged.remove(&hit.row).expect("just looked");
            log.drop(&gone, DropReason::Clash, sure);
        }
        // The line note sounding over the hit, if any, ends where the hit starts.
        if let Some((_, (false, before))) = merged.range_mut(..hit.row).next_back()
            && before.length.get() > hit.row - before.row
        {
            let length = NonZeroU32::new(hit.row - before.row).expect("rows are distinct");
            let id = log.id("trim", before.row);
            let id = log.record(
                id,
                before.row,
                Proposal::TrimNote {
                    voice: at.voice,
                    row: before.row,
                    pitch: before.pitch,
                    length,
                    source: before.source,
                },
                Confidence::CERTAIN,
            );
            before.length = length;
            before.decision.get_or_insert(id);
        }
        merged.insert(hit.row, (true, hit));
    }
    let mut notes: Vec<(bool, Note)> = merged.into_values().collect();
    // Drum hits end where the next note starts; line notes were trimmed above, with their decisions.
    for i in 1..notes.len() {
        let next = notes[i].1.row;
        let (drum, prev) = &mut notes[i - 1];
        if *drum && prev.length.get() > next - prev.row {
            prev.length = NonZeroU32::new(next - prev.row).expect("rows are distinct");
        }
    }
    // A tie stays only straight after a line note ending on its row: after a drum hit, or a cut, it starts fresh.
    let mut previous: Option<(bool, u32)> = None; // (a drum?, end row)
    for (drum, n) in &mut notes {
        let tied_to_line = previous == Some((false, n.row));
        if n.attack == Attack::Legato && !tied_to_line {
            let id = log.id("tie", n.row);
            let id = log.record(
                id,
                n.row,
                Proposal::Retrigger {
                    voice: at.voice,
                    row: n.row,
                    pitch: n.pitch,
                    source: n.source,
                },
                Confidence::CERTAIN,
            );
            n.attack = Attack::Fresh;
            n.decision.get_or_insert(id);
        }
        previous = Some((*drum, n.row.saturating_add(n.length.get())));
    }
    Reduced {
        notes: notes.into_iter().map(|(_, n)| n).collect(),
        decisions: log.decisions,
    }
}

/// Fold `hits` into one note per row with the kit's sounds, keeping only kinds `keep` allows (the rest dropped as
/// [`DropReason::Interleave`]; a kind the kit has no sound for is [`DropReason::Unmapped`] first); lengths are the
/// sounds' own. Each kept hit comes with its kind and the hit as transcribed.
fn fold(
    hits: impl IntoIterator<Item = Note>,
    kit: &DrumKit,
    log: &mut Log,
    keep: impl Fn(DrumKind) -> bool,
) -> Vec<(DrumKind, Note, Note)> {
    let kind = |n: &Note| DrumKind::from_midi(n.pitch.midi());
    let mut hits: Vec<Note> = hits.into_iter().collect();
    // Canonical order: row, priority, confidence, then everything else, so no two different hits tie.
    hits.sort_by(|a, b| {
        let source = |n: &Note| n.source.map(|s| (s.content(), s.start_us(), s.end_us()));
        (a.row, kind(a))
            .cmp(&(b.row, kind(b)))
            .then(b.confidence.get().total_cmp(&a.confidence.get()))
            .then_with(|| a.pitch.cmp(&b.pitch))
            .then_with(|| a.length.cmp(&b.length))
            .then_with(|| source(a).cmp(&source(b)))
            .then_with(|| a.decision.cmp(&b.decision))
            .then_with(|| a.instrument.cmp(&b.instrument))
            .then_with(|| (a.attack == Attack::Legato).cmp(&(b.attack == Attack::Legato)))
    });
    let mut out: Vec<(DrumKind, Note, Note)> = Vec::with_capacity(hits.len());
    for hit in hits {
        let k = kind(&hit);
        let Some(sound) = kit.sound(k) else {
            log.drop(&hit, DropReason::Unmapped, Confidence::CERTAIN);
            continue;
        };
        if !keep(k) {
            log.drop(&hit, DropReason::Interleave, Confidence::CERTAIN);
            continue;
        }
        if let Some((winner, w, _)) = out.last().filter(|(_, raw, _)| raw.row == hit.row) {
            // Across kinds the priority decides; within one, the confidence did.
            let sure = if *winner == k {
                (w.confidence.get() - hit.confidence.get()).clamp(0.0, 1.0)
            } else {
                1.0
            };
            log.drop(
                &hit,
                DropReason::Clash,
                Confidence::new(sure).expect("clamped"),
            );
            continue;
        }
        out.push((
            k,
            hit.clone(),
            Note {
                row: hit.row,
                length: sound.rows,
                pitch: sound.pitch,
                attack: Attack::Fresh,
                instrument: Some(sound.instrument.clone()),
                source: hit.source,
                confidence: hit.confidence,
                decision: hit.decision,
            },
        ));
    }
    out
}

#[cfg(test)]
mod tests;
