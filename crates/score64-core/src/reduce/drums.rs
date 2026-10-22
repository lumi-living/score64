//! Drum folding (#43): transcribed drum hits onto a voice of their own, or interleaved with another line (a bass, say)
//! when no voice is free for drums.
//!
//! The sounds themselves (a noise snare, a triangle kick) are instrument presets; this module refers to them by
//! preset id through a project's [`DrumKit`].

use std::collections::BTreeMap;
use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};

use crate::provenance::{Confidence, Decision, DecisionId, DropReason, Proposal, Provenance};
use crate::score::{Attack, Note, Pitch, PresetId};

use super::{Placement, Reduced};

/// The rule name and version [`fold_drums`] records in its decisions.
pub const FOLD_DRUMS: &str = "fold-drums@1";

/// The rule name and version [`interleave_drums`] records in its decisions.
pub const INTERLEAVE_DRUMS: &str = "interleave-drums@1";

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
    /// Hi-hats (GM 42, 44, 46) and cymbals (GM 49, 51, 52, 55, 57, 59).
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
            42 | 44 | 46 | 49 | 51 | 52 | 55 | 57 | 59 => DrumKind::Hat,
            _ => DrumKind::Other,
        }
    }
}

/// One drum sound: the preset that plays it, the pitch it is played at, and how many rows it lasts at most.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct DrumSound {
    /// The instrument preset.
    pub instrument: PresetId,
    /// The pitch to play it at.
    pub pitch: Pitch,
    /// Its length, in rows, unless the next hit comes sooner.
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

    /// The kind a folded drum note plays, by its instrument.
    pub fn kind_of(&self, note: &Note) -> Option<DrumKind> {
        let instrument = note.instrument.as_ref()?;
        [
            DrumKind::Kick,
            DrumKind::Snare,
            DrumKind::Hat,
            DrumKind::Other,
        ]
        .into_iter()
        .find(|&k| self.sound(k).is_some_and(|s| &s.instrument == instrument))
    }
}

/// Which note keeps a row when a drum hit and a line note start on it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Clash {
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
    pub on_clash: Clash,
}

/// Fold transcribed drum `hits` (General MIDI drum notes, in any order) onto a voice of their own.
///
/// Each hit becomes a note with its kind's instrument and pitch from `kit`, lasting the sound's rows or until the next
/// hit. Of the hits on one row, the highest-priority kind wins (kick, snare, hat, other; then the more confident) and
/// the rest are dropped as [`DropReason::Clash`]; hits whose kind has no sound in the kit are dropped as
/// [`DropReason::Unmapped`]. Each drop is a decision (`drums-<reason>-v<voice>-r<row>-p<semitone>`, with `-2`,
/// ... for repeats). The result does not depend on the order of `hits`.
pub fn fold_drums(hits: impl IntoIterator<Item = Note>, at: &Placement, kit: &DrumKit) -> Reduced {
    let mut log = Log::new(at, FOLD_DRUMS, "drums");
    let mut hits: Vec<Note> = hits.into_iter().collect();
    hits.sort_by(|a, b| {
        let kind = |n: &Note| DrumKind::from_midi(n.pitch.midi());
        (a.row, kind(a))
            .cmp(&(b.row, kind(b)))
            .then(b.confidence.get().total_cmp(&a.confidence.get()))
            .then_with(|| a.pitch.cmp(&b.pitch))
            .then_with(|| a.length.cmp(&b.length))
            .then_with(|| {
                let source = |n: &Note| n.source.map(|s| (s.content(), s.start_us(), s.end_us()));
                source(a).cmp(&source(b))
            })
    });
    let mut line: Vec<Note> = Vec::with_capacity(hits.len());
    for hit in hits {
        let kind = DrumKind::from_midi(hit.pitch.midi());
        let Some(sound) = kit.sound(kind) else {
            log.drop(&hit, DropReason::Unmapped);
            continue;
        };
        if line.last().is_some_and(|n| n.row == hit.row) {
            log.drop(&hit, DropReason::Clash);
            continue;
        }
        line.push(Note {
            row: hit.row,
            length: sound.rows,
            pitch: sound.pitch,
            attack: Attack::Fresh,
            instrument: Some(sound.instrument.clone()),
            source: hit.source,
            confidence: hit.confidence,
            decision: hit.decision,
        });
    }
    end_at_next(&mut line);
    Reduced {
        notes: line,
        decisions: log.decisions,
    }
}

