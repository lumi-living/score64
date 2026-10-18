//! Golden tests (#22): annotated dumps of the committed GoatTracker fixtures, as `insta` snapshots in
//! `tests/snapshots/`. When a writer or fixture changes on purpose, review the diff with `cargo insta review`;
//! the diff names the changed header field, orderlist, instrument, table row, pattern row or effect frame.
//!
//! The same dumps are the harness for the `.sng` writer (#47) and the sound-effect writer (#86): their tests write
//! bytes and snapshot `annotate::sng` / `annotate::sfx` of the result.

use std::path::PathBuf;

use score64_formats::annotate;

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/gt2-original")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn sixteen_steps_song() {
    insta::assert_snapshot!(
        "sixteen-steps.sng",
        annotate::sng(&fixture("sixteen-steps.sng")).unwrap()
    );
}

#[test]
fn effect_instruments() {
    for name in ["footstep", "gunshot", "clink"] {
        let file = format!("sfx-{name}.ins");
        insta::assert_snapshot!(file.clone(), annotate::ins(&fixture(&file)).unwrap());
    }
}

#[test]
fn effect_data() {
    for name in ["footstep", "gunshot", "clink"] {
        let file = format!("sfx-{name}.bin");
        insta::assert_snapshot!(file.clone(), annotate::sfx(&fixture(&file)).unwrap());
    }
}
