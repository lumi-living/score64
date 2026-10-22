//! The optional Python sidecar (ADR-009): run a script from the `sidecar/` uv project and check what it wrote.
//!
//! Contract: input paths as arguments, files plus a `manifest.json` (schema 1, `sidecar/manifest.py`) in the output
//! directory. The runner:
//! - reports a missing `uv` or sidecar project as [`Unavailable`], not as a failure, so callers can skip visibly;
//! - starts `uv run --frozen` with an argument list (never a shell), with a timeout, and kills it and every process it
//!   started (the Python script included) if it overruns or the runner is dropped while it runs;
//! - keeps stderr for the error message;
//! - validates the manifest (schema, tool, and every output's size and blake3 hash) before anything uses an output.
//!   blake3 is also the content-addressed cache's hash (ADR-004).

use std::ffi::OsStr;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// The manifest schema this runner understands.
pub const MANIFEST_SCHEMA: u64 = 1;

/// Why the sidecar cannot run here. Not an error: the default pipeline never needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unavailable(pub String);

impl std::fmt::Display for Unavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "sidecar unavailable: {}", self.0)
    }
}

impl std::error::Error for Unavailable {}

/// Why a sidecar run failed.
#[derive(Debug)]
pub enum SidecarError {
    /// The process could not be started.
    Spawn(std::io::Error),
    /// The script exited with an error.
    Failed {
        /// Exit status.
        status: ExitStatus,
        /// The end of its stderr.
        stderr: String,
    },
    /// The script ran longer than allowed and was killed.
    Timeout {
        /// The limit.
        after: Duration,
        /// The end of its stderr.
        stderr: String,
    },
    /// The manifest is missing, malformed or of another schema or tool.
    BadManifest(String),
    /// An output does not match its manifest entry.
    OutputMismatch {
        /// The output's path in the manifest.
        path: String,
        /// What differs.
        why: String,
    },
}

impl std::fmt::Display for SidecarError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SidecarError::Spawn(e) => write!(f, "could not start the sidecar: {e}"),
            SidecarError::Failed { status, stderr } => {
                write!(f, "sidecar failed ({status}):\n{stderr}")
            }
            SidecarError::Timeout { after, stderr } => {
                write!(
                    f,
                    "sidecar killed after {:.0} s:\n{stderr}",
                    after.as_secs_f64()
                )
            }
            SidecarError::BadManifest(why) => write!(f, "bad sidecar manifest: {why}"),
            SidecarError::OutputMismatch { path, why } => write!(f, "sidecar output {path}: {why}"),
        }
    }
}

impl std::error::Error for SidecarError {}

/// One file in a manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    /// Path as written in the manifest (outputs: relative to the output directory).
    pub path: String,
    /// Size in bytes.
    pub bytes: u64,
    /// blake3, lowercase hex.
    pub blake3: String,
}

/// A validated `manifest.json`.
#[derive(Debug, Clone)]
pub struct Manifest {
    /// Directory the outputs are in.
    pub dir: PathBuf,
    /// Script that wrote it.
    pub tool: String,
    /// The script's own version.
    pub tool_version: u64,
    /// Python version.
    pub python: String,
    /// Package versions the script reported, in its order.
    pub packages: Vec<(String, String)>,
    /// Script parameters, as written.
    pub params: serde_json::Value,
    /// Inputs with their hashes.
    pub inputs: Vec<FileEntry>,
    /// Outputs with their hashes, all checked.
    pub outputs: Vec<FileEntry>,
}

