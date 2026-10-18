"""Minimal GoatTracker 2 song (GTS5) and instrument (GTI5) reader/writer, written from the format description in the
GoatTracker 2.77 readme (section 6). Used to generate the Score64 fixture song; not the product's writer (#47).

    python3 gtsng.py --roundtrip song.sng     # parse and re-serialise an existing song, check it is byte-identical
"""

import sys
from dataclasses import dataclass, field


@dataclass
class Instrument:
    ad: int
    sr: int
    wave: int = 0
    pulse: int = 0
    filter: int = 0
    vibrato: int = 0
    vibrato_delay: int = 0
    gate_timer: int = 2
    first_wave: int = 0x09
    name: str = ""

    def to_bytes(self) -> bytes:
        b = bytes([self.ad, self.sr, self.wave, self.pulse, self.filter, self.vibrato, self.vibrato_delay,
                   self.gate_timer, self.first_wave])
        return b + self.name.encode("ascii")[:16].ljust(16, b"\0")


@dataclass
class Table:
    left: list = field(default_factory=list)
    right: list = field(default_factory=list)

    def add(self, l, r) -> int:
        """Append a row, return its 1-based position."""
        self.left.append(l)
        self.right.append(r)
        return len(self.left)

    def to_bytes(self) -> bytes:
        assert len(self.left) == len(self.right) <= 255
        return bytes([len(self.left)]) + bytes(self.left) + bytes(self.right)


@dataclass
class Song:
    name: str
    author: str
    copyright: str
    orderlists: list  # [subtune][channel] -> (list of bytes before RST, restart position)
    instruments: list
    tables: list  # wave, pulse, filter, speed
    patterns: list  # list of list of (note, instrument, command, data); the $FF end row is added on write

    def to_bytes(self) -> bytes:
        out = bytearray(b"GTS5")
        for s in (self.name, self.author, self.copyright):
            out += s.encode("ascii")[:32].ljust(32, b"\0")
        out.append(len(self.orderlists))
        for subtune in self.orderlists:
            assert len(subtune) == 3
            for data, restart in subtune:
                # The length counts the entries and the $FF endmark, not the restart position after it.
                out.append(len(data) + 1)
                out += bytes(data) + bytes([0xFF, restart])
        out.append(len(self.instruments))
        for ins in self.instruments:
            out += ins.to_bytes()
        assert len(self.tables) == 4
        for t in self.tables:
            out += t.to_bytes()
        out.append(len(self.patterns))
        for rows in self.patterns:
            full = list(rows) + [(0xFF, 0, 0, 0)]
            assert len(full) <= 129
            out.append(len(full))
            for r in full:
                out += bytes(r)
        return bytes(out)


def parse(data: bytes) -> Song:
    assert data[:4] == b"GTS5", data[:4]
    p = 4
    strs = []
    for _ in range(3):
        strs.append(data[p : p + 32].rstrip(b"\0").decode("latin-1"))
        p += 32
    n_sub = data[p]; p += 1
    orderlists = []
    for _ in range(n_sub):
        sub = []
        for _ in range(3):
            n = data[p]; p += 1
            body = list(data[p : p + n + 1]); p += n + 1
            assert body[-2] == 0xFF, body
            sub.append((body[:-2], body[-1]))
        orderlists.append(sub)
    n_ins = data[p]; p += 1
    instruments = []
    for _ in range(n_ins):
        f = data[p : p + 9]
        name = data[p + 9 : p + 25].rstrip(b"\0").decode("latin-1")
        instruments.append(Instrument(*f, name=name))
        p += 25
    tables = []
    for _ in range(4):
        n = data[p]; p += 1
        tables.append(Table(list(data[p : p + n]), list(data[p + n : p + 2 * n])))
        p += 2 * n
    n_pat = data[p]; p += 1
    patterns = []
    for _ in range(n_pat):
        m = data[p]; p += 1
        rows = [tuple(data[p + 4 * i : p + 4 * i + 4]) for i in range(m)]
        p += 4 * m
        assert rows[-1][0] == 0xFF, rows[-1]
        patterns.append(rows[:-1])
    assert p == len(data), (p, len(data))
    return Song(*strs, orderlists, instruments, tables, patterns)


def instrument_file(ins: Instrument, tables) -> bytes:
    """GTI5: the instrument plus the four tables it uses (pointers as stored)."""
    return b"GTI5" + ins.to_bytes() + b"".join(t.to_bytes() for t in tables)


if __name__ == "__main__":
    if sys.argv[1] == "--roundtrip":
        raw = open(sys.argv[2], "rb").read()
        song = parse(raw)
        again = song.to_bytes()
        print(f"{sys.argv[2]}: {len(song.orderlists)} subtunes, {len(song.instruments)} instruments, "
              f"{len(song.patterns)} patterns, round trip {'identical' if again == raw else 'DIFFERS'}")
        sys.exit(0 if again == raw else 1)
