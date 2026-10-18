//! The minimal C64 a packed music player needs, and the harness that calls it.
//!
//! Memory map (TAD §5.1):
//! - 64 KB of flat RAM, no ROMs and no bank switching. `$00`/`$01` are plain RAM, not the 6510 port: without ROMs
//!   there is nothing for the port to switch. Writes to them are counted so a player that relies on banking is noticed.
//! - The SID at `$D400`, mirrored every 32 bytes up to `$D7FF`. Registers `$1D`–`$1F` of each mirror are ignored.
//!   Everything else in `$D000`–`$DFFF` is plain RAM.
//!
//! Interrupt model: none. The harness calls init once and then play once per PAL frame (19,656 cycles) from its own
//! loop, the way a game's raster interrupt would. The IRQ and NMI vectors are not used and no CIA or VIC is emulated,
//! so a player that installs its own interrupt (PSID play address 0, or RSID) is rejected.

use crate::cpu::{Bus, Cpu, Registers, StepError};
use crate::sid::SidClock;

/// CPU cycles per PAL frame (312 lines of 63 cycles).
pub const PAL_CYCLES_PER_FRAME: u64 = 19_656;
/// PAL CPU clock in Hz.
pub const PAL_CLOCK_HZ: u32 = 985_248;

/// The harness returns here: the address it pushes as the return address of init and play. Execution stops when
/// the PC reaches it, so the byte there is never run.
const RETURN_TRAP: u16 = 0xFFF0;

/// One SID register write, as the player made it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SidWrite {
    /// Absolute cycle since the machine started.
    pub cycle: u64,
    /// Register `$00`–`$1C`.
    pub register: u8,
    /// Value written.
    pub value: u8,
}

/// RAM, the SID and the cycle counter.
pub struct Memory {
    ram: Box<[u8; 0x10000]>,
    /// The SID, clocked lazily up to the cycle of each access.
    pub sid: SidClock,
    /// Cycle at which the current instruction started.
    instruction_start: u64,
    /// Every SID write so far, in order.
    pub writes: Vec<SidWrite>,
    /// Writes to `$00`/`$01`.
    pub port_writes: u32,
}

impl Memory {
    /// Empty RAM and a fresh SID.
    pub fn new(sid: SidClock) -> Self {
        Self {
            ram: Box::new([0; 0x10000]),
            sid,
            instruction_start: 0,
            writes: Vec::new(),
            port_writes: 0,
        }
    }

    /// Copy `data` into RAM at `address`.
    pub fn load(&mut self, address: u16, data: &[u8]) {
        let start = usize::from(address);
        let end = (start + data.len()).min(0x10000);
        self.ram[start..end].copy_from_slice(&data[..end - start]);
    }

    fn sid_register(address: u16) -> Option<u8> {
        (0xD400..=0xD7FF)
            .contains(&address)
            .then_some((address & 0x1F) as u8)
    }
}

impl Bus for Memory {
    fn read(&mut self, address: u16) -> u8 {
        match Self::sid_register(address) {
            Some(register) => {
                // Reads happen early in an instruction; catching up to its start is within a few cycles.
                self.sid.advance_to(self.instruction_start);
                self.sid.read(register)
            }
            None => self.ram[usize::from(address)],
        }
    }

    fn write(&mut self, address: u16, value: u8, cycle: u8) {
        if address <= 1 {
            self.port_writes += 1;
        }
        match Self::sid_register(address) {
            Some(register) if register <= 0x1C => {
                let at = self.instruction_start + u64::from(cycle);
                self.sid.write_at(at, register, value);
                self.writes.push(SidWrite {
                    cycle: at,
                    register,
                    value,
                });
            }
            Some(_) => {}
            None => self.ram[usize::from(address)] = value,
        }
    }

    fn peek(&self, address: u16) -> u8 {
        self.ram[usize::from(address)]
    }
}

/// Why a call into the player failed.
#[derive(Debug)]
pub enum CallError {
    /// The CPU hit a JAM opcode.
    Cpu(StepError),
    /// The routine did not return within the cycle budget.
    Timeout {
        /// Routine address.
        routine: u16,
        /// Cycle budget.
        budget: u64,
    },
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CallError::Cpu(e) => e.fmt(f),
            CallError::Timeout { routine, budget } => {
                write!(
                    f,
                    "routine at ${routine:04X} did not return within {budget} cycles"
                )
            }
        }
    }
}

impl std::error::Error for CallError {}

/// A CPU, its memory and a cycle counter.
pub struct Machine<C: Cpu> {
    /// The CPU.
    pub cpu: C,
    /// RAM and SID.
    pub memory: Memory,
    /// Absolute cycle count.
    pub cycle: u64,
    /// How many times each opcode ran.
    pub opcode_counts: [u64; 256],
}

impl<C: Cpu> Machine<C> {
    /// A machine around `cpu` and `memory`, at cycle 0.
    pub fn new(cpu: C, memory: Memory) -> Self {
        Self {
            cpu,
            memory,
            cycle: 0,
            opcode_counts: [0; 256],
        }
    }

    /// Call `routine` like `JSR routine` with the given A, X and Y, and run until it returns. Returns the cycles it took.
    pub fn call(
        &mut self,
        routine: u16,
        a: u8,
        x: u8,
        y: u8,
        budget: u64,
    ) -> Result<u64, CallError> {
        // Push the return address the way JSR does (address of the trap minus one).
        let mut s = 0xFF_u8;
        let ret = RETURN_TRAP.wrapping_sub(1).to_le_bytes();
        self.memory.load(0x0100 | u16::from(s), &[ret[1]]);
        s = s.wrapping_sub(1);
        self.memory.load(0x0100 | u16::from(s), &[ret[0]]);
        s = s.wrapping_sub(1);
        // Keep the flags (decimal mode in particular) of the previous call cleared, as a KERNAL IRQ entry would.
        self.cpu.set_registers(Registers {
            pc: routine,
            s,
            a,
            x,
            y,
            p: 0x24,
        });

        let start = self.cycle;
        while self.cpu.registers().pc != RETURN_TRAP {
            if self.cycle - start > budget {
                return Err(CallError::Timeout { routine, budget });
            }
            self.memory.instruction_start = self.cycle;
            let step = self.cpu.step(&mut self.memory).map_err(CallError::Cpu)?;
            self.opcode_counts[usize::from(step.opcode)] += 1;
            self.cycle += u64::from(step.cycles);
        }
        Ok(self.cycle - start)
    }

    /// Let the SID run, with the CPU idle, up to `cycle`.
    pub fn idle_until(&mut self, cycle: u64) {
        self.cycle = self.cycle.max(cycle);
        self.memory.sid.advance_to(self.cycle);
    }
}
