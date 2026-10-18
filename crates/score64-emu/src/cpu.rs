//! The 6502 core behind a small trait (ADR-010), so the core can be replaced without touching the machine or the SID
//! side.
//!
//! The current core is `emulator_6502` 1.1.0. It performs all bus accesses of an instruction at once and reports the
//! cycle count, so this wrapper:
//!
//! 1. refuses JAM opcodes instead of letting the crate panic,
//! 2. corrects `STA (zp),Y` (`$91`), which the crate charges one cycle too many when the address crosses a page,
//! 3. places each write on the cycle a real NMOS 6502 performs it: the last write of an instruction on its last cycle,
//!    any earlier one a cycle before that. That rule holds for every store and read-modify-write instruction, the only
//!    instructions that can write the SID. The crate does not issue the dummy write of a read-modify-write (the old
//!    value, one cycle early); on the SID that write changes nothing, so only the final value is passed on.

use emulator_6502::{Interface6502, MOS6502};

/// Memory as the CPU sees it.
pub trait Bus {
    /// Read a byte. May have side effects (for example catching the SID up to the current cycle).
    fn read(&mut self, address: u16) -> u8;
    /// Write a byte. `cycle` is the offset of the write within the current instruction (0 = opcode fetch).
    fn write(&mut self, address: u16, value: u8, cycle: u8);
    /// Read a byte without side effects, for decoding the next instruction.
    fn peek(&self, address: u16) -> u8;
}

/// The registers of an NMOS 6502.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Registers {
    /// Program counter.
    pub pc: u16,
    /// Stack pointer (offset into page 1).
    pub s: u8,
    /// Accumulator.
    pub a: u8,
    /// Index register X.
    pub x: u8,
    /// Index register Y.
    pub y: u8,
    /// Status flags (NV-BDIZC).
    pub p: u8,
}

/// One executed instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    /// The opcode that ran.
    pub opcode: u8,
    /// Cycles it took.
    pub cycles: u8,
}

/// Why the CPU stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepError {
    /// A JAM (KIL) opcode: a real 6502 locks up until reset.
    Jam {
        /// Address of the opcode.
        pc: u16,
        /// The opcode.
        opcode: u8,
    },
}

impl std::fmt::Display for StepError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StepError::Jam { pc, opcode } => {
                write!(f, "CPU jammed: opcode ${opcode:02X} at ${pc:04X}")
            }
        }
    }
}

impl std::error::Error for StepError {}

/// A 6502 core that runs one instruction at a time.
pub trait Cpu {
    /// Current registers.
    fn registers(&self) -> Registers;
    /// Replace the registers.
    fn set_registers(&mut self, registers: Registers);
    /// Run one instruction.
    fn step(&mut self, bus: &mut dyn Bus) -> Result<Step, StepError>;
}

/// The twelve JAM opcodes of the NMOS 6502.
pub const JAM_OPCODES: [u8; 12] = [
    0x02, 0x12, 0x22, 0x32, 0x42, 0x52, 0x62, 0x72, 0x92, 0xB2, 0xD2, 0xF2,
];

/// `emulator_6502` behind [`Cpu`].
pub struct Emulator6502 {
    core: MOS6502,
}

impl Emulator6502 {
    /// A CPU with the given registers.
    pub fn new(registers: Registers) -> Self {
        let mut cpu = Self {
            core: MOS6502::new_start(registers.pc),
        };
        cpu.set_registers(registers);
        cpu
    }
}

/// Adapts our [`Bus`] to the crate's interface and collects the writes so they can be timed once the cycle count is
/// known.
struct Collect<'a> {
    bus: &'a mut dyn Bus,
    writes: Vec<(u16, u8)>,
}

impl Interface6502 for Collect<'_> {
    fn read(&mut self, address: u16) -> u8 {
        // Some undocumented read-modify-write opcodes read back what they just wrote; serve that from the writes
        // still waiting for their timestamps.
        match self.writes.iter().rev().find(|(a, _)| *a == address) {
            Some(&(_, value)) => value,
            None => self.bus.read(address),
        }
    }

    fn write(&mut self, address: u16, value: u8) {
        self.writes.push((address, value));
    }
}

impl Cpu for Emulator6502 {
    fn registers(&self) -> Registers {
        Registers {
            pc: self.core.get_program_counter(),
            s: self.core.get_stack_pointer(),
            a: self.core.get_accumulator(),
            x: self.core.get_x_register(),
            y: self.core.get_y_register(),
            p: self.core.get_status_register(),
        }
    }

