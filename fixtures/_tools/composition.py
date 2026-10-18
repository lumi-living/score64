"""The Score64 fixture composition ("Sixteen Steps", Lumi Living, MIT): one 8-bar score shared by the GoatTracker song,
the MIDI files and the rendered audio fixtures, so tests can follow one piece from audio to SID.

Times are in tracker rows: 16 rows per bar, 4 rows per beat, 125 BPM (tempo 6 at 50 Hz, docs/reference/tempo-grid.md).
Pitches are MIDI note numbers.
"""

BPM = 125
ROWS_PER_BAR = 16
ROWS_PER_BEAT = 4
BARS = 8
SECONDS_PER_ROW = 60 / BPM / ROWS_PER_BEAT

# Lead melody: (bar, row in bar, length in rows, pitch). A minor.
MELODY = [
    (0, 0, 4, 69), (0, 4, 2, 72), (0, 6, 6, 76), (0, 12, 4, 74),
    (1, 0, 4, 72), (1, 4, 4, 69), (1, 8, 4, 65), (1, 12, 4, 69),
    (2, 0, 4, 67), (2, 4, 2, 72), (2, 6, 6, 76), (2, 12, 4, 79),
    (3, 0, 4, 77), (3, 4, 2, 76), (3, 6, 2, 74), (3, 8, 8, 71),
    (4, 0, 4, 69), (4, 4, 2, 72), (4, 6, 6, 76), (4, 12, 4, 81),
    (5, 0, 4, 79), (5, 4, 4, 77), (5, 8, 4, 76), (5, 12, 4, 72),
    (6, 0, 8, 71), (6, 8, 4, 68), (6, 12, 4, 71),
    (7, 0, 12, 76),
]

# One chord per bar: (root pitch, "m" or "M"). Am F C G Am F E E.
CHORDS = [(57, "m"), (53, "M"), (60, "M"), (55, "M"), (57, "m"), (53, "M"), (52, "M"), (52, "M")]
CHORD_INTERVALS = {"m": (0, 3, 7), "M": (0, 4, 7)}

# Bass: two half notes per bar, the last bar walks up to the leading tone.
BASS = [
    (bar, row, 8, pitch)
    for bar, pitches in enumerate([(45, 45), (41, 41), (48, 48), (43, 43), (45, 45), (41, 41), (40, 40), (40, 44)])
    for row, pitch in zip((0, 8), pitches)
]

# Combat bass (subtune 2): eighth-note pluck on root, octave, root, fifth.
COMBAT_BASS = [
    (bar, row, 2, root - 12 + offset)
    for bar, (root, _) in enumerate(CHORDS)
    for row, offset in zip((0, 4, 8, 12), (0, 12, 0, 7))
]

# Drum loop, one bar, repeated: (row, drum). Kick, snare, closed hat.
DRUMS = [(0, "kick"), (2, "hat"), (4, "snare"), (6, "hat"), (8, "kick"), (10, "kick"), (12, "snare"), (14, "hat")]
GM_DRUMS = {"kick": 36, "snare": 38, "hat": 42}


def events(part, bars=BARS):
    """(start row, length rows, pitch) over the whole piece for a (bar, row, len, pitch) list."""
    return [(bar * ROWS_PER_BAR + row, length, pitch) for bar, row, length, pitch in part if bar < bars]


def chord_events():
    """Sustained chord tones, one bar each."""
    return [
        (bar * ROWS_PER_BAR, ROWS_PER_BAR, root + i)
        for bar, (root, kind) in enumerate(CHORDS)
        for i in CHORD_INTERVALS[kind]
    ]


def drum_events(bars=BARS):
    return [(bar * ROWS_PER_BAR + row, 1, GM_DRUMS[d]) for bar in range(bars) for row, d in DRUMS]
