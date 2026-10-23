# Score64

**Turn music and sound into game-ready Commodore 64 SID assets.**

Start from a recording, an AI-generated track, a MIDI file or a reference sound effect. Score64 transcribes it, reduces the arrangement to the SID's three voices, builds adaptive layers for in-game music, matches sound effects to SID parameters, and packs everything into player-ready files for GoatTracker-based games. Then you **listen, compare and approve** in Score64 Studio, with source and SID renders on one playhead.

> Score64 is a **review-and-decide tool, not a tracker.** It makes the arrangement decisions visible and overridable (which part goes to which voice, which chord becomes which arpeggio, where the loop sits), and it checks size, timing and player constraints. Detailed note editing stays in GoatTracker or SID Factory II; hand-edited songs come back in as new versions.

**Status:** pre-alpha, planning complete, scaffolding in progress. Nothing runs yet. See [docs/PDD.md](docs/PDD.md) and [docs/TAD.md](docs/TAD.md).

Sister tool of [Convert64](https://github.com/lumi-living/convert64), which does the same job for pictures.

---

## Why

Getting music into a C64 game is mostly mechanical work that nobody enjoys: transcribing parts, squeezing chords into arpeggios, deciding what to drop when there are only three voices, fitting the result into 4 KB, keeping a voice free for sound effects, and checking it all on both SID revisions. Existing converters stop at "MIDI into a tracker" and leave the rest to hand work.

Score64 automates the mechanical part and keeps every judgement call visible:

- "the pad's chords become a minor arpeggio on voice 2's weak beats; the lead stays on voice 1",
- "voice 3 carries only drums here, so sound effects can borrow it without breaking the tune",
- "this version is 0.4 KB over budget and the melody similarity dropped; the previous one is approved".

## How it works (short version)

```
audio / MIDI ──► stems (optional) ──► notes (transcription) ──► beat grid
                                                                  │
 player-ready .prg/.sid ◄── pack ◄── render & score ◄── instruments ◄── reduce to SID voices
                                         │
                       Score64 Studio: listen · map · pipeline · history · approve
```

Every stage reads files and writes files, cached by a hash of its inputs, so changing one decision re-runs only what depends on it, and paid transcription or LLM calls happen once per unique input. Details: [docs/TAD.md](docs/TAD.md).

## Outputs

| Phase | Outputs |
|---|---|
| 1 | GoatTracker 2 `.sng` (editable), packed `.prg` / `.sid` via gt2reloc with sound-effect support, WAV renders (6581 and 8580), HTML report |
| 2 | GoatTracker sound-effect data in priority order, matched from reference recordings; Score64 Studio |
| 3 | Adaptive multi-subtune sets with bar-synchronised switching, live audition, C64 Ultimate push, DAWproject export |

## Architecture

Pure Rust, one cargo workspace (native: macOS, Windows, Linux):

- `crates/score64-core` — no I/O: tracker-grid score model, voice profiles, reduction rules, validator.
- `crates/score64-formats` — byte-exact GoatTracker song, instrument and sound-effect files; MIDI and WAV.
- `crates/score64-project` — track files, decisions, approvals, `audio.lock`, content-addressed cache.
- `crates/score64-analysis` — spectral features, onsets, fidelity metrics.
- `crates/score64-adapters` — transcription providers, beat tracking, decision makers, packer, VICE, C64 Ultimate.
- `crates/score64-engine` — the command engine: every user-visible action as a command, shared by the app, CLI and MCP server; owns the emulator helper.
- `crates/score64-cli` — `score64` binary for Makefiles and batch work.
- `crates/score64-mcp` — MCP server exposing the same commands to AI agents.
- `crates/score64-app` — Score64 Studio (egui/eframe).
- `crates/score64-emu` — **GPL-3.0-or-later** helper binary: 6502 + SID emulation for rendering and live audition. Kept separate so everything else stays MIT ([ADR-002](docs/adr/ADR-002-mit-with-gpl-emulator-helper.md)).
- `crates/residfp-sys` — **GPL-2.0-or-later**: reSIDfp (libresidfp, vendored) behind a C shim, the helper's SID engine ([ADR-011](docs/adr/ADR-011-residfp-sid-engine.md)). Only `score64-emu` uses it.
- `xtask/` — maintenance tasks (`cargo xtask …`).

## Roadmap

See [docs/PDD.md §Roadmap](docs/PDD.md#roadmap) and the [epics](docs/github/EPICS.md) / GitHub issues. Milestones: **M0 Scaffold → M1 Pipeline MVP → M2 Studio & sound effects → M3 Adaptive & integrations**.

## Building

```sh
cargo run -p score64-app                 # Score64 Studio
cargo build --release -p score64-cli     # the score64 CLI
cargo build --release -p score64-emu     # the emulator helper the CLI and app start
```

Release archives come from dist, one per program: `score64-cli`, `score64-app` and `score64-emu`. Extract them into one directory: `score64` and Score64 Studio look for `score64-emu` beside themselves, then on `PATH`, unless `SCORE64_EMU` names it ([docs/TOOLING.md](docs/TOOLING.md#releases)).

## Licence

MIT for the workspace ([LICENSE](LICENSE)), except `crates/score64-emu`, which is GPL-3.0-or-later ([crates/score64-emu/COPYING](crates/score64-emu/COPYING)), and `crates/residfp-sys`, GPL-2.0-or-later, because accurate SID emulation is GPL-licensed. Third-party notices: [THIRD_PARTY.md](THIRD_PARTY.md).
