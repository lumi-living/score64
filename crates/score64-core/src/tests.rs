//! Model tests: value ranges, file round trips in JSON and TOML, byte-stable output, and rejection of unknown fields.

use std::num::{NonZeroU8, NonZeroU32};

use proptest::prelude::*;
use serde::{Deserialize, Serialize};

use crate::strategies;
use crate::*;

fn hash(b: u8) -> ContentHash {
    ContentHash([b; 32])
}

/// A small song with one of everything, for the snapshot and the field tests.
fn example() -> (Song, Vec<Decision>) {
    let n = |row, length, pitch: &str, attack| Note {
        row,
        length: NonZeroU32::new(length).unwrap(),
        pitch: pitch.parse().unwrap(),
        attack,
        source: None,
        confidence: Confidence::CERTAIN,
        decision: None,
    };
    let mut lead = vec![
        n(0, 4, "A-4", Attack::Fresh),
        n(4, 2, "C-5", Attack::Legato),
        n(8, 8, "E-5", Attack::Fresh),
    ];
    lead[0].source = SourceRef::new(hash(0xAB), 1_250_000, 1_500_000);
    lead[0].confidence = Confidence::new(0.9).unwrap();
    lead[2].decision = DecisionId::new("octave-bar0");
    let song = Song {
        schema: Schema::default(),
        profile: Profile::Combat,
        subtunes: vec![Subtune {
            frames_per_row: NonZeroU8::new(6).unwrap(),
            tempo: vec![TempoChange {
                row: 16,
                frames_per_row: NonZeroU8::new(5).unwrap(),
            }],
            rows_per_bar: NonZeroU8::new(16).unwrap(),
            length: NonZeroU32::new(32).unwrap(),
            loop_row: 0,
            voices: vec![VoiceLine {
                voice: Voice::new(1).unwrap(),
                role: Role::Lead,
                instrument: PresetId("lead-saw".into()),
                notes: lead,
                arps: vec![],
                marks: vec![0, 16],
            }],
        }],
    };
    let decisions = vec![
        Decision {
            id: DecisionId::new("octave-bar0").unwrap(),
            subtune: 0,
            bar: 0,
            proposal: Proposal::Octave {
                voice: Voice::new(1).unwrap(),
                shift: 1,
            },
            provenance: Provenance::Llm {
                model: "claude-opus-5-5".into(),
                prompt: hash(0x11),
            },
            confidence: Confidence::new(0.7).unwrap(),
            reviewed: false,
        },
        Decision {
            id: DecisionId::new("loop").unwrap(),
            subtune: 0,
            bar: 0,
            proposal: Proposal::LoopRow { row: 0 },
            provenance: Provenance::Manual,
            confidence: Confidence::CERTAIN,
            reviewed: true,
        },
    ];
    (song, decisions)
}

/// `decisions.toml`'s shape, for TOML round trips (TOML needs a table at the root).
#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DecisionsFile {
    decision: Vec<Decision>,
}

#[test]
fn pitches_use_goattracker_names_and_range() {
    assert_eq!("C-0".parse::<Pitch>().unwrap(), Pitch::MIN);
    assert_eq!("G#7".parse::<Pitch>().unwrap(), Pitch::MAX);
    assert_eq!("A-4".parse::<Pitch>().unwrap().midi(), 69);
    assert_eq!(Pitch::from_midi(69).unwrap().to_string(), "A-4");
    for bad in ["A-7", "B-7", "H-4", "C 4", "c-4", "C-", "C#10", "é-1"] {
        assert!(bad.parse::<Pitch>().is_err(), "{bad}");
    }
    assert_eq!(Pitch::from_midi(11), None);
    assert_eq!(Pitch::new(93), None);
    for s in 0..=92 {
        let p = Pitch::new(s).unwrap();
        assert_eq!(p.to_string().parse::<Pitch>().unwrap(), p);
    }
}

