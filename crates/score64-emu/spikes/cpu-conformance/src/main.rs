//! Score each candidate 6502 core against SingleStepTests/65x02 (6502/v1).
use serde::Deserialize;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Deserialize, Clone)]
struct St {
    pc: u16,
    s: u8,
    a: u8,
    x: u8,
    y: u8,
    p: u8,
    ram: Vec<(u16, u8)>,
}
#[derive(Deserialize)]
struct Case {
    initial: St,
    #[serde(rename = "final")]
    fin: St,
    cycles: Vec<(u16, u8, String)>,
}

#[derive(Default, Clone, Copy)]
struct Score {
    state: u32,
    cycles: u32,
    bus: u32,
    total: u32,
    panics: u32,
}

struct Out {
    pc: u16,
    s: u8,
    a: u8,
    x: u8,
    y: u8,
    p: u8,
    mem: Vec<u8>,
    cyc: usize,
    bus: Vec<(u16, u8, bool)>,
}

type Mem = Rc<RefCell<(Vec<u8>, Vec<(u16, u8, bool)>)>>;
fn mem_from(st: &St) -> Mem {
    let mut m = vec![0u8; 65536];
    for &(a, v) in &st.ram {
        m[a as usize] = v;
    }
    Rc::new(RefCell::new((m, Vec::new())))
}
fn rd(m: &Mem, a: u16) -> u8 {
    let mut b = m.borrow_mut();
    let v = b.0[a as usize];
    b.1.push((a, v, false));
    v
}
fn wr(m: &Mem, a: u16, v: u8) {
    let mut b = m.borrow_mut();
    b.0[a as usize] = v;
    b.1.push((a, v, true));
}

// --- mos6502
struct B1(Mem);
impl mos6502::memory::Bus for B1 {
    fn get_byte(&mut self, a: u16) -> u8 {
        rd(&self.0, a)
    }
    fn set_byte(&mut self, a: u16, v: u8) {
        wr(&self.0, a, v)
    }
}
fn run_mos6502(i: &St) -> Out {
    use mos6502::{
        cpu::CPU,
        instruction::Nmos6502,
        registers::{StackPointer, Status},
    };
    let m = mem_from(i);
    let mut c = CPU::new(B1(m.clone()), Nmos6502);
    c.registers.accumulator = i.a;
    c.registers.index_x = i.x;
    c.registers.index_y = i.y;
    c.registers.stack_pointer = StackPointer(i.s);
    c.registers.program_counter = i.pc;
    c.registers.status = Status::from_bits_truncate(i.p);
    let c0 = c.cycles;
    c.single_step();
    let r = &c.registers;
    let b = m.borrow();
    Out {
        pc: r.program_counter,
        s: r.stack_pointer.0,
        a: r.accumulator,
        x: r.index_x,
        y: r.index_y,
        p: r.status.bits(),
        mem: b.0.clone(),
        cyc: (c.cycles - c0) as usize,
        bus: b.1.clone(),
    }
}
// --- emulator_6502
struct B2(Mem);
impl emulator_6502::Interface6502 for B2 {
    fn read(&mut self, a: u16) -> u8 {
        rd(&self.0, a)
    }
    fn write(&mut self, a: u16, v: u8) {
        wr(&self.0, a, v)
    }
}
fn run_emu6502(i: &St) -> Out {
    let m = mem_from(i);
    let mut bus = B2(m.clone());
    let mut c = emulator_6502::MOS6502::new_start(i.pc);
    c.set_accumulator(i.a);
    c.set_x_register(i.x);
    c.set_y_register(i.y);
    c.set_stack_pointer(i.s);
    c.set_status_register(i.p);
    let mut n = 0;
    loop {
        c.cycle(&mut bus);
        n += 1;
        if c.get_remaining_cycles() == 0 {
            break;
        }
    }
    let b = m.borrow();
    Out {
        pc: c.get_program_counter(),
        s: c.get_stack_pointer(),
        a: c.get_accumulator(),
        x: c.get_x_register(),
        y: c.get_y_register(),
        p: c.get_status_register(),
        mem: b.0.clone(),
        cyc: n,
        bus: b.1.clone(),
    }
}
// --- mos6502_model
struct B3(Mem);
impl mos6502_model::machine::Memory for B3 {
    fn read_u8(&mut self, a: u16) -> u8 {
        rd(&self.0, a)
    }
    fn write_u8(&mut self, a: u16, v: u8) {
        wr(&self.0, a, v)
    }
}
fn run_model(i: &St) -> Out {
    let m = mem_from(i);
    let mut bus = B3(m.clone());
    let mut c = mos6502_model::machine::Cpu::new();
    c.pc = i.pc;
    c.sp = i.s;
    c.acc = i.a;
    c.x = i.x;
    c.y = i.y;
    c.status.set(i.p);
    let n = c.step(&mut bus).unwrap_or(0) as usize;
    let b = m.borrow();
    Out {
        pc: c.pc,
        s: c.sp,
        a: c.acc,
        x: c.x,
        y: c.y,
        p: c.status.masked_with_brk_and_expansion(),
        mem: b.0.clone(),
        cyc: n,
        bus: b.1.clone(),
    }
}
// --- zinc64 (GPL)
struct B4(Mem);
impl zinc64_core::Addressable for B4 {
    fn read(&self, a: u16) -> u8 {
        rd(&self.0, a)
    }
    fn write(&mut self, a: u16, v: u8) {
        wr(&self.0, a, v)
    }
}
fn run_zinc(i: &St) -> Out {
    use zinc64_core::{Cpu, IoPort, IrqLine, Pin, new_shared};
    let m = mem_from(i);
    let mem: zinc64_core::Shared<dyn zinc64_core::Addressable> = new_shared(B4(m.clone()));
    let mut c = zinc64_emu::cpu::Cpu6510::new(
        mem,
        new_shared(IoPort::new(0x00, 0xff)),
        new_shared(Pin::new_high()),
        new_shared(IrqLine::new("irq")),
        new_shared(IrqLine::new("nmi")),
    );
    c.set_pc(i.pc);
    c.set_sp(i.s);
    c.set_a(i.a);
    c.set_x(i.x);
    c.set_y(i.y);
    c.set_p(i.p);
    let ticks = Rc::new(std::cell::Cell::new(0usize));
    let t2 = ticks.clone();
    let tick: zinc64_core::TickFn = Rc::new(move || t2.set(t2.get() + 1));
    c.step(&tick);
    let b = m.borrow();
    Out {
        pc: c.get_pc(),
        s: c.get_sp(),
        a: c.get_a(),
        x: c.get_x(),
        y: c.get_y(),
        p: c.get_p(),
        mem: b.0.clone(),
        cyc: ticks.get(),
        bus: b.1.clone(),
    }
}

