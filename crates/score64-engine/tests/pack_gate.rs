//! The budget gate end to end (#198, #51). The fixture song is packed by the real gt2reloc 2.73 with the options a
//! project's `[pack]` gives. Both its subtunes are traced in the emulator helper, with the gunshot effect fired on
//! voice 3 of the second one (its profile, `sfx-shares-v3`, makes voice 3 expendable). Rule 8 then checks the facts
//! against the same `[pack]`: a generous window passes, and a window too small, a raster budget too low and a wrong
//! zero page each fail with the matching problem. Runs when gt2reloc and the helper are available; otherwise it skips
//! visibly (CI has no gt2reloc).

use std::collections::BTreeSet;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use score64_adapters::emu::Helper;
use score64_adapters::gt2reloc::Gt2Reloc;
use score64_core::validate::Severity;
use score64_core::{Rule, VideoStandard, Voice};
use score64_engine::pack::{
    GateReport, PackReport, PackRequest, SubtunePlan, TracePlan, gate, options, pack_and_gate,
};
use score64_engine::{Engine, EngineConfig};
use score64_formats::psid::Psid;
use score64_project::ProjectFile;
use score64_project::project::Pack;

/// Two passes of the fixture's 8-bar piece at 125 BPM, 768 frames each (`fixtures/_tools/verify_song.sh` renders 31
/// seconds), so each subtune plays through its loop once after the player's 8 start-up frames, as
/// `TracePlan::for_song` would ask.
const FRAMES: u32 = 1554;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/gt2-original")
}

/// gt2reloc 2.73 and the helper, or `None` with a visible skip message.
fn tools(test: &str) -> Option<Gt2Reloc> {
    match (Gt2Reloc::locate("2.73"), Helper::locate()) {
        (Ok(packer), Some(_)) => Some(packer),
        _ => {
            // Straight to the process's stderr: libtest captures eprintln! of passing tests, and a skip must show.
            let _ = writeln!(
                std::io::stderr(),
                "SKIPPED {test}: needs gt2reloc on PATH and the score64-emu helper"
            );
            None
        }
    }
}

