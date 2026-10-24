//! The budget gate end to end (#198, #51). The fixture song is packed by the real gt2reloc 2.73 with the options a
//! project's `[pack]` gives. Both its subtunes are traced in the emulator helper, with the gunshot effect fired on
//! voice 3 of the second one (its profile, `sfx-shares-v3`, makes voice 3 expendable). Rule 8 then checks the facts
//! against the same `[pack]`: a generous window passes, and a window too small, a raster budget too low and a wrong
//! zero page each fail with the matching problem. An exported score is gated with the plan `TracePlan::for_song`
//! gives it. These run when gt2reloc and the helper are available; otherwise they skip visibly (CI has no gt2reloc).
//!
//! Hand-made tunes cover what gt2reloc never makes. A broken jump table or an inverted window isn't traced (no
//! helper needed, so these run in CI). A play call that jams or never returns, an effect call that jams, and an init
//! or play call that writes over the trace's driver are each reported with what was measured (helper needed).

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write as _;
use std::num::{NonZeroU8, NonZeroU32};
use std::path::{Path, PathBuf};

use score64_adapters::emu::Helper;
use score64_adapters::gt2reloc::{Format, Gt2Reloc, Packed};
use score64_core::profile::ProfileName;
use score64_core::validate::Severity;
use score64_core::{
    AdaptiveSet, Attack, Confidence, FramesPerRow, Note, PackBudget, PresetId, Profiles, Role,
    Roles, Rule, Schema, Song, Subtune, SubtuneName, Switch, TempoChange, VideoStandard, Voice,
    VoiceLine,
};
use score64_engine::pack::{
    GateReport, PackReport, PackRequest, SubtunePlan, TracePlan, gate, options, pack_and_gate,
};
use score64_engine::{Engine, EngineConfig, ErrorKind};
use score64_formats::gt2::export::{Header, Library, export};
use score64_formats::gt2::sng::{Orderlist, Sng};
use score64_formats::gt2::{Ins, Instrument, Table};
use score64_formats::psid::Psid;
use score64_project::ProjectFile;
use score64_project::project::Pack;

