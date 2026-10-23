//! Arpeggios as GoatTracker 2 wavetable programs (#42, `docs/reference/goattracker-files.md`).
//!
//! The wavetable runs one row per frame. A row's left side sets the waveform register (`$10`–`$DF`), and its right side
//! `$00`–`$5F` plays that many semitones above the pattern's note. `$FF` on the left jumps to the row on the right. A
//! pattern row points a voice at a program with command [`SET_WAVE_POINTER`] (`8XY`).

use std::fmt;
use std::num::NonZeroU8;

use super::Table;

/// Pattern command `8XY`: set the wavetable pointer to row `XY` (1-based; `$00` stops the wavetable).
pub const SET_WAVE_POINTER: u8 = 0x8;

/// Wavetable left side: jump to the row on the right side.
pub const WAVE_JUMP: u8 = 0xFF;

/// The audible waveform values a wavetable row can set. Lower values are delays or no change, and higher ones run
/// commands or jump.
pub const WAVEFORMS: std::ops::RangeInclusive<u8> = 0x10..=0xDF;

/// The highest relative note a wavetable row can play; `$60` and above mean lower pitches or absolute notes.
pub const MAX_INTERVAL: u8 = 0x5F;

/// Why an arpeggio can't be written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WaveError {
    /// The waveform isn't an audible waveform value (`$10`–`$DF`).
    Waveform(u8),
    /// An interval is above `$5F` semitones.
    Interval(u8),
    /// The chord has no intervals.
    Empty,
    /// The wavetable would pass 255 rows.
    Full,
}

impl fmt::Display for WaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WaveError::Waveform(w) => write!(f, "waveform ${w:02X} is not $10-$DF"),
            WaveError::Interval(i) => write!(f, "interval {i} is above {MAX_INTERVAL} semitones"),
            WaveError::Empty => write!(f, "an arpeggio needs at least one interval"),
            WaveError::Full => write!(f, "the wavetable would pass 255 rows"),
        }
    }
}

impl std::error::Error for WaveError {}

/// The rows of an arpeggio program starting at row `start`: each interval with `waveform`, held for
/// `frames_per_step` frames, then a jump back to `start`.
fn program(waveform: u8, intervals: &[u8], frames_per_step: NonZeroU8, start: u8) -> Vec<(u8, u8)> {
    let mut rows: Vec<(u8, u8)> = intervals
        .iter()
        .flat_map(|&i| std::iter::repeat_n((waveform, i), usize::from(frames_per_step.get())))
        .collect();
    rows.push((WAVE_JUMP, start));
    rows
}

/// Write an arpeggio into `table` and return its first row (1-based), for [`SET_WAVE_POINTER`] on the note's row.
/// The voice cycles through `intervals` (semitones above the note), `frames_per_step` frames each, with `waveform`
/// held, until something else sets its wavetable pointer. An identical program already in the table is reused.
pub fn arpeggio(
    table: &mut Table,
    waveform: u8,
    intervals: &[u8],
    frames_per_step: NonZeroU8,
) -> Result<u8, WaveError> {
    if !WAVEFORMS.contains(&waveform) {
        return Err(WaveError::Waveform(waveform));
    }
    if intervals.is_empty() {
        return Err(WaveError::Empty);
    }
    if let Some(&i) = intervals.iter().find(|&&i| i > MAX_INTERVAL) {
        return Err(WaveError::Interval(i));
    }
    let len = intervals.len() * usize::from(frames_per_step.get()) + 1;
    // An earlier copy: the same rows, jumping back to where they start.
    for at in 0..table.rows.len().saturating_sub(len - 1) {
        let start = (at + 1) as u8;
        if table.rows[at..at + len] == program(waveform, intervals, frames_per_step, start)[..] {
            return Ok(start);
        }
    }
    if table.rows.len() + len > 255 {
        return Err(WaveError::Full);
    }
    let start = (table.rows.len() + 1) as u8;
    table
        .rows
        .extend(program(waveform, intervals, frames_per_step, start));
    Ok(start)
}

#[cfg(test)]
mod tests;
