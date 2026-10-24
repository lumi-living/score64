# ADR-004: Stage pipeline with a content-addressed cache; files are the source of truth

**Status:** Accepted
**Date:** 2026-10-18
**Deciders:** @jahaa

## Context

Iteration is the core workflow: change a decision, re-render, compare. Paid transcription and LLM calls must not repeat. Versions must be reproducible and diffable, and the app needs fast queries.

## Decision

- Every stage is a pure function of (inputs, config, stage version); outputs are cached under a `blake3` hash in `.score64/cache/`.
- Intent (track files, decisions, presets, approvals, comments, `audio.lock`) is text in git.
- A version is an immutable run identified by intent + input hashes + tool version.
- The SQLite index is derived and rebuildable; nothing lives only there.

## Consequences
- Easier: incremental re-runs; reproducible builds; works without the app.
- Harder: cache garbage collection needs care (keep approved and locked versions, last N per asset).
  *Amended 2026-10-24 (#29):* roots are what recorded versions use; approvals and `audio.lock` add theirs; the rest goes oldest-first beyond a size cap. There is no "last N per asset": every recorded version's entries stay.
- Revisit when: multiple machines need to share a cache.
