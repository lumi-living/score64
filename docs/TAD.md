# Score64 — Technical Architecture Document

**Status:** draft for M0. Facts about the hardware, player and formats live in [`docs/reference/`](reference/README.md); this document describes how Score64 uses them.

## 1. Constraints (the type system of the output)

### 1.1 The SID
Three voices (triangle, sawtooth, pulse, noise; ADSR each), one shared multimode filter, one master volume, no per-voice volume. Two chip revisions (6581, 8580) differ mainly in filter and combined waveforms. Details: [`reference/sid.md`](reference/sid.md).

### 1.2 The player
GoatTracker 2's packed player, built by gt2reloc with sound-effect support (`-D1`) and, for adaptive sets, author info and timing marks (`-H1`). Jump table: init (A = subtune), play, sound-effect entry. Details: [`reference/player.md`](reference/player.md).

### 1.3 Validator rules
The validator in `score64-core` rejects a song that:
1. has more than one note sounding at a time on any voice,
2. places a note off the row grid,
3. exceeds the voice count of its profile,
4. in `combat` / `mission` climax profiles, puts lead or bass on voice 3, routes voice 3 through the filter, or starts a voice-3 note without an explicit pulse width,
5. has a subtune without a timing mark on every bar (adaptive sets),
6. violates the role's pitch range after octave folding,
7. produces sound-effect data that breaks the format rules (`reference/goattracker-sfx.md`),
8. packs larger than the configured budget, or uses zero page outside the configured pair.

## 2. Pipeline

Each stage is a pure function of (input artifacts, stage config, stage version) → (output artifacts, manifest). Outputs are cached under a hash of those inputs (`blake3`), so re-runs are incremental and paid calls happen once.

```
source ─► separate? ─► transcribe ─► grid ─► reduce ─► instruments ─► render ─► score ─► pack ─► report
             │             │           │        │                       │                 │
          Separator   Transcriber  BeatTracker Decider               Renderer           Packer
          (port)      (port)       (port)      (rules | llm | manual) (emu helper)       (gt2reloc)
```

Stages and ports:

| Stage | Port | Adapters (M1–M2) |
|---|---|---|
| separate (optional) | `Separator` | provider stems (read files), ONNX models via `ort`, sidecar for PyTorch-only models |
| transcribe | `Transcriber` | hosted multi-instrument API, Basic Pitch ONNX per stem, sidecar (non-commercial models flagged) |
| grid | `BeatTracker` | Beat This! (ONNX) |
| reduce | `Decider` | `RulesDecider` (default), `LlmDecider` (structured output), `ManualDecider` (`decisions.toml`) |
| instruments | — | preset library (`presets/*.toml` + `.ins`) |
| render | `Renderer` | `score64-emu` helper over a pipe (GPL, separate process; reSIDfp engine, ADR-011) |
| score | — | `score64-analysis` |
| pack | `Packer` | gt2reloc (external program) |
| report | — | static HTML |

## 3. Score model (`score64-core`)

The core model is a **tracker grid**, not MIDI ticks (ADR-006):

```rust
pub struct Note { pub row: u32, pub length: u32, pub pitch: u8, pub source: SourceRef, pub confidence: f32 }
pub enum Role { Lead, Bass, Arp, Drums, Drone, Ostinato, Bed, Texture }
pub struct VoiceLine { pub voice: Voice /* 1..=3 */, pub role: Role, pub notes: Vec<Note>, pub instrument: PresetId,
                       pub arps: Vec<ArpEvent>, pub marks: Vec<u32> /* rows with timing marks */ }
pub struct Subtune { pub index: u8, pub frames_per_row: u8, pub voices: Vec<VoiceLine>, pub loop_row: u32 }
pub struct Song { pub subtunes: Vec<Subtune>, pub profile: Profile }
```

Tempo is frames per row at 50 Hz (PAL). With four rows per beat, BPM = 750 / frames-per-row (see `reference/tempo-grid.md`).

### 3.1 Profiles
| Profile | Voices | Rules |
|---|---|---|
| `menu` | 3, free | Any role on any voice |
| `combat` | 3 | V1 lead, V2 bass (+ weak-beat arps), V3 expendable (drums/arp/texture); validator rule 4 |
| `combat2` | 2 | V3 silent, reserved for sound effects |
| `mission` | 1 → 3 | Subtunes 0 bed (V1), 1 drone, 2 ostinato (V2), 3 climax (`combat` rules), 4–5 stings |

### 3.2 Reduction
Lead selection by salience (pitch, density, overrides) → monophonise (skyline) → octave fold into role ranges → chord detection per beat → wavetable arpeggio → drum folding → minimum note length and ghost-note removal → loop search on downbeats → timing marks on bar lines.

### 3.3 Decisions
Every judgement call is a `Decision { subtune, bar, kind, value, provenance: Rules | Llm | Manual, confidence, reviewed }`. Manual decisions override LLM suggestions, which override rules. LLM output is structured, validated against the invariants, and never written straight into song data (ADR-008).

## 4. Project and versions (`score64-project`)

| State | Lives in |
|---|---|
| Intent: `track.toml`, `decisions.toml`, presets, scenarios, approvals, comments, `audio.lock` | git (text) |
| Artifacts: stems, MIDI, `.sng`, renders, `.prg` / `.sid`, metrics | content-addressed cache `.score64/cache/<stage>/<hash>/` |
| Index for the app: versions, metrics, search | SQLite `.score64/index.db`, rebuildable from manifests and git |

A version is an immutable run, identified by a hash of intent + input hashes + tool version. Restoring creates a new version. Hand-edited `.sng` files import as versions with provenance `manual`.

