# ADR-016: Source provenance and licence flags

**Status:** Proposed
**Date:** 2026-10-24
**Deciders:** @jahaa

## Context

Score64 starts from material people bring (PDD): their own recordings and compositions, stock libraries, commissioned work, and music a model or service generated. Some of that material may not be used commercially. Some of the tools in the pipeline have terms of their own:
- a transcription model may be released for non-commercial use only (PDD, Feasibility; TAD §2 already says "non-commercial models flagged");
- a hosted service's terms depend on the plan;
- a separated stem carries the terms of both the mix and the separator.

Today nothing records any of this. `track.toml`'s `[[source]]` (ADR-014 §3) says where a file is and what it hashes to, but not where it came from or under which terms. A cache manifest (TAD §4) says how an artifact was made, but not what it may be used for. A target that ships commercially has no way to check that nothing it ships was made from non-commercial material.

Constraints from earlier decisions:
- **Files are the source of truth** (ADR-004). Licence statements are intent, so they live in text files in git. The cache is derived and rebuildable.
- **Paid calls happen once** (ADR-004, TAD §2). Correcting a licence must not re-run a transcription.
- **The version id** covers everything in the intent that can change the output, and nothing else (ADR-014 §6, TAD §4). Approvals, comments and `audio.lock` stay out for that reason.
- **One command engine** (ADR-005, ADR-012). A licence gate must behave the same in the CLI, the app and the MCP server, and decide failure the way ADR-013 says.

## Decision

**1. Every source states its provider and its licence.** `[[source]]` in `track.toml` gains these fields:

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
    - `model`: made by a model or service, whether generated from a prompt or processed from other sources;
    - `other`;
    - `unknown`.
  - `name` says who: the person, library or service. It's required for every kind except `own` and `unknown`.
  - `model` (optional, `model` kind only) is the model or service version.
  - `ref` (optional) is the evidence: a catalogue number, an order or licence id, or a URL.
- **`licence`** (required) is one of three things:
  - an SPDX licence expression, such as `CC0-1.0`, `CC-BY-NC-4.0` or `CC-BY-NC-4.0 OR LicenseRef-bought-licence`;
  - a `LicenseRef-<name>` that the project defines (point 4), for a licence SPDX doesn't list. This is where free-text terms go, with their name and URL;
  - `unknown`.

  Anything else is an error. That catches a mistyped SPDX id, which would otherwise pass as free text and be classed as unknown without anyone noticing.
- **`attribution`** (optional) is the credit text the licence asks for. Reports and builds list it.
- **`from`** (optional) names the sources this one was made from outside Score64, such as stems a service separated from the track's mix. Their flags join this source's own.
- **Required means required.** A source without `provider` or `licence` is an error that names the source and suggests `licence = "unknown"`. "Not known" is then something a person wrote down, not something left out. Nothing has been released, so `schema` stays 1, as ADR-013 changed exit codes before a release.
- **A licence belongs to content.** Two `[[source]]` entries anywhere in the project with the same `blake3` must state the same provider and licence. Otherwise it's an error naming both tracks.

**2. Stages add their own provider and terms when a model, a service or third-party code shapes their output.** These are a stage's **contributors**:
- the separation model or service (separate);
- the transcription model or service (transcribe);
- the beat model (grid);
- the model behind applied `llm` decisions (reduce, ADR-008);
- the player routine that the packer puts into every packed file (pack).

Each adapter declares its contributor as data next to its model pin: a name, the model version, and a licence in the same form as a source's, with a link to the evidence. A hosted service whose terms depend on the plan declares a `LicenseRef-`, which is `unknown` until the project classes it. By default a model's terms reach its outputs. That's the cautious reading, and the project can override it (point 4). Stages that only compute (the rules, instruments, render, score and report) add nothing.

**3. Flags are carried forward as a set, in the cache manifests.** A manifest (TAD §4) gains a `provenance` block:

```json
"provenance": {
  "sources": [
    { "blake3": "…", "provider": { "kind": "library", "name": "Example Sound Library" }, "licence": "CC-BY-4.0" }
  ],
  "contributors": [
    { "stage": "transcribe", "name": "Example transcriber", "model": "2.1", "licence": "Apache-2.0" }
  ]
}
```

- **An artifact's flags are the union** of its inputs' flags and its own stage's contributors. Entries are sorted and deduplicated. Stems, transcriptions, the song, packed files, renders and reports all carry them, so any artifact can say what it was made from and under which terms.
- **The set is never collapsed.** For a verdict, the commercial class of an artifact is the most restrictive class in its set: `no` if any member is `no`, otherwise `unknown` if any is `unknown`, otherwise `yes`. Keeping the set lets every refusal and warning name each source or contributor behind it.
- **SPDX expressions** are classed as SPDX defines them. `A OR B` is `yes` if either branch is, since the licensee may choose. `A AND B` is `yes` only if both are. `A WITH exception` takes the table's class for the whole expression if it has one, and `A`'s otherwise.
- **Not in the cache key.** Correcting a licence must never re-run a paid stage. The manifest's `sources` terms are therefore what was stated when the entry was made. The files win: every check and every output resolves each source's terms from the current track files by its `blake3`. The recorded terms only matter for an entry whose source no track states any more, and then they are the last terms known. The entry's lineage (which sources) and its contributors never change for an entry, since they follow from its inputs and its stage config.
- The manifest `schema` goes up by one, and older entries are misses. No paid stage writes to the cache yet.

