# Tempo grid at 50 Hz (PAL)

The player advances once per frame. A tracker row lasts a whole number of frames. With four rows per beat (one row = a 16th note):

`BPM = 3000 / (4 × frames_per_row) = 750 / frames_per_row` (PAL frame rate rounded to 50 Hz; the true rate is about 50.12 Hz)

| Frames / row | BPM |
|---|---|
| 12 | 62.5 |
| 10 | 75 |
| 9 | 83.3 |
| 8 | 93.75 |
| 7 | 107.1 |
| 6 | 125 |
| 5 | 150 |
| 4 | 187.5 |

Source material produced at one of these tempos snaps to the grid without drift. `score64` snaps a detected tempo to the nearest row length and reports the error.
