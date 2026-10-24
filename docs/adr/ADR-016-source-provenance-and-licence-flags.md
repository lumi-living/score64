# ADR-016: Source provenance, commercial use and distribution obligations

**Status:** Proposed
**Date:** 2026-10-24 (reworked after review, 2026-10-24)
**Deciders:** @jahaa

## Context

Score64 starts from material people bring (PDD): their own recordings and compositions, stock libraries, commissioned work, and music a model or service generated. Its outputs also contain code it didn't write: every packed file embeds the GoatTracker 2 playroutine. Two different questions follow, and they need different answers:

- **May this be used commercially?** This is a question about the source audio and the tools that processed it:
  - a sample may be CC-BY-NC;
  - a generation service's terms may depend on the plan;
  - a transcription model may be released for non-commercial use only (PDD, Feasibility; TAD §2 already says "non-commercial models flagged").

  A "no" here means the target can't ship commercially with that material at all.
- **What does distributing it carry?** Attribution for a CC-BY sample, share-alike, or copyleft from code embedded in the output. These are obligations, not bans. The GPL, for example, doesn't forbid commercial use. It requires things such as offering the corresponding source. Refusing a build over an obligation would be wrong. Losing track of one would be wrong too.

**This is a real decision, not metadata.** It has to be settled before any target ships. A game released with music from a non-commercial sample, or with an obligation nobody recorded, has a problem that no later rebuild fixes.

Today nothing records any of this. `track.toml`'s `[[source]]` (ADR-014 §3) says where a file is and what it hashes to, but not where it came from or under which terms. A cache manifest (TAD §4) says how an artifact was made, but not what it may be used for or what it carries.

**What the evidence says about the embedded player.** It disagrees with itself:
- **Debian's copyright file for `goattracker`** ([stable_copyright](https://metadata.ftp-master.debian.org/changelogs/main/g/goattracker/stable_copyright)) puts `Files: *` under `GPL-2-or-later`, copyright Lasse Öörni. No stanza names `player.s`, and none records an exception for it. The file also lists the bundled 6510 assembler (`src/asm/`) under a non-commercial licence. That concerns distributing the tool itself, which Score64 doesn't do (THIRD_PARTY.md).
- **GoatTracker 2.77's own files** (`GoatTracker_2.77.zip` from the project's SourceForge page, read and not built):
  - `readme.txt` says the program is distributed under the GNU GPL and points to `copying`, which is the GPL version 2 text.
  - `gt2reloc.c` carries the usual "version 2 of the License, or (at your option) any later version" header.
  - `player.s` and `altplayer.s` each open with a notice of their own. It begins "This playroutine source code does not fall under the GPL license!" Then it allows use of the routine, and of song binaries made from it, for any purpose, commercial or noncommercial.
  - `greloc.c`, the relocator, has no licence header. It assembles `player.s` or `altplayer.s` into each packed file.
- **GoatTracker 2.72's `player.s`** (the local source tree, whose header says 2.68 playroutine) has the same notice, word for word.

So the packer is GPL-2.0-or-later, and the playroutine that packed files embed is outside the GPL by its author's own notice in the source. Debian's file doesn't record that notice. This ADR follows the source, and question 1 asks the maintainer to confirm that reading.

**SID Factory II's drivers** are a second case, for a future SF2 output (#188; `docs/research/2026-10-24-sid-factory-ii.md` §3).
- **What's known:** the repository (Chordian/sidfactory2) is GPL-2.0-or-later, and its drivers (11.xx, 12–16, np20, by Laxity and JCH) are 6502 binaries in it. The review found no separate licence or "free to use in productions" statement for them, unlike GoatTracker's `player.s`.
- **What follows:** a packed SF2 tune embeds driver code under the repository's licence unless its authors say otherwise. That makes commercial use `yes`, with the GPL's obligations: a source offer, a licence notice and share-alike.
- **Before relying on it:** for closed-source work, check with the driver authors first. A written grant can be recorded as point 5's project `LicenseRef-`.
- **The demo songs** in `SIDFactoryII/music/` have no stated licence, so they're never committed fixtures.