    fn set_registers(&mut self, r: Registers) {
        self.core.set_program_counter(r.pc);
        self.core.set_stack_pointer(r.s);
        self.core.set_accumulator(r.a);
        self.core.set_x_register(r.x);
        self.core.set_y_register(r.y);
        self.core.set_status_register(r.p);
    }

    fn step(&mut self, bus: &mut dyn Bus) -> Result<Step, StepError> {
        let pc = self.core.get_program_counter();
        let opcode = bus.peek(pc);
        if JAM_OPCODES.contains(&opcode) {
            return Err(StepError::Jam { pc, opcode });
        }
        // STA (zp),Y is always 6 cycles; the crate adds a page-cross cycle.
        let sta_izy_cross = opcode == 0x91 && {
            let zp = bus.peek(pc.wrapping_add(1));
            let base = u16::from_le_bytes([
                bus.peek(u16::from(zp)),
                bus.peek(u16::from(zp.wrapping_add(1))),
            ]);
            let y = u16::from(self.core.get_y_register());
            (base & 0xFF00) != (base.wrapping_add(y) & 0xFF00)
        };

        let mut collect = Collect {
            bus,
            writes: Vec::with_capacity(3),
        };
        let mut cycles: u8 = 0;
        loop {
            self.core.cycle(&mut collect);
            cycles += 1;
            if self.core.get_remaining_cycles() == 0 {
                break;
            }
        }
        if sta_izy_cross {
            cycles -= 1;
        }

        let n = collect.writes.len();
        for (k, &(address, value)) in collect.writes.iter().enumerate() {
            let from_end = (n - 1 - k) as u8;
            collect
                .bus
                .write(address, value, cycles.saturating_sub(1 + from_end));
        }
        Ok(Step { opcode, cycles })
    }
}

/// Whether an opcode is one of the 151 documented NMOS 6502 opcodes.
pub fn is_documented(opcode: u8) -> bool {
    const DOCUMENTED: [u64; 4] = documented_bitmap();
    DOCUMENTED[usize::from(opcode >> 6)] & (1 << (opcode & 63)) != 0
}

const fn documented_bitmap() -> [u64; 4] {
    const OPS: [u8; 151] = [
        0x00, 0x01, 0x05, 0x06, 0x08, 0x09, 0x0A, 0x0D, 0x0E, 0x10, 0x11, 0x15, 0x16, 0x18, 0x19,
        0x1D, 0x1E, 0x20, 0x21, 0x24, 0x25, 0x26, 0x28, 0x29, 0x2A, 0x2C, 0x2D, 0x2E, 0x30, 0x31,
        0x35, 0x36, 0x38, 0x39, 0x3D, 0x3E, 0x40, 0x41, 0x45, 0x46, 0x48, 0x49, 0x4A, 0x4C, 0x4D,
        0x4E, 0x50, 0x51, 0x55, 0x56, 0x58, 0x59, 0x5D, 0x5E, 0x60, 0x61, 0x65, 0x66, 0x68, 0x69,
        0x6A, 0x6C, 0x6D, 0x6E, 0x70, 0x71, 0x75, 0x76, 0x78, 0x79, 0x7D, 0x7E, 0x81, 0x84, 0x85,
        0x86, 0x88, 0x8A, 0x8C, 0x8D, 0x8E, 0x90, 0x91, 0x94, 0x95, 0x96, 0x98, 0x99, 0x9A, 0x9D,
        0xA0, 0xA1, 0xA2, 0xA4, 0xA5, 0xA6, 0xA8, 0xA9, 0xAA, 0xAC, 0xAD, 0xAE, 0xB0, 0xB1, 0xB4,
        0xB5, 0xB6, 0xB8, 0xB9, 0xBA, 0xBC, 0xBD, 0xBE, 0xC0, 0xC1, 0xC4, 0xC5, 0xC6, 0xC8, 0xC9,
        0xCA, 0xCC, 0xCD, 0xCE, 0xD0, 0xD1, 0xD5, 0xD6, 0xD8, 0xD9, 0xDD, 0xDE, 0xE0, 0xE1, 0xE4,
        0xE5, 0xE6, 0xE8, 0xE9, 0xEA, 0xEC, 0xED, 0xEE, 0xF0, 0xF1, 0xF5, 0xF6, 0xF8, 0xF9, 0xFD,
        0xFE,
    ];
    let mut map = [0u64; 4];
    let mut i = 0;
    while i < OPS.len() {
        map[(OPS[i] >> 6) as usize] |= 1 << (OPS[i] & 63);
        i += 1;
    }
    map
}

#[cfg(test)]
mod conformance;