/// What `TracePlan::for_song` asks for each of the fixture's subtunes (`the_fixtures_frames_are_its_plans`): its 8
/// bars of 16 rows at 6 frames a row (`F06`, 125 BPM) are 768 frames, played once and, looping to the start, once
/// more, after the player's 8 start-up frames, and then the loop row's 6 frames as a margin.
const FRAMES: u32 = 8 + 768 + 768 + 6;

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
    // The player's variables are in its own bytes, past $13FF: the trace measured writes outside the window.
    assert_eq!(
        m[1..],
        ["the player writes outside its window $1000–$13FF, in page $14xx"]
    );
    assert_eq!(
        report.gate.facts.pages_written_outside,
        BTreeSet::from([0x14])
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

/// The fixture packed as a `.prg` in `dir` with `pack`'s options: no PSID header, so a plan may trace fewer subtunes
/// than the song has.
fn pack_prg(packer: &Gt2Reloc, dir: &Path, pack: &Pack) -> Packed {
    packer
        .pack(
            &fixtures().join("sixteen-steps.sng"),
            &dir.join("tune.prg"),
            &options(pack, VideoStandard::Pal, false).unwrap(),
        )
        .unwrap()
}

/// A plan of `frames` for each subtune, firing on `voices`.
fn plan_of(subtunes: &[(u32, &[Voice])]) -> TracePlan {
    TracePlan {
        subtunes: subtunes
            .iter()
            .map(|&(frames, voices)| SubtunePlan {
                frames,
                effect_voices: voices.to_vec(),
            })
            .collect(),
        effect: std::fs::read(fixtures().join("sfx-gunshot.bin")).unwrap(),
    }
}

fn gate_of(packed: &Packed, budget: &PackBudget, plan: &TracePlan) -> GateReport {
    gate(
        &mut Engine::new(EngineConfig::default()),
        packed,
        budget,
        plan,
    )
    .unwrap()
}

#[test]
fn the_fixtures_frames_are_its_plans() {
    // The fixture's own song, read with the formats crate: each subtune's rows and loop row from its order lists,
    // and its tempo from the `F` command on its first row. Its subtunes as a score (`main` plays `full`, `combat`
    // `sfx-shares-v3`, README) get the plan these tests use. Needs no tools.
    let sng = Sng::read(&std::fs::read(fixtures().join("sixteen-steps.sng")).unwrap()).unwrap();
    assert_eq!(sng.subtunes.len(), 2);
    let shape = |voices: &[Orderlist; 3]| {
        let rows = |entries: &[u8]| -> u32 {
            entries
                .iter()
                .map(|&e| {
                    // Pattern numbers only: the fixture has no repeats or transposes.
                    assert!(usize::from(e) < sng.patterns.len(), "entry ${e:02X}");
                    sng.patterns[usize::from(e)].rows.len() as u32
                })
                .sum()
        };
        let (length, loop_row) = (
            rows(&voices[0].entries),
            rows(&voices[0].entries[..usize::from(voices[0].restart)]),
        );
        for v in voices {
            assert_eq!(
                (rows(&v.entries), rows(&v.entries[..usize::from(v.restart)])),
                (length, loop_row),
                "every voice in step"
            );
        }
        let tempo = voices
            .iter()
            .map(|v| sng.patterns[usize::from(v.entries[0])].rows[0])
            .find(|r| r.command == 0xF)
            .expect("a tempo on the first row")
            .data;
        (length, loop_row, tempo)
    };
    let subtune = |n: usize, name: &str, profile: &str| {
        let (length, loop_row, tempo) = shape(&sng.subtunes[n]);
        Subtune {
            name: SubtuneName::new(name).unwrap(),
            profile: Some(ProfileName::new(profile).unwrap()),
            frames_per_row: FramesPerRow::new(tempo).unwrap(),
            tempo: vec![],
            rows_per_bar: NonZeroU8::new(16).unwrap(),
            length: NonZeroU32::new(length).unwrap(),
            loop_row,
            marks: vec![],
            voices: vec![],
        }
    };
    let song = Song {
        schema: Schema::default(),
        profile: ProfileName::new("full").unwrap(),
        subtunes: vec![
            subtune(0, "main", "full"),
            subtune(1, "combat", "sfx-shares-v3"),
        ],
    };
    let effect = std::fs::read(fixtures().join("sfx-gunshot.bin")).unwrap();
    assert_eq!(
        TracePlan::for_song(&song, &Profiles::builtin(), effect).unwrap(),
        plan(true)
    );
}

#[test]
fn every_subtune_counts_not_only_the_last() {
    let Some(packer) = tools("every_subtune_counts_not_only_the_last") else {
        return;
    };
    // The window holds only the player's first page, so its variables' pages are written outside it. Subtune 1
    // plays through; subtune 2 only its first play call, which touches no zero page, writes fewer pages and is
    // shorter than the longest of subtune 1's.
    let dir = scratch("every-subtune");
    let packed = pack_prg(&packer, &dir, &generous(4000));
    let budget = PackBudget {
        last: 0x10FF,
        ..generous(4000).budget()
    };
    let first = gate_of(&packed, &budget, &plan_of(&[(FRAMES, &[])]));
    let both = gate_of(&packed, &budget, &plan_of(&[(FRAMES, &[]), (1, &[])]));
    let _ = std::fs::remove_dir_all(&dir);
    let (first, both) = (&first.facts, &both.facts);
    assert_eq!(first.zero_page_used, BTreeSet::from([0xF8, 0xF9]));
    assert_eq!(
        first.pages_written_outside,
        BTreeSet::from([0x11, 0x12, 0x13, 0x14])
    );
    assert!(first.max_play_cycles.unwrap() > 900, "{first:?}");
    // The second subtune's single call adds nothing, and takes nothing away.
    assert_eq!(both.zero_page_used, first.zero_page_used);
    assert_eq!(both.pages_written_outside, first.pages_written_outside);
    assert_eq!(both.max_play_cycles, first.max_play_cycles);
}

#[test]
fn effects_are_really_fired() {
    let Some(packer) = tools("effects_are_really_fired") else {
        return;
    };
    // Effects starting on all three voices, three times in 20 play calls, make subtune 1's longest play call longer
    // than its music alone: the trace saw the effect calls' work. Subtune 2 plays one call, the shortest.
    let dir = scratch("fired");
    let pack = generous(4000);
    let packed = pack_prg(&packer, &dir, &pack);
    let all: Vec<Voice> = (1..=3).filter_map(Voice::new).collect();
    let music = gate_of(&packed, &pack.budget(), &plan_of(&[(20, &[]), (1, &[])]));
    let fired = gate_of(&packed, &pack.budget(), &plan_of(&[(20, &all), (1, &[])]));
    // Packed without the effect entry, the same plan fires nothing.
    let without = Pack {
        sound_effects: false,
        ..pack.clone()
    };
    let packed = pack_prg(&packer, &dir, &without);
    let unfired = gate_of(&packed, &pack.budget(), &plan_of(&[(20, &all), (1, &[])]));
    let music_only = gate_of(&packed, &pack.budget(), &plan_of(&[(20, &[]), (1, &[])]));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(music.passes() && fired.passes(), "{:?}", messages(&fired));
    let (m, f) = (
        music.facts.max_play_cycles.unwrap(),
        fired.facts.max_play_cycles.unwrap(),
    );
    assert!(f > m, "{f} cycles with effects, {m} without");
    assert_eq!(unfired.facts, music_only.facts);
}

/// A packed file written by hand: `data` at `load`, in `format` (`Prg`, or `Sid` with `songs` subtunes).
fn hand_packed(dir: &Path, format: Format, load: u16, data: &[u8], songs: u16) -> Packed {
    let mut bytes = vec![];
    let path = match format {
        Format::Sid => {
            let mut h = vec![0u8; 0x7C];
            h[0..4].copy_from_slice(b"PSID");
            h[4..6].copy_from_slice(&2u16.to_be_bytes());
            h[6..8].copy_from_slice(&0x7Cu16.to_be_bytes());
            h[0x0A..0x0C].copy_from_slice(&load.to_be_bytes());
            h[0x0C..0x0E].copy_from_slice(&(load + 3).to_be_bytes());
            h[0x0E..0x10].copy_from_slice(&songs.to_be_bytes());
            h[0x10..0x12].copy_from_slice(&1u16.to_be_bytes());
            bytes.extend(h);
            dir.join("hand.sid")
        }
        _ => dir.join("hand.prg"),
    };
    bytes.extend(load.to_le_bytes());
    bytes.extend(data);
    std::fs::write(&path, bytes).unwrap();
    Packed {
        path,
        format,
        start: load,
        size: data.len() as u32,
        version: "by hand".into(),
        program_blake3: String::new(),
    }
}

/// A window at $1000–$10FF for a target without effects, and no raster budget.
fn hand_budget() -> PackBudget {
    PackBudget {
        first: 0x1000,
        last: 0x10FF,
        zero_page: 0xF8,
        sound_effects: false,
        max_play_cycles: None,
    }
}

#[test]
fn a_broken_jump_table_is_reported_not_traced() {
    // No helper needed: nothing is traced, so nothing jams or hangs.
    let dir = scratch("broken-table");
    let one = plan_of(&[(100, &[])]);
    // Init jumps out of the tune.
    let out = hand_packed(
        &dir,
        Format::Prg,
        0x1000,
        &[0x4C, 0x00, 0x30, 0x4C, 0x06, 0x10, 0x60],
        1,
    );
    let report = gate_of(&out, &hand_budget(), &one);
    assert_eq!(
        messages(&report),
        [
            "the init entry at $1000 jumps to $3000, outside the tune",
            "the player wasn't traced, so its zero page and its writes outside the window weren't measured"
        ]
    );
    assert_eq!(report.facts.traced_window, None);
    assert_eq!(report.facts.max_play_cycles, None);
    // Play jumps to itself, inside the jump table: it would never return.
    let itself = hand_packed(
        &dir,
        Format::Prg,
        0x1000,
        &[0x4C, 0x06, 0x10, 0x4C, 0x03, 0x10, 0x60],
        1,
    );
    let report = gate_of(&itself, &hand_budget(), &one);
    assert_eq!(
        messages(&report),
        [
            "the play entry at $1003 jumps to $1003, inside the jump table",
            "the player wasn't traced, so its zero page and its writes outside the window weren't measured"
        ]
    );
    assert_eq!(report.facts.traced_window, None);
    // With effects configured, the table is 9 bytes: play jumping to start+6 is inside it too.
    let short = hand_packed(
        &dir,
        Format::Prg,
        0x1000,
        &[0x4C, 0x09, 0x10, 0x4C, 0x06, 0x10, 0x4C, 0x09, 0x10, 0x60],
        1,
    );
    let effects = PackBudget {
        sound_effects: true,
        ..hand_budget()
    };
    let report = gate_of(&short, &effects, &one);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        messages(&report),
        [
            "the play entry at $1003 jumps to $1006, inside the jump table",
            "the player wasn't traced, so its zero page and its writes outside the window weren't measured"
        ]
    );
}

