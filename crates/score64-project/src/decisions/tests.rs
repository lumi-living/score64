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
fn errors_name_the_line() {
    let text =
        "schema = 1\n\n[[decision]]\nid = \"x\"\nsubtune = \"main\"\nbar = 0\nconfidence = 3\n";
    let err = DecisionsFile::parse(text).unwrap_err();
    match &err {
        Error::Invalid {
            at: Some((line, _)),
            message,
            ..
        } => {
            assert!(*line >= 3, "{err}");
            assert!(!message.is_empty());
        }
        other => panic!("{other:?}"),
    }
    let shown = err.to_string();
    assert!(shown.starts_with("decisions:"), "{shown}");
    // An unknown field, with its line.
    let text = "schema = 1\nextra = true\n";
    let err = DecisionsFile::parse(text).unwrap_err();
    assert!(
        matches!(
            err,
            Error::Invalid {
                at: Some((2, _)),
                ..
            }
        ),
        "{err}"
    );
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
    assert!(!dir.join("decisions.toml.part").exists());
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
