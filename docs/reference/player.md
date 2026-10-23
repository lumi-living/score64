# GoatTracker 2 packed player

Score64 targets the packed player that GoatTracker 2's relocator (`gt2reloc`, or the editor's F9 packer) produces. Pin the GoatTracker version used for packing in the project and in CI fixtures.

## Jump table (relative to the player start address)

| Offset | Routine | Call |
|---|---|---|
| +0 | Init | `LDA #subtune` · `JSR start` |
| +3 | Play | `JSR start+3` once per frame (50 Hz PAL), from the same raster interrupt every frame |
| +6 | Play sound effect (only when packed with sound-effect support) | `LDA #<effect` · `LDY #>effect` · `LDX #channel` (0, 7 or 14 for voices 1–3) · `JSR start+6` |
| +6 or +9 | Set master volume (only with volume-change support; +9 when sound effects are also enabled) | `LDA #volume (0–15)` · `JSR …` |

Each entry is a `JMP abs` (`$4C lo hi`) into the player. Without sound-effect support (`-D0`) and without volume-change support (gt2reloc's default; Score64 never passes `-E1`) there is no entry at +6: the bytes there are the start of the player's code (`B9 76 24`, `LDA $2476,Y`, in the fixture below). Rule 8 refuses a pack whose init or play entry, or with effects configured its effect entry, isn't a `JMP abs` into the tune past the jump table. (With `-E1` and no effects, +6 would be the volume entry, a `JMP` the effect check can't tell apart; Score64 never asks for `-E1`.)

## Output containers

gt2reloc picks the container from the output's extension (`sid`, `prg` or `bin`, lowercase; anything else, `.SID` or no extension included, gives a `.prg`). All three hold the same bytes, which load at the player page (`-W`):

| Container | Layout |
|---|---|
| `.prg` | The load address (little-endian), then the bytes |
| `.bin` | The bytes alone; they load at `-W`, which the file doesn't record |
| `.sid` | A PSID v2 header of `$7C` bytes. Its load address (+8) is 0, so the load address (little-endian) comes first in the data. Init (+$0A) is the player start and play (+$0C) is start+3. The name, author and copyright fields come from the song |

The size gt2reloc reports is the bytes without any header or load address, so `.prg` is that size plus 2 and `.sid` that size plus `$7C` + 2.

**Evidence.** `fixtures/gt2-original/sixteen-steps.sng` packed by gt2reloc 2.73 with `-W20 -ZFB` (2026-10-23):
- with `-D1` it reports 2,028 bytes: `.bin` is 2,028 bytes, `.prg` 2,030 and `.sid` 2,154; `.SID`, `.Prg`, `.xyz` or no extension give the 2,030-byte `.prg`;
- the `.prg` starts `00 20 4C 7D 20 4C B2 20 4C 87 20`;
- the `.sid` header starts `50 53 49 44 00 02 00 7C 00 00 20 00 20 03`, its name, author and copyright fields are the song's (`Sixteen Steps`, …), and its data starts `00 20 4C 7D 20`;
- with `-D0` it reports 1,769 bytes, and the `.prg` starts `00 20 4C 7A 20 4C 84 20 B9 76 24`.

`crates/score64-adapters/tests/pack_facts.rs` checks, for every container with and without `-D1`: the same bytes at the player page, each file's length, the `.sid` header's version, data offset, load address 0, init and play, the jump table, and no `JMP` at +6 without `-D1`.

## gt2reloc options used by Score64

`gt2reloc <song.sng> <out.prg|.sid|.bin> [options]`

| Option | Meaning | Score64 use |
|---|---|---|
| `-Dx` | Sound-effect support on/off (implies buffered SID writes) | `[pack] sound_effects` (ADR-014); `-D1` when the target plays effects |
| `-Hx` | Store author info at start+$20…+$3F and enable timing marks | `-H1` for adaptive sets |
| `-Wxx` | Player start address, high byte (default `10` = `$1000`) | `[pack] player_page` (ADR-014) |
| `-Zxx` | Zero-page location (needs 2 consecutive bytes; default `FC`) | `[pack] zero_page` (ADR-014), from the target's zero-page map. The default `$FC–$FD` overlaps the common `$FB–$FE` pointer convention used by many C64 programs, so Score64 always passes `-Z` explicitly |
| `-Axx` | Hard-restart ADSR value (default `0F00`) | `[pack] hard_restart_adsr` (ADR-014); gt2reloc's default when absent |
| `-Bx` | Buffered SID writes | Implied by `-D1` |
| `-Cx` | Zero-page ghost registers | Avoid with sound effects (shared register state can leave a wrong pulse width after an effect) |
| `-Jx` | Full buffering | Avoid with sound effects (same reason) |
| `-Ex` | Volume-change support | Not used (master volume would also silence effects) |
| `-P` / `-N` | PAL (default) / NTSC timing | PAL |

