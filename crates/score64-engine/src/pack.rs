//! The pack stage's budget gate (rule 8; #51, #198): pack the song, read the packed bytes back, trace the player in
//! the emulator helper, and check the measured facts against the project's `[pack]` budget.
//!
//! **Not a front-end surface.** Front-ends never call this module: every user-visible action is a command (ADR-005),
//! and the app, CLI and MCP server go through [`crate::REGISTRY`] or the command types. There is no pack command
//! yet; these functions are what it will run, inside this crate:
//! - [`options`] maps a resolved `[pack]` to gt2reloc's options; [`Pack::budget`] gives rule 8's budget;
//! - [`TracePlan::for_song`] works out what to trace from the score: every subtune through its loop once and one row
//!   more, and an effect call on each voice its profile gives to effects;
//! - [`gate`] reads a packed file, traces it and runs `check_pack`; [`pack_and_gate`] packs first.
//!
//! The module is public, and hidden from the docs, only so that the end-to-end tests (`tests/pack_gate.rs`) can run
//! the gate against the real gt2reloc and helper.
//!
//! **How a subtune is traced.** The packed bytes are loaded fresh, with a trace armed for the budget's window, and
//! init is called with the subtune number. Play is then called once a frame for the plan's frames. The helper calls
//! a three-byte driver, `JMP start+3`, rather than play itself, so that the driver can be pointed
//! at an effect call between two play calls. On such a frame the driver calls the sound-effect entry (start+6) with
//! the effect's address in A/Y and the voice's channel offset in X, as a game's main loop would, *in place of* play.
//! The trace sees everything either call reads or writes. Each play call's time is the helper's measurement less the
//! driver's `JMP abs`, 3 cycles. The effect calls themselves aren't play calls, so their time doesn't count. The
//! driver and the effect data are only read, never written, and the host's own writes (`Load`, `Poke`) aren't traced,
//! so none of them shows up as a write outside the window.
//!
//! The driver and the effect data go outside the window when there is room, and otherwise inside it, past the
//! tune's end: GoatTracker's packed player keeps its variables in its own bytes, so it doesn't write there. Then the
//! trace watches the driver, and a player that writes over it anyway gets a problem saying so.
//!
//! A tune whose init or play entry doesn't jump into it, past the jump table, isn't traced, and the effect entry is
//! only called when it does: rule 8 then reports the jump table, where the helper would only report a CPU jam or a
//! timeout. A call that fails while the trace runs (a CPU jam, or a call that doesn't return in time) stops that
//! subtune's trace, but what was measured up to it is kept: the facts hold it, and the report adds a problem naming
//! the call. So when a relocation bug makes the player write over the driver and crash, that write is still reported.
//!
//! **What isn't traced.** Each subtune is traced from a fresh load. A game with an adaptive set inits one layer
//! while another is playing; that path isn't traced, so zero page and writes outside the window are measured only
//! for subtunes started from a freshly loaded player (TAD §1.3, rule 8).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use score64_adapters::emu::{DynHelper, HelperError};
use score64_adapters::gt2reloc::{
    Format, Gt2Reloc, PackError, PackOptions, Packed, Timing, ZeroPage,
};
use score64_core::validate::{Problem, Severity, has_errors};
use score64_core::{
    PackBudget, PackFacts, Profiles, Rule, Song, VideoStandard, Voice, VoiceUse, check_pack,
};
use score64_formats::emu_protocol::ErrorCode;
use score64_formats::packed::Payload;
use score64_formats::psid::Psid;
use score64_project::project::Pack;

use crate::{CommandError, Engine, ErrorKind};

/// gt2reloc's own hard-restart ADSR, used when `[pack]` sets none.
pub const DEFAULT_HARD_RESTART_ADSR: u16 = 0x0F00;

/// The play calls a plan adds to a subtune's rows before they start. Row 0 starts on the eighth play call after init
/// (frame 7, `docs/reference/player.md` §Timing marks). With these, the plan's play calls run to the first one after
/// the loop's last row, and [`TracePlan::for_song`] adds the loop row's frames once more as a margin.
pub const START_FRAMES: u64 = 8;

/// How many play calls are rendered at a time: one PAL second. A call that fails is reported within this many.
const CHUNK_FRAMES: u32 = 50;

/// How often an effect is fired while a subtune plays, in play calls: every two PAL seconds, cycling through the
/// voices that take effects. A subtune too short for one on each voice gets them closer together.
pub const EFFECT_EVERY: u32 = 100;