Constraints from earlier decisions:
- **Files are the source of truth** (ADR-004). Licence statements are intent, so they live in text files in git. The cache is derived.
- **Paid calls happen once** (ADR-004, TAD §2). Correcting a licence must not re-run a transcription.
- **The version id** covers what can change the output, and nothing else (ADR-014 §6, TAD §4).
- **One command engine** (ADR-005, ADR-012). A gate behaves the same in the CLI, the app and the MCP server, and decides failure as ADR-013 says.

## Decision

**1. Two flags, kept apart.** Every licence Score64 knows yields two things:
- **a commercial-use class:** `yes`, `no` or `unknown`. `--commercial` refuses `no` and `unknown` (point 6).
- **a set of distribution obligations,** each one of:
  - `attribution`: credit the work, with text where the licence gives it;
  - `share-alike`: adaptations go under the same licence;
  - `source-offer`: offer the corresponding source, as copyleft code requires;
  - `licence-notice`: ship the licence text or notice;
  - `unknown-terms`: the terms aren't known, so the obligations can't be listed.

  Obligations are recorded and reported, and never refused on.

**2. Per-source fields** in `track.toml` answer both questions for the source audio:

```toml
[[source]]
id = "mix"
path = "audio/main-theme.wav"
blake3 = "…"
kind = "mix"
provider = { kind = "library", name = "Example Sound Library", ref = "catalogue 1234" }  # required
licence = "CC-BY-4.0"                                  # required
attribution = "Main theme by A. Composer (CC BY 4.0)"  # optional
# from = ["mix"]                                       # optional, see below
```

- **`provider`** (required) says where the source came from.
  - `kind` is one of:
    - `own`: the person's or team's own recording or composition;
    - `commissioned`: made for the project by someone else;
    - `library`: a stock or sample library;
    - `model`: made by a model or service, generated from a prompt or processed from other sources;
    - `other`;
    - `unknown`.
  - `name` says who. It's required except for `own` and `unknown`.
  - `model` (optional) is the model or service version.
  - `ref` (optional) is the evidence: a catalogue number, an order or licence id, or a URL.
- **`licence`** (required) is one of three things:
  - an SPDX licence expression, such as `CC0-1.0`, `CC-BY-NC-4.0` or `CC-BY-NC-4.0 OR LicenseRef-bought-licence`;
  - a `LicenseRef-<name>` the project defines (point 5), which is where free-text terms go;
  - `unknown`.

  Anything else is an error, so a mistyped SPDX id can't pass as free text.
- **`attribution`** (optional) is the credit text an `attribution` obligation reports.
- **`from`** (optional) names sources this one was made from outside Score64, such as stems a service separated from the mix. Their flags join this one's.
- **Required means required.** A source without `provider` or `licence` is an error that suggests `licence = "unknown"`, so "not known" is written down rather than left out. Nothing is released, so `schema` stays 1, as ADR-013 changed exit codes before a release.
- **A licence belongs to content.** Two sources anywhere in the project with the same `blake3` must state the same provider and licence.

**3. Contributors and components** are declared by adapters, not by people:
- **A contributor** is a model or service whose output becomes an intermediate artifact:
  - separation;
  - transcription;
  - the beat model;
  - the model behind applied `llm` decisions (ADR-008).

  Its terms can affect commercial use (a non-commercial model) and add obligations. By default a model's terms reach its outputs, which is the cautious reading. The project can reclassify (point 5).
