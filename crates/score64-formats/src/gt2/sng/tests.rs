//! Song files: the fixture byte for byte, typed fields, structural and semantic errors, and round trips.

use proptest::prelude::*;

use super::*;

fn fixture() -> Vec<u8> {
    std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/gt2-original/sixteen-steps.sng"
    ))
    .unwrap()
}

#[test]
fn the_fixture_round_trips_byte_for_byte() {
    let bytes = fixture();
    let song = Sng::read(&bytes).unwrap();
    song.check().unwrap();
    assert_eq!(song.write().unwrap(), bytes);
}

#[test]
fn the_fixture_reads_as_its_annotated_dump_says() {
    // tests/snapshots/golden__sixteen-steps.sng.snap.
    let song = Sng::read(&fixture()).unwrap();
    assert_eq!(header_text(&song.name), "Sixteen Steps");
    assert_eq!(header_text(&song.author), "Lumi Living");
    assert_eq!(header_text(&song.copyright), "2026 Lumi Living, MIT");
    assert_eq!(song.subtunes.len(), 2);
    assert_eq!(
        song.subtunes[0][0].entries,
        (0x00..=0x07).collect::<Vec<u8>>()
    );
    assert_eq!(
        song.subtunes[0][2].entries,
        (0x10..=0x17).collect::<Vec<u8>>()
    );
    assert_eq!(song.subtunes[1][2].entries, [0x28; 8]);
    assert!(song.subtunes.iter().flatten().all(|o| o.restart == 0));
    assert_eq!(song.instruments.len(), 12);
    assert_eq!(song.patterns.len(), 41);
    assert!(song.patterns.iter().all(|p| p.rows.len() == 16));
}

#[test]
fn every_truncation_is_an_error_not_a_panic() {
    let bytes = fixture();
    for n in 0..bytes.len() {
        assert!(Sng::read(&bytes[..n]).is_err(), "{n} bytes");
    }
    let mut longer = bytes.clone();
    longer.push(0);
    assert_eq!(Sng::read(&longer), Err(SngError::Trailing(bytes.len())));
    let mut magic = bytes.clone();
    magic[3] = b'4';
    assert_eq!(Sng::read(&magic), Err(SngError::BadMagic));
}

#[test]
fn missing_end_marks_are_found() {
    let bytes = fixture();
    // The first orderlist's end mark, after its 8 entries.
    let mut no_ol_end = bytes.clone();
    no_ol_end[0x65 + 1 + 8] = 0x00;
    assert!(matches!(
        Sng::read(&no_ol_end),
        Err(SngError::NoOrderlistEnd(_))
    ));
    // Pattern 00's end row is its 17th, after its length byte at $2AB.
    let mut no_pat_end = bytes.clone();
    no_pat_end[0x2AB + 1 + 16 * 4] = REST;
    assert!(matches!(
        Sng::read(&no_pat_end),
        Err(SngError::NoPatternEnd(_))
    ));
}

fn invalid(song: &Sng) -> String {
    match song.check() {
        Err(SngError::Invalid(why) | SngError::TooMany(why)) => {
            assert!(song.write().is_err(), "write checks first");
            why
        }
        other => panic!("expected a check error, got {other:?}"),
    }
}

#[test]
fn check_finds_what_goattracker_refuses() {
    let good = Sng::read(&fixture()).unwrap();
    let mut s = good.clone();
    s.subtunes[0][1].entries.push(0x29);
    assert!(invalid(&s).contains("pattern 29 does not exist"));
    let mut s = good.clone();
    s.subtunes[0][1].entries.push(0xE3);
    assert!(invalid(&s).contains("not a pattern number"));
    let mut s = good.clone();
    s.subtunes[0][1].entries.push(0xD0);
    assert!(
        invalid(&s).contains("not a pattern number"),
        "the first repeat command"
    );
    let mut s = good.clone();
    s.subtunes[0][1].restart = 8;
    assert!(invalid(&s).contains("restart position 8"));
    let mut s = good.clone();
    s.subtunes[1][0].entries.clear();
    assert!(invalid(&s).contains("empty"));
    let mut s = good.clone();
    s.subtunes[0][0].entries.insert(0, END);
    assert!(invalid(&s).contains("end mark"));
    let mut s = good.clone();
    s.patterns[3].rows[5].note = 0x5F;
    assert!(invalid(&s).contains("pattern 03 row 05"));
    for past_key_on in [0xC0, END] {
        let mut s = good.clone();
        s.patterns[3].rows[5].note = past_key_on;
        assert!(invalid(&s).contains("is not a note"), "{past_key_on:02X}");
    }
    let mut s = good.clone();
    s.patterns[3].rows[5].instrument = 13;
    assert!(invalid(&s).contains("instrument 0D does not exist"));
    let mut s = good.clone();
    s.patterns[3].rows[5].command = 0x10;
    assert!(invalid(&s).contains("command"));
    let mut s = good.clone();
    s.patterns[0].rows = vec![
        Row {
            note: REST,
            ..Row::default()
        };
        MAX_ROWS + 1
    ];
    assert!(invalid(&s).contains("129 rows"));
    let mut s = good.clone();
    s.patterns[0].rows.clear();
    assert!(invalid(&s).contains("0 rows"));
    let mut s = good.clone();
    s.instruments[0].wave = s.wave.rows.len() as u8 + 1;
    assert!(invalid(&s).contains("instrument 01 points past the end of the wavetable"));
    let mut s = good.clone();
    s.instruments[2].vibrato = s.speed.rows.len() as u8 + 1;
    assert!(invalid(&s).contains("instrument 03 points past the end of the speedtable"));
    let mut s = good.clone();
    s.subtunes = vec![s.subtunes[0].clone(); MAX_SUBTUNES + 1];
    assert!(invalid(&s).contains("33 subtunes"));
    let mut s = good.clone();
    s.subtunes.clear();
    assert!(invalid(&s).contains("at least one subtune"));
    let mut s = good.clone();
    s.instruments = vec![s.instruments[0].clone(); MAX_INSTRUMENTS + 1];
    assert!(invalid(&s).contains("64 instruments"));
    let mut s = good.clone();
    s.patterns = vec![s.patterns[0].clone(); MAX_PATTERNS + 1];
    assert!(invalid(&s).contains("209 patterns"));
    let mut s = good.clone();
    s.subtunes[0][0].entries = vec![0; MAX_ORDERLIST + 1];
    assert!(invalid(&s).contains("255 orderlist entries"));
    // At the limits it is fine.
    let mut s = good;
    s.subtunes[0][0].entries = vec![0; MAX_ORDERLIST];
    s.subtunes[0][0].restart = (MAX_ORDERLIST - 1) as u8;
    s.patterns[0].rows = vec![
        Row {
            note: KEY_ON,
            ..Row::default()
        };
        MAX_ROWS
    ];
    assert!(s.check().is_ok());
}

