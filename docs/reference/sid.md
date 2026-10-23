# SID (MOS 6581 / 8580)

Base address `$D400`. Registers `$D400–$D418` are write-only; `$D419–$D41C` are read-only.

## Per-voice registers (voice 1 at `$D400`, voice 2 at `$D407`, voice 3 at `$D40E`)

| Offset | Register |
|---|---|
| +0, +1 | Frequency, low / high (16-bit) |
| +2, +3 | Pulse width, low 8 bits / high 4 bits (12-bit) |
| +4 | Control: bit 0 gate, 1 sync, 2 ring modulation, 3 test, 4 triangle, 5 sawtooth, 6 pulse, 7 noise |
| +5 | Attack (high nybble) / Decay (low nybble) |
| +6 | Sustain level (high nybble) / Release (low nybble) |

## Filter and volume

| Address | Register |
|---|---|
| `$D415` | Cutoff, low 3 bits |
| `$D416` | Cutoff, high 8 bits (11-bit cutoff) |
| `$D417` | Resonance (high nybble) / routing: bit 0 voice 1, 1 voice 2, 2 voice 3, 3 external input |
| `$D418` | Bit 4 low-pass, 5 band-pass, 6 high-pass, 7 voice 3 off; low nybble master volume |

## Read-only

| Address | Register |
|---|---|
| `$D419`, `$D41A` | Paddle X / Y |
| `$D41B` | Voice 3 oscillator output (upper 8 bits) |
| `$D41C` | Voice 3 envelope output |

## Envelope timing

Attack: 2, 8, 16, 24, 38, 56, 68, 80, 100, 250, 500, 800 ms, 1, 3, 5, 8 s (values 0–15).
Decay and release: 6, 24, 48, 72, 114, 168, 204, 240, 300, 750 ms, 1.5, 2.4, 3, 9, 15, 24 s (values 0–15; three times the attack time).
Sustain is a level (0–15), not a time.

## After the release

After the gate goes off and the release reaches zero, the voice isn't silent: the oscillator keeps running and a little of the waveform leaks to the output, a faint ghost of the last note. Selecting no waveform silences it. GoatTracker's command `7XY` sets the waveform register; it has no effect while a wavetable program is still writing the waveform (GoatTracker 2 readme, "Command 7XY"). Score64's opt-in `silence_after_release` writes `7 00` once the release has fully decayed (#186).

Measured in the emulator helper (reSIDfp, as sidplayfp sets it up), 10 fresh helper processes per cell. The note is played at full sustain, gated off with release 0, and 0.2 s later the waveform register is left alone or set as shown. Levels are the RMS of 0.5 s windows with the DC taken out, in dBFS. "Settled" is the loudest window from 1 s to 4 s after the write. The helper isn't deterministic: identical input gives slightly different samples in each process (#207). So each cell is the range over the 10 runs, with how many of them were exactly silent.

| Chip | Waveform | Left alone (the ghost) | `$00` settled | `$08` (test bit) settled |
|---|---|---|---|---|
| 6581 | pulse | −60.2 to −59.6 | silent (10/10) | silent (9/10), else −114.9 |
| 6581 | saw, triangle, noise | −64.7 to −64.5 | silent (10/10) | silent (10/10) |
| 8580 | pulse | −69.2 to −67.2 | −102.8 to −90.2 (0/10) | −90.9 to silent (1/10) |
| 8580 | saw | −73.4 to −71.8 | −90.8 to silent (4/10) | −90.6 to silent (5/10) |
| 8580 | triangle | −73.2 to −72.1 | −93.7 to silent (5/10) | −91.1 to silent (5/10) |
| 8580 | noise | −73.5 to −72.0 | −90.4 to silent (2/10) | −91.7 to silent (4/10) |

**What the table shows:**
- Either value takes the ghost away: at least 17 dB down on the 8580, and to silence on the 6581.
- On the 8580 both values first fade through a bump in the first second, as the waveform output floats down: −72.7 to −69.7 dBFS for pulse and saw, −80.4 to −75.2 for triangle, −84.4 to −79.3 for noise.
- The two values settle in the same range; the helper shows no level difference between them.

