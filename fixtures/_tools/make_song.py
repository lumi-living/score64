"""Write the original GoatTracker 2 fixture song and its three sound-effect instruments (#18).

    python3 fixtures/_tools/make_song.py fixtures/gt2-original

Writes sixteen-steps.sng and sfx-footstep.ins, sfx-gunshot.ins, sfx-clink.ins. Everything here is Lumi Living's
own work under MIT. What the song exercises is listed in fixtures/gt2-original/README.md.
"""

import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).parent))
import composition as c  # noqa: E402
from gtsng import Instrument, Song, Table, instrument_file  # noqa: E402

REST, KEYOFF = 0xBD, 0xBE
TEMPO = 6  # frames per row; F06 on the first row of channel 1


def note(midi: int) -> int:
    """GoatTracker note byte: $60 is C-0 (MIDI 12)."""
    n = 0x60 + midi - 12
    assert 0x60 <= n <= 0xBC, midi
    return n


def absnote(midi: int) -> int:
    """Wavetable absolute note: $80 + semitone from C-0 ($81 = C#0 ... $DF = B-7)."""
    n = 0x80 + midi - 12
    assert 0x82 <= n <= 0xDF, midi  # also valid as sound-effect data (D-0 upwards)
    return n


class Tables:
    def __init__(self):
        self.wave, self.pulse, self.filter, self.speed = Table(), Table(), Table(), Table()

    def program(self, table: Table, rows, loop_to=None):
        """Append rows ending in a jump (to loop_to, an index into rows, or stop). Returns the 1-based start."""
        start = len(table.left) + 1
        for left, right in rows:
            table.add(left, right)
        table.add(0xFF, 0 if loop_to is None else start + loop_to)
        return start


