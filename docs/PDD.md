# Score64 — Product Design Document

**Status:** draft for M0 · **Owner:** Lumi Living

## One-liner

Turn music and sound into game-ready Commodore 64 SID assets — and make every arrangement decision visible, comparable and overridable.

## Problem

Modern tools can generate convincing music and sound effects in seconds. None of it plays on a C64. The SID has three voices, one shared filter, no per-voice volume, and games usually need one voice for sound effects and the whole tune in a few kilobytes. Today the path from a recording or AI track to a player-ready file is:

1. transcribe by ear or with a generic audio-to-MIDI tool,
2. clean up MIDI in a DAW,
3. convert MIDI to a tracker format with a script that only assigns channels,
4. rebuild every instrument, arpeggio, drum and loop by hand in the tracker,
5. pack, test on both SID revisions, discover it doesn't fit, repeat.

Steps 1–3 lose most of the musical judgement, and step 4 redoes it by hand. Nothing records *why* a part went to a voice or was dropped, so iterations are expensive and hard to compare.

## Users

- **Game developers** shipping C64 games with GoatTracker-based players, who need music, adaptive layers and sound effects that fit a memory and raster budget.
- **Musicians** who compose elsewhere (DAW, AI generators, field recordings) and want a SID version that preserves the idea.
- **Sound designers** building foley and effects for the SID from reference recordings.

The first production user is *No Glory*, a turn-based C64 game, with ambience-and-foley gameplay audio and film-score-style adaptive music near mission ends.

## Scope decisions

- **Review-and-decide tool, not a tracker** (ADR-003). Score64 decides and explains; GoatTracker and SID Factory II edit notes. Hand edits re-import as versions.
- **GoatTracker 2 first** (ADR-007): its song format is documented, its player has a built-in sound-effect engine and timing marks, and both GoatTracker and SID Factory II open `.sng`.
- **Files are the source of truth** (ADR-004). Everything a user decides is a text file in git.
- **Rust + egui, native desktop** (ADR-001), same family as Convert64.
- **MIT, with a separate GPL emulator helper** (ADR-002).

## Feature list (✅ = M1/M2, ⏩ = later)

### Input
- ✅ Audio (WAV, MP3, FLAC, OGG, AAC) and MIDI input
- ✅ Transcription via provider adapters: hosted multi-instrument transcription, local Basic Pitch (ONNX) per stem
- ✅ Optional stem separation (provider stems, ONNX models; PyTorch-only models via the optional sidecar)
- ✅ Beat and downbeat grid; tempo snapping to PAL frame-exact row lengths

### Arrangement (reduction)
- ✅ Voice profiles: `menu` (3 voices, free allocation), `combat` (voice 3 expendable for sound effects), `combat2` (2-voice fallback), and mission sets of stage profiles (`bed`, `drone`, `ostinato`, `climax`, `sting`) for adaptive layers; profiles are per subtune
- ✅ Lead selection, monophonisation, octave folding into role ranges
- ✅ Chord detection → GoatTracker wavetable arpeggios
- ✅ Drum folding (dedicated voice or interleaved with bass)
- ✅ Loop finding on downbeats
- ✅ Decisions recorded with provenance (rules, LLM, manual) and confidence; overrides in `decisions.toml`
- ✅ Optional LLM suggestions for judgement calls, structured and validated (ADR-008)

### Output
- ✅ GoatTracker 2 `.sng` with instrument presets
- ✅ Packed `.prg` / `.sid` via gt2reloc with sound-effect support, timing marks and configurable zero page
- ✅ Budget checks: packed size, raster time, zero page
- ⏩ Adaptive sets: subtunes for bed, drone, ostinato, climax and stings, with timing marks on every bar
- ✅ Sound effects: GoatTracker effect format, ≤ 128 bytes, resident block in priority order
- ⏩ Effects matched from reference recordings (parameter search), pitch variants

### Review
- ✅ Renders on 6581 and 8580; melody and rhythm fidelity scores; voice-3 dropout test
- ✅ HTML report per run (shareable)
- ✅ Score64 Studio: library, listen and compare on one playhead, piano roll and tracker views, voice map, pipeline view
- ✅ Version history, diffs, approvals, `audio.lock` for the game build
- ⏩ Mission audition: stage changes and simulated effects over the real packed set, live
- ⏩ C64 Ultimate push, DAWproject export

### Automation
- ✅ CLI with parity to the app; Makefile-friendly
- ✅ MCP server for AI agents

## Non-goals

- A note or pattern editor.
- A general DAW or a SID synthesiser plug-in.
- Generating music from scratch. Score64 starts from material you bring.
- Players other than GoatTracker 2 before 1.0 (the writer is behind a port so others can follow).

## Feasibility (summary)

- **Transcription:** multi-instrument transcription of full mixes became usable in 2026 (hosted Audio-to-MIDI Pro; open MuScriptor for non-commercial use). Basic Pitch ships an official ONNX model for local per-stem use.
- **Emulation:** reSID-family emulation is available in Rust (`resid-rs`) and C++ (reSIDfp); running the real packed player in a 6502 core makes renders match what ships.
- **Formats:** GoatTracker 2 song and sound-effect formats are documented in its readme; an Apache-2.0 Python implementation exists as a test oracle.
- **Risk:** six subtunes in a 4 KB player slot is tight; dense AI mixes transcribe with ghost notes. Both are handled by budget gates, sparse prompting and decision overrides.

## Roadmap

| Milestone | Goal |
|---|---|
| **M0 Scaffold** | Workspace, CI, score model and validator, fixtures, de-risking spike (emulation, ONNX transcription) |
| **M1 Pipeline MVP** | CLI end to end: transcribe → grid → reduce → `.sng` → pack → render → score → HTML report |
| **M2 Studio & sound effects** | Score64 Studio (library, listen, map, pipeline, history, approvals), LLM decider, sound-effect bench, MCP server |
| **M3 Adaptive & integrations** | Adaptive mission sets and live audition, VICE harness, C64 Ultimate, DAWproject, release 0.1 |

Epics and issues: [docs/github/EPICS.md](github/EPICS.md).
