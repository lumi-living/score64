# Spike: a gt2reloc-packed tune in score64-emu, compared with VICE and sidplayfp

**Issue:** #20 (part of E3, #17) · **Date:** 2026-10-18 · **Core:** `emulator_6502` 1.1.0 (ADR-010) · **SID:** `resid-rs` 1.1.1

## Question

Can `score64-emu` run a real GoatTracker packed player (init, then play at 50 Hz) on our 6502 core and `resid-rs`, and how far are its renders from a reference player on both SID models? Is `resid-rs` enough, or do we need reSIDfp through FFI?

## Answer

- **The CPU and harness side is exact.** Over 60 s (3,007 frames), the helper's SID writes match VICE's cycle-exact 6510 write for write. Every frame has the same registers and values in the same order, and every write sits on the same cycle relative to the frame's first write, gate-off and ADSR (hard-restart) writes included.
- **The audio gap to sidplayfp is the SID engine, not the CPU.** Playing VICE's exact write log through our own SID gives the same distance to sidplayfp as the helper does (table below). sidplayfp uses reSIDfp, and `resid-rs` is a port of the older reSID, so the gap comes from the different filter and waveform models.
- **Recommendation:** keep `resid-rs` while M1 gets going: the pipeline works and the part we own is proven exact. Before renders are what people judge by ear (E8, #54), move the default engine to reSIDfp through FFI. `libresidfp` is now a standalone C++ library (1.2.3), which makes that practical. Write the change as an ADR. Two reasons: reSIDfp models the 6581 filter and combined waveforms more accurately, and running the same engine as the oracle turns #59 from a loose feature-tolerance test into a tight one.

## Setup

| Item | Version |
|---|---|
| gt2reloc | GoatTracker 2.77 (`GoatTracker_2.77.zip`, SHA-256 `96c2bd6a…61ef`), built from source |
| sidplayfp | 3.2.2 (`af84346b…953d8b`) on libsidplayfp 3.1.1 (`12b79190…c626`) with libresidfp 1.2.3 (`e9b53a98…2f76d`), built from source. The Homebrew libsidplayfp 3.1.1 bottle ships only the `sidlite` engine |
| VICE | 3.10 (Homebrew), `vsid` with reSID |
| Test tune | "On a Sanction from CIA" (Cadaver), a GoatTracker 2.77 example. Used locally and **not committed**; rights-clear fixtures come with #18 |
| Packing | `gt2reloc sanction.sng sanction_d1h1.sid -D1 -H1 -ZF8` (2,523 bytes; player at `$1000`, zero page `$F8/$F9`). Also without `-D1 -H1` (2,272 bytes) for the opcode log |
| Renders | 60 s, 48 kHz mono, PAL. sidplayfp `-mof` / `-mnf` (forced 6581 / 8580), `--delay=0` |

Reproduce with `crates/score64-emu/spikes/packed-player/run.sh <song.sng> scratch/<dir> [effect.bin]`.

## Register writes: helper vs VICE

VICE's `-sounddev dump` logs every SID write with the cycles since the previous one. Its 6510, VIC and CIA are cycle-exact. The PSID driver calls play from a raster interrupt.

| Measure | Value |
|---|---|
| Frames compared | 3,007 (VICE's first burst, the PSID driver setting the volume, skipped) |
| Frames with identical register/value sequence | **3,007 / 3,007** |
| Intra-frame timing, all 72,148 writes (helper − VICE) | **0 cycles, 100 % exact** |
| … gate (control register) writes | 0 cycles, 9,021 / 9,021 exact |
| … ADSR writes | 0 cycles, 18,036 / 18,036 exact |
| VICE play-call period | 19,240–19,719 cycles, mean 19,655.88; the helper's is exactly 19,656 |

The only timing difference is *when play starts*. On a C64, interrupt latency and the work the IRQ does first make the play call drift by a few hundred cycles from frame to frame. The helper calls play exactly once per 19,656 cycles (TAD §5.1). The drift is the reason "helper vs VICE timing" below is not bit-identical.

## Audio

Each pair is aligned, then level-matched: least squares when both use the same SID engine, RMS when they differ. Waveforms from different engines don't correlate, because their oscillators start at different phases, so the envelope and spectral measures are the ones that mean anything there. "VICE timing" is VICE's write log played through the helper's `resid-rs` (`score64-emu replay`).

| Chip | Pair | Lag (ms) | Gain (dB) | Waveform r | Envelope r | Envelope \|Δ\| (dB) | Log-spectral distance (dB) | Centroid r |
|---|---|---|---|---|---|---|---|---|
| 6581 | helper vs VICE timing (same engine) | 0.00 | 0.0 | 0.995 | 0.970 | 0.07 | 2.51 | 0.999 |
| 6581 | helper vs sidplayfp | −9.00 | −3.1 | 0.087 | 0.737 | 1.15 | 6.23 | 0.867 |
| 6581 | VICE timing vs sidplayfp (engine only) | −9.00 | −3.1 | 0.086 | 0.761 | 1.14 | 6.12 | 0.867 |
| 8580 | helper vs VICE timing (same engine) | 0.00 | 0.0 | 0.995 | 1.000 | 0.05 | 2.92 | 0.999 |
| 8580 | helper vs sidplayfp | −9.00 | −3.0 | 0.090 | 0.862 | 0.79 | 6.06 | 0.901 |
| 8580 | VICE timing vs sidplayfp (engine only) | −9.00 | −3.0 | 0.089 | 0.862 | 0.79 | 5.89 | 0.901 |

How to read it:

- **Helper vs VICE timing** shows the harness's simplification, play called at an exact frame period, costs almost nothing: 0.995 waveform correlation and 0.07 dB envelope difference.
- **Helper vs sidplayfp** and **VICE timing vs sidplayfp** are within 0.03 on every correlation and within 0.2 dB on every level and distance. Whatever separates us from sidplayfp happens after the register writes, in the SID engine.
- The engine gap is larger on the 6581 (envelope r 0.74 against 0.86 on the 8580), which fits reSIDfp's very different 6581 filter model. sidplayfp is about 3 dB quieter, and its output starts 9 ms later (the PSID driver's start-up).

Not measured: `resid-rs` against C++ reSID audio. VICE's WAV device never writes its data when `-limitcycles` or a signal stops a headless run, so that pair is missing. The register log above was the more important half.

## Opcodes the player runs

| Build | Distinct opcodes | Undocumented | Instructions run (60 s) |
|---|---|---|---|
| `-D1 -H1`, no effects | 49 | 0 | 658,719 |
| `-D1 -H1`, gun effect on voice 3 every 37 frames (81 effects) | 51 (adds `STY abs`, `CPY #`) | 0 | 681,943 |
| plain (no `-D1`) | 49 | 0 | 584,072 |

The same holds for a second example tune ("The Consultant"): 46 opcodes, none of them undocumented, with or without effects. The effect path only runs when an effect is triggered, so the helper's `render --effect` calls the player's sound-effect entry (start+6, `A`/`Y` = data address, `X` = 0/7/14) before play, as a game would. The documented-only assumption in ADR-010 holds for GoatTracker 2.77's player.

Play routine cost (the helper counts it exactly): mean 803, max 1,071 cycles (17.0 raster lines) with `-D1`. With effects active the max is 1,096 (17.4 lines). Without `-D1` the max is 943 (15.0 lines).

## Corrections found on the way

- `emulator_6502`'s ARR (`$6B`) gets C and V wrong in about half of *all* cases, not only in decimal mode as ADR-010 first said. The ADR is corrected.
- The crate doesn't issue the dummy write of a read-modify-write (the old value, one cycle early). On the SID that write changes nothing. ADR-010 is corrected.
- The wrapper first held writes back until the cycle count was known, and that broke undocumented read-modify-write opcodes that read back what they just wrote. The conformance subset caught it; reads now see pending writes.

## For #59 (oracle test)

Split the oracle in two:

1. **Register level, against VICE:** require identical per-frame register/value sequences and identical intra-frame cycles. This catches any CPU, wrapper or harness regression exactly, without depending on the engine.
2. **Audio:** against our own replay of the VICE log, require waveform r ≥ 0.99 and log-spectral distance ≤ 3.5 dB. Against sidplayfp, keep only a loose sanity bound (envelope r ≥ 0.7 on 6581 and ≥ 0.8 on 8580, log-spectral distance ≤ 7 dB) for as long as the engines differ. Tighten it if we move to reSIDfp.

## Listening

The WAVs are not committed; see the PR for where they are. Suggested order: 1 against 2 (6581), 3 against 4 (8580), then 1 against 5 (same engine, real-machine timing) and 6 (effects stealing voice 3).
