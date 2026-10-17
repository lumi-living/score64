# Tooling

## Toolchain

| Need | Tool |
|---|---|
| Rust | stable, pinned in `rust-toolchain.toml`; `cargo fmt`, `cargo clippy -D warnings`, `cargo test` |
| UI | egui via eframe (`cargo run -p score64-app`) |
| MCP | `rmcp` (official Rust MCP SDK), stdio transport |
| Packing | `gt2reloc` from GoatTracker 2 (external program, pinned version documented in `docs/reference/player.md`) |
| Emulation | `score64-emu` helper (GPL), plus VICE `x64sc` and sidplayfp for verification |
| Local ML (optional) | ONNX Runtime via `ort` for Basic Pitch and ONNX separation models; optional Python sidecar via `uv` for PyTorch-only models (ADR-009) |
| CI | `ci.yml`: jobs `rust` (fmt, clippy, test on ubuntu/macos/windows) and `licence-boundary` |
| Snapshots | `insta` (`cargo insta review`) |
| Verification | VICE `x64sc` or sidplayfp on 6581 and 8580 models for every packed file; a C64 Ultimate or real hardware when available |

## Setup

1. Install `rustup` from <https://rustup.rs>. The first `cargo` command in the repo reads `rust-toolchain.toml` and installs the pinned compiler with `rustfmt` and `clippy`.
2. Check everything works:

```sh
cargo test --workspace
cargo run -p score64-cli -- --version
```

External tools (gt2reloc, VICE, sidplayfp, the optional sidecar) are only needed for the stages that use them; setup notes arrive with the epics that introduce them.

## Reference material

Everything the code must agree with is in [`docs/reference/`](reference/README.md). Do not rely on memory for register layouts, player addresses, file formats or budgets — read the file.

## AI-assisted development

This project is developed with AI coding agents alongside humans. If you use one:

- Point it at `CLAUDE.md` (the non-negotiables) and `docs/reference/` (the facts). The file name follows the Claude Code convention but the content is agent-agnostic.
- Have it check egui/eframe, rmcp and ort APIs against current docs rather than training data.
- Treat its output like anyone else's: tests, differential checks for ports, and a VICE or sidplayfp listen before merge.

No private prompts, skills or MCP servers are required to work on Score64.
