// Rust translation of parts of Basic Pitch (https://github.com/spotify/basic-pitch, v0.4.0):
// Copyright 2022 Spotify AB, licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0). Changed: translated from Python to Rust; pitch bends left out.
// See THIRD_PARTY.md.

//! Frames → notes: a port of `basic_pitch.note_creation.output_to_notes_polyphonic` and `model_frames_to_time`
//! (basic-pitch 0.4.0, Apache-2.0). Kept line for line close to the original so the differential test against the
//! Python package (`sidecar/basic_pitch_reference.py`) stays meaningful. Pitch bends are not ported: the tracker grid
//! has no use for them yet.
//!
//! Precision follows numpy: the model output is `f32`; inferred onsets and the remaining energy are `f64` (numpy
//! promotes when it concatenates with `np.zeros`); note amplitudes are means of the `f32` frames.

use super::{ANNOT_N_FRAMES, AUDIO_N_SAMPLES, AUDIO_SAMPLE_RATE, FFT_HOP, N_NOTES};

/// MIDI pitch of the lowest model bin (A0).
pub const MIDI_OFFSET: u8 = 21;
const MAX_FREQ_IDX: usize = 87;

/// Note-creation settings. The defaults are `basic_pitch.inference.predict()`'s.
#[derive(Clone, Debug, PartialEq)]
pub struct NoteParams {
    /// Minimum onset activation for a note to start.
    pub onset_thresh: f64,
    /// Minimum frame activation for a note to continue.
    pub frame_thresh: f64,
    /// Minimum note length in frames (127.70 ms at 86 frames/s rounds to 11).
    pub min_note_len: usize,
    /// Add onsets where frame activations jump.
    pub infer_onsets: bool,
    /// Also turn energy left over after onset-based notes into notes.
    pub melodia_trick: bool,
    /// Frames below the threshold tolerated inside a note.
    pub energy_tol: usize,
    /// Drop activations above this frequency (Hz).
    pub max_freq: Option<f64>,
    /// Drop activations below this frequency (Hz).
    pub min_freq: Option<f64>,
}

impl Default for NoteParams {
    fn default() -> Self {
        Self {
            onset_thresh: 0.5,
            frame_thresh: 0.3,
            min_note_len: 11,
            infer_onsets: true,
            melodia_trick: true,
            energy_tol: 11,
            max_freq: None,
            min_freq: None,
        }
    }
}

/// One detected note, in model frames.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NoteEvent {
    /// First frame.
    pub start_frame: usize,
    /// Frame after the last one (exclusive, as in the original).
    pub end_frame: usize,
    /// MIDI pitch.
    pub pitch: u8,
    /// Mean frame activation over the note, 0–1.
    pub amplitude: f32,
}

/// A row-major `(frames, 88)` matrix.
struct Grid<T> {
    data: Vec<T>,
}

impl<T: Copy> Grid<T> {
    fn at(&self, t: usize, f: usize) -> T {
        self.data[t * N_NOTES + f]
    }
    fn set(&mut self, t: usize, f: usize, v: T) {
        self.data[t * N_NOTES + f] = v;
    }
}

fn hz_to_midi(hz: f64) -> f64 {
    12.0 * (hz / 440.0).log2() + 69.0
}

