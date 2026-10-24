//! The gate's pure parts: `[pack]` to gt2reloc's options, the trace plan from a score, when effects fire and where
//! the driver goes. The gate itself runs against the real gt2reloc and helper in `tests/pack_gate.rs`.

use std::num::{NonZeroU8, NonZeroU32};
use std::path::Path;

use score64_core::profile::ProfileName;
use score64_core::{FramesPerRow, Profiles};
use score64_core::{Schema, Subtune, SubtuneName, TempoChange};
use score64_project::ProjectFile;

use super::*;

fn pack(text: &str) -> Pack {
    ProjectFile::parse(&format!("[pack]\n{text}"), Path::new("score64.toml"))
        .unwrap()
        .pack
        .unwrap()
}

#[test]
fn the_window_start_is_the_player_page_and_the_rest_maps_across() {
    let p =
        pack("window = { start = 0x2000, end = 0x2FFF }\nzero_page = 0xFB\nsound_effects = true\n");
    let o = options(&p, VideoStandard::Pal, false).unwrap();
    assert_eq!(
        (
            o.player_page,
            o.zero_page.address(),
            o.sound_effects,
            o.timing_marks
        ),
        (0x20, 0xFB, true, false)
    );
    assert_eq!(o.hard_restart_adsr, DEFAULT_HARD_RESTART_ADSR);
    assert_eq!(o.timing, Timing::Pal);
    assert_eq!(
        o.args().unwrap(),
        ["-D1", "-H0", "-W20", "-ZFB", "-A0F00", "-P"]
    );
    let p = pack(
        "window = { start = 0xC000, end = 0xC7FF }\nzero_page = 0x02\nsound_effects = false\nhard_restart_adsr = 0x0A00\nmax_play_cycles = 3000\n",
    );
    let o = options(&p, VideoStandard::Ntsc, true).unwrap();
    assert_eq!(
        o.args().unwrap(),
        ["-D0", "-H1", "-WC0", "-Z02", "-A0A00", "-N"]
    );
    // The same [pack] is the gate's budget.
    assert_eq!(
        p.budget(),
        PackBudget {
            first: 0xC000,
            last: 0xC7FF,
            zero_page: 0x02,
            sound_effects: false,
            max_play_cycles: Some(3000),
        }
    );
    // A pack built in code with a zero page gt2reloc can't take is refused before packing.
    let bad = Pack {
        zero_page: 0xFF,
        ..p.clone()
    };
    assert_eq!(
        options(&bad, VideoStandard::Pal, false).unwrap_err().kind,
        ErrorKind::InvalidInput
    );
}

fn subtune(name: &str, profile: Option<&str>, length: u32, loop_row: u32) -> Subtune {
    Subtune {
        name: SubtuneName::new(name).unwrap(),
        profile: profile.map(|p| ProfileName::new(p).unwrap()),
        frames_per_row: FramesPerRow::new(6).unwrap(),
        tempo: vec![TempoChange {
            row: 32,
            frames_per_row: FramesPerRow::new(3).unwrap(),
        }],
        rows_per_bar: NonZeroU8::new(16).unwrap(),
        length: NonZeroU32::new(length).unwrap(),
        loop_row,
        marks: vec![],
        voices: vec![],
    }
}

