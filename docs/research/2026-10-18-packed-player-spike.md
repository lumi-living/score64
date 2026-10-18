# Spike: a gt2reloc-packed tune in score64-emu, compared with VICE and sidplayfp

**Issue:** #20 (part of E3, #17) · **Date:** 2026-10-18 · **Core:** `emulator_6502` 1.1.0 (ADR-010) · **SID:** `resid-rs` 1.1.1

## Question

Can `score64-emu` run a real GoatTracker packed player (init, then play at 50 Hz) on our 6502 core and `resid-rs`, and how far are its renders from a reference player on both SID models? Is `resid-rs` enough, or do we need reSIDfp through FFI?

## Answer

- **The CPU and harness side is exact.** Over 60 s (3,007 frames), the helper's SID writes match VICE's cycle-exact 6510 write for write. Every frame has the same registers and values in the same order, and every write sits on the same cycle relative to the frame's first write, gate-off and ADSR (hard-restart) writes included.
- **The same holds with sound effects.** A KickAssembler driver fired the gun effect on the same schedule in x64sc. All 3,007 frames came out identical there too, write for write and cycle for cycle.
- **The audio gap to sidplayfp is the SID engine, not the CPU.** Playing VICE's exact write log through our own SID gives the same distance to sidplayfp as the helper does (table below).
- **`resid-rs` is the outlier, and you can hear it.** It is a port of **reSID 0.16** (2004), not reSID 1.0. Its filter makes the filtered voice too loud. In this tune that's the bass on voice 3, so the mix sounds unbalanced (Jaakko's listening test: "one channel is overpowering"). On the 6581, VICE's C++ reSID 1.0 and sidplayfp's reSIDfp agree on the bass balance, and the helper is 2 dB bass-heavy. Played alone, voice 3 comes out about 5 dB louder relative to voice 1 than in reSIDfp, on both chips.
- **Recommendation:** move the default engine to reSIDfp through FFI. `libresidfp` is now a standalone C++ library (1.2.3), which makes that practical. Do it *before* M1 renders are judged by ear (E8, #54, ADR first), rather than later as first planned. reSIDfp also models the 6581 filter and combined waveforms more accurately, and using the oracle's own engine turns #59 into a tight test.

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
| Play-call jitter, VICE vs helper | ±2 cycles (mean 0.86) |

The only timing difference is *when play starts*. On a C64, interrupt latency moves the play call by a few cycles from frame to frame. The helper calls play exactly once per 19,656 cycles (TAD §5.1). That jitter, ±2 cycles here and ±19 in the x64sc effect driver below, is why "helper vs VICE timing" below is not bit-identical. (An earlier draft said "a few hundred cycles". That figure was the spacing between each frame's first write, which also changes with how much work the player does before writing.)

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

## Mix balance: `resid-rs` against C++ reSID and reSIDfp

Listening to the first renders, the helper's mix sounded unbalanced on both chips, with "one channel overpowering", while sidplayfp's sounded balanced. In this tune voice 1 is the lead (pulse and triangle), voice 2 is sawtooth, and voice 3 is the bass: pulse, sustain F, and the only voice routed through the filter (`$D417 = $14`, low-pass `$D418 = $1F`).

VICE's WAV device only writes audio in real time (not warp) and only on a clean exit, so VICE was recorded live for 63 s and stopped with `quit` through its remote monitor. Bass share (30–250 Hz energy against 250 Hz–12 kHz):

| Chip | Helper (`resid-rs`, reSID 0.16) | VICE reSID 1.0 | VICE reSID 1.0, filter bias 0 | sidplayfp (reSIDfp) |
|---|---|---|---|---|
| 6581 | **+2.0 dB** | −0.1 dB | +0.1 dB | +0.1 dB |
| 8580 | +1.8 dB | −0.1 dB | −0.1 dB | +1.9 dB |

Each voice alone (the other two muted: `--mute` in the helper, `-u` in sidplayfp), voice 3 relative to voice 1:

| Chip | Helper | sidplayfp |
|---|---|---|
| 6581 | **+4.5 dB** | −0.9 dB |
| 8580 | **+4.6 dB** | −0.7 dB |

On both chips the filtered bass comes out about 5 dB louder, relative to the lead, than in reSIDfp. On the 6581 the two modern engines agree on the overall balance and the helper is 2 dB bass-heavy. VICE's filter bias setting is not the cause. `resid-rs`'s filter carries the reSID 0.16 copyright and f0 spline tables; reSID 1.0 replaced that model in 2010. Against VICE's reSID 1.0 directly, the helper's audio has waveform r 0.70, envelope r 0.89/0.83 and log-spectral distance 5.6/6.0 dB (6581/8580). That is no closer than reSIDfp is.

(reSIDfp's mixer is non-linear: its three solo levels add up to 3.4 dB more than its full mix. So the solo figures are for comparing voices within one engine, not for summing.)

## Sound effects on a real 6510

`spikes/packed-player/sfx-driver.asm` (KickAssembler 5.25) is a test program for x64sc. It banks out the KERNAL, blanks the screen (no bad lines), installs its own raster IRQ, calls init, then play every frame. It fires the gun effect on voice 3 on the same frames as `score64-emu render --effect sfx_gun.bin --effect-every 37 --effect-voice 3`. x64sc's dump gives **3,007 / 3,007 identical frames, all 60,828 writes on the same intra-frame cycle**. The helper drives the effect entry exactly as a game on a real machine would.

What voice 3 does around one shot (frame 37):

- frames 37–42: the shot: noise, pulse, noise.
- frames 43–65: the effect's tail, gate off, noise waveform, pitch sweeping down from 10,004 to 702 over about half a second. Low-pitched SID noise sounds like a run of random tones, which is the "weird notes between shots" heard in the first listening test.
- frames 66–73: silence (`$D412 = $00`). The GoatTracker player gives the voice back to the music only at the music's next new note on it. This bass holds long notes, so at a shot every 37 frames it never returns.

That is the player's behaviour and this effect's design, not a helper problem. For a game it means two things: a resident effect should be short, and a voice the music relies on for sustained notes is a poor choice for frequent effects (the `combat` profile's rule).

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

The WAVs are not committed; see the PR for where they are. Suggested order: 1 against 2 (6581), 3 against 4 (8580), then 1 against 5 (same engine, real-machine timing) and 6 (effects stealing voice 3). 7 fires the effect every 5 s, so one shot's tail is heard whole and the bass comes back on its next note. 8 is voice 3 alone with a shot every 37 frames.

First listening (Jaakko): 2 and 4 sound like a more balanced mix. In 1 and 3 one channel overpowers the others (see "Mix balance"). In 6 there are odd notes between shots (see "Sound effects on a real 6510").
