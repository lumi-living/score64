# GoatTracker 2 files

Byte layouts of GoatTracker 2's own files, as Score64 reads and writes them. The source is the GoatTracker 2 readme, section 6.2 (instruments) and section 6.1 (songs). The song layout is transcribed with the `.sng` writer (#47).

## Instruments (`.ins`, `GTI5`)

| Offset | Bytes | Field |
|---|---|---|
| 0 | 4 | Magic `GTI5` |
| 4 | 1 | Attack/decay (SID `$05`) |
| 5 | 1 | Sustain/release (SID `$06`) |
| 6 | 1 | Wavetable start, 1-based within this file's wavetable; 0 for none |
| 7 | 1 | Pulsetable start, likewise |
| 8 | 1 | Filtertable start, likewise |
| 9 | 1 | Vibrato parameter: a speedtable entry, 1-based; 0 for none |
| 10 | 1 | Vibrato delay, in frames |
| 11 | 1 | Gate timer: hard-restart frames in bits 0–5. `$80`: no hard restart. `$40`: no gate-off at the note's end, for legato instruments (see `player.md`) |
| 12 | 1 | First-frame waveform register value |
| 13 | 16 | Name, zero-padded |
| 29 | … | Wavetable, pulsetable, filtertable and speedtable, in that order. Each is 1 byte of length *n*, then *n* left-column bytes, then *n* right-column bytes |

The tables are the instrument's own, so a file of an instrument with no tables is 33 bytes. Nothing follows the speedtable.

**Evidence.**
- `score64_formats::annotate::ins` decodes this layout and accounts for every byte. Its golden dumps (`crates/score64-formats/tests/snapshots`) cover the three fixture instruments in `fixtures/gt2-original/`.
- GoatTracker's own converter, `ins2snd2`, read those same files and produced the sound-effect data in `sfx-*.bin`. Those effects play as designed in VICE (`x64sc`) and in sidplayfp; the fixture README has the verification.
- `score64_formats::gt2::Ins` (#48) reads and writes the layout. All three fixtures round-trip byte for byte.