/// The driver's `JMP abs` to play, which the helper counts as part of each play call.
const DRIVER_CYCLES: u32 = 3;

/// The gt2reloc options for a resolved `[pack]`, for a target on `video`: the player at the window's start page
/// (`-W`), its zero-page pair (`-Z`), sound effects (`-D`), the hard-restart ADSR (`-A`, gt2reloc's `$0F00` when
/// unset), PAL or NTSC timing (`-P`/`-N`), and timing marks (`-H`) when the song has adaptive sets.
pub fn options(
    pack: &Pack,
    video: VideoStandard,
    timing_marks: bool,
) -> Result<PackOptions, CommandError> {
    let zero_page = ZeroPage::new(pack.zero_page)
        .map_err(|e| CommandError::new(ErrorKind::InvalidInput, e.to_string()))?;
    Ok(PackOptions {
        player_page: pack.window.player_page(),
        zero_page,
        sound_effects: pack.sound_effects,
        timing_marks,
        hard_restart_adsr: pack.hard_restart_adsr.unwrap_or(DEFAULT_HARD_RESTART_ADSR),
        timing: match video {
            VideoStandard::Pal => Timing::Pal,
            VideoStandard::Ntsc => Timing::Ntsc,
        },
    })
}

/// What to trace: each subtune in song order, and the effect to fire.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TracePlan {
    /// One entry per subtune, in song order: subtune *n* is init'ed with A = *n*.
    pub subtunes: Vec<SubtunePlan>,
    /// GoatTracker sound-effect data (`docs/reference/goattracker-sfx.md`) to fire on the voices that take effects.
    /// Only needed when one does and the pack has sound-effect support.
    pub effect: Vec<u8>,
}

/// How to trace one subtune.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubtunePlan {
    /// How many play calls: at least through its loop once.
    pub frames: u32,
    /// The voices to fire the effect on, which its profile gives to effects (expendable or effects-only).
    pub effect_voices: Vec<Voice>,
}

impl TracePlan {
    /// The plan for `song`, with its subtunes' profiles from `profiles` and `effect` to fire. Each subtune plays its
    /// rows once from the start and its loop once more, after the player's start ([`START_FRAMES`]), following its
    /// tempo changes, and then its loop row once more, so the plan doesn't end exactly on the jump back. Effects go
    /// on every voice its profile doesn't keep for music alone.
    pub fn for_song(
        song: &Song,
        profiles: &Profiles,
        effect: Vec<u8>,
    ) -> Result<Self, CommandError> {
        let subtunes = song
            .subtunes
            .iter()
            .map(|s| {
                let name = song.profile_of(s);
                let profile = profiles.get(name).ok_or_else(|| {
                    CommandError::new(
                        ErrorKind::InvalidInput,
                        format!(
                            "subtune `{}` plays profile `{name}`, which isn't built in or the project's",
                            s.name.as_str()
                        ),
                    )
                })?;
                let end = s.length.get();
                let margin = s.frames(s.loop_row, s.loop_row + 1);
                let frames = START_FRAMES + s.frames(0, end) + s.frames(s.loop_row, end) + margin;
                let frames = u32::try_from(frames).map_err(|_| {
                    CommandError::new(
                        ErrorKind::InvalidInput,
                        format!("subtune `{}` is too long to trace: {frames} frames", s.name.as_str()),
                    )
                })?;
                let effect_voices = (1..=3)
                    .filter_map(Voice::new)
                    .filter(|&v| profile.voice(v).usage() != VoiceUse::Music)
                    .collect();
                Ok(SubtunePlan {
                    frames,
                    effect_voices,
                })
            })
            .collect::<Result<_, CommandError>>()?;
        Ok(TracePlan { subtunes, effect })
    }
}

/// What the gate found: the measured facts and every rule 8 problem with them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GateReport {
    /// What was measured from the packed bytes and the trace.
    pub facts: PackFacts,
    /// Rule 8's problems, all errors; none when the pack fits its budget.
    pub problems: Vec<Problem>,
}

impl GateReport {
    /// Whether the pack fits its budget: no errors.
    pub fn passes(&self) -> bool {
        !has_errors(&self.problems)
    }
}

