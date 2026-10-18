# Basic Pitch differential fixtures

Written by `sidecar/basic_pitch_reference.py <stem.wav> <out> --fixture <here>/<stem>` (basic-pitch 0.4.0, onnxruntime
1.31.0, `nmp.onnx` SHA-256 `2c3c1d14…59a0ec`). Each directory holds the package's note and onset activations for
the stem as little-endian float16 (`(frames, 88)`, row-major) and `notes.json`, the notes the package's own
`output_to_notes_polyphonic` makes from exactly those float16 values with `predict()`'s defaults. Used by
`src/basic_pitch/notes/tests.rs`.

| Stem | Source | Licence |
|---|---|---|
| `vocadito_10` | `tests/resources/vocadito_10.wav` in basic-pitch, from the vocadito dataset: R. Bittner, K. Pasalo, J. J. Bosch et al., "vocadito: A dataset of solo vocals with f0, note, and lyric annotations", <https://zenodo.org/record/5578807>. Only model activations derived from the audio are stored here, not the audio | CC BY 4.0 |
| `synth_poly` | `sidecar/make_test_stem.py`: synthetic bass, chords and melody (12 s) | MIT (this repository) |
