//! Transcribed parts in source time, and putting them on the beat grid (#36).
//!
//! A transcriber (Basic Pitch per stem, a hosted service, a MIDI file) gives each part as notes in wall-clock time.
//! [`BeatGrid::quantise_part`] turns a part into grid [`Note`]s that point back at their source spans, and reports how
//! well the part fits the grid. It keeps every note it can place, overlaps included: making lines monophonic is the
//! reduction's job (#40).

use serde::{Deserialize, Serialize};

use crate::grid::BeatGrid;
use crate::provenance::{Confidence, ContentHash, SourceRef};
use crate::score::{Attack, Note, Pitch};

/// A note as transcribed, in source time.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct SourceNote {
    /// Start, in microseconds from the start of the source.
    pub start_us: u64,
    /// End (exclusive), in microseconds.
    pub end_us: u64,
    /// MIDI note number (A4 = 69).
    pub midi: u8,
    /// How sure the transcriber was.
    pub confidence: Confidence,
}

/// One transcribed part: the notes of one stem or instrument, and the source they came from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Part {
    /// The stem or source file the notes were transcribed from, by content hash.
    pub source: ContentHash,
    /// The notes, in any order.
    pub notes: Vec<SourceNote>,
}

/// Why a transcribed note could not be put on the grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Unplaced {
    /// Its pitch is outside what GoatTracker can play (C-0 to G#7, MIDI 12–104).
    PitchOutOfRange,
    /// It starts more than half a row before the first beat, before row 0.
    BeforeGrid,
    /// It ends where it starts, or before.
    Empty,
}

/// A transcribed note that was left out, and why.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Dropped {
    /// The note.
    pub note: SourceNote,
    /// Why.
    pub reason: Unplaced,
}

/// How well a part fits the grid. Start errors are in rows, as [`crate::grid::Quantised::start_error`]; only starts
/// are measured, because ends are rounded on their own.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct QuantiseReport {
    /// Notes placed on the grid.
    pub placed: usize,
    /// The mean distance a start moved, in rows (0 to 0.5; 0 when nothing was placed).
    pub mean_start_error: f32,
    /// The largest distance a start moved, in rows.
    pub max_start_error: f32,
    /// Starts that moved more than a quarter of a row: notes the grid fits loosely, worth a listen.
    pub loose: usize,
    /// Notes that could not be placed, in the part's order.
    pub dropped: Vec<Dropped>,
}

/// A part on the grid.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct QuantisedPart {
    /// The source the part came from.
    pub source: ContentHash,
    /// Its notes on the grid, by row and then pitch. Each is a fresh attack with the transcriber's confidence and a
    /// [`SourceRef`] to its span; notes may overlap.
    pub notes: Vec<Note>,
    /// How well it fitted.
    pub report: QuantiseReport,
}

/// A start error above this many rows counts as loose.
const LOOSE: f32 = 0.25;

impl BeatGrid {
    /// Put `part` on this grid: every note it can place, as grid notes, and a report on the fit.
    pub fn quantise_part(&self, part: &Part) -> QuantisedPart {
        let mut placed = Vec::with_capacity(part.notes.len());
        let mut errors = Vec::with_capacity(part.notes.len());
        let mut dropped = Vec::new();
        for &note in &part.notes {
            let reason = match (
                Pitch::from_midi(note.midi),
                SourceRef::new(part.source, note.start_us, note.end_us),
            ) {
                (_, None) => Unplaced::Empty,
                (None, _) => Unplaced::PitchOutOfRange,
                (Some(pitch), Some(source)) => match self.quantise(note.start_us, note.end_us) {
                    None => Unplaced::BeforeGrid,
                    Some(q) => {
                        errors.push(q.start_error.abs());
                        placed.push(Note {
                            row: q.row,
                            length: q.length,
                            pitch,
                            attack: Attack::Fresh,
                            instrument: None,
                            source: Some(source),
                            confidence: note.confidence,
                            decision: None,
                        });
                        continue;
                    }
                },
            };
            dropped.push(Dropped { note, reason });
        }
        placed.sort_by_key(|n| (n.row, n.pitch, n.source.map(|s| s.start_us())));
        let sum: f32 = errors.iter().sum();
        let report = QuantiseReport {
            placed: placed.len(),
            mean_start_error: if errors.is_empty() {
                0.0
            } else {
                sum / errors.len() as f32
            },
            max_start_error: errors.iter().copied().fold(0.0, f32::max),
            loose: errors.iter().filter(|&&e| e > LOOSE).count(),
            dropped,
        };
        QuantisedPart {
            source: part.source,
            notes: placed,
            report,
        }
    }
}

#[cfg(test)]
mod tests;
