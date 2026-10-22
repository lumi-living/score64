//! One protocol session (`docs/reference/emu-protocol.md`): the machine, the SID settings and the play timeline.

use score64_formats::emu_protocol::{self as proto, ErrorCode};

use crate::cpu::{Emulator6502, Registers, StepError};
use crate::machine::{CallError, Machine, Memory, PAL_CLOCK_HZ, PAL_CYCLES_PER_FRAME};
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

/// Everything the helper holds between requests after `Hello`.
pub struct Session {
    machine: Machine<Emulator6502>,
    sample_rate: u32,
    timeline: Option<Timeline>,
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
        })
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
        self.machine.call(init, subtune, 0, 0, INIT_BUDGET)?;
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
        }
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
