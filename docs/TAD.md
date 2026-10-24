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
4. routes a voice through the filter where its subtune's profile does not allow it, or starts a note on an expendable voice without an explicit pulse width (and warns about lead, bass or long notes there),
5. breaks a declared adaptive set: a layer missing or out of order, a declared switch point without a timing mark, or a layer shared by sets that switch at different rows (and warns about marks where no set switches) (§3.1),
6. violates the role's pitch range after octave folding (a warning), or plays an arpeggio note above B-7, the player's highest (an error),
7. produces sound-effect data that breaks the format rules (`reference/goattracker-sfx.md`),
8. packs empty or outside its memory window, without a jump table into the tune, using zero page outside the configured pair, writing memory outside its window, or (when budgeted) taking longer than the raster budget per play call or not measured against it; or when its budget puts the zero-page pair on the 6510's port, or the player wasn't traced, or was traced against another window,
9. gives an instrument a gate timer that is not below the tempo on a row it plays, or turns a note's gate off before it can sound (and warns when a note keeps only one or two gated frames, or when a fresh note's instrument has gate-off disabled so it won't retrigger; see `reference/sid.md` §Hard restart).

`score64_core::validate(song, &Context)` checks rules 1–6 and 9 and returns every `Problem` with its rule, **severity**, subtune, bar, row, voice, a message and a fix hint, in subtune, bar and voice order. **Errors** are things the player or the format gets wrong. **Warnings** are matters of taste or of tidiness: pitch ranges by role, timing marks that no set switches on, lead or bass on an expendable voice, long notes on an expendable voice, notes a hard restart leaves with only one or two gated frames, and fresh notes whose instrument can't retrigger them (gate-off disabled). The CLI, the MCP server and the app fail only on errors (`has_errors`); they show warnings. `score64 validate <song> [--context <file>] [--json]` (the engine's `validate` command) prints every problem with its severity. It exits 0 when there are no errors, warnings included; 1 when there are errors; and 2 when the command itself fails. Until the project files (E4) carry the context, an optional **validation context file** supplies it. That's a provisional format, replaced by E4. It's JSON or TOML (by extension, any case), and it holds:

- `schema = 1` (absent means 1; newer versions are refused);
- `[[profile]]` entries (as in §3.1);
- `[[adaptive]]` sets (`name`, `layers`, optional `switch`);
- `[instruments.<preset>]` facts (`filter`, `pulse_at_start`, `gate_timer`, `hard_restart`, `gate_off`);
- an optional `ranges` table (one `{ low, high }` per role);
- an optional `expendable_long_rows`.

