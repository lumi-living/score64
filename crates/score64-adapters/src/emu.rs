//! Client for the emulator helper (`score64-emu serve`, GPL), over its stdin/stdout protocol
//! (`docs/reference/emu-protocol.md`). This crate stays MIT because it only ever talks to the helper as a separate
//! process (ADR-002); the protocol codec is shared through `score64_formats::emu_protocol`.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, PoisonError};

use score64_formats::emu_protocol::{self as proto, ErrorCode, Reply, Request};
use score64_formats::psid::Psid;

pub use score64_formats::emu_protocol::{Chip, SidWrite};

/// Why a helper call failed.
#[derive(Debug)]
pub enum HelperError {
    /// The pipe to the helper broke, or it could not be started.
    Io(io::Error),
    /// The helper sent something that is not a valid reply, or the wrong kind of reply.
    Protocol(String),
    /// The helper answered with an error.
    Refused {
        /// Its error code.
        code: ErrorCode,
        /// Its message.
        message: String,
    },
}

impl std::fmt::Display for HelperError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HelperError::Io(e) => write!(f, "emulator helper: {e}"),
            HelperError::Protocol(m) => write!(f, "emulator helper protocol: {m}"),
            HelperError::Refused { code, message } => {
                write!(f, "emulator helper refused ({code:?}): {message}")
            }
        }
    }
}

impl std::error::Error for HelperError {}

impl From<io::Error> for HelperError {
    fn from(e: io::Error) -> Self {
        HelperError::Io(e)
    }
}

/// What the helper said in its `Ready` reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelperInfo {
    /// Protocol version.
    pub version: u16,
    /// Sample rate in use.
    pub sample_rate: u32,
    /// Helper name and version.
    pub helper: String,
    /// SID engine.
    pub engine: String,
}

/// Frames rendered by the helper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    /// Frames rendered.
    pub frames: u32,
    /// Longest play call, in CPU cycles.
    pub max_play_cycles: u32,
    /// Mono 16-bit samples.
    pub samples: Vec<i16>,
    /// SID writes, if asked for.
    pub writes: Vec<SidWrite>,
}

/// A connected helper session. Generic over the pipe so it can be tested without a process.
pub struct Helper<R: Read, W: Write> {
    reader: R,
    writer: W,
    info: HelperInfo,
    killer: Killer,
}

/// A helper on boxed pipes: what [`Helper::spawn`] returns, and what fakes in tests can stand in for.
pub type DynHelper = Helper<Box<dyn Read + Send>, Box<dyn Write + Send>>;

/// Kills a helper's process from any thread, including while another thread is blocked reading its replies (killing
/// it closes the pipe, so that read returns). Cloning shares the same process. A killer with no process does nothing.
#[derive(Clone, Default)]
pub struct Killer(Arc<Mutex<Option<Child>>>);

impl Killer {
    /// A killer with no process yet; [`Helper::spawn_with`] puts the child in it before saying hello.
    pub fn new() -> Self {
        Self::default()
    }

    /// Kill the process, if any, and reap it.
    pub fn kill(&self) {
        if let Some(mut child) = self.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    fn take(&self) -> Option<Child> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).take()
    }
}

impl DynHelper {
    /// Start `binary serve` and say hello.
    pub fn spawn(binary: &Path, sample_rate: u32) -> Result<Self, HelperError> {
        Self::spawn_with(binary, sample_rate, Killer::new())
    }

    /// As [`Self::spawn`], with the child placed in `killer` before the handshake, so a caller can kill a helper that
    /// hangs even while saying hello.
    pub fn spawn_with(
        binary: &Path,
        sample_rate: u32,
        killer: Killer,
    ) -> Result<Self, HelperError> {
        let mut child = Command::new(binary)
            .arg("serve")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let stdout: Box<dyn Read + Send> = Box::new(child.stdout.take().expect("piped"));
        let stdin: Box<dyn Write + Send> = Box::new(child.stdin.take().expect("piped"));
        *killer.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(child);
        let mut helper =
            Helper::connect(stdout, stdin, sample_rate).inspect_err(|_| killer.kill())?;
        helper.killer = killer;
        Ok(helper)
    }

    /// Where the helper binary is: `SCORE64_EMU`, else next to the running executable (or one directory up, which is
    /// where Cargo puts binaries relative to test executables), else `None`.
    pub fn locate() -> Option<PathBuf> {
        if let Some(p) = std::env::var_os("SCORE64_EMU") {
            return Some(PathBuf::from(p));
        }
        let name = format!("score64-emu{}", std::env::consts::EXE_SUFFIX);
        let exe = std::env::current_exe().ok()?;
        let dir = exe.parent()?;
        [dir.join(&name), dir.parent()?.join(&name)]
            .into_iter()
            .find(|p| p.is_file())
    }
}

