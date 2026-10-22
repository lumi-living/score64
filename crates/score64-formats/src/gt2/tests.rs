//! Instrument files: the fixtures byte for byte, typed fields, errors and round trips.

use proptest::prelude::*;

use super::*;

fn fixture(name: &str) -> Vec<u8> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/gt2-original/");
    std::fs::read(format!("{path}{name}")).unwrap()
}

#[test]
fn the_fixtures_round_trip_byte_for_byte() {
    for name in ["sfx-footstep.ins", "sfx-gunshot.ins", "sfx-clink.ins"] {
        let bytes = fixture(name);
        let ins = Ins::read(&bytes).unwrap();
        assert_eq!(ins.write().unwrap(), bytes, "{name}");
    }
}

#[test]
fn the_footstep_reads_as_its_annotated_dump_says() {
    // tests/snapshots: AD 05 SR 00, wave 01, pulse 01, gate timer 2, first wave 09, "SFX footstep"; a six-row
    // wavetable and a two-row pulsetable.
    let ins = Ins::read(&fixture("sfx-footstep.ins")).unwrap();
    let i = &ins.instrument;
    assert_eq!(
        (i.ad, i.sr, i.wave, i.pulse, i.filter),
        (0x05, 0x00, 0x01, 0x01, 0x00)
    );
    assert_eq!(
        (i.vibrato, i.vibrato_delay, i.gate_timer, i.first_wave),
        (0, 0, 0x02, 0x09)
    );
    assert_eq!(i.name_text(), "SFX footstep");
    assert_eq!(ins.wave.rows.len(), 6);
    assert_eq!(ins.wave.rows[0], (0x81, 0xC0));
    assert_eq!(ins.wave.rows[5], (0xFF, 0x00));
    assert_eq!(ins.pulse.rows, [(0x88, 0x00), (0xFF, 0x00)]);
    assert!(ins.filter.rows.is_empty() && ins.speed.rows.is_empty());
}

#[test]
fn bad_files_are_refused_with_the_reason() {
    let good = fixture("sfx-footstep.ins");
    assert_eq!(Ins::read(b"GTS5"), Err(InsError::BadMagic));
    assert_eq!(Ins::read(b"GT"), Err(InsError::BadMagic));
    assert_eq!(
        Ins::read(&good[..10]),
        Err(InsError::Truncated("instrument"))
    );
    assert_eq!(
        Ins::read(&good[..32]),
        Err(InsError::Truncated("wavetable"))
    );
    let mut extra = good.clone();
    extra.push(0);
    assert_eq!(Ins::read(&extra), Err(InsError::Trailing(good.len())));
    let mut long = Ins::read(&good).unwrap();
    long.speed.rows = vec![(0, 0); 256];
    assert_eq!(long.write(), Err(InsError::TableTooLong("speedtable")));
    // A pointer past its table can't be written: the wavetable has six rows.
    let mut past = Ins::read(&good).unwrap();
    past.instrument.wave = 7;
    assert_eq!(past.write(), Err(InsError::PointerOutOfRange("wavetable")));
    past.instrument.wave = 6;
    assert!(past.write().is_ok());
    past.instrument.vibrato = 1; // the speedtable is empty
    assert_eq!(past.check(), Err(InsError::PointerOutOfRange("speedtable")));
}

#[test]
fn every_field_has_its_own_byte() {
    // A distinct value in every field and tables of different lengths, written by hand from the reference layout.
    let ins = Ins {
        instrument: Instrument {
            ad: 0x11,
            sr: 0x22,
            wave: 0x01,
            pulse: 0x02,
            filter: 0x03,
            vibrato: 0x01,
            vibrato_delay: 0x77,
            gate_timer: 0x80 | 0x05,
            first_wave: 0x99,
            name: *b"Name\0\0\0\0\0\0\0\0\0\0\0\0",
        },
        wave: Table {
            rows: vec![(0xA1, 0xB1)],
        },
        pulse: Table {
            rows: vec![(0xA2, 0xB2), (0xA3, 0xB3)],
        },
        filter: Table {
            rows: vec![(0xA4, 0xB4), (0xA5, 0xB5), (0xA6, 0xB6)],
        },
        speed: Table {
            rows: vec![(0xA7, 0xB7), (0xA8, 0xB8), (0xA9, 0xB9), (0xAA, 0xBA)],
        },
    };
    let mut expected = b"GTI5".to_vec();
    expected.extend([0x11, 0x22, 0x01, 0x02, 0x03, 0x01, 0x77, 0x85, 0x99]);
    expected.extend(b"Name\0\0\0\0\0\0\0\0\0\0\0\0");
    expected.extend([1, 0xA1, 0xB1]);
    expected.extend([2, 0xA2, 0xA3, 0xB2, 0xB3]);
    expected.extend([3, 0xA4, 0xA5, 0xA6, 0xB4, 0xB5, 0xB6]);
    expected.extend([4, 0xA7, 0xA8, 0xA9, 0xAA, 0xB7, 0xB8, 0xB9, 0xBA]);
    assert_eq!(ins.write().unwrap(), expected);
    assert_eq!(Ins::read(&expected).unwrap(), ins);
    // The annotator, written from the same layout separately, reads each field from the same byte.
    let dump = crate::annotate::ins(&expected).unwrap();
    assert!(
        dump.contains(
            "\"Name\" AD 11 SR 22 wave 01 pulse 02 filter 03 vib 01/77 gate 05 no-HR 1st 99"
        ),
        "{dump}"
    );
    assert!(dump.contains("speedtable: 4 rows"), "{dump}");
}

