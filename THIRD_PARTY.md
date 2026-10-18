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

No other third-party source code or data is vendored yet. Crate dependencies (for example `emulator_6502` and `resid-rs` in `score64-emu`) are pulled by Cargo, not copied.

When it is, each entry here names the project, the commit or version, the files ported or included, and the licence text. Planned items and their licences, for reference:

| Item | Use | Licence | Where it may live |
|---|---|---|---|
| reSID-family SID emulation (e.g. `resid-rs`) | Rendering and live audition | GPL | `crates/score64-emu` only (ADR-002) |
| libresidfp 1.2.3 (reSIDfp) | Default SID engine, vendored source with one documented patch (ADR-011) | GPL-2.0-or-later | `crates/residfp-sys`, used only by `crates/score64-emu` |
| `emulator_6502` 1.1.0 | 6502 core of the emulator helper (ADR-010) | MIT | `crates/score64-emu` only |
| GoatTracker 2 file formats | `.sng` / `.ins` / sound-effect writers, written from the format documentation | Format documentation | `crates/score64-formats` |
| pygoattracker | Differential-test oracle for the `.sng` writer (test-only, via the sidecar) | Apache-2.0 | `sidecar/` (not linked) |
| Basic Pitch model (`nmp.onnx`) | Local transcription | Apache-2.0 | Downloaded at first use, not committed |
| gt2reloc (GoatTracker 2) | Packing; called as an external program | GoatTracker licence | Not vendored |