#[test]
fn a_plan_must_match_the_psids_subtunes() {
    // No helper needed: the plan is refused before anything runs.
    let dir = scratch("psid-count");
    let tune = [0x4C, 0x06, 0x10, 0x4C, 0x06, 0x10, 0x60];
    let two = hand_packed(&dir, Format::Sid, 0x1000, &tune, 2);
    let e = gate(
        &mut Engine::new(EngineConfig::default()),
        &two,
        &hand_budget(),
        &plan_of(&[(100, &[])]),
    )
    .unwrap_err();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(e.kind, ErrorKind::InvalidInput);
    assert!(
        e.message
            .ends_with("hand.sid holds 2 subtunes; the trace plan has 1"),
        "{}",
        e.message
    );
}

/// The helper, or `None` with a visible skip message.
fn helper(test: &str) -> Option<()> {
    Helper::locate().map(|_| ()).or_else(|| {
        let _ = writeln!(
            std::io::stderr(),
            "SKIPPED {test}: needs the score64-emu helper"
        );
        None
    })
}

#[test]
fn a_call_that_fails_mid_trace_keeps_what_was_measured() {
    if helper("a_call_that_fails_mid_trace_keeps_what_was_measured").is_none() {
        return;
    }
    let dir = scratch("jam");
    // Play writes a JAM over the driver's first byte, at $C000 outside the window: the next play call jams. The
    // write that caused it is still reported.
    let jams = hand_packed(
        &dir,
        Format::Prg,
        0x1000,
        &[
            0x4C, 0x06, 0x10, // init
            0x4C, 0x07, 0x10, // play
            0x60, // init: RTS
            0xA9, 0x02, // play: LDA #$02 (JAM)
            0x8D, 0x00, 0xC0, // STA $C000
            0x60, // RTS
        ],
        1,
    );
    let report = gate_of(&jams, &hand_budget(), &plan_of(&[(100, &[])]));
    assert_eq!(report.facts.pages_written_outside, BTreeSet::from([0xC0]));
    assert_eq!(report.facts.traced_window, Some((0x1000, 0x10FF)));
    let m = messages(&report);
    assert_eq!(
        m[0],
        "the player writes outside its window $1000–$10FF, in page $C0xx"
    );
    assert!(
        m[1].starts_with("subtune 1's trace stopped: one of play calls 1–50 failed: "),
        "{m:?}"
    );
    assert_eq!(m.len(), 2, "{m:?}");
    assert!(!report.passes());
    // An init that jams is reported the same way, with nothing measured.
    let init_jams = hand_packed(
        &dir,
        Format::Prg,
        0x1000,
        &[0x4C, 0x06, 0x10, 0x4C, 0x07, 0x10, 0x02, 0x60],
        1,
    );
    let report = gate_of(&init_jams, &hand_budget(), &plan_of(&[(100, &[])]));
    let _ = std::fs::remove_dir_all(&dir);
    let m = messages(&report);
    assert_eq!(m.len(), 1, "{m:?}");
    assert!(
        m[0].starts_with("subtune 1's trace stopped: its init call failed: "),
        "{m:?}"
    );
    assert_eq!(report.facts.pages_written_outside, BTreeSet::new());
    assert_eq!(report.facts.max_play_cycles, None);
}

