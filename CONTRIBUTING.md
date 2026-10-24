# Contributing to Score64

Thanks for looking. The project is early; the most useful help right now is **rights-clear audio fixtures** (see below), corrections to the reference documents, and reports of packed files that misbehave on real hardware.

## Ground rules

- **Scope:** Score64 is a review-and-decide tool, not a tracker. Features that make arrangement decisions visible, comparable and overridable are welcome; a note editor will be declined (see [ADR-003](docs/adr/ADR-003-review-tool-not-tracker.md)).
- **Hardware truth wins.** Output must play correctly with the target player within the limits in [`docs/reference/`](docs/reference/README.md). If the validator and your ears disagree, the validator is right until proven otherwise with a VICE, sidplayfp or hardware check — then fix the reference and the validator together.
- **Core stays pure.** `score64-core` has no `std::fs`, no network, no platform code.
- **Licence boundary.** Everything is MIT except `crates/score64-emu` (GPL-3.0-or-later), which runs as a separate process. Do not depend on it from another crate and do not copy GPL code anywhere else ([ADR-002](docs/adr/ADR-002-mit-with-gpl-emulator-helper.md)).
- **Dependencies.** A new crate dependency must be MIT, Apache-2.0, BSD, zlib, public domain, CC0 or the Unlicense, with every transitive one too. It should be maintained, with its version pinned in the workspace `Cargo.toml`. Anything else needs a maintainer's OK first.
- **Decisions are recorded.** Anything that changes architecture, formats or scope gets an ADR in `docs/adr/` (copy `ADR-000-template.md`).

## Workflow

1. Open or pick an issue. Epics are labelled `epic`; pick a child issue, not the epic.
2. Branch from `main`: `feat/<short-name>`, `fix/<short-name>`, `docs/<short-name>`.
3. `cargo fmt`, `cargo clippy --workspace -- -D warnings` and `cargo test --workspace` must pass.
4. Changes that alter generated songs, renders or packed files update the relevant snapshot or golden file and say *why* in the PR.
5. PR against `main`; fill in the template. One reviewer approval; squash-merge.

## Audio fixtures

We need source audio and MIDI whose rights allow redistribution under MIT: your own recordings or compositions, CC0 material, or explicitly permitted files. Put them in `fixtures/<name>/` with a `README.md` stating origin and licence. Useful kinds: a sparse melody-plus-bass track, a dense mix, a drum loop, an ambience bed, and single sound-effect references (footsteps, a gunshot, a metallic clink).

Every file in the repository must be MIT-compatible, test data included. AI-generated audio is only accepted if the generator's terms allow redistribution, and the README must say which service and plan produced it.
