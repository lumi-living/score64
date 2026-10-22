//! The tempo grid (`docs/reference/tempo-grid.md`): snapping a detected tempo to a whole number of player frames per
//! row, and quantising source times onto rows.
//!
//! Two clocks meet here. The source has a beat grid in wall-clock time (from the beat tracker), which places notes on
//! rows. The C64 plays rows at a whole number of frames each, at the frame rate of its video standard
//! ([`VideoStandard`]: PAL or NTSC), which fixes the tempo the music is heard at. [`snap_tempo`] chooses the row
//! length and reports how far the C64's tempo is from the source's; [`BeatGrid`] turns source times into rows.

use std::num::{NonZeroU8, NonZeroU32};

use serde::{Deserialize, Serialize};

use crate::score::FramesPerRow;

/// The video standard of the C64 a song is made for (#139). It sets the frame rate the player runs at, and so the
/// tempo every row length plays at; PAL unless the project says otherwise.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum VideoStandard {
    /// PAL (VIC-II 6569): 312 lines of 63 cycles, 985,248 Hz, about 50.1246 frames a second.
    #[default]
    Pal,
    /// NTSC (VIC-II 6567R8): 263 lines of 65 cycles, 1,022,727 Hz (14.31818 MHz / 14), about 59.826 frames a second.
    Ntsc,
}

impl VideoStandard {
    /// The CPU clock, in Hz.
    pub const fn clock_hz(self) -> u32 {
        match self {
            VideoStandard::Pal => 985_248,
            VideoStandard::Ntsc => 1_022_727,
        }
    }

    /// CPU cycles per video frame: raster lines × cycles per line.
    pub const fn cycles_per_frame(self) -> u32 {
        match self {
            VideoStandard::Pal => 312 * 63,
            VideoStandard::Ntsc => 263 * 65,
        }
    }

    /// The real frame rate, in Hz: the clock over the cycles per frame.
    pub const fn frame_rate(self) -> f64 {
        self.clock_hz() as f64 / self.cycles_per_frame() as f64
    }

    /// The rounded frame rate tempos are quoted at: 50 Hz for PAL, as trackers do (GoatTracker: BPM = 750 / frames
    /// per row at four rows per beat), and by the same rule 60 Hz for NTSC (Score64's convention).
    pub const fn nominal_frame_rate(self) -> f64 {
        match self {
            VideoStandard::Pal => 50.0,
            VideoStandard::Ntsc => 60.0,
        }
    }
}

/// The PAL C64's real frame rate: 985,248 Hz CPU clock / 19,656 cycles per frame ≈ 50.1246 Hz.
pub const PAL_FRAME_RATE: f64 = VideoStandard::Pal.frame_rate();

/// The NTSC C64's real frame rate: 1,022,727 Hz CPU clock / 17,095 cycles per frame ≈ 59.826 Hz.
pub const NTSC_FRAME_RATE: f64 = VideoStandard::Ntsc.frame_rate();

/// The rounded frame rate PAL trackers quote tempos at (GoatTracker: BPM = 750 / frames per row at four rows per
/// beat).
pub const PAL_NOMINAL_FRAME_RATE: f64 = VideoStandard::Pal.nominal_frame_rate();

/// The tempo, in beats per minute, of `frames_per_row` at `rows_per_beat` and `frame_rate`.
pub fn bpm_at(frames_per_row: FramesPerRow, rows_per_beat: NonZeroU8, frame_rate: f64) -> f64 {
    frame_rate * 60.0 / (f64::from(rows_per_beat.get()) * f64::from(frames_per_row.get()))
}

/// A detected tempo snapped to the grid, with how far the C64 ends up from the source.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct TempoSnap {
    /// The tempo the source was detected at, in BPM.
    pub detected_bpm: f64,
    /// Rows per beat the snap assumed.
    pub rows_per_beat: NonZeroU8,
    /// The video standard the snap was for.
    pub standard: VideoStandard,
    /// The chosen row length.
    pub frames_per_row: FramesPerRow,
    /// The tempo a tracker quotes for it, at the standard's nominal rate (50 Hz PAL, 60 Hz NTSC).
    pub nominal_bpm: f64,
    /// The tempo a C64 of that standard really plays it at.
    pub real_bpm: f64,
    /// `real_bpm / detected_bpm - 1`: positive means the C64 plays faster than the source.
    pub error: f64,
    /// How far the C64 gets ahead of (positive) or behind the source per minute of source, in milliseconds. Matters
    /// when the music must stay in step with something else, such as video or a recorded part.
    pub drift_ms_per_minute: f64,
}

