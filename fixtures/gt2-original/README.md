# gt2-original: "Sixteen Steps" (GoatTracker 2 fixture song)

An original GoatTracker 2 song by Lumi Living, **MIT**, written by `../_tools/make_song.py` from the shared score in `../_tools/composition.py`. It is the same 8-bar piece as `sparse-melody-bass` and `dense-mix`, arranged for the SID, so a test can follow one piece from audio to packed player. It is the acceptance fixture for #116 and the reference song for #47 (the `.sng` writer).

| File | What |
|---|---|
| `sixteen-steps.sng` | The song (GTS5). Opens in GoatTracker 2.77 and SID Factory II |
| `sfx-footstep.ins`, `sfx-gunshot.ins`, `sfx-clink.ins` | The three sound effects as GoatTracker instruments (GTI5) |
| `sfx-*.bin` | The same effects as GoatTracker sound-effect data (`ins2snd2 -b`): 10, 12 and 14 bytes |

Packed `.prg`/`.sid` files are not committed, because they contain GoatTracker's player. Make them with `../_tools/verify_song.sh`, which also checks them.

## What it exercises

| Feature | Where |
|---|---|
| Hard restart | Every instrument: gate timer 2, first-frame wave `$09` (gate + test bit), packed with the default hard-restart ADSR (`-A0F00`) |
| PWM | Instrument 1 "Lead PWM": pulse table starts at `$400` and sweeps ±`$30` per tick, 32 ticks each way, looping |
| Filtered voice 3 | Subtune 1: "Bass filtered" routes voice 3 through low-pass (resonance 8) and sweeps the cutoff on every note. Subtune 2: the drums start a static low-pass program on voice 3 with pattern command `A` |
| Two subtunes | 1 "main": lead, arpeggio chords (wavetable arpeggios, minor and major), filtered bass on voice 3. 2 "combat": the same lead, a plucked bass on voice 2, filtered drums on voice 3, with only short notes there, because after a sound effect GoatTracker gives the voice back to the music at its next note (#119) |
| Sound effects | Footstep, gunshot and metallic clink (instruments 9–11, not used in the patterns), converted with `ins2snd2` |
| Also | Tempo command `F06` (125 BPM, docs/reference/tempo-grid.md), vibrato with a speed-table entry and delay, keyoff, drum wavetables with absolute notes |

## Verification (2026-10-18)

`KICKASS=/path/to/KickAss.jar ../_tools/verify_song.sh scratch/<dir>` packs the song with `-D1 -H1 -ZF8` and with `-ZF8` only. It renders both subtunes in score64-emu and sidplayfp on 6581 and 8580, and diffs score64-emu's SID writes against VICE:

| Build | Subtune | Against | Frames identical | Writes on the same cycle | Undocumented opcodes |
|---|---|---|---|---|---|
| `-D1 -H1` | 1, 2 | vsid | 1,554 / 1,554 each | 37,276 / 37,276 each | 0 |
| plain | 1, 2 | vsid | 1,554 / 1,554 each | 21,352 and 21,868, all | 0 |
| `-D1 -H1`, footstep / gunshot / clink on voice 3 every 37 frames | 1, 2 | x64sc + `crates/score64-emu/spikes/packed-player/sfx-driver.asm` | 1,554 / 1,554 each | 35,962 / 35,962 each | 0 |

Packed size: 1,933 bytes with `-D1 -H1`, 1,640 without. Play routine: at most 16.3 raster lines. Basic Pitch run on voice 1 alone finds all 28 melody notes at the right pitch, with onsets within 30 ms of the score (after a constant 186 ms start-up offset).