#[test]
fn a_window_over_all_ram_gets_the_driver_after_the_tune() {
    if helper("a_window_over_all_ram_gets_the_driver_after_the_tune").is_none() {
        return;
    }
    // The window leaves no room outside, so the driver goes right after the tune, inside the window: only a budget
    // built in code can do this, since a `[pack]` window can't cover the I/O area. A player that writes over the
    // driver there (here the effect call's JSR, which a frame without an effect doesn't run) is reported, though
    // the write is inside its window.
    let dir = scratch("all-ram");
    let mut tune = vec![
        0x4C, 0x06, 0x02, // init
        0x4C, 0x07, 0x02, // play
        0x60, // init: RTS
        0xA9, 0x00, // play: LDA #0
        0x8D, 0x00, 0x00, // STA driver+11, patched below
        0x60, // RTS
    ];
    let driver = 0x0200 + tune.len() as u16;
    tune[10..12].copy_from_slice(&(driver + 11).to_le_bytes());
    let budget = PackBudget {
        first: 0x0200,
        last: 0xFFFF,
        ..hand_budget()
    };
    let overwrites = hand_packed(&dir, Format::Prg, 0x0200, &tune, 1);
    let report = gate_of(&overwrites, &budget, &plan_of(&[(10, &[])]));
    let m = messages(&report);
    assert_eq!(
        m,
        [format!(
            "subtune 1 wrote over the trace's driver at ${:04X}, inside the window past the tune's end, where it \
             went for want of room outside the window; what was measured after that can't be trusted",
            driver + 11
        )]
    );
    assert_eq!(report.facts.traced_window, Some((0x0200, 0xFFFF)));
    // One that leaves it alone passes.
    tune[7..12].copy_from_slice(&[0xEA; 5]);
    let quiet = hand_packed(&dir, Format::Prg, 0x0200, &tune, 1);
    let report = gate_of(&quiet, &budget, &plan_of(&[(10, &[])]));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(report.problems.is_empty(), "{:?}", messages(&report));
    assert!(report.facts.max_play_cycles.is_some());
}

