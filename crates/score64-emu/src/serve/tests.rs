//! The serve loop driven in memory: requests in, replies out (client → helper direction).

use score64_formats::emu_protocol::{self as proto, Chip, ErrorCode, Reply, Request};

use super::serve;

/// A tiny player. Init (`$1000`) returns at once. Play (`$1003`) writes the mailbox byte at `$0340` to the volume
/// register, then starts a sawtooth on voice 1 at full sustain.
fn player() -> Vec<u8> {
    let mut p = vec![0x60, 0xEA, 0xEA]; // $1000: RTS; padding
    p.extend_from_slice(&[
        0xAD, 0x40, 0x03, // LDA $0340
        0x8D, 0x18, 0xD4, // STA $D418
        0xA9, 0x11, 0x8D, 0x01, 0xD4, // LDA #$11, STA $D401 (frequency high)
        0xA9, 0xF0, 0x8D, 0x06, 0xD4, // LDA #$F0, STA $D406 (sustain F)
        0xA9, 0x21, 0x8D, 0x04, 0xD4, // LDA #$21, STA $D404 (sawtooth + gate)
        0x60, // RTS
    ]);
    p
}

fn run(requests: &[Request]) -> Vec<Reply> {
    let input: Vec<u8> = requests.iter().flat_map(Request::encode).collect();
    run_bytes(&input)
}

fn run_bytes(input: &[u8]) -> Vec<Reply> {
    let mut out = Vec::new();
    serve(input, &mut out).unwrap();
    let mut stream = out.as_slice();
    let mut replies = Vec::new();
    while let Some(body) = proto::read_body(&mut stream).unwrap() {
        replies.push(Reply::decode(&body).unwrap());
    }
    replies
}

fn hello() -> Request {
    Request::Hello {
        version: proto::VERSION,
        sample_rate: 48_000,
    }
}

fn code(r: &Reply) -> ErrorCode {
    match r {
        Reply::Error { code, .. } => *code,
        other => panic!("expected an error, got {other:?}"),
    }
}

#[test]
fn full_session() {
    let replies = run(&[
        hello(),
        Request::Load {
            address: 0x1000,
            data: player(),
        },
        Request::Poke {
            address: 0x0340,
            value: 0x0F,
        },
        Request::Init {
            init: 0x1000,
            play: 0x1003,
            subtune: 0,
        },
        Request::Render {
            frames: 50,
            writes: true,
        },
        Request::Model {
            chip: Chip::Mos8580,
        },
        Request::Render {
            frames: 1,
            writes: false,
        },
        Request::Quit,
        Request::Render {
            frames: 1,
            writes: false,
        }, // after Quit: never answered
    ]);
    assert_eq!(replies.len(), 8);
    assert!(
        matches!(
            &replies[0],
            Reply::Ready {
                version: proto::VERSION,
                sample_rate: 48_000,
                ..
            }
        ),
        "{:?}",
        replies[0]
    );
    assert_eq!(&replies[1..4], [Reply::Ok, Reply::Ok, Reply::Ok]);
    let Reply::Rendered {
        frames,
        max_play_cycles,
        samples,
        writes,
    } = &replies[4]
    else {
        panic!("{:?}", replies[4])
    };
    assert_eq!(*frames, 50);
    // 50 frames of 19,656 cycles at 985,248 Hz is 0.9975 s: 47,880 samples at 48 kHz, give or take the resampler.
    assert!(
        (47_870..=47_900).contains(&samples.len()),
        "{} samples",
        samples.len()
    );
    assert!(
        samples.iter().map(|s| s.unsigned_abs()).max().unwrap() > 1000,
        "silent"
    );
    assert!(*max_play_cycles > 0 && *max_play_cycles < 100);
    assert_eq!(writes.len(), 50 * 4);
    assert_eq!(
        (writes[0].frame, writes[0].register, writes[0].value),
        (0, 0x18, 0x0F),
        "poked mailbox reaches the SID"
    );
    assert_eq!(writes[199].frame, 49);
    assert!(writes.iter().all(|w| u64::from(w.cycle) < 19_656));
    assert_eq!(replies[5], Reply::Ok);
    let Reply::Rendered {
        writes, samples, ..
    } = &replies[6]
    else {
        panic!("{:?}", replies[6])
    };
    assert!(writes.is_empty(), "writes only when asked");
    assert!(
        (955..=960).contains(&samples.len()),
        "{} samples for one frame",
        samples.len()
    );
    assert_eq!(replies[7], Reply::Ok);
}