Score64 uses `$00`. The test bit doesn't only silence: it resets the oscillator and holds it at zero, and it resets the noise generator, while set (MOS 6581 datasheet, control register bit 3). `$00` leaves both alone, and the next note's own waveform takes over from there.

`crates/score64-adapters/tests/silence_after_release.rs` checks this end to end, through an exported song packed by gt2reloc 2.73:
- **Without the setting,** the gap after a key-off keeps the ghost: −60 to −73 dBFS.
- **With it,** the gap is exactly silent in most runs. Otherwise, it's −98 dBFS or lower on the 6581, and −90 dBFS or lower on the 8580, over the gap's last 3 s.
- **Its checks:** below −85 dBFS, and 15 dB under the ghost, on both chips.

**When it's written.** A release lasts at most 756 envelope rate periods: 255 steps, the lower ones slowed by the exponential counter. Add one wrap of the 15-bit rate counter, which the "ADSR bug" can add before the first step. The periods (9, 32, 63, … 31,251 cycles by nibble) are reSIDfp's, as in reSID. That's 300.8 ms for release 8, against the datasheet's 300.

GoatTracker clears the gate when it fetches the key-off row, the instrument's gate timer of frames before that row (`gplay.c`, the early fetch), so the count starts there.

A wavetable step that writes a waveform after `7XY` undoes it, so the export also waits for the note's wavetable program to stop.

## Hard restart

Envelope state carries over between notes and can make a new note start late or silent (the "ADSR bug"). Players avoid it with a **hard restart**: gate off with fast release a couple of frames before the next note, optionally set the test bit for one frame, then start the note. GoatTracker's packed player does this per instrument; gt2reloc's `-Axx` sets the hard-restart ADSR value (default `0F00`). Score64 instrument presets must state their hard-restart behaviour.

In GoatTracker 2 (readme §3.3, "HR/Gate Timer"; `player.s` and `gplay.c`) an instrument's **gate timer** (0–63) is how many frames before the next row the player fetches that row. If the row holds a new note, the gate goes off then:

- The **timing** comes from the instrument playing in the row before the new note. The player reloads the timer from the channel's instrument at the start of every row.
- **Whether** the gate goes off comes from the new note's instrument. Bit `$40` disables gate-off, which makes a legato instrument. Bit `$80` disables only the hard-restart ADSR write; the gate still goes off.
- Tie notes (toneportamento `3xx`) get neither.
- The gate timer must be at most the tempo minus 1 on every row the instrument plays; "in case of illegal (too high) gateoff timer values, the song playback is stopped". Tempos below 3 are funktempo, which the score doesn't model.

Validator rule 9 checks this:

- **Error:** a gate timer not below a row's frames per row.
- **Error:** a gate-off at or before the start of the note it cuts. With legal gate timers this can only happen alongside the error above, because one row is always longer than the gate timer.
- **Warning:** a note left with only one or two gated frames. That's legal but probably sounds clipped.
- **Warning:** a fresh note straight after another whose instrument has gate-off disabled (`$40`). The gate is still on and nothing turns it off, so the note won't retrigger and plays legato. Mark it `Legato`, or use an instrument that gates off.

The gated time is counted from the attack that started the note, through any legato notes tied to it. Notes after silence, legato notes and the first note on a voice have nothing to cut.

## Frequency

`register = f_out × 2^24 / clock`; PAL clock 985 248 Hz, NTSC 1 022 727 Hz.

## Constraints that shape arrangements

- Three voices, one note each at a time.
- **One filter** shared by all voices; **one master volume**; **no per-voice volume** (loudness comes from envelope and waveform).
- Combined waveforms and filter response differ between chips: the 6581 filter varies between individual chips and is non-linear; the 8580's is more consistent. Test filter-dependent sounds on both.
- Sample playback through `$D418` is loud on the 6581 and nearly silent on an unmodified 8580; Score64 does not rely on it.

Sources: Commodore 64 Programmer's Reference Guide (SID chapter); reSID documentation.
