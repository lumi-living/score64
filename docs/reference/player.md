# GoatTracker 2 packed player

Score64 targets the packed player that GoatTracker 2's relocator (`gt2reloc`, or the editor's F9 packer) produces. Pin the GoatTracker version used for packing in the project and in CI fixtures.

## Jump table (relative to the player start address)

| Offset | Routine | Call |
|---|---|---|
| +0 | Init | `LDA #subtune` · `JSR start` |
| +3 | Play | `JSR start+3` once per frame (50 Hz PAL), from the same raster interrupt every frame |
| +6 | Play sound effect (only when packed with sound-effect support) | `LDA #<effect` · `LDY #>effect` · `LDX #channel` (0, 7 or 14 for voices 1–3) · `JSR start+6` |
| +6 or +9 | Set master volume (only with volume-change support; +9 when sound effects are also enabled) | `LDA #volume (0–15)` · `JSR …` |

## gt2reloc options used by Score64

`gt2reloc <song.sng> <out.prg|.sid|.bin> [options]`

| Option | Meaning | Score64 use |
|---|---|---|
| `-Dx` | Sound-effect support on/off (implies buffered SID writes) | Always `-D1` for game builds |
| `-Hx` | Store author info at start+$20…+$3F and enable timing marks | `-H1` for adaptive sets |
| `-Wxx` | Player start address, high byte (default `10` = `$1000`) | From project config |
| `-Zxx` | Zero-page location (needs 2 consecutive bytes; default `FC`) | From the game's zero-page map. The default `$FC–$FD` overlaps the common `$FB–$FE` pointer convention used by many C64 programs, so Score64 always passes `-Z` explicitly |
| `-Axx` | Hard-restart ADSR value (default `0F00`) | From the instrument preset set |
| `-Bx` | Buffered SID writes | Implied by `-D1` |
| `-Cx` | Zero-page ghost registers | Avoid with sound effects (shared register state can leave a wrong pulse width after an effect) |
| `-Jx` | Full buffering | Avoid with sound effects (same reason) |
| `-Ex` | Volume-change support | Not used (master volume would also silence effects) |
| `-P` / `-N` | PAL (default) / NTSC timing | PAL |

## Timing marks

With author info enabled, a master-volume command `DXY` with parameter `$10–$FF` is a **timing mark**: the parameter is copied to start+$3F when playback reaches it. Score64 writes a mark on every bar with alternating values so each bar registers as a change; the game switches subtunes when the byte changes.

## Sound-effect priority

An effect at a higher memory address is never interrupted by one at a lower address. Effect data is therefore laid out lowest priority first.

## Integration rules

- Call play from a raster interrupt that saves and restores A, X and Y and acknowledges `$D019`. With KERNAL banked out (common in games), the interrupt vector is `$FFFE/$FFFF`, not `$0314/$0315`.

- Do not call the sound-effect entry from main code while the interrupt is inside the play routine. Use a one-byte request mailbox written by the game and consumed by the interrupt just before calling play.
- With sound effects enabled, start notes on any voice that effects can steal with an explicit pulse width, so a note resuming after an effect does not inherit the effect's pulse width.

Source: GoatTracker 2 readme, sections 4.4 (GT2RELOC), 5.1 (playroutine options) and 6.3.
