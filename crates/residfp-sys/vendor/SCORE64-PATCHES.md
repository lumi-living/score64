# Changes to the vendored libresidfp

Source: libresidfp **1.2.3** release tarball (`libresidfp-1.2.3.tar.gz`, SHA-256
`e9b53a984ca708c3e8d9139eb6aad6141f73ffc22d70a020f7966803f4e2f76d`), tag `v1.2.3`, commit
`32c8525418a643eaed8dfafa3d7039e250050cfa`, <https://github.com/libsidplayfp/libresidfp>. GPL-2.0-or-later (`libresidfp/COPYING`).

Only the library sources are kept (`src/`, without tests and autotools files). The two headers configure would generate
are in `../generated/` (written for Score64, not upstream files).

## Patches

1. `src/FilterModelConfig.h`, `Randomnoise`: the float-to-int dither table came from `std::default_random_engine` and
   `std::uniform_real_distribution`, which are a different engine on each standard library (libc++ `minstd_rand`,
   libstdc++ `minstd_rand0`, MSVC `mt19937`). The patch computes what libc++ computes, spelled out:
   `std::minstd_rand` with its default seed, two draws per value as in `std::generate_canonical`. The table is now the same
   on every platform and bit-identical to upstream on macOS (checked: all 1,024 values). Also adds `#include <cmath>`.
   ADR-011 §5.

2. **The filter's dither counter** (#207). `src/FilterModelConfig.h` (`Randomnoise`, `getNormalizedValue`, `getNormalizedVoice`), `src/Filter.h`, `src/Filter.cpp`, `src/FilterModelConfig6581.cpp` (`getDAC`), `src/Integrator6581.h`, `src/Integrator8580.h`.
   - **Upstream** drew every float-to-int value's dither from one counter, `Randomnoise::index`, shared by every caller in the process.
   - **The tables** (summer, mixer, volume, resonance) are built on four to six threads at once, so which dither each entry got depended on how the threads interleaved, and the counter was a data race. The same input gave different samples in each process.
   - **At run time** every SID drew from the same counter, so a SID's output also depended on what other SIDs in the process had drawn before it.
   - **The patch** keeps the same 1,024-value sequence but gives each caller its own counter:
     - each table, starting from the sequence's start;
     - each SID's filter, from the start at construction and at every `reset()`;
     - the few values set up once, by a fixed position.
   - **The shared counter is gone.** The threaded build is kept.
   - **Result:** identical writes give byte-identical samples, across processes and between SIDs in one process. `src/lib.rs`'s `identical_writes_give_identical_samples` and `score64-adapters`' `helper_determinism` check this.
   - **Relative to upstream:** the dither is the same kind of noise from the same sequence, only assigned in a fixed order. Upstream's order was itself different on every run.

Nothing else is changed. To update, replace `libresidfp/` with a newer release, re-apply the patches above, regenerate
`../generated/` from its `.in` templates, and re-run the acceptance check (`crates/score64-emu` README / #116).
