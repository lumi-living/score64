//! The helper lifecycle against scripted fake helpers, and against the real binary when it has been built.

use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use score64_formats::emu_protocol::{self as proto, Reply, Request};

use super::*;

/// What a fake helper does with a request.
enum Act {
    Reply(Reply),
    /// Never answer (the pipe stays open).
    Hang,
    /// Close the pipe, as a crash would.
    Close,
}

/// A spawner of fake helpers. Each answers Hello itself, then asks `script(instance, request)`, where instance counts
/// from 1 across restarts.
fn fake(script: impl Fn(u32, &Request) -> Act + Send + Sync + 'static) -> Spawn {
    let script = Arc::new(script);
    let count = Arc::new(AtomicU32::new(0));
    Arc::new(move |_killer| {
        let n = count.fetch_add(1, Ordering::SeqCst) + 1;
        let (client_in, mut helper_out) = io::pipe()?;
        let (mut helper_in, client_out) = io::pipe()?;
        let script = script.clone();
        thread::spawn(move || {
            while let Ok(Some(body)) = proto::read_body(&mut helper_in) {
                let request = Request::decode(&body).unwrap();
                let act = match request {
                    Request::Hello { sample_rate, .. } => Act::Reply(Reply::Ready {
                        version: proto::VERSION,
                        sample_rate,
                        helper: format!("fake {n}"),
                        engine: "none".into(),
                    }),
                    ref r => script(n, r),
                };
                match act {
                    Act::Reply(r) => proto::write_frame(&mut helper_out, &r.encode()).unwrap(),
                    Act::Hang => loop {
                        thread::park();
                    },
                    Act::Close => return,
                }
            }
        });
        let r: Box<dyn Read + Send> = Box::new(client_in);
        let w: Box<dyn Write + Send> = Box::new(client_out);
        Helper::connect(r, w, 48_000)
    })
}

fn poke(slot: &mut HelperSlot) -> Result<(), CommandError> {
    slot.call(|h| h.poke(0x0340, 1))
}

fn ok_slot(script: impl Fn(u32, &Request) -> Act + Send + Sync + 'static) -> HelperSlot {
    HelperSlot::with_spawn(fake(script), Duration::from_millis(300))
}

#[test]
fn starts_lazily_and_is_reused() {
    let mut slot = ok_slot(|_, _| Act::Reply(Reply::Ok));
    assert_eq!(slot.starts(), 0, "nothing runs before a command needs it");
    poke(&mut slot).unwrap();
    poke(&mut slot).unwrap();
    assert_eq!(slot.starts(), 1);
    let info = slot.call(|h| Ok(h.info().clone())).unwrap();
    assert_eq!(info.helper, "fake 1");
}

#[test]
fn a_refusal_keeps_the_helper() {
    let mut slot = ok_slot(|_, r| match r {
        Request::Render { .. } => Act::Reply(Reply::Error {
            code: ErrorCode::Sequence,
            message: "render before init".into(),
        }),
        _ => Act::Reply(Reply::Ok),
    });
    let e = slot.call(|h| h.render(1, false)).unwrap_err();
    assert_eq!(e.kind, ErrorKind::HelperRefused);
    poke(&mut slot).unwrap();
    assert_eq!(slot.starts(), 1);
}

#[test]
fn malformed_restarts_on_next_use() {
    let mut slot = ok_slot(|n, _| match n {
        1 => Act::Reply(Reply::Error {
            code: ErrorCode::Malformed,
            message: "bad length".into(),
        }),
        _ => Act::Reply(Reply::Ok),
    });
    assert_eq!(poke(&mut slot).unwrap_err().kind, ErrorKind::HelperFailed);
    poke(&mut slot).unwrap();
    assert_eq!(slot.starts(), 2);
}

#[test]
fn a_crash_restarts_on_next_use() {
    let mut slot = ok_slot(|n, _| {
        if n == 1 {
            Act::Close
        } else {
            Act::Reply(Reply::Ok)
        }
    });
    assert_eq!(poke(&mut slot).unwrap_err().kind, ErrorKind::HelperFailed);
    poke(&mut slot).unwrap();
    assert_eq!(slot.starts(), 2);
}

#[test]
fn a_hung_helper_times_out_and_restarts() {
    let mut slot = ok_slot(|n, _| {
        if n == 1 {
            Act::Hang
        } else {
            Act::Reply(Reply::Ok)
        }
    });
    let t = Instant::now();
    assert_eq!(poke(&mut slot).unwrap_err().kind, ErrorKind::HelperTimeout);
    assert!(t.elapsed() < Duration::from_secs(5), "{:?}", t.elapsed());
    poke(&mut slot).unwrap();
    assert_eq!(slot.starts(), 2);
}

#[test]
fn a_hung_handshake_times_out() {
    let spawn: Spawn = Arc::new(|_killer| {
        let (client_in, _helper_out) = io::pipe()?;
        let (_helper_in, client_out) = io::pipe()?;
        // Keep the helper's ends open and never answer Hello.
        thread::spawn(move || {
            let _keep = (_helper_out, _helper_in);
            loop {
                thread::park();
            }
        });
        let r: Box<dyn Read + Send> = Box::new(client_in);
        let w: Box<dyn Write + Send> = Box::new(client_out);
        Helper::connect(r, w, 48_000)
    });
    let mut slot = HelperSlot::with_spawn(spawn, Duration::from_millis(300));
    assert_eq!(poke(&mut slot).unwrap_err().kind, ErrorKind::HelperTimeout);
}

#[test]
fn a_panicking_job_restarts_on_next_use() {
    let mut slot = ok_slot(|_, _| Act::Reply(Reply::Ok));
    let e = slot
        .call(|_| -> Result<(), HelperError> { panic!("job bug") })
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::HelperFailed);
    poke(&mut slot).unwrap();
    assert_eq!(slot.starts(), 2);
}

#[test]
fn a_binary_that_does_not_start_is_reported() {
    let mut slot = HelperSlot::new(HelperConfig {
        binary: Some("/nonexistent/score64-emu".into()),
        ..HelperConfig::default()
    });
    assert_eq!(poke(&mut slot).unwrap_err().kind, ErrorKind::HelperFailed);
}

/// $1000: RTS (init). $1003: start voice 1, RTS (play).
const PLAYER: [u8; 9] = [0x60, 0xEA, 0xEA, 0xA9, 0x21, 0x8D, 0x04, 0xD4, 0x60];

#[test]
fn the_real_helper_is_killed_on_timeout_and_restarted() {
    let mut slot = HelperSlot::new(HelperConfig::default());
    if slot.binary().is_none() {
        let _ = writeln!(
            io::stderr(),
            "SKIPPED the_real_helper_is_killed_on_timeout_and_restarted: build score64-emu or set SCORE64_EMU"
        );
        return;
    }
    let info = slot.call(|h| Ok(h.info().clone())).unwrap();
    assert!(info.helper.starts_with("score64-emu "), "{info:?}");
    slot.call(|h| {
        h.load(0x1000, &PLAYER)?;
        h.init(0x1000, 0x1003, 0)
    })
    .unwrap();
    // Ten minutes of audio cannot render in 50 ms: the watchdog kills the process.
    let e = slot
        .call_for(Duration::from_millis(50), |h| {
            h.render(proto::MAX_RENDER_FRAMES, false).map(|_| ())
        })
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::HelperTimeout);
    // A fresh helper has no timeline.
    let e = slot.call(|h| h.render(1, false).map(|_| ())).unwrap_err();
    assert_eq!(e.kind, ErrorKind::HelperRefused);
    assert_eq!(slot.starts(), 2);
}
