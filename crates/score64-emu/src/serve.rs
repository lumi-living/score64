//! The protocol loop: requests on stdin, one reply each on stdout (`docs/reference/emu-protocol.md`).

use std::io::{self, Read, Write};

use score64_formats::emu_protocol::{self as proto, ErrorCode, Reply, Request};

use crate::session::{Failure, Session};
use crate::sid::Chip;

/// Name and version reported in `Ready`.
const HELPER: &str = concat!("score64-emu ", env!("CARGO_PKG_VERSION"));
/// SID engine reported in `Ready` (ADR-011).
const ENGINE: &str = "residfp 1.2.3";

fn error(code: ErrorCode, message: impl Into<String>) -> Reply {
    Reply::Error {
        code,
        message: message.into(),
    }
}

fn result(r: Result<(), Failure>) -> Reply {
    match r {
        Ok(()) => Reply::Ok,
        Err(f) => error(f.code, f.message),
    }
}

/// Serve one session until `Quit`, end of input, or a malformed message.
pub fn serve(mut input: impl Read, mut output: impl Write) -> io::Result<()> {
    let mut session: Option<Session> = None;
    loop {
        let body = match proto::read_body(&mut input) {
            Ok(Some(b)) => b,
            Ok(None) => return Ok(()),
            Err(e) => {
                let reply = error(ErrorCode::Malformed, e.to_string());
                return proto::write_frame(&mut output, &reply.encode());
            }
        };
        let request = match Request::decode(&body) {
            Ok(r) => r,
            Err(e) => {
                let reply = error(ErrorCode::Malformed, e.to_string());
                return proto::write_frame(&mut output, &reply.encode());
            }
        };
        let (reply, quit) = handle(&mut session, request);
        proto::write_frame(&mut output, &reply.encode())?;
        if quit {
            return Ok(());
        }
    }
}

fn handle(session: &mut Option<Session>, request: Request) -> (Reply, bool) {
    let Some(s) = session.as_mut() else {
        return match request {
            Request::Hello {
                version,
                sample_rate,
            } => {
                if version != proto::VERSION {
                    return (
                        error(
                            ErrorCode::Version,
                            format!(
                                "protocol {version} not supported; this helper speaks {}",
                                proto::VERSION
                            ),
                        ),
                        false,
                    );
                }
                if !(8_000..=192_000).contains(&sample_rate) {
                    return (
                        error(
                            ErrorCode::Limit,
                            format!("sample rate {sample_rate} Hz out of 8000..=192000"),
                        ),
                        false,
                    );
                }
                match Session::new(sample_rate) {
                    Ok(new) => {
                        *session = Some(new);
                        (
                            Reply::Ready {
                                version: proto::VERSION,
                                sample_rate,
                                helper: HELPER.into(),
                                engine: ENGINE.into(),
                            },
                            false,
                        )
                    }
                    Err(f) => (error(f.code, f.message), false),
                }
            }
            Request::Quit => (Reply::Ok, true),
            _ => (error(ErrorCode::Sequence, "Hello must come first"), false),
        };
    };
    match request {
        Request::Hello { .. } => (error(ErrorCode::Sequence, "Hello was already sent"), false),
        Request::Load { address, data } => (result(s.load(address, &data)), false),
        Request::Init {
            init,
            play,
            subtune,
        } => (result(s.init(init, play, subtune)), false),
        Request::Render { frames, writes } => match s.render(frames, writes) {
            Ok(r) => (
                Reply::Rendered {
                    frames: r.frames,
                    max_play_cycles: r.max_play_cycles,
                    samples: r.samples,
                    writes: r.writes,
                },
                false,
            ),
            Err(f) => (error(f.code, f.message), false),
        },
        Request::Poke { address, value } => {
            s.poke(address, value);
            (Reply::Ok, false)
        }
        Request::Model { chip } => {
            let chip = match chip {
                proto::Chip::Mos6581 => Chip::Mos6581,
                proto::Chip::Mos8580 => Chip::Mos8580,
            };
            (result(s.model(chip)), false)
        }
        Request::Quit => (Reply::Ok, true),
    }
}

#[cfg(test)]
mod tests;
