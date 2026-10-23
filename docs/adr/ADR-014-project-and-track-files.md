# ADR-014: Project and track files: `score64.toml` and `track.toml`

**Status:** Proposed
**Date:** 2026-10-23
**Deciders:** @jahaa

## Context

ADR-004 makes files the source of truth, and TAD §4 lists `track.toml` as a track's intent. No document says what that intent holds, so several pieces of work have stopped at the same gap:

- `decisions.toml` (#25) is done, but `track.toml` is not.
- Versions (#27) hash "intent + inputs + tool version" and need to know what intent is.
- The index (#28), cache garbage collection (#29) and per-artifact provenance (#30) all key off a track.
- The video-standard project setting (#139) has nowhere to live.
- The packer adapter (#50) takes its options from the caller, with "project config" still to come.
- ADR-013 added a provisional validation context file, to be replaced once the project files exist.

Some of this intent belongs to a track: its sources, subtunes and adaptive sets. Some belongs to the game the music is for, and every track shares it: the voice plan (profiles), where the player sits in memory, its zero page, and the pinned packer. Today the validation context mixes the two.

`decisions.toml` already set conventions that the other text files should share (#170 review):
- a `schema` key;
- deterministic writes;
- atomic replacement;
- errors located by line and column.

## Decision

**1. Two files, at two levels.** A project is a directory, normally a git repository:

```
score64.toml               the project: one game's shared settings
tracks/<track>/track.toml  a track: its sources, subtunes, adaptive sets and rule settings
tracks/<track>/decisions.toml
tracks/<track>/approvals.toml   (E4/E10, later)
audio.lock                      (E10, later)
.score64/                       cache and index, not committed
```

A track's id is its directory name, and must be a plain name as the cache defines it (TAD §4).

**2. `score64.toml` holds what every track of the game shares.**

```toml
schema = 1

[target]
video = "pal"                 # "pal" (default) or "ntsc"

# The game's voice plan: profiles in the shape of TAD §3.1, added to the built-in ones.
[[profile]]
name = "calm"
v1 = { usage = "music", roles = ["bed"], filter = true }
v2 = { usage = "effects" }
v3 = { usage = "effects" }

[pack]                        # gt2reloc (#50); these have no defaults
player_page = 0x10            # player at $1000
zero_page = 0xF8              # the player's two zero-page bytes, from the game's memory map
sound_effects = true
hard_restart_adsr = 0x0F00    # optional; gt2reloc's default

[tools]
gt2reloc = "2.73"             # the pin (#177 may make it a hash)

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
blake3 = "…"                  # the content the track was made from
kind = "mix"                  # "mix", "stem" or "midi"

[[subtune]]
name = "calm"                 # becomes the subtune's name in the song
profile = "calm"              # optional; the song's default profile otherwise
from = "mix"                  # a source id
# start = 12.5                optional time range in the source, in seconds
# end = 74.0

[[adaptive]]                  # TAD §3.1 shape
name = "score"
layers = ["calm", "tension"]
switch = { every_bars = 2 }   # optional; every bar by default

[rules.reduce]                # optional settings for one stage, owned and versioned by that stage
# min_note_rows = 1
```

- **`profile`** at the top level is optional and names the song's default profile. Otherwise the default is `full`.
- **Overrides:** a track may override `[target] video` and any `[pack]` key under its own `[target]` and `[pack]` tables. The project's values apply otherwise. Nothing else in `score64.toml` can be overridden per track, so the voice plan is one per game.
- **Instrument facts** don't belong in either file. They move to the presets (E7, ADR-013's plan), so they have one source.

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

**6. Version identity (#27)** is the blake3 of three things, in canonical JSON as the cache uses:
- the track's resolved intent (`track.toml` with the project's values applied, plus `decisions.toml`);
- its sources' hashes;
- the tool version.

A change to `score64.toml` therefore makes a new version of every track that uses the changed values.

## Options considered

### Option A: project file plus one file per track (chosen)
| Dimension | Assessment |
|---|---|
| Complexity | Two schemas, one override rule |
| Performance | Trivial |
| Maintenance | Each setting has one home. Game-wide settings change in one place |
| Fit | Matches how a game is built: one memory map and voice plan, many pieces of music |

### Option B: everything in each `track.toml`
| Dimension | Assessment |
|---|---|
| Complexity | One schema |
| Performance | Trivial |
| Maintenance | The zero page, player address and voice plan are copied into every track, and drift between them is a silent build bug |
| Fit | Poor for a game with more than one track |

### Option C: one `score64.toml` listing every track
| Dimension | Assessment |
|---|---|
| Complexity | One file, but it grows with every track |
| Performance | Trivial |
| Maintenance | Every track's edit touches one shared file: merge conflicts, and noisy diffs |
| Fit | Breaks the "one directory per track" layout that decisions and approvals already assume |

## Trade-off analysis

Option A costs one override rule, plus the need to show resolved values in the app and CLI so a person can see where a value came from. It removes the duplication in Option B, which matters most for the zero page and player address: a mismatch there corrupts the game, and nothing in a single track would show it. Option C keeps one file but makes every track edit a shared-file edit, against ADR-004's aim of diffs that read cleanly.

## Consequences

- **Easier:**
  - #25 can finish;
  - #27–#30 have a track to key on;
  - #139 gets its setting;
  - the pack stage gets its options and pin;
  - ADR-013's provisional file can go.
- **Harder:**
  - the override rule must be shown wherever values are displayed;
  - moving a track between projects carries only its own file, so its new project's game-wide settings apply.
- **Revisit when:**
  - a project needs more than one game target, for example two memory maps;
  - a second output format (SID Factory II) needs its own pack settings;
  - sources need storage outside git (LFS or a remote store), which `path` plus `blake3` already allows.

## Open questions for review

1. Is "one project = one game" right, or should `score64.toml` allow several targets?
2. Should the per-track overrides be just `[target] video` and `[pack]` as proposed, or none?
3. Should subtunes take a time range from their source (`start`/`end`), or always the whole source?
4. Should source audio be committed (or in LFS), or only hashed and found by path?

## Action items

- [ ] `score64_project::ProjectFile` and `TrackFile` with the shared conventions, plus a `Project` that loads both and resolves overrides (#25).
- [ ] Build the validator `Context` from the files; remove ADR-013's provisional file (#136 follow-up).
- [ ] Take the pack options and the `gt2reloc` pin from `[pack]` and `[tools]` (pack stage, E7).
- [ ] Version identity from the resolved intent (#27).
- [ ] TAD §4: the layout and both files.
