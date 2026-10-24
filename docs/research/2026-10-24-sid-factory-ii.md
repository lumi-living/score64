# Research: SID Factory II, and what Score64 can use

**Repository:** [Chordian/sidfactory2](https://github.com/Chordian/sidfactory2), reviewed at `d6852fa` (2026-10-11), release 20260314.
**Purpose:** input for #188 (SID Factory II output, M3), the budget gate (#51/#198), ADR-015 (filter events), ADR-016 (licences) and the Studio UX.
**Licence:** GPL-2.0-or-later (`SIDFactoryII/COPYING` and the source headers). Score64 may use **facts** from it, such as file layouts and documented behaviour, but must not copy code into any MIT crate. Any port would need a clean-room write-up first (CLAUDE.md rule 3).

---

## 1. What SID Factory II is

- A cross-platform tracker (Windows, macOS, Linux), C++ with SDL2, in open beta, actively maintained (commits in 2026).
- It plays its music through **drivers**: 6502 player routines by Laxity and JCH, shipped as `.prg` binaries in `SIDFactoryII/drivers/`.
  - **11.xx "The Standard"** is the default.
  - **12** is minimal; **13** emulates Rob Hubbard's driver; **14** is experimental, with short gate-off; **15 and 16** are tiny, with all variables in zero page; **np20** is NewPlayer 20.
- Emulation is **reSIDfp**, vendored in `source/libraries/residfp/`. That's the same engine Score64 chose in ADR-011.
- **No driver supports sound effects.** The notes for drivers 11–16 have no effect entry point or mailbox; the only "effects" are musical ones (slides, vibrato). Multispeed is also missing (`faq.txt`).

## 2. What it means for Score64

### 2.1 GoatTracker stays the primary target (confirms ADR-007)
SID Factory II can't play sound effects over music, so it can't serve a game whose effects steal a voice. For Score64:
- **SF2 output (#188) is for music-only pieces:** title, menu and jingle tunes, demos and music disks.
- **The validator should refuse an SF2 target when a subtune's profile has expendable or effects-only voices,** because no SF2 driver can honour them. That's a natural extension of #216, which already flags effect voices in a pack built without sound-effect support.

### 2.2 A cheap first step for #188: SF2 already imports GoatTracker songs
SID Factory II's editor imports `.sng` (F10; `dist/documentation/converter.txt` and `source/runtime/editor/converters/gt/source_sng.cpp`). Score64's `.sng` export can therefore already be opened in SF2 for hand-editing, within the importer's limits:

| GoatTracker feature | SF2's importer |
|---|---|
| More than one subtune | **Only the first is converted** (so adaptive sets are lost) |
| `7xx` set waveform | Dropped (affects #203's silence after release) |
| `Bxy` / `Cxy` filter control and cutoff | Dropped (affects ADR-015's `B00` "filter off") |
| `Axx` filter-table pointer | Converted (ADR-015's events survive) |
| `8xx` wave-table pointer | Converted (our arpeggio programs survive) |
| `Dxy` with x ≠ 0 (timing marks) | Silently dropped; `D0y` volume is converted |
| `Exx`, `F00–F01` funktempo; `F83–FFF` per-channel tempo | Dropped (Score64 doesn't use them) |
| `1xx`/`2xx`/`3xx` with divisor ("hi-fi") | Dropped |
| Delays and pattern commands in the wave table | Ignored |
| Vibrato set in the instrument | Ignored (SF2 wants command `$01`) |
| A song with more instruments or patterns than SF2 allows | The converter gives up |

**Suggestion:** before building a native `.sf2` writer, add an **"SF2-importable" export check** that warns about exactly these features. It costs little, and it gives users a working path into SID Factory II now. A native writer then becomes a later step, needed only for features the importer drops.

### 2.3 Facts for a native `.sf2` writer (if #188 goes that far)
From `source/runtime/editor/driver/driver_info.h/.cpp` and `dist/documentation/notes_driver11.txt`:
- **The container is a PRG.** The driver and the song data load together. A file ID of **`0x1337`** sits at the top address, and the auxiliary-data pointer is at **`$0FFB`**.
- **The driver describes itself through header blocks**, each with an ID:
  - `1` Descriptor (type, size, name, code top and size, version);
  - `2` DriverCommon (init, stop, update addresses, plus state addresses the editor reads);
  - `3` DriverTables (table definitions: type `0x80` instruments, `0x81` commands, `0x00` generic; layout and properties);
  - `4` instrument descriptor; `5` MusicData (track count, order-list and sequence pointer tables, sizes); `6–8` colour, insert/delete and action rules for the editor; `9` instrument data descriptor; `0xFF` end.
- **Music layout:**
  - one order list per track, with transposition;
  - shared sequences, with per-event duration;
  - gate on `+++`, gate off `---`, tie `**`.

  SF2's explicit gate events map directly onto Score64's `Fresh`/`Legato` notes and key-offs.
- **Driver 11 tables**, all documented in `notes_driver11.txt`:
  - instruments are 6 bytes: AD, SR, flags, filter index, pulse index, wave index. The flags are: `$80` hard restart, `$40` start the filter program, `$20` enable the filter on this channel, `$10` oscillator reset, `$08` skip pulse reset, `$0X` hard-restart table index;
  - commands `T0–Tf`: slide, vibrato, portamento, arpeggio (speed plus arp-table index), ADSR, filter, wave, pulse and tempo programs, main volume;
  - wave, pulse, filter and arpeggio tables, each with its own jump convention (`7f xx`, or `7X` in the arp table).
- **Hard restart is driver-specific.** Driver 11 has a per-instrument flag plus an 8-row hard-restart table; drivers 15.02 and 16 always write ADSR `0F00`. Score64's `InstrumentFacts` (gate timer, `$40`, `$80`) are GoatTracker terms. An SF2 target will need its facts mapped per driver, so keep the facts model tied to the target rather than hard-wired to GoatTracker.
- **Writer tests:** the demo songs in `SIDFactoryII/music/` exercise each driver (arpeggio, filter, tie notes, polyphony). They're useful for differential tests, **but they're not fixtures**: their licence isn't stated per file, so read them locally and don't commit them.

### 2.4 The packer confirms the budget gate's design (#51)
`source/runtime/editor/packer/packer.cpp` relocates a tune by **disassembling the driver's code** over the exact range the driver's header declares (code top and size). It relocates absolute operands and moves the zero-page operands it finds into a range starting at a configurable "lowest ZP". It also patches a few bytes of init code that set up the zero-page base, and it keeps the standard jump table (init at the start, update via a vector).
- **Static disassembly is reliable when the code range is known.** #51's design note kept it as a cross-check only. For SF2 packs the header gives that range, so the cross-check becomes trustworthy there. For gt2reloc packs, the helper's trace stays the primary source.
- **Score64's budget gate works unchanged on SF2 packs:** a standard jump table, a window, a zero-page set from the trace.

### 2.5 Filter routing: SF2 makes the same choice as ADR-015
In Driver 11.03 and later, a channel is filtered only if **both** the instrument's "enable filter" flag (`$20`) and the filter program's channel bitmask say so. That's the same idea as ADR-015's "the score decides routing; the program supplies the sound": routing is kept separate from the filter program. It's worth citing in ADR-015 as prior art.

### 2.6 Ideas for Score64 Studio and the hardware harness
- **A raster-time "flight recorder":** SF2 draws CPU use per frame, turning orange or red above configurable thresholds (`Visualizer.CPU.Medium.Rasterlines`, `Visualizer.CPU.High.Rasterlines`). Score64 already measures the longest play call (`max_play_cycles`); a per-frame strip in the Build view, with the project's raster budget as the red line, would make #198's budget visible.
- **ASID output (real SID hardware over MIDI):** SF2 can play through ASID devices (USBSID-Pico, TherapSID; `Playback.ASID.MidiInterface`). For E16 (hardware harness and integrations), ASID is a cheap path to a real 6581 or 8580 next to the C64 Ultimate route. Score64 already logs every SID write per frame with its cycle (`render --writes`), which is exactly the data ASID streams.
- **Editor conveniences worth copying in spirit:** play from cursor, bookmarks, per-voice mute (Ctrl+1/2/3), PAL/NTSC and 6581/8580 toggles (F9, Shift+F9), a help overlay for the editor and the current driver (F12), and an optimiser that drops unused sequences and table rows before packing (`source/runtime/editor/optimize/`).

## 3. Licence notes (for ADR-016)
- **SID Factory II itself:** GPL-2.0-or-later.
- **The drivers** are 6502 binaries in the same GPL-2.0-or-later repository. No separate licence or "free to use in productions" statement was found in the docs. So a packed SF2 tune embeds driver code under the repository's licence unless the authors (Laxity, JCH) say otherwise. **This is the same situation as the GoatTracker player in ADR-016:** record it as a distribution obligation in the pack report, and check with the authors before relying on it for closed-source work.
- **The demo songs:** their licences aren't stated, so don't use them as committed fixtures.