/// A packed tune and what the gate found.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PackReport {
    /// The file written.
    pub path: PathBuf,
    /// Where the player loads.
    pub start: u16,
    /// Its packed size, as gt2reloc reported it and the file confirmed.
    pub size: u32,
    /// The version gt2reloc's banner names.
    pub packer_version: String,
    /// The blake3 of the gt2reloc program that packed it.
    pub packer_blake3: String,
    /// The gate's facts and problems.
    #[serde(flatten)]
    pub gate: GateReport,
}

/// What to pack, and how to gate it.
#[derive(Clone, Copy, Debug)]
pub struct PackRequest<'a> {
    /// The song (`.sng`).
    pub song: &'a Path,
    /// The output, whose extension (`sid`, `prg` or `bin`) names the format.
    pub out: &'a Path,
    /// The resolved `[pack]`.
    pub pack: &'a Pack,
    /// The resolved video standard.
    pub video: VideoStandard,
    /// Whether the song carries timing marks for adaptive sets (gt2reloc `-H1`).
    pub timing_marks: bool,
    /// What to trace.
    pub plan: &'a TracePlan,
}

/// Pack `request.song` with `packer` and run the gate on the result with `request.pack`'s budget. Packing failures
/// are errors; a pack that doesn't fit its budget is a report whose problems say why ([`GateReport::passes`]).
pub fn pack_and_gate(
    engine: &mut Engine,
    packer: &Gt2Reloc,
    request: &PackRequest<'_>,
) -> Result<PackReport, CommandError> {
    let options = options(request.pack, request.video, request.timing_marks)?;
    let packed = packer
        .pack(request.song, request.out, &options)
        .map_err(pack_error)?;
    let gate = gate(engine, &packed, &request.pack.budget(), request.plan)?;
    Ok(PackReport {
        path: packed.path,
        start: packed.start,
        size: packed.size,
        packer_version: packed.version,
        packer_blake3: packed.program_blake3,
        gate,
    })
}

/// The gate on a packed file: read its bytes back (checked against what gt2reloc reported), trace every subtune of
/// `plan` in the engine's helper, and run rule 8 against `budget`. A call that fails during the trace is a problem in
/// the report, after rule 8's, with what was measured up to it in the facts; a helper that breaks is an error.
pub fn gate(
    engine: &mut Engine,
    packed: &Packed,
    budget: &PackBudget,
    plan: &TracePlan,
) -> Result<GateReport, CommandError> {
    let payload = packed.payload().map_err(pack_error)?;
    check_plan(packed, budget, plan)?;
    // Only a tune whose init and play entries jump into it, past the jump table, can run, and the effect entry is
    // called only where it does too; otherwise the trace is skipped (or fires nothing) and rule 8 reports the jump
    // table instead of the helper refusing a CPU jam or a call that never returns. An inverted window is rule 8's to
    // report too; there is nothing to trace writes outside of.
    let load = u32::from(payload.load);
    let tune = load..load + payload.data.len() as u32;
    let table_end = load + 3 * if budget.sound_effects { 3 } else { 2 };
    let jumps = payload.jump_table();
    let into_tune = |i: usize| {
        jumps[i].is_some_and(|to| tune.contains(&u32::from(to)) && u32::from(to) >= table_end)
    };
    let window = (budget.first <= budget.last).then_some((budget.first, budget.last));
    let (mut zero_page_used, mut pages_written_outside) = (BTreeSet::new(), BTreeSet::new());
    let mut max_play_cycles: Option<u32> = None;
    let mut stopped = Vec::new();
    let traced = into_tune(0) && into_tune(1);
    if traced {
        let effects = budget.sound_effects
            && into_tune(2)
            && plan.subtunes.iter().any(|s| !s.effect_voices.is_empty());
        let effect = if effects { plan.effect.clone() } else { vec![] };
        let len = DRIVER_BYTES + effect.len();
        let scratch = scratch(&payload, window, len).ok_or_else(|| {
            CommandError::new(
                ErrorKind::InvalidInput,
                format!(
                    "no room for the trace's driver and effect data ({len} bytes): it goes outside the window and \
                     the tune, clear of zero page, the stack, the I/O area and the CPU's vectors, or else inside the \
                     window past the tune's end, and the tune leaves neither; leave {len} bytes free outside the \
                     window, or inside it after the tune"
                ),
            )
        })?;
        for (n, sub) in plan.subtunes.iter().enumerate() {
            let fires = if effects {
                fires(sub.frames, &sub.effect_voices)
            } else {
                vec![]
            };
            let job = Job {
                payload: payload.clone(),
                window,
                scratch,
                effect: effect.clone(),
                subtune: n as u8,
                frames: sub.frames,
                fires,
            };
            // Real time, a frame every 20 ms, on top of the usual timeout: the helper runs far faster.
            let timeout = engine.helper.timeout()
                + Duration::from_millis(20 * (u64::from(sub.frames) + job.fires.len() as u64));
            let measured = engine.helper.call_for(timeout, move |h| job.run(h))?;
            zero_page_used.extend(measured.zero_page);
            pages_written_outside.extend(measured.outside);
            max_play_cycles = max_play_cycles.max(measured.max_play_cycles);
            if let Some((call, why)) = measured.failure {
                stopped.push(pack_problem(
                    format!("subtune {}'s trace stopped: {call} failed: {why}", n + 1),
                    "check the packer's start address, and what the player writes outside its window (gt2reloc -W)",
                ));
            }
            if let Some(at) = measured.overwrote_driver {
                stopped.push(pack_problem(
                    format!(
                        "subtune {} wrote over the trace's driver at ${at:04X}, inside the window past the tune's \
                         end, where it went for want of room outside the window; what was measured after that \
                         can't be trusted",
                        n + 1
                    ),
                    "leave room outside the window for the trace, or check what the player writes past its end",
                ));
            }
        }
    }
    let facts = PackFacts {
        start: payload.load,
        size: payload.data.len() as u32,
        jump_table: jumps,
        zero_page_used,
        pages_written_outside,
        traced_window: window.filter(|_| traced),
        max_play_cycles,
    };
    let mut problems = check_pack(&facts, budget);
    problems.extend(stopped);
    Ok(GateReport { facts, problems })
}