- **A component** is third-party code or data embedded in an output. Its licence matters mainly for its obligations.
  - The pack adapter declares one: the **GoatTracker 2 playroutine**, at the version its `gt2reloc` pin assembles, under the built-in `LicenseRef-goattracker2-playroutine`. That licence classes `yes` with no obligations, and its evidence is the notice at the top of `player.s` and `altplayer.s` (Context).
  - An SF2 output adapter (#188) declares the **SID Factory II driver** it embeds, such as Driver 11, at the version it was taken from, under `GPL-2.0-or-later`. Its obligations are reported in every SF2 pack report until the authors grant otherwise (Context).
  - Other vendored code that ever reaches an output, such as a different player, is declared the same way.
  - The packer itself (`gt2reloc`, GPL-2.0-or-later) is a tool. It isn't a component, since its own code isn't in the output.
- Each declaration is data next to the adapter's pin: a name, a version, a licence in a source's form, and a pointer to the evidence. Stages that only compute (the rules, instruments, render, score and report) add nothing.

**4. Carried forward in the cache manifests, combined per flag.** A manifest (TAD §4) gains a `provenance` block:

```json
"provenance": {
  "sources": [{ "blake3": "…", "provider": { "kind": "library", "name": "Example Sound Library" }, "licence": "CC-BY-4.0" }],
  "contributors": [{ "stage": "transcribe", "name": "Example transcriber", "model": "2.1", "licence": "Apache-2.0" }],
  "components": [{ "stage": "pack", "name": "GoatTracker 2 playroutine", "version": "2.73", "licence": "LicenseRef-goattracker2-playroutine" }]
}
```

- **Members accumulate.** An artifact's members are the union of its inputs' members and its own stage's contributors and components, sorted and deduplicated. Stems, transcriptions, the song, packed files, renders and reports all carry them.
- **Commercial use: the most restrictive wins.** The class is `no` if any member is `no`, otherwise `unknown` if any is `unknown`, otherwise `yes`.
  - `A OR B` is `yes` if either branch is, since the licensee may choose.
  - `A AND B` is `yes` only if both are.
  - `A WITH exception` takes the table's entry for the whole expression if it has one, and `A`'s otherwise.
- **Obligations: a set.** Every member's obligations, each tagged with the member it comes from, so a report can say "attribution: source `mix`" or "licence notice: component GoatTracker 2 playroutine". `A OR B` takes the obligations of the branch that decides the class, the lighter one when both are `yes`. `A AND B` takes both.
- **Only licence ids are stored.** The class and the obligations are worked out from the table and the project's overrides whenever they are needed, so a reclassification never leaves a stale verdict in the cache.
- **Not in the cache key.** Correcting a licence must never re-run a paid stage. The manifest's source licences are what was stated when the entry was made. The files win: checks and reports resolve each source's licence from the current track files by its `blake3`. The recorded licence matters only for an entry whose source no track states any more.
- The manifest `schema` goes up by one, and older entries are misses. No paid stage writes to the cache yet.

**5. The table: built in, plus project overrides.**
- **Built in** (`score64-project`), a small table of SPDX ids, each with a class and obligations:

  | Licences | Commercial use | Obligations |
  |---|---|---|
  | `CC0-1.0`, `Unlicense` | `yes` | none |
  | `CC-BY-3.0`, `CC-BY-4.0` | `yes` | attribution |
  | `CC-BY-SA-3.0`, `CC-BY-SA-4.0` | `yes` | attribution, share-alike |
  | `MIT`, `BSD-2-Clause`, `BSD-3-Clause`, `ISC`, `Apache-2.0` | `yes` | licence notice |
  | `GPL-2.0-or-later`, `GPL-3.0-or-later` | `yes` | source offer, licence notice, share-alike |
  | `CC-BY-NC-*`, `CC-BY-NC-SA-*`, `CC-BY-NC-ND-*` | `no` | attribution |
  | `CC-BY-ND-*` | `no`: every Score64 output is an adaptation, and ND doesn't allow sharing one | attribution |
  | `LicenseRef-goattracker2-playroutine` | `yes` | none |
  | any other SPDX id, and `unknown` | `unknown` | unknown terms |

- **Project overrides** go in `score64.toml`, since one project is one target (ADR-014):

  ```toml
  [licences."LicenseRef-example-library-standard"]
  name = "Example Sound Library standard licence"
  url = "https://example.com/licence"
  commercial = true
  obligations = ["attribution"]
  ```

  - Every `LicenseRef-` a source uses must be defined here. Adapters' built-in ones need not be.
  - An entry for an SPDX id or a built-in ref overrides it and must carry a `note` saying why.
  - **An author's permission is recorded the same way:** a project `LicenseRef-` with `commercial`, the obligations that remain (often none), a `note`, and a `url` or path to the written grant. A source or component then takes that ref (question 3). This turns an obligation, or an `unknown`, into a recorded grant.
  - Tracks can't override `[licences]`.

**6. `--commercial` refuses on commercial use only.**
- **Where:** the commands that write deliverables take `commercial: bool`: the `.sng` export, pack, and the build from `audio.lock`. It's an engine argument (ADR-005, ADR-012), so the CLI's `--commercial`, the MCP tool's `commercial: true` and the app's "Commercial build" switch behave alike.
- **With it:** the command resolves its members before running any stage, so nothing is paid for and nothing is written. Every member whose class is `no` or `unknown` is listed with its source id and track, or with its stage and name, and with its licence:

  ```text
  track main-theme: source `mix` is CC-BY-NC-4.0 (not for commercial use)
  track main-theme: transcribe contributor "Example transcriber" is LicenseRef-example-transcriber-terms (unknown)
  ```

  `fails` is true, so the CLI exits 1, as `validate` does with errors (ADR-013).
- **Never on obligations.** Obligations are listed in the output as information, with or without the flag. A GPL or attribution obligation never fails a build.
- **Without it:** the commercial-use list comes back as warnings, the artifacts are written with their members, and the exit code is 0.

**7. What the pack and export reports show.** Both the command's output and the HTML report (#63) show:
- **the commercial-use class,** with every member behind a `no` or `unknown`;
- **the obligations,** grouped by kind, each naming its member, with the attribution texts collected for credits;
- **the embedded components,** for pack, always. An SF2 pack names its driver and its GPL-2.0-or-later obligations the same way. Every pack report says so in one line, with the version and the evidence: "this file embeds the GoatTracker 2 playroutine (v2.73); its source says it is outside the GPL and free for any use". If the maintainer reads the evidence otherwise (question 1), this line and the table row change, and nothing else does.
- **the members of each source,** under the sources section.

The Budget view (UX §3) shows the same per asset.

**8. Interplay with the other records.**
- **Version id: licence fields stay out.** `provider`, `licence`, `attribution` and `from` are left out of the intent, as `path` is (TAD §4).
  - They don't change a byte of the output, and the version id covers exactly what does.
  - Including them would make a corrected typo a new version of the same music: a new history entry, stale approvals and stale `audio.lock` pins, and two tracks with the same music no longer sharing a version.
  - Whether something may ship, and what it carries, are facts about a build, checked from the current files.

  A different model or service does change the version, through the stage config already in the intent.
- **Run history (ADR-014 §7): no flags.** An entry is recorded only when the version changes, so a licence edit wouldn't make one, and flags stored there would go stale. Git's history of `track.toml` and `score64.toml` records when a statement changed and who changed it.
- **Approvals: no flags.** An approval says the music is right. The approval view shows the class and the obligations, but the gate is the build.
- **`audio.lock` (#82)** records each locked asset's licences, its class and its obligations, so a change shows in the lock's diff. A build whose current resolution differs is an error that asks for a re-lock, because the lock pins what ships, terms included.

## Options considered

### Option A: two flags from per-member licences, carried in manifests (recommended)
As in the decision.

| Dimension | Assessment |
|---|---|
| Complexity | Moderate: source fields, adapter declarations, a manifest block, a two-column table, one command argument |
| Performance | Trivial: a set union per stage, and a lookup before deliverable commands |
| Maintenance | Each statement lives once: a source's on the source, a model's or component's on its adapter. The cache carries what each artifact holds |
| Fit | Separates "may it ship commercially" from "what does shipping carry". Matches ADR-004 and ADR-014, and the gate is an engine argument (ADR-005) |

### Option B: one flag (the first draft)
One class per member, most restrictive wins, and `--commercial` refuses `no` and `unknown`.

| Dimension | Assessment |
|---|---|
| Complexity | Slightly lower: one column |
| Performance | Trivial |
| Maintenance | Obligations are either folded into the class or dropped |
| Fit | Poor. If copyleft counts against the class, a GPL component refuses every commercial pack, though the GPL allows commercial use. If it doesn't, the obligation goes unrecorded |

### Option C: flags only at export time
No manifest data. Deliverable commands read the track's sources and check them.

| Dimension | Assessment |
|---|---|
| Complexity | Lowest |
| Performance | Trivial |
| Maintenance | Contributors and components are invisible unless every export works out again which models and code made its inputs. Shared renders and reports carry no terms |
| Fit | Partial: it answers "may this ship?" but not "what was it made from, and what does it carry?" |

### Option D: a separate licence ledger
`licences.toml` at the project root maps source hashes, models and components to their terms.

| Dimension | Assessment |
|---|---|
| Complexity | Moderate: one more file and schema |
| Performance | Trivial |
| Maintenance | A second list of sources to keep in step with every `track.toml`. A moved track loses its terms |
| Fit | One place to audit, but it splits a source's description across two files, against ADR-014's one directory per track. It still needs A's manifest block |

## Trade-off analysis

B is what the first draft proposed. It's simpler, but it can't tell a ban from a duty, and that matters most for the one component every packed file has. C stops a bad commercial build but can't say which model made a stem or what code a packed file carries. D gathers statements into one file at the cost of a second source list, and it still needs A's carrying.

A costs two columns instead of one, and adapter declarations kept up to date. Its weak spot is that a manifest's recorded source licences can go stale. Checks and reports always resolve them from the current files, so a stale copy never decides a build.

## Consequences
- **Easier:**
  - a commercial target proves at build time that nothing it ships is non-commercial or of unknown terms;
  - every obligation it ships with is listed with its origin, attributions included;
  - any artifact, even one shared alone, says what it was made from and what it carries.
- **Harder:**
  - every source needs a statement, even `unknown`;
  - adapters must declare and maintain their models' and components' terms, with evidence;
  - the playroutine's terms rest on a notice in its source that Debian's file doesn't record, so the evidence has to be kept, and checked again when the `gt2reloc` pin moves.
- **Revisit when:**
  - presets (#49) come from third-party libraries, which makes them members too;
  - a second player target arrives, with its own component terms;
  - a target needs other classes, such as "broadcast" or "no AI training";
  - several projects share one cache.

## Questions for review
1. **The playroutine's terms.** The 2.77 and 2.72 sources say `player.s` and `altplayer.s` are outside the GPL and free for any use. Debian's file says GPL-2.0-or-later for every file. Should Score64 follow the source notice (proposed: `yes`, no obligations, evidence kept)? Or should it follow Debian's file (`yes`, with source-offer and licence-notice obligations) until the author confirms in writing?
2. **The `.sid` header.** Should the pack also write the embedded-component notice into the `.sid` header's author or copyright field (32 bytes each, PSID v2)? Or should the header stay the song's own, with the notice in the report only?
3. **An author's permission.** Is a project `LicenseRef-` with a `note` and a link to the written grant enough (proposed)? Or should a grant be its own table, naming the grantor, the date, the scope (which component or source, which targets) and the evidence file?
4. **Required fields.** Should a source without `provider` or `licence` be an error (proposed), or be read as `unknown` with a warning so existing track files stay valid?
5. **Model terms and outputs.** Should a model's or service's licence reach its outputs by default (proposed)? Or should each adapter state, per model, whether its terms cover outputs?

## Action items
- [ ] `track.toml`: `provider`, `licence`, `attribution` and `from` on `[[source]]`, with the required checks and one statement per hash (`score64-project`).
- [ ] `score64.toml`: `[licences]` with `commercial`, `obligations`, `note` and `url`, including recorded author permissions.
- [ ] The built-in table (class and obligations) and SPDX expression parsing. Check the `spdx` crate against its current documentation before using it.
- [ ] Manifest `provenance` (sources, contributors, components; licence ids only), written by `Cache::put` outside the key, with a manifest schema bump.
- [ ] Contributor declarations next to each adapter's pin: separate (#37), transcribe (#33, #34), grid (#35), the LLM decider (#76).
- [ ] When an SF2 output exists (#188): its adapter's component declaration for the SID Factory II driver it embeds (GPL-2.0-or-later, Laxity and JCH), and a question to the driver authors about closed-source use.
- [ ] The pack adapter's component declaration for the GoatTracker 2 playroutine. Record the evidence in `docs/reference/player.md`: the `player.s` and `altplayer.s` notices, Debian's file, and the versions checked. Re-check when the `gt2reloc` pin moves.
- [ ] `commercial` on the export, pack and build commands: a pre-run gate on the class only, obligations as information, and `fails` (ADR-013). Front-ends: CLI, MCP and app.
- [ ] Pack and export reports (#63) and the Budget view: class, obligations by member, attributions, and the embedded-component line.
- [ ] Version id: leave the new fields out of the intent, with a test that a licence edit keeps the id (#27).
- [ ] `audio.lock`: record each asset's licences, class and obligations, and compare them on build (#82).
- [x] THIRD_PARTY.md: the `gt2reloc` row names the packer's licence and the playroutine's notice, citing this ADR.
- [x] TAD §4: a note that this ADR is proposed. On acceptance, describe the fields, the manifest block, the table and the gate there.
