//! Wrap a gt2reloc `-D1` PSID so it fires sound effects itself, the way a game would, for players that can't call
//! the effect entry point (sidplayfp) and for the helper alike.
//!
//! A hand-assembled copy of `crates/score64-emu/spikes/packed-player/sfx-psid.asm`: wrapper code at `$0E00` (init) and
//! `$0E20` (play), up to three effects at `$0E80`/`$0F00`/`$0F80`, the player at `$1000`. Every 37 frames it fires the
//! next effect on voice 3 (X = 14) through the player's sound-effect entry, start+6: the schedule of
//! `score64-emu render --effect … --effect-every 37 --effect-voice 3`, whose SID writes it matches frame for frame.

const PLAYER: u16 = 0x1000;
const LOAD: u16 = 0x0E00;
const FX_BASE: u16 = 0x0E80;
const FX_STRIDE: u16 = 0x80;
/// Frames between effects.
pub const FX_EVERY: u8 = 37;
const FX_CHANNEL: u8 = 14;
const FIRST: u16 = 0x0E54;
const FX_COUNT: u16 = 0x0E55;
const FX_INDEX: u16 = 0x0E56;
const FX_LO: u16 = 0x0E57;
const FX_HI: u16 = 0x0E5A;

fn w(addr: u16) -> [u8; 2] {
    addr.to_le_bytes()
}

/// The wrapper's code and data from `$0E00` up to the player, with room for the effects, for `count` effects.
fn code(count: u8) -> Vec<u8> {
    let mut image = vec![0; usize::from(PLAYER - LOAD)];
    let mut put = |at: u16, bytes: &[u8]| {
        let i = usize::from(at - LOAD);
        image[i..i + bytes.len()].copy_from_slice(bytes);
    };
    // init, $0E00: A = subtune, passed on to the player.
    put(
        0x0E00,
        &[
            [0xA2, 0x01].as_slice(), // LDX #1
            &[0x8E],
            &w(FIRST),         // STX first
            &[0xA2, FX_EVERY], // LDX #37
            &[0x8E],
            &w(FX_COUNT),  // STX fx_count
            &[0xA2, 0x00], // LDX #0
            &[0x8E],
            &w(FX_INDEX), // STX fx_index
            &[0x4C],
            &w(PLAYER), // JMP player init
        ]
        .concat(),
    );
    // play, $0E20: the first frame plays music only; then every 37 frames the next effect, then the music.
    let play = [
        [0xAD].as_slice(),
        &w(FIRST),     // 0E20 LDA first
        &[0xF0, 0x08], // 0E23 BEQ countdown ($0E2D)
        &[0xA9, 0x00, 0x8D],
        &w(FIRST), // 0E25 LDA #0 / STA first
        &[0x4C],
        &w(PLAYER + 3), // 0E2A JMP player play
        &[0xCE],
        &w(FX_COUNT),  // 0E2D countdown: DEC fx_count
        &[0xD0, 0x1F], // 0E30 BNE done ($0E51)
        &[0xA9, FX_EVERY, 0x8D],
        &w(FX_COUNT), // 0E32 LDA #37 / STA fx_count
        &[0xAE],
        &w(FX_INDEX), // 0E37 LDX fx_index
        &[0xBC],
        &w(FX_HI), // 0E3A LDY fx_hi,X
        &[0xBD],
        &w(FX_LO),                  // 0E3D LDA fx_lo,X
        &[0x48, 0xE8, 0xE0, count], // 0E40 PHA / INX / CPX #count
        &[0xD0, 0x02, 0xA2, 0x00],  // 0E44 BNE skip / LDX #0
        &[0x8E],
        &w(FX_INDEX),              // 0E48 skip: STX fx_index
        &[0x68, 0xA2, FX_CHANNEL], // 0E4B PLA / LDX #14
        &[0x20],
        &w(PLAYER + 6), // 0E4E JSR player sound effect
        &[0x4C],
        &w(PLAYER + 3), // 0E51 done: JMP player play
    ]
    .concat();
    assert_eq!(
        0x0E20 + play.len() as u16,
        FIRST,
        "the play routine ends where its data starts"
    );
    put(0x0E20, &play);
    let addr = |i: u16| FX_BASE + i * FX_STRIDE;
    let mut data = vec![1, FX_EVERY, 0];
    data.extend((0..3u16).map(|i| {
        if i < u16::from(count) {
            addr(i).to_le_bytes()[0]
        } else {
            0
        }
    }));
    data.extend((0..3u16).map(|i| {
        if i < u16::from(count) {
            addr(i).to_le_bytes()[1]
        } else {
            0
        }
    }));
    put(FIRST, &data);
    assert!(FIRST + data.len() as u16 <= FX_BASE);
    image
}