**4. Classification: a built-in table plus project overrides.**
- **Built in** (`score64-project`), a small table of SPDX ids:
  - `yes`: `CC0-1.0`, `CC-BY-3.0`, `CC-BY-4.0`, `CC-BY-SA-3.0`, `CC-BY-SA-4.0`, `MIT`, `Apache-2.0`, `BSD-2-Clause`, `BSD-3-Clause`, `ISC`, `Unlicense`.
  - `no`: every `CC-BY-NC-*`, `CC-BY-NC-SA-*` and `CC-BY-NC-ND-*`, and `CC-BY-ND-*`. Every Score64 output is an adaptation, and ND licences don't allow sharing one.
  - Any other valid SPDX id, and `unknown`, is `unknown`.
  - Obligations such as attribution and share-alike don't change the class. Reports and builds list them.
- **Project overrides** go in `score64.toml`, because one project is one target (ADR-014), and whether it ships commercially is a target fact:

  ```toml
  [licences."LicenseRef-example-library-standard"]
  name = "Example Sound Library standard licence"
  url = "https://example.com/licence"
  commercial = true

  [licences."LicenseRef-example-transcriber-terms"]   # a contributor's terms, once the plan is known
  name = "Example transcription service, paid plan"
  commercial = true
  ```

  - Every `LicenseRef-` that a source or adapter uses must be defined here, apart from adapters' built-in ones.
  - An entry for an SPDX id overrides the built-in class and must carry a `note` saying why.
  - Tracks can't override `[licences]`. ADR-014's per-track overrides stay `[target] video` and `[pack]`.

**5. `--commercial` refuses; without it, Score64 warns and records.**
- **Where.** The commands that write deliverables take `commercial: bool` in their arguments: the `.sng` export, pack, and the build from `audio.lock`. It's an engine argument (ADR-005, ADR-012), so the CLI's `--commercial`, the MCP tool's `commercial: true` and the app's "Commercial build" switch behave alike.
- **With it.** The command resolves the flags of everything it would write before it runs any stage, so nothing is paid for and nothing is written. Every member whose class is `no` or `unknown` is listed with the source id, the track, the licence and the class, or with the contributor, its stage and its licence:

  ```text
  track main-theme: source `mix` is CC-BY-NC-4.0 (not for commercial use)
  track main-theme: transcribe contributor "Example transcriber" is LicenseRef-example-transcriber-terms (unknown)
  ```

  The command's `fails` is true, so the CLI exits 1, as `validate` does with errors (ADR-013). The output keeps the list as data for the MCP server and the app.
- **Without it.** The same list comes back as warnings. The artifacts are written and carry their flags, and the report shows them. The exit code is 0.

**6. Interplay with the other records.**
- **Version id: unchanged by licence fields.** `provider`, `licence`, `attribution` and `from` are left out of the intent, as `path` is (TAD §4). The argument:
  - they don't change a byte of the output, and the version id covers exactly what does;
  - including them would make a corrected typo a new version of the same music. It would add a history entry, invalidate approvals and `audio.lock` pins, and stop two tracks with the same music from sharing a version;
  - whether something may ship commercially is a fact about a build. It is checked at build time from the current files, and a version doesn't need to carry it.

  A different model or service does change the version, through the stage config already in the intent.