/// Interleave a folded drum line (from [`fold_drums`] with `kit`) with a monophonic `line` on one voice.
///
/// Hi-hats are dropped unless `rules.hats` ([`DropReason::Interleave`]). A hit cuts the line note sounding over it,
/// which does not resume (a [`Proposal::TrimNote`] decision); where a hit and a line note start on the same row,
/// `rules.on_clash` picks the one that stays and the other is dropped ([`DropReason::Clash`]). The drums keep their
/// instruments, so the voice line's own instrument plays only the line's notes. A tie broken this way starts fresh (a
/// [`Proposal::Retrigger`] decision). Decision ids are `inter-trim-v<voice>-r<row>`, `inter-tie-v<voice>-r<row>` and
/// `inter-<reason>-v<voice>-r<row>-p<semitone>`.
pub fn interleave_drums(
    line: Vec<Note>,
    drums: Vec<Note>,
    at: &Placement,
    kit: &DrumKit,
    rules: &InterleaveRules,
) -> Reduced {
    let mut log = Log::new(at, INTERLEAVE_DRUMS, "inter");
    let mut merged: BTreeMap<u32, Note> = line.into_iter().map(|n| (n.row, n)).collect();
    for hit in drums {
        if !rules.hats && kit.kind_of(&hit) == Some(DrumKind::Hat) {
            log.drop(&hit, DropReason::Interleave);
            continue;
        }
        match merged.get(&hit.row) {
            Some(_) if rules.on_clash == Clash::Line => {
                log.drop(&hit, DropReason::Clash);
                continue;
            }
            Some(_) => {
                let gone = merged.remove(&hit.row).expect("just looked");
                log.drop(&gone, DropReason::Clash);
            }
            None => {}
        }
        // The note sounding over the hit, if any, ends where the hit starts.
        if let Some((_, before)) = merged.range_mut(..hit.row).next_back()
            && before.row.saturating_add(before.length.get()) > hit.row
            && kit.kind_of(before).is_none()
        {
            let length = NonZeroU32::new(hit.row - before.row).expect("rows are distinct");
            let id = log.record(
                format!("inter-trim-v{}-r{}", at.voice.get(), before.row),
                before.row,
                Proposal::TrimNote {
                    voice: at.voice,
                    row: before.row,
                    pitch: before.pitch,
                    length,
                    source: before.source,
                },
            );
            before.length = length;
            before.decision.get_or_insert(id);
        }
        merged.insert(hit.row, hit);
    }
    let mut notes: Vec<Note> = merged.into_values().collect();
    // Only drum hits are shortened here: a line note was trimmed above, with its decision, or ends in time already.
    for i in 1..notes.len() {
        let next = notes[i].row;
        let prev = &mut notes[i - 1];
        if prev.row.saturating_add(prev.length.get()) > next && kit.kind_of(prev).is_some() {
            prev.length = NonZeroU32::new(next - prev.row).expect("rows are distinct");
        }
    }
    // A tie that now follows a drum hit, or a cut note, starts fresh: the line it was tied to is broken.
    let mut end = None;
    for n in &mut notes {
        if n.attack == Attack::Legato && end != Some(n.row) {
            let id = log.record(
                format!("inter-tie-v{}-r{}", at.voice.get(), n.row),
                n.row,
                Proposal::Retrigger {
                    voice: at.voice,
                    row: n.row,
                    pitch: n.pitch,
                    source: n.source,
                },
            );
            n.attack = Attack::Fresh;
            n.decision.get_or_insert(id);
        }
        end = Some(n.row.saturating_add(n.length.get()));
    }
    Reduced {
        notes,
        decisions: log.decisions,
    }
}

/// Shorten each note to end where the next starts.
fn end_at_next(line: &mut [Note]) {
    for i in 1..line.len() {
        let next = line[i].row;
        let prev = &mut line[i - 1];
        if prev.row.saturating_add(prev.length.get()) > next {
            prev.length = NonZeroU32::new(next - prev.row).expect("rows are distinct");
        }
    }
}

/// The decisions one call makes.
struct Log<'a> {
    at: &'a Placement<'a>,
    rule: &'static str,
    prefix: &'static str,
    decisions: Vec<Decision>,
}

impl<'a> Log<'a> {
    fn new(at: &'a Placement<'a>, rule: &'static str, prefix: &'static str) -> Self {
        Log {
            at,
            rule,
            prefix,
            decisions: Vec::new(),
        }
    }

    /// Record a decision under `id`, or `id-2`, `id-3`, ... if that is taken.
    fn record(&mut self, id: String, row: u32, proposal: Proposal) -> DecisionId {
        let id = (1..)
            .map(|k| {
                if k == 1 {
                    id.clone()
                } else {
                    format!("{id}-{k}")
                }
            })
            .map(|s| DecisionId::new(s).expect("a valid id"))
            .find(|id| self.decisions.iter().all(|d| &d.id != id))
            .expect("a free id");
        self.decisions.push(Decision {
            id: id.clone(),
            subtune: self.at.subtune.clone(),
            bar: row / u32::from(self.at.rows_per_bar.get()),
            proposal,
            provenance: Provenance::Rules {
                rule: self.rule.into(),
            },
            confidence: Confidence::CERTAIN,
            reviewed: false,
        });
        id
    }

    fn drop(&mut self, note: &Note, reason: DropReason) {
        let what = match reason {
            DropReason::Clash => "clash",
            DropReason::Unmapped => "unmapped",
            DropReason::Interleave => "interleave",
            DropReason::Ghost => "ghost",
            DropReason::TooShort => "short",
            DropReason::Repeat => "repeat",
            DropReason::Skyline => "skyline",
            DropReason::Overlap => "overlap",
        };
        self.record(
            format!(
                "{}-{what}-v{}-r{}-p{}",
                self.prefix,
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
        );
    }
}

#[cfg(test)]
mod tests;
