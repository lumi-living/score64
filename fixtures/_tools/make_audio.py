"""Render Score64's rights-clear audio and MIDI fixtures (#18). Everything is synthesised here from
composition.py and fixed random seeds; no samples or third-party recordings are involved (MIT).

    uv run --frozen --project sidecar python fixtures/_tools/make_audio.py fixtures

Writes source.wav (44.1 kHz, 16-bit mono) and, where the material has notes, source.mid into:
    sparse-melody-bass/   lead + bass, 8 bars
    dense-mix/            lead, bass, sustained chords, 16th-note arpeggio, drums and a short echo, 8 bars
    drum-loop/            kick, snare, closed hat, 2 bars
    ambience-bed/         wind-like noise, a low drone and sparse drips, 10 s
    sfx-footstep/ sfx-gunshot/ sfx-clink/   single sound-effect references
"""

import pathlib
import struct
import sys
import wave

import numpy as np

sys.path.insert(0, str(pathlib.Path(__file__).parent))
import composition as c  # noqa: E402

SR = 44_100


def hz(midi):
    return 440.0 * 2 ** ((midi - 69) / 12)


def env_adsr(n, a, d, s, r_start, r):
    """Linear attack, exponential decay to sustain, release from r_start (all in seconds)."""
    t = np.arange(n) / SR
    e = np.where(t < a, t / max(a, 1e-6), s + (1 - s) * np.exp(-(t - a) / max(d, 1e-6)))
    rel = t >= r_start
    e[rel] = e[rel] * np.exp(-(t[rel] - r_start) / max(r, 1e-6))
    return e


def voice(pitch, dur, kind, amp):
    tail = 0.25
    n = int((dur + tail) * SR)
    t = np.arange(n) / SR
    f = hz(pitch)
    if kind == "lead":  # soft square-ish lead with slight vibrato
        ph = 2 * np.pi * f * t + 0.004 * np.sin(2 * np.pi * 5.5 * t) * f
        y = sum(np.sin((2 * k + 1) * ph) / (2 * k + 1) for k in range(5))
        e = env_adsr(n, 0.01, 0.15, 0.7, dur, 0.08)
    elif kind == "bass":
        y = np.sin(2 * np.pi * f * t) + 0.35 * np.sin(4 * np.pi * f * t) + 0.12 * np.sin(6 * np.pi * f * t)
        e = env_adsr(n, 0.005, 0.25, 0.6, dur, 0.06)
    elif kind == "pad":
        y = sum(np.sin(2 * np.pi * f * (1 + d) * t) for d in (-0.003, 0.0, 0.003)) / 3
        e = env_adsr(n, 0.15, 0.4, 0.8, dur, 0.3)
    elif kind == "pluck":
        y = sum((0.55**k) * np.sin(2 * np.pi * f * (k + 1) * t) for k in range(6))
        e = env_adsr(n, 0.002, 0.12, 0.0, dur, 0.05)
    else:
        raise ValueError(kind)
    return amp * y * e


def drum(name, rng):
    if name == "kick":
        n = int(0.35 * SR)
        t = np.arange(n) / SR
        f = 45 + 110 * np.exp(-t * 30)
        return np.sin(2 * np.pi * np.cumsum(f) / SR) * np.exp(-t * 9)
    if name == "snare":
        n = int(0.25 * SR)
        t = np.arange(n) / SR
        noise = rng.standard_normal(n)
        return 0.6 * noise * np.exp(-t * 18) + 0.5 * np.sin(2 * np.pi * 185 * t) * np.exp(-t * 25)
    if name == "hat":
        n = int(0.06 * SR)
        t = np.arange(n) / SR
        noise = rng.standard_normal(n)
        hp = noise - np.concatenate([[0.0], noise[:-1]])  # crude high-pass
        return 0.35 * hp * np.exp(-t * 70)
    raise ValueError(name)


def place(buf, start_s, y):
    i = int(round(start_s * SR))
    if i >= len(buf):
        return
    m = min(len(y), len(buf) - i)
    buf[i : i + m] += y[:m]


def render_notes(buf, events, kind, amp):
    for row, length, pitch in events:
        place(buf, row * c.SECONDS_PER_ROW, voice(pitch, length * c.SECONDS_PER_ROW, kind, amp))


