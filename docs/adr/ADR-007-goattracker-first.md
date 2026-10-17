# ADR-007: GoatTracker 2 as the first output target

**Status:** Accepted
**Date:** 2026-10-18
**Deciders:** @jahaa

## Context

Score64 needs an editable song format, a game-ready player with a sound-effect engine, and bar-synchronised subtune switching for adaptive music.

## Decision

Write GoatTracker 2 `.sng` songs and `.ins` instruments; pack with gt2reloc using sound-effect support and author info (timing marks). Produce GoatTracker sound-effect data for the resident effect block. The writers sit behind ports so other players or effect drivers can follow.

## Consequences
- Easier: documented formats; GoatTracker and SID Factory II both open `.sng`; built-in effect engine and timing marks.
- Harder: the effect format has no pulse modulation or filter; priority is fixed by memory address.
- Revisit when: an alternative effect driver (for example one with filter and pitch variation) proves better in a game.
