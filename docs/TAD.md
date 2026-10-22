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
4. in subtunes under the `combat` or `climax` profile, puts lead or bass on voice 3, routes voice 3 through the filter, or starts a voice-3 note without an explicit pulse width,
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
pub struct Song { pub schema: Schema /* = 1 */, pub profile: Profile, pub subtunes: Vec<Subtune> /* index = A at init */ }
pub struct Subtune { pub name: SubtuneName /* unique, stable */, pub frames_per_row: FramesPerRow /* 3..=127 */,
                     pub tempo: Vec<TempoChange>, pub rows_per_bar: NonZeroU8, pub length: NonZeroU32,
                     pub loop_row: u32, pub marks: Vec<u32> /* rows with timing marks */, pub voices: Vec<VoiceLine> }
pub struct VoiceLine { pub voice: Voice /* 1..=3 */, pub roles: Roles /* [{row, role}], first at row 0 */,
                       pub instrument: PresetId /* default */, pub notes: Vec<Note>, pub arps: Vec<ArpEvent> }
pub struct Note { pub row: u32, pub length: NonZeroU32, pub pitch: Pitch /* C-0..=G#7 */, pub attack: Attack,
                  pub instrument: Option<PresetId> /* overrides the line's */, pub source: Option<SourceRef>,
                  pub confidence: Confidence, pub decision: Option<DecisionId> }
