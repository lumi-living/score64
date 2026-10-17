# ADR-001: Rust workspace with an egui/eframe app, aligned with Convert64

**Status:** Accepted
**Date:** 2026-10-18
**Deciders:** @jahaa

## Context

Score64 needs a pipeline (transcription, reduction, packing, rendering), a CLI, an MCP server and a desktop app for listening and comparing. A first sketch used Python for the pipeline and a React web UI. Convert64, the family's image tool, is one Rust workspace with an egui/eframe app.

## Decision

One Rust workspace (edition 2024, toolchain pinned like Convert64) with crates for core, formats, project, analysis, adapters, CLI, MCP and app. The app uses egui/eframe, native only (no web build planned: the tool needs local files, external programs and a native audio thread).

## Options considered

### Option A: Python pipeline + FastAPI + React app
| Dimension | Assessment |
|---|---|
| Complexity | Three languages; HTTP layer between UI and core |
| Performance | Fine for the pipeline; browser audio sync and in-browser SID emulation are approximate |
| Maintenance | Rich ML/audio ecosystem; two stacks to keep current |
| Fit | Breaks family consistency with Convert64 |

### Option B: Rust + egui (chosen)
| Dimension | Assessment |
|---|---|
| Complexity | One language; app calls the core in-process |
| Performance | Native audio thread; the real packed player runs in an emulator for accurate live audition |
| Maintenance | Some ports needed (GoatTracker writer, feature extraction, Basic Pitch post-processing) |
| Fit | Shares theme, icons, conventions and tooling with Convert64 |

## Trade-off analysis

Total effort is similar. Rust wins on consistency, accurate audition and single-binary distribution; Python wins on quick experimentation and ML breadth. PyTorch-only models are handled by an optional sidecar (ADR-009).

## Consequences
- Easier: one codebase for agents and humans; shared crates with Convert64 later; native playback.
- Harder: reduction heuristics iterate slower than in a notebook (mitigated by rules-as-data and snapshot tests).
- Revisit when: a required model has neither ONNX export nor an acceptable hosted alternative.
