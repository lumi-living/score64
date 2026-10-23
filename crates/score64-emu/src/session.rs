//! One protocol session (`docs/reference/emu-protocol.md`): the machine, the SID settings and the play timeline.

use score64_formats::emu_protocol::{self as proto, ErrorCode};

use crate::cpu::{Emulator6502, Registers, StepError};
use crate::machine::{CallError, Machine, Memory, PAL_CLOCK_HZ, PAL_CYCLES_PER_FRAME, TraceLog};
use crate::sid::{Chip, Engine, SidClock};

/// A request that failed, as the reply will report it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub code: ErrorCode,
    pub message: String,
}

impl Failure {
    fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl From<CallError> for Failure {
    fn from(e: CallError) -> Self {
        let code = match e {
            CallError::Cpu(StepError::Jam { .. }) => ErrorCode::CpuJam,
            CallError::Timeout { .. } => ErrorCode::Timeout,
        };
        Failure::new(code, e.to_string())
    }
}

/// What `Init` set up.
struct Timeline {
    play: u16,
    /// Cycle of the first play call (frame 0).
    first_play: u64,
    /// Frames rendered since `Init`.
    frames: u64,
}

/// What `Trace` asked for, applied at the next `Init`.
#[derive(Clone, Debug, Default)]
struct TraceRequest {
    zero_page: bool,
    outside: Option<(u16, u16)>,
    watch: Vec<u16>,
}

/// One watched address on a traced timeline.
struct Watch {
    address: u16,
    /// Its value at the start of the current span (after `Init`, or at the last `Report`).
    start: u8,
    /// Its value after the latest play call (or poke).
    last: u8,
    /// Changes since the last `Report`.
    changes: Vec<(u32, u8)>,
}

/// Everything the helper holds between requests after `Hello`.
pub struct Session {
    machine: Machine<Emulator6502>,
    sample_rate: u32,
    timeline: Option<Timeline>,
    /// The trace to start at the next `Init`, if any.
    armed: Option<TraceRequest>,
    /// The watched addresses, while the timeline is traced.
    watches: Vec<Watch>,
    /// Frames rendered since the last `Report`, for the size of the next one.
    unreported_frames: u64,
}

/// The rendered frames, ready for a `Rendered` reply.
pub struct Rendered {
    pub frames: u32,
    pub max_play_cycles: u32,
    pub samples: Vec<i16>,
    pub writes: Vec<proto::SidWrite>,
}

/// Cycles `init` may take: one second.
const INIT_BUDGET: u64 = PAL_CLOCK_HZ as u64;

impl Session {
    /// A fresh machine (empty RAM, 6581) producing audio at `sample_rate`.
    pub fn new(sample_rate: u32) -> Result<Self, Failure> {
        let sid = SidClock::new(Engine::Residfp, Chip::Mos6581, PAL_CLOCK_HZ, sample_rate)
            .map_err(|e| Failure::new(ErrorCode::Internal, e))?;
        Ok(Self {
            machine: Machine::new(Emulator6502::new(Registers::default()), Memory::new(sid)),
            sample_rate,
            timeline: None,
            armed: None,
            watches: Vec::new(),
            unreported_frames: 0,
        })
    }

    /// `Trace`: arm a trace for the next `Init`, or disarm with everything off.
    pub fn trace(
        &mut self,
        zero_page: bool,
        outside: Option<(u16, u16)>,
        watch: Vec<u16>,
    ) -> Result<(), Failure> {
        let limit = |m: String| Err(Failure::new(ErrorCode::Limit, m));
        if watch.len() > proto::MAX_WATCH {
            return limit(format!(
                "{} watched addresses; at most {}",
                watch.len(),
                proto::MAX_WATCH
            ));
        }
        if let Some(a) = watch.iter().find(|a| (0xD400..=0xD7FF).contains(*a)) {
            return limit(format!(
                "${a:04X} is a SID register, not RAM; SID writes come with Render"
            ));
        }
        let mut seen = std::collections::BTreeSet::new();
        if let Some(a) = watch.iter().find(|a| !seen.insert(**a)) {
            return limit(format!("${a:04X} is watched twice"));
        }
        if let Some((first, last)) = outside
            && first > last
        {
            return limit(format!(
                "window ${first:04X}–${last:04X} ends before it starts"
            ));
        }
        self.armed =
            (zero_page || outside.is_some() || !watch.is_empty()).then_some(TraceRequest {
                zero_page,
                outside,
                watch,
            });
        Ok(())
    }