#[test]
fn the_fixture_gates_with_the_driver_inside_a_window_over_all_ram() {
    let Some(packer) = tools("the_fixture_gates_with_the_driver_inside_a_window_over_all_ram")
    else {
        return;
    };
    // GoatTracker's player doesn't write past its bytes, so with the driver and the effect inside the window, after
    // the tune, the facts are those of a generous window: the same zero page, the same longest play call, and with
    // effects fired.
    let dir = scratch("fixture-all-ram");
    let pack = generous(4000);
    let packed = pack_prg(&packer, &dir, &pack);
    let all_ram = PackBudget {
        first: 0x0200,
        last: 0xFFFF,
        ..pack.budget()
    };
    let inside = gate_of(&packed, &all_ram, &plan(true));
    let outside = gate_of(&packed, &pack.budget(), &plan(true));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(inside.problems.is_empty(), "{:?}", messages(&inside));
    assert_eq!(inside.facts.zero_page_used, outside.facts.zero_page_used);
    assert_eq!(inside.facts.max_play_cycles, outside.facts.max_play_cycles);
    assert_eq!(inside.facts.pages_written_outside, BTreeSet::new());
}

/// A pulse instrument: hard restart 2 frames ahead, first frame $09 (test bit + gate).
fn pulse() -> Ins {
    let mut instrument = Instrument::from_bytes(&[0; 25]);
    instrument.ad = 0x09;
    instrument.sr = 0xA0;
    instrument.wave = 1;
    instrument.pulse = 1;
    instrument.gate_timer = 2;
    instrument.first_wave = 0x09;
    instrument.set_name("test pulse");
    Ins {
        instrument,
        wave: Table {
            rows: vec![(0x41, 0x00), (0xFF, 0x00)],
        },
        pulse: Table {
            rows: vec![(0x88, 0x00), (0xFF, 0x00)],
        },
        filter: Table::default(),
        speed: Table::default(),
    }
}