#[test]
fn sequence_and_version_errors() {
    let replies = run(&[
        Request::Load {
            address: 0x1000,
            data: vec![0x60],
        },
        Request::Hello {
            version: 9,
            sample_rate: 48_000,
        },
        Request::Hello {
            version: proto::VERSION,
            sample_rate: 1_000,
        },
        hello(),
        hello(),
        Request::Render {
            frames: 1,
            writes: false,
        },
    ]);
    assert_eq!(code(&replies[0]), ErrorCode::Sequence);
    assert_eq!(code(&replies[1]), ErrorCode::Version);
    assert_eq!(code(&replies[2]), ErrorCode::Limit);
    assert!(matches!(replies[3], Reply::Ready { .. }));
    assert_eq!(code(&replies[4]), ErrorCode::Sequence, "second Hello");
    assert_eq!(code(&replies[5]), ErrorCode::Sequence, "Render before Init");
}

#[test]
fn limits_and_cpu_errors_keep_the_session() {
    let replies = run(&[
        hello(),
        Request::Load {
            address: 0xFFFF,
            data: vec![1, 2],
        },
        Request::Load {
            address: 0x1000,
            data: vec![0x60, 0xEA, 0xEA, 0x02],
        }, // play at $1003 is a JAM
        Request::Init {
            init: 0x1000,
            play: 0x1003,
            subtune: 0,
        },
        Request::Render {
            frames: 0,
            writes: false,
        },
        Request::Render {
            frames: 1,
            writes: false,
        },
        Request::Render {
            frames: 1,
            writes: false,
        },
        Request::Load {
            address: 0x1000,
            data: player(),
        },
        Request::Init {
            init: 0x1000,
            play: 0x1003,
            subtune: 0,
        },
        Request::Render {
            frames: 1,
            writes: false,
        },
    ]);
    assert_eq!(code(&replies[1]), ErrorCode::Limit);
    assert_eq!(replies[2], Reply::Ok);
    assert_eq!(replies[3], Reply::Ok);
    assert_eq!(code(&replies[4]), ErrorCode::Limit);
    assert_eq!(code(&replies[5]), ErrorCode::CpuJam);
    assert_eq!(
        code(&replies[6]),
        ErrorCode::Sequence,
        "a failed play call ends the timeline"
    );
    assert!(
        matches!(replies[9], Reply::Rendered { frames: 1, .. }),
        "recovers after a new Init"
    );
}

/// A render the helper accepts must never produce a reply the client has to reject (over `MAX_MESSAGE`).
#[test]
fn oversized_renders_are_refused_and_the_session_continues() {
    let rate = 192_000;
    let replies = run(&[
        Request::Hello {
            version: proto::VERSION,
            sample_rate: rate,
        },
        Request::Load {
            address: 0x1000,
            data: player(),
        },
        Request::Init {
            init: 0x1000,
            play: 0x1003,
            subtune: 0,
        },
        // 10 minutes at 192 kHz: about 230 MB of samples.
        Request::Render {
            frames: proto::MAX_RENDER_FRAMES,
            writes: false,
        },
        // Just over the cap with writes asked for.
        Request::Render {
            frames: proto::max_render_frames(rate, true) + 1,
            writes: true,
        },
        Request::Render {
            frames: 1,
            writes: true,
        },
    ]);
    for r in &replies[3..5] {
        assert_eq!(code(r), ErrorCode::Limit);
        let Reply::Error { message, .. } = r else {
            unreachable!()
        };
        assert!(message.contains("at most"), "{message}");
    }
    match &replies[5] {
        Reply::Rendered {
            frames: 1,
            samples,
            writes,
            ..
        } => {
            assert!(
                (3_820..=3_840).contains(&samples.len()),
                "{}",
                samples.len()
            );
            assert_eq!(
                writes[0].frame, 0,
                "the refused renders did not advance the timeline"
            );
        }
        other => panic!("expected Rendered, got {other:?}"),
    }
}

#[test]
fn malformed_input_is_answered_then_the_helper_stops() {
    let mut input = hello().encode();
    input.extend_from_slice(&[0x01, 0, 0, 0, 0x55]); // unknown kind
    input.extend(Request::Quit.encode());
    let replies = run_bytes(&input);
    assert_eq!(replies.len(), 2);
    assert_eq!(code(&replies[1]), ErrorCode::Malformed);

    let replies = run_bytes(&[0x09, 0, 0]); // ends inside the length prefix
    assert_eq!(replies.len(), 1);
    assert_eq!(code(&replies[0]), ErrorCode::Malformed);

    assert!(run_bytes(&[]).is_empty(), "clean end of input: no reply");
}