impl<R: Read, W: Write> Helper<R, W> {
    /// Say hello over an existing pipe and check the helper's answer.
    pub fn connect(reader: R, writer: W, sample_rate: u32) -> Result<Self, HelperError> {
        let mut h = Helper {
            reader,
            writer,
            info: HelperInfo {
                version: 0,
                sample_rate: 0,
                helper: String::new(),
                engine: String::new(),
            },
            killer: Killer::new(),
        };
        match h.ask(&Request::Hello {
            version: proto::VERSION,
            sample_rate,
        })? {
            Reply::Ready {
                version,
                sample_rate: rate,
                helper,
                engine,
            } => {
                if version != proto::VERSION || rate != sample_rate {
                    return Err(HelperError::Protocol(format!(
                        "helper is ready with protocol {version} at {rate} Hz, asked for {} at {sample_rate} Hz",
                        proto::VERSION
                    )));
                }
                h.info = HelperInfo {
                    version,
                    sample_rate: rate,
                    helper,
                    engine,
                };
                Ok(h)
            }
            other => Err(unexpected("Ready", &other)),
        }
    }

    /// The helper's `Ready` details.
    pub fn info(&self) -> &HelperInfo {
        &self.info
    }

    /// A handle that kills this helper's process from another thread.
    pub fn killer(&self) -> Killer {
        self.killer.clone()
    }

    /// The most frames one [`Self::render`] may ask for at this session's sample rate; split longer renders into
    /// chunks of this size.
    pub fn max_render_frames(&self, writes: bool) -> u32 {
        proto::max_render_frames(self.info.sample_rate, writes)
    }

    fn ask(&mut self, request: &Request) -> Result<Reply, HelperError> {
        proto::write_frame(&mut self.writer, &request.encode())?;
        let body = proto::read_body(&mut self.reader)?
            .ok_or_else(|| HelperError::Protocol("helper closed its output".into()))?;
        match Reply::decode(&body).map_err(|e| HelperError::Protocol(e.to_string()))? {
            Reply::Error { code, message } => Err(HelperError::Refused { code, message }),
            reply => Ok(reply),
        }
    }

    fn ok(&mut self, request: &Request) -> Result<(), HelperError> {
        match self.ask(request)? {
            Reply::Ok => Ok(()),
            other => Err(unexpected("Ok", &other)),
        }
    }

    /// Copy bytes into C64 RAM.
    pub fn load(&mut self, address: u16, data: &[u8]) -> Result<(), HelperError> {
        self.ok(&Request::Load {
            address,
            data: data.to_vec(),
        })
    }

    /// Load a PSID's data at its load address; returns its (init, play) addresses for [`Self::init`].
    pub fn load_psid(&mut self, psid: &Psid) -> Result<(u16, u16), HelperError> {
        self.load(psid.load_address, &psid.data)?;
        Ok((psid.init_address, psid.play_address))
    }

    /// Call init with A = `subtune` (0-based) and start a new timeline.
    pub fn init(&mut self, init: u16, play: u16, subtune: u8) -> Result<(), HelperError> {
        self.ok(&Request::Init {
            init,
            play,
            subtune,
        })
    }

    /// Render `frames` frames.
    pub fn render(&mut self, frames: u32, writes: bool) -> Result<Rendered, HelperError> {
        match self.ask(&Request::Render { frames, writes })? {
            Reply::Rendered {
                frames,
                max_play_cycles,
                samples,
                writes,
            } => Ok(Rendered {
                frames,
                max_play_cycles,
                samples,
                writes,
            }),
            other => Err(unexpected("Rendered", &other)),
        }
    }

    /// Write one byte to C64 memory now.
    pub fn poke(&mut self, address: u16, value: u8) -> Result<(), HelperError> {
        self.ok(&Request::Poke { address, value })
    }

    /// Select the SID model.
    pub fn model(&mut self, chip: Chip) -> Result<(), HelperError> {
        self.ok(&Request::Model { chip })
    }

    /// End the session and wait for the helper to exit.
    pub fn quit(mut self) -> Result<(), HelperError> {
        self.ok(&Request::Quit)?;
        if let Some(mut child) = self.killer.take() {
            child.wait()?;
        }
        Ok(())
    }
}

