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
| 12 | 1 | First-frame waveform register value. Whether GoatTracker gives some values special meanings is not yet checked against its source |
| 13 | 16 | Name, zero-padded. Score64 writes at most 15 characters, so the name always ends in a zero; whether GoatTracker needs that is not yet checked |
| 29 | … | Wavetable, pulsetable, filtertable and speedtable, in that order. Each is 1 byte of length *n*, then *n* left-column bytes, then *n* right-column bytes |

The tables are the instrument's own, so a file of an instrument with no tables is 33 bytes. Nothing follows the speedtable.

**Evidence.**
- **GoatTracker's packer read these records.** `sixteen-steps.sng`'s instruments use the same 25-byte record, from `$A8`. GoatTracker's own `gt2reloc` packed the song, and the result plays as composed (`fixtures/gt2-original/README.md`). That supports the fields the music exercises:
  - "Lead PWM" sweeps its pulse width from the pulse pointer, and its vibrato is set by `01`/`$18` (parameter, delay);
  - "Bass filtered" sweeps the filter from its filter pointer;
  - every instrument's gate timer is 2, under the tempo, where a value read from the wrong byte would stop playback;
  - the wave and pulse pointers differ between instruments.

  A test (`gt2::tests::the_songs_instruments_are_the_same_record`) reads them with this layout. The match with VICE shows only that `score64-emu` plays the packed bytes as VICE does; it says nothing about the layout.
- **The table layout is supported by GoatTracker's own `ins2snd2`.** It read the three fixture instrument files, including their wave and pulse tables, and produced the sound-effect data in `sfx-*.bin`. Those effects play as designed.
- **A consistency check:** `score64_formats::annotate::ins` was written in this project from the same readme, so it is not independent evidence. It decodes the same layout and accounts for every byte.
- **The files' origin:** they were written by the project's own `make_song.py`, not saved from GoatTracker's editor. A file saved by GoatTracker 2.77 would add a further check.

## Wavetable arpeggios

`score64_formats::gt2::wave::arpeggio`. Sources:
- the GoatTracker 2 readme v2.72: §3.4.1 (Wavetable) with its examples, the `8XY` command, the gate-bit note under the pattern note keys, and §4's error conditions;
- the player's stepping, as `src/gplay.c` (lines 438, 513, 522, 695–709) and `src/player.s` (236, 893–951) implement it, in leafo/goattracker2 at commit `a78e7e5273bf`. That mirror doesn't state its version, so it may not be v2.72.

- **The wavetable steps one row per frame.**
  - **Left side:**
    - `$00` keeps the waveform and plays the right side's note;
    - `$01`–`$0F` waits that many frames first, then plays it;
    - `$10`–`$DF` sets the waveform register and plays it;
    - `$E0`–`$EF` sets an inaudible waveform;
    - `$F0`–`$FE` runs a pattern command;
    - `$FF` jumps to the row on the right without taking a frame (`$00` stops the table).

    The readme's delayed minor chord, `21 00 / 02 03 / 02 07 / 02 00 / FF 02`, takes 3 ticks a step.
  - **Right side:** `$00`–`$5F` plays that many semitones above the pattern's note, `$60`–`$7F` below, `$80` keeps the frequency, and `$81`–`$DF` are absolute notes.
- **The gate.** The player writes the wavetable's waveform ANDed with the gate mask, so a waveform without bit `$01` releases the note. Key-on can't set a gate the wavetable cleared.
- **Command `8XY`** sets a voice's wavetable pointer, replacing the instrument's program for the note (`$00` stops the wavetable).
- **Rules the readme gives:**
  - Never jump straight onto a jump row, from an instrument pointer or `8XY`.
  - An instrument's first wavetable step should not be a delay or a no-change row (after `8XY` they are allowed).
  - Commands `0XY`, `8XY` and `EXY` must not run from the wavetable.
- **Score64's arpeggio program follows the readme's idiom.**
  - **Rows:** the waveform row with the first interval; then a row per further interval and one for the first again, each `$00` (one frame a step) or a delay of frames − 1; then a jump to the second row. A single interval is held: its row, then a stop.
  - **Waveform:** it must have the gate bit on and the test bit off.
  - **Limits:** a step takes at most 16 frames, and identical programs are reused.
- **For the `.sng` writer** (from `gplay.c`, via the review): the note plus an interval wraps at 128, so it must stay at or below B-7. The wavetable doesn't run on a note's first frame.

**Evidence:**
- the readme, as above;
- a test interpreter that plays the readme's 3-ticks-a-step example as the readme says.

Playing a packed arpeggio in the emulator and in VICE comes with the `.sng` writer (#47). Until then the layout is unverified in playback.
