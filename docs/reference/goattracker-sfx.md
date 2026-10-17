# GoatTracker sound-effect data

Produced from GoatTracker instruments by `INS2SND2` (GoatTracker 2 utility) or by Score64's writer, which must produce identical bytes for the same instrument.

## Layout

| Offset | Size | Field |
|---|---|---|
| +0 | 1 | Attack / Decay |
| +1 | 1 | Sustain / Release |
| +2 | 1 | Pulse width, **nybbles reversed** relative to the editor (middle pulse `$800` is stored as `$08`); written to both `$D402` and `$D403` |
| +3 | n | Wavetable: note/waveform pairs, order differs from the instrument wavetable. A waveform may be omitted when unchanged because the value ranges do not overlap |

Wavetable values:

| Value | Meaning |
|---|---|
| `$00` | End of effect |
| `$01–$81` | Waveform |
| `$82–$DF` | Absolute note D-0 … B-7 |

A note cannot be omitted to store a waveform change alone.

## Conversion rules (errors)

- Result larger than 128 bytes.
- Instrument wavetable with relative notes, absolute notes C-0 or C#0, or waveforms above `$81`.
- Pulse-width modulation and filter settings of the instrument are discarded.

## Score64 additions

- Pitch variants (for example footsteps) are produced at runtime by the game copying the effect and offsetting note bytes (`$82–$DF` only; waveform bytes and the terminator untouched). Score64's bench previews the same transform.
- The resident effect block is ordered by priority (lowest first), see `player.md`.

Source: GoatTracker 2 readme, sections 4.1 and 6.3.