impl<R: Read, W: Write> Drop for Helper<R, W> {
    fn drop(&mut self) {
        // Never leave a helper process behind.
        self.killer.kill();
    }
}

fn unexpected(wanted: &str, got: &Reply) -> HelperError {
    let kind = match got {
        Reply::Ready { .. } => "Ready",
        Reply::Ok => "Ok",
        Reply::Rendered { .. } => "Rendered",
        Reply::Error { .. } => "Error",
    };
    HelperError::Protocol(format!("expected {wanted}, got {kind}"))
}

#[cfg(test)]
mod tests {
    //! The client against a scripted fake helper (helper → client direction).

    use super::*;
    use std::io::{PipeReader, PipeWriter};

    /// A fake helper on a thread: answers each request with the next scripted reply, and records the requests.
    fn fake(
        script: Vec<Reply>,
    ) -> (
        PipeReader,
        PipeWriter,
        std::thread::JoinHandle<Vec<Request>>,
    ) {
        let (client_in, mut helper_out) = std::io::pipe().unwrap();
        let (mut helper_in, client_out) = std::io::pipe().unwrap();
        let thread = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for reply in script {
                let Some(body) = proto::read_body(&mut helper_in).unwrap() else {
                    break;
                };
                seen.push(Request::decode(&body).unwrap());
                proto::write_frame(&mut helper_out, &reply.encode()).unwrap();
            }
            seen
        });
        (client_in, client_out, thread)
    }

    fn ready(rate: u32) -> Reply {
        Reply::Ready {
            version: proto::VERSION,
            sample_rate: rate,
            helper: "fake 1".into(),
            engine: "none".into(),
        }
    }

    #[test]
    fn handshake_and_calls() {
        let rendered = Reply::Rendered {
            frames: 2,
            max_play_cycles: 800,
            samples: vec![1, -1, 2],
            writes: vec![SidWrite {
                frame: 0,
                cycle: 10,
                register: 4,
                value: 0x21,
            }],
        };
        let (r, w, thread) = fake(vec![
            ready(48_000),
            Reply::Ok,
            Reply::Ok,
            rendered,
            Reply::Ok,
        ]);
        let mut h = Helper::connect(r, w, 48_000).unwrap();
        assert_eq!(h.info().helper, "fake 1");
        let psid = Psid {
            version: 2,
            load_address: 0x1000,
            init_address: 0x1000,
            play_address: 0x1003,
            songs: 1,
            start_song: 1,
            name: String::new(),
            data: vec![0x60],
        };
        let (init, play) = h.load_psid(&psid).unwrap();
        h.init(init, play, 0).unwrap();
        let r = h.render(2, true).unwrap();
        assert_eq!((r.frames, r.samples.len(), r.writes.len()), (2, 3, 1));
        h.quit().unwrap();
        assert_eq!(
            thread.join().unwrap(),
            [
                Request::Hello {
                    version: proto::VERSION,
                    sample_rate: 48_000
                },
                Request::Load {
                    address: 0x1000,
                    data: vec![0x60]
                },
                Request::Init {
                    init: 0x1000,
                    play: 0x1003,
                    subtune: 0
                },
                Request::Render {
                    frames: 2,
                    writes: true
                },
                Request::Quit,
            ]
        );
    }

    #[test]
    fn errors_wrong_replies_and_bad_handshakes() {
        let (r, w, _t) = fake(vec![
            ready(48_000),
            Reply::Error {
                code: ErrorCode::Sequence,
                message: "render before init".into(),
            },
            Reply::Ok,
        ]);
        let mut h = Helper::connect(r, w, 48_000).unwrap();
        assert!(matches!(
            h.render(1, false),
            Err(HelperError::Refused {
                code: ErrorCode::Sequence,
                ..
            })
        ));
        assert!(
            matches!(h.render(1, false), Err(HelperError::Protocol(_))),
            "Ok is not a Rendered"
        );

        let (r, w, _t) = fake(vec![ready(44_100)]);
        assert!(
            matches!(Helper::connect(r, w, 48_000), Err(HelperError::Protocol(_))),
            "rate mismatch"
        );

        let (r, w, _t) = fake(vec![Reply::Error {
            code: ErrorCode::Version,
            message: "no".into(),
        }]);
        assert!(matches!(
            Helper::connect(r, w, 48_000),
            Err(HelperError::Refused {
                code: ErrorCode::Version,
                ..
            })
        ));
    }
}
