// Rust translation of parts of Basic Pitch (https://github.com/spotify/basic-pitch, v0.4.0):
// Copyright 2022 Spotify AB, licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0). Changed: translated from Python to Rust; pitch bends left out.
// See THIRD_PARTY.md.

//! Running `nmp.onnx`: a port of `basic_pitch.inference.run_inference` (windowing, model call, unwrapping).

use std::path::Path;

use ort::environment::Environment;
use ort::session::Session;
use ort::session::builder::AutoDevicePolicy;
use ort::value::Tensor;

use super::{ANNOTATIONS_FPS, AUDIO_N_SAMPLES, AUDIO_SAMPLE_RATE, FFT_HOP, N_CONTOURS, N_NOTES};

/// Overlap between consecutive windows, in output frames.
const N_OVERLAPPING_FRAMES: usize = 30;
const OVERLAP_LEN: usize = N_OVERLAPPING_FRAMES * FFT_HOP;
const HOP_SIZE: usize = AUDIO_N_SAMPLES - OVERLAP_LEN;

const INPUT: &str = "serving_default_input_2:0";
const OUTPUT_NOTE: &str = "StatefulPartitionedCall:1";
const OUTPUT_ONSET: &str = "StatefulPartitionedCall:2";
const OUTPUT_CONTOUR: &str = "StatefulPartitionedCall:0";

/// Model activations for a whole input, row-major.
#[derive(Clone, Debug, Default)]
pub struct ModelOutput {
    /// Number of frames.
    pub frames: usize,
    /// Note activations, `(frames, 88)`.
    pub note: Vec<f32>,
    /// Onset activations, `(frames, 88)`.
    pub onset: Vec<f32>,
    /// Pitch contour, `(frames, 264)`.
    pub contour: Vec<f32>,
}

/// A loaded Basic Pitch model.
pub struct BasicPitch {
    session: Session,
}

impl BasicPitch {
    /// Load `nmp.onnx` in `env` (create one with `ort::environment::init().build()`; share it between sessions).
    ///
    /// The model runs on the CPU. From `ort` 2.0.0-rc.14 a new session picks its device by ONNX Runtime's "max
    /// efficiency" policy, which prefers an NPU: on Apple Silicon that is the Neural Engine, at reduced precision, with
    /// activations up to 0.16 away from the Python package and different notes. Registering the CPU execution provider
    /// does not override that policy; `PreferCPU` does, and then the output matches the Python package bit for bit.
    pub fn load(env: &Environment, model: &Path) -> ort::Result<Self> {
        let session = Session::builder(env)?
            .with_auto_device(AutoDevicePolicy::PreferCPU)
            .map_err(ort::Error::<()>::from)?
            .commit_from_file(model)?;
        Ok(Self { session })
    }

    /// Run the model over mono 22,050 Hz audio.
    pub fn infer(&mut self, audio: &[f32]) -> ort::Result<ModelOutput> {
        // Half an overlap of silence in front, then windows of AUDIO_N_SAMPLES every HOP_SIZE, zero-padded at the end.
        let mut padded = vec![0f32; OVERLAP_LEN / 2];
        padded.extend_from_slice(audio);

        let n_olap = N_OVERLAPPING_FRAMES / 2;
        let mut out = ModelOutput::default();
        let mut start = 0;
        while start < padded.len() {
            let end = (start + AUDIO_N_SAMPLES).min(padded.len());
            let mut window = padded[start..end].to_vec();
            window.resize(AUDIO_N_SAMPLES, 0.0);
            let input = Tensor::from_array(([1usize, AUDIO_N_SAMPLES, 1], window))?;
            let outputs = self.session.run(ort::inputs![INPUT => input])?;
            for (name, bins, dst) in [
                (OUTPUT_NOTE, N_NOTES, &mut out.note),
                (OUTPUT_ONSET, N_NOTES, &mut out.onset),
                (OUTPUT_CONTOUR, N_CONTOURS, &mut out.contour),
            ] {
                let (shape, data) = outputs[name].try_extract_tensor::<f32>()?;
                let frames = shape[1] as usize;
                // Drop half the overlap at both ends of every window.
                dst.extend_from_slice(&data[n_olap * bins..(frames - n_olap) * bins]);
            }
            start += HOP_SIZE;
        }

        // Trim to the length of the original audio.
        out.frames = audio.len() * ANNOTATIONS_FPS / AUDIO_SAMPLE_RATE;
        out.note.truncate(out.frames * N_NOTES);
        out.onset.truncate(out.frames * N_NOTES);
        out.contour.truncate(out.frames * N_CONTOURS);
        Ok(out)
    }
}
