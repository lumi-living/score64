# gt2-original: "Sixteen Steps" (GoatTracker 2 fixture song)

An original GoatTracker 2 song by Lumi Living, **MIT**, written by `../_tools/make_song.py` from the shared score in `../_tools/composition.py`. It is the same 8-bar piece as `sparse-melody-bass` and `dense-mix`, arranged for the SID, so a test can follow one piece from audio to packed player. It is the acceptance fixture for #116 and the reference song for #47 (the `.sng` writer).

| File | What |
|---|---|
| `sixteen-steps.sng` | The song (GTS5). Opens in GoatTracker 2.77 and SID Factory II |
| `sfx-footstep.ins`, `sfx-gunshot.ins`, `sfx-clink.ins` | The three sound effects as GoatTracker instruments (GTI5) |
| `sfx-*.bin` | The same effects as GoatTracker sound-effect data (`ins2snd2 -b`): footstep 12, gunshot 28, clink 23 bytes |

Packed `.prg`/`.sid` files are not committed, because they contain GoatTracker's player. Make them with `../_tools/verify_song.sh`, which also checks them.

## What it exercises

| Feature | Where |
|---|---|
| Hard restart | Every instrument: gate timer 2, first-frame wave `$09` (gate + test bit), packed with the default hard-restart ADSR (`-A0F00`) |
| PWM | Instrument 1 "Lead PWM": pulse table starts at `$400` and sweeps ±`$30` per tick, 32 ticks each way, looping |
| Filtered voice 3 | Subtune 1: "Bass filtered" routes voice 3 through low-pass (resonance 8) and sweeps the cutoff on every note. Subtune 2 keeps voice 3 dry: a sound effect doesn't touch the filter, so an effect on a filtered voice is muffled with it (TAD §1.3 rule 4). There the filter goes on voices 1 and 2 instead (low-pass, cutoff `$38`, pattern command `A`), to make room for the effects |
| Two subtunes | 1 "main" (profile `full`: effects are not meant to fire here): lead, arpeggio chords (wavetable arpeggios, minor and major), filtered bass on voice 3. 2 "combat" (profile `sfx-shares-v3`): the same melody on a quieter lead (faster decay, sustain 5, a thinner pulse sweep around `$280`), a plucked bass on voice 2, unfiltered drums on voice 3, with only short notes there, because after a sound effect GoatTracker gives the voice back to the music at its next note (#119) |
| Sound effects | Footstep, gunshot and metallic clink (instruments 10–12, not used in the patterns), converted with `ins2snd2`. The footstep is a bright two-frame noise scuff, then a square-wave thud (5 frames). The gunshot is a bright crack, then straight down to a low noise body with a one-frame pulse thump (20 frames). The clink is a noise strike, then inharmonic pitches that change every frame (10 frames). `make_song.py --variant gunshot=b --variant clink=b` writes the alternatives used in the listening comparison |
| Effect level | Effects must be louder than the music. Measured on reSIDfp (sidplayfp, 6581), A-weighted, in 20 ms windows, against the combat music's voices 1 and 2 (the effect replaces voice 3): footstep +4.8 dB, gunshot +6.1 dB and clink +9.7 dB above the music's loud moments (90th percentile), and at least 10 dB above its median |
| Also | Tempo command `F06` (125 BPM, docs/reference/tempo-grid.md), vibrato with a speed-table entry and delay, keyoff, drum wavetables with absolute notes |

## Verification (2026-10-18)

`KICKASS=/path/to/KickAss.jar ../_tools/verify_song.sh scratch/<dir>` packs the song with `-D1 -H1 -ZF8` and with `-ZF8` only. It renders both subtunes in score64-emu and sidplayfp on 6581 and 8580, and diffs score64-emu's SID writes against VICE:

| Build | Subtune | Against | Frames identical | Writes on the same cycle | Undocumented opcodes |
|---|---|---|---|---|---|
| `-D1 -H1` | 1, 2 | vsid | 1,554 / 1,554 each | 37,276 / 37,276 each | 0 |
| plain | 1, 2 | vsid | 1,554 / 1,554 each | 21,352 and 21,868, all | 0 |
| `-D1 -H1`, footstep / gunshot / clink on voice 3 every 37 frames | 1, 2 | x64sc + `crates/score64-emu/spikes/packed-player/sfx-driver.asm` | 1,554 / 1,554 each | 34,681 / 34,681 each | 0 |

Packed size: 2,062 bytes with `-D1 -H1`, 1,769 without. Play routine: at most 16.3 raster lines. Basic Pitch run on voice 1 alone finds all 28 melody notes at the right pitch, with onsets within 30 ms of the score (after a constant 186 ms start-up offset).

To hear the music with its effects on any SID player, `verify_song.sh` also builds `sixteen_fx.sid` (`crates/score64-emu/spikes/packed-player/sfx-psid.asm` and `make_sfx_psid.py`). It's a PSID whose own init and play routines fire the three effects on the same schedule, then call the GoatTracker player, and its SID writes match the helper's frame for frame. It renders subtune 2 with effects in sidplayfp on both chips.

## Listening

The first check found the effects drowned by the music, a gunshot like a swoosh and a clink like a beep. That led to the changes above: no filter on voice 3 in combat, a quieter combat lead and filtered voices 1 and 2, wider pulses on the effects, a new gunshot and a new clink. The open check is #121.
