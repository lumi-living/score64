//! Arpeggios as GoatTracker 2 wavetable programs (#42, `docs/reference/goattracker-files.md`).
//!
//! The wavetable steps one row per frame. A row's left side sets the waveform register (`$10`–`$DF`), leaves it
//! unchanged (`$00`), or first waits `$01`–`$0F` more frames; its right side `$00`–`$5F` then plays that many semitones
//! above the pattern's note. `$FF` on the left jumps to the row on the right without taking a frame (`$00` stops). A
//! pattern row points a voice at a program with command [`SET_WAVE_POINTER`] (`8XY`). Readme v2.72 §3.4.1; the
//! player's stepping is `gplay.c`'s.

use std::fmt;
use std::num::NonZeroU8;

use super::Table;

/// Pattern command `8XY`: set the wavetable pointer to row `XY` (1-based; `$00` stops the wavetable).
pub const SET_WAVE_POINTER: u8 = 0x8;

/// Wavetable left side: jump to the row on the right side.
pub const WAVE_JUMP: u8 = 0xFF;

/// The waveform values a wavetable row can set. Lower values are delays or no change, and higher ones run commands
/// or jump.
pub const WAVEFORMS: std::ops::RangeInclusive<u8> = 0x10..=0xDF;

/// The waveform register's gate bit. The player writes the wavetable's waveform ANDed with the gate mask, so a
/// waveform without it releases the note.
pub const GATE: u8 = 0x01;

/// The waveform register's test bit, which silences and resets the oscillator.
pub const TEST: u8 = 0x08;

/// Most frames one step can take: a delay row waits up to 15 frames before its own.
pub const MAX_FRAMES_PER_STEP: u8 = 16;

/// The highest relative note a wavetable row can play; `$60` and above mean lower pitches or absolute notes.
pub const MAX_INTERVAL: u8 = 0x5F;

/// Why an arpeggio can't be written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WaveError {
    /// The waveform isn't a waveform value (`$10`–`$DF`) with the gate bit on and the test bit off.
    Waveform(u8),
    /// A step can't take more than 16 frames.
    FramesPerStep(u8),
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
            WaveError::Waveform(w) => write!(
                f,
                "waveform ${w:02X} is not $10-$DF with the gate bit on and the test bit off"
            ),
            WaveError::FramesPerStep(n) => {
                write!(f, "{n} frames a step; at most {MAX_FRAMES_PER_STEP}")
            }
            WaveError::Interval(i) => write!(f, "interval {i} is above {MAX_INTERVAL} semitones"),
            WaveError::Empty => write!(f, "an arpeggio needs at least one interval"),
            WaveError::Full => write!(f, "the wavetable would pass 255 rows"),
        }
    }
}

impl std::error::Error for WaveError {}

/// The rows of an arpeggio program starting at row `start`, in the readme's idiom: the waveform with the first
/// interval, then each further interval on a row that keeps the waveform and waits `frames_per_step` − 1 frames
/// first, then the first interval again, and a jump back to the second row. One interval is held: its row, then a
/// stop.
fn program(waveform: u8, intervals: &[u8], frames_per_step: NonZeroU8, start: u8) -> Vec<(u8, u8)> {
    let mut rows = vec![(waveform, intervals[0])];
    if intervals.len() == 1 {
        rows.push((WAVE_JUMP, 0));
        return rows;
    }
    // $00 keeps the waveform; $01-$0F also waits that many frames first.
    let wait = frames_per_step.get() - 1;
    rows.extend(intervals[1..].iter().map(|&i| (wait, i)));
    rows.push((wait, intervals[0]));
    rows.push((WAVE_JUMP, start + 1));
    rows
}

/// Write an arpeggio into `table` and return its first row (1-based), for [`SET_WAVE_POINTER`] on the note's row.
/// The voice cycles through `intervals` (semitones above the note), `frames_per_step` frames each, with `waveform`
/// held, until something else sets its wavetable pointer. An identical program already in the table is reused.
///
/// For the `.sng` writer: `8XY` replaces the instrument's own wave program for the note; the player adds an interval
/// to the note modulo 128, so the note plus its largest interval must stay at or below B-7; and the wavetable doesn't
/// run on a note's first frame (`gplay.c`), so the arpeggio starts on its second.
pub fn arpeggio(
    table: &mut Table,
    waveform: u8,
    intervals: &[u8],
    frames_per_step: NonZeroU8,
) -> Result<u8, WaveError> {
    if !WAVEFORMS.contains(&waveform) || waveform & GATE == 0 || waveform & TEST != 0 {
        return Err(WaveError::Waveform(waveform));
    }
    if frames_per_step.get() > MAX_FRAMES_PER_STEP {
        return Err(WaveError::FramesPerStep(frames_per_step.get()));
    }
    if intervals.is_empty() {
        return Err(WaveError::Empty);
    }
    // A table already past what a song can hold (`Table.rows` is public).
    if table.rows.len() > 255 {
        return Err(WaveError::Full);
    }
    if let Some(&i) = intervals.iter().find(|&&i| i > MAX_INTERVAL) {
        return Err(WaveError::Interval(i));
    }
    let len = program(waveform, intervals, frames_per_step, 1).len();
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