pub enum Attack { Fresh /* new gate, hard restart before it */, Legato /* tied: gate stays on */ }
pub enum Role { Lead, Bass, Arp, Drums, Drone, Ostinato, Bed, Texture }
pub struct SourceRef { content: ContentHash /* blake3 of the source or stem */, start_us: u64, end_us: u64 }
```

- **Time is rows.** Notes, arpeggios, marks, tempo changes and loop points are integer rows; a note's gate ends at `row + length` unless the next note is `Legato`. Wall-clock time appears only in `SourceRef`, as integer microseconds into the source file it names by content hash (ADR-004), so provenance survives renames.
- **Tempo** is frames per row at 50 Hz (PAL), set per subtune and changed by `TempoChange` events on the grid. With four rows per beat, BPM = 750 / frames-per-row (see `reference/tempo-grid.md`). `FramesPerRow` is 3 to 127. GoatTracker tempo values below 3 select **funktempo** (alternating row lengths), which the score does **not** model yet, and a hard restart needs the frames anyway; tempo snapping (#15) chooses only from 3 upwards.
- **Instruments and roles change within a line.** A voice line has a default instrument, and any note may name its own (a drum line's kick, snare and hi-hat). Its roles are spans from a row on, the first at row 0, so a voice can switch role at a bar line (for example, voice 3 from bass to drums in `combat`), as `voice_role` decisions do.
- **Subtunes have stable names** (`bed`, `drone`, `climax`, …), unique in the song. The position in the list is the number the game passes to init; decisions and project files refer to the name, so adding or reordering subtunes cannot re-target them.
- **Timing marks belong to the subtune.** The player has one mark byte (start+$3F with `-H1`), which the game reads per subtune.
- **Pitch** is what GoatTracker patterns can play: C-0 to G#7 (note bytes `$60`–`$BC`), written as tracker names (`"A-4"`). Vibrato, slides and pulse modulation belong to instruments, not notes.
- **Files** (ADR-004): fields serialise in declaration order, lists instead of maps, absent options left out, unknown fields and newer schema versions rejected. JSON and TOML round trips are byte-stable (property-tested). The crate's `schema` feature derives JSON Schemas for engine commands; its `proptest` feature exposes generators of structurally sound songs and decisions.

### 3.1 Profiles
A profile says which voices carry music in a subtune, with which roles, and which voice sound effects may interrupt. **It belongs to the subtune:** `Subtune.profile` overrides `Song.profile`, which is the default (`Song::profile_of`). One packed mission set mixes stages with different voice rules. The quiet stages use all three voices for music; in the climax, effects take voice 3, which therefore can't hold sustained bass (#119). The rules are data in `score64_core::profile`, and the validator (#13) checks songs against them.

| Profile | V1 | V2 | V3 |
|---|---|---|---|
| `menu` | any | any | any |
| `combat` | lead | bass or arp (weak-beat arps) | **expendable**: drums, arp or texture; rule 4 |
| `combat2` | lead | bass or arp | no music (effects only) |
| `bed` | any | — | — |
| `drone` | any | — | any (the drone) |
| `ostinato` | any | any (the ostinato) | any |
| `climax` | lead | bass or arp | **expendable**, as `combat` |
| `sting` | any | any | any |

**Mission sets** are songs whose subtunes use the stage profiles, laid out in the order the game numbers them: 0 `bed`, 1 `drone`, 2 `ostinato`, 3 `climax`, then up to two `sting`s (victory, defeat), so 4–6 subtunes. `check_mission_layout` enforces this for any song that uses a stage profile; other songs are not mission sets. Voices grow from one (bed) to three (ostinato), as before. The drone stage puts the drone on V3 and leaves V2 for the ostinato that follows; this reading of the original "1 → 3" table is open to change.

An expendable voice comes back after an effect only at the music's next new note (#119), so it should carry short, frequent notes. Its notes start with an explicit pulse width, and it is not routed through the filter (rule 4).

### 3.2 Reduction
Lead selection by salience (pitch, density, overrides) → monophonise (skyline) → octave fold into role ranges → chord detection per beat → wavetable arpeggio → drum folding → minimum note length and ghost-note removal → loop search on downbeats → timing marks on bar lines.

### 3.3 Decisions
Every judgement call is a `Decision { id, subtune /* by name */, bar, proposal, provenance, confidence, reviewed }`:

- `proposal` is the change proposed, one variant per kind of decision: voice role, lead source, octave shift, chord, loop row, frames per row.
- `provenance` is `Rules { rule }`, `Llm { model, prompt }` (a blake3 hash of the exact prompt, which is also the cache key), or `Manual`.

Manual decisions override LLM suggestions, which override rules. A decision never changes the song by itself. An engine command (`writes_project`) validates the proposal and applies it, and the notes it produces record the decision's `id`; decisions do not point at notes. LLM output is structured, validated against the invariants and never written straight into song data, and LLM decisions stay unreviewed until a person accepts them (ADR-008).

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
  score64-adapters   transcribers, beat tracker, deciders, gt2reloc, VICE, C64 Ultimate, sidecar, helper client
  score64-engine     commands, registry and shared resources (the helper) behind every front-end (ADR-005, ADR-012)
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
`score64-emu serve` reads a packed player plus commands on stdin (load, init subtune, poke mailbox, render N frames, set chip model) and writes PCM frames and register logs on stdout, in the length-prefixed binary protocol of [`reference/emu-protocol.md`](reference/emu-protocol.md). The app keeps one helper process for live audition and renders in short chunks that continue one timeline; the CLI starts one per render. The codec (`score64_formats::emu_protocol`, MIT) is shared by the helper and the client (`score64_adapters::emu`, MIT), so the GPL code stays behind the process boundary. The protocol is versioned and covered by tests in both directions: the client against a scripted fake helper and against the real binary, and the helper against scripted request streams. The CPU sits behind a `Cpu` trait (ADR-010): the helper steps one instruction at a time, timestamps each SID write on the cycle it lands, and checks the core against a subset of the SingleStepTests 6502 vectors.

The machine the helper emulates is deliberately minimal (measured against VICE in [the #20 spike](research/2026-10-18-packed-player-spike.md)):

- **Memory:** 64 KB of flat RAM with no ROMs and no bank switching. The SID is at `$D400`, mirrored every 32 bytes up to `$D7FF`; registers `$1D`–`$1F` of each mirror are ignored. The rest of `$D000`–`$DFFF` is plain RAM. `$00`/`$01` are plain RAM, not the 6510 port: with no ROMs there is nothing to switch. The helper counts writes to them, so a player that relies on banking gets noticed.
- **Interrupts: none.** The harness calls init once, then play once per PAL frame (19,656 cycles) from its own loop, as a game's raster interrupt would, and calls the sound-effect entry (start+6) the same way. The IRQ/NMI vectors at `$FFFA`–`$FFFF` are not used and no CIA or VIC is emulated, so there is no interrupt latency and no bad-line cycle stealing. PSID files whose play address is 0, and RSID files, are rejected. On a real machine interrupt latency moves the play call by a few cycles from frame to frame; the helper's is exact.
- **Write log:** every SID write is recorded in order with its cycle inside the frame. Oracle tests diff these per frame (register, value and cycle) as well as the audio. Gate-off and hard-restart timing, where the ADSR bug shows up, depends on this order and timing.
- **SID engine:** behind a `SidEngine` trait. The default is reSIDfp (libresidfp 1.2.3, vendored in `residfp-sys`, ADR-011), set up as sidplayfp 3.2.2 sets it (filter curves and 6581 range 0.5, combined waveforms average, resampling). Fed VICE's exact write log, it matches sidplayfp's own render at waveform r 0.998 (6581) and 0.996 (8580). Where material depends on the free-running envelope counters (slow decays), two sidplayfp runs with different power-on delays differ by as much as our engine does. `--engine resid-rs` keeps the old engine for comparison until #59.
- Besides `serve`, the helper has two command-line modes for research and oracle work: `render` (PSID → WAV, write log, opcode histogram, optional sound-effect triggers) and `replay` (a VICE `-sounddev dump` log through the same SID).

### 5.2 The command engine
`score64-engine` holds every user-visible action as a `Command` (ADR-005, ADR-012). A command has a stable snake_case name, a one-line summary, an effect (`read_only`, `writes_project` or `destructive`: MCP tool hints, and which changes get provenance and undo), `serde` arguments and output with JSON Schemas from `schemars`, and a `run` against the `Engine`. `REGISTRY` lists them, and `CommandError` carries a stable `kind` (`unknown_command`, `invalid_args`, `helper_missing`, `helper_failed`, `helper_timeout`, `helper_refused`, `internal`) and a message.

- **Front-ends:** the CLI has typed subcommands for common commands and `score64 run <name> '<json>'` for any of them; `score64 commands --json` prints every name with both schemas. The MCP server (E14) will list the registry as tools. The app calls commands by type. CLI tests run every registry command through `score64 run`, so a command cannot exist in one front-end only.
- **Threading:** one command at a time (`run` takes `&mut Engine`). The engine is `Send`, which a compile-time assertion checks. The app owns it on a background thread behind a channel and never calls it on the UI thread. Long-running jobs with progress and cancellation (live audition, batch renders) are designed in #132 before the M1 render work.
- **The helper belongs to the engine.** Commands hand it jobs and never hold the raw client. It starts on the first job, and runs on a worker thread. Each job, and the handshake, waits under a timeout (30 s by default; long renders pass their own); on timeout the engine kills the process. After a timeout, a `Malformed` reply, a broken pipe or a crash, the next job starts a fresh helper; a helper that refuses a request keeps running. The engine counts starts, so a command holding helper state (an audition session) knows when to load and init again.

## 6. Testing

| Layer | What |
|---|---|
| Unit | Score model operations, reduction rules, tempo snapping, sound-effect encoding |
| Property (`proptest`) | Validator invariants hold for any reduced song; encoders round-trip |
| Snapshot (`insta`) | Annotated dumps (`score64_formats::annotate`) of GoatTracker song, instrument and sound-effect bytes: one line per header field, orderlist, instrument, table row, pattern row and effect frame, accounting for every byte. Today they cover the committed fixtures (`fixtures/gt2-original`); the `.sng` and effect writers (#47, #86) snapshot their output the same way, and fixture MIDI → `.sng` follows |
| Differential | Ports against originals: `.sng` writer vs pygoattracker; Basic Pitch vs the Python package (#21, end to end through the sidecar); features vs librosa. The sidecar (`sidecar/`, ADR-009) is a locked uv project whose scripts write a blake3 manifest that `score64_adapters::sidecar` validates. These tests are opt-in (`SCORE64_SIDECAR=1`), print SKIPPED otherwise, and run in CI's `sidecar` job |
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
