# GoatTracker 2 files

Byte layouts of GoatTracker 2's own files, as Score64 reads and writes them. The source is the GoatTracker 2 readme, section 6.2 (instruments) and section 6.1 (songs), read from v2.72's readme (<https://raw.githubusercontent.com/leafo/goattracker2/master/readme.txt>, a mirror; its history says v2.59 introduced GTS5).

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

## Songs (`.sng`, `GTS5`)

`score64_formats::gt2::sng::Sng`. Offsets count from the start of the file for the header and from the start of each part after it.

| Part | Bytes | Contents |
|---|---|---|
| Magic | 4 | `GTS5` |
| Name, author, copyright | 32 each | Zero-padded text. Score64 writes at most 31 characters, so each ends in a zero; whether GoatTracker needs that is not checked |
| Subtune count | 1 | 1 to 32 |
| Orderlists | per subtune, voices 1, 2, 3 | A length *n*, then *n* + 1 bytes: the entries, the end mark `$FF`, and the restart position. *n* counts the entries and the end mark, not the restart position |
| Instrument count | 1 | 0 to 63. Instrument 0, the empty one, is not stored; the records are instruments 1 onwards |
| Instruments | 25 each | The record above (`.ins` offsets 4–28) |
| Tables | | Wave, pulse, filter and speed, each as in an instrument file. Instrument pointers count from 1 into these, the song's own tables |
| Pattern count | 1 | 1 to 208 (numbers `$00`–`$CF`) |
| Patterns | per pattern | A row count *m*, then *m* rows of 4 bytes. *m* includes the end row, whose note byte is `$FF` |

Nothing follows the last pattern.

**Orderlist entries:**
- `$00`–`$CF`: a pattern number.
- `$D0`–`$DF`: repeat the next pattern (`R1`–`RF`, and `R0` means 16 times).
- `$E1`–`$FE`: transpose by the entry minus `$F0` (`$F0` none, `$FE` +14, `$E1` −15). The readme gives the ranges, up 0–14 and down 1–15; the mapping is the player's (`gplay.c`). `$E0` would be −16, outside the range, and Score64 refuses it.
- Before each pattern number, at most one transpose, then at most one repeat. The readme says transpose must come first, and the player reads one of each in that order; the editor halts playback otherwise.
- At most 254 entries before the end mark. The last must be a pattern number, and the restart position must lie inside the list.
- GoatTracker saves an empty orderlist (no entries) for an unused subtune, and `gt2reloc` drops that subtune when packing. Score64 reads such files but doesn't write them: its own rule, not GoatTracker's.

**Pattern rows:** note, instrument, command, command data.
- **Note:** `$60`–`$BC` (C-0 to G#7), `$BD` rest, `$BE` key off, `$BF` key on.
- **Instrument:** `$01`–`$3F`, or `$00` for no change.
- **Command:** `$0`–`$F`.
- At most 128 rows, plus the end row. GoatTracker's editor writes the end row as `$FF 00 00 00`, but loading keeps whatever the file has, so Score64 keeps those three bytes too.

**Evidence.**
- **GoatTracker's own loader and saver agree:** `src/gsong.c` (`loadsong`, `savesong`) in leafo/goattracker2 reads and writes the orderlist length, pattern row count and instrument count as above (checked in the #181 review).
- **GoatTracker's packer read this layout.** `gt2reloc` packed `sixteen-steps.sng` (`fixtures/gt2-original/README.md`), and the result plays as composed. Every length rule above is exercised: a wrong orderlist length or pattern row count would misplace every byte after it.
- **Byte-for-byte round trip.** `Sng` reads the fixture and writes it back identically (`gt2::sng::tests`). A property test round-trips any song within the limits.
- **A consistency check:** the fixture was written by the project's `fixtures/_tools/gtsng.py`, and `score64_formats::annotate::sng` reads the same layout. Both were written here from the same readme, so neither is independent evidence. The differential test against pygoattracker comes with #47.
- **The limits** (32 subtunes, 254 orderlist entries, 208 patterns, 128 rows, 63 instruments) are from the readme, sections 3.1–3.3. Instrument 63 can carry a tempo setting (the readme's v2.5 history entry); how is not transcribed here, so a writer should leave instrument 63 unused for now.
- **Versions:** the readme mirror is v2.72; the fixture's `gtsng.py` was written against the 2.77 readme; the installed `gt2reloc` reports v2.73 (#177).