/// `output_to_notes_polyphonic`: `frames` and `onsets` are `(n_frames, 88)`, row-major.
pub fn output_to_notes(frames: &[f32], onsets: &[f32], params: &NoteParams) -> Vec<NoteEvent> {
    assert_eq!(frames.len(), onsets.len());
    assert_eq!(frames.len() % N_NOTES, 0);
    let n_frames = frames.len() / N_NOTES;
    let mut frames = frames.to_vec();
    let mut onsets = onsets.to_vec();

    // constrain_frequency
    let zero_from = |m: &mut [f32], lo: usize, hi: usize| {
        for t in 0..n_frames {
            m[t * N_NOTES + lo..t * N_NOTES + hi].fill(0.0);
        }
    };
    if let Some(f) = params.max_freq {
        let idx = (hz_to_midi(f) - f64::from(MIDI_OFFSET))
            .round()
            .clamp(0.0, N_NOTES as f64) as usize;
        zero_from(&mut onsets, idx, N_NOTES);
        zero_from(&mut frames, idx, N_NOTES);
    }
    if let Some(f) = params.min_freq {
        let idx = (hz_to_midi(f) - f64::from(MIDI_OFFSET))
            .round()
            .clamp(0.0, N_NOTES as f64) as usize;
        zero_from(&mut onsets, 0, idx);
        zero_from(&mut frames, 0, idx);
    }

    let onsets: Vec<f64> = if params.infer_onsets {
        inferred_onsets(&onsets, &frames, n_frames)
    } else {
        onsets.iter().map(|&v| f64::from(v)).collect()
    };
    let onsets = Grid { data: onsets };
    let frames_g = Grid { data: frames };

    // scipy.signal.argrelmax along time (strict, edges never peak), then threshold. np.where gives row-major
    // order; the original walks it backwards in time.
    let mut onset_idx = Vec::new();
    for t in 1..n_frames.saturating_sub(1) {
        for f in 0..N_NOTES {
            let v = onsets.at(t, f);
            if v > onsets.at(t - 1, f) && v > onsets.at(t + 1, f) && v >= params.onset_thresh {
                onset_idx.push((t, f));
            }
        }
    }

    let mut remaining = Grid {
        data: frames_g
            .data
            .iter()
            .map(|&v| f64::from(v))
            .collect::<Vec<f64>>(),
    };
    let mut notes = Vec::new();
    let mean = |start: usize, end: usize, f: usize| -> f32 {
        let s: f32 = (start..end).map(|t| frames_g.at(t, f)).sum();
        s / (end - start) as f32
    };

    for &(note_start, f) in onset_idx.iter().rev() {
        if note_start >= n_frames - 1 {
            continue;
        }
        let mut i = note_start + 1;
        let mut k = 0;
        while i < n_frames - 1 && k < params.energy_tol {
            if remaining.at(i, f) < params.frame_thresh {
                k += 1;
            } else {
                k = 0;
            }
            i += 1;
        }
        i -= k;
        if i - note_start <= params.min_note_len {
            continue;
        }
        for t in note_start..i {
            remaining.set(t, f, 0.0);
            if f < MAX_FREQ_IDX {
                remaining.set(t, f + 1, 0.0);
            }
            if f > 0 {
                remaining.set(t, f - 1, 0.0);
            }
        }
        notes.push(NoteEvent {
            start_frame: note_start,
            end_frame: i,
            pitch: f as u8 + MIDI_OFFSET,
            amplitude: mean(note_start, i, f),
        });
    }

    if params.melodia_trick {
        loop {
            // np.argmax: first maximum in row-major order.
            let (mut best, mut at) = (f64::NEG_INFINITY, 0);
            for (j, &v) in remaining.data.iter().enumerate() {
                if v > best {
                    best = v;
                    at = j;
                }
            }
            if best <= params.frame_thresh {
                break;
            }
            let (i_mid, f) = (at / N_NOTES, at % N_NOTES);
            remaining.set(i_mid, f, 0.0);
            let clear = |remaining: &mut Grid<f64>, i: usize| {
                remaining.set(i, f, 0.0);
                if f < MAX_FREQ_IDX {
                    remaining.set(i, f + 1, 0.0);
                }
                if f > 0 {
                    remaining.set(i, f - 1, 0.0);
                }
            };

            // Forward pass.
            let mut i = i_mid + 1;
            let mut k = 0;
            while i < n_frames - 1 && k < params.energy_tol {
                if remaining.at(i, f) < params.frame_thresh {
                    k += 1;
                } else {
                    k = 0;
                }
                clear(&mut remaining, i);
                i += 1;
            }
            let i_end = i - 1 - k;

            // Backward pass (stops at frame 1, as in the original).
            let mut i = i_mid as isize - 1;
            let mut k = 0;
            while i > 0 && k < params.energy_tol {
                if remaining.at(i as usize, f) < params.frame_thresh {
                    k += 1;
                } else {
                    k = 0;
                }
                clear(&mut remaining, i as usize);
                i -= 1;
            }
            let i_start = (i + 1 + k as isize) as usize;

            if i_end <= i_start || i_end - i_start <= params.min_note_len {
                continue;
            }
            notes.push(NoteEvent {
                start_frame: i_start,
                end_frame: i_end,
                pitch: f as u8 + MIDI_OFFSET,
                amplitude: mean(i_start, i_end, f),
            });
        }
    }
    notes
}

/// `get_infered_onsets` with `n_diff = 2`.
fn inferred_onsets(onsets: &[f32], frames: &[f32], n_frames: usize) -> Vec<f64> {
    const N_DIFF: usize = 2;
    let mut diff = vec![0f64; frames.len()];
    for t in 0..n_frames {
        for f in 0..N_NOTES {
            let cur = f64::from(frames[t * N_NOTES + f]);
            let d = (1..=N_DIFF)
                .map(|n| {
                    cur - if t >= n {
                        f64::from(frames[(t - n) * N_NOTES + f])
                    } else {
                        0.0
                    }
                })
                .fold(f64::INFINITY, f64::min);
            diff[t * N_NOTES + f] = if t < N_DIFF { 0.0 } else { d.max(0.0) };
        }
    }
    let max_onset = onsets.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let max_diff = diff.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    onsets
        .iter()
        .zip(&diff)
        .map(|(&o, &d)| {
            // numpy divides by zero here and gets NaN; with no frame increase at all, keep the predicted onsets.
            let scaled = if max_diff > 0.0 {
                f64::from(max_onset) * d / max_diff
            } else {
                0.0
            };
            f64::from(o).max(scaled)
        })
        .collect()
}

/// `model_frames_to_time`: start time in seconds of each output frame, corrected for the window overlap.
pub fn frames_to_time(n_frames: usize) -> Vec<f64> {
    let hop_s = FFT_HOP as f64 / AUDIO_SAMPLE_RATE as f64;
    let window_offset =
        hop_s * (ANNOT_N_FRAMES as f64 - AUDIO_N_SAMPLES as f64 / FFT_HOP as f64) + 0.0018;
    (0..n_frames)
        .map(|n| n as f64 * hop_s - window_offset * (n / ANNOT_N_FRAMES) as f64)
        .collect()
}

#[cfg(test)]
mod tests;
