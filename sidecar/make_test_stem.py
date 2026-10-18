# /// script
# requires-python = ">=3.11"
# dependencies = ["numpy==2.4.6"]
# ///
"""A synthetic, rights-clear polyphonic test stem with known notes (#21).

    uv run sidecar/make_test_stem.py <out.wav> <out_notes.json>

12 s at 44.1 kHz mono: a bass line, three-note chords and a melody, played with a plucked additive tone
(harmonics with exponential decay). The notes are the ground truth, in seconds and MIDI pitch.
"""

import json
import sys
import wave

import numpy as np

SR = 44_100
BPM = 100
BEAT = 60 / BPM


def tone(pitch, dur, amp):
    t = np.arange(int(dur * SR)) / SR
    f0 = 440.0 * 2 ** ((pitch - 69) / 12)
    y = sum((0.6**k) * np.sin(2 * np.pi * f0 * (k + 1) * t) for k in range(6) if f0 * (k + 1) < SR / 2)
    env = np.minimum(1, t / 0.005) * np.exp(-t * 2.5) * np.minimum(1, (dur - t) / 0.02)
    return amp * y * env


def main():
    out_wav, out_notes = sys.argv[1], sys.argv[2]
    notes = []
    chords = [(48, [60, 64, 67]), (45, [57, 60, 64]), (41, [57, 60, 65]), (43, [55, 59, 62])]  # C Am F G
    melody = [72, 74, 76, 79, 77, 76, 74, 72, 69, 71, 72, 74, 76, 74, 72, 71]
    for bar in range(5):
        root, chord = chords[bar % 4]
        t0 = bar * 4 * BEAT
        notes.append((t0, t0 + 2 * BEAT, root, 0.5))
        notes.append((t0 + 2 * BEAT, t0 + 4 * BEAT, root, 0.5))
        for p in chord:
            notes.append((t0, t0 + 3.5 * BEAT, p, 0.25))
        for k in range(4):
            p = melody[(bar * 4 + k) % len(melody)]
            notes.append((t0 + k * BEAT, t0 + (k + 0.9) * BEAT, p, 0.35))
    length = 12.0
    y = np.zeros(int(length * SR))
    for s, e, p, a in notes:
        seg = tone(p, e - s, a)
        i = int(s * SR)
        y[i : i + len(seg)] += seg[: len(y) - i]
    y = 0.8 * y / np.max(np.abs(y))
    with wave.open(out_wav, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(SR)
        w.writeframes((y * 32767).astype("<i2").tobytes())
    truth = [{"start_s": s, "end_s": e, "pitch": p} for s, e, p, _ in sorted(notes) if s < length]
    with open(out_notes, "w") as f:
        json.dump(truth, f, indent=1)
    print(f"{out_wav}: {len(truth)} notes")


if __name__ == "__main__":
    main()
