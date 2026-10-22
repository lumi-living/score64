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

The gated time is counted from the attack that started the note, through any legato notes tied to it. Notes after silence, legato notes and the first note on a voice have nothing to cut.

## Frequency

`register = f_out × 2^24 / clock`; PAL clock 985 248 Hz, NTSC 1 022 727 Hz.

## Constraints that shape arrangements

- Three voices, one note each at a time.
- **One filter** shared by all voices; **one master volume**; **no per-voice volume** (loudness comes from envelope and waveform).
- Combined waveforms and filter response differ between chips: the 6581 filter varies between individual chips and is non-linear; the 8580's is more consistent. Test filter-dependent sounds on both.
- Sample playback through `$D418` is loud on the 6581 and nearly silent on an unmodified 8580; Score64 does not rely on it.

Sources: Commodore 64 Programmer's Reference Guide (SID chapter); reSID documentation.
