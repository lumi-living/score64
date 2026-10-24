//! `decisions.toml`: round trips, errors with line numbers, and the checks.

use proptest::prelude::*;

use score64_core::{Confidence, ContentHash, Proposal, Provenance, strategies};

use super::*;

fn decision(id: &str, subtune: &str, proposal: Proposal, provenance: Provenance) -> Decision {
    Decision {
        id: DecisionId::new(id).unwrap(),
        subtune: SubtuneName::new(subtune).unwrap(),
        bar: 5,
        proposal,
        provenance,
        confidence: Confidence::new(0.9).unwrap(),
        reviewed: true,
    }
}

#[test]
fn a_file_reads_as_the_module_docs_show_it() {
    let text = r#"schema = 1

[[decision]]
id = "loop"
subtune = "main"
bar = 5
confidence = 0.9
reviewed = true

[decision.proposal]
change = "loop_row"
row = 80

[decision.provenance]
by = "manual"
"#;
    let file = DecisionsFile::parse(text).unwrap();
    assert_eq!(
        file.decision,
        [decision(
            "loop",
            "main",
            Proposal::LoopRow { row: 80 },
            Provenance::Manual
        )]
    );
    // And it writes back to the same text.
    assert_eq!(file.to_toml().unwrap(), text);
}

#[test]
fn writing_sorts_by_subtune_then_id() {
    let file = DecisionsFile::new(vec![
        decision(
            "b",
            "title",
            Proposal::LoopRow { row: 0 },
            Provenance::Manual,
        ),
        decision(
            "z",
            "main",
            Proposal::LoopRow { row: 0 },
            Provenance::Manual,
        ),
        decision(
            "a",
            "title",
            Proposal::LoopRow { row: 0 },
            Provenance::Manual,
        ),
    ]);
    let back = DecisionsFile::parse(&file.to_toml().unwrap()).unwrap();
    let order: Vec<_> = back
        .decision
        .iter()
        .map(|d| (d.subtune.as_str(), d.id.as_str()))
        .collect();
    assert_eq!(order, [("main", "z"), ("title", "a"), ("title", "b")]);
}

#[test]
fn a_missing_schema_is_1_and_a_newer_one_is_refused() {
    assert!(DecisionsFile::parse("").unwrap().decision.is_empty());
    let err = DecisionsFile::parse("schema = 2\n")
        .unwrap_err()
        .to_string();
    assert!(err.contains("schema 2 is not supported"), "{err}");
}

