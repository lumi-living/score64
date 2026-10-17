# ADR-003: A review-and-decide tool, not a tracker

**Status:** Accepted
**Date:** 2026-10-18
**Deciders:** @jahaa

## Context

The work Score64 automates ends in a tracker song. It would be tempting to grow a note editor inside the app.

## Decision

Score64 decides, explains, compares and approves. It does not edit notes or patterns. Detailed edits happen in GoatTracker or SID Factory II; a hand-edited `.sng` re-imports as a new version with provenance `manual`, and its diff shows exactly what changed.

## Consequences
- Easier: a focused app; no competition with mature trackers.
- Harder: some fixes need a round-trip through a tracker.
- Revisit when: users repeatedly need one specific small edit that a decision override cannot express.
