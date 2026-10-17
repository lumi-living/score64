# ADR-002: MIT, with the SID emulator in a separate GPL helper binary

**Status:** Accepted
**Date:** 2026-10-18
**Deciders:** @jahaa

## Context

Score64 is open source under Lumi Living, like Convert64 (MIT). Accurate SID emulation (reSID, reSIDfp, `resid-rs`) is GPL-licensed; linking it would make the distributed binary GPL.

## Decision

The workspace is MIT. `crates/score64-emu` is a **GPL-3.0-or-later binary** that contains the 6502 core and SID emulation. Other crates talk to it only as a separate process over stdin/stdout (versioned protocol, TAD §5.1). No crate depends on it as a library and no GPL code is copied elsewhere; CI's `licence-boundary` job enforces the first rule.

## Options considered

### Option A: GPL-3.0 for everything
| Dimension | Assessment |
|---|---|
| Complexity | Simplest: emulator linked in-process |
| Fit | Breaks licence symmetry with Convert64; blocks reuse of Score64 code in MIT projects |

### Option B: MIT + GPL helper (chosen)
| Dimension | Assessment |
|---|---|
| Complexity | A process boundary and a small protocol; live audition streams PCM from the helper |
| Fit | Family stays MIT; the GPL part is small and clearly bounded |

## Consequences
- Easier: shared MIT crates with Convert64; clear contributor rules.
- Harder: live audition latency must be managed across the pipe (buffering, small render chunks).
- Revisit when: an MIT-compatible emulator of sufficient accuracy exists.