#[test]
fn the_plan_plays_each_subtune_through_its_loop_and_fires_where_its_profile_allows() {
    let song = Song {
        schema: Schema::default(),
        profile: ProfileName::new("sfx-shares-v3").unwrap(),
        subtunes: vec![
            subtune("title", Some("full"), 64, 0),
            subtune("level", None, 64, 16),
            subtune("boss", Some("sfx-owns-v2-v3"), 16, 0),
        ],
    };
    let plan = TracePlan::for_song(&song, &Profiles::builtin(), vec![1, 2, 3]).unwrap();
    let v = |n| Voice::new(n).unwrap();
    // 64 rows: 32 at 6 frames and 32 at 3 = 288 frames a pass. The title loops to row 0 (288 more), the level to row
    // 16 (16 × 6 + 32 × 3 = 192 more), and the boss's 16 rows at 6 are 96 a pass; each after 8 start-up frames.
    assert_eq!(
        plan,
        TracePlan {
            subtunes: vec![
                SubtunePlan {
                    frames: 8 + 288 + 288,
                    effect_voices: vec![],
                },
                SubtunePlan {
                    frames: 8 + 288 + 192,
                    effect_voices: vec![v(3)],
                },
                SubtunePlan {
                    frames: 8 + 96 + 96,
                    effect_voices: vec![v(2), v(3)],
                },
            ],
            effect: vec![1, 2, 3],
        }
    );
    let unknown = Song {
        profile: ProfileName::new("busy").unwrap(),
        ..song
    };
    let e = TracePlan::for_song(&unknown, &Profiles::builtin(), vec![]).unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidInput);
    assert!(e.message.contains("profile `busy`"), "{}", e.message);
}

#[test]
fn effects_fire_on_every_voice_in_turn_while_the_subtune_plays() {
    let v = |n| Voice::new(n).unwrap();
    assert!(fires(1000, &[]).is_empty());
    // Every 100 play calls, centred, cycling the voices.
    let f = fires(1000, &[v(2), v(3)]);
    assert_eq!(f.len(), 10);
    assert_eq!(f[0], (50, v(2)));
    assert_eq!(f[1], (150, v(3)));
    assert_eq!(f[9], (950, v(3)));
    // Too short for one every 100: still one on each voice, inside the subtune.
    let f = fires(30, &[v(1), v(2), v(3)]);
    assert_eq!(f, [(5, v(1)), (15, v(2)), (25, v(3))]);
    for frames in [1, 2, 3, 99, 100, 101, 12_345] {
        let f = fires(frames, &[v(1), v(2), v(3)]);
        assert!(f.len() >= 3, "{frames}");
        assert!(f.windows(2).all(|w| w[0].0 <= w[1].0), "{frames}: in order");
        assert!(f.iter().all(|&(at, _)| at < frames), "{frames}: inside");
        for voice in 1..=3 {
            assert!(
                f.iter().any(|&(_, x)| x == v(voice)),
                "{frames}: voice {voice}"
            );
        }
    }
}

#[test]
fn the_driver_goes_outside_the_window_the_tune_and_the_io_area() {
    let tune = |load, len| Payload {
        load,
        data: vec![0; len],
    };
    // $C000 when it's free.
    assert_eq!(
        scratch(&tune(0x1000, 2000), Some((0x1000, 0x1FFF)), 64),
        Some(0xC000)
    );
    // Otherwise the lowest page that is.
    assert_eq!(
        scratch(&tune(0xC000, 2000), Some((0xC000, 0xCFFF)), 64),
        Some(0x0200)
    );
    assert_eq!(
        scratch(&tune(0x0200, 0x100), Some((0x0200, 0x02FF)), 64),
        Some(0xC000)
    );
    assert_eq!(
        scratch(&tune(0x0200, 0x100), Some((0x0200, 0xCFFF)), 64),
        Some(0xE000),
        "past the I/O area"
    );
    // A tune that runs out of its window is avoided too.
    assert_eq!(
        scratch(&tune(0xBF00, 0x200), Some((0xBF00, 0xBFFF)), 64),
        Some(0x0200)
    );
    // Room is needed for the whole driver and effect; none at all is None.
    assert_eq!(
        scratch(&tune(0x0200, 0x100), Some((0x0200, 0xFFFF)), 16),
        None
    );
    assert_eq!(
        scratch(&tune(0x0200, 0x100), Some((0x0200, 0xFEFF)), 16),
        Some(0xFF00)
    );
    assert_eq!(
        scratch(&tune(0x0200, 0x100), Some((0x0200, 0xFEFF)), 0xFB),
        None,
        "clear of the vectors"
    );
}