/// A rule 8 error found by the gate itself rather than by `check_pack`.
fn pack_problem(message: String, hint: &str) -> Problem {
    Problem {
        rule: Rule::Pack,
        severity: Severity::Error,
        subtune: None,
        bar: None,
        row: None,
        voice: None,
        message,
        hint: hint.into(),
    }
}

/// The plan must name at least one subtune, each played at least once, an effect when one is to be fired, and, for a
/// PSID, as many subtunes as its header.
fn check_plan(packed: &Packed, budget: &PackBudget, plan: &TracePlan) -> Result<(), CommandError> {
    let bad = |m: String| Err(CommandError::new(ErrorKind::InvalidInput, m));
    if plan.subtunes.is_empty() || plan.subtunes.len() > 256 {
        return bad(format!(
            "the trace plan has {} subtunes; a packed song has 1 to 256",
            plan.subtunes.len()
        ));
    }
    if let Some(n) = plan.subtunes.iter().position(|s| s.frames == 0) {
        return bad(format!(
            "the trace plan plays subtune {} for no frames",
            n + 1
        ));
    }
    if budget.sound_effects
        && plan.effect.is_empty()
        && plan.subtunes.iter().any(|s| !s.effect_voices.is_empty())
    {
        return bad("the trace plan fires effects but has no effect data to fire".into());
    }
    if packed.format == Format::Sid {
        let bytes = std::fs::read(&packed.path).map_err(|e| {
            CommandError::new(
                ErrorKind::InvalidInput,
                format!("{}: {e}", packed.path.display()),
            )
        })?;
        let psid = Psid::parse(&bytes).map_err(|e| {
            CommandError::new(
                ErrorKind::InvalidInput,
                format!("{}: {e}", packed.path.display()),
            )
        })?;
        if usize::from(psid.songs) != plan.subtunes.len() {
            return bad(format!(
                "{} holds {} subtunes; the trace plan has {}",
                packed.path.display(),
                psid.songs,
                plan.subtunes.len()
            ));
        }
    }
    Ok(())
}

/// The driver: `JMP` (3 bytes), then the effect call (`LDA #`, `LDY #`, `LDX #`, `JMP`: 9 bytes), then 4 spare, then
/// the effect data.
const DRIVER_BYTES: usize = 16;
/// The driver's code: the `JMP` and the effect call.
const DRIVER_CODE: usize = 12;
/// Where in the driver the effect call starts, and where its `LDX #` operand is.
const FIRE: u16 = 3;
const FIRE_CHANNEL: u16 = FIRE + 5;

/// Where the driver and effect data go: their address, and whether it is inside the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Scratch {
    at: u16,
    in_window: bool,
}