## Timing marks

With author info enabled, a master-volume command `DXY` with parameter `$10–$FF` is a **timing mark**: the parameter is copied to start+$3F when playback reaches it. Score64 writes a mark at each switch point an adaptive set declares (every bar by default, every N bars, or explicit rows; TAD §3.1). Successive marks alternate their values so each one registers as a change, and the game switches subtunes when the byte changes.

What a packed song does with them (gt2reloc 2.73, `-H1`). This was traced in the emulator helper, whose 6510 core matched VICE cycle for cycle in the #20 spike (`crates/score64-adapters/tests/timing_marks.rs`, #52). The confirmation in x64sc belongs to the VICE harness (#104).
- The byte changes on the first frame of the marked row, the same frame the row's notes start, at a bar start or mid-bar, on a row with a note or without. A row's frame is the tempo map's, so rows 0, 16, 32 and 48 of a layer at 5 frames a row, and 3 from row 40, land 0, 80, 160 and 224 frames after row 0.
- A jump back to the loop row runs that row's mark like any other.
- Init doesn't reset the byte. A fresh load holds the author field's last byte there (`$20`, a space, for a short author), and a subtune started after another keeps the other's last value until its row-0 mark. Row 0 starts on the eighth play call after init (frame 7).

## Sound effects and the music

What the GoatTracker 2.77 packed player does when a sound effect plays on a voice the music also uses. The evidence is VICE, plus the sources cited in each point:

- the #20 spike ran the player on x64sc with effects on voice 3 and matched `score64-emu` on all 3,007 frames and 60,828 SID writes, each on the same cycle (`docs/research/2026-10-18-packed-player-spike.md`, "Sound effects on a real 6510");
- the fixture song's verification matched x64sc on 1,554 frames and 34,681 writes per subtune, with the footstep, gunshot and clink on voice 3 every 37 frames (`fixtures/gt2-original/README.md`, Verification).

The register values below are from that verified case (subtune 1, `-D1 -H1`). The helper's writes are identical to x64sc's there, so they are x64sc's values.

- **The voice stays silent until the music's next new note.** When the effect ends, the player writes `$00` to that voice's control register and gives the voice back to the music only at the music's next new note on it. A note that was sustaining when the effect started does not come back. In the verified case:
  - the footstep starts at frame 37 and ends with `$80` at frame 43;
  - voice 3 is held at `$00` from frame 44 to 54;
  - it plays again at frame 55, the music's next new note (`$09`, then `$41`);
  - the note that was sounding when the effect started never resumes;
  - the gunshot at frame 74 does the same: it ends at frame 94, the voice is held at `$00` from 96 to 102 and returns at 103.
- **An effect plays through the music's filter.** Effect data has no filter settings at all: the converter discards them (`goattracker-sfx.md`, Conversion rules). The music keeps writing its own routing every frame. In the verified case, `$D417` stays `$84` (resonance 8, voice 3 routed to the filter) on every frame of every effect, so the effects are filtered along with the music's sweeps. This is why a voice that effects may interrupt must not use the filter (TAD §3.1, and rule 4 in §1.3).
- **The SID has no per-voice volume** (`sid.md`), so to make an effect stand out, keep the music quieter rather than the effect louder: the filter on other voices, lower sustain levels and thinner pulse widths for the music, and wide pulses and noise for the effects. The fixture's second subtune is arranged this way. Measured on reSIDfp (6581), A-weighted, against the music's voices 1 and 2, its effects are 4.8 to 9.7 dB above the music's loud moments (90th percentile) (`fixtures/gt2-original/README.md`).

What this means when arranging, for any voice a profile marks expendable (for example voice 3 in the built-in `sfx-shares-v3`):

- Keep its notes short and frequent (drums, arpeggio, texture), so the voice comes back soon after an effect, or leave it empty. Sustained notes (a bass, a drone or a held lead) can stay silent for a long time after an effect. The validator warns about long notes and about lead or bass on an expendable voice.
- Route it dry: no filter.
- Start its notes with an explicit pulse width (see the integration rules below).

## Sound-effect priority

An effect at a higher memory address is never interrupted by one at a lower address. Effect data is therefore laid out lowest priority first.

## Integration rules

- Call play from a raster interrupt that saves and restores A, X and Y and acknowledges `$D019`. With KERNAL banked out (common in games), the interrupt vector is `$FFFE/$FFFF`, not `$0314/$0315`.

- Do not call the sound-effect entry from main code while the interrupt is inside the play routine. Use a one-byte request mailbox written by the game and consumed by the interrupt just before calling play.
- With sound effects enabled, start notes on any voice that effects can steal with an explicit pulse width, so the music's next note after an effect does not inherit the effect's pulse width.

Source: GoatTracker 2 readme, sections 4.4 (GT2RELOC), 5.1 (playroutine options) and 6.3.