/// Snap `detected_bpm` to the row length (3–127 frames) whose real tempo on a `standard` C64 is nearest, at
/// `rows_per_beat` rows per beat. `None` when the tempo is not a positive, finite number. A tempo outside what 3–127
/// frames per row can play snaps to the nearest end, and the error says how far off that is.
pub fn snap_tempo(
    detected_bpm: f64,
    rows_per_beat: NonZeroU8,
    standard: VideoStandard,
) -> Option<TempoSnap> {
    if !(detected_bpm.is_finite() && detected_bpm > 0.0) {
        return None;
    }
    let frame_rate = standard.frame_rate();
    let ideal = frame_rate * 60.0 / (f64::from(rows_per_beat.get()) * detected_bpm);
    let lo = ideal.floor().clamp(
        f64::from(FramesPerRow::MIN.get()),
        f64::from(FramesPerRow::MAX.get()),
    );
    let candidates = [lo, lo + 1.0].map(|f| {
        let f = f.clamp(
            f64::from(FramesPerRow::MIN.get()),
            f64::from(FramesPerRow::MAX.get()),
        );
        FramesPerRow::new(f as u8).expect("clamped to the legal range")
    });
    let error = |f: FramesPerRow| bpm_at(f, rows_per_beat, frame_rate) / detected_bpm - 1.0;
    let frames_per_row = candidates
        .into_iter()
        .min_by(|a, b| error(*a).abs().total_cmp(&error(*b).abs()))
        .expect("two candidates");
    let real_bpm = bpm_at(frames_per_row, rows_per_beat, frame_rate);
    let error = real_bpm / detected_bpm - 1.0;
    Some(TempoSnap {
        detected_bpm,
        rows_per_beat,
        standard,
        frames_per_row,
        nominal_bpm: bpm_at(frames_per_row, rows_per_beat, standard.nominal_frame_rate()),
        real_bpm,
        error,
        // A source minute holds detected_bpm beats; the C64 plays them in 60 / (1 + error) seconds.
        drift_ms_per_minute: 60_000.0 * error / (1.0 + error),
    })
}

/// A source's beat grid: beat times in microseconds from the start of the source, and how many rows each beat has.
/// Row 0 is the first beat. Between beats, rows are spread evenly, so tempo changes and rubato in the source do not
/// add up into drift; before the first and after the last beat, the nearest beat interval carries on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(try_from = "BeatGridFields", into = "BeatGridFields")]
pub struct BeatGrid {
    beats_us: Vec<u64>,
    rows_per_beat: NonZeroU8,
}

/// [`BeatGrid`] as written in files.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct BeatGridFields {
    /// Beat times in microseconds, at least two, strictly increasing.
    beats_us: Vec<u64>,
    /// Rows per beat (4 for sixteenth-note rows).
    rows_per_beat: NonZeroU8,
}

/// Where a source span lands on the grid.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Quantised {
    /// Start row.
    pub row: u32,
    /// Length in rows (at least one).
    pub length: NonZeroU32,
    /// How far the start moved, in rows (positive: later). At most half a row.
    pub start_error: f32,
}

impl BeatGrid {
    /// A grid, if there are at least two beats in strictly increasing order.
    pub fn new(beats_us: Vec<u64>, rows_per_beat: NonZeroU8) -> Result<Self, String> {
        if beats_us.len() < 2 {
            return Err("a beat grid needs at least two beats".into());
        }
        if beats_us.windows(2).any(|w| w[0] >= w[1]) {
            return Err("beat times must strictly increase".into());
        }
        Ok(BeatGrid {
            beats_us,
            rows_per_beat,
        })
    }

    /// Rows per beat.
    pub fn rows_per_beat(&self) -> NonZeroU8 {
        self.rows_per_beat
    }

    /// The beat times.
    pub fn beats_us(&self) -> &[u64] {
        &self.beats_us
    }

    /// The tempo of the median beat interval, in BPM: a robust single tempo for [`snap_tempo`].
    pub fn median_bpm(&self) -> f64 {
        let mut gaps: Vec<u64> = self.beats_us.windows(2).map(|w| w[1] - w[0]).collect();
        gaps.sort_unstable();
        let n = gaps.len();
        let median = if n % 2 == 1 {
            gaps[n / 2] as f64
        } else {
            (gaps[n / 2 - 1] + gaps[n / 2]) as f64 / 2.0
        };
        60e6 / median
    }

    /// The fractional row of source time `t_us` (negative before the first beat).
    pub fn row_at(&self, t_us: u64) -> f64 {
        let b = &self.beats_us;
        let rpb = f64::from(self.rows_per_beat.get());
        // The beat interval containing t, or the first or last one beyond the ends.
        let i = b.partition_point(|&x| x <= t_us).clamp(1, b.len() - 1) - 1;
        let (start, end) = (b[i] as f64, b[i + 1] as f64);
        (i as f64 + (t_us as f64 - start) / (end - start)) * rpb
    }

    /// The span `start_us..end_us` on the grid: start rounded to the nearest row, end rounded likewise, at least one
    /// row long. `None` for an empty span or one that starts before row 0 (more than half a row before the first
    /// beat).
    pub fn quantise(&self, start_us: u64, end_us: u64) -> Option<Quantised> {
        if end_us <= start_us {
            return None;
        }
        let start = self.row_at(start_us);
        let row = start.round();
        if row < 0.0 || row > f64::from(u32::MAX) {
            return None;
        }
        let end = self.row_at(end_us).round().max(row + 1.0);
        let length = (end - row).min(f64::from(u32::MAX));
        Some(Quantised {
            row: row as u32,
            length: NonZeroU32::new(length as u32).expect("at least one row"),
            start_error: (row - start) as f32,
        })
    }
}

impl TryFrom<BeatGridFields> for BeatGrid {
    type Error = String;
    fn try_from(f: BeatGridFields) -> Result<Self, String> {
        BeatGrid::new(f.beats_us, f.rows_per_beat)
    }
}

impl From<BeatGrid> for BeatGridFields {
    fn from(g: BeatGrid) -> Self {
        BeatGridFields {
            beats_us: g.beats_us,
            rows_per_beat: g.rows_per_beat,
        }
    }
}

#[cfg(test)]
mod tests;