/// A player that touches known memory. Init (`$1000` → `$1010`) clears `$FB` and `$2000`. Play (`$1003` → `$1020`)
/// counts frames in `$FB`, adds one to `$2000` every 8th frame, and writes `$C123`, the SID volume and the stack.
fn tracer() -> Vec<u8> {
    let mut p = vec![0xEA; 0x40];
    p[0x00..0x06].copy_from_slice(&[0x4C, 0x10, 0x10, 0x4C, 0x20, 0x10]); // JMP $1010, JMP $1020
    p[0x10..0x18].copy_from_slice(&[
        0xA9, 0x00, // LDA #$00
        0x85, 0xFB, // STA $FB
        0x8D, 0x00, 0x20, // STA $2000
        0x60, // RTS
    ]);
    p[0x20..0x34].copy_from_slice(&[
        0xE6, 0xFB, // INC $FB
        0xA5, 0xFB, // LDA $FB
        0x29, 0x07, // AND #$07
        0xD0, 0x03, // BNE +3
        0xEE, 0x00, 0x20, // INC $2000
        0x8D, 0x23, 0xC1, // STA $C123
        0x8D, 0x18, 0xD4, // STA $D418
        0x48, // PHA
        0x68, // PLA
        0x60, // RTS
    ]);
    p
}

fn trace_all() -> Request {
    Request::Trace {
        zero_page: true,
        outside: Some((0x1000, 0x10FF)),
        watch: vec![0x2000, 0x3000],
    }
}

fn traced(r: &Reply) -> &proto::Traced {
    match r {
        Reply::Traced(t) => t,
        other => panic!("expected Traced, got {other:?}"),
    }
}

#[test]
fn a_trace_reports_zero_page_outside_writes_and_watched_changes() {
    let render = |frames| Request::Render {
        frames,
        writes: false,
    };
    let replies = run(&[
        hello(),
        Request::Load {
            address: 0x1000,
            data: tracer(),
        },
        trace_all(),
        Request::Init {
            init: 0x1000,
            play: 0x1003,
            subtune: 0,
        },
        render(20),
        Request::Report,
        // The host's own writes are neither changes nor outside writes.
        Request::Poke {
            address: 0x2000,
            value: 0x50,
        },
        Request::Poke {
            address: 0x3000,
            value: 0x01,
        },
        render(10),
        Request::Report,
        Request::Report,
    ]);
    assert!(
        replies[..5].iter().skip(1).take(3).all(|r| *r == Reply::Ok),
        "{replies:?}"
    );
    let first = traced(&replies[5]);
    assert_eq!(first.zero_page, [0xFB].into());
    assert_eq!(first.pages_written_outside, [0x20, 0xC1].into());
    // $FB counts 1, 2, ...; $2000 goes up when it reaches 8 (frame 7) and 16 (frame 15).
    assert_eq!(
        first.watched,
        [
            proto::Watched {
                address: 0x2000,
                start: 0,
                changes: vec![(7, 1), (15, 2)],
            },
            proto::Watched {
                address: 0x3000,
                start: 0,
                changes: vec![],
            },
        ]
    );
    // The second span starts where the first ended, then sees the poke's value go up at frame 23. The sets are
    // cumulative; the changes are only the new ones.
    let second = traced(&replies[9]);
    assert_eq!(second.zero_page, [0xFB].into());
    assert_eq!(second.pages_written_outside, [0x20, 0xC1].into());
    assert_eq!(second.watched[0].start, 2);
    assert_eq!(second.watched[0].changes, [(23, 0x51)]);
    assert_eq!(second.watched[1].changes, []);
    let third = traced(&replies[10]);
    assert_eq!(
        (third.watched[0].start, third.watched[0].changes.len()),
        (0x51, 0)
    );
}

#[test]
fn a_trace_starts_at_init_and_has_its_rules() {
    let init = Request::Init {
        init: 0x1000,
        play: 0x1003,
        subtune: 0,
    };
    let load = Request::Load {
        address: 0x1000,
        data: tracer(),
    };
    let watch = |watch: Vec<u16>| Request::Trace {
        zero_page: false,
        outside: None,
        watch,
    };
    let replies = run(&[
        hello(),
        load.clone(),
        Request::Report, // nothing traced yet
        init.clone(),
        Request::Report, // Init without a trace
        watch((0..17).collect()),
        watch(vec![0x2000, 0x2000]),
        watch(vec![0xD418]),
        Request::Trace {
            zero_page: true,
            outside: Some((0x2000, 0x1000)),
            watch: vec![],
        },
        // Only what was asked for is collected.
        watch(vec![0x2000]),
        init.clone(),
        Request::Render {
            frames: 8,
            writes: false,
        },
        Request::Report,
        // Everything off disarms it for the next Init.
        Request::Trace {
            zero_page: false,
            outside: None,
            watch: vec![],
        },
        init,
        Request::Report,
    ]);
    assert_eq!(code(&replies[2]), ErrorCode::Sequence);
    assert_eq!(code(&replies[4]), ErrorCode::Sequence);
    for r in &replies[5..9] {
        assert_eq!(code(r), ErrorCode::Limit, "{r:?}");
    }
    let t = traced(&replies[12]);
    assert!(t.zero_page.is_empty() && t.pages_written_outside.is_empty());
    assert_eq!(t.watched[0].changes, [(7, 1)]);
    assert_eq!(code(&replies[15]), ErrorCode::Sequence);
}
