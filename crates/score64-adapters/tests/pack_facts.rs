//! The fixture song packed by the real gt2reloc into each format reads back as the same bytes at the player page, of
//! the size gt2reloc reported, with a jump table into the tune (#51). Runs when gt2reloc is available; otherwise it
//! skips visibly (CI has no gt2reloc).

use std::collections::BTreeSet;
use std::io::Write as _;
use std::path::PathBuf;

use score64_adapters::gt2reloc::{Gt2Reloc, PackOptions, ZeroPage};
use score64_core::{PackBudget, PackFacts, check_pack};

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
                // The containers' layouts (docs/reference/player.md): the bytes, plus a load address, plus a PSID
                // header of $7C bytes whose own load address is 0.
                let bytes = std::fs::read(&packed.path).unwrap();
                let header = match *ext {
                    "bin" => 0,
                    "prg" => 2,
                    _ => 0x7C + 2,
                };
                assert_eq!(bytes.len() as u32, packed.size + header, "{ext}");
                if *ext == "sid" {
                    assert_eq!(
                        &bytes[4..10],
                        [0x00, 0x02, 0x00, 0x7C, 0x00, 0x00],
                        "version 2, data at $7C, load 0"
                    );
                    assert_eq!(
                        &bytes[10..14],
                        [0x20, 0x00, 0x20, 0x03],
                        "init $2000, play $2003"
                    );
                    assert_eq!(
                        &bytes[0x7C..0x7E],
                        [0x00, 0x20],
                        "the load address first in the data"
                    );
                }
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
        // Without -D1 there is no third entry: start+6 isn't a JMP at all.
        assert_eq!(inside(jumps[2]), effects, "{jumps:04X?}");
        assert_eq!(jumps[2].is_none(), !effects, "{jumps:04X?}");
        // The measured facts pass rule 8's byte checks for a window at $2000, with the effect entry required only when
        // packed for it. (The zero-page and outside-write sets come from the helper's trace, which isn't run here, so
        // the missing trace is the one problem.)
        let facts = PackFacts {
            start: first.load,
            size: *size,
            jump_table: jumps,
            zero_page_used: BTreeSet::new(),
            pages_written_outside: BTreeSet::new(),
            traced_window: None,
            max_play_cycles: None,
        };
        let budget = |sound_effects| PackBudget {
            first: 0x2000,
            last: 0x2FFF,
            zero_page: 0xFB,
            sound_effects,
            max_play_cycles: None,
        };
        let messages = |b| {
            check_pack(&facts, &b)
                .into_iter()
                .map(|p| p.message)
                .collect::<Vec<_>>()
        };
        let untraced = "the player wasn't traced, so its zero page and its writes outside the window weren't measured";
        assert_eq!(messages(budget(effects)), [untraced]);
        if !effects {
            let m = messages(budget(true));
            assert_eq!(m.len(), 2, "{m:?}");
            assert!(m[0].contains("sound-effect entry"), "{}", m[0]);
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}