fn judge(o: &Out, t: &Case, s: &mut Score) {
    let f = &t.fin;
    let regs = o.pc == f.pc
        && o.s == f.s
        && o.a == f.a
        && o.x == f.x
        && o.y == f.y
        && (o.p | 0x30) == (f.p | 0x30);
    let ram = f.ram.iter().all(|&(a, v)| o.mem[a as usize] == v);
    if regs && ram {
        s.state += 1;
    }
    if o.cyc == t.cycles.len() {
        s.cycles += 1;
    }
    let want: Vec<(u16, u8, bool)> = t
        .cycles
        .iter()
        .map(|(a, v, k)| (*a, *v, k == "write"))
        .collect();
    if o.bus == want {
        s.bus += 1;
    }
}

fn main() {
    let dir = std::env::args().nth(1).expect("dir");
    let per = std::env::args()
        .nth(2)
        .map(|s| s.parse().unwrap())
        .unwrap_or(10000usize);
    std::panic::set_hook(Box::new(|_| {}));
    let names = ["mos6502", "emulator_6502", "mos6502_model", "zinc64-emu"];
    let runners: [fn(&St) -> Out; 4] = [run_mos6502, run_emu6502, run_model, run_zinc];
    println!(
        "op\t{}",
        names.map(|n| format!("{n}:state/cyc/bus")).join("\t")
    );
    for op in 0..256u32 {
        let path = format!("{dir}/{op:02x}.json");
        let cases: Vec<Case> = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let mut sc = [Score::default(); 4];
        for t in cases.iter().take(per) {
            for (k, r) in runners.iter().enumerate() {
                sc[k].total += 1;
                match std::panic::catch_unwind(|| r(&t.initial)) {
                    Ok(o) => judge(&o, t, &mut sc[k]),
                    Err(_) => sc[k].panics += 1,
                }
            }
        }
        let cells: Vec<String> = sc
            .iter()
            .map(|s| format!("{}/{}/{}/{}p", s.state, s.cycles, s.bus, s.panics))
            .collect();
        println!("{op:02x}\t{}\tn={}", cells.join("\t"), sc[0].total);
    }
}
