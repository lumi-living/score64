# ADR-009: An optional Python sidecar for PyTorch-only models and test oracles

**Status:** Accepted
**Date:** 2026-10-18
**Deciders:** @jahaa

## Context

Some useful models (certain separation and transcription models) exist only as PyTorch checkpoints, and some ports are best verified against their Python originals.

## Decision

`sidecar/` holds optional Python scripts managed with `uv`. Contract: paths in as arguments, files plus a `manifest.json` out; no shared memory, no long-lived service. The default pipeline (hosted transcription, ONNX models, Rust analysis) needs no Python. Test-only oracles (for example pygoattracker) run through the same mechanism and are skipped when the sidecar is not installed.

## Consequences
- Easier: access to PyTorch-only models; honest differential tests.
- Harder: a second runtime for users who opt in.
- Revisit when: the models we rely on gain ONNX exports.