impl Manifest {
    /// Read `dir/manifest.json`, check schema and tool, and check every output against its size and hash.
    pub fn load(dir: &Path, tool: &str) -> Result<Self, SidecarError> {
        let bad = |why: String| SidecarError::BadManifest(why);
        let text = std::fs::read_to_string(dir.join("manifest.json"))
            .map_err(|e| bad(format!("manifest.json: {e}")))?;
        let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| bad(e.to_string()))?;
        let schema = v["schema"]
            .as_u64()
            .ok_or_else(|| bad("no schema".into()))?;
        if schema != MANIFEST_SCHEMA {
            return Err(bad(format!("schema {schema}, expected {MANIFEST_SCHEMA}")));
        }
        let field = |name: &str| {
            v[name]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| bad(format!("no {name}")))
        };
        let got_tool = field("tool")?;
        if got_tool != tool {
            return Err(bad(format!("written by {got_tool:?}, expected {tool:?}")));
        }
        let files = |name: &str| -> Result<Vec<FileEntry>, SidecarError> {
            v[name]
                .as_array()
                .ok_or_else(|| bad(format!("no {name}")))?
                .iter()
                .map(|e| {
                    Ok(FileEntry {
                        path: e["path"]
                            .as_str()
                            .ok_or_else(|| bad(format!("{name}: entry without path")))?
                            .to_owned(),
                        bytes: e["bytes"]
                            .as_u64()
                            .ok_or_else(|| bad(format!("{name}: entry without bytes")))?,
                        blake3: e["blake3"]
                            .as_str()
                            .ok_or_else(|| bad(format!("{name}: entry without blake3")))?
                            .to_owned(),
                    })
                })
                .collect()
        };
        let packages = v["packages"]
            .as_object()
            .ok_or_else(|| bad("no packages".into()))?
            .iter()
            .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_owned()))
            .collect();
        let manifest = Manifest {
            dir: dir.to_path_buf(),
            tool: got_tool,
            tool_version: v["tool_version"]
                .as_u64()
                .ok_or_else(|| bad("no tool_version".into()))?,
            python: field("python")?,
            packages,
            params: v["params"].clone(),
            inputs: files("inputs")?,
            outputs: files("outputs")?,
        };
        for out in &manifest.outputs {
            let path =
                manifest
                    .output_path(&out.path)
                    .ok_or_else(|| SidecarError::OutputMismatch {
                        path: out.path.clone(),
                        why: "path leaves the output directory".into(),
                    })?;
            let data = std::fs::read(&path).map_err(|e| SidecarError::OutputMismatch {
                path: out.path.clone(),
                why: e.to_string(),
            })?;
            if data.len() as u64 != out.bytes {
                return Err(SidecarError::OutputMismatch {
                    path: out.path.clone(),
                    why: format!("{} bytes, manifest says {}", data.len(), out.bytes),
                });
            }
            let hash = blake3::hash(&data).to_hex().to_string();
            if hash != out.blake3 {
                return Err(SidecarError::OutputMismatch {
                    path: out.path.clone(),
                    why: format!("blake3 {hash}, manifest says {}", out.blake3),
                });
            }
        }
        Ok(manifest)
    }

    /// Path of a listed output, if the manifest lists it.
    pub fn output(&self, name: &str) -> Option<PathBuf> {
        self.outputs
            .iter()
            .find(|o| o.path == name)
            .and_then(|o| self.output_path(&o.path))
    }

    fn output_path(&self, rel: &str) -> Option<PathBuf> {
        let p = Path::new(rel);
        (p.is_relative()
            && p.components()
                .all(|c| matches!(c, std::path::Component::Normal(_))))
        .then(|| self.dir.join(p))
    }
}

/// A located sidecar: `uv` plus the `sidecar/` project.
#[derive(Debug, Clone)]
pub struct Sidecar {
    uv: PathBuf,
    project: PathBuf,
    timeout: Duration,
}

impl Sidecar {
    /// Find `uv` (`SCORE64_UV`, else on `PATH`) and check that `project` is the sidecar's uv project with a lock file.
    pub fn locate(project: &Path) -> Result<Self, Unavailable> {
        for f in ["pyproject.toml", "uv.lock"] {
            if !project.join(f).is_file() {
                return Err(Unavailable(format!(
                    "{} not found",
                    project.join(f).display()
                )));
            }
        }
        let uv = match std::env::var_os("SCORE64_UV") {
            Some(p) => PathBuf::from(p),
            None => find_on_path("uv")
                .ok_or_else(|| Unavailable("uv is not on PATH (see docs/TOOLING.md)".into()))?,
        };
        Ok(Self {
            uv,
            project: project.to_path_buf(),
            timeout: Duration::from_secs(30 * 60),
        })
    }

    /// Change the time limit for one run (default 30 minutes, which allows a first `uv` environment download).
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Run `script` (a file in the sidecar project) with `args`, then load and validate the manifest it wrote into
    /// `out_dir`, which must be among the arguments where the script expects it.
    pub fn run(
        &self,
        script: &str,
        args: &[&OsStr],
        out_dir: &Path,
        tool: &str,
    ) -> Result<Manifest, SidecarError> {
        let mut cmd = Command::new(&self.uv);
        cmd.args(["run", "--frozen", "--quiet", "--project"])
            .arg(&self.project)
            .arg("python")
            .arg(self.project.join(script))
            .args(args)
            .stdin(Stdio::null());
        run_with_timeout(cmd, self.timeout)?;
        Manifest::load(out_dir, tool)
    }
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let exts: &[&str] = if cfg!(windows) { &[".exe", ""] } else { &[""] };
    std::env::split_paths(&std::env::var_os("PATH")?)
        .flat_map(|dir| exts.iter().map(move |ext| dir.join(format!("{name}{ext}"))))
        .find(|p| p.is_file())
}

