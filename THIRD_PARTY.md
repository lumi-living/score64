# Third-party code

## SingleStepTests/65x02 (test vectors)

- **What:** `crates/score64-emu/testdata/sst-6502-subset.txt`, the first 16 cases per opcode from `6502/v1`, rewritten as one line per case by `crates/score64-emu/spikes/cpu-conformance/make_subset.py`. Used only by `score64-emu`'s CPU conformance test.
- **Source:** <https://github.com/SingleStepTests/65x02>
- **Licence:** MIT

```
MIT License

Copyright (c) 2024 Thomas Harte et al

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## Basic Pitch (ported code)

- **What:** `crates/score64-adapters/src/basic_pitch/notes.rs` and `model.rs` are Rust translations of `basic_pitch/note_creation.py` (note creation, frame times) and `basic_pitch/inference.py` (windowing and unwrapping) from Basic Pitch v0.4.0. Changes: translated to Rust; pitch bends left out. The model file `nmp.onnx` is not committed.
- **Source:** <https://github.com/spotify/basic-pitch>
- **Licence:** Apache-2.0 (<http://www.apache.org/licenses/LICENSE-2.0>). Its NOTICE file:

```
Basic Pitch
Copyright 2022 Spotify AB

This product includes software developed at
Spotify AB (http://www.spotify.com/).

This product includes software from Librosa (ISC).
* Copyright (C) 2013--2017, librosa development team.

This product includes software from mir_eval (MIT)
* Copyright (C) 2014 Colin Raffel

This product includes software from numpy (BSD)
* Copyright (C) 2005-2022, NumPy Developers.

This product includes software from pretty-midi (MIT)
* Copyright (C) 2014 Colin Raffel

This product includes software from resampy (ISC)
* Copyright (C) 2016, Brian McFee

This product includes software from scipy (BSD)
* Copyright (C) 2001-2002 Enthought, Inc. 2003-2022, SciPy Developers

This product includes software from tensorflow (Apache 2.0)
* Copyright (C) 2019 Google, LLC <packages@tensorflow.org>

The tests for `basic-pitch` include audio files from the
Vocadito dataset liscened under Creative Commons
Attribution 4.0 International. The dataset can be found at:
https://zenodo.org/record/5578807#.YnRm5vPMKDU
```

## ONNX Runtime (linked library)

- **What:** ONNX Runtime 1.31.0, linked statically into the `score64` CLI through the `ort` crate (2.0.0-rc.14, MIT or Apache-2.0), which downloads pyke's prebuilt builds of it. It runs the Basic Pitch model.
- **Source:** Microsoft, <https://github.com/microsoft/onnxruntime>. Its own third-party notices: <https://github.com/microsoft/onnxruntime/blob/v1.31.0/ThirdPartyNotices.txt>.
- **Licence:** MIT:

```
MIT License

Copyright (c) Microsoft Corporation

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## vocadito (derived test data)

- **What:** `crates/score64-adapters/testdata/basic-pitch/vocadito_10/`: Basic Pitch activations computed from `vocadito_10.wav` (the audio itself is not committed).
- **Source:** R. Bittner, K. Pasalo, J. J. Bosch et al., *vocadito: A dataset of solo vocals with f0, note, and lyric annotations*, <https://zenodo.org/record/5578807>
- **Licence:** CC BY 4.0 (<https://creativecommons.org/licenses/by/4.0/>)

## libresidfp (vendored)

- **What:** `crates/residfp-sys/vendor/libresidfp/`: reSIDfp, the SID emulation of libsidplayfp, as its standalone library, version **1.2.3** (tag `v1.2.3`, commit `32c8525418a643eaed8dfafa3d7039e250050cfa`, release tarball SHA-256 `e9b53a984ca708c3e8d9139eb6aad6141f73ffc22d70a020f7966803f4e2f76d`). It is the default SID engine of `score64-emu` (ADR-011). Only the library sources are kept, plus two patches, described in `crates/residfp-sys/vendor/SCORE64-PATCHES.md`: a platform-independent dither table in `src/FilterModelConfig.h`, and each caller of that table given its own counter, so renders are byte-identical across processes (#207). The headers in `crates/residfp-sys/generated/` are derived from its `.in` templates.
- **Source:** <https://github.com/libsidplayfp/libresidfp>
- **Licence:** GPL-2.0-or-later; full text in `crates/residfp-sys/vendor/libresidfp/COPYING`. Copyright (c) 2000–2011 Dag Lem, (c) 2007–2010 Antti Lankila, (c) 2010–2026 Leandro Nini. `residfp-sys` carries the same licence, and only the GPL-3.0-or-later `score64-emu` may depend on it (ADR-002; CI job `licence-boundary`).

Apart from the entries in this file, no third-party source code or data is vendored. Crate dependencies (for example `emulator_6502` and `resid-rs` in `score64-emu`, `ort` in `score64-adapters`) are pulled by Cargo, not copied.

When it is, each entry here names the project, the commit or version, the files ported or included, and the licence text. Planned items and their licences, for reference:

| Item | Use | Licence | Where it may live |
|---|---|---|---|
| reSID-family SID emulation (e.g. `resid-rs`) | Rendering and live audition | GPL | `crates/score64-emu` only (ADR-002) |
| `emulator_6502` 1.1.0 | 6502 core of the emulator helper (ADR-010) | MIT | `crates/score64-emu` only |
| GoatTracker 2 file formats | `.sng` / `.ins` / sound-effect writers, written from the format documentation | Format documentation | `crates/score64-formats` |
| Basic Pitch model (`nmp.onnx`) | Local transcription | Apache-2.0 | Downloaded at first use, not committed |
| gt2reloc (GoatTracker 2) | Packing; called as an external program | GoatTracker licence | Not vendored |

## Inter and JetBrains Mono (fonts)

- **What:** `crates/score64-app/assets/fonts/`: Inter 4.1 Regular and Medium, and JetBrains Mono 2.304 Regular, unmodified static TTFs. They are bundled into Score64 Studio, the same files as Convert64's. Sources and SHA-256 hashes are in that directory's `README.md`.
- **Sources:** <https://github.com/rsms/inter>, <https://github.com/JetBrains/JetBrainsMono>
- **Licence:** SIL Open Font License 1.1. The full texts are in `OFL-Inter.txt` and `OFL-JetBrainsMono.txt` next to the fonts.

## Material Symbols (icon font)

- **What:** the Material Symbols icon font, bundled into Score64 Studio through the `egui_material_icons` crate, version 0.8.0 (MIT), with only its `outline` feature. That embeds `MaterialSymbolsRounded-Regular.ttf`: the unfilled cut of Material Symbols **Rounded**, which the crate calls its outline variant. The crate's filled cut is not included.
- **Source:** Google, <https://github.com/google/material-design-icons>; the crate at <https://github.com/lucasmerlin/hello_egui/tree/main/crates/egui_material_icons>.
- **Licence:** Apache License 2.0 (the font); MIT (the crate).