/// A fresh scratch folder for one test.
fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("score64-pack-gate-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// The `[pack]` of a project with `keys`.
fn pack(keys: &str) -> Pack {
    ProjectFile::parse(&format!("[pack]\n{keys}"), Path::new("score64.toml"))
        .unwrap()
        .pack
        .unwrap()
}

/// A target's slot at $1000–$1FFF, its pair at $F8, effects on, and a raster budget of `max_play_cycles`.
fn generous(max_play_cycles: u32) -> Pack {
    pack(&format!(
        "window = {{ start = 0x1000, end = 0x1FFF }}\nzero_page = 0xF8\nsound_effects = true\n\
         max_play_cycles = {max_play_cycles}\n"
    ))
}

/// Subtune 1 ("main", profile `full`) takes no effects; subtune 2 ("combat", `sfx-shares-v3`) takes them on voice 3.
fn plan(effects: bool) -> TracePlan {
    let v3 = Voice::new(3).unwrap();
    TracePlan {
        subtunes: vec![
            SubtunePlan {
                frames: FRAMES,
                effect_voices: vec![],
            },
            SubtunePlan {
                frames: FRAMES,
                effect_voices: if effects { vec![v3] } else { vec![] },
            },
        ],
        effect: std::fs::read(fixtures().join("sfx-gunshot.bin")).unwrap(),
    }
}

fn run(packer: &Gt2Reloc, dir: &Path, pack: &Pack, plan: &TracePlan) -> PackReport {
    let mut engine = Engine::new(EngineConfig::default());
    pack_and_gate(
        &mut engine,
        packer,
        &PackRequest {
            song: &fixtures().join("sixteen-steps.sng"),
            out: &dir.join("tune.sid"),
            pack,
            video: VideoStandard::Pal,
            timing_marks: false,
            plan,
        },
    )
    .unwrap()
}

fn messages(report: &GateReport) -> Vec<&str> {
    report.problems.iter().map(|p| p.message.as_str()).collect()
}

#[test]
fn a_pack_inside_a_generous_window_passes() {
    let Some(packer) = tools("a_pack_inside_a_generous_window_passes") else {
        return;
    };
    let dir = scratch("generous");
    let report = run(&packer, &dir, &generous(4000), &plan(true));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(report.gate.passes(), "{:?}", messages(&report.gate));
    assert!(
        report.gate.problems.is_empty(),
        "{:?}",
        messages(&report.gate)
    );
    let facts = &report.gate.facts;
    assert_eq!((report.start, facts.start), (0x1000, 0x1000));
    assert_eq!(facts.size, report.size);
    assert!(
        facts.jump_table.iter().all(Option::is_some),
        "{:04X?}",
        facts.jump_table
    );
    assert_eq!(facts.zero_page_used, BTreeSet::from([0xF8, 0xF9]));
    assert_eq!(facts.pages_written_outside, BTreeSet::new());
    assert_eq!(facts.traced_window, Some((0x1000, 0x1FFF)));
    // The fixture's play routine takes at most 16.3 raster lines (README), about 1,030 cycles.
    let cycles = facts.max_play_cycles.unwrap();
    assert!((100..=1100).contains(&cycles), "{cycles} cycles");
    assert_eq!(report.packer_version, "2.73");
}

#[test]
fn the_raster_measurement_is_the_play_routines_own() {
    let Some(packer) = tools("the_raster_measurement_is_the_play_routines_own") else {
        return;
    };
    // Without effects, the gate's longest play call is the one the helper measures calling play directly, each
    // subtune from a freshly loaded player as the gate runs it: the driver's JMP is taken off exactly.
    let dir = scratch("raster");
    let report = run(&packer, &dir, &generous(4000), &plan(false));
    let psid = Psid::parse(&std::fs::read(&report.path).unwrap()).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let mut h = Helper::spawn(&Helper::locate().unwrap(), 48_000).unwrap();
    // The longest play call over `FRAMES` frames, rendered in chunks the protocol allows.
    let play = |h: &mut Helper<_, _>| {
        let (mut left, mut max) = (FRAMES, 0);
        while left > 0 {
            let n = left.min(h.max_render_frames(false));
            max = max.max(h.render(n, false).unwrap().max_play_cycles);
            left -= n;
        }
        max
    };
    let mut direct = vec![];
    for subtune in 0..2 {
        h.load(psid.load_address, &psid.data).unwrap();
        h.init(psid.init_address, psid.play_address, subtune)
            .unwrap();
        direct.push(play(&mut h));
    }
    // A fresh load matters: the second subtune's first play calls take longer than after the first subtune has
    // run, so the gate measures each subtune as a game starting with it would.
    h.load(psid.load_address, &psid.data).unwrap();
    h.init(psid.init_address, psid.play_address, 0).unwrap();
    play(&mut h);
    h.init(psid.init_address, psid.play_address, 1).unwrap();
    let switched = play(&mut h);
    assert!(
        switched < direct[1],
        "{switched} after subtune 1, {} fresh",
        direct[1]
    );
    assert_eq!(
        report.gate.facts.max_play_cycles,
        direct.iter().max().copied()
    );
}

#[test]
fn a_window_too_small_fails() {
    let Some(packer) = tools("a_window_too_small_fails") else {
        return;
    };
    let dir = scratch("small");
    let small =
        pack("window = { start = 0x1000, end = 0x13FF }\nzero_page = 0xF8\nsound_effects = true\n");
    let report = run(&packer, &dir, &small, &plan(true));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(!report.gate.passes());
    assert!(report.size > 0x400, "the fixture is larger than 1 KB");
    let m = messages(&report.gate);
    assert_eq!(
        m[0],
        format!(
            "the tune takes $1000–${:04X} ({} bytes), outside the window $1000–$13FF",
            0x1000 + report.size - 1,
            report.size
        )
    );
    assert_eq!(
        report.gate.problems[0].hint,
        "shorten patterns, share instruments or give the tune a larger window"
    );
    assert!(
        report
            .gate
            .problems
            .iter()
            .all(|p| p.rule == Rule::Pack && p.severity == Severity::Error)
    );
    assert_eq!(report.gate.facts.traced_window, Some((0x1000, 0x13FF)));
}

#[test]
fn a_raster_budget_too_low_fails() {
    let Some(packer) = tools("a_raster_budget_too_low_fails") else {
        return;
    };
    let dir = scratch("raster-low");
    let report = run(&packer, &dir, &generous(200), &plan(true));
    let _ = std::fs::remove_dir_all(&dir);
    let cycles = report.gate.facts.max_play_cycles.unwrap();
    assert_eq!(
        messages(&report.gate),
        [format!(
            "the longest play call takes {cycles} cycles, over the budget of 200"
        )]
    );
}

#[test]
fn a_wrong_zero_page_fails() {
    let Some(packer) = tools("a_wrong_zero_page_fails") else {
        return;
    };
    // Packed for a pair at $FB, checked against a target whose pair is at $F8.
    let dir = scratch("zero-page");
    let configured = generous(4000);
    let other = Pack {
        zero_page: 0xFB,
        ..configured.clone()
    };
    let packed = packer
        .pack(
            &fixtures().join("sixteen-steps.sng"),
            &dir.join("tune.sid"),
            &options(&other, VideoStandard::Pal, false).unwrap(),
        )
        .unwrap();
    let mut engine = Engine::new(EngineConfig::default());
    let report = gate(&mut engine, &packed, &configured.budget(), &plan(true)).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(report.facts.zero_page_used, BTreeSet::from([0xFB, 0xFC]));
    assert_eq!(
        messages(&report),
        ["the player uses zero page $FB, $FC outside the configured $F8–$F9"]
    );
}

#[test]
fn a_pack_without_the_effect_entry_fails_without_firing_into_it() {
    let Some(packer) = tools("a_pack_without_the_effect_entry_fails_without_firing_into_it") else {
        return;
    };
    // Packed without sound-effect support, for a target that calls the effect entry: the gate traces the music,
    // fires nothing at start+6, and rule 8 names the missing entry.
    let dir = scratch("no-effects");
    let configured = generous(4000);
    let without = Pack {
        sound_effects: false,
        ..configured.clone()
    };
    let packed = packer
        .pack(
            &fixtures().join("sixteen-steps.sng"),
            &dir.join("tune.sid"),
            &options(&without, VideoStandard::Pal, false).unwrap(),
        )
        .unwrap();
    let mut engine = Engine::new(EngineConfig::default());
    let report = gate(&mut engine, &packed, &configured.budget(), &plan(true)).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        messages(&report),
        ["no JMP at $1006, the sound-effect entry"]
    );
    assert_eq!(report.facts.traced_window, Some((0x1000, 0x1FFF)));
    assert_eq!(report.facts.zero_page_used, BTreeSet::from([0xF8, 0xF9]));
}
