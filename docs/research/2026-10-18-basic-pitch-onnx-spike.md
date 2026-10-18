# Spike: Basic Pitch through ONNX Runtime (`ort`) against the Python package

**Issue:** #21 (part of E3, #17) · **Date:** 2026-10-18 · **Verdict: GO**

## Question

Can Score64 run Spotify's official Basic Pitch model (`nmp.onnx`) from Rust through `ort`, with the frames-to-notes post-processing ported, and get the same notes as the Python package?

## Answer

**Go.** On both test stems the Rust pipeline gives *exactly* the Python package's note list (same start and end frames, pitches and amplitudes, in the same order). Model activations differ from Python's by at most 1.2 × 10⁻⁷, which is float rounding between two ONNX Runtime versions. Inference runs at about 250× real time on one CPU thread. ONNX Runtime links statically and needs nothing installed.

## Setup

| Item | Version |
|---|---|
| Model | `nmp.onnx` from the basic-pitch 0.4.0 wheel, SHA-256 `2c3c1d144bfa61ad236e92e169c13535c880469a12a047d4e73451f2c059a0ec` |
| Rust | `ort` `=2.0.0-rc.13` (bundles ONNX Runtime 1.28, downloaded as a static library at build time by `ort-sys`) |
| Python reference | basic-pitch 0.4.0 with onnxruntime 1.31.0, numpy 2.4.6, librosa 0.11.0, run by `sidecar/basic_pitch_reference.py` through `uv` (ADR-009) |
| Note settings | `predict()`'s defaults: onset threshold 0.5, frame threshold 0.3, minimum length 11 frames (127.7 ms), inferred onsets, melodia trick |

Test stems:

| Stem | What | Length |
|---|---|---|
| `vocadito_10` | Solo singing from the vocadito dataset (CC BY 4.0), shipped in basic-pitch's own tests | 9.1 s |
| `synth_poly` | Synthetic bass, three-note chords and melody with a plucked additive tone; known notes (`sidecar/make_test_stem.py`) | 12.0 s |

Both sides get identical input: the 22,050 Hz mono samples as librosa loads them. Resampling in Rust is #32's job and is not part of this comparison.

## Results

| Stem | Frames | Max \|Δ\| note / onset / contour | Notes, Rust / Python | Identical | Inference (Rust, release) |
|---|---|---|---|---|---|
| `vocadito_10` | 782 | 8.9e-8 / 8.9e-8 / 8.9e-8 | 28 / 28 | **28** | 39 ms |
| `synth_poly` | 1,032 | 1.2e-7 / 8.9e-8 / 8.9e-8 | 98 / 98 | **98** | 46 ms |

Python's `predict()` gives the same 98 notes as calling `output_to_notes_polyphonic` directly, so the reference is what users of the package actually get.

## What was ported

- `inference.run_inference`: half an overlap of silence in front, 43,844-sample windows every 36,164 samples with zero padding at the end, 15 frames dropped from each side of every window, then trimmed to the input length (`crates/score64-adapters/src/basic_pitch/model.rs`).
- `note_creation.output_to_notes_polyphonic` and `model_frames_to_time`: frequency limits, inferred onsets, strict local maxima along time (as `scipy.signal.argrelmax`), the backwards onset walk, and the melodia trick with numpy's first-maximum `argmax` order (`notes.rs`). Precision follows numpy's promotions (`f32` activations, `f64` energies), which is why the result is exact, not just close.
- **Not ported:** pitch bends; the tracker grid has no use for them yet. The NaN numpy produces when no frame activation ever rises, in inferred onsets: the port keeps the predicted onsets there instead.

## Tests

- **In CI, no model needed:** `basic_pitch::notes::tests` runs the port on committed fixtures, the package's activations rounded to float16 (650 KB for both stems), and compares with the notes the package itself makes from those rounded values. Both pass exactly.
- **Ignored by default, needs the model:** `tests/basic_pitch_onnx.rs` runs `nmp.onnx` through `ort` and compares activations (≤ 1e-3) and notes (exact) with a sidecar reference directory. The run is in the README at the top of the test.

## Accuracy, for context

This doesn't affect go or no-go, but it matters for E5 and E6. Against the synthetic stem's 45 true notes, Basic Pitch finds all of them but adds harmonics and re-triggers:

| | Notes | Precision | Recall | F1 (onset + pitch) | F1 (with offsets) |
|---|---|---|---|---|---|
| Basic Pitch (Python = Rust) | 98 | 0.46 | 1.00 | 0.63 | 0.43 |

28 of the extra notes are pitches not in the score: octaves, fifths and higher harmonics of the plucked tone. The rest are notes split in two. Ghost-note removal and octave folding in the reduction (#40, #41) have to expect this, and so do per-stem threshold choices in #34.

## Risks and notes for #34

- **`ort` is a release candidate** (2.0.0-rc.13; rc.14 is six days old). It's pinned exactly; a bump is caught by the ignored end-to-end test, so run it on bumps.
- **Build-time download:** `ort-sys` fetches the ONNX Runtime static library (about 86 MB cache) during the first build. Offline or locked-down builds need `ORT_LIB_LOCATION` or the `load-dynamic` feature. The default path builds, links and passes the tests on Ubuntu, macOS and Windows (CI for #118).
- **Size:** ONNX Runtime adds about 25 MB to binaries that link `score64-adapters` (macOS release, static, CoreML framework linked from the system). That's relevant for cargo-dist bundles (#109).
- **Model delivery:** `nmp.onnx` is 230 KB, Apache-2.0. Download on first use with the SHA-256 above (#34), or ship it in the bundle, since it is small.
- **Resampling:** the comparison used librosa's resampled input. #32 needs its own differential test, because resampler differences reach the activations directly.

## Reproduce

```sh
uv run --frozen --project sidecar python sidecar/make_test_stem.py scratch/synth_poly.wav scratch/synth_poly_truth.json
uv run --frozen --project sidecar python sidecar/basic_pitch_reference.py scratch/synth_poly.wav scratch/ref_synth_poly
BASIC_PITCH_MODEL=<site-packages>/basic_pitch/saved_models/icassp_2022/nmp.onnx \
BASIC_PITCH_REFERENCE=$PWD/scratch/ref_synth_poly \
  cargo test --release -p score64-adapters --test basic_pitch_onnx -- --ignored --nocapture
```