/// A place for `len` bytes of driver and effect data that the trace can read without counting as the tune's, clear
/// of the tune, zero page, the stack, the I/O area (`$D000`–`$DFFF`) and the CPU's vectors. Outside the window when
/// there is room, page-aligned: `$C000` first, as a game's free RAM usually is, then the lowest page that fits.
/// Otherwise inside the window, past the tune's end, where GoatTracker's player doesn't write: the first address
/// after the tune, else the lowest page above it.
fn scratch(payload: &Payload, window: Option<(u16, u16)>, len: usize) -> Option<Scratch> {
    let len = len as u32;
    let tune = (
        u32::from(payload.load),
        u32::from(payload.load) + payload.data.len() as u32,
    );
    let span = |a: u32| (a, a + len);
    let apart = |(a, b): (u32, u32), (x, y): (u32, u32)| b <= x || a >= y;
    let usable = |base: u32| {
        let s = span(base);
        s.1 <= 0xFFFA && apart(s, (0xD000, 0xE000)) && apart(s, tune)
    };
    let outside =
        |base: u32| window.is_none_or(|(f, l)| apart(span(base), (u32::from(f), u32::from(l) + 1)));
    let pages = || (0x02..=0xFF).map(|p: u32| p << 8);
    if let Some(at) = std::iter::once(0xC000)
        .chain(pages())
        .find(|&base| usable(base) && outside(base))
    {
        return Some(Scratch {
            at: at as u16,
            in_window: false,
        });
    }
    let (first, last) = window?;
    let inside = |base: u32| base >= u32::from(first) && span(base).1 <= u32::from(last) + 1;
    std::iter::once(tune.1)
        .chain(pages().filter(|&p| p > tune.1))
        .find(|&base| usable(base) && inside(base))
        .map(|at| Scratch {
            at: at as u16,
            in_window: true,
        })
}

/// When to fire an effect while `frames` play calls run, and on which voice: every [`EFFECT_EVERY`] play calls,
/// cycling through `voices`, and closer together where that gives fewer than one per voice. Each is the number of
/// play calls before it, in order.
fn fires(frames: u32, voices: &[Voice]) -> Vec<(u32, Voice)> {
    if voices.is_empty() {
        return vec![];
    }
    let k = voices.len().max((frames / EFFECT_EVERY) as usize) as u64;
    (0..k)
        .map(|i| {
            let at = (2 * i + 1) * u64::from(frames) / (2 * k);
            (at as u32, voices[i as usize % voices.len()])
        })
        .collect()
}

/// One subtune's trace, run on the helper's thread.
struct Job {
    payload: Payload,
    window: Option<(u16, u16)>,
    scratch: Scratch,
    effect: Vec<u8>,
    subtune: u8,
    frames: u32,
    fires: Vec<(u32, Voice)>,
}

/// What one subtune's trace measured.
#[derive(Default)]
struct Measured {
    zero_page: BTreeSet<u8>,
    outside: BTreeSet<u8>,
    max_play_cycles: Option<u32>,
    /// The call that failed and stopped the trace, and the helper's reason.
    failure: Option<(String, String)>,
    /// The first driver address the player wrote a new value to, when the driver is inside the window.
    overwrote_driver: Option<u16>,
}

/// The driver for a tune at `start` whose effect data is at `fx`: `JMP play`, then the effect call that [`fire`]
/// points it at.
fn driver_code(start: u16, fx: u16) -> [u8; DRIVER_CODE] {
    let [play_lo, play_hi] = start.wrapping_add(3).to_le_bytes();
    let [fx_lo, fx_hi] = fx.to_le_bytes();
    let [entry_lo, entry_hi] = start.wrapping_add(6).to_le_bytes();
    [
        0x4C, play_lo, play_hi, // JMP play
        0xA9, fx_lo, // fire: LDA #<effect
        0xA0, fx_hi, // LDY #>effect
        0xA2, 0x00, // LDX #channel
        0x4C, entry_lo, entry_hi, // JMP start+6
    ]
}

