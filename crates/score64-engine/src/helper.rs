//! The emulator helper's lifecycle (ADR-012). Commands hand the slot a job; the slot makes sure a helper is running,
//! runs the job on the helper's worker thread and waits for it with a timeout. A helper that breaks is dropped and a
//! fresh one starts on the next use.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use score64_adapters::emu::{DynHelper, Helper, HelperError, HelperInfo, Killer};
use score64_formats::emu_protocol::ErrorCode;

use crate::{CommandError, ErrorKind};

/// How the engine runs the emulator helper.
#[derive(Clone, Debug)]
pub struct HelperConfig {
    /// The `score64-emu` binary. `None` looks for it as [`Helper::locate`] does: `SCORE64_EMU`, then next to the
    /// running program.
    pub binary: Option<PathBuf>,
    /// Output sample rate.
    pub sample_rate: u32,
    /// How long one job (or start-up) may take before the helper is killed. Commands with long renders pass their
    /// own, longer timeout.
    pub timeout: Duration,
}

impl Default for HelperConfig {
    fn default() -> Self {
        Self {
            binary: None,
            sample_rate: 48_000,
            timeout: Duration::from_secs(30),
        }
    }
}

/// Starts a helper, putting its process (if any) in the killer first.
pub(crate) type Spawn = Arc<dyn Fn(Killer) -> Result<DynHelper, HelperError> + Send + Sync>;

type Job = Box<dyn FnOnce(&mut DynHelper) + Send>;

/// A running helper: its worker thread's job queue and a way to kill it from here.
struct Worker {
    jobs: mpsc::Sender<Job>,
    killer: Killer,
}

impl Drop for Worker {
    fn drop(&mut self) {
        // Closing the queue ends the worker loop; killing closes the pipes, so a worker blocked on a read returns too.
        self.killer.kill();
    }
}

/// The engine's helper: none until a command needs it.
pub(crate) struct HelperSlot {
    spawn: Option<Spawn>,
    binary: Option<String>,
    timeout: Duration,
    worker: Option<Worker>,
    starts: u32,
}

impl HelperSlot {
    pub fn new(config: HelperConfig) -> Self {
        let binary = config.binary.or_else(Helper::locate);
        let rate = config.sample_rate;
        let spawn: Option<Spawn> = binary.clone().map(|path| -> Spawn {
            Arc::new(move |killer| Helper::spawn_with(&path, rate, killer))
        });
        Self {
            spawn,
            binary: binary.map(|p| p.display().to_string()),
            timeout: config.timeout,
            worker: None,
            starts: 0,
        }
    }

    /// A slot that starts helpers with `spawn` (scripted fakes in tests).
    #[cfg(test)]
    pub fn with_spawn(spawn: Spawn, timeout: Duration) -> Self {
        Self {
            spawn: Some(spawn),
            binary: Some("fake".into()),
            timeout,
            worker: None,
            starts: 0,
        }
    }

    /// The helper binary in use, if one was found.
    pub fn binary(&self) -> Option<String> {
        self.binary.clone()
    }

    /// How long one job may take by default.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// How many helpers this slot has started.
    pub fn starts(&self) -> u32 {
        self.starts
    }

    /// Run `job` on the helper with the configured timeout.
    pub fn call<R: Send + 'static>(
        &mut self,
        job: impl FnOnce(&mut DynHelper) -> Result<R, HelperError> + Send + 'static,
    ) -> Result<R, CommandError> {
        self.call_for(self.timeout, job)
    }

    /// Run `job` on the helper, starting one if none is running, and wait at most `timeout` for it.
    pub fn call_for<R: Send + 'static>(
        &mut self,
        timeout: Duration,
        job: impl FnOnce(&mut DynHelper) -> Result<R, HelperError> + Send + 'static,
    ) -> Result<R, CommandError> {
        self.ensure()?;
        let worker = self.worker.as_ref().expect("ensure started a worker");
        let (tx, rx) = mpsc::channel();
        let queued = worker.jobs.send(Box::new(move |h: &mut DynHelper| {
            let _ = tx.send(job(h));
        }));
        if queued.is_err() {
            self.worker = None;
            return Err(failed("the helper's worker has stopped; it will restart"));
        }
        match rx.recv_timeout(timeout) {
            Ok(Ok(r)) => Ok(r),
            Ok(Err(e)) => {
                let (fatal, err) = classify(e);
                if fatal {
                    self.worker = None;
                }
                Err(err)
            }
            Err(RecvTimeoutError::Timeout) => {
                self.worker = None;
                Err(CommandError::new(
                    ErrorKind::HelperTimeout,
                    format!(
                        "the emulator helper did not answer within {timeout:?} and was stopped; it will restart"
                    ),
                ))
            }
            Err(RecvTimeoutError::Disconnected) => {
                self.worker = None;
                Err(failed(
                    "the helper's worker stopped mid-job; it will restart",
                ))
            }
        }
    }

    /// Start a helper on its own thread if none is running. The handshake is under the same timeout as a job.
    fn ensure(&mut self) -> Result<(), CommandError> {
        if self.worker.is_some() {
            return Ok(());
        }
        let Some(spawn) = self.spawn.clone() else {
            return Err(CommandError::new(
                ErrorKind::HelperMissing,
                "score64-emu not found: install it next to score64, or set SCORE64_EMU to its path",
            ));
        };
        let killer = Killer::new();
        let (jobs, queue) = mpsc::channel::<Job>();
        let (ready_tx, ready) = mpsc::channel::<Result<HelperInfo, HelperError>>();
        let k = killer.clone();
        thread::Builder::new()
            .name("score64-emu".into())
            .spawn(move || {
                let mut helper = match spawn(k) {
                    Ok(h) => {
                        let _ = ready_tx.send(Ok(h.info().clone()));
                        h
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                for job in queue {
                    job(&mut helper);
                }
            })
            .map_err(|e| CommandError::new(ErrorKind::Internal, format!("helper thread: {e}")))?;
        self.starts += 1;
        let worker = Worker { jobs, killer };
        match ready.recv_timeout(self.timeout) {
            Ok(Ok(_)) => {
                self.worker = Some(worker);
                Ok(())
            }
            Ok(Err(e)) => Err(match e {
                HelperError::Io(e) => failed(format!("could not start the emulator helper: {e}")),
                e => classify(e).1,
            }),
            Err(RecvTimeoutError::Timeout) => Err(CommandError::new(
                ErrorKind::HelperTimeout,
                format!(
                    "the emulator helper did not say hello within {:?}",
                    self.timeout
                ),
            )),
            Err(RecvTimeoutError::Disconnected) => {
                Err(failed("the helper's worker stopped while starting"))
            }
        }
    }
}

/// Whether an error leaves the helper unusable, and how to report it.
fn classify(e: HelperError) -> (bool, CommandError) {
    match e {
        HelperError::Refused {
            code: ErrorCode::Malformed,
            message,
        } => (
            true,
            failed(format!(
                "the emulator helper rejected a message and stopped ({message}); it will restart"
            )),
        ),
        HelperError::Refused { code, message } => (
            false,
            CommandError::new(
                ErrorKind::HelperRefused,
                format!("emulator helper ({code:?}): {message}"),
            ),
        ),
        HelperError::Io(e) => (
            true,
            failed(format!("emulator helper pipe: {e}; it will restart")),
        ),
        HelperError::Protocol(m) => (
            true,
            failed(format!("emulator helper protocol: {m}; it will restart")),
        ),
    }
}

fn failed(message: impl Into<String>) -> CommandError {
    CommandError::new(ErrorKind::HelperFailed, message)
}

#[cfg(test)]
mod tests;