/// A layer of an adaptive set: four bars of 16 rows at 5 frames a row and 3 from row 40, looping to bar 2, with a
/// note on voice 1 every bar.
fn layer(name: &str, profile: &str, pitch: &str) -> Subtune {
    let note = |row| Note {
        row,
        length: NonZeroU32::new(8).unwrap(),
        pitch: pitch.parse().unwrap(),
        attack: Attack::Fresh,
        instrument: None,
        source: None,
        confidence: Confidence::new(1.0).unwrap(),
        decision: None,
    };
    Subtune {
        name: SubtuneName::new(name).unwrap(),
        profile: Some(ProfileName::new(profile).unwrap()),
        frames_per_row: FramesPerRow::new(5).unwrap(),
        tempo: vec![TempoChange {
            row: 40,
            frames_per_row: FramesPerRow::new(3).unwrap(),
        }],
        rows_per_bar: NonZeroU8::new(16).unwrap(),
        length: NonZeroU32::new(64).unwrap(),
        loop_row: 16,
        marks: vec![],
        voices: vec![VoiceLine {
            voice: Voice::new(1).unwrap(),
            roles: Roles::constant(Role::Lead),
            instrument: PresetId("pulse".into()),
            notes: [0, 16, 32, 48].into_iter().map(note).collect(),
            arps: vec![],
        }],
    }
}

#[test]
fn an_exported_score_gates_with_the_plan_for_its_song() {
    let Some(packer) = tools("an_exported_score_gates_with_the_plan_for_its_song") else {
        return;
    };
    // Two layers of an adaptive set, timing marks at every bar: `calm` plays `full`, `tense` `sfx-shares-v3`.
    let mut song = Song {
        schema: Schema::default(),
        profile: ProfileName::new("full").unwrap(),
        subtunes: vec![
            layer("calm", "full", "C-4"),
            layer("tense", "sfx-shares-v3", "G-4"),
        ],
    };
    let set = AdaptiveSet::with_switch(
        "layers",
        song.subtunes.iter().map(|s| s.name.clone()).collect(),
        Switch::EveryBar,
    )
    .unwrap();
    set.place_marks(&mut song);
    let library: Library = BTreeMap::from([(PresetId("pulse".into()), pulse())]);
    let exported = export(
        &song,
        &library,
        std::slice::from_ref(&set),
        &Header::default(),
    )
    .unwrap();
    let dir = scratch("exported");
    let sng = dir.join("score.sng");
    std::fs::write(&sng, exported.sng.write().unwrap()).unwrap();
    let effect = std::fs::read(fixtures().join("sfx-gunshot.bin")).unwrap();
    let plan = TracePlan::for_song(&song, &Profiles::builtin(), effect).unwrap();
    let pack = generous(4000);
    let mut engine = Engine::new(EngineConfig::default());
    let report = pack_and_gate(
        &mut engine,
        &packer,
        &PackRequest {
            song: &sng,
            out: &dir.join("score.sid"),
            pack: &pack,
            video: VideoStandard::Pal,
            timing_marks: exported.timing_marks,
            plan: &plan,
        },
    )
    .unwrap();
    let psid = Psid::parse(&std::fs::read(&report.path).unwrap()).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        report.gate.problems.is_empty(),
        "{:?}",
        messages(&report.gate)
    );
    assert_eq!(
        report.gate.facts.zero_page_used,
        BTreeSet::from([0xF8, 0xF9])
    );
    assert_eq!(report.gate.facts.pages_written_outside, BTreeSet::new());
    assert_eq!(
        plan.subtunes
            .iter()
            .map(|s| s.effect_voices.clone())
            .collect::<Vec<_>>(),
        [vec![], vec![Voice::new(3).unwrap()]]
    );
    // The plan covers each layer's loop. Its timing-mark byte (start+$3F) changes at each bar: the last change is
    // the loop row's mark after the second jump back, and the plan plays that whole row once more: its last play
    // call is the first after it.
    let mut h = Helper::spawn(&Helper::locate().unwrap(), 48_000).unwrap();
    let mark = psid.load_address + 0x3F;
    for (n, (subtune, sub)) in song.subtunes.iter().zip(&plan.subtunes).enumerate() {
        let (first, loop_pass) = (subtune.frames(0, 64), subtune.frames(16, 64));
        let margin = subtune.frames(16, 17);
        assert_eq!(u64::from(sub.frames), 8 + first + loop_pass + margin);
        h.load_psid(&psid).unwrap();
        h.trace(false, None, &[mark]).unwrap();
        h.init(psid.init_address, psid.play_address, n as u8)
            .unwrap();
        h.render(sub.frames, false).unwrap();
        let changes = h.report().unwrap().watched[0].changes.clone();
        let last = u64::from(changes.last().unwrap().0);
        assert_eq!(
            last,
            7 + first + loop_pass,
            "{}: {changes:?}",
            subtune.name.as_str()
        );
        assert_eq!(
            u64::from(sub.frames) - 1,
            last + margin,
            "{}",
            subtune.name.as_str()
        );
    }
}

