//! End-to-end differential test of Basic Pitch through `ort` against the Python package (#21).
//!
//! Needs the model and a reference directory from `sidecar/basic_pitch_reference.py`, so it is ignored by default:
//!
//! ```sh
//! uv run sidecar/basic_pitch_reference.py stem.wav ref/
//! BASIC_PITCH_MODEL=path/to/nmp.onnx BASIC_PITCH_REFERENCE=ref/ \
//!   cargo test -p score64-adapters --test basic_pitch_onnx -- --ignored --nocapture
//! ```

use std::path::PathBuf;

use score64_adapters::basic_pitch::{BasicPitch, N_CONTOURS, N_NOTES, NoteParams, output_to_notes};

fn read_f32(path: &PathBuf) -> Vec<f32> {
    std::fs::read(path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .as_chunks::<4>()
        .0
        .iter()
        .map(|&b| f32::from_le_bytes(b))
        .collect()
}

fn diff(name: &str, got: &[f32], want: &[f32]) -> f32 {
    assert_eq!(got.len(), want.len(), "{name}: length");
    let max = got
        .iter()
        .zip(want)
        .map(|(a, b)| (a - b).abs())
        .fold(0f32, f32::max);
    let mean = got
        .iter()
        .zip(want)
        .map(|(a, b)| f64::from((a - b).abs()))
        .sum::<f64>()
        / got.len() as f64;
    println!("{name:8} max |Δ| {max:.3e}  mean |Δ| {mean:.3e}");
    max
}

#[test]
#[ignore = "needs BASIC_PITCH_MODEL and BASIC_PITCH_REFERENCE"]
fn onnx_matches_python_package() {
    let model = PathBuf::from(std::env::var("BASIC_PITCH_MODEL").expect("BASIC_PITCH_MODEL"));
    let reference =
        PathBuf::from(std::env::var("BASIC_PITCH_REFERENCE").expect("BASIC_PITCH_REFERENCE"));
    let audio = read_f32(&reference.join("input_22050.f32"));

    let mut bp = BasicPitch::load(&model).expect("load model");
    let started = std::time::Instant::now();
    let out = bp.infer(&audio).expect("inference");
    println!(
        "{} s of audio, {} frames, inference {:.0} ms",
        audio.len() as f64 / 22_050.0,
        out.frames,
        started.elapsed().as_secs_f64() * 1000.0
    );
    assert_eq!(out.note.len(), out.frames * N_NOTES);
    assert_eq!(out.contour.len(), out.frames * N_CONTOURS);

    let worst = [
        diff("note", &out.note, &read_f32(&reference.join("note.f32"))),
        diff("onset", &out.onset, &read_f32(&reference.join("onset.f32"))),
        diff(
            "contour",
            &out.contour,
            &read_f32(&reference.join("contour.f32")),
        ),
    ]
    .into_iter()
    .fold(0f32, f32::max);
    assert!(worst < 1e-3, "activations differ by up to {worst}");

    let got = output_to_notes(&out.note, &out.onset, &NoteParams::default());
    let want: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(reference.join("notes.json")).unwrap())
            .unwrap();
    let want: Vec<(usize, usize, usize)> = want
        .as_array()
        .unwrap()
        .iter()
        .map(|n| {
            let k = |f: &str| n[f].as_u64().unwrap() as usize;
            (k("start_frame"), k("end_frame"), k("pitch"))
        })
        .collect();
    let got: Vec<(usize, usize, usize)> = got
        .iter()
        .map(|n| (n.start_frame, n.end_frame, usize::from(n.pitch)))
        .collect();
    let same = got.iter().zip(&want).filter(|(a, b)| a == b).count();
    println!(
        "notes: Rust {} / Python {}, identical in order: {same}",
        got.len(),
        want.len()
    );
    assert_eq!(got, want);
}
