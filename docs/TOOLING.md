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

### Packing and reference players (E3)

Versions used by the #20 spike ([report](research/2026-10-18-packed-player-spike.md)); macOS with Homebrew shown.

- **gt2reloc**: build it from the GoatTracker 2.77 source (SourceForge `goattracker2`). It links SDL 1.2: `brew install sdl12-compat`, then in `src/` run `CFLAGS="-I$(brew --prefix)/include" make ../linux/gt2reloc ../linux/ins2snd2` and copy both binaries onto your `PATH`.
- **sidplayfp** 3.2.2: Homebrew has only libsidplayfp, and its bottle has no reSIDfp. Build [libresidfp](https://github.com/libsidplayfp/libresidfp) 1.2.3, then [libsidplayfp](https://github.com/libsidplayfp/libsidplayfp) 3.1.1, then [sidplayfp](https://github.com/libsidplayfp/sidplayfp) 3.2.2, each with `./configure --prefix=$HOME/.local && make install` and `PKG_CONFIG_PATH=$HOME/.local/lib/pkgconfig` (`brew install pkgconf fmt` first).
- **VICE** 3.10: `brew install vice` (gives `x64sc` and `vsid`). `-sounddev dump` logs SID writes in any mode. `-sounddev wav` only writes audio in real time (not `-warp`) and only on a clean exit, for example `quit` through `-remotemonitor`.
- **KickAssembler** 5.25 (Java) for test drivers such as `crates/score64-emu/spikes/packed-player/sfx-driver.asm`: download from <http://theweb.dk/KickAssembler/> and run `java -jar KickAss.jar`.
- **uv**, for the spike's analysis script (numpy and scipy are pulled per run).

## Reference material

Everything the code must agree with is in [`docs/reference/`](reference/README.md). Do not rely on memory for register layouts, player addresses, file formats or budgets — read the file.

## AI-assisted development

This project is developed with AI coding agents alongside humans. If you use one:

- Point it at `CLAUDE.md` (the non-negotiables) and `docs/reference/` (the facts). The file name follows the Claude Code convention but the content is agent-agnostic.
- Have it check egui/eframe, rmcp and ort APIs against current docs rather than training data.
- Treat its output like anyone else's: tests, differential checks for ports, and a VICE or sidplayfp listen before merge.

No private prompts, skills or MCP servers are required to work on Score64.