- **Run history (ADR-014 §7): no flags.** A history entry is recorded only when the version changes, so a licence edit wouldn't make one, and flags stored there would go stale. A version's flags are resolved from its lineage and the current files. When a licence statement changed, and who changed it, is in git's history of `track.toml` and `score64.toml`.
- **Approvals: no flags.** An approval says the music is right, and a licence fix doesn't undo that. The approval view shows each asset's class, so a person sees an `unknown` before approving, but the gate is the build.
- **`audio.lock` (#82): records the terms it was locked with.** Each locked asset lists its sources' and contributors' licences and its class, so the lock is self-describing and a licence change shows in its diff. A build compares them with the current resolution. A difference is an error that asks for a re-lock, because the lock pins what ships, terms included. With `--commercial`, the build then applies point 5 as well.
- **Reports and the Budget view** (UX §3) list each artifact's sources, contributors, licences, class and attributions.

## Options considered

### Option A: per-source records, flags in manifests (recommended)
As in the decision.

| Dimension | Assessment |
|---|---|
| Complexity | Moderate: new source fields, a manifest block, a small table, one command argument |
| Performance | Trivial: a set union per stage, and a lookup before deliverable commands |
| Maintenance | Terms live once, on the source in its track file, and contributors once, on the adapter. The cache carries what each artifact descends from |
| Fit | Matches ADR-004 (statements in git, the cache derived) and ADR-014 (a track carries its own sources). The gate is an engine argument, as ADR-005 asks |

### Option B: flags only at export time
No manifest data. A deliverable command reads the track's sources, checks their licences and refuses or warns.

| Dimension | Assessment |
|---|---|
| Complexity | Lowest: one check in a few commands |
| Performance | Trivial |
| Maintenance | Contributors are invisible unless every export works out again which models made each stage's input. Renders and reports shared for review carry no terms |
| Fit | Partial: it answers "may this ship?" but not "what was this made from?", and that question is the one the issue asks |

### Option C: a separate licence ledger
`licences.toml` at the project root maps source hashes and models to their terms.

| Dimension | Assessment |
|---|---|
| Complexity | Moderate: one more file and its schema |
| Performance | Trivial |
| Maintenance | A second list of sources that has to stay in step with every `track.toml`, with entries that are orphaned or missing. A track moved to another project loses its terms |
| Fit | One place to audit, but it splits a source's description across two files, against ADR-014's one directory per track. Artifacts still need flags carried forward, so it replaces only point 1 |

### Option D: licence fields in the cache key and the version id
Flags are fresh by construction: change a licence, and everything downstream is a new entry and a new version.

| Dimension | Assessment |
|---|---|
| Complexity | Low in the cache, high in what follows from it |
| Performance | Poor: correcting a licence re-runs every stage, paid transcription included |
| Maintenance | A typo fix becomes a new version, with a history entry, stale approvals and a stale `audio.lock` for the same music |
| Fit | Breaks "paid calls happen once" (ADR-004) and the version rule that only what changes the output counts (ADR-014 §6) |

## Trade-off analysis

B is the cheapest way to block a commercial build. But it can't say which model made a stem or a transcription, and the risk sits there as much as in the sources. C gathers statements into one auditable file at the cost of a second list of sources to keep in step, and it still needs A's manifest block. D buys freshness with re-runs and new versions for no change in sound.

A keeps each statement in one file and carries a derived set through the cache. Its one cost is that a manifest's recorded terms can go stale. Every check and output resolves terms from the current files, so a stale copy is never what decides a build, and the lineage it is resolved through never changes.

## Consequences
- **Easier:**
  - a commercial target can prove, at build time, that nothing it ships came from non-commercial or unknown material;
  - any artifact, even one shared alone, says what it was made from and under which terms;
  - attributions are collected for credits.
- **Harder:**
  - every source needs a licence statement, even if it's `unknown`;
  - each adapter has to declare and keep up to date the terms of the model or service it uses;
  - the player routine's terms have to be settled with evidence, or every packed file is `unknown`.
- **Revisit when:**
  - presets (#49) come from third-party libraries, which makes them contributors too;
  - a target needs classes other than commercial use, such as "broadcast" or "no AI training";
  - several projects share one cache, since the recorded terms then come from different projects.

## Questions for review
1. **Required fields.** Should a source without `provider` or `licence` be an error (proposed), or be read as `unknown` with a warning, so that existing track files stay valid?
2. **Model terms and outputs.** Should a model's or service's licence reach its outputs by default (proposed, the cautious reading), with the project overriding it in `[licences]`? Or should adapters state, per model, whether its terms cover outputs?
3. **Commercial by default.** Should `score64.toml` be able to say `[target] commercial = true`, so that a commercial target is never built without the check? Or is the per-run flag enough?
4. **Free-text licences.** Only as project-defined `LicenseRef-` entries in `score64.toml` (proposed, shared by every track that uses a library), or also as inline text on a source?
5. **A changed `audio.lock`.** When the current terms differ from those locked, is the build an error that asks for a re-lock (proposed), or a warning unless `--commercial` is set?

## Action items
- [ ] `track.toml`: `provider`, `licence`, `attribution` and `from` on `[[source]]`, with the required checks and the rule that one hash has one statement (`score64-project`).
- [ ] `score64.toml`: `[licences]` with `LicenseRef-` definitions and notes on SPDX overrides.
- [ ] The built-in class table and SPDX expression parsing. Check the `spdx` crate against its current documentation before using it.
- [ ] Manifest `provenance` (sources and contributors), written by `Cache::put` outside the key, with a manifest schema bump.
- [ ] Contributor declarations next to each adapter's model pin: separate (#37), transcribe (#33, #34), grid (#35), and the LLM decider (#76).
- [ ] The GoatTracker player routine's terms in `docs/reference/player.md`, with evidence, before `--commercial` ships.
- [ ] `commercial` on the export, pack and build commands, with the pre-run gate, warnings, and `fails` (ADR-013). Front-ends: CLI, MCP and app.
- [ ] Version id: leave the new fields out of the intent, with a test that a licence edit keeps the id (#27).
- [ ] `audio.lock`: record each asset's terms and compare them on build (#82).
- [ ] Report (#63) and Budget view: sources, contributors, licences, class and attributions.
- [x] TAD §4: a note that this ADR is proposed. On acceptance, describe the fields, the manifest block and the gate there.
