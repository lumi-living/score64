# Third-party code

No third-party source code is vendored yet.

When it is, each entry here names the project, the commit or version, the files ported or included, and the licence text. Planned items and their licences, for reference:

| Item | Use | Licence | Where it may live |
|---|---|---|---|
| reSID-family SID emulation (e.g. `resid-rs`) | Rendering and live audition | GPL | `crates/score64-emu` only (ADR-002) |
| libresidfp 1.2.3 (reSIDfp) | Default SID engine, vendored source with one documented patch (ADR-011) | GPL-2.0-or-later | `crates/residfp-sys`, used only by `crates/score64-emu` |
| `emulator_6502` 1.1.0 | 6502 core of the emulator helper (ADR-010) | MIT | `crates/score64-emu` only |
| SingleStepTests/65x02 (6502 vectors) | CPU conformance tests; a subset is committed with the tests | MIT | `crates/score64-emu` tests |
| GoatTracker 2 file formats | `.sng` / `.ins` / sound-effect writers, written from the format documentation | Format documentation | `crates/score64-formats` |
| pygoattracker | Differential-test oracle for the `.sng` writer (test-only, via the sidecar) | Apache-2.0 | `sidecar/` (not linked) |
| Basic Pitch model (`nmp.onnx`) | Local transcription | Apache-2.0 | Downloaded at first use, not committed |
| gt2reloc (GoatTracker 2) | Packing; called as an external program | GoatTracker licence | Not vendored |
