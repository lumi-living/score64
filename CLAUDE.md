# Score64 — instructions for AI coding agents

Read this before touching the repo. The human-facing overview is `README.md`; the product is `docs/PDD.md`; the architecture is `docs/TAD.md`; decisions live in `docs/adr/`; hardware and file-format facts live in `docs/reference/`.

## What this is

A tool that turns music and sound into game-ready Commodore 64 SID assets. One Rust workspace: core (`crates/score64-core`, pure) + formats + project + analysis + adapters + CLI + MCP server + egui/eframe app (ADR-001), plus a separate GPL emulator helper (`crates/score64-emu`, ADR-002). Review-and-decide tool, **not** a tracker (ADR-003).

## Non-negotiables

1. Output must satisfy `docs/reference/`: the SID and player constraints, the GoatTracker formats and the memory budget. Never relax the validator to make a test pass; if the reference is wrong, fix it in the same PR with evidence (VICE, sidplayfp or hardware).
2. `score64-core` stays free of `std::fs`, network access, threads-by-default and platform code.
3. **Licence boundary:** `crates/score64-emu` is GPL-3.0-or-later and is only ever run as a separate process. No other crate may depend on it, and no GPL code may be copied into any other crate (CI job `licence-boundary`).
4. Every user-visible action is a command in the shared engine (ADR-005); the app, CLI and MCP server are thin front-ends.
5. Files are the source of truth (ADR-004): track files, decisions, approvals and `audio.lock` are text in git; artifacts live in the content-addressed cache; the index is rebuildable and never holds anything that is not also in files.
6. LLM decisions are advisory (ADR-008): structured output, validated by the core, recorded with provenance, overridable, and never written directly into song bytes.
7. Architecture, format or scope changes get an ADR (`docs/adr/ADR-000-template.md`); update `docs/TAD.md` in the same PR.
8. File writers match `docs/reference/` byte for byte; add a round-trip test for every format.

## Working method

- Check library APIs (`egui`/`eframe`, `rmcp`, `ort`, `clap`, `insta`, `symphonia`, `cpal`) against current documentation rather than memory; these move fast.
- Ports of existing code (GoatTracker file handling, Basic Pitch post-processing, feature extraction) get **differential tests** against the original on the same inputs.
- A packed file is only "done" once it plays correctly in VICE (`x64sc`) or sidplayfp, on both 6581 and 8580 models.

## Conventions

- Rust 2024 edition, `cargo fmt`, `clippy -D warnings`. Snapshot tests with `insta`; property tests with `proptest`.
- UI state lives in the app crate; the core never depends on egui.
- Commit messages: imperative, scoped (`core: keep voice 3 free of lead and bass in combat profile`).
- Issues: epics labelled `epic`, children are sub-issues and say "Part of #N"; milestones M0–M3.

Maintainers may keep personal tooling notes in `.claude/*.local.md` (git-ignored); nothing in the project depends on them.