#[test]
fn errors_name_the_line_and_column() {
    // confidence = 3 on line 7, its value at column 14.
    let text =
        "schema = 1\n\n[[decision]]\nid = \"x\"\nsubtune = \"main\"\nbar = 0\nconfidence = 3\n";
    let err = DecisionsFile::parse(text).unwrap_err();
    assert!(
        matches!(
            &err,
            Error::Invalid {
                at: Some((7, 14)),
                ..
            }
        ),
        "{err:?}"
    );
    assert!(err.to_string().starts_with("decisions:7:14: "), "{err}");
    // An unknown field, with its line.
    let err = DecisionsFile::parse("schema = 1\nextra = true\n").unwrap_err();
    assert!(
        matches!(
            err,
            Error::Invalid {
                at: Some((2, 1)),
                ..
            }
        ),
        "{err:?}"
    );
    // Read from a file, the error names it.
    let dir = std::env::temp_dir().join(format!("score64-decisions-err-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("decisions.toml");
    std::fs::write(&path, text).unwrap();
    let shown = DecisionsFile::read(&path).unwrap_err().to_string();
    assert!(
        shown.starts_with(&format!("{}:7:14: ", path.display())),
        "{shown}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_manual_provenance_takes_no_other_fields() {
    let text = r#"[[decision]]
id = "loop"
subtune = "main"
bar = 5
confidence = 0.9
reviewed = true

[decision.proposal]
change = "loop_row"
row = 80

[decision.provenance]
by = "manual"
rule = "stray"
"#;
    let err = DecisionsFile::parse(text).unwrap_err();
    assert!(err.to_string().contains("unknown field `rule`"), "{err}");
    assert!(
        matches!(
            err,
            Error::Invalid {
                at: Some((1, 1)),
                ..
            }
        ),
        "{err:?}"
    );
}

/// ADR-014 §8: `change = "keep_note"`, with `voice`, `row` and `pitch`.
#[test]
fn a_keep_note_reads_and_writes_as_the_adr_shows_it() {
    let text = r#"schema = 1

[[decision]]
id = "keep-high-g"
subtune = "main"
bar = 2
confidence = 1.0
reviewed = true

[decision.proposal]
change = "keep_note"
voice = 1
row = 36
pitch = "G-5"

[decision.provenance]
by = "manual"
"#;
    let file = DecisionsFile::parse(text).unwrap();
    assert_eq!(
        file.decision[0].proposal,
        Proposal::KeepNote {
            voice: score64_core::Voice::new(1).unwrap(),
            row: 36,
            pitch: "G-5".parse().unwrap(),
        }
    );
    assert_eq!(file.to_toml().unwrap(), text);
    // Unknown keys stay errors: a keep names its note by row and pitch only, not by length or source.
    for extra in [
        "length = 4",
        "source = { content = \"00\", start_us = 0, end_us = 1 }",
        "reason = \"ghost\"",
    ] {
        let bad = text.replace("pitch = \"G-5\"\n", &format!("pitch = \"G-5\"\n{extra}\n"));
        let err = DecisionsFile::parse(&bad).unwrap_err().to_string();
        assert!(err.contains("unknown field"), "{extra}: {err}");
    }
    // And each of its fields is needed.
    for gone in ["voice = 1\n", "row = 36\n", "pitch = \"G-5\"\n"] {
        assert!(
            DecisionsFile::parse(&text.replace(gone, "")).is_err(),
            "without {gone}"
        );
    }
}

#[test]
fn a_duplicate_read_from_a_file_names_both_lines() {
    let one = r#"[[decision]]
id = "same"
subtune = "main"
bar = 0
confidence = 1.0
reviewed = true

[decision.proposal]
change = "loop_row"
row = 0

[decision.provenance]
by = "manual"
"#;
    let text = format!("schema = 1\n\n{one}\n{one}");
    let err = DecisionsFile::parse(&text).unwrap_err();
    assert!(
        matches!(
            err,
            Error::DuplicateId {
                lines: Some((3, 17)),
                ..
            }
        ),
        "{err:?}"
    );
    assert!(err.to_string().ends_with("(lines 3 and 17)"), "{err}");
}

#[test]
fn two_decisions_with_one_id_in_one_subtune_are_refused() {
    let twice = DecisionsFile::new(vec![
        decision(
            "same",
            "main",
            Proposal::LoopRow { row: 0 },
            Provenance::Manual,
        ),
        decision(
            "same",
            "main",
            Proposal::LoopRow { row: 16 },
            Provenance::Manual,
        ),
    ]);
    assert!(matches!(twice.to_toml(), Err(Error::DuplicateId { .. })));
    // The same id in another subtune is fine.
    let apart = DecisionsFile::new(vec![
        decision(
            "same",
            "main",
            Proposal::LoopRow { row: 0 },
            Provenance::Manual,
        ),
        decision(
            "same",
            "title",
            Proposal::LoopRow { row: 16 },
            Provenance::Manual,
        ),
    ]);
    let text = apart.to_toml().unwrap();
    assert_eq!(DecisionsFile::parse(&text).unwrap().decision.len(), 2);
}

#[test]
fn files_are_written_whole_and_read_back() {
    let dir = std::env::temp_dir().join(format!("score64-decisions-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("decisions.toml");
    let file = DecisionsFile::new(vec![decision(
        "lead",
        "main",
        Proposal::LeadSource {
            source: ContentHash([7; 32]),
        },
        Provenance::Llm {
            model: "any".into(),
            prompt: ContentHash([1; 32]),
        },
    )]);
    file.write(&path).unwrap();
    // No temporary file is left beside it.
    let left: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(left, ["decisions.toml"]);
    assert_eq!(DecisionsFile::read(&path).unwrap(), file);
    let missing = DecisionsFile::read(&dir.join("nope.toml")).unwrap_err();
    assert!(matches!(missing, Error::Io { .. }));
    assert!(missing.to_string().contains("nope.toml"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Decisions with ids unique per subtune.
fn decisions() -> impl Strategy<Value = Vec<Decision>> {
    proptest::collection::vec(strategies::decision(), 0..16).prop_map(|ds| {
        let mut seen = std::collections::BTreeSet::new();
        ds.into_iter()
            .filter(|d| seen.insert((d.subtune.clone(), d.id.clone())))
            .collect()
    })
}

proptest! {
    /// Any decisions, of every kind, write and read back equal (in file order), and writing again gives the same
    /// bytes.
    #[test]
    fn any_decisions_round_trip_byte_for_byte(ds in decisions()) {
        let file = DecisionsFile::new(ds);
        let text = file.to_toml().unwrap();
        let back = DecisionsFile::parse(&text).unwrap();
        let mut sorted = file.decision.clone();
        sorted.sort_by(|a, b| (&a.subtune, &a.id).cmp(&(&b.subtune, &b.id)));
        prop_assert_eq!(&back.decision, &sorted);
        prop_assert_eq!(back.to_toml().unwrap(), text);
    }
}
