//! Arpeggios on the bass's weak beats (#164): when no voice is free for an arp, the bass plays its notes on the
//! strong beats and the chord as an arpeggio on the weak ones.

use std::num::{NonZeroU8, NonZeroU32};

use serde::{Deserialize, Serialize};

use crate::provenance::{Confidence, DropReason, Proposal};
use crate::score::{ArpEvent, Attack, Note, Pitch};
use crate::validate::PitchRange;

use super::{Arpeggio, ChordSpan, Log, Placement};

/// The rule name and version [`arpeggiate_on_bass`] records in its decisions.
pub const BASS_ARPS: &str = "bass-arps@1";

/// Where a bass arpeggio's root sits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum BassArpRoot {
    /// The chord's root nearest the bass note before it, inside the bass range.
    #[default]
    InRange,
    /// The chord's root nearest an octave above the bass note before it.
    OctaveAbove,
}

/// The settings of [`arpeggiate_on_bass`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct BassArpRules {
    /// Rows in a beat.
    pub rows_per_beat: NonZeroU8,
    /// The weak beats of a bar, counted from 1: where the arpeggios go.
    pub weak_beats: Vec<u8>,
    /// Where each arpeggio's root sits.
    pub root: BassArpRoot,
}

impl Default for BassArpRules {
    /// Beats 2 and 4 of a 4/4 bar of four-row beats; roots in the bass range.
    fn default() -> Self {
        BassArpRules {
            rows_per_beat: NonZeroU8::new(4).expect("non-zero"),
            weak_beats: vec![2, 4],
            root: BassArpRoot::InRange,
        }
    }
}

