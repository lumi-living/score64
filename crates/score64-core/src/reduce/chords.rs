//! Chords (#42): detecting the harmony beat by beat, and playing it as arpeggios on an arp voice.
//!
//! On the SID a chord is one voice cycling through its notes every frame or two: a wavetable arpeggio. The score holds
//! it as a root note with an [`ArpEvent`] of intervals above it; the GoatTracker writer turns that into wavetable
//! rows.

use std::num::{NonZeroU8, NonZeroU32};

use serde::{Deserialize, Serialize};

use crate::provenance::{Confidence, Decision, DecisionId, Proposal, Provenance};
use crate::score::{ArpEvent, Attack, Note, Pitch};
use crate::validate::PitchRange;

use super::{Placement, Skyline, canonical};

/// The rule name and version the chord decisions record.
pub const CHORDS: &str = "chords@1";

/// What a template costs for each tone past three, so a triad wins over a seventh chord unless the seventh sounds.
const EXTRA_TONE_COST: f32 = 0.05;

/// The score a beat's best chord needs: below it, the beat has no chord (a cluster, noise, or a lone note: a melody
/// on its own is not harmony).
pub const MIN_SCORE: f32 = 0.4;

/// The chord qualities detection knows, simplest first: on a tie the earlier one wins.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ChordQuality {
    /// Root, major third, fifth.
    Major,
    /// Root, minor third, fifth.
    Minor,
    /// Root, fourth, fifth.
    Sus4,
    /// Root, second, fifth.
    Sus2,
    /// Major triad and minor seventh.
    Dominant7,
    /// Minor triad and minor seventh.
    Minor7,
    /// Major triad and major seventh.
    Major7,
}

impl ChordQuality {
    /// Every quality, simplest first.
    pub const ALL: [ChordQuality; 7] = [
        ChordQuality::Major,
        ChordQuality::Minor,
        ChordQuality::Sus4,
        ChordQuality::Sus2,
        ChordQuality::Dominant7,
        ChordQuality::Minor7,
        ChordQuality::Major7,
    ];

    /// Semitones above the root, starting with 0.
    pub const fn intervals(self) -> &'static [u8] {
        match self {
            ChordQuality::Major => &[0, 4, 7],
            ChordQuality::Minor => &[0, 3, 7],
            ChordQuality::Sus4 => &[0, 5, 7],
            ChordQuality::Sus2 => &[0, 2, 7],
            ChordQuality::Dominant7 => &[0, 4, 7, 10],
            ChordQuality::Minor7 => &[0, 3, 7, 10],
            ChordQuality::Major7 => &[0, 4, 7, 11],
        }
    }
}

/// A chord: a root pitch class and a quality.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Chord {
    /// Root as a pitch class, 0 (C) to 11 (B).
    pub root: u8,
    /// The quality.
    pub quality: ChordQuality,
}

/// A chord over a span of rows, and how sure detection was.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ChordSpan {
    /// Start row.
    pub row: u32,
    /// Length in rows.
    pub length: NonZeroU32,
    /// The chord.
    pub chord: Chord,
    /// The mean match of its beats, 0 to 1.
    pub confidence: Confidence,
}

/// Detect the chord of each beat of a subtune `length` rows long from `notes` (any parts, overlapping, in any order),
/// and merge consecutive beats with the same chord into one span. Beats count from row 0 in `rows_per_beat` rows.
///
/// Each beat's pitch classes are weighted by the rows each note sounds in the beat, times its confidence. Every root
/// and [`ChordQuality`] is scored: the weight on its tones minus the weight off them, over the beat's weight, scaled
/// by how fully its tones sound (each counts up to the tones' mean weight, so a faint note adds little), less a little
/// for each tone past three. A
/// chord's root must sound. The best scores; on a tie the one rooted on the beat's lowest sounding note, then the
/// simpler quality, then the lower root. A beat with nothing sounding, or whose best scores under
/// [`MIN_SCORE`], has no chord. A span's confidence is the mean of its beats' scores.
pub fn detect_chords(
    notes: &[Note],
    rows_per_beat: NonZeroU8,
    length: NonZeroU32,
) -> Vec<ChordSpan> {
    let beat = u32::from(rows_per_beat.get());
    let beats = length.get().div_ceil(beat);
    // A fixed order, so the weights sum the same whatever order the notes come in.
    let mut notes: Vec<&Note> = notes.iter().collect();
    notes.sort_by(|a, b| canonical(a, b, Skyline::Highest));
    let mut spans: Vec<(ChordSpan, f32, u32)> = Vec::new(); // the span, its score sum and its beat count
    for b in 0..beats {
        let start = b * beat;
        let end = (start + beat).min(length.get());
        let mut weight = [0.0f32; 12];
        let mut lowest: Option<Pitch> = None;
        for n in &notes {
            let from = n.row.max(start);
            let to = n.row.saturating_add(n.length.get()).min(end);
            if from < to && n.confidence.get() > 0.0 {
                weight[usize::from(n.pitch.semitone() % 12)] +=
                    (to - from) as f32 * n.confidence.get();
                lowest = Some(lowest.map_or(n.pitch, |l| l.min(n.pitch)));
            }
        }
        let bass = lowest.map(|p| p.semitone() % 12);
        let Some((chord, score)) = best_chord(&weight, bass).filter(|&(_, s)| s >= MIN_SCORE)
        else {
            continue;
        };
        let span_length = NonZeroU32::new(end - start).expect("a beat has rows");
        match spans.last_mut() {
            Some((last, sum, count))
                if last.chord == chord && last.row + last.length.get() == start =>
            {
                last.length = last.length.saturating_add(span_length.get());
                *sum += score;
                *count += 1;
            }
            _ => spans.push((
                ChordSpan {
                    row: start,
                    length: span_length,
                    chord,
                    confidence: Confidence::CERTAIN,
                },
                score,
                1,
            )),
        }
    }
    spans
        .into_iter()
        .map(|(mut span, sum, count)| {
            span.confidence =
                Confidence::new((sum / count as f32).clamp(0.0, 1.0)).expect("clamped");
            span
        })
        .collect()
}

