# Python sidecar (optional)

Python for the jobs Rust shouldn't do itself (ADR-009): reference outputs from original Python packages for
differential tests, PyTorch-only models later, and fixture generation. The default pipeline never needs it.

## Setup

One [uv](https://docs.astral.sh/uv/) project with a committed `uv.lock` and Python 3.11 (`.python-version`). Direct
dependencies are pinned exactly in `pyproject.toml`, and the lock pins everything else. Always run with `--frozen`, so
the lock is used as committed and never re-resolved:

```sh
uv run --frozen --project sidecar python sidecar/<script>.py ...
```

The first run downloads Python and the packages (about 380 MB). basic-pitch's TensorFlow, TFLite and CoreML backends
are dropped by `override-dependencies`: only the ONNX model is used, and the environment is the same on every OS.

To change a dependency: edit `pyproject.toml`, run `uv lock --project sidecar`, commit both, then regenerate any testdata
that depends on it and say so in the PR.

## Contract

A script takes its input paths as arguments and writes files plus `manifest.json` (schema 1, written by `manifest.py`)
to its output directory:

```json
{"schema": 1, "tool": "basic_pitch_reference", "tool_version": 1, "python": "3.11.15",
 "packages": {"basic-pitch": "0.4.0", ...}, "params": {...},
 "inputs":  [{"path": "...", "bytes": 802472, "blake3": "..."}],
 "outputs": [{"path": "note.f32", "bytes": 275264, "blake3": "..."}]}
```

blake3 is the content-addressed cache's hash (ADR-004), so results can go straight into the cache.
`score64_adapters::sidecar` runs a script and validates its manifest (schema, tool, every output's size and blake3)
before anything reads an output. It starts `uv` with an argument list, never a shell, with a timeout, keeps stderr for
errors, and kills the process if it overruns.

## Scripts

| Script | Use |
|---|---|
| `basic_pitch_reference.py` | Basic Pitch 0.4.0 (ONNX) reference: activations, notes and the model itself, for the Rust port's differential test (#21, #34). `--fixture` writes the float16 testdata in `crates/score64-adapters/testdata/basic-pitch` |
| `make_test_stem.py` | A synthetic polyphonic stem with known notes (#21) |

`fixtures/_tools/make_audio.py` and the spike scripts under `crates/score64-emu/spikes/` run in the same environment.

## Tests

Sidecar tests are opt-in: `SCORE64_SIDECAR=1 cargo test -p score64-adapters --test basic_pitch_onnx`. Without the
variable they print `SKIPPED ...` and pass. With it they fail if `uv` or the project is missing, so they can't pass by
skipping. CI runs them in the `sidecar (ubuntu-latest)` job, and the other jobs show them as skipped.