/// The driver against the real gt2reloc and helper: pointed at play, it plays the tune as calling play directly
/// would, and firing through it really starts the effect on the voice asked for. Skips visibly without them.
#[test]
fn the_driver_plays_the_tune_and_fires_the_effect() {
    use score64_adapters::emu::Helper;
    use std::io::Write as _;
    let (Ok(packer), Some(binary)) = (Gt2Reloc::locate("2.73"), Helper::locate()) else {
        let _ = writeln!(
            std::io::stderr(),
            "SKIPPED the_driver_plays_the_tune_and_fires_the_effect: needs gt2reloc on PATH and the score64-emu helper"
        );
        return;
    };
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/gt2-original");
    let dir = std::env::temp_dir().join(format!("score64-pack-driver-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p =
        pack("window = { start = 0x1000, end = 0x1FFF }\nzero_page = 0xF8\nsound_effects = true\n");
    let packed = packer
        .pack(
            &fixtures.join("sixteen-steps.sng"),
            &dir.join("tune.prg"),
            &options(&p, VideoStandard::Pal, false).unwrap(),
        )
        .unwrap();
    let payload = packed.payload().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let effect = std::fs::read(fixtures.join("sfx-gunshot.bin")).unwrap();
    let driver = scratch(
        &payload,
        Some((0x1000, 0x1FFF)),
        DRIVER_BYTES + effect.len(),
    )
    .unwrap();
    let fx = driver + DRIVER_BYTES as u16;
    let mut h = Helper::spawn(&binary, 48_000).unwrap();
    // The SID writes of 50 frames of subtune 2 (0-based 1), played through `play` from a fresh load, with `before`
    // run first.
    let mut writes = |play: u16, before: &mut dyn FnMut(&mut DynHelper)| {
        h.load(payload.load, &payload.data).unwrap();
        h.load(driver, &driver_code(payload.load, fx)).unwrap();
        h.load(fx, &effect).unwrap();
        h.init(payload.load, play, 1).unwrap();
        before(&mut h);
        h.render(50, true).unwrap().writes
    };
    let direct = writes(payload.load + 3, &mut |_| {});
    let driven = writes(driver, &mut |_| {});
    // The same writes, each the driver's JMP later.
    let shifted: Vec<_> = direct
        .iter()
        .map(|w| score64_adapters::emu::SidWrite {
            cycle: w.cycle + DRIVER_CYCLES as u16,
            ..*w
        })
        .collect();
    assert!(!direct.is_empty());
    assert_eq!(
        driven, shifted,
        "the driver plays as play does, 3 cycles later"
    );
    let v3 = Voice::new(3).unwrap();
    // A fire takes one frame, in which the effect call runs in place of play: after it the music is one frame
    // behind, so it compares with the music unfired, a frame earlier.
    let fired = writes(driver, &mut |h| fire(h, driver, payload.load, v3).unwrap());
    let voice = |ws: &[score64_adapters::emu::SidWrite], v: u8, shift: u32| {
        ws.iter()
            .filter(|w| (7 * (v - 1)..7 * v).contains(&w.register) && w.frame >= shift)
            .filter(|w| w.frame - shift < 49)
            .map(|w| (w.frame - shift, w.register, w.value))
            .collect::<Vec<_>>()
    };
    assert!(!voice(&driven, 3, 0).is_empty());
    assert_ne!(
        voice(&fired, 3, 1),
        voice(&driven, 3, 0),
        "the effect changed voice 3"
    );
    assert_eq!(
        voice(&fired, 1, 1),
        voice(&driven, 1, 0),
        "and only voice 3"
    );
    assert_eq!(
        voice(&fired, 2, 1),
        voice(&driven, 2, 0),
        "and only voice 3"
    );
}