    /// `Report`: what the trace collected since `Init`, and the watched changes since the last `Report`.
    pub fn report(&mut self) -> Result<proto::Traced, Failure> {
        let Some(log) = self.machine.memory.trace.as_ref() else {
            return Err(Failure::new(
                ErrorCode::Sequence,
                "report without a traced Init: send Trace, then Init",
            ));
        };
        let set = |bits: &[bool; 256]| (0..=255u8).filter(|&n| bits[usize::from(n)]).collect();
        let traced = proto::Traced {
            zero_page: set(&log.zero_page_used),
            pages_written_outside: set(&log.pages_outside),
            watched: self
                .watches
                .iter_mut()
                .map(|w| {
                    let start = std::mem::replace(&mut w.start, w.last);
                    proto::Watched {
                        address: w.address,
                        start,
                        changes: std::mem::take(&mut w.changes),
                    }
                })
                .collect(),
        };
        self.unreported_frames = 0;
        Ok(traced)
    }

    /// `Load`: copy bytes into RAM.
    pub fn load(&mut self, address: u16, data: &[u8]) -> Result<(), Failure> {
        if usize::from(address) + data.len() > 0x10000 {
            return Err(Failure::new(
                ErrorCode::Limit,
                format!("{} bytes at ${address:04X} run past $FFFF", data.len()),
            ));
        }
        self.machine.memory.load(address, data);
        Ok(())
    }

    /// `Poke`: one byte now.
    pub fn poke(&mut self, address: u16, value: u8) {
        let now = self.machine.cycle;
        self.machine.memory.poke(address, value, now);
        // The host's own change isn't one the player made.
        for w in self.watches.iter_mut().filter(|w| w.address == address) {
            w.last = value;
        }
    }

    /// `Model`: a new SID of the given model, joining the timeline where the machine is.
    pub fn model(&mut self, chip: Chip) -> Result<(), Failure> {
        let mut sid = SidClock::new(Engine::Residfp, chip, PAL_CLOCK_HZ, self.sample_rate)
            .map_err(|e| Failure::new(ErrorCode::Internal, e))?;
        let now = self.machine.cycle;
        self.machine.memory.sid.advance_to(now);
        let pending = std::mem::take(&mut self.machine.memory.sid.samples);
        sid.start_at(now);
        sid.samples = pending;
        self.machine.memory.sid = sid;
        Ok(())
    }

    /// `Init`: call `init` with A = `subtune` and start a new timeline whose frame 0 is the next frame boundary.
    pub fn init(&mut self, init: u16, play: u16, subtune: u8) -> Result<(), Failure> {
        self.timeline = None;
        self.watches.clear();
        self.unreported_frames = 0;
        self.machine.memory.trace = self
            .armed
            .as_ref()
            .map(|t| TraceLog::new(t.zero_page, t.outside));
        if let Err(e) = self.machine.call(init, subtune, 0, 0, INIT_BUDGET) {
            self.machine.memory.trace = None;
            return Err(e.into());
        }
        if let Some(t) = &self.armed {
            let memory = &self.machine.memory;
            self.watches = t
                .watch
                .iter()
                .map(|&address| {
                    let v = crate::cpu::Bus::peek(memory, address);
                    Watch {
                        address,
                        start: v,
                        last: v,
                        changes: Vec::new(),
                    }
                })
                .collect();
        }
        // Run up to frame 0 and drop what init and the lead-in produced: a render covers exactly its frames, and the
        // write log starts at frame 0. (Init's SID writes still reach the SID.)
        let first_play = (self.machine.cycle / PAL_CYCLES_PER_FRAME + 1) * PAL_CYCLES_PER_FRAME;
        self.machine.idle_until(first_play);
        self.machine.memory.writes.clear();
        self.machine.memory.sid.samples.clear();
        self.timeline = Some(Timeline {
            play,
            first_play,
            frames: 0,
        });
        Ok(())
    }