/// Kills the child and everything it started if it is still running when dropped (timeout, panic, or the runner
/// going away).
struct Guard(Child);

impl Drop for Guard {
    fn drop(&mut self) {
        if let Ok(None) = self.0.try_wait() {
            kill_tree(&mut self.0);
        }
    }
}

/// Start `cmd` so that [`kill_tree`] can end it and every process it starts: on Unix in a process group of its own.
fn spawn_tree(cmd: &mut Command) -> std::io::Result<Child> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    cmd.spawn()
}

/// Kill `child` and its descendants, then reap it. `uv` does not pass a kill on to the Python it starts, so killing
/// only `uv` would leave a hung script running (#126). Unix: `kill -9 -<pgid>` on the group [`spawn_tree`] made.
/// Windows: `taskkill /T /F`, which follows the process tree. Both are standard system commands; `Child::kill`
/// stays as a fallback if they fail.
fn kill_tree(child: &mut Child) {
    let pid = child.id().to_string();
    #[cfg(unix)]
    let mut tree = {
        let mut c = Command::new("kill");
        c.args(["-9", &format!("-{pid}")]);
        c
    };
    #[cfg(windows)]
    let mut tree = {
        let mut c = Command::new("taskkill");
        c.args(["/PID", &pid, "/T", "/F"]);
        c
    };
    let _ = tree.stdout(Stdio::null()).stderr(Stdio::null()).status();
    let _ = child.kill();
    let _ = child.wait();
}

const STDERR_TAIL: usize = 4000;

fn tail(bytes: &[u8]) -> String {
    let s = String::from_utf8_lossy(bytes);
    let start = s.len().saturating_sub(STDERR_TAIL);
    let start = (start..s.len())
        .find(|&i| s.is_char_boundary(i))
        .unwrap_or(s.len());
    s[start..].trim().to_owned()
}

