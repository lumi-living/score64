# Third-party code

No third-party source code is vendored yet.

When it is, each entry here names the project, the commit or version, the files ported or included, and the licence text. Planned items and their licences, for reference:

| Item | Use | Licence | Where it may live |
|---|---|---|---|
| reSID-family SID emulation (e.g. `resid-rs`) | Rendering and live audition | GPL | `crates/score64-emu` only (ADR-002) |
| GoatTracker 2 file formats | `.sng` / `.ins` / sound-effect writers, written from the format documentation | Format documentation | `crates/score64-formats` |
| pygoattracker | Differential-test oracle for the `.sng` writer (test-only, via the sidecar) | Apache-2.0 | `sidecar/` (not linked) |
| Basic Pitch model (`nmp.onnx`) | Local transcription | Apache-2.0 | Downloaded at first use, not committed |
| gt2reloc (GoatTracker 2) | Packing; called as an external program | GoatTracker licence | Not vendored |
