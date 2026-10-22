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
}

#[test]
fn names_are_sixteen_zero_padded_bytes() {
    let mut ins = Ins::read(&fixture("sfx-clink.ins")).unwrap();
    ins.instrument.set_name("A much longer name than fits");
    assert_eq!(ins.instrument.name_text(), "A much longer na");
    ins.instrument.set_name("Bass");
    assert_eq!(&ins.instrument.name[..5], b"Bass\0");
    assert_eq!(ins.instrument.name_text(), "Bass");
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
        .prop_map(|(b, name, wave, pulse, filter, speed)| Ins {
            instrument: Instrument {
                ad: b[0],
                sr: b[1],
                wave: b[2],
                pulse: b[3],
                filter: b[4],
                vibrato: b[5],
                vibrato_delay: b[6],
                gate_timer: b[7],
                first_wave: b[8],
                name,
            },
            wave,
            pulse,
            filter,
            speed,
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