## 5. Repository layout and dependencies

```
crates/
  score64-core       pure model, profiles, reduction, validator
  score64-formats    .sng/.ins/sound-effect bytes, MIDI, WAV
  score64-project    track files, decisions, cache, index, audio.lock
  score64-analysis   features, onsets, fidelity metrics, effect feature curves
  score64-adapters   transcribers, beat tracker, deciders, gt2reloc, VICE, C64 Ultimate, sidecar
  score64-cli        `score64`
  score64-mcp        `score64-mcp`
  score64-app        Score64 Studio (egui/eframe)
  score64-emu        GPL helper binary (6502 + SID), separate process
  residfp-sys        GPL, vendored libresidfp (reSIDfp) + C shim; only score64-emu may depend on it (ADR-011)
xtask/               maintenance tasks
docs/                PDD, TAD, UX, ADRs, reference, epics
presets/             instrument presets
fixtures/            rights-clear audio and MIDI with expected outputs
sidecar/             optional Python scripts (uv), files in → files out (ADR-009)
```

Dependencies are pinned in the workspace `Cargo.toml`. Expected set (added by the epics that need them): `clap`, `serde`, `toml`, `serde_json`, `blake3`, `rusqlite`, `midly`, `symphonia`, `cpal`, `rustfft`, `ort`, `reqwest`, `schemars`, `cmaes`, `rmcp`, `eframe`/`egui`, `egui_plot`, `egui-snarl`, `egui_material_icons`, `rfd`, `insta`, `proptest`. The SID engine (reSIDfp from libresidfp, GPL, vendored in `residfp-sys`, ADR-011; `resid-rs` kept for comparison until #59) and the 6502 core (`emulator_6502`, MIT, ADR-010) are dependencies of `score64-emu` only.

### 5.1 The emulator helper
`score64-emu` reads a packed player plus commands on stdin (load, init subtune, poke mailbox, render N frames, set chip model) and writes PCM frames and register logs on stdout. The app keeps one helper process for live audition; the CLI starts one per render. The protocol is versioned and covered by tests in both directions. The CPU sits behind a `Cpu` trait (ADR-010): the helper steps one instruction at a time, timestamps each SID write on the cycle it lands, and checks the core against a subset of the SingleStepTests 6502 vectors.

The machine the helper emulates is deliberately minimal (measured against VICE in [the #20 spike](research/2026-10-18-packed-player-spike.md)):

- **Memory:** 64 KB of flat RAM with no ROMs and no bank switching. The SID is at `$D400`, mirrored every 32 bytes up to `$D7FF`; registers `$1D`–`$1F` of each mirror are ignored. The rest of `$D000`–`$DFFF` is plain RAM. `$00`/`$01` are plain RAM, not the 6510 port: with no ROMs there is nothing to switch. The helper counts writes to them, so a player that relies on banking gets noticed.
- **Interrupts: none.** The harness calls init once, then play once per PAL frame (19,656 cycles) from its own loop, as a game's raster interrupt would, and calls the sound-effect entry (start+6) the same way. The IRQ/NMI vectors at `$FFFA`–`$FFFF` are not used and no CIA or VIC is emulated, so there is no interrupt latency and no bad-line cycle stealing. PSID files whose play address is 0, and RSID files, are rejected. On a real machine interrupt latency moves the play call by a few cycles from frame to frame; the helper's is exact.
- **Write log:** every SID write is recorded in order with its cycle inside the frame. Oracle tests diff these per frame (register, value and cycle) as well as the audio. Gate-off and hard-restart timing, where the ADSR bug shows up, depends on this order and timing.
- **SID engine:** behind a `SidEngine` trait. The default is reSIDfp (libresidfp 1.2.3, vendored in `residfp-sys`, ADR-011), set up as sidplayfp 3.2.2 sets it (filter curves and 6581 range 0.5, combined waveforms average, resampling). Fed VICE's exact write log, it matches sidplayfp's own render at waveform r 0.998 (6581) and 0.996 (8580). Where material depends on the free-running envelope counters (slow decays), two sidplayfp runs with different power-on delays differ by as much as our engine does. `--engine resid-rs` keeps the old engine for comparison until #59.
- Until the stdin protocol arrives (#8), the helper has two command-line modes: `render` (PSID → WAV, write log, opcode histogram, optional sound-effect triggers) and `replay` (a VICE `-sounddev dump` log through the same SID).

## 6. Testing

| Layer | What |
|---|---|
| Unit | Score model operations, reduction rules, tempo snapping, sound-effect encoding |
| Property (`proptest`) | Validator invariants hold for any reduced song; encoders round-trip |
| Snapshot (`insta`) | Fixture MIDI → `.sng` bytes as annotated dumps |
| Differential | Ports against originals: `.sng` writer vs pygoattracker; Basic Pitch post-processing vs the Python package; features vs librosa (test-only sidecar) |
| Oracle | Packed player rendered by `score64-emu` vs sidplayfp, within tolerance |
| Audio regression (slow) | Reference tracks: melody similarity, onset F1, voice-3 dropout, both chips |
| Hardware harness (slow) | KickAssembler test programs in VICE: raster time, zero-page guard, sound-effect mailbox, subtune switching on timing marks |

## 7. Performance budgets

| Item | Budget |
|---|---|
| Re-run after one decision change (transcription cached) | < 10 s per track |
| Studio: load a 3-minute set with 6 sources | < 2 s |
| Studio: switch audible source | < 20 ms, no playback interruption |
| Packed player play routine (in game) | measured by the VICE harness against the game's configured raster budget |