def render_drums(buf, events, rng, amp=1.0):
    names = {v: k for k, v in c.GM_DRUMS.items()}
    for row, _, note in events:
        place(buf, row * c.SECONDS_PER_ROW, amp * drum(names[note], rng))


def write_wav(path, y, peak=0.8):
    y = peak * y / max(np.max(np.abs(y)), 1e-9)
    with wave.open(str(path), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(SR)
        w.writeframes(np.round(y * 32767).astype("<i2").tobytes())


def write_midi(path, tracks):
    """Format 1 SMF, 480 ticks per beat. tracks: list of (name, channel, [(start row, length rows, pitch, velocity)])."""
    tpq = 480
    ticks_per_row = tpq // c.ROWS_PER_BEAT

    def vlq(v):
        out = [v & 0x7F]
        v >>= 7
        while v:
            out.append(0x80 | (v & 0x7F))
            v >>= 7
        return bytes(reversed(out))

    def chunk(kind, data):
        return kind + struct.pack(">I", len(data)) + data

    tempo = int(round(60_000_000 / c.BPM))
    conductor = b"\x00\xff\x51\x03" + tempo.to_bytes(3, "big") + b"\x00\xff\x58\x04\x04\x02\x18\x08" + b"\x00\xff\x2f\x00"
    out = [chunk(b"MTrk", conductor)]
    for name, ch, notes in tracks:
        evs = []
        for row, length, pitch, vel in notes:
            evs.append((row * ticks_per_row, 1, bytes([0x90 | ch, pitch, vel])))
            evs.append(((row + length) * ticks_per_row, 0, bytes([0x80 | ch, pitch, 0])))
        evs.sort(key=lambda e: (e[0], e[1]))
        data = b"\x00\xff\x03" + vlq(len(name)) + name.encode()
        now = 0
        for tick, _, msg in evs:
            data += vlq(tick - now) + msg
            now = tick
        data += b"\x00\xff\x2f\x00"
        out.append(chunk(b"MTrk", data))
    header = chunk(b"MThd", struct.pack(">HHH", 1, len(out), tpq))
    pathlib.Path(path).write_bytes(header + b"".join(out))


def main():
    root = pathlib.Path(sys.argv[1])
    piece_s = c.BARS * c.ROWS_PER_BAR * c.SECONDS_PER_ROW
    melody, bass, chords = c.events(c.MELODY), c.events(c.BASS), c.chord_events()
    arps = [(bar * 16 + r, 1, root + 12 + iv)
            for bar, (root, kind) in enumerate(c.CHORDS)
            for r, iv in zip(range(16), [*c.CHORD_INTERVALS[kind], 12] * 4)]

    # Sparse melody + bass.
    d = root / "sparse-melody-bass"
    d.mkdir(parents=True, exist_ok=True)
    y = np.zeros(int((piece_s + 1) * SR))
    render_notes(y, melody, "lead", 0.5)
    render_notes(y, bass, "bass", 0.45)
    write_wav(d / "source.wav", y)
    write_midi(d / "source.mid", [("Lead", 0, [(*e, 100) for e in melody]), ("Bass", 1, [(*e, 100) for e in bass])])

    # Dense mix.
    rng = np.random.default_rng(1801)
    d = root / "dense-mix"
    d.mkdir(parents=True, exist_ok=True)
    y = np.zeros(int((piece_s + 1) * SR))
    render_notes(y, melody, "lead", 0.45)
    render_notes(y, bass, "bass", 0.45)
    render_notes(y, chords, "pad", 0.16)
    render_notes(y, arps, "pluck", 0.12)
    drums = c.drum_events()
    render_drums(y, drums, rng, 0.5)
    delay = int(0.36 * SR)  # a dotted-eighth echo, like a busy production
    y[delay:] += 0.25 * y[:-delay]
    write_wav(d / "source.wav", y)
    write_midi(d / "source.mid", [
        ("Lead", 0, [(*e, 100) for e in melody]), ("Bass", 1, [(*e, 100) for e in bass]),
        ("Chords", 2, [(*e, 70) for e in chords]), ("Arpeggio", 3, [(*e, 60) for e in arps]),
        ("Drums", 9, [(*e, 100) for e in drums]),
    ])

    # Drum loop.
    rng = np.random.default_rng(1802)
    d = root / "drum-loop"
    d.mkdir(parents=True, exist_ok=True)
    drums = c.drum_events(bars=2)
    y = np.zeros(int((2 * 16 * c.SECONDS_PER_ROW) * SR))
    render_drums(y, drums, rng)
    write_wav(d / "source.wav", y)
    write_midi(d / "source.mid", [("Drums", 9, [(*e, 100) for e in drums])])

    # Ambience bed: brown-ish wind with slow swells, a low drone, sparse drips.
    rng = np.random.default_rng(1803)
    d = root / "ambience-bed"
    d.mkdir(parents=True, exist_ok=True)
    n = 10 * SR
    t = np.arange(n) / SR
    wind = np.cumsum(rng.standard_normal(n))
    wind -= np.convolve(wind, np.ones(4410) / 4410, mode="same")  # remove drift
    wind *= 0.5 + 0.5 * np.sin(2 * np.pi * 0.13 * t) ** 2
    drone = 0.25 * np.sin(2 * np.pi * 55 * t) + 0.12 * np.sin(2 * np.pi * 82.5 * t + 0.4 * np.sin(2 * np.pi * 0.2 * t))
    y = wind / np.max(np.abs(wind)) + drone
    for start in rng.uniform(0.5, 9.5, 7):
        m = int(0.15 * SR)
        tt = np.arange(m) / SR
        place(y, start, 0.3 * np.sin(2 * np.pi * (1800 - 900 * tt / 0.15) * tt) * np.exp(-tt * 40))
    write_wav(d / "source.wav", y, peak=0.6)

    # Sound-effect references.
    rng = np.random.default_rng(1804)
    d = root / "sfx-footstep"
    d.mkdir(parents=True, exist_ok=True)
    m = int(0.4 * SR)
    tt = np.arange(m) / SR
    # Heel then toe: a soft low thump and a grittier second contact 70 ms later, both with gravel crunch.
    thump = np.sin(2 * np.pi * (70 + 40 * np.exp(-tt * 40)) * tt) * np.exp(-tt * 35)
    crunch = rng.standard_normal(m) * (rng.uniform(size=m) > 0.6)
    heel = 0.35 * thump + 0.5 * crunch * np.exp(-tt * 28)
    toe = np.zeros(m)
    lag = int(0.07 * SR)
    toe[lag:] = 0.45 * crunch[: m - lag] * np.exp(-tt[: m - lag] * 22) + 0.15 * thump[: m - lag]
    write_wav(d / "source.wav", heel + toe)

    d = root / "sfx-gunshot"
    d.mkdir(parents=True, exist_ok=True)
    m = int(0.9 * SR)
    tt = np.arange(m) / SR
    crack = rng.standard_normal(m) * np.exp(-tt * 45)
    boom = np.sin(2 * np.pi * (50 + 80 * np.exp(-tt * 20)) * tt) * np.exp(-tt * 6)
    tail = np.convolve(rng.standard_normal(m), np.ones(40) / 40, mode="same") * np.exp(-tt * 5)
    write_wav(d / "source.wav", crack + 0.8 * boom + 0.6 * tail)

    d = root / "sfx-clink"
    d.mkdir(parents=True, exist_ok=True)
    m = int(0.35 * SR)
    tt = np.arange(m) / SR
    # Two small metal objects knocking: a hard broadband strike, then many inharmonic partials that die within
    # about 100 ms (a bell rings for seconds on a few partials; a clink does not).
    partials = [(1.0, 1.0), (1.47, 0.8), (2.09, 0.7), (2.56, 0.6), (3.39, 0.55), (4.17, 0.45), (5.03, 0.4),
                (6.11, 0.3), (7.29, 0.25), (8.84, 0.2)]
    base = 1650.0
    y = sum(a * np.sin(2 * np.pi * base * r * tt + rng.uniform(0, 2 * np.pi)) * np.exp(-tt * (28 + 9 * r))
            for r, a in partials)
    strike = rng.standard_normal(m)
    strike -= np.concatenate([[0.0], strike[:-1]])  # bright
    y += 1.2 * strike * np.exp(-tt * 220)
    y[int(0.045 * SR):] += 0.5 * y[: m - int(0.045 * SR)] * np.exp(-tt[: m - int(0.045 * SR)] * 10)  # small rebound
    write_wav(d / "source.wav", y)
    print("wrote", ", ".join(p.name for p in sorted(root.iterdir()) if (p / "source.wav").exists()))


if __name__ == "__main__":
    main()