/// Run `cmd` (stdout discarded, stderr captured) for at most `timeout`.
pub(crate) fn run_with_timeout(mut cmd: Command, timeout: Duration) -> Result<(), SidecarError> {
    cmd.stdout(Stdio::null()).stderr(Stdio::piped());
    let mut guard = Guard(spawn_tree(&mut cmd).map_err(SidecarError::Spawn)?);
    // Drain stderr on a thread so a chatty process can't block on a full pipe.
    let mut pipe = guard.0.stderr.take().expect("stderr is piped");
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = pipe.read_to_end(&mut buf);
        buf
    });
    let started = Instant::now();
    let status = loop {
        if let Some(status) = guard.0.try_wait().map_err(SidecarError::Spawn)? {
            break Some(status);
        }
        if started.elapsed() >= timeout {
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    match status {
        Some(status) => {
            let stderr = tail(&reader.join().unwrap_or_default());
            if status.success() {
                Ok(())
            } else {
                Err(SidecarError::Failed { status, stderr })
            }
        }
        None => {
            drop(guard); // kill and reap, which also closes the pipe
            Err(SidecarError::Timeout {
                after: timeout,
                stderr: tail(&reader.join().unwrap_or_default()),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("score64-sidecar-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write_manifest(
        dir: &Path,
        schema: u64,
        tool: &str,
        file: &str,
        data: &[u8],
        hash_of: &[u8],
    ) {
        std::fs::write(dir.join(file), data).unwrap();
        let m = serde_json::json!({
            "schema": schema, "tool": tool, "tool_version": 1, "python": "3.11.15",
            "packages": {"numpy": "2.4.6"}, "params": {},
            "inputs": [],
            "outputs": [{"path": file, "bytes": data.len(), "blake3": blake3::hash(hash_of).to_hex().to_string()}],
        });
        std::fs::write(dir.join("manifest.json"), m.to_string()).unwrap();
    }

    #[test]
    fn manifest_checks_hashes_schema_and_tool() {
        let d = temp_dir("manifest");
        write_manifest(&d, 1, "t", "a.bin", b"abc", b"abc");
        let m = Manifest::load(&d, "t").unwrap();
        assert_eq!(m.output("a.bin"), Some(d.join("a.bin")));
        assert!(matches!(
            Manifest::load(&d, "other"),
            Err(SidecarError::BadManifest(_))
        ));
        write_manifest(&d, 2, "t", "a.bin", b"abc", b"abc");
        assert!(matches!(
            Manifest::load(&d, "t"),
            Err(SidecarError::BadManifest(_))
        ));
        write_manifest(&d, 1, "t", "a.bin", b"abd", b"abc");
        assert!(matches!(
            Manifest::load(&d, "t"),
            Err(SidecarError::OutputMismatch { .. })
        ));
        write_manifest(&d, 1, "t", "../escape", b"x", b"x");
        assert!(matches!(
            Manifest::load(&d, "t"),
            Err(SidecarError::OutputMismatch { .. })
        ));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn missing_project_is_unavailable_not_an_error() {
        let d = temp_dir("noproject");
        assert!(Sidecar::locate(&d).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    fn rustc() -> Command {
        Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
    }

    #[test]
    fn failure_keeps_stderr() {
        let mut cmd = rustc();
        cmd.arg("--no-such-flag");
        match run_with_timeout(cmd, Duration::from_secs(60)) {
            Err(SidecarError::Failed { stderr, .. }) => {
                assert!(stderr.contains("no-such-flag"), "{stderr}")
            }
            other => panic!("expected a failure, got {other:?}"),
        }
    }

    #[test]
    fn timeout_kills_the_process() {
        // rustc reading source from an open, empty stdin waits forever.
        let mut cmd = rustc();
        cmd.arg("-").stdin(Stdio::piped());
        let started = Instant::now();
        assert!(matches!(
            run_with_timeout(cmd, Duration::from_millis(300)),
            Err(SidecarError::Timeout { .. })
        ));
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    /// The roles the test binary plays when it runs itself for [`timeout_kills_the_whole_process_tree`].
    const ROLE: &str = "SCORE64_TREE_ROLE";
    const PIDFILE: &str = "SCORE64_TREE_PIDFILE";
    const TREE_TEST: &str = "sidecar::tests::timeout_kills_the_whole_process_tree";

    /// This test binary, running only the tree test, in `role`.
    fn myself(role: &str, pidfile: &Path) -> Command {
        let mut c = Command::new(std::env::current_exe().unwrap());
        c.args([TREE_TEST, "--exact", "--nocapture", "--test-threads=1"])
            .env(ROLE, role)
            .env(PIDFILE, pidfile);
        c
    }

    /// Whether process `pid` still exists.
    fn alive(pid: u32) -> bool {
        #[cfg(unix)]
        {
            Command::new("kill")
                .args(["-0", &pid.to_string()])
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        }
        #[cfg(windows)]
        {
            let out = Command::new("tasklist")
                .args(["/FI", &format!("PID eq {pid}"), "/NH"])
                .output()
                .expect("tasklist");
            String::from_utf8_lossy(&out.stdout).contains(&pid.to_string())
        }
    }

    /// The runner kills the process it started and every process that one started, as `uv` starting Python would
    /// (#126). Run as a test it plays the runner; run by itself with SCORE64_TREE_ROLE it plays the "parent", which
    /// starts a sleeping "grandchild", records its PID and sleeps.
    #[test]
    fn timeout_kills_the_whole_process_tree() {
        match std::env::var(ROLE).as_deref() {
            Ok("parent") => {
                let pidfile = PathBuf::from(std::env::var_os(PIDFILE).unwrap());
                let mut grandchild = myself("grandchild", &pidfile).spawn().unwrap();
                std::fs::write(&pidfile, grandchild.id().to_string()).unwrap();
                std::thread::sleep(Duration::from_secs(60));
                // Only reached if nobody killed this tree; tidy up rather than leave a zombie.
                let _ = grandchild.kill();
                let _ = grandchild.wait();
                return;
            }
            Ok("grandchild") => {
                std::thread::sleep(Duration::from_secs(60));
                return;
            }
            _ => {}
        }
        let d = temp_dir("tree");
        let pidfile = d.join("grandchild.pid");
        let cmd = myself("parent", &pidfile);
        // On a thread with a deadline: a surviving grandchild holds the stderr pipe open, which would block the
        // runner's reader and hang the test instead of failing it.
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(run_with_timeout(cmd, Duration::from_secs(3)));
        });
        let result = rx
            .recv_timeout(Duration::from_secs(30))
            .expect("the runner returns: nothing holds its stderr pipe open");
        assert!(
            matches!(result, Err(SidecarError::Timeout { .. })),
            "{result:?}"
        );
        let pid: u32 = std::fs::read_to_string(&pidfile)
            .expect("the parent recorded its child")
            .trim()
            .parse()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while alive(pid) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(!alive(pid), "grandchild {pid} survived the timeout");
        let _ = std::fs::remove_dir_all(&d);
    }
}
