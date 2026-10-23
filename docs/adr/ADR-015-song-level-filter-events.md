# ADR-015: Song-level filter events in the score

**Status:** Proposed
**Date:** 2026-10-23
**Deciders:** @jahaa

## Context

The SID has one filter, shared by the voices routed through it. Today a score can filter a voice only through its instrument: a preset's filtertable program starts when one of its notes does. An arrangement often needs filter changes that belong to the song instead. The fixture's `combat` subtune filters voices 1 and 2 (low-pass, resonance 2, cutoff `$38`) with pattern command `AXY`, to make room for sound effects on the dry voice 3. The score can't say that, so the export plays `combat` unfiltered.

#187 measured the result in the emulator helper. On the 6581 the music comes out 4 dB louder, and the effects lose about 3 dB of headroom, which was audible. Carrying the same filter on the lead instrument matched the original within 0.5 dB. That works only where one instrument can own the song's filter.

What GoatTracker offers (readme, §3.4.3 and commands A–C):
- **`AXY`** points the filtertable at a program. The program runs every frame, whatever the voices do.
- **A program's rows** set the passband, resonance and routing (left `$80`–`$F0`; right: resonance in the high nybble, a voice bitmask in the low one), set the cutoff (left `$00`), sweep (left `$01`–`$7F`: time and signed speed), or jump or stop (left `$FF`).
- **`BXY`** sets resonance and routing directly, and `$00` turns the filter off. **`CXY`** sets the cutoff directly.
- **Who wins:** the last pointer written. An instrument with a filtertable position sets the pointer at its note's start; value `$00` leaves the filter alone.

## Decision

Recommended, pending review: **Option B.**

A subtune gets `filter: Vec<FilterEvent>`, each event `{ row, program: PresetId, voices: [Voice] }`:
- `program` names a **filter program preset**, a GoatTracker filtertable program kept in `presets/` like instruments (ADR-014), so a target reuses its programs;
- `voices` are the voices routed through the filter from that row on. An empty list turns the filter off.

**Validator, rule 4:**
- routing an expendable or effects-only voice is an error, as for instruments' filters today;
- so is an event on a row where an instrument of one of its voices also starts a filter program, because which one wins would depend on the order of writes;
- the routing is in force until the next event, or the end of the subtune.

**Export:**
- each program is copied into the song's filtertable, relocated like instruments' tables;
- its "set filter parameters" rows get the event's voice bitmask, so the score, not the program, decides the routing;
- each event becomes `AXY` on its row, in a free command slot, with the same slot rules as tempo changes and timing marks;
- an event with no voices becomes `B00`.

**Who wins, documented in `docs/reference/player.md` with evidence:** the last pointer written. A song event therefore holds until a note whose instrument has a filtertable position starts on a routed voice. Rule 4's check keeps that from happening silently.

## Options considered

### Option A: filter events with Score64's own program description
`FilterEvent { row, passband, resonance, cutoff, sweep: Option<Sweep>, voices }`, which the export turns into a filtertable program.

| Dimension | Assessment |
|---|---|
| Complexity | Higher: a program model, its validation, and a compiler to filtertable rows |
| Performance | Trivial |
| Maintenance | A second description of what GoatTracker programs already express |
| Fit | Player-neutral on paper, but sweeps only make sense in a tracker's terms |

### Option B: filter events that name a program preset (recommended)
As in the decision.

| Dimension | Assessment |
|---|---|
| Complexity | Lower: programs are `.ins`-style table data, which the export already relocates |
| Performance | Trivial |
| Maintenance | One format for programs; presets are reused across tracks |
| Fit | Matches how GoatTracker and the fixture work; routing stays in the score, where the validator can see it |

### Option C: keep the filter on instruments only
A song filter means giving one instrument the program.

| Dimension | Assessment |
|---|---|
| Complexity | None |
| Performance | Trivial |
| Maintenance | None |
| Fit | Poor: it fails when no single instrument plays throughout, or when several instruments share a filtered voice, and the routing is hidden in table data |

## Trade-off analysis

A is the most general, but it rebuilds GoatTracker's filtertable in a new shape. Without a second player target, nothing is gained for the extra validation and compilation. B keeps programs as the table data the export already handles, and adds only what the validator needs to see: the row and the routing. C is what we have, and #187's measurements show where it falls short.

## Consequences
- **Easier:**
  - arrangements that free a voice for effects by filtering the others;
  - filter sweeps across bars that don't depend on which instrument plays;
  - matching the fixture's `combat`.
- **Harder:**
  - one more kind of preset;
  - a new rule-4 case;
  - the score format gains a field, so song files with filter events need a Score64 that reads them.
- **Revisit when:** a second player target arrives (Option A's description might then pay off), or presets (#49) settle a different home for table data.

## Questions for review
1. Option B as described, or A?
2. Should the score's routing replace the program's routing nybble, as proposed, or should programs be routing-free and the export add a parameters row?
3. Should an instrument's filtertable position on a routed voice be an error (proposed), a warning, or allowed with "last write wins" documented?

## Action items
- [ ] Score model: `Subtune::filter` and `FilterEvent`, with a schema update (core).
- [ ] Rule 4: routing per profile, and the conflict with instruments' filter programs.
- [ ] Filter program presets in `presets/` (with #49).
- [ ] Export: program relocation, routing, `AXY` and `B00` placement; `docs/reference/player.md` on precedence, with a render as evidence.
- [ ] End-to-end test: `combat` with the filter as a score event matches the original fixture's effect margin on the 6581 to within 1 dB (#187).
- [ ] Listening on 6581 and 8580, with effects.
