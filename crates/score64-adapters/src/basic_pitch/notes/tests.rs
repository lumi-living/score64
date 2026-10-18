//! Differential test of the note-creation port against the Python package (#21).
//!
//! The fixtures in `testdata/basic-pitch/<stem>/` come from `sidecar/basic_pitch_reference.py --fixture`: the
//! package's model output for the stem rounded to float16, and the notes the package's own
//! `output_to_notes_polyphonic` makes from exactly those rounded values.

use super::*;

fn f16_to_f32(bits: u16) -> f32 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = i32::from((bits >> 10) & 0x1F);
    let frac = f32::from(bits & 0x3FF);
    match exp {
        0 => sign * frac * 2f32.powi(-24),
        31 => {
            if frac == 0.0 {
                sign * f32::INFINITY
            } else {
                f32::NAN
            }
        }
        _ => sign * (1.0 + frac / 1024.0) * 2f32.powi(exp - 15),
    }
}

fn read_f16(path: &str) -> Vec<f32> {
    std::fs::read(path)
        .unwrap_or_else(|e| panic!("{path}: {e}"))
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&b| f16_to_f32(u16::from_le_bytes(b)))
        .collect()
}

fn check(stem: &str) {
    let dir = format!("{}/testdata/basic-pitch/{stem}", env!("CARGO_MANIFEST_DIR"));
    let note = read_f16(&format!("{dir}/note.f16"));
    let onset = read_f16(&format!("{dir}/onset.f16"));
    let expected: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(format!("{dir}/notes.json")).unwrap())
            .unwrap();
    let frames = expected["frames"].as_u64().unwrap() as usize;
    assert_eq!(note.len(), frames * N_NOTES);

    let got = output_to_notes(&note, &onset, &NoteParams::default());
    let want = expected["notes"].as_array().unwrap();
    assert_eq!(got.len(), want.len(), "{stem}: note count");
    let times = frames_to_time(frames);
    for (k, (g, w)) in got.iter().zip(want).enumerate() {
        let key = |n: &str| w[n].as_u64().unwrap() as usize;
        assert_eq!(
            (g.start_frame, g.end_frame, usize::from(g.pitch)),
            (key("start_frame"), key("end_frame"), key("pitch")),
            "{stem}: note {k}"
        );
        let amp = w["amplitude"].as_f64().unwrap();
        assert!(
            (f64::from(g.amplitude) - amp).abs() < 1e-5,
            "{stem}: note {k} amplitude {} vs {amp}",
            g.amplitude
        );
        for (frame, field) in [(g.start_frame, "start_s"), (g.end_frame, "end_s")] {
            let t = w[field].as_f64().unwrap();
            assert!(
                (times[frame] - t).abs() < 1e-9,
                "{stem}: note {k} {field} {} vs {t}",
                times[frame]
            );
        }
    }
}

#[test]
fn matches_python_on_solo_voice() {
    check("vocadito_10");
}

#[test]
fn matches_python_on_polyphonic_stem() {
    check("synth_poly");
}

#[test]
fn f16_decoding() {
    assert_eq!(f16_to_f32(0x3C00), 1.0);
    assert_eq!(f16_to_f32(0x3800), 0.5);
    assert_eq!(f16_to_f32(0x0001), 2f32.powi(-24));
    assert_eq!(f16_to_f32(0xC000), -2.0);
}
