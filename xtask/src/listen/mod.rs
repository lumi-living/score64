//! `cargo xtask listen`: a listening set for a PR that changes what the packed player plays (`needs-vice-check`).
//!
//! For each case (a built-in score, or a score file with an instrument folder) it:
//! - exports the score with `score64_formats::gt2::export`;
//! - packs it with the pinned gt2reloc, with sound-effect support (`.sfx`, `-D1`) and without (`.nosfx`);
//! - wraps the `.sfx` file so it fires the fixture's three effects on voice 3 every 37 frames (`.sfx+fx`);
//! - renders every subtune of all three on the 6581 and the 8580, with the emulator helper and with sidplayfp;
//! - measures what can be measured, and writes `LISTEN.md`: the tools, what to listen for, each case's timeline and
//!   the measurements.
//!
//! The output goes to `scratch/<pr>-listen/`, which git ignores.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use score64_adapters::gt2reloc::{Gt2Reloc, PackOptions, ZeroPage};
use score64_core::Song;
use score64_formats::gt2::Ins;
use score64_formats::gt2::export::{Header, Library, Options, export_with};

mod fixture;
mod measure;
mod render;
mod report;
mod wrap;

use render::CHIPS;

/// `cargo xtask listen` arguments.
#[derive(clap::Args)]
pub struct Args {
    /// The PR the set is for: output goes to `scratch/<pr>-listen/`.
    #[arg(long, required_unless_present = "out")]
    pr: Option<u32>,
    /// Output folder instead of `scratch/<pr>-listen/`.
    #[arg(long)]
    out: Option<PathBuf>,
    /// Built-in cases to render (default: all of them): sixteen-steps, sixteen-steps-adaptive, combat-filtered,
    /// ties-keyoffs-arp, loop-tempo.
    cases: Vec<String>,
    /// A score file (`Song` as TOML or JSON) to render as well.
    #[arg(long, requires = "instruments")]
    score: Option<PathBuf>,
    /// The score's instruments: a folder of `<preset id>.ins` files (GoatTracker GTI5).
    #[arg(long)]
    instruments: Option<PathBuf>,
    /// Seconds per render.
    #[arg(long, default_value_t = 60.0)]
    seconds: f64,
    /// The gt2reloc binary's blake3 to pin. Without it, the installed gt2reloc's hash is pinned and recorded.
    #[arg(long)]
    gt2reloc_blake3: Option<String>,
    /// The version gt2reloc's banner must name.
    #[arg(long, default_value = "2.73")]
    gt2reloc_version: String,
    /// Export with `silence_after_release` on for every preset (#186), add `+silence` to each case's name and write
    /// `LISTEN-silence.md`, so a second run into the same folder sits beside the first.
    #[arg(long)]
    silence_after_release: bool,
}

/// The repository root (xtask sits in `<root>/xtask`).
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask is inside the workspace")
        .to_path_buf()
}

/// The emulator helper: `SCORE64_EMU`, or a release build (it renders far faster than a debug one).
fn helper() -> Result<PathBuf, String> {
    if let Some(p) = std::env::var_os("SCORE64_EMU").filter(|p| !p.is_empty()) {
        return Ok(PathBuf::from(p));
    }
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let status = std::process::Command::new(cargo)
        .args(["build", "--release", "-q", "-p", "score64-emu"])
        .current_dir(root())
        .status()
        .map_err(|e| format!("building score64-emu: {e}"))?;
    if !status.success() {
        return Err("building score64-emu failed".into());
    }
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .filter(|p| !p.is_empty())
        // Relative to the workspace root, as cargo reads it.
        .map_or_else(|| root().join("target"), |p| root().join(p));
    let path = target
        .join("release")
        .join(format!("score64-emu{}", std::env::consts::EXE_SUFFIX));
    path.is_file()
        .then_some(path)
        .ok_or_else(|| "built score64-emu but can't find it; set SCORE64_EMU".into())
}

/// A score file's `Song`, by extension.
fn read_song(path: &Path) -> Result<Song, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("json") => serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display())),
        _ => toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display())),
    }
}

