//! The fixture song packed by the real gt2reloc into each format reads back as the same bytes at the player page, of
//! the size gt2reloc reported, with a jump table into the tune (#51). Runs when gt2reloc is available; otherwise it
//! skips visibly (CI has no gt2reloc).

use std::io::Write as _;
use std::path::PathBuf;

use score64_adapters::gt2reloc::{Gt2Reloc, PackOptions, ZeroPage};

#[test]
fn every_format_holds_the_same_player() {
    let Ok(packer) = Gt2Reloc::locate("2.73") else {
        let _ = writeln!(
            std::io::stderr(),
            "SKIPPED every_format_holds_the_same_player: needs gt2reloc on PATH"
        );
        return;
    };
    let song = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/gt2-original/sixteen-steps.sng");
    let dir = std::env::temp_dir().join(format!("score64-pack-facts-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for effects in [true, false] {
        let options = PackOptions {
            sound_effects: effects,
            ..PackOptions::new(0x20, ZeroPage::new(0xFB).unwrap())
        };
        let payloads: Vec<_> = ["sid", "prg", "bin"]
            .iter()
            .map(|ext| {
                let packed = packer
                    .pack(&song, &dir.join(format!("tune.{ext}")), &options)
                    .unwrap();
                assert_eq!(packed.start, 0x2000);
                (packed.size, packed.payload().unwrap())
            })
            .collect();
        let (size, first) = &payloads[0];
        assert!(
            payloads.iter().all(|(s, p)| s == size && p == first),
            "formats differ"
        );
        assert_eq!(first.load, 0x2000);
        let end = 0x2000 + *size;
        let jumps = first.jump_table();
        let inside = |j: Option<u16>| j.is_some_and(|to| (0x2000..end).contains(&u32::from(to)));
        assert!(inside(jumps[0]) && inside(jumps[1]), "{jumps:04X?}");
        // Without -D1 the third entry is not a jump into the tune.
        assert_eq!(inside(jumps[2]), effects, "{jumps:04X?}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