#[test]
fn the_songs_instruments_are_the_same_record() {
    // sixteen-steps.sng's twelve instruments, from offset $A8, use the same 25-byte record; their playback matches
    // VICE frame for frame (fixtures/gt2-original/README.md). "Lead PWM" has vibrato 01 delay $18; "Bass filtered"
    // a filter table pointer.
    let sng = fixture("sixteen-steps.sng");
    let record = |n: usize| -> Instrument {
        let at = 0xA8 + 25 * n;
        Instrument::from_bytes(sng[at..at + 25].try_into().unwrap())
    };
    let lead = record(0);
    assert_eq!(lead.name_text(), "Lead PWM");
    assert_eq!(
        (lead.ad, lead.sr, lead.wave, lead.pulse),
        (0x09, 0x8A, 0x01, 0x01)
    );
    assert_eq!(
        (
            lead.filter,
            lead.vibrato,
            lead.vibrato_delay,
            lead.gate_timer
        ),
        (0, 0x01, 0x18, 0x02)
    );
    let combat = record(1);
    assert_eq!(
        (combat.wave, combat.pulse, combat.vibrato),
        (0x03, 0x05, 0x02)
    );
    let bass = record(4);
    assert_eq!(bass.name_text(), "Bass filtered");
    assert_eq!((bass.wave, bass.pulse, bass.filter), (0x11, 0x09, 0x01));
    for n in 0..12 {
        let at = 0xA8 + 25 * n;
        assert_eq!(record(n).to_bytes(), sng[at..at + 25], "instrument {n}");
    }
}

#[test]
fn names_are_sixteen_zero_padded_bytes() {
    let mut ins = Ins::read(&fixture("sfx-clink.ins")).unwrap();
    ins.instrument.set_name("A much longer name than fits");
    assert_eq!(ins.instrument.name_text(), "A much longer na");
    ins.instrument.set_name("Bass");
    assert_eq!(&ins.instrument.name[..5], b"Bass\0");
    assert_eq!(ins.instrument.name_text(), "Bass");
    // Only printable ASCII is stored and shown; the rest becomes '?', never half a character.
    ins.instrument.set_name("Bäss\t");
    assert_eq!(&ins.instrument.name[..6], b"B?ss?\0");
    ins.instrument.name[0] = 0x07;
    assert_eq!(ins.instrument.name_text(), "??ss?");
}

fn table() -> impl Strategy<Value = Table> {
    proptest::collection::vec(any::<(u8, u8)>(), 0..=255).prop_map(|rows| Table { rows })
}

fn ins() -> impl Strategy<Value = Ins> {
    (
        any::<[u8; 9]>(),
        any::<[u8; 16]>(),
        table(),
        table(),
        table(),
        table(),
    )
        .prop_map(|(b, name, wave, pulse, filter, speed)| {
            // Pointers inside their tables (or 0), as a file the writer accepts has them.
            let inside = |p: u8, t: &Table| (usize::from(p) % (t.rows.len() + 1)) as u8;
            Ins {
                instrument: Instrument {
                    ad: b[0],
                    sr: b[1],
                    wave: inside(b[2], &wave),
                    pulse: inside(b[3], &pulse),
                    filter: inside(b[4], &filter),
                    vibrato: inside(b[5], &speed),
                    vibrato_delay: b[6],
                    gate_timer: b[7],
                    first_wave: b[8],
                    name,
                },
                wave,
                pulse,
                filter,
                speed,
            }
        })
}

proptest! {
    /// Any instrument writes and reads back equal, its bytes are 4 + 25 + 4 + 2 × rows long, and the annotator reads
    /// them too.
    #[test]
    fn any_instrument_round_trips(ins in ins()) {
        let bytes = ins.write().unwrap();
        let rows = ins.wave.rows.len() + ins.pulse.rows.len() + ins.filter.rows.len() + ins.speed.rows.len();
        prop_assert_eq!(bytes.len(), 4 + 25 + 4 + 2 * rows);
        prop_assert_eq!(Ins::read(&bytes).unwrap(), ins);
        // The annotator, written separately from the same layout, accounts for every byte the writer makes.
        prop_assert!(crate::annotate::ins(&bytes).is_ok());
    }
}
