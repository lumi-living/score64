# Python sidecar (optional)

Optional scripts for PyTorch-only models and test-only oracles (ADR-009), managed with `uv`. Contract: input paths as arguments; output files plus a `manifest.json`. The default pipeline does not need Python.

Each script declares its own pinned dependencies inline (PEP 723), so `uv run sidecar/<script>.py …` is all it takes. The full bootstrap with a Rust runner is #23.

| Script | Use |
|---|---|
| `basic_pitch_reference.py` | Basic Pitch 0.4.0 (ONNX) reference output for the Rust port's differential tests (#21, #34) |
| `make_test_stem.py` | A synthetic polyphonic test stem with known notes (#21) |