#[test]
fn ranges_are_checked_on_read() {
    let err = |json: &str| serde_json::from_str::<Note>(json).unwrap_err().to_string();
    let note = |field: &str| {
        let mut v = serde_json::to_value(&example().0.subtunes[0].voices[0].notes[0]).unwrap();
        let (k, x) = field.split_once('=').unwrap();
        v[k] = serde_json::from_str(x).unwrap();
        v.to_string()
    };
    assert!(
        err(&note("length=0")).contains("nonzero"),
        "{}",
        err(&note("length=0"))
    );
    assert!(err(&note(r#"pitch="A-7""#)).contains("C-0 to G#7"));
    assert!(err(&note("confidence=1.5")).contains("from 0 to 1"));
    assert!(err(&note(r#"attack="soft""#)).contains("unknown variant"));
    assert!(err(&note(r#"source={"content":"00","start_us":0,"end_us":1}"#)).contains("blake3"));
    let empty = format!(
        r#"source={{"content":"{}","start_us":5,"end_us":5}}"#,
        hash(1)
    );
    assert!(err(&note(&empty)).contains("empty"));
    assert!(err(&note(r#"decision="Bad Id""#)).contains("decision id"));
    assert!(serde_json::from_str::<Voice>("4").is_err());
    assert!(serde_json::from_str::<Voice>("0").is_err());
}

#[test]
fn newer_schema_versions_are_rejected() {
    let mut v = serde_json::to_value(example().0).unwrap();
    v["schema"] = 2.into();
    let e = serde_json::from_value::<Song>(v).unwrap_err().to_string();
    assert!(e.contains("score schema 2 is not supported"), "{e}");
}

#[test]
fn unknown_fields_are_errors_at_every_level() {
    let (song, decisions) = example();
    let song = serde_json::to_value(&song).unwrap();
    let decision = serde_json::to_value(&decisions[0]).unwrap();
    for path in [
        "",
        "/subtunes/0",
        "/subtunes/0/tempo/0",
        "/subtunes/0/voices/0",
        "/subtunes/0/voices/0/notes/0",
        "/subtunes/0/voices/0/notes/0/source",
    ] {
        let mut v = song.clone();
        v.pointer_mut(path).unwrap()["colour"] = "blue".into();
        let e = serde_json::from_value::<Song>(v).unwrap_err().to_string();
        assert!(e.contains("unknown field `colour`"), "{path}: {e}");
    }
    for path in ["", "/proposal", "/provenance"] {
        let mut v = decision.clone();
        v.pointer_mut(path).unwrap()["colour"] = "blue".into();
        let e = serde_json::from_value::<Decision>(v)
            .unwrap_err()
            .to_string();
        assert!(e.contains("colour"), "{path}: {e}");
    }
}

#[test]
fn files_look_like_this() {
    let (song, decisions) = example();
    insta::assert_snapshot!("song_json", serde_json::to_string_pretty(&song).unwrap());
    insta::assert_snapshot!(
        "decisions_toml",
        toml::to_string(&DecisionsFile {
            decision: decisions
        })
        .unwrap()
    );
}

#[test]
fn only_unreviewed_model_decisions_need_review() {
    let (_, decisions) = example();
    assert!(decisions[0].needs_review());
    assert!(!decisions[1].needs_review(), "manual");
    let reviewed = Decision {
        reviewed: true,
        ..decisions[0].clone()
    };
    assert!(!reviewed.needs_review());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// Reading back what was written gives the same song, and writing it again gives the same bytes.
    #[test]
    fn songs_round_trip_byte_for_byte(song in strategies::song()) {
        let json = serde_json::to_string_pretty(&song).unwrap();
        let back: Song = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(&back, &song);
        prop_assert_eq!(serde_json::to_string_pretty(&back).unwrap(), json);

        let toml = toml::to_string(&song).unwrap();
        let back: Song = toml::from_str(&toml).unwrap();
        prop_assert_eq!(&back, &song);
        prop_assert_eq!(toml::to_string(&back).unwrap(), toml);
    }

    #[test]
    fn decisions_round_trip_byte_for_byte(decision in proptest::collection::vec(strategies::decision(), 0..8)) {
        let file = DecisionsFile { decision };
        let toml = toml::to_string(&file).unwrap();
        let back: DecisionsFile = toml::from_str(&toml).unwrap();
        prop_assert_eq!(&back, &file);
        prop_assert_eq!(toml::to_string(&back).unwrap(), toml);
        let json = serde_json::to_string(&file.decision).unwrap();
        prop_assert_eq!(serde_json::from_str::<Vec<Decision>>(&json).unwrap(), file.decision);
    }

    /// The generators keep their promises, so property tests elsewhere can rely on them.
    #[test]
    fn generated_lines_are_ordered_and_do_not_overlap(subtune in strategies::subtune()) {
        for line in &subtune.voices {
            let mut end = 0;
            for (i, n) in line.notes.iter().enumerate() {
                prop_assert!(n.row >= end);
                prop_assert!(n.attack == Attack::Fresh || (i > 0 && n.row == end));
                end = n.row + n.length.get();
            }
            prop_assert!(end <= subtune.length.get());
        }
    }
}

#[cfg(feature = "schema")]
#[test]
fn schemas_carry_ranges_and_reject_unknown_fields() {
    let s = serde_json::to_value(schemars::schema_for!(Song)).unwrap();
    let text = s.to_string();
    assert_eq!(s["additionalProperties"], false);
    assert!(text.contains(r#""maximum":3"#), "voice range");
    assert!(text.contains("G#"), "pitch pattern");
    let d = serde_json::to_value(schemars::schema_for!(Decision))
        .unwrap()
        .to_string();
    assert!(d.contains("\"llm\"") && d.contains("\"loop_row\""), "{d}");
}
