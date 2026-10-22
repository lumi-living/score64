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
                version: 0,
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
            version: 0,
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