/// Put `spans`' chords (from `detect_chords`) on the weak beats of the `bass` line on the voice in `at`, as
/// arpeggios over a root note, and keep the bass's own notes on the other beats.
///
/// A weak beat inside a chord span gets that chord from the beat's first row to the beat's end (or the span's, if
/// sooner). There, bass notes that start inside the beat are dropped ([`DropReason::Interleave`]); a bass note sounding
/// into it ends where it starts ([`Proposal::TrimNote`]), and a tie that lost its note starts fresh
/// ([`Proposal::Retrigger`]). Each arpeggio's root comes from the chord root, nearest the bass note before it (or the
/// middle of `range` when none comes before), inside `range` ([`BassArpRoot::InRange`]) or an octave above the bass
/// ([`BassArpRoot::OctaveAbove`]), always low enough for the whole chord to stay playable. Each arpeggio is one
/// [`Proposal::Chord`] decision (`bassarp-chord-v<voice>-r<row>`, as sure as the span), which its root note and
/// arpeggio record; the drops, trims and restarts are certain, with ids `bassarp-<kind>-…` as for monophonise. Beats
/// without a chord keep the bass as it is. The bass is expected one note at a time (after `monophonise`).
pub fn arpeggiate_on_bass(
    bass: impl IntoIterator<Item = Note>,
    spans: &[ChordSpan],
    at: &Placement,
    rules: &BassArpRules,
    range: PitchRange,
) -> Arpeggio {
    let mut log = Log::new(at, BASS_ARPS, "bassarp");
    let mut notes: Vec<Note> = bass.into_iter().collect();
    notes.sort_by_key(|n| (n.row, std::cmp::Reverse(n.pitch)));
    let bar = u32::from(at.rows_per_bar.get());
    let beat = u32::from(rules.rows_per_beat.get());
    // Each weak beat inside a span: its rows, and the span.
    let mut windows: Vec<(u32, u32, &ChordSpan)> = Vec::new();
    for span in spans {
        let (s0, s1) = (span.row, span.row.saturating_add(span.length.get()));
        for b in (s0 / bar)..=(s1.saturating_sub(1) / bar) {
            for &w in &rules.weak_beats {
                // Beat 0, or one past the bar, isn't a beat of it.
                let Some(offset) = u32::from(w)
                    .checked_sub(1)
                    .map(|k| k * beat)
                    .filter(|&o| o < bar)
                else {
                    continue;
                };
                let start = b * bar + offset;
                if start < s0 || start >= s1 {
                    continue;
                }
                let end = (start + beat).min((b + 1) * bar).min(s1);
                windows.push((start, end, span));
            }
        }
    }
    windows.sort_by_key(|w| w.0);
    windows.dedup_by_key(|w| w.0);

    let middle = (i16::from(range.low.semitone()) + i16::from(range.high.semitone())) / 2;
    let mut out = Arpeggio {
        notes: Vec::new(),
        arps: Vec::new(),
        decisions: Vec::new(),
    };
    let mut roots = Vec::with_capacity(windows.len());
    for &(start, end, span) in &windows {
        // The bass notes starting in the beat go; one sounding into it ends there.
        let mut kept = Vec::with_capacity(notes.len());
        for n in notes.drain(..) {
            if n.row >= start && n.row < end {
                log.drop(&n, DropReason::Interleave, Confidence::CERTAIN);
            } else {
                kept.push(n);
            }
        }
        notes = kept;
        let before = notes.iter_mut().filter(|n| n.row < start).last();
        let anchor = before.as_ref().map(|n| i16::from(n.pitch.semitone()));
        if let Some(n) = before
            && n.row + n.length.get() > start
        {
            let length = NonZeroU32::new(start - n.row).expect("starts before the beat");
            let id = log.id("trim", n.row);
            let id = log.record(
                id,
                n.row,
                Proposal::TrimNote {
                    voice: at.voice,
                    row: n.row,
                    pitch: n.pitch,
                    length,
                    source: n.source,
                },
                Confidence::CERTAIN,
            );
            n.length = length;
            n.decision.get_or_insert(id);
        }
        let tones = span.chord.quality.intervals();
        let top = *tones.last().expect("a chord has tones");
        let target = match (rules.root, anchor) {
            (BassArpRoot::OctaveAbove, Some(a)) => a + 12,
            (_, Some(a)) => a,
            (_, None) => middle,
        };
        let playable = (0u8..=Pitch::MAX.semitone())
            .filter(|s| s % 12 == span.chord.root && s + top <= Pitch::MAX.semitone());
        let nearest = |s: &u8| ((i16::from(*s) - target).abs(), *s);
        let root = match rules.root {
            BassArpRoot::InRange => playable
                .clone()
                .filter(|&s| range.contains(Pitch::new(s).expect("in range")))
                .min_by_key(nearest)
                .or_else(|| playable.clone().min_by_key(nearest)),
            BassArpRoot::OctaveAbove => playable.clone().min_by_key(nearest),
        }
        .and_then(Pitch::new)
        .expect("every pitch class has a playable root");
        let id = log.record(
            log.id("chord", start),
            start,
            Proposal::Chord {
                row: start,
                root: span.chord.root,
                intervals: tones.to_vec(),
            },
            span.confidence,
        );
        let length = NonZeroU32::new(end - start).expect("a beat has rows");
        roots.push(Note {
            row: start,
            length,
            pitch: root,
            attack: Attack::Fresh,
            instrument: None,
            source: None,
            confidence: span.confidence,
            decision: Some(id.clone()),
        });
        out.arps.push(ArpEvent {
            row: start,
            length,
            intervals: tones.to_vec(),
            decision: Some(id),
        });
    }
    // The bass and the roots, one line; a tie stays only straight after the note it's tied to.
    let mut line: Vec<(bool, Note)> = notes
        .into_iter()
        .map(|n| (false, n))
        .chain(roots.into_iter().map(|n| (true, n)))
        .collect();
    line.sort_by_key(|(_, n)| n.row);
    let mut previous: Option<(bool, u32)> = None; // (a root?, end row)
    for (root, n) in &mut line {
        let tied = previous == Some((false, n.row));
        if !*root && n.attack == Attack::Legato && !tied {
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
        previous = Some((*root, n.row.saturating_add(n.length.get())));
    }
    out.notes = line.into_iter().map(|(_, n)| n).collect();
    out.decisions = log.decisions;
    out
}

#[cfg(test)]
mod tests;