#[test]
fn an_effect_frame_counts_the_effect_call_and_the_play_call() {
    let Some(packer) = tools("an_effect_frame_counts_the_effect_call_and_the_play_call") else {
        return;
    };
    // One frame, the first play call, with the gunshot fired on voice 3 just before it, as the game's interrupt
    // does: its time is the effect call's and the play call's. A budget between play alone and the two together
    // passes the music and fails the effect frame.
    let dir = scratch("effect-frame");
    let pack = generous(4000);
    let packed = pack_prg(&packer, &dir, &pack);
    let v3 = [Voice::new(3).unwrap()];
    let play = gate_of(&packed, &pack.budget(), &plan_of(&[(1, &[])]));
    let both = gate_of(&packed, &pack.budget(), &plan_of(&[(1, &v3)]));
    let (p, b) = (
        play.facts.max_play_cycles.unwrap(),
        both.facts.max_play_cycles.unwrap(),
    );
    // The gunshot's effect call takes 45 cycles here (`pack::tests`, measured apart).
    assert!(
        (30..80).contains(&(b - p)),
        "{p} cycles alone, {b} with the effect"
    );
    let budget = PackBudget {
        max_play_cycles: Some(p + (b - p) / 2),
        ..pack.budget()
    };
    let play = gate_of(&packed, &budget, &plan_of(&[(1, &[])]));
    let both = gate_of(&packed, &budget, &plan_of(&[(1, &v3)]));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(play.passes(), "{:?}", messages(&play));
    assert_eq!(
        messages(&both),
        [format!(
            "the longest play call takes {b} cycles, over the budget of {}",
            p + (b - p) / 2
        )]
    );
}

#[test]
fn a_play_call_that_never_returns_is_a_problem_not_an_error() {
    if helper("a_play_call_that_never_returns_is_a_problem_not_an_error").is_none() {
        return;
    }
    // Play is `JMP *`: the helper times the call out after a frame's budget, and the gate reports it.
    let dir = scratch("timeout");
    let hangs = hand_packed(
        &dir,
        Format::Prg,
        0x1000,
        &[
            0x4C, 0x09, 0x10, // init
            0x4C, 0x06, 0x10, // play
            0x4C, 0x06, 0x10, // play: JMP *
            0x60, // init: RTS
        ],
        1,
    );
    let report = gate_of(&hangs, &hand_budget(), &plan_of(&[(100, &[])]));
    let _ = std::fs::remove_dir_all(&dir);
    let m = messages(&report);
    assert_eq!(m.len(), 1, "{m:?}");
    assert!(
        m[0].starts_with("subtune 1's trace stopped: one of play calls 1–50 failed: ")
            && m[0].contains("did not return"),
        "{m:?}"
    );
    assert_eq!(report.facts.traced_window, Some((0x1000, 0x10FF)));
}

