//! GPL-3.0-or-later helper process: a 6502 core plus SID emulation that runs packed C64 music players, renders them
//! to PCM and drives live audition. It is a separate binary so the rest of Score64 stays MIT (ADR-002); other
//! crates must never depend on it as a library.
//!
//! For now it has two command-line modes, `render` and `replay`, used by the E3 spikes and fixtures. The versioned
//! stdin/stdout protocol of TAD §5.1 arrives with #8 and will drive the same machine. The SID engine is reSIDfp by
//! default (ADR-011).

mod cpu;
mod machine;
mod psid;
mod sid;

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use cpu::{Emulator6502, Registers};
use machine::{Machine, Memory, PAL_CLOCK_HZ, PAL_CYCLES_PER_FRAME};
use psid::Psid;
use sid::{Chip, Engine, SidClock};

#[derive(Parser)]
#[command(
    name = "score64-emu",
    version,
    about = "Run packed C64 music players (GPL helper)"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Render a PSID tune (one subtune) to a WAV file.
    Render {
        /// PSID file, for example from `gt2reloc song.sng song.sid -D1 -H1 -Z..`.
        sid: PathBuf,
        /// Subtune, 1-based (default: the file's start song).
        #[arg(long)]
        subtune: Option<u16>,
        /// Length in seconds.
        #[arg(long, default_value_t = 30.0)]
        seconds: f64,
        /// SID model: 6581 or 8580.
        #[arg(long, default_value = "6581")]
        chip: Chip,
        /// SID emulation: residfp (reSIDfp, sidplayfp's settings) or resid-rs.
        #[arg(long, default_value = "residfp")]
        engine: Engine,
        /// Voices to silence, for example `--mute 2,3` to hear voice 1 alone.
        #[arg(long, value_delimiter = ',', value_parser = clap::value_parser!(u8).range(1..=3))]
        mute: Vec<u8>,
        /// Output sample rate.
        #[arg(long, default_value_t = 48_000)]
        rate: u32,
        /// Output WAV (16-bit mono).
        #[arg(long)]
        wav: PathBuf,
        /// Also write every SID register write (frame, cycle in frame, register, value) as TSV.
        #[arg(long)]
        writes: Option<PathBuf>,
        /// Also write how often each opcode ran as TSV.
        #[arg(long)]
        opcodes: Option<PathBuf>,
        /// Sound-effect data (GoatTracker format, for example from `ins2snd2 -b`) to trigger through the player's
        /// sound-effect entry (start+6; only for tunes packed with `-D1`). Repeat to rotate several effects; the n-th
        /// is loaded at `$C000 + n * $80`.
        #[arg(long)]
        effect: Vec<PathBuf>,
        /// Trigger an effect every this many frames.
        #[arg(long, default_value_t = 50)]
        effect_every: u64,
        /// Voice the effect plays on (1–3).
        #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u8).range(1..=3))]
        effect_voice: u8,
    },
    /// Play a VICE `-sounddev dump` register log (lines of `cycles-since-last register value`) through the same SID,
    /// to separate CPU and timing differences from SID-model differences.
    Replay {
        /// VICE dump file.
        dump: PathBuf,
        /// Length in seconds.
        #[arg(long, default_value_t = 30.0)]
        seconds: f64,
        /// SID model: 6581 or 8580.
        #[arg(long, default_value = "6581")]
        chip: Chip,
        /// SID emulation: residfp (reSIDfp, sidplayfp's settings) or resid-rs.
        #[arg(long, default_value = "residfp")]
        engine: Engine,
        /// Voices to silence, for example `--mute 2,3` to hear voice 1 alone.
        #[arg(long, value_delimiter = ',', value_parser = clap::value_parser!(u8).range(1..=3))]
        mute: Vec<u8>,
        /// Output sample rate.
        #[arg(long, default_value_t = 48_000)]
        rate: u32,
        /// Output WAV (16-bit mono).
        #[arg(long)]
        wav: PathBuf,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli.command) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("score64-emu: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(command: Command) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        Command::Render {
            sid,
            subtune,
            seconds,
            chip,
            engine,
            mute,
            rate,
            wav,
            writes,
            opcodes,
            effect,
            effect_every,
            effect_voice,
        } => {
            let effect = if effect.is_empty() {
                None
            } else {
                Some(Effect {
                    data: effect.iter().map(std::fs::read).collect::<Result<_, _>>()?,
                    every: effect_every.max(1),
                    voice: effect_voice,
                })
            };
            render(
                &sid,
                subtune,
                seconds,
                chip,
                engine,
                mute_mask(&mute),
                rate,
                &wav,
                writes,
                opcodes,
                effect,
            )
        }
        Command::Replay {
            dump,
            seconds,
            chip,
            engine,
            mute,
            rate,
            wav,
        } => replay(&dump, seconds, chip, engine, mute_mask(&mute), rate, &wav),
    }
}

