# ADR-006: A tracker-grid score model as the core representation

**Status:** Accepted
**Date:** 2026-10-18
**Deciders:** @jahaa

## Context

MIDI-shaped representations (ticks, overlapping notes, channels) do not express what matters for the SID: rows, frames per row, monophonic voices, roles, arpeggio tables, timing marks and profiles.

## Decision

The core model is a tracker grid: notes on rows, one voice line per SID voice with a role, subtunes with frames-per-row tempo and loop rows, and timing-mark rows. Invariants are enforced by the validator and covered by property tests. MIDI is an input format only.

## Consequences
- Easier: validation, profiles and GoatTracker output are direct.
- Harder: importing expressive MIDI loses information by design; provenance keeps track of what was dropped.
