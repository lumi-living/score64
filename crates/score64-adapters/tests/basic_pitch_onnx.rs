//! End-to-end differential test of Basic Pitch through `ort` against the Python package (#21), driven by the sidecar
//! (#23). Opt-in, because it needs `uv`:
//!
//! ```sh
//! SCORE64_SIDECAR=1 cargo test --release -p score64-adapters --test basic_pitch_onnx -- --nocapture
//! ```
//!
//! For each committed stem the sidecar runs basic-pitch 0.4.0 and writes its activations, notes and the exact
//! `nmp.onnx` it used. The test runs that model through `ort` on the same 22,050 Hz samples and requires the same
//! activations (within 1e-3) and the same notes.

mod common;

use std::path::{Path, PathBuf};

use score64_adapters::basic_pitch::{BasicPitch, N_CONTOURS, N_NOTES, NoteParams, output_to_notes};
use score64_adapters::sidecar::Manifest;

fn read_f32(path: &Path) -> Vec<f32> {
    std::fs::read(path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .as_chunks::<4>()
        .0
        .iter()
        .map(|&b| f32::from_le_bytes(b))
        .collect()
}

fn max_diff(name: &str, got: &[f32], want: &[f32]) -> f32 {
    assert_eq!(got.len(), want.len(), "{name}: length");
    let max = got
        .iter()
        .zip(want)
        .map(|(a, b)| (a - b).abs())
        .fold(0f32, f32::max);
    println!("  {name:8} max |Δ| {max:.3e}");
    max
}

fn check(manifest: &Manifest) {
    let out = |name: &str| {
        manifest
            .output(name)
            .unwrap_or_else(|| panic!("manifest lists no {name}"))
    };
    let audio = read_f32(&out("input_22050.f32"));
    let mut bp = BasicPitch::load(&out("nmp.onnx")).expect("load model");
    let act = bp.infer(&audio).expect("inference");
    assert_eq!(act.contour.len(), act.frames * N_CONTOURS);
    assert_eq!(act.note.len(), act.frames * N_NOTES);
    let worst = [
        max_diff("note", &act.note, &read_f32(&out("note.f32"))),
        max_diff("onset", &act.onset, &read_f32(&out("onset.f32"))),
        max_diff("contour", &act.contour, &read_f32(&out("contour.f32"))),
    ]
    .into_iter()
    .fold(0f32, f32::max);
    assert!(worst < 1e-3, "activations differ by up to {worst}");

    let got: Vec<(usize, usize, usize)> =
        output_to_notes(&act.note, &act.onset, &NoteParams::default())
            .iter()
            .map(|n| (n.start_frame, n.end_frame, usize::from(n.pitch)))
            .collect();
    let want: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out("notes.json")).unwrap()).unwrap();
    let want: Vec<(usize, usize, usize)> = want
        .as_array()
        .unwrap()
        .iter()
        .map(|n| {
            let k = |f: &str| n[f].as_u64().unwrap() as usize;
            (k("start_frame"), k("end_frame"), k("pitch"))
        })
        .collect();
    println!("  notes: Rust {} / Python {}", got.len(), want.len());
    assert_eq!(got, want);
}

#[test]
fn onnx_matches_python_package() {
    let Some(sidecar) = common::sidecar("onnx_matches_python_package") else {
        return;
    };
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
    for stem in ["sparse-melody-bass", "dense-mix"] {
        let out_dir = common::scratch(&format!("basic-pitch-{stem}"));
        let audio = fixtures.join(stem).join("source.wav");
        let manifest = sidecar
            .run(
                "basic_pitch_reference.py",
                &[audio.as_os_str(), out_dir.as_os_str()],
                &out_dir,
                "basic_pitch_reference",
            )
            .unwrap_or_else(|e| panic!("{stem}: {e}"));
        println!(
            "{stem}: basic-pitch {} on Python {}",
            manifest
                .packages
                .iter()
                .find(|(k, _)| k == "basic-pitch")
                .map_or("?", |(_, v)| v.as_str()),
            manifest.python
        );
        check(&manifest);
        let _ = std::fs::remove_dir_all(&out_dir);
    }
}
