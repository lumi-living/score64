# ADR-011: reSIDfp (libresidfp) via FFI as the default SID engine

**Status:** Accepted
**Date:** 2026-10-18
**Deciders:** @jahaa

## Context

The #20 spike ([report](../research/2026-10-18-packed-player-spike.md)) ran a gt2reloc-packed GoatTracker player in `score64-emu` on `emulator_6502` (ADR-010) and `resid-rs` 1.1.1. The part we own is exact: every SID write matches VICE's cycle-exact 6510, write for write and cycle for cycle, with and without sound effects. The engine is the weak part:

- `resid-rs` is a Rust port of **reSID 0.16** (2004). VICE moved to reSID 1.0 in 2010 and sidplayfp uses reSIDfp, a fork of reSID 0.16 with much newer 6581 and 8580 filter, mixer and combined-waveform models.
- In listening tests the helper's mix sounded unbalanced on both chips, with "one channel overpowering", while sidplayfp's sounded balanced. Measured, the filtered bass on voice 3 comes out about 5 dB louder relative to the lead than in reSIDfp. On the 6581 the helper's bass share is +2.0 dB, while VICE's reSID 1.0 (−0.1 dB) and reSIDfp (+0.1 dB) agree with each other.
- Score64 is a tool for judging by ear (ADR-003, UX J1). If renders mislead about the balance, the tool is wrong where it matters most.
- Our oracle (#59) is sidplayfp. With a different engine the oracle can only check loose envelope and spectral features (envelope r 0.74 on 6581).

`libresidfp` 1.2.3 (2026) is reSIDfp split out of libsidplayfp into a standalone C++ library, GPL-2.0-or-later. Its public class `reSIDfp::residfp` has a small API: chip model, sampling parameters, `write`, `read`, `clock(cycles, buffer)`, filter curve and range, combined-waveform strength. Inside sidplayfp, which also emulates the rest of the C64, it renders 60 s in about 2 s on an M-series Mac, around 30× real time.

## Decision

Make **reSIDfp, through libresidfp, the default SID engine** of `score64-emu`, before the M1 render work (#54) builds on the engine:

1. **`crates/residfp-sys`**, a new workspace crate, GPL-2.0-or-later (the vendored code's licence; it combines with the GPL-3.0-or-later helper). It holds libresidfp 1.2.3's source unchanged except for documented patches (below, and `vendor/SCORE64-PATCHES.md`). It also holds pre-generated `siddefs-fp.h` and `sidversion.h` (the only autotools outputs the sources need), a `build.rs` that compiles everything with the `cc` crate (C++17), and an `extern "C"` shim over `reSIDfp::residfp`. `unsafe` is confined to this crate; it overrides the workspace lint and wraps the shim in a safe Rust type.
2. **A `SidEngine` trait** in `score64-emu` (write at a cycle, advance and collect samples, read), with two implementations: `residfp` (default) and `resid-rs` (`--engine resid-rs`). `resid-rs` stays for comparison until #59 lands; after that it is removed unless something still needs it.
3. **Settings match sidplayfp 3.2.2's defaults**: 6581 filter curve 0.5 and range 0.5, 8580 filter curve 0.5, combined waveforms `AVERAGE`, `RESAMPLE` sampling. That way the oracle compares like with like. The settings are exposed on the command line, not fixed in code.
4. **The licence boundary covers both GPL crates.** Only `score64-emu` may depend on `residfp-sys`, and CI's `licence-boundary` job checks it the same way it checks `score64-emu` (ADR-002).
5. **Determinism patch:** libresidfp seeds its float-to-int dither from `std::default_random_engine`, which is a different generator in libstdc++, libc++ and MSVC. The vendored copy fills that 1024-value table from a fixed generator, `std::minstd_rand`, with its values derived without `std::uniform_real_distribution`, whose output is not fully specified either. The same table on every OS removes one source of difference between platforms. Floating-point differences between compilers can remain, so audio golden tests use tolerances, and register-level tests stay exact.

   *Amended 2026-10-24 (#207):* a second patch gives each caller of that dither table its own counter. Upstream's shared counter was advanced by the filter tables' build threads at once, so the same input rendered differently in each process. The same build now renders byte-identically for the same input, across processes and between SID instances.

## Options considered

### Option A: libresidfp via FFI, vendored and built with `cc` (chosen)
| Dimension | Assessment |
|---|---|
| Complexity | About 20 C++ files and a 15-function C shim; no `cxx` or `bindgen` needed |
| Performance | Fast enough (about 30× real time inside a full C64 emulator) |
| Maintenance | Upstream is active (2026). Updating means re-vendoring a tagged release and re-running the oracle |
| Fit | The engine of our oracle and of most current SID playback; fixes the balance problem |

### Option B: link a system libresidfp (pkg-config)
| Dimension | Assessment |
|---|---|
| Fit | Hardly packaged: Homebrew's libsidplayfp bottle doesn't even include reSIDfp; on Windows it's a manual build. That breaks `cargo build` for contributors and cargo-dist bundles |

### Option C: port reSIDfp to Rust
| Dimension | Assessment |
|---|---|
| Complexity | About 9,000 lines of numeric C++ to port and keep in step with upstream |
| Fit | Removes the C++ toolchain, but it's months of work, and every upstream fix would need re-porting with differential tests |

### Option D: keep `resid-rs`, or port reSID 1.0's filter into a fork
| Dimension | Assessment |
|---|---|
| Fit | Keeps the audible imbalance (A) or replaces it with reSID 1.0's model, which still isn't the oracle's engine, so #59 stays loose |

## Trade-off analysis

Option A costs a C++ compiler in every build of `score64-emu` and a small amount of `unsafe` in one crate. GitHub's runners, Xcode and MSVC all provide the compiler, and the `unsafe` is a thin, testable shim. In return the renders people judge by ear use the most accurate maintained engine, and the oracle becomes the same engine run from the same register writes. Porting (C) is the only option that keeps the helper pure Rust, but it's months of work with no audible gain over A. `resid-rs` stays behind the trait for now, so an unexpected FFI problem can fall back without blocking M1.

## Consequences
- Easier: renders sound like sidplayfp (and, by reputation, closer to real chips). #59 can require waveform correlation instead of loose feature bounds. The mix balance problem from #20 goes away.
- Harder: `score64-emu` needs a C++17 compiler; the build is slower on a cold cache; a vendored library to keep current; `unsafe` in one crate; audio is not bit-identical across platforms.
- Revisit when: a Rust reSIDfp port of equal accuracy exists; libresidfp's API changes incompatibly; or the C++ build becomes a real problem for contributors or cargo-dist.

## Action items
- [ ] #116: `residfp-sys`, the `SidEngine` trait, the default switch, CI and `licence-boundary`, `THIRD_PARTY.md`.
- [ ] #116 acceptance: VICE's write log replayed through reSIDfp against sidplayfp gives waveform r ≥ 0.99 on 6581 and 8580, and the mix balance passes a listening check.
- [ ] #59: tighten the audio tolerances once #116 lands (the #20 report has the starting values).