/// The best chord for a beat's pitch-class weights, and its score. Of chords that score the same (the sus2 on D and
/// the sus4 on A share their notes, say), the one rooted on the lowest sounding note, `bass`, wins.
fn best_chord(weight: &[f32; 12], bass: Option<u8>) -> Option<(Chord, f32)> {
    let total: f32 = weight.iter().sum();
    if total <= 0.0 {
        return None;
    }
    let mut best: Option<(Chord, f32)> = None;
    // Simplest quality first, then lowest root, so a later candidate wins only by scoring higher.
    for quality in ChordQuality::ALL {
        for root in 0u8..12 {
            if weight[usize::from(root)] <= 0.0 {
                continue;
            }
            let tones = quality.intervals();
            // Summed over the pitch classes in order, so chords sharing their notes score exactly the same.
            let mut mask = [false; 12];
            for &i in tones {
                mask[usize::from((root + i) % 12)] = true;
            }
            let on: f32 = (0..12).filter(|&pc| mask[pc]).map(|pc| weight[pc]).sum();
            // How fully the chord sounds: each tone counts up to the tones' mean weight, so a faint note is a faint
            // part of the chord, not a whole one.
            let mean = on / tones.len() as f32;
            let full: f32 = (0..12)
                .filter(|&pc| mask[pc])
                .map(|pc| weight[pc].min(mean))
                .sum();
            let share = if on > 0.0 { full / on } else { 0.0 };
            let score =
                (2.0 * on - total) / total * share - EXTRA_TONE_COST * (tones.len() - 3) as f32;
            let on_bass = |r: u8| bass == Some(r);
            if best.is_none_or(|(c, s)| (score, on_bass(root)) > (s, on_bass(c.root))) {
                best = Some((Chord { root, quality }, score));
            }
        }
    }
    best
}

/// An arp voice's line: root notes, the arpeggios over them, and the decisions that chose the chords.
#[derive(Clone, Debug, PartialEq)]
pub struct Arpeggio {
    /// One root note per chord span, in row order.
    pub notes: Vec<Note>,
    /// One arpeggio per chord span, over its root note.
    pub arps: Vec<ArpEvent>,
    /// One `Chord` decision per span (`chord-v<voice>-r<row>`).
    pub decisions: Vec<Decision>,
}

/// Play `spans` (from [`detect_chords`]) as arpeggios on the voice in `at`: each span's root, in the octave nearest
/// the middle of `range` that keeps every chord tone playable, with an [`ArpEvent`] of the chord's intervals (the
/// lower of two equally near). When no such root lies in the range (a range narrower than an octave, or at the very
/// top), the playable root nearest its middle is taken. Each span is one [`Proposal::Chord`] decision, which the note
/// and the arpeggio record.
pub fn arpeggiate(spans: &[ChordSpan], at: &Placement, range: PitchRange) -> Arpeggio {
    let middle = (i16::from(range.low.semitone()) + i16::from(range.high.semitone())) / 2;
    let mut out = Arpeggio {
        notes: Vec::with_capacity(spans.len()),
        arps: Vec::with_capacity(spans.len()),
        decisions: Vec::with_capacity(spans.len()),
    };
    for span in spans {
        let tones = span.chord.quality.intervals();
        let top = *tones.last().expect("a chord has tones");
        // Roots of this pitch class whose whole chord GoatTracker can play.
        let roots = (0u8..=Pitch::MAX.semitone())
            .filter(|s| s % 12 == span.chord.root && s + top <= Pitch::MAX.semitone());
        let nearest = |s: &u8| (i16::from(*s) - middle).abs();
        let root = roots
            .clone()
            .filter(|&s| range.contains(Pitch::new(s).expect("in range")))
            .min_by_key(nearest)
            .or_else(|| roots.min_by_key(nearest))
            .and_then(Pitch::new)
            .expect("every pitch class has a playable root");
        let id = DecisionId::new(format!("chord-v{}-r{}", at.voice.get(), span.row))
            .expect("a valid id");
        out.decisions.push(Decision {
            id: id.clone(),
            subtune: at.subtune.clone(),
            bar: span.row / u32::from(at.rows_per_bar.get()),
            proposal: Proposal::Chord {
                row: span.row,
                root: span.chord.root,
                intervals: tones.to_vec(),
            },
            provenance: Provenance::Rules {
                rule: CHORDS.into(),
            },
            confidence: span.confidence,
            reviewed: false,
        });
        out.notes.push(Note {
            row: span.row,
            length: span.length,
            pitch: root,
            attack: Attack::Fresh,
            instrument: None,
            source: None,
            confidence: span.confidence,
            decision: Some(id.clone()),
        });
        out.arps.push(ArpEvent {
            row: span.row,
            length: span.length,
            intervals: tones.to_vec(),
            decision: Some(id),
        });
    }
    out
}

#[cfg(test)]
mod tests;