/// Every `<preset id>.ins` in `dir`.
fn read_library(dir: &Path) -> Result<Library, String> {
    let mut lib = Library::new();
    for entry in std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.extension().and_then(|e| e.to_str()) == Some("ins") {
            let ins = Ins::read(&std::fs::read(&path).map_err(|e| e.to_string())?)
                .map_err(|e| format!("{}: {e}", path.display()))?;
            let id = path
                .file_stem()
                .expect("a file name")
                .to_string_lossy()
                .into_owned();
            lib.insert(score64_core::PresetId(id), ins);
        }
    }
    Ok(lib)
}

/// One case's packed files and measurements, for the report.
struct Done {
    name: String,
    song: Song,
    notes: String,
    packs: Vec<(String, u32)>,
    timing_marks: bool,
    measured: Vec<String>,
}

/// Build the listening set.
pub fn run(args: Args) -> Result<(), String> {
    let out = match (&args.out, args.pr) {
        (Some(o), _) => o.clone(),
        (None, Some(pr)) => root().join("scratch").join(format!("{pr}-listen")),
        (None, None) => unreachable!("clap requires one"),
    };
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;

    // The cases: the named built-ins (all by default), plus a score file.
    let names: Vec<String> = if args.cases.is_empty() && args.score.is_none() {
        fixture::NAMES.iter().map(|s| s.to_string()).collect()
    } else {
        args.cases.clone()
    };
    let mut cases: Vec<(
        String,
        Song,
        Vec<score64_core::AdaptiveSet>,
        Library,
        String,
    )> = Vec::new();
    for n in &names {
        let c = fixture::case(n).ok_or_else(|| {
            format!(
                "no built-in case `{n}` (have: {})",
                fixture::NAMES.join(", ")
            )
        })?;
        cases.push((
            c.name.to_string(),
            c.song,
            c.adaptive,
            fixture::library(),
            c.notes.to_string(),
        ));
    }
    if let (Some(score), Some(dir)) = (&args.score, &args.instruments) {
        let stem = score
            .file_stem()
            .expect("a file name")
            .to_string_lossy()
            .into_owned();
        cases.push((
            stem,
            read_song(score)?,
            vec![],
            read_library(dir)?,
            format!("From `{}`.", score.display()),
        ));
    }

    let helper = helper()?;
    let sidplayfp = render::find_sidplayfp();
    let found = Gt2Reloc::locate(&args.gt2reloc_version).map_err(|e| e.to_string())?;
    let installed = found.program_blake3().map_err(|e| e.to_string())?;
    let pin = args
        .gt2reloc_blake3
        .clone()
        .unwrap_or_else(|| installed.clone());
    let packer = found.with_blake3(&pin).map_err(|e| e.to_string())?;
    let fx_dir = root().join("fixtures/gt2-original");
    let effects: Vec<Vec<u8>> = ["footstep", "gunshot", "clink"]
        .iter()
        .map(|k| {
            std::fs::read(fx_dir.join(format!("sfx-{k}.bin")))
                .map_err(|e| format!("sfx-{k}.bin: {e}"))
        })
        .collect::<Result<_, _>>()?;

    let mut done = Vec::new();
    for (name, song, adaptive, library, notes) in cases {
        let (name, options) = if args.silence_after_release {
            let all = library.keys().cloned().collect();
            (
                format!("{name}+silence"),
                Options {
                    silence_after_release: all,
                },
            )
        } else {
            (name, Options::default())
        };
        eprintln!("{name}: exporting, packing, rendering");
        let header = Header {
            name: name.clone(),
            author: "Score64 export".into(),
            copyright: "listening test".into(),
        };
        let exported = export_with(&song, &library, &adaptive, &header, &options)
            .map_err(|e| format!("{name}: {e}"))?;
        let sng = out.join(format!("{name}.sng"));
        std::fs::write(
            &sng,
            exported.sng.write().map_err(|e| format!("{name}: {e}"))?,
        )
        .map_err(|e| e.to_string())?;
        let mut packs = Vec::new();
        for sfx in [true, false] {
            let tag = if sfx { "sfx" } else { "nosfx" };
            let mut options = PackOptions::new(0x10, ZeroPage::new(0xF8).expect("valid"));
            options.sound_effects = sfx;
            options.timing_marks = exported.timing_marks;
            let packed = packer
                .pack(&sng, &out.join(format!("{name}.{tag}.sid")), &options)
                .map_err(|e| format!("{name} {tag}: {e}"))?;
            packs.push((format!("{name}.{tag}.sid"), packed.size));
        }
        let plain =
            std::fs::read(out.join(format!("{name}.sfx.sid"))).map_err(|e| e.to_string())?;
        std::fs::write(
            out.join(format!("{name}.sfx+fx.sid")),
            wrap::wrap(&plain, &effects)?,
        )
        .map_err(|e| e.to_string())?;

        let mut measured = Vec::new();
        for (k, sub) in song.subtunes.iter().enumerate() {
            let k = k as u8;
            let base = format!("{name}-{}", sub.name.as_str());
            for (chip, model) in CHIPS {
                let mut helper_wavs = Vec::new();
                for tag in ["sfx", "nosfx", "sfx+fx"] {
                    let sid = out.join(format!("{name}.{tag}.sid"));
                    let samples = render::helper(&helper, &sid, k, chip, args.seconds)?;
                    render::write_wav(
                        &out.join(format!("{base}.{tag}.helper-{model}.wav")),
                        &samples,
                    )?;
                    helper_wavs.push(samples);
                    if let Some(sp) = &sidplayfp {
                        render::sidplayfp(
                            sp,
                            &sid,
                            k,
                            chip,
                            args.seconds,
                            &[],
                            &out.join(format!("{base}.{tag}.sidplayfp-{model}.wav")),
                        )?;
                    }
                }
                if let Some((r, _)) = measure::residual(&helper_wavs[0], &helper_wavs[1]) {
                    measured.push(format!("| `{base}` | {model} | `.sfx` vs `.nosfx` (helper) | {:.1} % | should sound the same; `-D1` buffers the SID writes, so some difference at sample level is expected |", 100.0 * r));
                }
                if let Some(sp) = &sidplayfp {
                    let theirs =
                        render::read_wav(&out.join(format!("{base}.sfx.sidplayfp-{model}.wav")))?;
                    if let Some((r, lag)) = measure::residual(&helper_wavs[0], &theirs) {
                        measured.push(format!("| `{base}` | {model} | helper vs sidplayfp (`.sfx`) | {:.1} % at {lag:.0} ms | close: both are reSIDfp |", 100.0 * r));
                    }
                    // The effect margin, from muted 20-second sidplayfp renders that aren't kept.
                    let tmp = out.join(".measure");
                    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
                    let t = |n: &str| tmp.join(format!("{n}.wav"));
                    render::sidplayfp(
                        sp,
                        &out.join(format!("{name}.sfx.sid")),
                        k,
                        chip,
                        20.0,
                        &[3],
                        &t("music"),
                    )?;
                    render::sidplayfp(
                        sp,
                        &out.join(format!("{name}.sfx+fx.sid")),
                        k,
                        chip,
                        20.0,
                        &[1, 2],
                        &t("v3fx"),
                    )?;
                    render::sidplayfp(
                        sp,
                        &out.join(format!("{name}.sfx.sid")),
                        k,
                        chip,
                        20.0,
                        &[1, 2],
                        &t("v3"),
                    )?;
                    if let Some((p90, fx)) = measure::effect_margin(
                        &render::read_wav(&t("music"))?,
                        &render::read_wav(&t("v3fx"))?,
                        &render::read_wav(&t("v3"))?,
                    ) {
                        measured.push(format!("| `{base}` | {model} | effects vs loud music (sidplayfp) | {:+.1} dB | effects {fx:.1} dBFS, music p90 {p90:.1} dBFS |", fx - p90));
                    }
                    let _ = std::fs::remove_dir_all(&tmp);
                }
            }
        }
        done.push(Done {
            name,
            song,
            notes,
            packs,
            timing_marks: exported.timing_marks,
            measured,
        });
    }

    let command = std::iter::once("cargo xtask listen".to_string())
        .chain(std::env::args().skip(2))
        .collect::<Vec<_>>()
        .join(" ");
    let md = listen_md(
        &done,
        &pin,
        &installed,
        sidplayfp.is_some(),
        args.seconds,
        args.pr,
        &command,
    );
    // A `+silence` run writes its own report, so it can sit beside a plain run's.
    let report = if args.silence_after_release {
        "LISTEN-silence.md"
    } else {
        "LISTEN.md"
    };
    std::fs::write(out.join(report), md).map_err(|e| e.to_string())?;
    eprintln!("wrote {}", out.join(report).display());
    Ok(())
}

