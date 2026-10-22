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
| 11 | 1 | Gate timer: how many frames before the next row the player fetches it, in bits 0–5. `$80`: skip the hard-restart ADSR write (the gate still goes off). `$40`: no gate-off before a new note played with this instrument, for legato instruments. See `sid.md` for which instrument decides what |
| 12 | 1 | First-frame waveform register value |
| 13 | 16 | Name, zero-padded |
| 29 | … | Wavetable, pulsetable, filtertable and speedtable, in that order. Each is 1 byte of length *n*, then *n* left-column bytes, then *n* right-column bytes |

The tables are the instrument's own, so a file of an instrument with no tables is 33 bytes. Nothing follows the speedtable.

**Evidence.**
- **The same 25-byte record holds `sixteen-steps.sng`'s instruments** (`fixtures/gt2-original/`), starting at `$A8`:
  - "Lead PWM" has vibrato `01` and delay `$18`;
  - "Bass filtered" has a filtertable pointer;
  - wave and pulse pointers differ between instruments.

  That song, packed, plays in `score64-emu` frame for frame and write for write as in VICE (`vsid`, `x64sc`), as the fixture README records. So every field's position is borne out by real playback. A test (`gt2::tests::the_songs_instruments_are_the_same_record`) reads them with this layout.
- **The table layout is borne out by GoatTracker's own `ins2snd2`.** It read the three fixture instrument files, including their wave and pulse tables, and produced the sound-effect data in `sfx-*.bin`. Those effects play as designed in VICE and sidplayfp.
- `score64_formats::annotate::ins`, written separately, decodes the same layout and accounts for every byte. Its golden dumps cover the fixtures.
- These files were written by the project's own `make_song.py`, not saved from GoatTracker's editor. A file saved by GoatTracker 2.77 would add a further check.
