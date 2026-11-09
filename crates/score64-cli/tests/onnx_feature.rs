//! ONNX Runtime is opt-in (#241): a default build of the CLI has no `ort` in its dependency tree, so it needs nothing
//! beyond crates.io and carries no ONNX Runtime, and the `basic-pitch-onnx` feature brings it in. This asks cargo
//! itself (`cargo tree`, as the licence-boundary job does), so it checks the default build however this test was
//! built.

use std::process::Command;

/// The packages in `score64-cli`'s normal and build dependency tree with `features` (none for the default build).
fn tree(features: &[&str]) -> Vec<String> {
    let mut cargo = Command::new(env!("CARGO"));
    cargo
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args(["tree", "--locked", "-p", "score64-cli"])
        .args(["-e", "normal,build", "--prefix", "none"]);
    if !features.is_empty() {
        cargo.args(["--features", &features.join(",")]);
    }
    let out = cargo.output().expect("run cargo tree");
    assert!(
        out.status.success(),
        "cargo tree failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .map(str::to_owned)
        .collect()
}

#[test]
fn a_default_build_has_no_onnx_runtime_and_the_feature_brings_it() {
    let default = tree(&[]);
    for crate_ in ["ort", "ort-sys"] {
        assert!(
            !default.iter().any(|p| p == crate_),
            "`{crate_}` is in the default build"
        );
    }
    let with = tree(&["basic-pitch-onnx"]);
    assert!(
        with.iter().any(|p| p == "ort"),
        "`ort` is missing with the feature"
    );
}