/// Voice numbers (1–3) as a bit mask (bit 0 = voice 1).
fn mute_mask(voices: &[u8]) -> u8 {
    voices.iter().fold(0, |m, v| m | 1 << (v - 1))
}

fn replay(
    dump: &Path,
    seconds: f64,
    chip: Chip,
    engine: Engine,
    muted: u8,
    rate: u32,
    wav: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let end = (seconds * f64::from(PAL_CLOCK_HZ)) as u64;
    let mut sid = SidClock::new(engine, chip, PAL_CLOCK_HZ, rate)?;
    sid.muted = muted;
    let mut cycle = 0u64;
    let mut count = 0usize;
    for (n, line) in std::fs::read_to_string(dump)?.lines().enumerate() {
        let fields: Vec<u64> = line
            .split_whitespace()
            .map(str::parse)
            .collect::<Result<_, _>>()
            .map_err(|e| format!("{}:{}: {e}", dump.display(), n + 1))?;
        let [delta, register, value] = fields[..] else {
            return Err(format!("{}:{}: expected three numbers", dump.display(), n + 1).into());
        };
        cycle += delta;
        if cycle >= end {
            break;
        }
        sid.write_at(cycle, register as u8, value as u8);
        count += 1;
    }
    sid.advance_to(end);
    write_wav(wav, rate, &sid.samples)?;
    println!("{}: {count} SID writes replayed", dump.display());
    Ok(())
}

/// Sound effects to trigger during a render, in rotation.
struct Effect {
    data: Vec<Vec<u8>>,
    every: u64,
    voice: u8,
}

/// Where the first `--effect` is loaded; each further one follows 128 bytes later (the format's maximum size).
const EFFECT_ADDRESS: u16 = 0xC000;
const EFFECT_STRIDE: u16 = 0x80;

