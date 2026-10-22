//! The real helper binary over real pipes: `score64-emu serve` driven with the shared codec.

use std::io::Write as _;
use std::process::{Command, Stdio};

use score64_formats::emu_protocol::{self as proto, Reply, Request};

#[test]
fn serve_over_pipes() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_score64-emu"))
        .arg("serve")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("start score64-emu serve");
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let mut ask = |req: Request| -> Reply {
        stdin.write_all(&req.encode()).unwrap();
        stdin.flush().unwrap();
        Reply::decode(&proto::read_body(&mut stdout).unwrap().expect("a reply")).unwrap()
    };

    assert!(matches!(
        ask(Request::Hello {
            version: proto::VERSION,
            sample_rate: 44_100
        }),
        Reply::Ready {
            sample_rate: 44_100,
            ..
        }
    ));
    // Play: sawtooth on voice 1 at full volume.
    let mut player = vec![0x60, 0xEA, 0xEA];
    player.extend_from_slice(&[
        0xA9, 0x0F, 0x8D, 0x18, 0xD4, 0xA9, 0x11, 0x8D, 0x01, 0xD4, 0xA9, 0xF0, 0x8D, 0x06, 0xD4,
        0xA9, 0x21, 0x8D, 0x04, 0xD4, 0x60,
    ]);
    assert_eq!(
        ask(Request::Load {
            address: 0x1000,
            data: player
        }),
        Reply::Ok
    );
    assert_eq!(
        ask(Request::Init {
            init: 0x1000,
            play: 0x1003,
            subtune: 0
        }),
        Reply::Ok
    );
    let mut total = 0;
    for _ in 0..5 {
        match ask(Request::Render {
            frames: 10,
            writes: true,
        }) {
            Reply::Rendered {
                frames: 10,
                samples,
                writes,
                ..
            } => {
                total += samples.len();
                assert_eq!(writes.len(), 40);
            }
            other => panic!("{other:?}"),
        }
    }
    // 50 frames in five chunks: the same 0.9975 s as one render (44.1 kHz).
    assert!((43_985..=44_005).contains(&total), "{total} samples");
    assert_eq!(ask(Request::Quit), Reply::Ok);
    drop(stdin);
    assert!(child.wait().unwrap().success());
}
