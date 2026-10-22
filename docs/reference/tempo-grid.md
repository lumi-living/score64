# Tempo grid (PAL and NTSC)

The player advances once per frame, and a tracker row lasts a whole number of frames (`FramesPerRow`, 3–127; GoatTracker treats lower values as funktempo, which the score does not model). With R rows per beat:

`BPM = frame_rate × 60 / (R × frames_per_row)`

The frame rate depends on the target machine's video standard (`score64_core::VideoStandard`, PAL by default, #139). Each standard has two rates that matter.

## PAL

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

## NTSC

- **Nominal, 60 Hz.** GoatTracker shows frames per row, not BPM, and its `750 / frames_per_row` is PAL's convention. For NTSC, Score64 quotes nominal tempos at the rounded 60 Hz by the same rule, `900 / frames_per_row` BPM at four rows per beat. That's a project convention, not a tracker's.
- **Real NTSC, 59.826 Hz.** An NTSC frame (VIC-II 6567R8) is 17,095 cycles: 263 raster lines of 65 cycles, which VICE measures (below) and which VICE's `c64.h` also gives (`C64_NTSC_SCREEN_LINES` 263, `C64_NTSC_CYCLES_PER_LINE` 65). The 1,022,727 Hz clock isn't measured; it's derived:
  - The NTSC master crystal is 14.31818 MHz, four times the 3.579545 MHz colour subcarrier. Divided by 14, that gives the CPU clock.
  - VICE rounds it to 1,022,730 Hz (`C64_NTSC_CYCLES_PER_SEC`). The difference is in the sixth significant figure, 0.0003 %.
  - The early 6567R56A (VICE's `C64_NTSCOLD`, 262 lines of 64 cycles) isn't supported. An NTSC C64 therefore plays every tempo **0.290 % slower** than its 60 Hz nominal value, and a song timed for PAL plays **19.4 % faster** (59.826 / 50.1246).

| Frames / row | Nominal BPM (4 rows per beat) | Real NTSC BPM |
|---|---|---|
| 12 | 75 | 74.78 |
| 10 | 90 | 89.74 |
| 9 | 100 | 99.71 |
| 8 | 112.5 | 112.17 |
| 7 | 128.6 | 128.20 |
| 6 | 150 | 149.57 |
| 5 | 180 | 179.48 |
| 4 | 225 | 224.35 |

**Evidence (VICE 3.10, 2026-10-23), for the cycles per frame.**

What was run:
- The tune was `fixtures/gt2-original/sixteen-steps.sng`, packed as a PSID.
- It was played under `vsid -pal -keepenv` and under `vsid -ntsc -keepenv`, with `-sounddev dump -limitcycles 3000000 -tune 1`.
- `-keepenv` is needed: without it, the PSID header puts the machine back to PAL.

The dump gives the cycle count of every SID write. We counted the cycles between successive bursts of writes; a burst starts after a pause of more than 2,000 cycles, and there's one per play call. The first gap, which follows init, is left out.

| | Median gap | Frames measured | Most common gaps |
|---|---|---|---|
| PAL | 19,656 cycles | 151 | 19,655–19,657 |
| NTSC | 17,095 cycles | 174 | 17,094–17,096 |

Single gaps vary by a few tens of cycles, and a few by several hundred. A play call's first write doesn't always land at the same point in the call, which is why the median is quoted.

The emulator helper (`score64-emu`) runs PAL only, so an NTSC target can't be rendered or auditioned yet (#139).

## Snapping

`score64_core::grid::snap_tempo(bpm, rows_per_beat, standard)` snaps a detected tempo to the row length whose **real** tempo on that standard is nearest. The same source can snap differently: 125 BPM is 6 frames per row on PAL (125.31) and 7 on NTSC (128.20). It reports:

- the nominal and real tempos;
- the relative error, positive when the C64 runs faster;
- the drift in milliseconds per minute of source.

Material made at a nominal tracker tempo snaps to its row length exactly, but still drifts by the 0.249 % on PAL (about 149 ms per minute) or the −0.290 % on NTSC (about 174 ms per minute behind). That matters only when the music must stay in step with something running on another clock, such as video or a recorded part.

## Beat grid

Source times become rows through a beat grid (`grid::BeatGrid`): beat times from the beat tracker, R rows per beat, and rows spread evenly between beats. Tempo changes and rubato in the source then cost at most half a row per note, never accumulated drift.
