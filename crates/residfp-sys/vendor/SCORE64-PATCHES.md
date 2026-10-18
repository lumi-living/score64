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

Nothing else is changed. To update, replace `libresidfp/` with a newer release, re-apply the patch above, regenerate
`../generated/` from its `.in` templates, and re-run the acceptance check (`crates/score64-emu` README / #116).
