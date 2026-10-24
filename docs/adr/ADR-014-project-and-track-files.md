# ADR-014: Project and track files: `score64.toml` and `track.toml`

**Status:** Accepted
**Date:** 2026-10-23 (amended 2026-10-24: §2 `[pack]`, §7 run history, §8 `KeepNote`)
**Deciders:** @jahaa

## Context

ADR-004 makes files the source of truth, and TAD §4 lists `track.toml` as a track's intent. No document says what that intent holds, so several pieces of work have stopped at the same gap:

- `decisions.toml` (#25) is done, but `track.toml` is not.
- Versions (#27) hash "intent + inputs + tool version" and need to know what intent is.
- The index (#28), cache garbage collection (#29) and per-artifact provenance (#30) all key off a track.
- The video-standard project setting (#139) has nowhere to live.
- The packer adapter (#50) takes its options from the caller, with "project config" still to come.
- ADR-013 added a provisional validation context file, to be replaced once the project files exist.

Some of this intent belongs to a track: its sources, subtunes and adaptive sets. Some belongs to the **target** the music is for (a game, a demo, any program that plays it), and every track shares it: the voice plan (profiles), where the player sits in memory, its zero page, and the pinned packer. Today the validation context mixes the two.

`decisions.toml` already set conventions that the other text files should share (#170 review):
- a `schema` key;
- deterministic writes;
- atomic replacement;
- errors located by line and column.

## Decision

**1. Two files, at two levels.** A project is a directory, normally a git repository:

```
score64.toml               the project: one target's shared settings
presets/                   instrument presets, one per preset id (E7, #49)
tracks/<track>/track.toml  a track: its sources, subtunes, adaptive sets and rule settings
tracks/<track>/decisions.toml
tracks/<track>/history/         one file per recorded run (§7)
tracks/<track>/approvals.toml   (E4/E10, later)
audio.lock                      (E10, later)
.score64/                       cache and index, not committed
```

A track's id is its directory name, and must be a plain name as the cache defines it (TAD §4).

**2. `score64.toml` holds what every track of the target shares.** One project is one target.

```toml
schema = 1

[target]
video = "pal"                 # "pal" (default) or "ntsc"

# The target's voice plan: profiles in the shape of TAD §3.1, added to the built-in ones.
[[profile]]
name = "calm"
v1 = { usage = "music", roles = ["bed"], filter = true }
v2 = { usage = "effects" }
v3 = { usage = "effects" }

[pack]                        # gt2reloc (#50) and the budget gate (#51); no defaults except where marked
window = { start = 0x1000, end = 0x1FFF }  # the memory the target gives the tune, inclusive; the player goes at start
zero_page = 0xF8              # the player's two zero-page bytes, from the target's memory map
sound_effects = true
hard_restart_adsr = 0x0F00    # optional; gt2reloc's default
max_play_cycles = 4000        # optional; the longest play call the target allows, in CPU cycles; unchecked if absent

[tools]
# The packer pin: its banner version, and the blake3 of packing Score64's reference fixture with the reference
# options. That hash is the same on every platform for the same packer, unlike the binary's own hash.
gt2reloc = { version = "2.73", reference = "…" }

[validate]                    # optional; TAD §1.3 settings
expendable_long_rows = 4
# ranges = { ... }            role pitch ranges, as the validator's RoleRanges
```

**3. `track.toml` holds one track's intent.**

```toml
schema = 1
title = "Main theme"          # optional, for people

[[source]]
id = "mix"
path = "audio/main-theme.wav" # relative to the project root, forward slashes, never outside it
blake3 = "…"                  # the content the track was made from; checked on every use
kind = "mix"                  # "mix", "stem" or "midi"

[[subtune]]
name = "calm"                 # becomes the subtune's name in the song
profile = "calm"              # optional; the song's default profile otherwise
from = "mix"                  # a source id
# start = "0:12.500"         optional range in the source: [h:]m:ss with up to six decimals, exact microseconds
# end = "1:14"

[[adaptive]]                  # TAD §3.1 shape
name = "score"
layers = ["calm", "tension"]
switch = { every_bars = 2 }   # optional; every bar by default

[rules.reduce]                # optional settings for one stage, owned and versioned by that stage
# min_note_rows = 1
```

- **`profile`** at the top level is optional and names the song's default profile. Otherwise the default is `full`.
- **`[pack] window`** (amended 2026-10-24, #198) replaces `player_page`. It's the target's memory for the tune, `start` to `end` inclusive.
  - `start` must be page-aligned, since gt2reloc places the player on a page boundary (`-W`), and the player is packed there.
  - `end` must be at least `start`.
  - The budget gate (rule 8, #51) checks that the packed player, its data and everything it writes outside the zero page stay inside the window.
  - **`max_play_cycles`**, when set, is the raster budget: the longest play call measured by the helper's trace must not exceed it. The VICE harness stays the final word on raster time (TAD §7).
- **Overrides:** a track may override `[target] video` and any `[pack]` key under its own `[target]` and `[pack]` tables. An overridden `window` replaces the whole window. The project's values apply otherwise. Nothing else in `score64.toml` can be overridden per track, so the voice plan is one per target.
- **Source ranges** are strings, `[h:]m:ss` with up to six decimals (`"0:12.500"`, `"1:14"`), read into whole microseconds, so a range is exact and diffs cleanly. Floats are refused. `start` defaults to the source's start and `end` to its end; `end` must come after `start`.
- **Sources are found by path and identified by hash.** Each `[[source]]` names a path under the project root and the blake3 of its content. The audio may be committed, kept in Git LFS, or left untracked and supplied by hand: Score64 only reads the file at its path.
  - A missing file is an error naming the source and the path ("source `mix`: `audio/main-theme.wav` is missing").
  - A file whose hash differs is an error naming both hashes ("…changed since the track was made: expected `…`, found `…`"), never a silent re-transcription.
  - An explicit command re-records the hash.
- **Instrument facts** don't belong in either file. They come from the presets in `presets/` (E7, #49, ADR-013's plan), so they have one source.

**4. Shared conventions for every project text file** (`score64.toml`, `track.toml`, `decisions.toml`, and later `approvals.toml` and `audio.lock`):
- `schema = 1`. A missing key means 1, and any other version is refused, naming the version this build reads. No migrations until one is needed.
- Unknown keys are errors. A stage's `[rules.<stage>]` keys are checked by that stage.
- Errors name the file, line and column.
- Writes are deterministic, so the same content always gives the same bytes. They go to a uniquely named temporary file beside the target, synced, then renamed into place.
- Paths are relative to the project root, use `/`, and are refused if they leave the root.

**5. The validation context comes from the files.** It is made of:
- the built-in profiles plus `score64.toml`'s;
- the track's adaptive sets;
- the presets' instrument facts;
- `[validate]`.

ADR-013's provisional context file goes once this lands. The packer's options and pin come from `[pack]` and `[tools]`.

**The packer pin is checked by a reference pack.** Before packing, Score64 packs its reference fixture with the reference options and compares the result's blake3 with `[tools] gt2reloc.reference`. A mismatch refuses the build, naming both hashes. The result is cached per packer binary, under `.score64/tools/gt2reloc/<binary blake3>`, so the reference pack runs once for each binary, not on every build.

**6. Version identity (#27)** is the blake3 of three things, in canonical JSON as the cache uses:
- the track's resolved intent (`track.toml` with the project's values applied, plus `decisions.toml`);
- its sources' hashes;
- the tool version.

A change to `score64.toml` therefore makes a new version of every track that uses the changed values.

**7. Run history** (amended 2026-10-24, #27; parent, time, `uses` and branches amended again in #211) is a separate record from the version id, because a content-addressed id gives a restored intent its old id back.
- **Where.** One file per recorded run, in `tracks/<track>/history/`, committed with the track. The name is the run's UTC time, then the first 12 hex digits of its version, for example `2026-10-24T091502Z-1a2b3c4d5e6f.toml`. It sorts by time and holds no characters Windows refuses. Further runs in the same second, whatever their versions, get `-2`, `-3`, … up to `-9999`.
- **What it holds:**

  ```toml
  schema = 1
  version = "…"                 # the full version id (§6)
  parent = "…"                  # the version that was latest when it was recorded; absent for the first
  time = 2026-10-24T09:15:02Z   # UTC, whole seconds: when it ran, or the latest entry's time if that was later
  restore = true                # only on a restore
  [provenance]                  # who made the change, tagged by `by`, as decisions' provenance is
  by = "manual"                 # "rules", "llm" or "manual"
  # model = "…"                 # for "llm": the model id
  # name = "…"                  # for "manual": the person's git user.name
  [[uses]]                      # optional: the cache entries the version was made from and into
  stage = "transcribe"          # sorted by stage and key, no repeats
  key = "…"
  ```

  Run provenance has its own fields (`model` and `name`), unlike a decision's (`rule`, `prompt`), because it says who started a run rather than who proposed a change.

- **When.** A run is recorded only when its version differs from the track's latest recorded version, or when it restores an earlier intent. A restore is recorded even though its version was seen before, with `restore = true` and the current version as its parent. A re-run that gives the same version writes nothing, so the history only grows when the output can change.
- **Branches and races.** Two machines or two git branches can each add runs after the same version. Git merges the new files, which have different names, without conflict. So a history is a graph rather than one chain:
  - an entry's parent may be any version recorded in the folder;
  - versions repeat with restores, so following parents can come back to a version;
  - entries order by time, then their number in that second, then name, and the latest is the last;
  - recording reads the folder again first, and again if another writer takes its name.

  Two branches recording the same version in the same second can still clash on a name, which git reports as a conflict.
- **Clocks.** A run's recorded time is never earlier than the latest entry's, so a clock a little behind another machine's never makes a new run look older. If the latest entry is more than a day ahead of the run, a clock was wrong, and recording refuses, naming the entry, rather than pin every later run to its time.
- **`uses`** links a version to the cache entries it was made from and into. The link is in committed files, so the index (#28) can be rebuilt with it and GC (#29) keeps what it names.
- **Provenance** says what started the change. A person who edited the track or its decisions is `manual`, with their git `user.name`. A model decider is `llm`, with the model id. A re-run that only picks up new tool or rule versions is `rules`.
- **The index** (#28) is rebuilt from these files, and GC (#29) keeps whatever a recorded version refers to.

**8. `KeepNote`** (amended 2026-10-24, #167) is a decision kind in `decisions.toml`: `change = "keep_note"`, with `voice`, `row` (inside the decision's bar) and `pitch`.
- It keeps the note there that a rule would drop: the reject case of `DropNote`.
- It competes with a `DropNote` of the note it would keep, so the stronger decision wins as for every other kind: a person's `KeepNote` over the rules' drop, and a person's `DropNote` over a model's keep. It names no length, so any note that the drops leave on its row and pitch meets it, and it keeps the longest. A person can therefore keep a shorter repeat by keeping its row and pitch and dropping the longer ones, whatever their ids, and bring back a longer repeat a model dropped by dropping the shorter ones too. A `DropNote` names its note by length, so of twins of one length it takes one only, and the other meets the keep. Only when the drops take out every note there does the keep compete with them; since that depends on the notes, the reduction that applies them settles it (`docs/TAD.md` §3.2–§3.3). (clarified 2026-10-24 in #212's review)
- A `KeepNote` for a note that isn't there is reported as unapplied, as other overrides are.

## Options considered

### Option A: project file plus one file per track (chosen)
| Dimension | Assessment |
|---|---|
| Complexity | Two schemas, one override rule |
| Performance | Trivial |
| Maintenance | Each setting has one home. Target-wide settings change in one place |
| Fit | Matches how a target is built: one memory map and voice plan, many pieces of music |

### Option B: everything in each `track.toml`
| Dimension | Assessment |
|---|---|
| Complexity | One schema |
| Performance | Trivial |
| Maintenance | The zero page, player address and voice plan are copied into every track, and drift between them is a silent build bug |
| Fit | Poor for a target with more than one track |

### Option C: one `score64.toml` listing every track
| Dimension | Assessment |
|---|---|
| Complexity | One file, but it grows with every track |
| Performance | Trivial |
| Maintenance | Every track's edit touches one shared file: merge conflicts, and noisy diffs |
| Fit | Breaks the "one directory per track" layout that decisions and approvals already assume |

## Trade-off analysis

Option A costs one override rule, plus the need to show resolved values in the app and CLI so a person can see where a value came from. It removes the duplication in Option B, which matters most for the zero page and player address: a mismatch there corrupts the target program, and nothing in a single track would show it. Option C keeps one file but makes every track edit a shared-file edit, against ADR-004's aim of diffs that read cleanly.

## Consequences

- **Easier:**
  - #25 can finish;
  - #27–#30 have a track to key on;
  - #139 gets its setting;
  - the pack stage gets its options and pin;
  - ADR-013's provisional file can go.
- **Harder:**
  - the override rule must be shown wherever values are displayed;
  - moving a track between projects carries only its own file, so its new project's target-wide settings apply.
- **Revisit when:**
  - a project needs more than one target, for example two memory maps;
  - a second output format (SID Factory II) needs its own pack settings;
  - sources need a remote store beyond LFS; `path` plus `blake3` leaves room for one.

## Review answers (2026-10-23)

1. **One project is one target.** "Target" names what the music is for, a game, a demo or any other program, instead of "game".
2. **Per-track overrides** stay as proposed: `[target] video` and `[pack]` only.
3. **Source ranges:** yes, `start` and `end`, as exact-microsecond strings like `"0:12.500"`, not floats.
4. **Source audio** is found by path and identified by its blake3. Committing it, LFS or neither are all allowed. A missing or changed source is a clear error.
5. **Presets** live in `presets/` at the project root.
6. **The `gt2reloc` pin** is the blake3 of packing a reference fixture with reference options, which is portable across platforms. It is checked once per packer binary and cached by the binary's hash in `.score64/`.

## Amendment answers (2026-10-24)

1. **The memory window and raster budget** (#198) live in `[pack]`: `window = { start, end }`, page-aligned, replacing `player_page`, plus an optional `max_play_cycles`.
2. **Run history** (#27) is one file per run in `tracks/<id>/history/`. Each holds the version, its parent, the provenance (with the model or the git name) and the UTC time. A run is recorded only when the version changes, or on a restore.
3. **Rejecting a rule's drop** (#167) is a proposal kind, `KeepNote { voice, row, pitch }`, in `decisions.toml`.

## Action items

- [x] `score64_project::ProjectFile` and `TrackFile` with the shared conventions, plus a `Project` that loads both and resolves overrides (#25).
- [ ] Build the validator `Context` from the files; remove ADR-013's provisional file (#192).
- [ ] Take the pack options and the `gt2reloc` pin from `[pack]` and `[tools]`, with the reference pack cached per binary in `.score64/tools/` (pack stage, E7).
- [ ] `[pack] window` and `max_play_cycles` in `ProjectFile`, replacing `player_page`, and the budget gate fed from them (#198, #51).
- [x] Source checks: a missing file and a changed hash are errors naming the source (#25).
- [x] Version identity from the resolved intent (#27).
- [x] Run history in `tracks/<id>/history/` (#27, #211).
- [x] `KeepNote` in `decisions.toml` and applied by `monophonise_with` (#167).
- [x] TAD §4: the layout and both files (#25).
