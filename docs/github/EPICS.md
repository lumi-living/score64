# Epics and milestones

Each epic is a GitHub issue labelled `epic`; child issues are GitHub sub-issues of the epic and say "Part of #N" in their body. Labels: `epic`, `core`, `formats`, `project`, `analysis`, `adapters`, `emu`, `cli`, `mcp`, `app`, `sfx`, `docs`, `fixtures`, `ci`, `release`, `spike`, `licence`, `needs-vice-check` (packed output verified in VICE or sidplayfp on 6581 and 8580 before merge), `needs-hardware-check`, `bug`, `enhancement`, `good first issue`.

| # | Milestone | Epics |
|---|---|---|
| 1 | **M0 Scaffold** | E1 E2 E3 |
| 2 | **M1 Pipeline MVP** | E4 E5 E6 E7 E8 E9 |
| 3 | **M2 Studio & sound effects** | E10 E11 E12 E13 E14 |
| 4 | **M3 Adaptive & integrations** | E15 E16 E17 |

## M0 Scaffold

Workspace, CI, score model and validator, fixtures, and the de-risking spike (emulation, ONNX transcription).

- **E1 Workspace & CI** — Cargo workspace with crate skeletons (core, formats, project, analysis, adapters, cli, mcp, app, emu), lints and toolchain pin matching Convert64; CI on macOS/Linux/Windows plus the licence-boundary job; app boots; command engine and emulator-helper skeletons.
  - ✓ Workspace skeleton: crates, lints, toolchain pin
  - ✓ CI: fmt, clippy, test matrix and licence-boundary job
  - ✓ Community files, issue templates, Dependabot
  - Branch protection on main with the CI contexts
  - score64-app boots an empty eframe window (light/dark, Material Symbols)
  - Command engine skeleton shared by CLI, MCP and app (ADR-005)
  - score64-emu helper skeleton and versioned stdin/stdout protocol
  - Release tooling: cargo-dist configuration
- **E2 Score model & validator** — The tracker-grid score model (ADR-006), voice profiles, tempo grid and the validator for TAD §1.3, with property tests.
  - Score model types: Note, VoiceLine, Role, Subtune, Song, Decision
  - Voice profiles: menu, combat, combat2, mission
  - Validator for TAD §1.3 rules with per-bar, per-voice errors
  - Property tests: any reduced song validates
  - Tempo grid: frames-per-row, BPM snapping with error report
  - Hard-restart lead-time check
- **E3 Fixtures & de-risking spike** — Rights-clear fixtures, golden-test harness, sidecar bootstrap, and a time-boxed spike on the two Rust-specific risks: running packed players in a 6502 core with SID emulation, and Basic Pitch through ONNX.
  - Rights-clear audio and MIDI fixtures
  - Spike: choose a 6502 core for score64-emu
  - Spike: run a gt2reloc-packed tune in 6502 core + resid-rs and compare with sidplayfp
  - Spike: Basic Pitch ONNX via ort vs the Python package
  - Golden-test harness for .sng and effect bytes (insta)
  - Python sidecar bootstrap (uv) with the manifest contract

## M1 Pipeline MVP

CLI end to end: transcribe → grid → reduce → .sng → pack → render → score → HTML report.

- **E4 Project & cache** — Files are the source of truth (ADR-004): track and decision files, content-addressed stage cache, immutable versions, rebuildable index.
  - track.toml and decisions.toml schemas with validation
  - Content-addressed stage cache with manifests
  - Immutable versions: identity from intent + inputs + tool version
  - SQLite index rebuildable from manifests and git
  - Cache garbage collection policy
  - Provenance and licence flags per artifact
- **E5 Transcription & beat grid** — Transcriber and beat-tracker ports with adapters; decode, tempo snapping and quantisation.
  - Audio decode and resampling (symphonia)
  - Transcriber port and hosted multi-instrument adapter
  - Basic Pitch ONNX adapter with differential test
  - Beat This! beat and downbeat grid (ONNX)
  - Quantise notes to rows on the snapped tempo grid
  - Optional separation adapters (provider stems, ONNX models)
- **E6 Reduction to SID voices** — Rules that turn transcribed parts into a valid song for a profile, with every judgement call recorded as a decision.
  - Lead selection by salience, with override
  - Monophonise, minimum note length, ghost-note removal
  - Octave folding into role ranges
  - Chord detection → GoatTracker wavetable arpeggio tables
  - Drum folding: dedicated voice or interleaved with bass
  - Loop finder on downbeats
  - Rules decider and manual decider (decisions.toml overrides)
