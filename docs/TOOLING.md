# Tooling

## Toolchain

| Need | Tool |
|---|---|
| Rust | stable, pinned in `rust-toolchain.toml`; `cargo fmt`, `cargo clippy -D warnings`, `cargo test` |
| UI | egui via eframe (`cargo run -p score64-app`) |
| MCP | `rmcp` (official Rust MCP SDK), stdio transport |
| Packing | `gt2reloc` from GoatTracker 2 (external program, pinned version documented in `docs/reference/player.md`) |
| Emulation | `score64-emu` helper (GPL) with reSIDfp vendored in `residfp-sys`, which needs a C++17 compiler (Xcode command-line tools, GCC/Clang, or MSVC; `cc` finds it). VICE `x64sc` and sidplayfp for verification |
| Local ML (optional) | ONNX Runtime via `ort` (pinned `=2.0.0-rc.14`, bumped by hand because the API still changes between release candidates; sessions are pinned to the CPU with `AutoDevicePolicy::PreferCPU`, because rc.14's default picks the Apple Neural Engine at reduced precision; the first build downloads a static ONNX Runtime 1.31, or set `ORT_LIB_LOCATION` for offline builds) for Basic Pitch and ONNX separation models; optional Python sidecar via `uv` for PyTorch-only models and test oracles (ADR-009) |
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
  - **Finding and pinning it.** Score64 finds it through `SCORE64_GT2RELOC`, or else on `PATH`. It has no version flag, so the pinned version is the one its banner names. The binary built this way prints `GoatTracker v2.73 Packer/Relocator` (observed 2026-10-23), so the pin is `2.73`. The banner can't tell this build from a real 2.73, so the adapter can also pin the program itself by its blake3 (`Gt2Reloc::with_blake3`; get the value with `b3sum`, or from any pack's `program_blake3`, which every pack records for provenance). A binary hash differs per platform and per build, so a project doesn't keep one: ADR-014 pins `[tools] gt2reloc = { version, reference }` in `score64.toml`, where `reference` is the blake3 of packing Score64's reference fixture with the reference options. That hash is the same on every platform for the same packer, and the pack stage (E7) caches it per binary hash in `.score64/tools/gt2reloc/`. A program is hashed and then run, so a swap between the two isn't caught, and a wrapper script is pinned rather than the program it starts.
  - **Quirks the adapter guards against** (observed with that binary):
    - A missing song, or a file that isn't a song, packs an empty tune and exits 0. Some junk bytes make it crash instead.
    - A malformed option is ignored, and the default is used.
    - The format comes from the output's extension, case-sensitively.
    - A song path over 59 bytes, or an output path over 255, crashes it.
