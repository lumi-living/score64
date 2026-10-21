"""Reference output of the Basic Pitch Python package, for the differential tests of Score64's Rust port (#21, #34).

    uv run --frozen --project sidecar python sidecar/basic_pitch_reference.py <audio> <out_dir> [--fixture <dir>]

Runs the package's own ONNX model (nmp.onnx) and writes to <out_dir>:
    nmp.onnx            the model it used, so a differential test runs exactly the same file
    input_22050.f32     the audio as the package loads it (librosa, 22,050 Hz mono), raw little-endian float32
    note.f32 onset.f32 contour.f32
                        model output after windowing and unwrapping, (frames, 88) / (frames, 88) / (frames, 264)
    notes.json          output_to_notes_polyphonic with predict()'s defaults: start/end frame, pitch, amplitude, times
    predict.json        predict()'s own note events, as a cross-check
    manifest.json       schema 1 (manifest.py): versions, parameters, shapes, and blake3 of every input and output

With --fixture, also writes note.f16, onset.f16 and notes.json for a committed post-processing test. The arrays
are rounded to float16 and the notes recomputed from the rounded arrays, so the fixture is self-consistent.
"""

import argparse
import json
import pathlib
import shutil

import numpy as np

from basic_pitch import ICASSP_2022_MODEL_PATH, note_creation
from basic_pitch.constants import AUDIO_SAMPLE_RATE, FFT_HOP
from basic_pitch.inference import Model, predict, run_inference

import manifest

TOOL_VERSION = 1

ONSET_THRESH = 0.5
FRAME_THRESH = 0.3
MIN_NOTE_LEN = int(np.round(127.70 / 1000 * (AUDIO_SAMPLE_RATE / FFT_HOP)))  # predict()'s default, in frames


def notes_from(note, onset):
    events = note_creation.output_to_notes_polyphonic(
        note.copy(),
        onset.copy(),
        onset_thresh=ONSET_THRESH,
        frame_thresh=FRAME_THRESH,
        min_note_len=MIN_NOTE_LEN,
        infer_onsets=True,
        max_freq=None,
        min_freq=None,
        melodia_trick=True,
    )
    times = note_creation.model_frames_to_time(note.shape[0])
    return [
        {
            "start_frame": int(s),
            "end_frame": int(e),
            "pitch": int(p),
            "amplitude": float(a),
            "start_s": float(times[s]),
            "end_s": float(times[e]),
        }
        for s, e, p, a in events
    ]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("audio", type=pathlib.Path)
    ap.add_argument("out_dir", type=pathlib.Path)
    ap.add_argument("--fixture", type=pathlib.Path)
    args = ap.parse_args()
    out = args.out_dir
    out.mkdir(parents=True, exist_ok=True)

    import librosa

    model_path = pathlib.Path(ICASSP_2022_MODEL_PATH).with_name("nmp.onnx")
    audio, _ = librosa.load(str(args.audio), sr=AUDIO_SAMPLE_RATE, mono=True)
    audio.astype("<f4").tofile(out / "input_22050.f32")

    model = Model(model_path)
    output = run_inference(args.audio, model)
    for k in ("note", "onset", "contour"):
        output[k].astype("<f4").tofile(out / f"{k}.f32")
    notes = notes_from(output["note"], output["onset"])
    (out / "notes.json").write_text(json.dumps(notes, indent=1))

    _, _, events = predict(args.audio, model, onset_threshold=ONSET_THRESH, frame_threshold=FRAME_THRESH)
    (out / "predict.json").write_text(
        json.dumps([{"start_s": float(s), "end_s": float(e), "pitch": int(p), "amplitude": float(a)}
                    for s, e, p, a, _ in events], indent=1)
    )

    shutil.copyfile(model_path, out / "nmp.onnx")
    manifest.write(
        out,
        "basic_pitch_reference",
        TOOL_VERSION,
        ["basic-pitch", "onnxruntime", "numpy", "librosa", "scipy"],
        {
            "onset_thresh": ONSET_THRESH,
            "frame_thresh": FRAME_THRESH,
            "min_note_len": MIN_NOTE_LEN,
            "infer_onsets": True,
            "melodia_trick": True,
            "frames": int(output["note"].shape[0]),
            "shapes": {k: list(v.shape) for k, v in output.items()},
            "samples_22050": int(audio.shape[0]),
        },
        [args.audio],
        ["nmp.onnx", "input_22050.f32", "note.f32", "onset.f32", "contour.f32", "notes.json", "predict.json"],
    )

    if args.fixture:
        fx = args.fixture
        fx.mkdir(parents=True, exist_ok=True)
        note16 = output["note"].astype("<f2")
        onset16 = output["onset"].astype("<f2")
        note16.tofile(fx / "note.f16")
        onset16.tofile(fx / "onset.f16")
        fx_notes = notes_from(note16.astype(np.float32), onset16.astype(np.float32))
        (fx / "notes.json").write_text(json.dumps({"frames": int(note16.shape[0]), "notes": fx_notes}, indent=1))
    print(f"{args.audio.name}: {output['note'].shape[0]} frames, {len(notes)} notes")


if __name__ == "__main__":
    main()
