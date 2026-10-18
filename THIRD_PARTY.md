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

## vocadito (derived test data)

- **What:** `crates/score64-adapters/testdata/basic-pitch/vocadito_10/`: Basic Pitch activations computed from `vocadito_10.wav` (the audio itself is not committed).
- **Source:** R. Bittner, K. Pasalo, J. J. Bosch et al., *vocadito: A dataset of solo vocals with f0, note, and lyric annotations*, <https://zenodo.org/record/5578807>
- **Licence:** CC BY 4.0 (<https://creativecommons.org/licenses/by/4.0/>)

No other third-party source code or data is vendored yet. Crate dependencies (for example `emulator_6502` and `resid-rs` in `score64-emu`, `ort` in `score64-adapters`) are pulled by Cargo, not copied.

When it is, each entry here names the project, the commit or version, the files ported or included, and the licence text. Planned items and their licences, for reference:

| Item | Use | Licence | Where it may live |
|---|---|---|---|
| reSID-family SID emulation (e.g. `resid-rs`) | Rendering and live audition | GPL | `crates/score64-emu` only (ADR-002) |
| `emulator_6502` 1.1.0 | 6502 core of the emulator helper (ADR-010) | MIT | `crates/score64-emu` only |
| GoatTracker 2 file formats | `.sng` / `.ins` / sound-effect writers, written from the format documentation | Format documentation | `crates/score64-formats` |
| pygoattracker | Differential-test oracle for the `.sng` writer (test-only, via the sidecar) | Apache-2.0 | `sidecar/` (not linked) |
| Basic Pitch model (`nmp.onnx`) | Local transcription | Apache-2.0 | Downloaded at first use, not committed |
| gt2reloc (GoatTracker 2) | Packing; called as an external program | GoatTracker licence | Not vendored |
