# Epics and milestones

Tracking board: **[Score64 project](https://github.com/orgs/lumi-living/projects/4)** (GitHub Projects, board by Status: Backlog → Ready → In progress → In review → Done; `Work type` field: Epic / Task / Bug / Docs).

Each epic is a GitHub issue labelled `epic`; child issues are GitHub sub-issues of the epic and say "Part of #N" in their body. Labels: `epic`, `core`, `formats`, `project`, `analysis`, `adapters`, `emu`, `cli`, `mcp`, `app`, `sfx`, `docs`, `fixtures`, `ci`, `release`, `spike`, `licence`, `needs-vice-check` (packed output verified in VICE or sidplayfp on 6581 and 8580 before merge; `cargo xtask listen --pr <n>` builds the listening set), `needs-hardware-check`, `bug`, `enhancement`, `good first issue`.

| # | Milestone | Epics |
|---|---|---|
| [1](https://github.com/lumi-living/score64/milestone/1) | **M0 Scaffold** | E1 [#1](https://github.com/lumi-living/score64/issues/1) E2 [#10](https://github.com/lumi-living/score64/issues/10) E3 [#17](https://github.com/lumi-living/score64/issues/17) |
| [2](https://github.com/lumi-living/score64/milestone/2) | **M1 Pipeline MVP** | E4 [#24](https://github.com/lumi-living/score64/issues/24) E5 [#31](https://github.com/lumi-living/score64/issues/31) E6 [#38](https://github.com/lumi-living/score64/issues/38) E7 [#46](https://github.com/lumi-living/score64/issues/46) E8 [#53](https://github.com/lumi-living/score64/issues/53) E9 [#60](https://github.com/lumi-living/score64/issues/60) |
| [3](https://github.com/lumi-living/score64/milestone/3) | **M2 Studio & sound effects** | E10 [#66](https://github.com/lumi-living/score64/issues/66) E11 [#73](https://github.com/lumi-living/score64/issues/73) E12 [#79](https://github.com/lumi-living/score64/issues/79) E13 [#85](https://github.com/lumi-living/score64/issues/85) E14 [#91](https://github.com/lumi-living/score64/issues/91) |
| [4](https://github.com/lumi-living/score64/milestone/4) | **M3 Adaptive & integrations** | E15 [#94](https://github.com/lumi-living/score64/issues/94) E16 [#100](https://github.com/lumi-living/score64/issues/100) E17 [#108](https://github.com/lumi-living/score64/issues/108) |

## M0 Scaffold

Workspace, CI, score model and validator, fixtures, and the de-risking spike (emulation, ONNX transcription).

- **E1 Workspace & CI** ([#1](https://github.com/lumi-living/score64/issues/1)) — Cargo workspace with crate skeletons (core, formats, project, analysis, adapters, cli, mcp, app, emu), lints and toolchain pin matching Convert64; CI on macOS/Linux/Windows plus the licence-boundary job; app boots; command engine and emulator-helper skeletons.
  - ✓ Workspace skeleton: crates, lints, toolchain pin [#2](https://github.com/lumi-living/score64/issues/2)
  - ✓ CI: fmt, clippy, test matrix and licence-boundary job [#3](https://github.com/lumi-living/score64/issues/3)
  - ✓ Community files, issue templates, Dependabot [#4](https://github.com/lumi-living/score64/issues/4)
  - ✓ Branch protection on main with the CI contexts [#5](https://github.com/lumi-living/score64/issues/5)
  - ✓ score64-app boots an empty eframe window (light/dark, Material Symbols) [#6](https://github.com/lumi-living/score64/issues/6)
  - ✓ Command engine skeleton shared by CLI, MCP and app (ADR-005) [#7](https://github.com/lumi-living/score64/issues/7)
  - ✓ score64-emu helper skeleton and versioned stdin/stdout protocol [#8](https://github.com/lumi-living/score64/issues/8)
  - ✓ Release tooling: cargo-dist configuration [#9](https://github.com/lumi-living/score64/issues/9)
  - Supply-chain checks with cargo-deny: licences, advisories, bans [#148](https://github.com/lumi-living/score64/issues/148)
- **E2 Score model & validator** ([#10](https://github.com/lumi-living/score64/issues/10)) — The tracker-grid score model (ADR-006), voice profiles, tempo grid and the validator for TAD §1.3, with property tests.
  - ✓ Score model types: Note, VoiceLine, Role, Subtune, Song, Decision [#11](https://github.com/lumi-living/score64/issues/11)
  - ✓ Voice profiles as data: built-in and project-defined, per subtune [#12](https://github.com/lumi-living/score64/issues/12)
  - ✓ Validator for TAD §1.3 rules with per-bar, per-voice errors [#13](https://github.com/lumi-living/score64/issues/13)
  - ✓ Property tests: any reduced song validates [#14](https://github.com/lumi-living/score64/issues/14)
  - ✓ Tempo grid: frames-per-row, BPM snapping with error report [#15](https://github.com/lumi-living/score64/issues/15)
  - ✓ Hard-restart lead-time check [#16](https://github.com/lumi-living/score64/issues/16)
- **E3 Fixtures & de-risking spike** ([#17](https://github.com/lumi-living/score64/issues/17)) — Rights-clear fixtures, golden-test harness, sidecar bootstrap, and a time-boxed spike on the two Rust-specific risks: running packed players in a 6502 core with SID emulation, and Basic Pitch through ONNX.
  - ✓ Rights-clear audio and MIDI fixtures [#18](https://github.com/lumi-living/score64/issues/18)
  - ✓ Spike: choose a 6502 core for score64-emu [#19](https://github.com/lumi-living/score64/issues/19)
  - ✓ Spike: run a gt2reloc-packed tune in 6502 core + resid-rs and compare with sidplayfp [#20](https://github.com/lumi-living/score64/issues/20)
  - ✓ Spike: Basic Pitch ONNX via ort vs the Python package [#21](https://github.com/lumi-living/score64/issues/21)
  - ✓ Golden-test harness for .sng and effect bytes (insta) [#22](https://github.com/lumi-living/score64/issues/22)
  - ✓ Python sidecar bootstrap (uv) with the manifest contract [#23](https://github.com/lumi-living/score64/issues/23)

## M1 Pipeline MVP

CLI end to end: transcribe → grid → reduce → .sng → pack → render → score → HTML report.

- **E4 Project & cache** ([#24](https://github.com/lumi-living/score64/issues/24)) — Files are the source of truth (ADR-004): track and decision files, content-addressed stage cache, immutable versions, rebuildable index.
  - ✓ track.toml and decisions.toml schemas with validation [#25](https://github.com/lumi-living/score64/issues/25)
  - ✓ Content-addressed stage cache with manifests [#26](https://github.com/lumi-living/score64/issues/26)
  - ✓ Immutable versions: identity from intent + inputs + tool version [#27](https://github.com/lumi-living/score64/issues/27)
  - ✓ SQLite index rebuildable from manifests and git [#28](https://github.com/lumi-living/score64/issues/28)
  - ✓ Cache garbage collection policy [#29](https://github.com/lumi-living/score64/issues/29)
  - Provenance and licence flags per artifact [#30](https://github.com/lumi-living/score64/issues/30)
- **E5 Transcription & beat grid** ([#31](https://github.com/lumi-living/score64/issues/31)) — Transcriber and beat-tracker ports with adapters; decode, tempo snapping and quantisation.
  - Audio decode and resampling (symphonia) [#32](https://github.com/lumi-living/score64/issues/32)
  - Transcriber port and hosted multi-instrument adapter [#33](https://github.com/lumi-living/score64/issues/33)
  - Basic Pitch ONNX adapter with differential test [#34](https://github.com/lumi-living/score64/issues/34)
  - Beat This! beat and downbeat grid (ONNX) [#35](https://github.com/lumi-living/score64/issues/35)
  - ✓ Quantise notes to rows on the snapped tempo grid [#36](https://github.com/lumi-living/score64/issues/36)
  - Optional separation adapters (provider stems, ONNX models) [#37](https://github.com/lumi-living/score64/issues/37)
  - ✓ Sidecar runner: kill the whole process tree on timeout, not just uv [#126](https://github.com/lumi-living/score64/issues/126)
  - Target video standard: PAL by default, NTSC as an option, in snap_tempo and project settings [#139](https://github.com/lumi-living/score64/issues/139)
- **E6 Reduction to SID voices** ([#38](https://github.com/lumi-living/score64/issues/38)) — Rules that turn transcribed parts into a valid song for a profile, with every judgement call recorded as a decision.
  - ✓ Lead selection by salience, with override [#39](https://github.com/lumi-living/score64/issues/39)
  - ✓ Monophonise, minimum note length, ghost-note removal [#40](https://github.com/lumi-living/score64/issues/40)
  - ✓ Octave folding into role ranges [#41](https://github.com/lumi-living/score64/issues/41)
  - ✓ Chord detection → GoatTracker wavetable arpeggio tables [#42](https://github.com/lumi-living/score64/issues/42)
  - ✓ Drum folding: dedicated voice or interleaved with bass [#43](https://github.com/lumi-living/score64/issues/43)
  - ✓ Loop finder on downbeats [#44](https://github.com/lumi-living/score64/issues/44)
  - ✓ Rules decider and manual decider (decisions.toml overrides) [#45](https://github.com/lumi-living/score64/issues/45)
- **E7 GoatTracker output & packing** ([#46](https://github.com/lumi-living/score64/issues/46)) — Byte-exact GoatTracker 2 song and instrument writers, instrument presets, packing with gt2reloc, budget gates and timing marks.
  - ✓ GoatTracker 2 .sng writer and score export, with round-trip tests [#47](https://github.com/lumi-living/score64/issues/47)
  - ✓ .ins reader and writer [#48](https://github.com/lumi-living/score64/issues/48)
  - Instrument preset library v1 with hard-restart settings [#49](https://github.com/lumi-living/score64/issues/49)
  - ✓ gt2reloc adapter: -D1 -H1 -W -Z -A, pinned version [#50](https://github.com/lumi-living/score64/issues/50)
  - Budget gate: packed size, zero page, jump table [#51](https://github.com/lumi-living/score64/issues/51)
  - ✓ Timing marks on every bar (alternating values) [#52](https://github.com/lumi-living/score64/issues/52)
  - ✓ docs: player.md - after a sound effect the voice stays silent until the music's next note [#119](https://github.com/lumi-living/score64/issues/119)
- **E8 Render & score** ([#53](https://github.com/lumi-living/score64/issues/53)) — Render packed players on both chip models through the helper and score fidelity.
  - ✓ Default SID engine: reSIDfp (libresidfp) via FFI behind a SidEngine trait (ADR-011) [#116](https://github.com/lumi-living/score64/issues/116)
  - Helper renders packed players to PCM on 6581 and 8580 [#54](https://github.com/lumi-living/score64/issues/54)
  - Register log output [#55](https://github.com/lumi-living/score64/issues/55)
  - Features: mel, chroma, onsets with differential tests [#56](https://github.com/lumi-living/score64/issues/56)
  - Fidelity metrics: melody similarity and onset F1 [#57](https://github.com/lumi-living/score64/issues/57)
  - Voice-3 dropout test [#58](https://github.com/lumi-living/score64/issues/58)
  - Oracle test: helper vs sidplayfp [#59](https://github.com/lumi-living/score64/issues/59)
  - Command engine job model: long-running commands with progress and cancellation [#132](https://github.com/lumi-living/score64/issues/132)
- **E9 CLI & HTML report** ([#60](https://github.com/lumi-living/score64/issues/60)) — The `score64` CLI over the command engine, and the shareable HTML report.
  - Commands: run, reduce, render, pack, report, index [#61](https://github.com/lumi-living/score64/issues/61)
  - JSON output and documented exit codes [#62](https://github.com/lumi-living/score64/issues/62)
  - HTML report: synced A/B players, piano roll, metrics [#63](https://github.com/lumi-living/score64/issues/63)
  - Example project with a Makefile [#64](https://github.com/lumi-living/score64/issues/64)
  - CLI reference document [#65](https://github.com/lumi-living/score64/issues/65)
  - ✓ validate command: every problem with its severity, non-zero exit only on errors, --json [#136](https://github.com/lumi-living/score64/issues/136)

## M2 Studio & sound effects

Score64 Studio (library, listen, map, pipeline, history, approvals), LLM decider, sound-effect bench, MCP server.

- **E10 Score64 Studio v1** ([#66](https://github.com/lumi-living/score64/issues/66)) — Library, listen and compare, pipeline view, command palette (docs/UX.md).
  - Shared family design tokens with Convert64 [#67](https://github.com/lumi-living/score64/issues/67)
  - Library view [#68](https://github.com/lumi-living/score64/issues/68)
  - Audio engine: multi-source sync, crossfade switching, loops (cpal) [#69](https://github.com/lumi-living/score64/issues/69)
  - Listen view: waveform, piano roll, tracker view, spectrogram [#70](https://github.com/lumi-living/score64/issues/70)
  - Pipeline view with job runner and progress (egui-snarl) [#71](https://github.com/lumi-living/score64/issues/71)
  - Command palette and score64:// deep links [#72](https://github.com/lumi-living/score64/issues/72)
  - App hosts the command engine on a worker thread (ADR-012) [#146](https://github.com/lumi-living/score64/issues/146)
  - Keep the family theme in step with Convert64 (xtask check) [#147](https://github.com/lumi-living/score64/issues/147)
- **E11 Voice map & decisions** ([#73](https://github.com/lumi-living/score64/issues/73)) — Make every arrangement decision visible and overridable; optional LLM suggestions (ADR-008).
  - Voice map view [#74](https://github.com/lumi-living/score64/issues/74)
  - Decision panel: provenance, confidence, override → stale → re-run [#75](https://github.com/lumi-living/score64/issues/75)
  - LLM decider with structured output and caching [#76](https://github.com/lumi-living/score64/issues/76)
  - Decision eval golden set and regression gate [#77](https://github.com/lumi-living/score64/issues/77)
  - Block approval on unreviewed low-confidence decisions [#78](https://github.com/lumi-living/score64/issues/78)
- **E12 Versions & approvals** ([#79](https://github.com/lumi-living/score64/issues/79)) — History, diffs, approvals and audio.lock for reproducible game builds.
  - History view with metric trends [#80](https://github.com/lumi-living/score64/issues/80)
  - Diffs: notes, config, decisions, metrics; audio A/B across versions [#81](https://github.com/lumi-living/score64/issues/81)
  - Approvals → approvals.toml and audio.lock (optional commit) [#82](https://github.com/lumi-living/score64/issues/82)
  - Comments pinned to a bar or time (comments.jsonl) [#83](https://github.com/lumi-living/score64/issues/83)
  - Import a hand-edited .sng as a manual version [#84](https://github.com/lumi-living/score64/issues/84)
- **E13 Sound-effect bench** ([#85](https://github.com/lumi-living/score64/issues/85)) — GoatTracker effect data, matching from reference recordings, variants and the resident block.
  - GoatTracker effect writer and validator with INS2SND2 parity [#86](https://github.com/lumi-living/score64/issues/86)
  - Effect matcher: per-frame features + CMA-ES over SID parameters [#87](https://github.com/lumi-living/score64/issues/87)
  - Pitch-variant preview (note-byte offset transform) [#88](https://github.com/lumi-living/score64/issues/88)
  - Priority-ordered resident block: editor and asm/bin export [#89](https://github.com/lumi-living/score64/issues/89)
  - Foley preset set: footsteps per terrain, gear clink, breathing [#90](https://github.com/lumi-living/score64/issues/90)
- **E14 MCP server** ([#91](https://github.com/lumi-living/score64/issues/91)) — Expose the command engine to AI agents (ADR-005).
  - rmcp stdio server over the command engine [#92](https://github.com/lumi-living/score64/issues/92)
  - Parity tests: MCP vs CLI [#93](https://github.com/lumi-living/score64/issues/93)

## M3 Adaptive & integrations

Adaptive sets and live audition, VICE harness, C64 Ultimate, DAWproject, release 0.1.

- **E15 Adaptive sets & audition** ([#94](https://github.com/lumi-living/score64/issues/94)) — Layered adaptive sets and live audition over the real packed set.
  - Adaptive set from one source: project-declared layers built from one arrangement [#95](https://github.com/lumi-living/score64/issues/95)
  - Audition driver (KickAssembler) with a request mailbox [#96](https://github.com/lumi-living/score64/issues/96)
  - Live audition in Studio via the helper [#97](https://github.com/lumi-living/score64/issues/97)
  - Scenario record → reference render [#98](https://github.com/lumi-living/score64/issues/98)
  - Event table export and reference scheduler (asm) [#99](https://github.com/lumi-living/score64/issues/99)
- **E16 Hardware harness & integrations** ([#100](https://github.com/lumi-living/score64/issues/100)) — KickAssembler test programs run headless in VICE, plus hardware and DAW integrations.
  - VICE harness: raster-time measurement of the play routine [#101](https://github.com/lumi-living/score64/issues/101)
  - VICE harness: zero-page guard [#102](https://github.com/lumi-living/score64/issues/102)
  - VICE harness: effect mailbox from main loop and IRQ [#103](https://github.com/lumi-living/score64/issues/103)
  - VICE harness: subtune switch on timing marks [#104](https://github.com/lumi-living/score64/issues/104)
  - C64 Ultimate push [#105](https://github.com/lumi-living/score64/issues/105)
  - DAWproject export [#106](https://github.com/lumi-living/score64/issues/106)
  - Evaluate an alternative effect driver behind the SfxWriter port [#107](https://github.com/lumi-living/score64/issues/107)
- **E17 Release 0.1** ([#108](https://github.com/lumi-living/score64/issues/108)) — First public release.
  - Native bundles via cargo-dist (macOS, Windows, Linux) [#109](https://github.com/lumi-living/score64/issues/109)
  - README screenshots and documentation pass [#110](https://github.com/lumi-living/score64/issues/110)
  - Name clearance check before announcing [#111](https://github.com/lumi-living/score64/issues/111)
  - CSDb announcement draft [#112](https://github.com/lumi-living/score64/issues/112)
  - Ship third-party licence notices with release binaries (Apache-2.0 icon font, OFL fonts, crates) [#143](https://github.com/lumi-living/score64/issues/143)
