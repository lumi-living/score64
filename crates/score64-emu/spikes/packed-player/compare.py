"""Compare score64-emu renders with VICE and sidplayfp (E3 spike, #20).

Usage (numpy and scipy via uv, nothing installed globally):

    uv run --frozen --project sidecar python crates/score64-emu/spikes/packed-player/compare.py <workdir> <tune>

<workdir> holds, for <tune> (for example "sanction_d1h1"):
    <tune>_helper_writes.tsv        score64-emu render --writes
    <tune>_vice.dmp                 vsid -sounddev dump (cycles-since-last register value)
    <tune>_helper_<chip>.wav        score64-emu render
    <tune>_vicetiming_<chip>.wav    score64-emu replay of the VICE dump (same SID engine, VICE's CPU timing)
    <tune>_sidplayfp_<chip>.wav     sidplayfp (reSIDfp)
for chip in 6581 and 8580. Prints Markdown tables.
"""

import sys
import wave
from collections import Counter
from pathlib import Path

import numpy as np
from scipy import signal

GATE_REGS = {0x04, 0x0B, 0x12}
ADSR_REGS = {0x05, 0x06, 0x0C, 0x0D, 0x13, 0x14}
FRAME = 19656


def helper_frames(path):
    frames = {}
    for line in Path(path).read_text().splitlines()[1:]:
        f, cyc, reg, val = line.split("\t")
        if f == "init":
            continue
        frames.setdefault(int(f), []).append((int(cyc), int(reg, 16), int(val, 16)))
    return [frames[k] for k in sorted(frames)]


def vice_bursts(path, gap=4000):
    bursts, cycle, last = [], 0, None
    for line in Path(path).read_text().split("\n"):
        if not line.strip():
            continue
        d, reg, val = map(int, line.split())
        cycle += d
        if last is None or cycle - last > gap:
            bursts.append([])
        bursts[-1].append((cycle, reg, val))
        last = cycle
    return bursts


def compare_writes(helper, vice):
    def seq(fr):
        return [(r, v) for _, r, v in fr]

    offset = next(o for o in range(8) if seq(vice[o]) == seq(helper[0]))
    pairs = list(zip(helper, vice[offset:]))
    same = 0
    first_diff = None
    deltas, gate_deltas, adsr_deltas = [], [], []
    for k, (h, v) in enumerate(pairs):
        if seq(h) == seq(v):
            same += 1
            h0, v0 = h[0][0], v[0][0]
            for (hc, r, _), (vc, _, _) in zip(h, v):
                d = (hc - h0) - (vc - v0)
                deltas.append(d)
                if r in GATE_REGS:
                    gate_deltas.append(d)
                if r in ADSR_REGS:
                    adsr_deltas.append(d)
        elif first_diff is None:
            first_diff = k
    # Call-time jitter: how much VICE's frame-to-frame spacing of each frame's first write differs from the
    # helper's (whose play calls are exactly FRAME apart). The player's own work before its first write cancels out.
    n = len(pairs)
    helper_first = np.array([h[0][0] for h in helper[:n]]) + FRAME * np.arange(n)
    vice_first = np.array([v[0][0] for v in vice[offset : offset + n]])
    jitter = np.diff(vice_first) - np.diff(helper_first)
    print("### SID register writes: helper vs VICE x64sc/vsid (reSID, cycle-exact 6510)\n")
    print("| Measure | Value |\n|---|---|")
    print(f"| Frames compared | {len(pairs)} (VICE frames skipped at start: {offset}, the PSID driver's own writes) |")
    print(f"| Frames with identical register/value sequence | {same} / {len(pairs)} |")
    print(f"| First differing frame | {first_diff if first_diff is not None else 'none'} |")
    for name, d in [("all writes", deltas), ("gate (control) writes", gate_deltas), ("ADSR writes", adsr_deltas)]:
        d = np.array(d)
        hist = Counter(d.tolist())
        top = ", ".join(f"{k:+d}: {v}" for k, v in sorted(hist.items(), key=lambda kv: -kv[1])[:5])
        print(f"| Intra-frame timing, {name} (helper − VICE, cycles) | mean {d.mean():+.2f}, "
              f"|max| {np.abs(d).max()}, {100 * np.mean(d == 0):.1f} % exact; most common: {top} |")
    print(f"| Play-call jitter, VICE vs helper (cycles) | min {jitter.min():+d}, max {jitter.max():+d}, "
          f"mean |Δ| {np.abs(jitter).mean():.2f} (helper calls play exactly every {FRAME}) |")
    print()


def load(path):
    with wave.open(str(path)) as w:
        x = np.frombuffer(w.readframes(w.getnframes()), dtype="<i2").astype(np.float64)
        if w.getnchannels() > 1:
            x = x.reshape(-1, w.getnchannels()).mean(axis=1)
        return x / 32768.0, w.getframerate()