#[test]
fn the_empty_song_is_valid_and_header_text_round_trips() {
    let mut s = Sng::empty();
    s.name = header_field("A song with a rather long name, over 31 characters");
    assert_eq!(header_text(&s.name), "A song with a rather long name,");
    assert_eq!(s.name[31], 0);
    s.author = header_field("Ünïcode");
    assert_eq!(header_text(&s.author), "?n?code");
    let bytes = s.write().unwrap();
    assert_eq!(Sng::read(&bytes).unwrap(), s);
    assert!(crate::annotate::sng(&bytes).is_ok());
}

fn row(instruments: usize) -> impl Strategy<Value = Row> {
    (
        NOTE_FIRST..=KEY_ON,
        0..=instruments as u8,
        0u8..=0x0F,
        any::<u8>(),
    )
        .prop_map(|(note, instrument, command, data)| Row {
            note,
            instrument,
            command,
            data,
        })
}

fn table() -> impl Strategy<Value = Table> {
    proptest::collection::vec(any::<(u8, u8)>(), 0..=20).prop_map(|rows| Table { rows })
}

/// Any song GoatTracker accepts, kept small.
fn song() -> impl Strategy<Value = Sng> {
    (1usize..=8, 0usize..=6, table(), table(), table(), table())
        .prop_flat_map(|(patterns, instruments, wave, pulse, filter, speed)| {
            let pointer = |t: &Table| 0..=t.rows.len() as u8;
            let instrument = (
                any::<[u8; 2]>(),
                pointer(&wave),
                pointer(&pulse),
                pointer(&filter),
                pointer(&speed),
                any::<[u8; 3]>(),
                any::<[u8; 16]>(),
            )
                .prop_map(|(adsr, w, p, f, v, rest, name)| Instrument {
                    ad: adsr[0],
                    sr: adsr[1],
                    wave: w,
                    pulse: p,
                    filter: f,
                    vibrato: v,
                    vibrato_delay: rest[0],
                    gate_timer: rest[1],
                    first_wave: rest[2],
                    name,
                });
            let orderlist = (
                proptest::collection::vec(prop_oneof![0..patterns as u8, 0xD0u8..=0xFE], 0..=6),
                0..patterns as u8,
                any::<prop::sample::Index>(),
            )
                .prop_map(|(mut entries, last, restart)| {
                    entries.push(last);
                    let restart = restart.index(entries.len()) as u8;
                    Orderlist { entries, restart }
                });
            let voices = [orderlist.clone(), orderlist.clone(), orderlist];
            (
                any::<([u8; 32], [u8; 32], [u8; 32])>(),
                proptest::collection::vec(voices, 1..=3),
                proptest::collection::vec(instrument, instruments),
                Just((wave, pulse, filter, speed)),
                proptest::collection::vec(
                    proptest::collection::vec(row(instruments), 1..=MAX_ROWS)
                        .prop_map(|rows| Pattern { rows }),
                    patterns,
                ),
            )
        })
        .prop_map(
            |(
                (name, author, copyright),
                subtunes,
                instruments,
                (wave, pulse, filter, speed),
                patterns,
            )| Sng {
                name,
                author,
                copyright,
                subtunes,
                instruments,
                wave,
                pulse,
                filter,
                speed,
                patterns,
            },
        )
}

proptest! {
    /// Any song GoatTracker accepts writes and reads back equal, writing it again gives the same bytes, and the
    /// annotator, written separately from the same layout, accounts for every byte.
    #[test]
    fn any_valid_song_round_trips(song in song()) {
        prop_assert!(song.check().is_ok(), "{:?}", song.check());
        let bytes = song.write().unwrap();
        let back = Sng::read(&bytes).unwrap();
        prop_assert_eq!(&back, &song);
        prop_assert_eq!(back.write().unwrap(), bytes.clone());
        prop_assert!(crate::annotate::sng(&bytes).is_ok());
    }

    /// Reading never panics, whatever the bytes after the magic.
    #[test]
    fn reading_any_bytes_never_panics(tail in proptest::collection::vec(any::<u8>(), 0..600)) {
        let mut bytes = SNG_MAGIC.to_vec();
        bytes.extend(tail);
        let _ = Sng::read(&bytes);
    }
}