#[allow(clippy::too_many_arguments)]
fn render(
    sid: &Path,
    subtune: Option<u16>,
    seconds: f64,
    chip: Chip,
    engine: Engine,
    muted: u8,
    rate: u32,
    wav: &Path,
    writes: Option<PathBuf>,
    opcodes: Option<PathBuf>,
    effect: Option<Effect>,
) -> Result<(), Box<dyn std::error::Error>> {
    let tune = Psid::parse(&std::fs::read(sid)?)?;
    let song = subtune.unwrap_or(tune.start_song.max(1));
    if song == 0 || song > tune.songs.max(1) {
        return Err(format!("subtune {song} out of range 1..={}", tune.songs).into());
    }

    let mut sid_clock = SidClock::new(engine, chip, PAL_CLOCK_HZ, rate)?;
    sid_clock.muted = muted;
    let mut memory = Memory::new(sid_clock);
    memory.load(tune.load_address, &tune.data);
    if let Some(e) = &effect {
        let tune_end = usize::from(tune.load_address) + tune.data.len();
        if tune_end > usize::from(EFFECT_ADDRESS) {
            return Err(format!(
                "tune ends at ${tune_end:04X}, over the effect data at ${EFFECT_ADDRESS:04X}"
            )
            .into());
        }
        for (n, data) in e.data.iter().enumerate() {
            if data.len() > usize::from(EFFECT_STRIDE) {
                return Err(
                    format!("effect {} is {} bytes; the limit is 128", n + 1, data.len()).into(),
                );
            }
            memory.load(EFFECT_ADDRESS + n as u16 * EFFECT_STRIDE, data);
        }
    }
    let cpu = Emulator6502::new(Registers::default());
    let mut m = Machine::new(cpu, memory);

    // Init gets up to one second; play is called at the start of each following frame.
    let init_cycles = m.call(
        tune.init_address,
        (song - 1) as u8,
        0,
        0,
        u64::from(PAL_CLOCK_HZ),
    )?;
    let first_play = m.cycle.div_ceil(PAL_CYCLES_PER_FRAME).max(1) * PAL_CYCLES_PER_FRAME;
    let frames = (seconds * f64::from(PAL_CLOCK_HZ) / PAL_CYCLES_PER_FRAME as f64).round() as u64;
    let mut play_cycles = Vec::with_capacity(frames as usize);
    let mut effects = 0u32;
    for k in 0..frames {
        m.idle_until(first_play + k * PAL_CYCLES_PER_FRAME);
        let mut cycles = 0;
        if let Some(e) = &effect
            && k > 0
            && k % e.every == 0
        {
            // LDA #<effect, LDY #>effect, LDX #channel (0, 7 or 14), JSR start+6, in the same frame before play.
            let which = (k / e.every - 1) as usize % e.data.len();
            let [lo, hi] = (EFFECT_ADDRESS + which as u16 * EFFECT_STRIDE).to_le_bytes();
            cycles += m.call(
                tune.init_address + 6,
                lo,
                (e.voice - 1) * 7,
                hi,
                PAL_CYCLES_PER_FRAME,
            )?;
            effects += 1;
        }
        cycles += m.call(tune.play_address, 0, 0, 0, PAL_CYCLES_PER_FRAME)?;
        play_cycles.push(cycles);
    }
    m.idle_until(first_play + frames * PAL_CYCLES_PER_FRAME);

    write_wav(wav, rate, &m.memory.sid.samples)?;
    if let Some(path) = writes {
        let mut out = BufWriter::new(File::create(path)?);
        writeln!(out, "frame\tcycle\tregister\tvalue")?;
        for w in &m.memory.writes {
            let (frame, cycle) = if w.cycle < first_play {
                ("init".to_string(), w.cycle)
            } else {
                let rel = w.cycle - first_play;
                (
                    (rel / PAL_CYCLES_PER_FRAME).to_string(),
                    rel % PAL_CYCLES_PER_FRAME,
                )
            };
            writeln!(out, "{frame}\t{cycle}\t{:02X}\t{:02X}", w.register, w.value)?;
        }
    }
    if let Some(path) = opcodes {
        let mut out = BufWriter::new(File::create(path)?);
        writeln!(out, "opcode\tdocumented\tcount")?;
        for (op, &n) in m.opcode_counts.iter().enumerate().filter(|(_, n)| **n > 0) {
            writeln!(out, "{op:02X}\t{}\t{n}", cpu::is_documented(op as u8))?;
        }
    }

    let max = play_cycles.iter().copied().max().unwrap_or(0);
    let mean = play_cycles.iter().sum::<u64>() as f64 / play_cycles.len().max(1) as f64;
    let undocumented: u64 = m
        .opcode_counts
        .iter()
        .enumerate()
        .filter(|(op, _)| !cpu::is_documented(*op as u8))
        .map(|(_, n)| n)
        .sum();
    println!(
        "{}: subtune {song}/{}, {frames} frames, init {init_cycles} cycles, play mean {mean:.0} / max {max} cycles \
         ({:.1} raster lines), {effects} effects triggered, {} SID writes, {undocumented} undocumented opcodes run, \
         {} writes to $00/$01",
        tune.name,
        tune.songs,
        max as f64 / 63.0,
        m.memory.writes.len(),
        m.memory.port_writes,
    );
    Ok(())
}

fn write_wav(path: &Path, rate: u32, samples: &[i16]) -> std::io::Result<()> {
    let mut out = BufWriter::new(File::create(path)?);
    let data_len = u32::try_from(samples.len() * 2).unwrap_or(u32::MAX);
    out.write_all(b"RIFF")?;
    out.write_all(&(36 + data_len).to_le_bytes())?;
    out.write_all(b"WAVEfmt ")?;
    out.write_all(&16u32.to_le_bytes())?;
    out.write_all(&1u16.to_le_bytes())?; // PCM
    out.write_all(&1u16.to_le_bytes())?; // mono
    out.write_all(&rate.to_le_bytes())?;
    out.write_all(&(rate * 2).to_le_bytes())?;
    out.write_all(&2u16.to_le_bytes())?;
    out.write_all(&16u16.to_le_bytes())?;
    out.write_all(b"data")?;
    out.write_all(&data_len.to_le_bytes())?;
    for s in samples {
        out.write_all(&s.to_le_bytes())?;
    }
    out.flush()
}