#[test]
fn an_effect_call_that_fails_is_a_problem() {
    if helper("an_effect_call_that_fails_is_a_problem").is_none() {
        return;
    }
    // The effect entry jams. Effects fire on voice 3 once in 100 play calls, before play call 51.
    let dir = scratch("effect-jams");
    let jams = hand_packed(
        &dir,
        Format::Prg,
        0x1000,
        &[
            0x4C, 0x09, 0x10, // init
            0x4C, 0x09, 0x10, // play
            0x4C, 0x0A, 0x10, // sound effect
            0x60, // init and play: RTS
            0x02, // sound effect: JAM
        ],
        1,
    );
    let effects = PackBudget {
        sound_effects: true,
        ..hand_budget()
    };
    let report = gate_of(
        &jams,
        &effects,
        &plan_of(&[(100, &[Voice::new(3).unwrap()])]),
    );
    let _ = std::fs::remove_dir_all(&dir);
    let m = messages(&report);
    assert_eq!(m.len(), 1, "{m:?}");
    assert!(
        m[0].starts_with(
            "subtune 1's trace stopped: the sound-effect call on voice 3 with play call 51 failed: "
        ),
        "{m:?}"
    );
    // The 50 play calls before it were measured.
    assert!(report.facts.max_play_cycles.is_some());
}

#[test]
fn an_init_that_writes_over_the_driver_is_reported() {
    if helper("an_init_that_writes_over_the_driver_is_reported").is_none() {
        return;
    }
    // As in the window over all RAM, but init writes over the driver (the JMP to play after an effect), and play
    // leaves it alone.
    let dir = scratch("init-overwrites");
    let mut tune = vec![
        0x4C, 0x06, 0x02, // init
        0x4C, 0x0C, 0x02, // play
        0xA9, 0x00, // init: LDA #0
        0x8D, 0x00, 0x00, // STA driver+13, patched below
        0x60, // RTS
        0x60, // play: RTS
    ];
    let driver = 0x0200 + tune.len() as u16;
    tune[9..11].copy_from_slice(&(driver + 13).to_le_bytes());
    let budget = PackBudget {
        first: 0x0200,
        last: 0xFFFF,
        ..hand_budget()
    };
    let overwrites = hand_packed(&dir, Format::Prg, 0x0200, &tune, 1);
    let report = gate_of(&overwrites, &budget, &plan_of(&[(10, &[])]));
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        messages(&report),
        [format!(
            "subtune 1 wrote over the trace's driver at ${:04X}, inside the window past the tune's end, where it \
             went for want of room outside the window; what was measured after that can't be trusted",
            driver + 13
        )]
    );
}

#[test]
fn an_inverted_window_is_reported_not_traced() {
    // No helper needed: a budget whose window ends before it starts has nothing to trace writes outside of, so
    // the gate traces nothing and rule 8 reports the window.
    let dir = scratch("inverted");
    let tune = hand_packed(
        &dir,
        Format::Prg,
        0x1000,
        &[0x4C, 0x06, 0x10, 0x4C, 0x06, 0x10, 0x60],
        1,
    );
    let inverted = PackBudget {
        first: 0x10FF,
        last: 0x1000,
        ..hand_budget()
    };
    let report = gate_of(&tune, &inverted, &plan_of(&[(100, &[])]));
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        messages(&report),
        [
            "the memory window $10FF–$1000 ends before it starts",
            "the player wasn't traced, so its zero page and its writes outside the window weren't measured"
        ]
    );
    assert_eq!(report.facts.traced_window, None);
    assert_eq!(report.facts.zero_page_used, BTreeSet::new());
    assert_eq!(report.facts.max_play_cycles, None);
}