Unknown keys are errors. Instrument facts move to the presets (E7) when those exist, so there is one source of truth (ADR-004). `Context` is gathered from the project: the profiles (built-in plus the project's), the adaptive sets it declares, per-instrument facts (whether the instrument routes the voice through the filter, whether it sets a pulse width at note start, and its gate timer, gate-off and hard-restart settings), and the role pitch ranges.

- **Rule 1** also covers one line per voice, and legato notes that don't start exactly where the previous note ends.
- **Rule 2:** rows are integers, so "off the grid" means outside the subtune or out of order: notes, arpeggios, tempo changes, loop row, marks and role spans.
- **Rule 3** checks each voice against its subtune's profile (§3.1), including the role of every span. A profile name that isn't defined is a problem; the profile checks are skipped for that subtune, but the others still run.
- **Song-level filter events** (ADR-015, accepted; #187). A subtune's `filter` events route voices through a filter program preset from their row on. The score owns the routing; the export writes it into the program's copy and restates the event in force on the loop row. Rule 4 makes an instrument's filter program an error on an event's row and a warning elsewhere while an event is in force (last pointer wins). Not built yet.
- **Rule 4** applies each voice's filter permission, and requires explicit pulse widths on expendable voices. An instrument without facts is an error, because it can't be checked. On expendable voices the rule also warns about lead or bass, and about notes longer than `Context::expendable_long_rows` (4 rows, one beat, by default): after an effect the voice is silent until its next note (#119).
- **Rule 5** applies each declared adaptive set (§3.1): layer order, and a timing mark at every declared switch point. A mark at a row where no set with that layer switches is a warning: it gets no `DXY` and does nothing. A subtune may be a layer of several sets only if they switch at the same rows, since otherwise it would need two sets of timing-mark values (an error).
- **Rule 6** warns when a note is outside its role's range, with the octave fold that would bring it in. The default ranges are a starting point to tune against real material: lead C-3–C-6, bass C-1–G-3, arpeggio root C-3–C-6, harmony C-2–C-5, drone C-1–C-4, ostinato C-2–C-5, bed C-1–C-6, and drums and texture anywhere. It is an **error** for an arpeggio note to pass B-7: the player adds the interval to the note, and its frequency table ends there.
- **Rule 7** waits for the sound-effect model (E13).
- **Rule 8** is `check_pack(&PackFacts, &PackBudget)`, run after packing on measured facts, not on the options the packer was given. The budget is a memory window (first and last address), the zero-page pair, whether the target calls the sound-effect entry, and an optional raster budget in cycles. The facts come from the packed bytes (`score64_formats::packed`: the load address, the size, cross-checked against gt2reloc's report, and the `JMP abs` at start+0, +3 and, with effects, +6) and from a trace of the player in the emulator helper (the zero-page addresses it touches, the pages it writes outside the window other than zero page, the stack and the SID's registers `$D400`–`$D418`, the window that was measured against, and its longest play call). The VICE harness stays the final word on raster time (§7).
- **Rule 9** follows GoatTracker's gate timer. Instrument facts state it (0–63), along with whether gate-off and hard restart happen. Frames are counted with the tempo map (`Subtune::frames`), so tempo changes between notes are respected.

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
| pack | `Packer` | gt2reloc (external program; `score64_adapters::gt2reloc`, #50) |
| report | — | static HTML |

## 3. Score model (`score64-core`)

The core model is a **tracker grid**, not MIDI ticks (ADR-006):

```rust
pub struct Song { pub schema: Schema /* = 1 */, pub profile: ProfileName /* default */, pub subtunes: Vec<Subtune> /* index = A at init */ }
pub struct Subtune { pub name: SubtuneName /* unique, stable */, pub profile: Option<ProfileName>, pub frames_per_row: FramesPerRow /* 3..=127 */,
                     pub tempo: Vec<TempoChange>, pub rows_per_bar: NonZeroU8, pub length: NonZeroU32,
                     pub loop_row: u32, pub marks: Vec<u32> /* rows with timing marks */, pub voices: Vec<VoiceLine> }
pub struct VoiceLine { pub voice: Voice /* 1..=3 */, pub roles: Roles /* [{row, role}], first at row 0 */,
                       pub instrument: PresetId /* default */, pub notes: Vec<Note>, pub arps: Vec<ArpEvent> }
pub struct Note { pub row: u32, pub length: NonZeroU32, pub pitch: Pitch /* C-0..=G#7 */, pub attack: Attack,
                  pub instrument: Option<PresetId> /* overrides the line's */, pub source: Option<SourceRef>,
                  pub confidence: Confidence, pub decision: Option<DecisionId> }
pub enum Attack { Fresh /* new gate, hard restart before it */, Legato /* tied: gate stays on */ }
pub enum Role { Lead, Bass, Arp, Harmony, Drums, Drone, Ostinato, Bed, Texture }
pub struct SourceRef { content: ContentHash /* blake3 of the source or stem */, start_us: u64, end_us: u64 }
```

- **Time is rows.** Notes, arpeggios, marks, tempo changes and loop points are integer rows; a note's gate ends at `row + length` unless the next note is `Legato`. Wall-clock time appears only in `SourceRef`, as integer microseconds into the source file it names by content hash (ADR-004), so provenance survives renames.
- **Tempo** is frames per row, set per subtune and changed by `TempoChange` events on the grid (`Subtune::frames_per_row_at`, `Subtune::frames`). With four rows per beat, a tracker quotes BPM = 750 / frames-per-row at a nominal 50 Hz; a PAL C64 really runs at 50.1246 Hz, 0.249 % faster. The frame rate follows the target's `VideoStandard`, PAL by default; NTSC runs at 59.826 Hz, so `snap_tempo` takes the standard (see `reference/tempo-grid.md`). The project setting that picks it arrives with E4 (#139). `FramesPerRow` is 3 to 127. GoatTracker tempo values below 3 select **funktempo** (alternating row lengths), which the score does **not** model yet, and a hard restart needs the frames anyway.
- **The grid** (`score64_core::grid`) joins the two clocks:
  - `snap_tempo` picks the row length whose real tempo on the target's video standard is nearest a detected tempo, choosing only from 3 upwards. It reports the nominal and real tempo, the relative error and the drift per minute.
  - `BeatGrid` turns source times (µs) into rows from the beat tracker's beats, spreading rows evenly between beats so rubato never accumulates. `quantise` gives a note's row and length, and how far its start moved (at most half a row).
  - `BeatGrid::quantise_part` (`score64_core::transcription`) puts a `TranscribedPart` on the grid: one stem's or instrument's `SourceNote`s in µs and MIDI pitch, with the source's content hash.
    - **Notes:** each comes back as a fresh-attack grid note with the transcriber's confidence and a `SourceRef` to its span, in row then pitch order. Overlaps stay; the reduction resolves them (#40).
    - **Report (`QuantiseReport`):** per part, the mean and largest start error in rows, how many starts moved more than a quarter row, how many notes were shorter than half a row and stretched to one, and every note left out, with why: a pitch GoatTracker can't play, a start before row 0 (or past the last row a song can have), or an empty span (#36). The notes come out in one canonical order whatever order the transcriber gave them in.
- **Instruments and roles change within a line.** A voice line has a default instrument, and any note may name its own (a drum line's kick, snare and hi-hat). Its roles are spans from a row on, the first at row 0, so a voice can switch role at a bar line (for example, voice 3 from bass to drums under `sfx-shares-v3`), as `voice_role` decisions do.
- **Subtunes have stable names** (`title`, `calm`, `peak`, …), unique in the song. The position in the list is the number the game passes to init; decisions and project files refer to the name, so adding or reordering subtunes cannot re-target them.
- **Timing marks belong to the subtune.** The player has one mark byte (start+$3F with `-H1`), which the game reads per subtune.
- **Pitch** is what GoatTracker patterns can play: C-0 to G#7 (note bytes `$60`–`$BC`), written as tracker names (`"A-4"`). Vibrato, slides and pulse modulation belong to instruments, not notes.
- **Files** (ADR-004): fields serialise in declaration order, lists instead of maps, absent options left out, unknown fields and newer schema versions rejected. JSON and TOML round trips are byte-stable (property-tested). The crate's `schema` feature derives JSON Schemas for engine commands; its `proptest` feature exposes generators of structurally sound songs and decisions.

### 3.1 Profiles
A profile says, for each SID voice, whether it carries **music**, **expendable** music (music that sound effects may interrupt), or **effects** only. It also says which roles the voice's music may have and whether it may use the filter. Profiles are validated data (`score64_core::profile`):

- A voice's roles list what its music may play; an empty list means any role.
- An effects-only voice has no roles and no filter.
- An expendable voice never uses the filter (rule 4). That rule is mechanical: the music's filter routing stays on while an effect plays, so the effect would be filtered too. Lead or bass on an expendable voice is allowed. Plenty of games let effects cut the lead on purpose, so the validator only warns about it.
- Every profile has at least one music voice.

Score64 ships four built-in profiles:

| Profile | V1 | V2 | V3 |
|---|---|---|---|
| `full` | music, any role, filter | music, any role, filter | music, any role, filter |
| `sfx-shares-v3` | music, any role, filter | music, any role, filter | **expendable**, any role, no filter |
| `sfx-owns-v3` | music, any role, filter | music, any role, filter | effects only |
| `sfx-owns-v2-v3` | music, any role, filter | effects only | effects only |

**Projects define their own** profiles in the same shape, so a game's voice plan lives in its project, not in Score64. Names must not repeat or redefine a built-in. For example, a quiet layer could keep the filter on a bed voice and give the rest to effects:

```toml
[[profile]]
name = "calm"
v1 = { usage = "music", roles = ["bed"], filter = true }
v2 = { usage = "effects" }
v3 = { usage = "effects" }
```

Roles can be narrowed per voice so the validator catches, say, a lead line in a layer meant for a bed. The filter flag matters because the SID has one filter: a filtered voice moves with every other filtered voice's cutoff sweeps.

**A profile belongs to a subtune**, by name. `Subtune.profile` overrides `Song.profile`, which is the default (`Song::profile_of`), because one packed file can mix subtunes with different voice rules.

**Adaptive sets** are declared by the project as an ordered list of layers, each a subtune by name, for example `calm`, `tension`, `pulse`, `peak`. Core checks the song against each set (`AdaptiveSet::check`, rule 5), and the sets against each other (`validate`):

- every layer exists;
- the layers sit at consecutive positions in the declared order, so the game can step through them by subtune number;
- every **switch point** the set declares has a timing mark in every layer, so the game knows when it may switch. A set switches on every bar by default (`switch = "every_bar"`), or on every N bars (`switch = { every_bars = 4 }`), or on explicit rows only (`switch = { rows = [0, 64, 128] }`). Explicit rows past a layer's end are problems too.

- a mark at a row where no set containing the layer switches gets no `DXY` and does nothing, which is a warning; and
- a subtune may be a layer of several sets only if they switch at the same rows in it, since it can carry only one set of timing-mark values (an error naming every set involved).

A song may hold other subtunes before or after a set, such as a title tune.

**Placing the marks** (#52):
- `AdaptiveSet::place_marks` adds a timing mark at every switch point of every layer that has none, which satisfies rule 5's mark check.
- `AdaptiveSet::timing_marks` gives each mark at a switch point its `DXY` parameter. Values alternate `$10` and `$11`, so each one changes start+`$3F`.
- A layer's marks that aren't switch points get no `DXY`, so the byte changes only where the set allows a switch.
- If the loop goes back to a mark with the same value as the last one, the last takes `$12`, so the jump back changes the byte too.
- A loop with fewer than two marks can't change the byte on every pass, and `loop_changes` says so.
- The `.sng` export (`score64_formats::gt2::export`, #47) writes these as `DXY` commands. A packed song needs `-H1` for them to reach start+`$3F`.

**What the game sees** (#52, `crates/score64-adapters/tests/timing_marks.rs`): a two-layer set exported, packed with `-H1` and traced in the emulator helper with start+`$3F` watched. The byte changes on exactly the switch-point frames the tempo map gives, including each first switch point after the loop back, and on no other frame. Init doesn't reset the byte, so a layer the game switches to keeps the old value until its row-0 mark on the eighth play call after init (`reference/player.md` §Timing marks). Every layer's row 0 is `$10`, which may equal the old value, so a game should treat its own init call as the switch rather than wait for a change. VICE confirmation belongs to the harness (§6).

An expendable voice comes back after an effect only at the music's next new note (#119), so short, frequent notes suit it best; the validator warns about long ones. Its notes start with an explicit pulse width, and it is not routed through the filter (rule 4).

### 3.2 Reduction
Lead selection by salience (pitch, density, overrides) → monophonise (ghost-note removal, minimum note length, skyline, overlaps) → octave fold into role ranges → chord detection per beat → wavetable arpeggio → drum folding → loop search on downbeats → timing marks at the switch points each adaptive set declares (every bar by default).

`score64_core::reduce` holds the rules. Each is a pure function. Most map notes to notes plus the `Decision`s they made (provenance `Rules`, with the rule's name and version); some, such as the loop finder, only propose decisions.

**Choose the lead** (`reduce::choose_lead`, rule `choose-lead@1`, #39) decides which transcribed part (a `QuantisedPart`) carries the melody in each bar of a subtune.

**Salience.** Every part gets a score per bar: the weighted mean (`LeadRules`) of four scores, which are kept for review.
- **Pitch** peaks at C-5.
- **Density** is best at about one note a beat.
- **Continuity** rewards a single line moving in steps; a repeated pitch gets no step credit.
- **Confidence** is the transcriber's.

Notes held over a bar line count in the bars they sound in. Notes past the subtune's end don't count, including in a last bar that is only partly inside it.

**Choosing.** A bar's best part becomes the lead only once it stays best for `min_run` bars, so a busy bar, a one-bar rest or the song's last bar doesn't flip it.

**Overrides.**
- A person's `LeadSource` decision holds from its bar until their next one. So does a model's decision once a person has accepted it (`reviewed`, ADR-008).
- A model's holds the same way, but only where no person's is in force. Re-running a model never undoes a person's choice. Overrides that lose (overruled, outranked on their bar, unknown, or past the end) are reported.
- The rules choose only before the first override.

**Decisions.** Each rules choice is a `LeadSource` decision (`lead-b<bar>`). Its confidence is the lead's mean salience margin over the best other part, relative to its own, over its run. It is 1 when no other part plays.

**Fold drums** (`reduce::fold_drums`, rule `fold-drums@1`, and `reduce::interleave_drums`, rule `interleave-drums@1`, #43) put transcribed drum hits on a voice.
- **Kinds.** A hit's General MIDI note gives its `DrumKind`: kick, snare, hat (and cymbals) or other.
- **Kit.** A project's `DrumKit` maps each kind to a `DrumSound`: an instrument preset, a pitch and a length in rows. The sounds themselves, such as a noise snare or a triangle kick, are presets, which Score64 doesn't name.
- **A voice of their own.** The highest-priority hit on a row wins (kick, snare, hat, other), and each note lasts its sound's rows or until the next hit. Clashing hits and kinds missing from the kit are dropped as decisions (`clash`, `unmapped`).
- **Interleaving.** Where no voice is free, `interleave_drums` takes the hits and another monophonic line, such as a bass, and folds the hits itself. That way it always knows which notes are drums, even when several kinds share one preset or the line plays a kit preset.
  - A hit cuts the line note sounding over it (a `TrimNote` decision), and that note doesn't resume.
  - `on_clash` decides a row that both start on, at confidence 0.5.
  - Hi-hats are dropped before lengths are set (`interleave`) unless allowed.
  - A line note after a drum hit always starts fresh (`Retrigger`).
  - The drums keep their own instruments.
- **Decision ids.** `drums-…` and `inter-…` ids follow monophonise's scheme (`<prefix>-<kind>-v<voice>-r<row>`, plus `-p<semitone>` for drops). A clash between kinds is certain, because priority is fixed. A clash within one kind is as sure as the winner was more confident.
- **No overrides yet.** Neither rule takes decisions in force, so a `drop_note`, `keep_note`, `trim_note` or `retrigger` about a drum voice is not applied yet; only monophonise applies them (#167). Nothing reports such a decision as unapplied for now: the engine command that runs the reductions will.

**Find the loop** (`reduce::find_loop`, rule `find-loop@1`, #44) proposes where a subtune jumps back to at its end.
- **Score.** A subtune plays to its end and carries on from `loop_row`, so a good loop row is a bar start after which the music once went the way the ending goes. The `context` bars just before it (default 2) should sound like the ending: the last `context` whole bars, up to the last bar with music in it. A part bar at the end and any trailing silence are left out. Because transcriptions often end in silence, many proposals come out half sure; trimming the subtune to its music is a separate decision for #167.
- **Comparison.** Bars are compared by chroma: each pitch class weighted by rows sounding times confidence, across all voices. Arpeggios count by their chord tones. The measure is cosine similarity.
- **Choice.** Candidates are the bars from 1 on whose context ends before the ending's starts; bars with silent context are skipped. The best candidate at or above `min_similarity` (0.8) wins, and the earliest on a tie, which gives the longest loop. With none, the loop row is 0, at confidence 0.
- **Decision.** One `LoopRow` decision (`loop`), as sure as the similarity. It's half as sure when the subtune ends inside a bar or after silence, because the jump then comes from music the comparison didn't see. Every candidate's score is returned for review. Tempo isn't compared: a loop row whose tempo differs from the ending's is not penalised.
- **In force** (`find_loop_with`, #167). A model's or a person's resolved `loop_row` replaces the choice, and `by` names it. It can be any row of the subtune where no note sounds across it and no tie starts: after the jump back that note couldn't sound, and the export refuses it (`inside_note`). One past the end is `past_end`. Of several, the strongest claims the loop row whether or not it can apply, as `resolve` would choose; when it can't, the rule's row stays. The rule still makes and returns its own decision for review. The pipeline applies tempo decisions before choosing the loop: a loop row whose tempo differs from the ending's gets its own `Fxx` in the export, which needs a free command slot on that row.

**Chords** (`reduce::detect_chords` and `reduce::arpeggiate`, rule `chords@1`, #42) find the harmony beat by beat and play it as arpeggios.
- **Detection.** Each beat's 12 pitch classes are weighted by the rows each note sounds in that beat, times its confidence. Templates for every root and quality are scored: major, minor, sus4, sus2, dominant 7th, minor 7th and major 7th.
  - **Score:** the weight on the template's tones minus the weight off them, over the beat's total weight. That is scaled by how fully the template's tones sound: a tone counts fully from a quarter of the tones' mean weight and in proportion below that, so a faint note adds little and a doubled one costs nothing. There is a small cost for each tone past three. A chord's root must sound. A beat whose best chord scores under 0.4 has no chord: a cluster, noise, or a lone note, since a melody on its own isn't harmony.
  - **Ties:** the chord rooted on the beat's lowest note wins, then the simpler quality, then the lower root. A bare fifth with its root in the bass therefore reads as major (with the fifth in the bass, as a sus4 on the fifth). Of the sus2 on D and the sus4 on A, which are the same notes, the bass decides.
  - **Changes:** a chord already sounding carries on through a beat while it scores at least half the floor and no other chord beats it by 0.25. A chord change needs clear evidence, so passing notes over a held chord don't spell a new one.
  - **Spans:** beats with the same chord merge into one `ChordSpan`, whose confidence is the mean of its beats' scores. A silent beat has no chord.
- **Arpeggios.** On an arp voice, each span is a root note and an `ArpEvent` of the chord's intervals.
  - The root sits in the octave nearest the middle of the arp range that keeps every chord tone playable. A range with no such root, one narrower than an octave, gets the playable root nearest its middle.
  - Each span is a `Chord` decision (`chord-v<voice>-r<row>`), which both the note and the arpeggio record.
  - **GoatTracker encoding:** `score64_formats::gt2::wave::arpeggio` writes an arp event's intervals as a looping wavetable program, reusing identical ones. The `.sng` export (#47) points the voice at it with `8XY` on the event's row, and back at the instrument's own program where the event ends while the note sounds on.

**Monophonise** (`reduce::monophonise`, rule `monophonise@1`, #40) makes one voice's notes a single line, in four steps:
1. It drops ghosts (confidence below `ghost_below`, 0.2 by default) and notes shorter than `min_rows`.
2. It keeps one note per row. The skyline takes the highest, or the lowest for bass. A repeat of the same pitch merges into the longest; the other notes are dropped.
3. It resolves overlaps. A note that starts less than `min_rows` after or before a more confident one is dropped; the most confident notes are placed first. Then a note that runs into the next is trimmed to end where the next starts. This covers ends that round past the next start (#138). Every drop is caused by a note that stays, so drops never cascade, and a note that trimming can keep is kept.
4. It restarts (`Fresh`) a tie whose predecessor no longer ends on its row.

Every drop, trim and restart is a decision. Its confidence is 1 for mechanical steps, 1 − the note's confidence for a ghost, 0.5 for a skyline pick, and for an overlap drop the confidence gap to the note that stays (1 when a keep forces it, below). Any order of the same notes gives the same line and the same decisions.

Decision ids name what they're about, so they stay the same when notes elsewhere change:
- `mono-trim-v<voice>-r<row>` for a trim;
- `mono-tie-v<voice>-r<row>` for a restarted tie;
- `mono-<reason>-v<voice>-r<row>-p<semitone>` for a drop, with `-2`, `-3` for further drops of one kind at one place. Those suffixes follow the drops' order, so another such drop can renumber them.

They're unique within the subtune when each voice is monophonised once, with all its notes in one call.

**Overrides** (`reduce::monophonise_with`, #167): the resolved decisions of a model or a person for the voice apply as the rule runs, rather than being patched onto its result.
- A `drop_note` takes its note out before step 1, so the rule's other choices follow from the line without it. It takes the note from its source span when it names one; of identical notes otherwise, the one the rule would drop. A drop, a trim and a fresh start of one note are separate slots, so a drop gives way (`overruled`, or `outranked` at the same rank) to a stronger trim or fresh start of that note, even one that then can't apply: a stronger decision about the note means it stays. "Stronger" here, as everywhere a keep, a drop, a trim or a fresh start meet, is `resolve`'s order: the higher rank, then the lower id.
- A `trim_note` sets its note's length once the line is built, and only shortens: longer than the rule left it is unapplied (`lengthens`). It replaces the rule's own trim of that note. Shorter than `min_rows` is a person's call; an unreviewed model's is unapplied (`out_of_range`).
- A `retrigger` makes its note fresh, so the rule records no tie of its own there.
- A `keep_note` (ADR-014 §8) keeps a note on its row and pitch that the rule would drop. Keeps apply one at a time, the strongest first, each with the stronger ones in force. The rule runs with the kept notes in place, so the notes around them give way by its own logic:
  - **Drops in force.** A keep is met by any note the drops leave on its row and pitch. So to keep a shorter repeat, keep the row and pitch and drop the longer ones, whatever their ids. To bring back a longer repeat a drop took, drop the shorter ones too. A drop names its note by length, so of twins of one length it takes one only, and the other meets the keep. Only when the drops take out every note there do they compete with the keep. It saves the longest note whose drop it's stronger than, and that drop loses to it (`overruled` or `outranked`); when every drop there is stronger, the keep loses to the strongest. When the keep then can't apply, the drop applies after all.
  - **Ghost, too short.** Where no note of the kept row and pitch passes step 1, the first (the longest) stays. Shorter than `min_rows` is a person's call: an unreviewed model's keep of it is unapplied (`out_of_range`), as for a trim.
  - **Skyline.** The kept pitch is the one its row keeps, in its longest note as for any repeats; the row's other notes are dropped (`repeat` or `skyline`). A skyline drop the keep forces is mechanical, so its confidence is 1; one the line without the keeps makes too keeps 0.5.
  - **Overlap.** Kept notes are placed first, so a note closer than `min_rows` to one is dropped (`overlap`) even when it's the surer. When a kept note is among the neighbours it's too close to, the drop is mechanical, so its confidence is 1; a note the line without the keeps drops as an overlap too keeps that drop's confidence. A knock-on drop, caused only by notes that aren't kept, has the usual gap. Then a note running into a kept note is trimmed where it starts, and a kept note running into the next is trimmed too. So keeping a note never makes two notes overlap: the other note is trimmed, or dropped when it can't keep `min_rows`.
  - **When a keep can't apply.** It's unapplied, and changes nothing, decisions included, when it can't keep its note: an unreviewed model's keep of a note too short (`out_of_range`), or one whose note a stronger keep's note pushes out, on its row or closer than `min_rows` (`overruled` or `outranked`). It's unapplied too when its note would push out a note that a stronger keep, trim or fresh start is about, so a model's keep never takes out a note a person trimmed or restarted (ADR-008).

  A keep that the line with the stronger keeps already meets changes no note, and when no other keep changes the line, no decision either. Its note still stays against weaker keeps and, like every kept note, is placed first, so with other keeps in force a drop next to it can become certain. A kept note records the keep's id when the keep saved it, from a drop in force or from the rule (the line with only the stronger keeps doesn't have it). It doesn't when it already records one, and a trim or fresh start in force on it leaves that id. Only monophonise applies keeps: a keep about a drum voice is not applied yet (see the drum rules above).

A note an override names that isn't there is unapplied (`no_such_note`). A note an override changed records the override's id in place of this rule's; an id from an earlier rule stays. Of several decisions for one slot, the strongest claims it whether or not it then applies, so the result is the one `resolve` and then the rule give. The rules' own decisions in the list are ignored, since the rule makes them afresh, and the order of the overrides doesn't matter.

**Fold octaves** (`reduce::fold_octaves`, rule `fold-octaves@1`, #41) moves a voice line into the pitch range of its roles. The ranges are the validator's `RoleRanges`, which a project's presets will supply, so the reduction and rule 6 agree.
- **One shift per bar.** Each bar gets a single whole-octave shift, so the melody keeps its shape inside the bar.
- **Choosing the shift.** Only shifts that keep every note playable count. Among those it picks the one that leaves the fewest notes outside the range of the role at their row. On a tie, it moves nothing if that's as good, so a bar moves only to bring notes in. Next it prefers the previous bar's shift, if the bars are next to each other and in the same role, so the line doesn't jump an octave at a bar line. Then it takes the smallest.
- **Notes that still don't fit** stay where they are, and rule 6 warns about them.
- **Full-range roles never move.** Notes whose role may use the whole range (drums and texture, by default) are left alone and don't count. That's a property of the ranges, so narrowing the drums' range folds drums too, just as rule 6 then checks them.
- **Decisions.** Each bar with a non-zero shift is one `Octave` decision, `octave-v<voice>-b<bar>`. Its confidence is the share of the bar's folded notes in range after the shift. The input can come in any order.
- **In force** (`fold_octaves_with`, #167). A model's or a person's resolved `octave` sets its bar's shift for the voice instead of the rule, and the next bar may carry it on as it would the rule's. The rule records no decision there, and the notes record the override's id unless they already record a decision. Notes count in the bar they start in. An override is unapplied when the bar has no note that folds (`no_such_note`) or the shift takes one past GoatTracker's range (`out_of_range`); of several, the strongest claims the bar, and when it can't apply the bar is the rule's.

**Tempo** (`reduce::apply_tempo`, #167). No rule proposes a tempo. A model's or a person's resolved `frames_per_row` sets the tempo from its bar's first row: the starting tempo at bar 0, and a tempo change after that, replacing one on the same row. A tempo already playing there adds no change, since each change is a command in the export. It returns the decision behind each row it set, for provenance, and those that couldn't apply: one past the end, or one beaten by a stronger decision for the same bar. Rule 9 checks the result like any tempo.

In all of these, `unapplied` is sorted by id, and of several decisions for one slot the strongest claims it, whether or not it then applies (`overruled` for a weaker one, `outranked` for an equal one with a higher id), so the result is the one `resolve` and then the rule give.

### 3.2.1 Writing a GoatTracker song

`score64_formats::gt2::export(song, library, adaptive, header)` turns a validated score into an `Sng` (#47). The library maps preset ids to GoatTracker instruments with their own tables, in `.ins` form; the presets (#49) will fill it. It refuses, rather than writes wrong: notes or tempo changes past the end, overlapping notes or arpeggios, a legato note with nothing to tie to (on the loop row, after the jump back too), a note sounding across the loop row (after the jump back it couldn't be), and an arpeggio ending on a tie (that row's command slot holds the tie, so the program couldn't be stopped).

- **Patterns.** Each voice's rows are cut at every bar start and at the loop row, into pieces of at most 128 rows. So all three orderlists stay in step, and the restart position is the pattern that starts at the loop row. Identical patterns are shared across voices and subtunes, up to GoatTracker's 208.
- **Notes.**
  - A fresh note plays with its instrument.
  - A legato note ties with `3` `00` and no instrument, so it isn't retriggered (#137).
  - A note followed by silence ends with a key-off. One followed directly by another note leaves the gate to that note's hard restart.
  - A note that runs to the end, when the loop row is silent, gets a key-off on the loop row, so it stops after the jump back.
- **Song-wide commands.** These go on the first voice with a free command slot, after the voices' own commands; a row with none is an error:
  - the tempo (`Fxx`, all voices) on row 0, at each tempo change, and on the loop row when the tempo there differs from the tempo at the end;
  - each timing mark of an adaptive layer (`DXY`, values from `AdaptiveSet::timing_marks`). `Exported::timing_marks` says the song must be packed with `-H1`.
- **Arpeggios** use `8XY` and §3.2's wavetable programs, one frame a step.
  - Every note sounding during the event gets the pointer: the one under its start, and each fresh note inside it, since a new note resets the wave pointer. A tie keeps the program running.
  - Where the event ends while a note sounds on, the voice goes back to that note's instrument's program, unless another arpeggio starts there.
  - A note plus an interval must stay at or below B-7 (rule 6 checks it), and the instrument's first wavetable row must give a gated waveform.
- **Instruments** are numbered by first use, up to 62; 63 stays free because it can carry a tempo setting. Their tables move into the song's: pointers, `$FF` jump targets, and the wavetable's command rows (`1`–`4XY` and `EXY` into the speedtable, `9XY` the pulsetable, `AXY` the filtertable), as gt2reloc relocates them.
- **Not yet checked:** that the library's gate timers and gate-off flags match the instrument facts the validator used (#49), and that the adaptive sets passed here are the validator's.
- **Evidence.** `crates/score64-adapters/tests/sng_export.rs` exports scores, packs them with the real `gt2reloc` and plays them in the emulator helper. It checks:
  - the notes' frequency ratios;
  - the tie holding the gate;
  - each key-off;
  - the tempo (5 frames a row, not GoatTracker's default 6);
  - every frame of an arpeggio, from its first row to its last;
  - after a jump back to a silent loop row, the last note stopped and the loop row's tempo restored.

  In those renders the gate timer closes the gate 2 frames before a key-off's row. The test runs where `gt2reloc` is installed, which CI isn't. Listening on 6581 and 8580, and VICE, are still to do.

### 3.3 Decisions
Every judgement call is a `Decision { id, subtune /* by name */, bar, proposal, provenance, confidence, reviewed }`:

- `proposal` is the change proposed, one variant per kind of decision: voice role, lead source, octave shift, chord (from a row, since chords change by the beat), loop row, frames per row, and the monophonise rule's `trim_note`, `drop_note` (with a reason: ghost, too short, repeat, skyline or overlap; for drums also clash, unmapped or interleave) and `retrigger`. Those three name their note by voice, row and pitch (a drop also by its length, to tell it from a kept repeat), plus its source span when it has one, because decisions do not point at notes. `keep_note` (ADR-014 §8) rejects a rule's drop: it names the note to keep by voice, row and pitch only. No rule makes one. A decision's id is unique within its subtune; notes refer to decisions of their own subtune.
- `provenance` is `Rules { rule }`, `Llm { model, prompt }` (a blake3 hash of the exact prompt, which is also the cache key), or `Manual`.

Manual decisions override LLM suggestions, which override rules. `score64_core::decide::resolve` (#45) applies that wherever decisions meet.
- **Slots.** Two decisions compete when they are about the same thing, their `Slot`:
  - a voice's role, the lead's source, or the tempo, from a bar on;
  - a voice's octave in a bar;
  - the chord from a row;
  - the subtune's loop row;
  - a note's trim, its drop, or its fresh start, by voice, row and pitch. A drop also uses the note's length, which tells two repeats on one row apart. These are three separate slots, since one note can have all three.
  - whether a note on a row and pitch is kept: a `keep_note` competes with the other keeps there. The drops of notes there are slots of their own, as a trim and a drop of one note are, so `resolve` chooses the strongest of each. Whether a keep and those drops conflict depends on the notes there, which only the reduction sees: the keep is met by any note the drops leave, and only when they take out every note does the stronger win (§3.2). So a person's keep beats the rules' drop, a person's drop beats a model's keep, and a person can keep a shorter repeat by keeping its row and pitch and dropping the longer ones, whatever their ids.
- **Ranks.** In each slot the strongest wins:
  1. a person's decision, or a model's that a person has accepted (`reviewed`);
  2. a model's, not yet reviewed;
  3. the rules'.

  Within a rank the lower id wins (ids compare as text), then the surer decision, so input order never matters. A later decision doesn't win by being later, so a command that writes a keep or a drop of a note should replace a same-rank decision about that note that contradicts it (a drop of the note the keep would keep, or a keep of the note the drop takes), rather than add beside it.
- **Over time.** Decisions that hold from a bar (voice role, lead source, tempo) also compete over time, as `choose_lead` treats leads. A decision made where a stronger one of its kind is already in force (made at that bar or earlier, for the same voice) is overruled by it. A person's choice therefore holds over any later model's or rules', until the person decides again.
- **Checked first.** `decide::check(decisions, &song, sources)` runs before `resolve`, so a decision that can't apply never holds a slot against one that can (#167). `sources` lists each subtune's parts. A decision can't apply when it names:
  - a subtune that isn't in the song;
  - a bar, or a row, past the subtune's end;
  - a row outside its own bar;
  - an octave shift beyond ±7, a chord root above 11, chord intervals that don't start at 0 and rise, or a chord whose top note is past G#7 even on its lowest root;
  - a lead source that isn't one of its own subtune's parts.

  Each one comes back whole with its `Unapplied` reason, since an id is only unique within its subtune. A `keep_note`'s row is checked like a drop's. Whether a note-level decision's note exists is left to the reduction that applies it. The rules only ever make decisions that pass, as a property test checks, as long as parts are clipped to the subtune before they're reduced.
- **What's returned.** Every loser comes back with the id that beat it, so a review can show both. A rule's drop is rejected with a `keep_note`; the reductions apply decisions in force (§3.2, #167). A rules' drop that a keep rejects comes back nowhere: `resolve` keeps both, and the reduction ignores the rules' own decisions and makes them afresh, without that drop. A review shows what the keep did by comparing the line with and without it.

A decision never changes the song by itself. An engine command (`writes_project`) validates the proposal and applies it, and the notes it produces record the decision's `id`; decisions do not point at notes. LLM output is structured, validated against the invariants and never written straight into song data, and LLM decisions stay unreviewed until a person accepts them (ADR-008).

## 4. Project and versions (`score64-project`)

| State | Lives in |
|---|---|
| Intent: `track.toml`, `decisions.toml`, presets, scenarios, approvals, comments, `audio.lock` | git (text) |
| Artifacts: stems, MIDI, `.sng`, renders, `.prg` / `.sid`, metrics | content-addressed cache `.score64/cache/<stage>/<hash>/` |
| Index for the app: versions, metrics, search | SQLite `.score64/index.db`, rebuildable from manifests and git |

**The stage cache** (`score64_project::Cache`, #26) lives at `.score64/cache/<stage>/<key>/`.
- **Key.** An entry's key is `Cache::key`: blake3 of canonical JSON (keys sorted at every depth) of the stage, its version, its config and its input hashes. The same work always has the same key, and any change gives another. Numbers are taken as written, so `1` and `1.0` differ; that costs a needless miss, never a wrong hit. `put` computes the key from the run itself, so an entry can't claim inputs it wasn't made from.
- **Contents.** An entry holds the artifacts and a `manifest.json`: schema, stage, key, the run (stage version, tool version, config, inputs, timings, optional cost) and each artifact's name, blake3 and size.
- **Writing.** `put` writes an entry whole into a fresh temporary directory, syncs it, and renames it into place. The directory is named by pid, counter and clock, so another process or container can't share it, and a crashed run's leftovers never block one. If two writers race, the first rename wins and the other returns that entry; stages are pure, so either copy is right. Something that isn't a whole entry in the way, damaged or from an older layout, is replaced. If what was moved aside turns out whole, another writer got there first, and it is put back and kept. An entry from a newer Score64 is left alone.
- **Reading.** `get` checks the manifest and every artifact's hash. A missing, partial or damaged entry is a miss, never a wrong hit.
- **Names.** Stage and artifact names are plain names, so nothing writes outside the cache. They are also safe on every system: no leading or trailing dot, no Windows device names, and no two artifacts whose names differ only by case.

**`decisions.toml`** (`score64_project::DecisionsFile`, #25) holds a track's decisions: `schema = 1`, then one `[[decision]]` table per decision, in the shape of §3.3. Each one carries `id`, `subtune`, `bar`, `confidence` and `reviewed`, a `[decision.proposal]` table tagged by `change`, and a `[decision.provenance]` table tagged by `by`.
- **Writing.** Decisions are sorted by subtune and then id. Each one's plain fields come in declaration order, then its tables, so the same decisions always give the same bytes. A file is written to a uniquely named temporary file beside it, synced, then renamed into place.
- **Reading.** A missing `schema` means 1, and any other version is refused.
  - An unknown field, a bad value or a TOML error is reported with its file, line and column. One inside a `proposal` or `provenance` table points at that table's header.
  - A manual provenance with stray fields is refused.
  - Two decisions with one id in one subtune are reported with both `[[decision]]` lines.

**The project and track files** (`score64_project::Project`, #25, ADR-014). A project is a folder for one target:

```text
score64.toml                 what every track of the target shares
presets/                     the target's instrument presets (E7)
tracks/<id>/track.toml       one track's intent
tracks/<id>/decisions.toml   its decisions
tracks/<id>/history/         its recorded runs (ADR-014 §7; #27, not built yet)
.score64/                    cache and index, not committed
```

- **`score64.toml`** (`ProjectFile`): `[target] video` (PAL by default), `[[profile]]` entries added to the built-in profiles (§3.1; redefining a built-in is an error), `[pack]` (`player_page`, `zero_page`, `sound_effects`, optional `hard_restart_adsr`; no defaults, since they come from the target's memory map. ADR-014's amendment replaces `player_page` with a page-aligned `window = { start, end }` and adds an optional `max_play_cycles`, #198), `[tools] gt2reloc = { version, reference }` (the banner version and the blake3 of packing the reference fixture with the reference options), and `[validate]` (`expendable_long_rows`, `ranges`).
- **`track.toml`** (`TrackFile`): an optional `title` and default `profile`, `[[source]]` (`id`, `path`, `blake3`, `kind` = `mix`, `stem` or `midi`), `[[subtune]]` (`name`, optional `profile`, `from` a source, optional `start` and `end`), `[[adaptive]]` sets (§3.1), and `[rules.<stage>]` tables that each stage checks for itself.
- **Checks** on reading:
  - source ids, subtune names and adaptive set names are unique;
  - a subtune's source exists and its range isn't empty;
  - adaptive layers are the track's subtunes;
  - profiles are built in or the project's;
  - `[pack]` is in range (`player_page` ≥ `$02`, `zero_page` in `$02`–`$FE`).
- **Overrides.** A track may override only `[target] video` and `[pack]` keys. The merged `[pack]` must be complete and is checked again. Any other project table in a track is an unknown key.
- **Times** are exact-microsecond strings, `[h:]m:ss` with up to six decimals (`"0:12.500"`). Floats are refused. They are written as `m:ss`, or as `h:mm:ss` from an hour, with three decimals for whole milliseconds and six otherwise.
- **Paths** are relative to the project root, with `/` between parts. None is absolute, empty, `.` or `..`, a Windows device name, a part ending in `.` or a space, or has `\`, `:`, `<`, `>`, `"`, `|`, `?`, `*`, or a control or bidirectional-override character. They are compared as written and aren't Unicode-normalised. Ids are letters, digits, `-` and `_`, and never a device name, so a track id is a safe folder name. A folder under `tracks/` with a `track.toml` but an invalid name is an error, and so are two track ids that differ only in case.
- **Sources** are found by path and identified by blake3 (`Project::check_sources`), hashed as they are read. A source is *missing* when there's no file, *unreadable* when it's a folder, has no permission, or leads out of the project through a link, and *changed* when its hash differs, which names both hashes. Only sources are checked for links out of the project; the track files are read where they are. Git LFS is optional. `Project::record_hash` records a new hash on purpose, and changes only that value in the file.
- **Writing.** Every write goes through a synced temporary file beside the target. A rewrite (`Project::record_hash`, `decisions.toml`) renames it over the old file and keeps its permissions. A new file (`Project::create`, `Project::create_track`) is hard-linked into place, so it never replaces a file, even one another writer creates meanwhile. Where hard links fail (FAT, exFAT), it checks and renames instead. A new file is written deterministically and must pass the same checks as reading, the project's included. Existing files are edited in place rather than rewritten through serde, which would drop comments and print numbers in decimal. A track id must be its folder's exact name, and a new track can't differ from an existing folder only in case. Errors give the file, line and column (in characters).

**Versions** (`score64_project::version`, #27, ADR-014 §6). A track's version id is the blake3, in the cache's canonical JSON, of its resolved intent, its sources' hashes and the tool version, tagged with the scheme (`version@1`). A test pins the fixture's id, so any change to what goes in forces a new scheme. The intent is:
- `track.toml` without its `[target]` and `[pack]` overrides, which count only through what they resolve to, and without its sources' paths: a source is its hash, so moving the file is no new version;
- `decisions.toml` in subtune and id order;
- the resolved video standard and `[pack]`;
- the definition of the profile each subtune plays with (its own, else the track's default, else `full`), so a change to a profile no subtune uses leaves it alone;
- the project's `[validate]` and `[tools]`.

How a file is written (comments, key order, hex or decimal, quoting) doesn't change the id. Everything in the intent that can change the output does, and numbers in `[rules]` must be finite so they can. Approvals, comments and `audio.lock` aren't in it, since they don't change the output; presets join it when they land (#49), with a new scheme. The track's id isn't in it, so two tracks with the same intent share a version, and a track's history keys on the track and the version together.

**Run history** (ADR-014 §7, #27, not built yet). A content-addressed id makes restoring an old intent give its old id back. So the history of runs is a separate record: which version came from which, and whether by a re-run, a model or a person, including hand-edited `.sng` files imported with provenance `manual`. It is one file per recorded run in `tracks/<id>/history/`, holding the version, its parent, the provenance and the UTC time. A run is recorded only when the version changes, or on a restore.

## 5. Repository layout and dependencies

```
crates/
  score64-core       pure model, profiles, reduction, validator
  score64-formats    .sng/.ins/sound-effect bytes, MIDI, WAV; the score as a GoatTracker song (depends on core)
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

Dependencies are pinned in the workspace `Cargo.toml`. Expected set (added by the epics that need them): `clap`, `serde`, `toml`, `serde_json`, `blake3`, `rusqlite`, `midly`, `symphonia`, `cpal`, `rustfft`, `ort`, `reqwest`, `schemars`, `cmaes`, `rmcp`, `eframe`/`egui`, `egui_plot`, `egui-snarl`, `egui_material_icons`, `rfd`, `insta`, `proptest`, and `process-wrap` for the sidecar. The SID engine (reSIDfp from libresidfp, GPL, vendored in `residfp-sys`, ADR-011; `resid-rs` kept for comparison until #59) and the 6502 core (`emulator_6502`, MIT, ADR-010) are dependencies of `score64-emu` only.

A sidecar run (`score64_adapters::sidecar`) starts `uv` in a process tree of its own, a process group on Unix and a Job object on Windows (`process-wrap`), because `uv` does not pass a kill on to the Python it starts. When the run ends (a timeout, an error, a normal exit, or the runner being dropped), whatever is left of the tree is killed, so no script outlives its run (#126). If Score64 itself is killed or stopped with Ctrl-C, the tree survives, and on Unix a descendant that leaves the group (`setsid`) escapes it (#154).

### 5.1 The emulator helper
`score64-emu serve` reads a packed player plus commands on stdin (load, init subtune, poke mailbox, render N frames, set chip model, trace) and writes PCM frames, register logs and trace reports on stdout, in the length-prefixed binary protocol of [`reference/emu-protocol.md`](reference/emu-protocol.md). The app keeps one helper process for live audition and renders in short chunks that continue one timeline; the CLI starts one per render. The codec (`score64_formats::emu_protocol`, MIT) is shared by the helper and the client (`score64_adapters::emu`, MIT), so the GPL code stays behind the process boundary. The protocol is versioned and covered by tests in both directions: the client against a scripted fake helper and against the real binary, and the helper against scripted request streams. The CPU sits behind a `Cpu` trait (ADR-010): the helper steps one instruction at a time, timestamps each SID write on the cycle it lands, and checks the core against a subset of the SingleStepTests 6502 vectors. A trace (protocol version 1, #51) gives what rule 8 and the timing-mark test need: the zero-page addresses the player touches, the pages it writes outside its window, and the frames where watched addresses change.

The machine the helper emulates is deliberately minimal (measured against VICE in [the #20 spike](research/2026-10-18-packed-player-spike.md)):

- **Memory:** 64 KB of flat RAM with no ROMs and no bank switching. The SID is at `$D400`, mirrored every 32 bytes up to `$D7FF`; registers `$1D`–`$1F` of each mirror are ignored. The rest of `$D000`–`$DFFF` is plain RAM. `$00`/`$01` are plain RAM, not the 6510 port: with no ROMs there is nothing to switch. The helper counts writes to them, so a player that relies on banking gets noticed.
- **PAL only.** The helper's clock and frame are PAL's (`score64_formats::emu_protocol::PAL_CLOCK_HZ`, `PAL_CYCLES_PER_FRAME`), so a project targeting NTSC can't be rendered or auditioned yet (#139).
- **Interrupts: none.** The harness calls init once, then play once per PAL frame (19,656 cycles) from its own loop, as a game's raster interrupt would, and calls the sound-effect entry (start+6) the same way. The IRQ/NMI vectors at `$FFFA`–`$FFFF` are not used and no CIA or VIC is emulated, so there is no interrupt latency and no bad-line cycle stealing. PSID files whose play address is 0, and RSID files, are rejected. On a real machine interrupt latency moves the play call by a few cycles from frame to frame; the helper's is exact.
- **Write log:** every SID write is recorded in order with its cycle inside the frame. Oracle tests diff these per frame (register, value and cycle) as well as the audio. Gate-off and hard-restart timing, where the ADSR bug shows up, depends on this order and timing.
- **SID engine:** behind a `SidEngine` trait. The default is reSIDfp (libresidfp 1.2.3, vendored in `residfp-sys`, ADR-011), set up as sidplayfp 3.2.2 sets it (filter curves and 6581 range 0.5, combined waveforms average, resampling). Fed VICE's exact write log, it matches sidplayfp's own render at waveform r 0.998 (6581) and 0.996 (8580). Where material depends on the free-running envelope counters (slow decays), two sidplayfp runs with different power-on delays differ by as much as our engine does. `--engine resid-rs` keeps the old engine for comparison until #59.
- Besides `serve`, the helper has two command-line modes for research and oracle work: `render` (PSID → WAV, write log, opcode histogram, optional sound-effect triggers) and `replay` (a VICE `-sounddev dump` log through the same SID).

### 5.2 The command engine
`score64-engine` holds every user-visible action as a `Command` (ADR-005, ADR-012). A command has a stable snake_case name, a one-line summary, an effect (`read_only`, `writes_project` or `destructive`: MCP tool hints, and which changes get provenance and undo), `serde` arguments and output with JSON Schemas from `schemars`, and a `run` against the `Engine`. `REGISTRY` lists them, and `CommandError` carries a stable `kind` (`unknown_command`, `invalid_args`, `invalid_input`, `helper_missing`, `helper_failed`, `helper_timeout`, `helper_refused`, `internal`) and a message.

- **Front-ends:** the CLI has typed subcommands for common commands and `score64 run <name> '<json>'` for any of them; `score64 commands --json` prints every name with both schemas. The MCP server (E14) will list the registry as tools. The app calls commands by type. CLI tests run every registry command through `score64 run`, so a command cannot exist in one front-end only.
- **CLI exit codes (ADR-013):**
  - 0 on success;
  - 1 when a command ran but its output is a failure. The command decides, through `Command::fails` (`validate` with errors), so the typed subcommand, `score64 run` and later the MCP server's error flag all agree;
  - 2 when a command failed (its JSON error goes to stderr) or the command line is wrong (clap's usage message, plain text).
- **Numbering:** text output numbers bars 1-based and rules by their §1.3 number (`bar 1`, `rule 4`). JSON keeps the data as it is: `"bar": 0` and `"rule": "filter_and_expendable"`.
- **Paths:** file arguments resolve against the process's working directory; a project root arrives with E4.
- **Threading:** one command at a time (`run` takes `&mut Engine`). The engine is `Send`, which a compile-time assertion checks. The app owns it on a background thread behind a channel and never calls it on the UI thread. Long-running jobs with progress and cancellation (live audition, batch renders) are designed in #132 before the M1 render work.
- **The helper belongs to the engine.** Commands hand it jobs and never hold the raw client. It starts on the first job, and runs on a worker thread. Each job, and the handshake, waits under a timeout (30 s by default; long renders pass their own); on timeout the engine kills the process. After a timeout, a `Malformed` reply, a broken pipe or a crash, the next job starts a fresh helper; a helper that refuses a request keeps running. The engine counts starts, so a command holding helper state (an audition session) knows when to load and init again.

## 6. Testing

| Layer | What |
|---|---|
| Unit | Score model operations, reduction rules, tempo snapping, sound-effect encoding |
| Property (`proptest`) | `score64_core::strategies` generates checked songs (a song plus the `Context` it satisfies), note soups and valid profiles. Properties: checked songs raise no errors (no false positives); breaking any one rule (1–6, 9) in its own way gets that rule reported (no misses), including a layer removed from or misplaced in a declared set, and rule 9's two warnings (as warnings, with no error); validation is unchanged by a JSON round trip (ADR-004); on any note soup, rule 1 fires exactly when the soup is not already a clean line; model files and encoders round-trip byte for byte. Any note soup monophonises to a line with no errors, every note kept or dropped by a decision, whatever the input order (#40); octave folding only moves pitches, by whole octaves, never leaves a bar with more notes out of range, and picks each bar's best playable shift (#41) |
| Snapshot (`insta`) | Annotated dumps (`score64_formats::annotate`) of GoatTracker song, instrument and sound-effect bytes: one line per header field, orderlist, instrument, table row, pattern row and effect frame, accounting for every byte. Today they cover the committed fixtures (`fixtures/gt2-original`); the `.sng` and effect writers (#47, #86) snapshot their output the same way, and fixture MIDI → `.sng` follows |
| GoatTracker | The `.sng` layer against GoatTracker itself. `crates/score64-adapters/tests/sng_export.rs` packs exported songs with the real `gt2reloc` and plays them in the emulator helper (#185), and `tests/pack_facts.rs` reads the same player back from every output container and passes rule 8 (#51), and `tests/timing_marks.rs` traces start+`$3F` in a packed adaptive song (#52); it runs where `gt2reloc` is installed and skips visibly elsewhere, CI included. The layout was read against GoatTracker's `gsong.c` in review (#181). Listening sets for `needs-vice-check` PRs come from `cargo xtask listen` (`docs/TOOLING.md`) |
| Differential | Ports against originals: Basic Pitch vs the Python package (#21, end to end through the sidecar); features vs librosa. The sidecar (`sidecar/`, ADR-009) is a locked uv project whose scripts write a blake3 manifest that `score64_adapters::sidecar` validates. These tests are opt-in (`SCORE64_SIDECAR=1`), print SKIPPED otherwise, and run in CI's `sidecar` job |
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