def build():
    t = Tables()
    ins = []

    def instrument(name, ad, sr, wave, pulse=None, filt=None, vib=None, vib_delay=0, gate_timer=2):
        w = t.program(t.wave, *wave) if isinstance(wave, tuple) else t.program(t.wave, wave)
        p = (t.program(t.pulse, *pulse) if isinstance(pulse, tuple) else t.program(t.pulse, pulse)) if pulse else 0
        f = (t.program(t.filter, *filt) if isinstance(filt, tuple) else t.program(t.filter, filt)) if filt else 0
        v = t.speed.add(*vib) if vib else 0
        ins.append(Instrument(ad, sr, w, p, f, v, vib_delay if vib else 0, gate_timer, 0x09, name))
        return len(ins)

    # Lead: pulse with a looping pulse-width sweep (PWM) and delayed vibrato.
    lead = instrument("Lead PWM", 0x09, 0x8A, [(0x41, 0x00)],
                      pulse=([(0x84, 0x00), (0x20, 0x30), (0x20, 0xD0)], 1), vib=(0x03, 0x18), vib_delay=0x18)
    # Chords as wavetable arpeggios (sawtooth), looping over the chord tones.
    arp = {
        kind: instrument(f"Arp {name}", 0x06, 0x8B,
                         ([(0x21, 0x00)] + [(0x00, i) for i in (*iv[1:], 12)] + [(0x00, 0x00)], 1))
        for kind, name, iv in (("m", "minor", c.CHORD_INTERVALS["m"]), ("M", "major", c.CHORD_INTERVALS["M"]))
    }
    # Bass on voice 3 through the low-pass filter, cutoff swept up and down on every note.
    bass_filtered = instrument("Bass filtered", 0x05, 0xC9, [(0x41, 0x00)], pulse=[(0x86, 0x00)],
                               filt=([(0x90, 0x84), (0x00, 0x28), (0x30, 0x01), (0x30, 0xFF)], 2))
    bass_pluck = instrument("Bass pluck", 0x0A, 0x00, [(0x41, 0x00)], pulse=[(0x83, 0x00)])
    kick = instrument("Kick", 0x00, 0xF8,
                      [(0x81, absnote(76)), (0x41, absnote(52)), (0x41, absnote(48)), (0x41, absnote(44)),
                       (0x40, absnote(40))], pulse=[(0x88, 0x00)])
    snare = instrument("Snare", 0x00, 0xF8,
                       [(0x81, absnote(88)), (0x41, absnote(54)), (0x41, absnote(48)), (0x80, absnote(88)),
                        (0x80, absnote(85))], pulse=[(0x88, 0x00)])
    hat = instrument("Hat", 0x00, 0x84, [(0x81, absnote(100)), (0x80, absnote(100))])
    # Filter program for the combat drums on voice 3, started by pattern command A.
    drum_filter = t.program(t.filter, [(0x90, 0x64), (0x00, 0x70)])

    # Sound effects (instruments only: absolute notes, waveforms <= $81, no PWM or filter; see goattracker-sfx.md).
    sfx_defs = {
        "footstep": ("SFX footstep", 0x04, 0x00, [(0x81, absnote(42)), (0x41, absnote(30)), (0x80, absnote(26))],
                     [(0x84, 0x00)]),
        "gunshot": ("SFX gunshot", 0x09, 0x00,
                    [(0x81, absnote(n)) for n in (104, 94, 84, 76, 68, 60, 52)], [(0x88, 0x00)]),
        "clink": ("SFX clink", 0x03, 0x00,
                  [(w, absnote(n)) for w, n in ((0x41, 97), (0x11, 105), (0x41, 101), (0x11, 97), (0x41, 105))],
                  [(0x82, 0x00)]),
    }
    sfx = {}
    for key, (name, ad, sr, wave, pulse) in sfx_defs.items():
        sfx[key] = instrument(name, ad, sr, wave, pulse=pulse)

    patterns = []

    def pattern(rows):
        patterns.append(rows)
        return len(patterns) - 1

    def bar_pattern(events, ins_no, first_cmd=None):
        rows = [(REST, 0, 0, 0)] * c.ROWS_PER_BAR
        rows = [list(r) for r in rows]
        for row, length, pitch in events:
            rows[row] = [note(pitch), ins_no, 0, 0]
            end = row + length
            if end < c.ROWS_PER_BAR and rows[end][0] == REST and not any(r == end for r, _, _ in events):
                rows[end] = [KEYOFF, 0, 0, 0]
        if first_cmd:
            rows[0][2], rows[0][3] = first_cmd
        return [tuple(r) for r in rows]

    def per_bar(part):
        return [[(row, length, pitch) for b, row, length, pitch in part if b == bar] for bar in range(c.BARS)]

    lead_pats = [bar_pattern(ev, lead, (0x0F, TEMPO) if bar == 0 else None)
                 for bar, ev in enumerate(per_bar(c.MELODY))]
    lead_ids = [pattern(p) for p in lead_pats]
    arp_ids = [pattern(bar_pattern([(0, c.ROWS_PER_BAR, root)], arp[kind])) for root, kind in c.CHORDS]
    bass_ids = [pattern(bar_pattern(ev, bass_filtered)) for ev in per_bar(c.BASS)]
    pluck_ids = [pattern(bar_pattern(ev, bass_pluck)) for ev in per_bar(c.COMBAT_BASS)]
    drum_ins = {"kick": kick, "snare": snare, "hat": hat}
    drum_rows = [[REST, 0, 0, 0] for _ in range(c.ROWS_PER_BAR)]
    for row, d in c.DRUMS:
        drum_rows[row] = [note(60), drum_ins[d], 0, 0]
    drum_rows[0][2], drum_rows[0][3] = 0x0A, drum_filter  # AXY: start the drum filter program
    drum_id = pattern([tuple(r) for r in drum_rows])

    orderlists = [
        # Subtune 1, "main": lead, arpeggio chords, filtered bass on voice 3.
        [(lead_ids, 0), (arp_ids, 0), (bass_ids, 0)],
        # Subtune 2, "combat": lead, plucked bass, filtered drums on voice 3 (short notes only: after a sound effect
        # GoatTracker returns the voice to the music at its next note, docs/reference/player.md).
        [(lead_ids, 0), (pluck_ids, 0), ([drum_id] * c.BARS, 0)],
    ]
    song = Song("Sixteen Steps", "Lumi Living", "2026 Lumi Living, MIT", orderlists, ins,
                [t.wave, t.pulse, t.filter, t.speed], patterns)

    # Standalone .ins files: the instrument with only its own table rows, pointers rebased to 1.
    ins_files = {}
    for key, (name, ad, sr, wave, pulse) in sfx_defs.items():
        w, p = Table(), Table()
        for l, r in wave:
            w.add(l, r)
        w.add(0xFF, 0)
        for l, r in pulse:
            p.add(l, r)
        p.add(0xFF, 0)
        ins_files[key] = instrument_file(Instrument(ad, sr, 1, 1, 0, 0, 0, 2, 0x09, name), [w, p, Table(), Table()])
    return song, ins_files


def main():
    out = pathlib.Path(sys.argv[1])
    out.mkdir(parents=True, exist_ok=True)
    song, ins_files = build()
    (out / "sixteen-steps.sng").write_bytes(song.to_bytes())
    for key, data in ins_files.items():
        (out / f"sfx-{key}.ins").write_bytes(data)
    print(f"sixteen-steps.sng: {len(song.orderlists)} subtunes, {len(song.instruments)} instruments, "
          f"{len(song.patterns)} patterns; {len(ins_files)} effect instruments")


if __name__ == "__main__":
    main()