def align(ref, x, sr, fine):
    # Coarse: envelopes at 1 kHz, search ±1 s. Fine (same SID engine only): waveform, ±4 ms around it. Different
    # engines start their oscillators at different phases, so their waveforms don't correlate; only envelopes do.
    step = sr // 1000
    def env(s):
        return np.abs(s[: len(s) // step * step]).reshape(-1, step).mean(axis=1)
    er, ex = env(ref), env(x)
    n = min(len(er), len(ex), 20_000)
    c = signal.correlate(er[:n] - er[:n].mean(), ex[:n] - ex[:n].mean(), mode="full", method="fft")
    lags = np.arange(-n + 1, n)
    keep = np.abs(lags) <= 1000
    coarse = lags[keep][np.argmax(c[keep])] * step
    if not fine:
        return coarse
    best, best_r = coarse, -2
    for lag in range(coarse - 2 * step * 2, coarse + 2 * step * 2 + 1):
        a, b = shift(ref, x, lag)
        m = min(len(a), len(b), sr * 10)
        r = np.corrcoef(a[:m], b[:m])[0, 1]
        if r > best_r:
            best, best_r = lag, r
    return best


def shift(ref, x, lag):
    if lag >= 0:
        a, b = ref[lag:], x
    else:
        a, b = ref, x[-lag:]
    n = min(len(a), len(b))
    return a[:n], b[:n]


def metrics(ref_path, x_path, same_engine):
    ref, sr = load(ref_path)
    x, sr2 = load(x_path)
    assert sr == sr2
    lag = align(ref, x, sr, same_engine)
    a, b = shift(ref, x, lag)
    a, b = a - a.mean(), b - b.mean()
    gain = np.dot(a, b) / np.dot(b, b) if same_engine else np.sqrt(np.dot(a, a) / np.dot(b, b))
    b = b * gain
    r = np.corrcoef(a, b)[0, 1]
    snr = 10 * np.log10(np.sum(a**2) / np.sum((a - b) ** 2))
    hop = sr // 50  # one PAL frame
    nf = len(a) // hop
    rms = lambda s: 20 * np.log10(np.sqrt((s[: nf * hop].reshape(nf, hop) ** 2).mean(axis=1)) + 1e-6)
    ea, eb = rms(a), rms(b)
    loud = ea > ea.max() - 40
    env_r = np.corrcoef(ea[loud], eb[loud])[0, 1]
    env_mad = np.mean(np.abs(ea[loud] - eb[loud]))
    f, _, sa = signal.stft(a, sr, nperseg=2048, noverlap=1536)
    _, _, sb = signal.stft(b, sr, nperseg=2048, noverlap=1536)
    ma, mb = np.abs(sa), np.abs(sb)
    floor = max(ma.max(), mb.max()) * 1e-4  # -80 dB
    da, db = 20 * np.log10(np.maximum(ma, floor)), 20 * np.log10(np.maximum(mb, floor))
    lsd = np.mean(np.sqrt(np.mean((da - db) ** 2, axis=0)))
    cen = lambda m: (f[:, None] * m).sum(axis=0) / (m.sum(axis=0) + 1e-12)
    ca, cb = cen(ma), cen(mb)
    voiced = ma.sum(axis=0) > np.percentile(ma.sum(axis=0), 20)
    cen_r = np.corrcoef(ca[voiced], cb[voiced])[0, 1]
    return dict(lag_ms=1000 * lag / sr, gain_db=20 * np.log10(abs(gain)), r=r, snr=snr, env_r=env_r,
                env_mad=env_mad, lsd=lsd, cen_r=cen_r)


def main():
    work, tune = Path(sys.argv[1]), sys.argv[2]
    compare_writes(helper_frames(work / f"{tune}_helper_writes.tsv"), vice_bursts(work / f"{tune}_vice.dmp"))
    print("### Audio (60 s, 48 kHz; the second file is aligned to the first and level-matched: least squares for the "
          "same SID engine, RMS across engines)\n")
    print("| Chip | Pair | Lag (ms) | Gain (dB) | Waveform r | Residual SNR (dB) | Envelope r | "
          "Envelope |Δ| (dB) | Log-spectral distance (dB) | Centroid r |")
    print("|---|---|---|---|---|---|---|---|---|---|")
    for chip in ("6581", "8580"):
        for ref, x, label, same in [
            ("vicetiming", "helper", "helper vs VICE timing (same SID engine)", True),
            ("sidplayfp", "helper", "helper vs sidplayfp", False),
            ("sidplayfp", "vicetiming", "VICE timing vs sidplayfp (SID engine only)", False),
        ]:
            m = metrics(work / f"{tune}_{ref}_{chip}.wav", work / f"{tune}_{x}_{chip}.wav", same)
            print(f"| {chip} | {label} | {m['lag_ms']:+.2f} | {m['gain_db']:+.1f} | {m['r']:.3f} | {m['snr']:.1f} | "
                  f"{m['env_r']:.3f} | {m['env_mad']:.2f} | {m['lsd']:.2f} | {m['cen_r']:.3f} |")


if __name__ == "__main__":
    main()