/// `tune` (a gt2reloc PSID with its player at `$1000`, packed with `-D1`) wrapped with `effects` (one to three, each
/// GoatTracker sound-effect data of at most 128 bytes).
pub fn wrap(tune: &[u8], effects: &[Vec<u8>]) -> Result<Vec<u8>, String> {
    if !(1..=3).contains(&effects.len()) {
        return Err(format!(
            "{} effects; the wrapper takes one to three",
            effects.len()
        ));
    }
    if tune.get(..4) != Some(b"PSID") {
        return Err("not a PSID file".into());
    }
    let be = |at: usize| u16::from_be_bytes([tune[at], tune[at + 1]]);
    let (offset, load, songs) = (usize::from(be(6)), be(8), be(14));
    let mut body = tune
        .get(offset..)
        .ok_or("the PSID ends inside its header")?;
    let load = if load == 0 {
        let at = u16::from_le_bytes([body[0], body[1]]);
        body = &body[2..];
        at
    } else {
        load
    };
    if load != PLAYER {
        return Err(format!("the player is at ${load:04X}, not $1000"));
    }
    let mut image = code(effects.len() as u8);
    for (i, fx) in effects.iter().enumerate() {
        if fx.len() > usize::from(FX_STRIDE) {
            return Err(format!(
                "effect {} is {} bytes; at most 128",
                i + 1,
                fx.len()
            ));
        }
        let at = usize::from(FX_BASE - LOAD) + i * usize::from(FX_STRIDE);
        image[at..at + fx.len()].copy_from_slice(fx);
    }
    image.extend_from_slice(body);
    let text = |at: usize| {
        tune[at..at + 32]
            .iter()
            .take_while(|&&b| b != 0)
            .copied()
            .collect::<Vec<u8>>()
    };
    let mut name = text(0x16);
    name.extend_from_slice(b" + effects");
    let mut out = Vec::with_capacity(0x7E + image.len());
    out.extend_from_slice(b"PSID");
    for v in [2u16, 0x7C, 0, LOAD, 0x0E20, songs, 1] {
        out.extend_from_slice(&v.to_be_bytes());
    }
    out.extend_from_slice(&0u32.to_be_bytes()); // speed: every subtune on the frame interrupt
    for field in [name, text(0x36), text(0x56)] {
        let mut f = field;
        f.resize(32, 0);
        f.truncate(32);
        out.extend_from_slice(&f);
    }
    out.extend_from_slice(&0x0014u16.to_be_bytes()); // flags: PAL, 6581
    out.extend_from_slice(&[0, 0, 0, 0]); // no relocation, no extra SIDs
    assert_eq!(out.len(), 0x7C);
    out.extend_from_slice(&LOAD.to_le_bytes());
    out.extend_from_slice(&image);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_play_routine_matches_the_assembly_listing() {
        let image = code(3);
        let at = |a: u16| usize::from(a - LOAD);
        // Each branch lands on its label: countdown $0E2D, done $0E51, skip $0E48.
        assert_eq!(&image[at(0x0E23)..at(0x0E25)], &[0xF0, 0x08]);
        assert_eq!(0x0E25 + 0x08, 0x0E2D);
        assert_eq!(image[at(0x0E2D)], 0xCE, "DEC fx_count at countdown");
        assert_eq!(0x0E32 + 0x1F, 0x0E51);
        assert_eq!(
            &image[at(0x0E51)..at(0x0E54)],
            &[0x4C, 0x03, 0x10],
            "JMP play at done"
        );
        assert_eq!(0x0E46 + 0x02, 0x0E48);
        assert_eq!(image[at(0x0E48)], 0x8E, "STX fx_index at skip");
        assert_eq!(
            &image[at(0x0E4E)..at(0x0E51)],
            &[0x20, 0x06, 0x10],
            "JSR the effect entry"
        );
        // Data: first, count, index, then the effects' addresses, low bytes and high bytes.
        assert_eq!(
            &image[at(FIRST)..at(FIRST) + 9],
            &[1, 37, 0, 0x80, 0x00, 0x80, 0x0E, 0x0F, 0x0F]
        );
    }

    #[test]
    fn a_tune_is_wrapped_after_its_header() {
        let mut tune = b"PSID".to_vec();
        for v in [2u16, 0x7C, 0, 0x1000, 0x1003, 2, 1] {
            tune.extend_from_slice(&v.to_be_bytes());
        }
        tune.extend_from_slice(&0u32.to_be_bytes());
        let mut name = b"song".to_vec();
        name.resize(32, 0);
        tune.extend_from_slice(&name);
        tune.extend_from_slice(&[0; 64]);
        tune.extend_from_slice(&[0, 0x14, 0, 0, 0, 0]);
        tune.extend_from_slice(&[0x00, 0x10, 0x4C, 0x00, 0x20]); // load address $1000, then the player
        let fx = vec![vec![1, 2, 3], vec![4], vec![5, 6]];
        let out = wrap(&tune, &fx).unwrap();
        assert_eq!(&out[0x0A..0x0C], &[0x0E, 0x00], "init $0E00");
        assert_eq!(&out[0x0C..0x0E], &[0x0E, 0x20], "play $0E20");
        assert_eq!(&out[0x0E..0x10], &[0x00, 0x02], "both subtunes");
        assert_eq!(&out[0x16..0x16 + 14], b"song + effects");
        let image = &out[0x7E..];
        assert_eq!(&image[0x80..0x83], &[1, 2, 3], "effect 1 at $0E80");
        assert_eq!(image[0x100], 4, "effect 2 at $0F00");
        assert_eq!(
            &image[0x200..0x203],
            &[0x4C, 0x00, 0x20],
            "the player at $1000"
        );
        assert!(wrap(&tune, &[]).is_err());
        assert!(wrap(&tune, &[vec![0; 129]]).is_err());
    }
}