/// Fire the effect on `voice` through the driver at `driver`, for a tune at `start`: one frame in which the driver
/// calls the sound-effect entry with the voice's channel offset in place of play, then back to play.
fn fire(h: &mut DynHelper, driver: u16, start: u16, voice: Voice) -> Result<(), HelperError> {
    let [fire_lo, fire_hi] = (driver + FIRE).to_le_bytes();
    let [play_lo, play_hi] = start.wrapping_add(3).to_le_bytes();
    h.poke(driver + FIRE_CHANNEL, 7 * (voice.get() - 1))?;
    h.poke(driver + 1, fire_lo)?;
    h.poke(driver + 2, fire_hi)?;
    h.render(1, false)?;
    h.poke(driver + 1, play_lo)?;
    h.poke(driver + 2, play_hi)
}

/// The helper's reason when `e` is the tune's own failure (a CPU jam, or a call that didn't return within its
/// budget), which the session survives; anything else is the helper's failure, and stays an error.
fn tune_failure(e: HelperError) -> Result<String, HelperError> {
    match e {
        HelperError::Refused {
            code: ErrorCode::CpuJam | ErrorCode::Timeout,
            message,
        } => Ok(message),
        e => Err(e),
    }
}

impl Job {
    fn run(self, h: &mut DynHelper) -> Result<Measured, HelperError> {
        let start = self.payload.load;
        let driver = self.scratch.at;
        let fx = driver + DRIVER_BYTES as u16;
        let code = driver_code(start, fx);
        h.load(start, &self.payload.data)?;
        h.load(driver, &code)?;
        if !self.effect.is_empty() {
            h.load(fx, &self.effect)?;
        }
        // Inside the window, a write over the driver wouldn't count as outside it: watch the driver instead.
        let watch: Vec<u16> = if self.scratch.in_window {
            (0..DRIVER_CODE as u16).map(|i| driver + i).collect()
        } else {
            vec![]
        };
        h.trace(true, self.window, &watch)?;
        let mut m = Measured::default();
        if let Err(e) = h.init(start, driver, self.subtune) {
            // A failed init leaves nothing to report.
            m.failure = Some(("its init call".into(), tune_failure(e)?));
            return Ok(m);
        }
        let chunk = h.max_render_frames(false).clamp(1, CHUNK_FRAMES);
        let mut played = 0;
        let mut fires = self.fires.iter().peekable();
        'trace: while played < self.frames {
            let until = fires
                .peek()
                .map_or(self.frames, |(at, _)| *at)
                .min(self.frames);
            while played < until {
                let n = (until - played).min(chunk);
                match h.render(n, false) {
                    Ok(r) => {
                        let cycles = r.max_play_cycles.saturating_sub(DRIVER_CYCLES);
                        m.max_play_cycles = m.max_play_cycles.max(Some(cycles));
                    }
                    Err(e) => {
                        let calls = if n == 1 {
                            format!("play call {}", played + 1)
                        } else {
                            format!("one of play calls {}–{}", played + 1, played + n)
                        };
                        m.failure = Some((calls, tune_failure(e)?));
                        break 'trace;
                    }
                }
                played += n;
            }
            while let Some(&&(_, voice)) = fires.peek().filter(|(at, _)| *at <= played) {
                fires.next();
                if let Err(e) = fire(h, driver, start, voice) {
                    let call = format!(
                        "the sound-effect call on voice {} after play call {played}",
                        voice.get()
                    );
                    m.failure = Some((call, tune_failure(e)?));
                    break 'trace;
                }
            }
        }
        // What was collected up to a failed play call can still be reported (emu-protocol.md, `Report`).
        let traced = h.report()?;
        m.zero_page = traced.zero_page;
        m.outside = traced.pages_written_outside;
        m.overwrote_driver = traced
            .watched
            .iter()
            .zip(code)
            .find(|(w, byte)| w.start != *byte || !w.changes.is_empty())
            .map(|(w, _)| w.address);
        Ok(m)
    }
}

/// A packing failure as a command error: gt2reloc missing, bad input, or gt2reloc misbehaving.
fn pack_error(e: PackError) -> CommandError {
    let kind = match &e {
        PackError::NotFound => ErrorKind::PackerMissing,
        PackError::Option(_)
        | PackError::NoSong(_)
        | PackError::NotASong(_)
        | PackError::NoOutputDir(_)
        | PackError::Format(_) => ErrorKind::InvalidInput,
        PackError::Io(_)
        | PackError::Failed { .. }
        | PackError::Timeout(_)
        | PackError::Version { .. }
        | PackError::Mismatch(_)
        | PackError::Binary { .. }
        | PackError::Replace { .. } => ErrorKind::PackerFailed,
    };
    CommandError::new(kind, e.to_string())
}

#[cfg(test)]
mod tests;
