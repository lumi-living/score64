//! Rendering a PSID to WAV with the emulator helper (a separate GPL program, ADR-002) and with sidplayfp.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use score64_adapters::emu::{Chip, Helper};
use score64_formats::psid::Psid;

/// The output sample rate.
pub const SAMPLE_RATE: u32 = 48_000;
/// PAL frames per second.
pub const PAL_HZ: f64 = 50.1246;

/// The two chip models, with the names used in file names.
pub const CHIPS: [(Chip, &str); 2] = [(Chip::Mos6581, "6581"), (Chip::Mos8580, "8580")];

/// `seconds` of subtune `subtune` (0-based) of `sid` through the helper at `helper`, as mono 16-bit samples.
pub fn helper(
    helper: &Path,
    sid: &Path,
    subtune: u8,
    chip: Chip,
    seconds: f64,
) -> Result<Vec<i16>, String> {
    let psid = Psid::parse(&std::fs::read(sid).map_err(|e| format!("{}: {e}", sid.display()))?)
        .map_err(|e| format!("{}: {e:?}", sid.display()))?;
    let mut h = Helper::spawn(helper, SAMPLE_RATE).map_err(|e| e.to_string())?;
    h.model(chip).map_err(|e| e.to_string())?;
    let (init, play) = h.load_psid(&psid).map_err(|e| e.to_string())?;
    h.init(init, play, subtune).map_err(|e| e.to_string())?;
    let mut left = (seconds * PAL_HZ).round() as u32;
    let mut samples = Vec::new();
    while left > 0 {
        let n = left.min(500);
        samples.extend(h.render(n, false).map_err(|e| e.to_string())?.samples);
        left -= n;
    }
    Ok(samples)
}

/// sidplayfp on PATH, if it is installed.
pub fn find_sidplayfp() -> Option<PathBuf> {
    let name = format!("sidplayfp{}", std::env::consts::EXE_SUFFIX);
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|d| d.join(&name))
        .find(|p| p.is_file())
}

/// `seconds` of subtune `subtune` (0-based) of `sid` through sidplayfp into `wav`: mono, PAL, the chip forced, one
/// track only, and `mute` voices (1-3) silenced.
pub fn sidplayfp(
    program: &Path,
    sid: &Path,
    subtune: u8,
    chip: Chip,
    seconds: f64,
    mute: &[u8],
    wav: &Path,
) -> Result<(), String> {
    let model = match chip {
        Chip::Mos6581 => "-mof",
        Chip::Mos8580 => "-mnf",
    };
    let mut cmd = Command::new(program);
    cmd.args([
        "-q",
        &format!("-t{seconds}"),
        "-m",
        model,
        "-vpf",
        "-os",
        &format!("-o{}", subtune + 1),
    ])
    .args(mute.iter().map(|v| format!("-u{v}")))
    .arg(format!("-w{}", wav.display()))
    .arg(sid)
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null());
    let status = cmd.status().map_err(|e| format!("sidplayfp: {e}"))?;
    if !status.success() || !wav.is_file() {
        return Err(format!("sidplayfp failed on {} ({status})", sid.display()));
    }
    Ok(())
}

/// Write mono 16-bit samples as a WAV file.
pub fn write_wav(path: &Path, samples: &[i16]) -> Result<(), String> {
    let io = |e: std::io::Error| format!("{}: {e}", path.display());
    let mut f = std::io::BufWriter::new(std::fs::File::create(path).map_err(io)?);
    let data = (samples.len() * 2) as u32;
    let mut header = Vec::with_capacity(44);
    header.extend_from_slice(b"RIFF");
    header.extend_from_slice(&(36 + data).to_le_bytes());
    header.extend_from_slice(b"WAVEfmt ");
    header.extend_from_slice(&16u32.to_le_bytes());
    header.extend_from_slice(&1u16.to_le_bytes()); // PCM
    header.extend_from_slice(&1u16.to_le_bytes()); // mono
    header.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    header.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
    header.extend_from_slice(&2u16.to_le_bytes());
    header.extend_from_slice(&16u16.to_le_bytes());
    header.extend_from_slice(b"data");
    header.extend_from_slice(&data.to_le_bytes());
    f.write_all(&header).map_err(io)?;
    for s in samples {
        f.write_all(&s.to_le_bytes()).map_err(io)?;
    }
    f.flush().map_err(io)
}

/// A mono 16-bit WAV file's samples (any header layout with a `data` chunk; the first channel if not mono).
pub fn read_wav(path: &Path) -> Result<Vec<i16>, String> {
    let b = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let bad = || format!("{}: not a 16-bit WAV file", path.display());
    if b.get(..4) != Some(b"RIFF") || b.get(8..12) != Some(b"WAVE") {
        return Err(bad());
    }
    let (mut at, mut channels) = (12, 1usize);
    while at + 8 <= b.len() {
        let id = &b[at..at + 4];
        let len = u32::from_le_bytes(b[at + 4..at + 8].try_into().expect("4 bytes")) as usize;
        let body = b.get(at + 8..(at + 8 + len).min(b.len())).ok_or_else(bad)?;
        match id {
            b"fmt " => {
                channels = usize::from(u16::from_le_bytes([body[2], body[3]])).max(1);
                if u16::from_le_bytes([body[14], body[15]]) != 16 {
                    return Err(bad());
                }
            }
            b"data" => {
                return Ok(body
                    .chunks_exact(2 * channels)
                    .map(|c| i16::from_le_bytes([c[0], c[1]]))
                    .collect());
            }
            _ => {}
        }
        at += 8 + len + (len & 1);
    }
    Err(bad())
}
