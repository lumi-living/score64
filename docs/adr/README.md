# Architecture Decision Records

One file per decision, `ADR-NNN-short-title.md`, copied from `ADR-000-template.md`. Status flows Proposed → Accepted → (Deprecated | Superseded by ADR-NNN). Never edit an accepted ADR's decision; write a new one that supersedes it.

| ADR | Title | Status |
|---|---|---|
| [000](ADR-000-template.md) | Template for new ADRs | — |
| [001](ADR-001-rust-workspace-egui.md) | Rust workspace with an egui/eframe app, aligned with Convert64 | Accepted |
| [002](ADR-002-mit-with-gpl-emulator-helper.md) | MIT, with the SID emulator in a separate GPL helper binary | Accepted |
| [003](ADR-003-review-tool-not-tracker.md) | A review-and-decide tool, not a tracker | Accepted |
| [004](ADR-004-files-are-the-source-of-truth.md) | Stage pipeline with a content-addressed cache; files are the source of truth | Accepted |
| [005](ADR-005-one-command-engine.md) | One command engine behind app, CLI and MCP | Accepted |
| [006](ADR-006-tracker-grid-score-model.md) | A tracker-grid score model as the core representation | Accepted |
| [007](ADR-007-goattracker-first.md) | GoatTracker 2 as the first output target | Accepted |
| [008](ADR-008-llm-decisions-are-advisory.md) | LLM decisions are advisory, structured and overridable | Accepted |
| [009](ADR-009-optional-python-sidecar.md) | An optional Python sidecar for PyTorch-only models and test oracles | Accepted |
| [010](ADR-010-6502-core.md) | `emulator_6502` as the 6502 core of score64-emu, behind a CPU trait | Accepted |
| [011](ADR-011-residfp-sid-engine.md) | reSIDfp (libresidfp) via FFI as the default SID engine | Accepted |
| [012](ADR-012-command-engine-crate.md) | The command engine is its own crate and owns the emulator helper | Accepted |
| [013](ADR-013-command-outcomes-and-exit-codes.md) | Commands decide failure; CLI exit codes; a provisional validation context file | Accepted |
| [014](ADR-014-project-and-track-files.md) | Project and track files: `score64.toml` and `track.toml` | Proposed |