    /// `Render`: run `frames` frames and hand back their audio (and SID writes, if asked).
    pub fn render(&mut self, frames: u32, with_writes: bool) -> Result<Rendered, Failure> {
        if frames == 0 || frames > proto::MAX_RENDER_FRAMES {
            return Err(Failure::new(
                ErrorCode::Limit,
                format!(
                    "frames must be 1..={}, not {frames}",
                    proto::MAX_RENDER_FRAMES
                ),
            ));
        }
        let t = self
            .timeline
            .as_mut()
            .ok_or_else(|| Failure::new(ErrorCode::Sequence, "render before init"))?;
        // Refuse before running anything if the reply could outgrow a message; the session stays where it is.
        // Samples and writes already pending (from Model or Poke since the last render) go in the same reply.
        let pending_writes = if with_writes {
            self.machine.memory.writes.len() as u64
        } else {
            0
        };
        let bound = proto::render_reply_bound(frames, self.sample_rate, with_writes)
            + 2 * self.machine.memory.sid.samples.len() as u64
            + 8 * pending_writes;
        // A traced timeline also keeps every watched change until the next Report, which must fit a message too.
        let watched = self.watches.len() as u64;
        let report_bound = proto::traced_size(
            self.watches.len(),
            watched * (self.unreported_frames + u64::from(frames)),
        );
        if report_bound > u64::from(proto::MAX_MESSAGE) {
            return Err(Failure::new(
                ErrorCode::Limit,
                format!(
                    "{} frames since the last Report with {watched} watched addresses could need a {report_bound}-byte \
                     Report; send Report first",
                    self.unreported_frames + u64::from(frames)
                ),
            ));
        }
        if bound > u64::from(proto::MAX_MESSAGE) {
            return Err(Failure::new(
                ErrorCode::Limit,
                format!(
                    "{frames} frames at {} Hz{} could need a {bound}-byte reply, over the {}-byte limit; \
                     render at most {} frames at a time",
                    self.sample_rate,
                    if with_writes { " with writes" } else { "" },
                    proto::MAX_MESSAGE,
                    proto::max_render_frames(self.sample_rate, with_writes),
                ),
            ));
        }
        let (play, first_play, start) = (t.play, t.first_play, t.frames);
        let mut max_play = 0u64;
        for k in start..start + u64::from(frames) {
            self.machine
                .idle_until(first_play + k * PAL_CYCLES_PER_FRAME);
            let cycles = self.machine.call(play, 0, 0, 0, PAL_CYCLES_PER_FRAME);
            let cycles = match cycles {
                Ok(c) => c,
                Err(e) => {
                    // The timeline is broken mid-frame; a new Init is needed.
                    self.timeline = None;
                    return Err(e.into());
                }
            };
            max_play = max_play.max(cycles);
            for w in &mut self.watches {
                let v = crate::cpu::Bus::peek(&self.machine.memory, w.address);
                if v != w.last {
                    w.changes.push((k as u32, v));
                    w.last = v;
                }
            }
        }
        self.unreported_frames += u64::from(frames);
        let end = first_play + (start + u64::from(frames)) * PAL_CYCLES_PER_FRAME;
        self.machine.idle_until(end);
        if let Some(t) = self.timeline.as_mut() {
            t.frames += u64::from(frames);
        }
        let samples = std::mem::take(&mut self.machine.memory.sid.samples);
        let raw = std::mem::take(&mut self.machine.memory.writes);
        let writes = if with_writes {
            raw.iter()
                .map(|w| {
                    let rel = w.cycle - first_play;
                    proto::SidWrite {
                        frame: (rel / PAL_CYCLES_PER_FRAME) as u32,
                        cycle: (rel % PAL_CYCLES_PER_FRAME) as u16,
                        register: w.register,
                        value: w.value,
                    }
                })
                .collect()
        } else {
            Vec::new()
        };
        Ok(Rendered {
            frames,
            max_play_cycles: max_play as u32,
            samples,
            writes,
        })
    }
}
