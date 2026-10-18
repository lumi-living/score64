//! Basic Pitch (Spotify, Apache-2.0) through ONNX Runtime: the official `nmp.onnx` model plus a port of the
//! package's windowing and note creation (E3 spike #21; the full adapter is #34).
//!
//! Input is mono audio at 22,050 Hz. Resampling to that rate belongs to the audio decoder (#32).

mod model;
pub mod notes;

pub use model::{BasicPitch, ModelOutput};
pub use notes::{NoteEvent, NoteParams, frames_to_time, output_to_notes};

/// Sample rate the model expects.
pub const AUDIO_SAMPLE_RATE: usize = 22_050;
/// Model frame hop in samples.
pub const FFT_HOP: usize = 256;
/// Model frames per second (integer division, as in the original).
pub const ANNOTATIONS_FPS: usize = AUDIO_SAMPLE_RATE / FFT_HOP;
/// Output frames per model window.
pub const ANNOT_N_FRAMES: usize = ANNOTATIONS_FPS * 2;
/// Samples per model window.
pub const AUDIO_N_SAMPLES: usize = AUDIO_SAMPLE_RATE * 2 - FFT_HOP;
/// Note bins (88 piano keys from A0).
pub const N_NOTES: usize = 88;
/// Contour bins (three per semitone).
pub const N_CONTOURS: usize = 264;
