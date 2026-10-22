# Reference

Facts the code must agree with. If code and reference disagree, one of them is wrong: fix it in the same PR, with evidence (VICE, sidplayfp, hardware, or the cited source).

| File | Contents |
|---|---|
| [sid.md](sid.md) | SID registers, waveforms, envelope, filter, 6581 vs 8580 |
| [player.md](player.md) | GoatTracker 2 packed player: jump table, gt2reloc options, timing marks, sound-effect calls, zero page |
| [goattracker-sfx.md](goattracker-sfx.md) | GoatTracker sound-effect data format and conversion rules |
| [tempo-grid.md](tempo-grid.md) | Rows, frames and BPM on PAL (nominal 50 Hz, real 50.1246 Hz) and NTSC (nominal 60 Hz, real 59.826 Hz) |
| [emu-protocol.md](emu-protocol.md) | The emulator helper's stdin/stdout protocol: framing, messages, errors, golden bytes |

Byte layouts of `.sng` and `.ins` follow the GoatTracker 2 readme (sections 6.1 and 6.2); a transcription with test vectors arrives with epic E7.
