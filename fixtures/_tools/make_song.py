"""Write the original GoatTracker 2 fixture song and its three sound-effect instruments (#18).

    python3 fixtures/_tools/make_song.py fixtures/gt2-original [--variant gunshot=b] [--variant clink=b]

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


# Sound effects: instruments only (absolute notes, waveforms <= $81, no PWM or filter; see goattracker-sfx.md).
# One wavetable row is one frame, and the effect ends (voice silenced) after the last row, so the row count is the
# effect's length. Variant "a" is the committed default; "b" exists for listening comparisons.
SFX = {
    # A bright two-frame noise scuff (where the ear is most sensitive) and a square-wave thud; the 50 % pulse
    # carries the most energy a voice can give.
    "footstep": {"a": ("SFX footstep", 0x05, 0x00,
                       [(0x81, 76), (0x81, 68), (0x41, 38), (0x41, 35), (0x80, 32)], 0x88)},
    "gunshot": {
        # A bang: bright crack, then straight down to a low noise body with a one-frame pulse thump, decaying.
        "a": ("SFX gunshot", 0x0A, 0x00,
              [(0x81, 98), (0x81, 74), (0x41, 34), (0x81, 55), (0x81, 52), (0x81, 50)] + [(0x81, 48)] * 8
              + [(0x81, 46)] * 4 + [(0x80, 44)] * 2, 0x88),
        # Heavier: two thumps and a darker body.
        "b": ("SFX gunshot", 0x0A, 0x00,
              [(0x81, 96), (0x41, 31), (0x81, 60), (0x41, 29), (0x81, 47)] + [(0x81, 45)] * 9
              + [(0x81, 43)] * 4 + [(0x80, 41)] * 2, 0x88),
    },
    "clink": {
        # Metal: a noise strike, then inharmonic pitches changing every frame on a thin pulse, fast decay.
        "a": ("SFX clink", 0x05, 0x00,
              [(0x81, 103), (0x41, 94), (0x41, 101), (0x11, 95), (0x41, 102), (0x11, 96), (0x41, 100), (0x11, 94),
               (0x41, 101), (0x40, 95)], 0x86),
        # Brighter and shorter, two alternating partials a tritone plus an octave apart.
        "b": ("SFX clink", 0x03, 0x00,
              [(0x81, 105), (0x41, 98), (0x41, 104), (0x41, 98), (0x41, 104), (0x41, 98), (0x40, 104)], 0x86),
    },
}


def build(variants=None):
    variants = variants or {}
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
    # The combat lead decays faster to a lower sustain (5 instead of 8) and sweeps a thinner pulse ($280 +/- $18
    # per tick), which carries less energy, so sound effects on voice 3 stand out.
    lead_combat = instrument("Lead combat", 0x06, 0x5A, [(0x41, 0x00)],
                             pulse=([(0x82, 0x80), (0x10, 0x18), (0x10, 0xE8)], 1), vib=(0x03, 0x18), vib_delay=0x18)
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
    # Combat filter program, started by pattern command A on the bass: low-pass on voices 1 and 2 only. It tames the
    # music so effects on the dry voice 3 stand out; there is no per-voice volume on the SID. Voice 3 itself stays
    # unfiltered in combat, because an effect does not touch the filter and would be muffled with it (TAD 1.3 rule 4).
    bass_filter = t.program(t.filter, [(0x90, 0x23), (0x00, 0x38)])

    sfx_defs = {}
    for key, options in SFX.items():
        name, ad, sr, wave, pulse_hi = options[variants.get(key, "a")]
        sfx_defs[key] = (name, ad, sr, [(w, absnote(n)) for w, n in wave], [(pulse_hi, 0x00)])
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
    combat_lead_ids = [pattern(bar_pattern(ev, lead_combat, (0x0F, TEMPO) if bar == 0 else None))
                       for bar, ev in enumerate(per_bar(c.MELODY))]
    pluck_ids = [pattern(bar_pattern(ev, bass_pluck, (0x0A, bass_filter) if bar == 0 else None))
                 for bar, ev in enumerate(per_bar(c.COMBAT_BASS))]
    drum_ins = {"kick": kick, "snare": snare, "hat": hat}
    drum_rows = [[REST, 0, 0, 0] for _ in range(c.ROWS_PER_BAR)]
    for row, d in c.DRUMS:
        drum_rows[row] = [note(60), drum_ins[d], 0, 0]
    drum_id = pattern([tuple(r) for r in drum_rows])

    orderlists = [
        # Subtune 1, "main": lead, arpeggio chords, filtered bass on voice 3.
        [(lead_ids, 0), (arp_ids, 0), (bass_ids, 0)],
        # Subtune 2, "combat": a quieter lead, filtered plucked bass on voice 2, unfiltered drums on voice 3 with short
        # notes only (after a sound effect GoatTracker returns the voice to the music at its next note, #119).
        [(combat_lead_ids, 0), (pluck_ids, 0), ([drum_id] * c.BARS, 0)],
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
    args = sys.argv[2:]
    variants = dict(args[i + 1].split("=") for i in range(0, len(args), 2) if args[i] == "--variant")
    song, ins_files = build(variants)
    (out / "sixteen-steps.sng").write_bytes(song.to_bytes())
    for key, data in ins_files.items():
        (out / f"sfx-{key}.ins").write_bytes(data)
    print(f"sixteen-steps.sng: {len(song.orderlists)} subtunes, {len(song.instruments)} instruments, "
          f"{len(song.patterns)} patterns; {len(ins_files)} effect instruments")


if __name__ == "__main__":
    main()
