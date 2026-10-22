# Tempo grid (PAL)

The player advances once per frame, and a tracker row lasts a whole number of frames (`FramesPerRow`, 3–127; GoatTracker treats lower values as funktempo, which the score does not model). With R rows per beat:

`BPM = frame_rate × 60 / (R × frames_per_row)`

Two frame rates matter:

- **Nominal, 50 Hz.** This is what trackers quote. GoatTracker's tempo at four rows per beat is `750 / frames_per_row`.
- **Real PAL, 50.1246 Hz.** A PAL frame is 19,656 cycles of the 985,248 Hz CPU clock: 312 raster lines of 63 cycles. The emulator helper uses the same numbers (`score64_formats::emu_protocol::PAL_CYCLES_PER_FRAME`, `PAL_CLOCK_HZ`), and VICE's PAL machine does too. A PAL C64 therefore plays every tempo **0.249 % faster** than its nominal value.

| Frames / row | Nominal BPM (4 rows per beat) | Real PAL BPM |
|---|---|---|
| 12 | 62.5 | 62.66 |
| 10 | 75 | 75.19 |
| 9 | 83.3 | 83.54 |
| 8 | 93.75 | 93.98 |
| 7 | 107.1 | 107.41 |
| 6 | 125 | 125.31 |
| 5 | 150 | 150.37 |
| 4 | 187.5 | 187.97 |

`score64_core::grid::snap_tempo(bpm, rows_per_beat)` snaps a detected tempo to the row length whose **real** PAL tempo is nearest. It reports:

- the nominal and real tempos;
- the relative error, positive when the C64 runs faster;
- the drift in milliseconds per minute of source.

Material made at a nominal tracker tempo snaps to its row length exactly, but still drifts by the 0.249 %: about 149 ms per minute. That matters only when the music must stay in step with something running on another clock, such as video or a recorded part.

Source times become rows through a beat grid (`grid::BeatGrid`): beat times from the beat tracker, R rows per beat, and rows spread evenly between beats. Tempo changes and rubato in the source then cost at most half a row per note, never accumulated drift.
