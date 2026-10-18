"""Summarise cpu-conformance output by opcode class (see ADR-010)."""

import sys

LEGAL = {int(x, 16) for x in """
00 01 05 06 08 09 0a 0d 0e 10 11 15 16 18 19 1d 1e 20 21 24 25 26 28 29 2a 2c 2d 2e 30 31 35 36 38 39 3d 3e
40 41 45 46 48 49 4a 4c 4d 4e 50 51 55 56 58 59 5d 5e 60 61 65 66 68 69 6a 6c 6d 6e 70 71 75 76 78 79 7d 7e
81 84 85 86 88 8a 8c 8d 8e 90 91 94 95 96 98 99 9a 9d a0 a1 a2 a4 a5 a6 a8 a9 aa ac ad ae b0 b1 b4 b5 b6 b8 b9 ba bc bd be
c0 c1 c4 c5 c6 c8 c9 ca cc cd ce d0 d1 d5 d6 d8 d9 dd de e0 e1 e4 e5 e6 e8 e9 ea ec ed ee f0 f1 f5 f6 f8 f9 fd fe
""".split()}
JAM = {int(x, 16) for x in "02 12 22 32 42 52 62 72 92 b2 d2 f2".split()}
UNSTABLE = {int(x, 16) for x in "8b ab 93 9f 9c 9e 9b".split()}  # ANE LXA SHA SHY SHX TAS
STABLE = set(range(256)) - LEGAL - JAM - UNSTABLE
DECIMAL = {int(x, 16) for x in "61 65 69 6d 71 75 79 7d e1 e5 e9 ed f1 f5 f9 fd".split()}

rows = [line.rstrip("\n").split("\t") for line in open(sys.argv[1])]
names = [h.split(":")[0] for h in rows[0][1:]]
data = {
    int(r[0], 16): [tuple(int(v.rstrip("p")) for v in c.split("/")) for c in r[1 : 1 + len(names)]]
    for r in rows[1:]
}


def agg(ops, k):
    n = len(ops) * 10000
    s = sum(data[o][k][0] for o in ops)
    c = sum(data[o][k][1] for o in ops)
    b = sum(data[o][k][2] for o in ops)
    p = sum(data[o][k][3] for o in ops)
    return f"state {100 * s / n:6.2f}%  cycles {100 * c / n:6.2f}%  bus {100 * b / n:5.1f}%  panics {p}"


for k, name in enumerate(names):
    print(f"== {name}")
    for label, ops in [
        ("documented (151)", LEGAL),
        ("decimal ADC/SBC (16)", DECIMAL),
        (f"stable illegal ({len(STABLE)})", STABLE),
        ("unstable illegal (7)", UNSTABLE),
    ]:
        print(f"  {label:22s} {agg(ops, k)}")
    imperfect = sorted(o for o in LEGAL | STABLE if min(data[o][k][:2]) < 10000)
    print("  imperfect:", " ".join(f"{o:02x}" for o in imperfect))
