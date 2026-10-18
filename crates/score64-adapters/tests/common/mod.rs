//! Shared test helpers.

use std::io::Write as _;
use std::path::PathBuf;

use score64_adapters::sidecar::Sidecar;

/// The sidecar for tests that need it, or `None` with a visible skip message.
///
/// Sidecar tests are opt-in with `SCORE64_SIDECAR=1` (they need `uv` and download a Python environment on first
/// use). Opted in but unavailable is a failure, so CI's sidecar job can't pass by skipping.
pub fn sidecar(test: &str) -> Option<Sidecar> {
    let project = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../sidecar");
    if std::env::var_os("SCORE64_SIDECAR").is_none_or(|v| v.is_empty() || v == "0") {
        // Straight to the process's stderr: libtest captures eprintln! of passing tests, and a skip must show.
        let _ = writeln!(
            std::io::stderr(),
            "SKIPPED {test}: set SCORE64_SIDECAR=1 to run sidecar tests (needs uv, see sidecar/README.md)"
        );
        return None;
    }
    match Sidecar::locate(&project) {
        Ok(s) => Some(s),
        Err(why) => panic!("SCORE64_SIDECAR is set but {why}"),
    }
}

/// A fresh scratch directory for one test.
pub fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("score64-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("scratch dir");
    d
}
