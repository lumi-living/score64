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

/// A player that touches known memory. Init (`$1000` → `$1010`) clears `$FB` and `$FC`, writes `$C200`, and sets
/// `$2000` to `$40`. Play (`$1003` → `$1030`) counts frames in `$FB`, reads `$FA`, `($FD),Y` with Y = `$10` (so
/// `$FD`, `$FE` and `$0010`) and `$FF,X` with X = 1 (which wraps to `$00`), adds one to `$2000` every 8th frame, and
/// writes `$C123`, the SID volume and the stack.
fn tracer() -> Vec<u8> {
    let mut p = vec![0xEA; 0x50];
    p[0x00..0x06].copy_from_slice(&[0x4C, 0x10, 0x10, 0x4C, 0x30, 0x10]); // JMP $1010, JMP $1030
    p[0x10..0x1F].copy_from_slice(&[
        0xA9, 0x00, // LDA #$00
        0x85, 0xFB, // STA $FB
        0x85, 0xFC, // STA $FC (init only)
        0x8D, 0x00, 0xC2, // STA $C200 (init only)
        0xA9, 0x40, // LDA #$40
        0x8D, 0x00, 0x20, // STA $2000
        0x60, // RTS
    ]);
    p[0x30..0x4E].copy_from_slice(&[
        0xE6, 0xFB, // INC $FB
        0xA5, 0xFA, // LDA $FA (read only)
        0xA0, 0x10, // LDY #$10
        0xB1, 0xFD, // LDA ($FD),Y: $FD/$FE hold 0, so this reads $0010
        0xA2, 0x01, // LDX #$01
        0xB5, 0xFF, // LDA $FF,X: wraps to $00
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

fn init() -> Request {
    Request::Init {
        init: 0x1000,
        play: 0x1003,
        subtune: 0,
    }
}

fn render(frames: u32) -> Request {
    Request::Render {
        frames,
        writes: false,
    }
}

#[test]
fn a_trace_reports_zero_page_outside_writes_and_watched_changes() {
    let replies = run(&[
        hello(),
        Request::Load {
            address: 0x1000,
            data: tracer(),
        },
        trace_all(),
        init(),
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
        Request::Load {
            address: 0x3000,
            data: vec![0x77],
        },
        render(10),
        Request::Report,
        Request::Report,
    ]);
    assert!(replies[1..4].iter().all(|r| *r == Reply::Ok));
    let first = traced(&replies[5]);
    // Init's accesses count ($FC, $C2xx), and so does every kind of read: zero page, a pointer and the data it
    // points at, and an indexed read that wraps.
    assert_eq!(
        first.zero_page,
        [0x00, 0x10, 0xFA, 0xFB, 0xFC, 0xFD, 0xFE].into()
    );
    assert_eq!(first.pages_written_outside, [0x20, 0xC1, 0xC2].into());
    // $2000 starts at init's $40, then goes up when $FB reaches 8 (frame 7) and 16 (frame 15).
    assert_eq!(
        first.watched,
        [
            proto::Watched {
                address: 0x2000,
                start: 0x40,
                changes: vec![(7, 0x41), (15, 0x42)],
            },
            proto::Watched {
                address: 0x3000,
                start: 0,
                changes: vec![],
            },
        ]
    );
    // The second span starts where the first ended, then sees the poked value go up at frame 23. The sets are
    // cumulative; the changes are only the new ones. The host's poke and load are never changes.
    let second = traced(&replies[10]);
    assert_eq!(second.zero_page, first.zero_page);
    assert_eq!(second.pages_written_outside, first.pages_written_outside);
    assert_eq!(second.watched[0].start, 0x42);
    assert_eq!(second.watched[0].changes, [(23, 0x51)]);
    assert_eq!(second.watched[1].changes, []);
    let third = traced(&replies[11]);
    assert_eq!(
        (third.watched[0].start, third.watched[0].changes.len()),
        (0x51, 0)
    );
    assert_eq!(
        (third.watched[1].start, third.watched[1].changes.len()),
        (0x77, 0)
    );
}

/// A player whose play routine only stores A at `address`.
fn sid_writer(address: u16) -> Vec<u8> {
    let [lo, hi] = address.to_le_bytes();
    vec![0x60, 0x8D, lo, hi, 0x60] // init: RTS; play ($1001): STA address, RTS
}

#[test]
fn only_the_sids_registers_are_left_out() {
    for (address, pages) in [
        (0xD400, vec![]),
        (0xD418, vec![]),
        (0xD419, vec![0xD4]),
        (0xD438, vec![0xD4]), // a mirror
        (0xD500, vec![0xD5]), // where a second SID often sits
        (0x01F0, vec![]),     // the stack page
        // The window is inclusive at both ends.
        (0x1080, vec![]),
        (0x10FF, vec![]),
        (0x1100, vec![0x11]),
        (0x0FFF, vec![0x0F]),
    ] {
        let replies = run(&[
            hello(),
            Request::Load {
                address: 0x1000,
                data: sid_writer(address),
            },
            Request::Trace {
                zero_page: false,
                outside: Some((0x1000, 0x10FF)),
                watch: vec![],
            },
            Request::Init {
                init: 0x1000,
                play: 0x1001,
                subtune: 0,
            },
            render(1),
            Request::Report,
        ]);
        let t = traced(&replies[5]);
        assert_eq!(
            t.pages_written_outside,
            pages.into_iter().collect(),
            "${address:04X}"
        );
    }
}

#[test]
fn a_trace_starts_at_init_and_has_its_rules() {
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
        init(),
        Request::Report, // Init without a trace
        watch((0..17).collect()),
        watch(vec![0x2000, 0x2000]),
        watch(vec![0xD418]),
        watch(vec![0xD7FF]),
        Request::Trace {
            zero_page: true,
            outside: Some((0x2000, 0x1000)),
            watch: vec![],
        },
        // Only what was asked for is collected.
        watch(vec![0x2000]),
        init(),
        render(8),
        Request::Report,
        // The trace stays armed for every Init until replaced: a second Init is traced afresh.
        init(),
        render(8),
        Request::Report,
        // Everything off disarms it for the next Init.
        Request::Trace {
            zero_page: false,
            outside: None,
            watch: vec![],
        },
        init(),
        Request::Report,
    ]);
    assert_eq!(code(&replies[2]), ErrorCode::Sequence);
    assert_eq!(code(&replies[4]), ErrorCode::Sequence);
    for r in &replies[5..10] {
        assert_eq!(code(r), ErrorCode::Limit, "{r:?}");
    }
    let t = traced(&replies[13]);
    assert!(t.zero_page.is_empty() && t.pages_written_outside.is_empty());
    assert_eq!(t.watched[0].changes, [(7, 0x41)]);
    let again = traced(&replies[16]);
    assert_eq!(
        (again.watched[0].start, &again.watched[0].changes[..]),
        (0x40, &[(7, 0x41)][..])
    );
    assert_eq!(code(&replies[19]), ErrorCode::Sequence);
}

#[test]
fn the_sid_range_refused_for_watching_is_d400_to_d7ff() {
    for (address, refused) in [
        (0xD3FF, false),
        (0xD400, true),
        (0xD7FF, true),
        (0xD800, false),
    ] {
        let replies = run(&[
            hello(),
            Request::Trace {
                zero_page: false,
                outside: None,
                watch: vec![address],
            },
        ]);
        assert_eq!(replies[1] != Reply::Ok, refused, "${address:04X}");
    }
}

#[test]
fn each_init_starts_afresh_and_a_new_trace_waits_for_one() {
    let replies = run(&[
        hello(),
        Request::Load {
            address: 0x1000,
            data: tracer(),
        },
        trace_all(),
        init(),
        Request::Report,
        // An Init that runs only an RTS ($104D): nothing from the first is carried over.
        Request::Init {
            init: 0x104D,
            play: 0x104D,
            subtune: 0,
        },
        render(1),
        Request::Report,
        // A Trace while a timeline runs changes nothing about it until the next Init.
        init(),
        render(8),
        Request::Trace {
            zero_page: false,
            outside: None,
            watch: vec![],
        },
        render(8),
        Request::Report,
    ]);
    assert!(traced(&replies[4]).zero_page.contains(&0xFC));
    let fresh = traced(&replies[7]);
    assert!(
        fresh.zero_page.is_empty() && fresh.pages_written_outside.is_empty(),
        "{fresh:?}"
    );
    assert_eq!(fresh.watched[0].changes, []);
    let running = traced(&replies[12]);
    assert_eq!(running.watched[0].changes, [(7, 0x41), (15, 0x42)]);
    assert!(running.zero_page.contains(&0xFB));
}

#[test]
fn a_failed_init_has_no_trace_but_a_failed_play_keeps_its_own() {
    let jam = Request::Load {
        address: 0x1100,
        data: vec![0x02], // JAM
    };
    let replies = run(&[
        hello(),
        Request::Load {
            address: 0x1000,
            data: tracer(),
        },
        jam,
        trace_all(),
        Request::Init {
            init: 0x1100,
            play: 0x1003,
            subtune: 0,
        },
        Request::Report,
        Request::Init {
            init: 0x1000,
            play: 0x1100,
            subtune: 0,
        },
        render(1),
        Request::Report,
    ]);
    assert_eq!(code(&replies[4]), ErrorCode::CpuJam);
    assert_eq!(
        code(&replies[5]),
        ErrorCode::Sequence,
        "a failed Init leaves nothing to report"
    );
    assert_eq!(code(&replies[7]), ErrorCode::CpuJam);
    // What init collected before the failed play call can still be reported.
    let t = traced(&replies[8]);
    assert_eq!(t.zero_page, [0xFB, 0xFC].into());
    assert_eq!(t.pages_written_outside, [0x20, 0xC2].into());
}

#[test]
fn a_failed_init_after_a_good_one_leaves_nothing_and_a_failed_play_keeps_its_changes() {
    let replies = run(&[
        hello(),
        Request::Load {
            address: 0x1000,
            data: tracer(),
        },
        Request::Load {
            address: 0x1100,
            data: vec![0x02], // JAM
        },
        trace_all(),
        init(),
        Request::Init {
            init: 0x1100,
            play: 0x1003,
            subtune: 0,
        },
        Request::Report,
        // Eight good frames, then the play routine jams.
        init(),
        render(8),
        Request::Poke {
            address: 0x1030,
            value: 0x02,
        },
        render(1),
        Request::Report,
    ]);
    assert_eq!(code(&replies[5]), ErrorCode::CpuJam);
    assert_eq!(
        code(&replies[6]),
        ErrorCode::Sequence,
        "the latest Init failed"
    );
    assert_eq!(code(&replies[10]), ErrorCode::CpuJam);
    let t = traced(&replies[11]);
    assert_eq!(
        t.watched[0].changes,
        [(7, 0x41)],
        "changes before the failed call survive it"
    );
}