- **sidplayfp** 3.2.2: Homebrew has only libsidplayfp, and its bottle has no reSIDfp. Build [libresidfp](https://github.com/libsidplayfp/libresidfp) 1.2.3, then [libsidplayfp](https://github.com/libsidplayfp/libsidplayfp) 3.1.1, then [sidplayfp](https://github.com/libsidplayfp/sidplayfp) 3.2.2, each with `./configure --prefix=$HOME/.local && make install` and `PKG_CONFIG_PATH=$HOME/.local/lib/pkgconfig` (`brew install pkgconf fmt` first).
- **VICE** 3.10: `brew install vice` (gives `x64sc` and `vsid`). `-sounddev dump` logs SID writes in any mode. `-sounddev wav` only writes audio in real time (not `-warp`) and only on a clean exit, for example `quit` through `-remotemonitor`.
- **KickAssembler** 5.25 (Java) for test drivers such as `crates/score64-emu/spikes/packed-player/sfx-driver.asm`: download from <http://theweb.dk/KickAssembler/> and run `java -jar KickAss.jar`.
- **uv** 0.12 runs the Python sidecar (`sidecar/`): one locked project used for differential tests, fixture generation and the spike analysis scripts. Run scripts with `uv run --frozen --project sidecar python ...`; see `sidecar/README.md`.

### Listening sets for `needs-vice-check` PRs

A PR that changes what the packed player plays (labelled `needs-vice-check`) gets a listening set before it merges:

```
cargo xtask listen --pr <n> [case ...]
cargo xtask listen --pr <n> --score <song.toml|json> --instruments <folder of <preset id>.ins>
```

It writes to `scratch/<n>-listen/`, which git ignores. For each case it:
- exports the score through `score64_formats::gt2::export`;
- packs it with the pinned `gt2reloc`, with sound-effect support (`.sfx`, `-D1`) and without (`.nosfx`);
- wraps the `.sfx` file so it fires the fixture's three effects on voice 3 every 37 frames (`.sfx+fx`). A plain `.sfx` file plays no effects;
- renders every subtune for 60 s with the emulator helper and with sidplayfp, on the 6581 and the 8580;
- writes `LISTEN.md`.

LISTEN.md has:
- what to listen for, and a timeline per subtune: when each tie, key-off, arpeggio, tempo change and loop comes;
- measurements: helper vs sidplayfp, `.sfx` vs `.nosfx`, and the effects' level against the music.

The built-in cases (all of them by default) are `sixteen-steps`, `sixteen-steps-adaptive`, `combat-filtered`, `ties-keyoffs-arp` and `loop-tempo` (`xtask/src/listen/fixture.rs`). It needs `gt2reloc` and builds the helper itself. sidplayfp is used if it's on `PATH`. `--gt2reloc-blake3` pins a specific binary; without it the installed one is pinned and its hash recorded.

## Releases

Release archives are built by [dist](https://github.com/axodotdev/cargo-dist) 0.33.0, configured in `dist-workspace.toml` (as in Convert64). It makes one archive per program, for macOS (arm64 and x86_64), Windows and Linux:

| Archive | Contains |
|---|---|
| `score64-cli-<target>` | `score64`, `LICENSE` (MIT), `README.md` |
| `score64-app-<target>` | `score64-app` (Score64 Studio), `LICENSE`, `README.md` |
| `score64-emu-<target>` | `score64-emu`, its GPL-3.0-or-later `LICENSE`, `THIRD_PARTY.md` (vendored reSIDfp), `README.md` |

dist builds one archive per package and can't combine packages. The helper must stay its own GPL package (ADR-002), so it ships separately. **Extract all the archives into one directory.** `score64` and `score64-app` look for `score64-emu` in this order (`Helper::locate`): `SCORE64_EMU`, then beside themselves, then on `PATH`. `score64-mcp` joins the releases when it has tools (E14).

- **Real releases:** `release.yml`, generated by `dist generate`, runs only on version tags. Never edit it by hand; change `dist-workspace.toml` and run `dist generate` again.
- **Test builds:** the manual `dist build (manual)` workflow (`dist-build.yml`, `workflow_dispatch`) builds every target and uploads workflow artifacts only. It makes no tag and no release.
- **Locally:** `dist build --artifacts=local --target=aarch64-apple-darwin` writes to `target/distrib/`.

The archives are unsigned and not notarised. Signed bundles, installers and licence notices inside the app are E17 (#109, #143).

## Reference material

Everything the code must agree with is in [`docs/reference/`](reference/README.md). Do not rely on memory for register layouts, player addresses, file formats or budgets — read the file.

## AI-assisted development

This project is developed with AI coding agents alongside humans. If you use one:

- Point it at `CLAUDE.md` (the non-negotiables) and `docs/reference/` (the facts). The file name follows the Claude Code convention but the content is agent-agnostic.
- Have it check egui/eframe, rmcp and ort APIs against current docs rather than training data.
- Treat its output like anyone else's: tests, differential checks for ports, and a VICE or sidplayfp listen before merge.

No private prompts, skills or MCP servers are required to work on Score64.