fn listen_md(
    done: &[Done],
    pin: &str,
    installed: &str,
    sidplayfp: bool,
    seconds: f64,
    pr: Option<u32>,
    command: &str,
) -> String {
    let mut s = String::new();
    let title = pr.map_or("Listening set".to_string(), |p| {
        format!("Listening set for #{p}")
    });
    let _ = writeln!(
        s,
        "# {title}\n\nMade by `cargo xtask listen`. Each score was exported by `score64_formats::gt2::export`, packed by the pinned `gt2reloc`, and rendered for {seconds:.0} s.\n"
    );
    let _ = writeln!(s, "**Tools**");
    let _ = writeln!(
        s,
        "- **gt2reloc:** pinned blake3 `{pin}`{}.",
        if pin == installed {
            " (the installed binary)"
        } else {
            ""
        }
    );
    let _ = writeln!(
        s,
        "- **Pack options:** player at `$1000`, zero page `$F8`, PAL, hard-restart ADSR `$0F00`; `-H1` where a song has timing marks."
    );
    let _ = writeln!(
        s,
        "- **Renders:** 48 kHz mono, 16-bit, PAL. `helper` is Score64's emulator helper (reSIDfp). `sidplayfp` is sidplayfp with the chip forced and a single track{}.",
        if sidplayfp {
            ""
        } else {
            ". **sidplayfp isn't installed, so those renders and the measurements that need it are missing**"
        }
    );
    let _ = writeln!(
        s,
        "\n**Files.** `<case>.sng`, and three packs per case:\n- `.sfx.sid`: with sound-effect support (`-D1`). **No effect plays:** it only means the player *can* play them.\n- `.nosfx.sid`: without (`-D0`).\n- `.sfx+fx.sid`: the `.sfx` file wrapped so it fires the fixture's footstep, gunshot and clink in turn on **voice 3, every 37 frames (0.74 s)**, as a game would.\n\nWAVs are `<case>-<subtune>.<sfx|nosfx|sfx+fx>.<helper|sidplayfp>-<6581|8580>.wav`.\n"
    );
    let _ = writeln!(
        s,
        "**What to listen for, everywhere**\n1. **Ties:** a tied note changes pitch with no new attack.\n2. **Key-offs** release cleanly, with no click and nothing left hanging.\n3. **Tempo** is steady; a tempo change takes effect from its row.\n4. **The loop** comes round without a click, gap, stuck note or tempo jump.\n5. **Arpeggios** step evenly, one step a frame, and stop cleanly.\n6. **Effects (`+fx`)** stand out over the music; voices 1 and 2 carry on unchanged; voice 3 gives the music back at its next note.\n\n**Should sound the same:** `.sfx` and `.nosfx`; `helper` and `sidplayfp` at one chip. **Should differ:** the 6581 and the 8580, in filter character, but not in notes, timing or envelopes.\n\n**Known, not a fault:** the gaps between notes aren't fully silent. After a key-off the oscillator keeps running and leaks a faint tone at the note's pitch, most clearly on the 8580 (#186).\n"
    );
    for d in done {
        let _ = writeln!(s, "---\n\n## `{}`\n\n{}\n", d.name, d.notes);
        let packs: Vec<String> = d
            .packs
            .iter()
            .map(|(f, n)| format!("`{f}` ({n} bytes)"))
            .collect();
        let _ = writeln!(
            s,
            "Packed: {}{}.\n",
            packs.join(", "),
            if d.timing_marks {
                ", with timing marks (`-H1`)"
            } else {
                ""
            }
        );
        for sub in &d.song.subtunes {
            let _ = writeln!(
                s,
                "### `{}`\n\n{}",
                sub.name.as_str(),
                report::timeline(sub)
            );
        }
        if !d.measured.is_empty() {
            let _ = writeln!(
                s,
                "**Measured** (seconds 5–15 for the comparisons, 20 ms windows for the effects):\n\n| Subtune | Chip | Pair | Result | Reading |\n|---|---|---|---|---|"
            );
            for m in &d.measured {
                let _ = writeln!(s, "{m}");
            }
            let _ = writeln!(s);
        }
    }
    let _ = writeln!(
        s,
        "---\n\nMade with `{command}`. Render another score with `--score <song.toml> --instruments <folder of <preset id>.ins>`."
    );
    s
}