- **E7 GoatTracker output & packing** — Byte-exact GoatTracker 2 song and instrument writers, instrument presets, packing with gt2reloc, budget gates and timing marks.
  - GoatTracker 2 .sng writer with round-trip and differential tests
  - .ins reader and writer
  - Instrument preset library v1 with hard-restart settings
  - gt2reloc adapter: -D1 -H1 -W -Z -A, pinned version
  - Budget gate: packed size, zero page, jump table
  - Timing marks on every bar (alternating values)
- **E8 Render & score** — Render packed players on both chip models through the helper and score fidelity.
  - Helper renders packed players to PCM on 6581 and 8580
  - Register log output
  - Features: mel, chroma, onsets with differential tests
  - Fidelity metrics: melody similarity and onset F1
  - Voice-3 dropout test
  - Oracle test: helper vs sidplayfp
- **E9 CLI & HTML report** — The `score64` CLI over the command engine, and the shareable HTML report.
  - Commands: run, reduce, render, pack, report, index
  - JSON output and documented exit codes
  - HTML report: synced A/B players, piano roll, metrics
  - Example project with a Makefile
  - CLI reference document

## M2 Studio & sound effects

Score64 Studio (library, listen, map, pipeline, history, approvals), LLM decider, sound-effect bench, MCP server.

- **E10 Score64 Studio v1** — Library, listen and compare, pipeline view, command palette (docs/UX.md).
  - Shared family design tokens with Convert64
  - Library view
  - Audio engine: multi-source sync, crossfade switching, loops (cpal)
  - Listen view: waveform, piano roll, tracker view, spectrogram
  - Pipeline view with job runner and progress (egui-snarl)
  - Command palette and score64:// deep links
- **E11 Voice map & decisions** — Make every arrangement decision visible and overridable; optional LLM suggestions (ADR-008).
  - Voice map view
  - Decision panel: provenance, confidence, override → stale → re-run
  - LLM decider with structured output and caching
  - Decision eval golden set and regression gate
  - Block approval on unreviewed low-confidence decisions
- **E12 Versions & approvals** — History, diffs, approvals and audio.lock for reproducible game builds.
  - History view with metric trends
  - Diffs: notes, config, decisions, metrics; audio A/B across versions
  - Approvals → approvals.toml and audio.lock (optional commit)
  - Comments pinned to a bar or time (comments.jsonl)
  - Import a hand-edited .sng as a manual version
- **E13 Sound-effect bench** — GoatTracker effect data, matching from reference recordings, variants and the resident block.
  - GoatTracker effect writer and validator with INS2SND2 parity
  - Effect matcher: per-frame features + CMA-ES over SID parameters
  - Pitch-variant preview (note-byte offset transform)
  - Priority-ordered resident block: editor and asm/bin export
  - Foley preset set: footsteps per terrain, gear clink, breathing
- **E14 MCP server** — Expose the command engine to AI agents (ADR-005).
  - rmcp stdio server over the command engine
  - Parity tests: MCP vs CLI

## M3 Adaptive & integrations

Adaptive mission sets and live audition, VICE harness, C64 Ultimate, DAWproject, release 0.1.

- **E15 Adaptive sets & audition** — Mission profile layering and live audition over the real packed set.
  - Mission profile: bed, drone, ostinato, climax and stings from one source
  - Audition driver (KickAssembler) with a request mailbox
  - Live audition in Studio via the helper
  - Scenario record → reference render
  - Ambient event table export and reference scheduler (asm)
- **E16 Hardware harness & integrations** — KickAssembler test programs run headless in VICE, plus hardware and DAW integrations.
  - VICE harness: raster-time measurement of the play routine
  - VICE harness: zero-page guard
  - VICE harness: effect mailbox from main loop and IRQ
  - VICE harness: subtune switch on timing marks
  - C64 Ultimate push
  - DAWproject export
  - Evaluate an alternative effect driver behind the SfxWriter port
- **E17 Release 0.1** — First public release.
  - Native bundles via cargo-dist (macOS, Windows, Linux)
  - README screenshots and documentation pass
  - Name clearance check before announcing
  - CSDb announcement draft
