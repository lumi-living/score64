//! A packed player as gt2reloc writes it (`.prg`, `.sid` or `.bin`): where its bytes load, the bytes themselves, and
//! the jump table at their start (`docs/reference/player.md`). The facts the budget gate (rule 8) reads from the file.

use crate::psid::{Psid, PsidError};

/// The file a player was packed into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Container {
    /// A C64 program: a little-endian load address, then the bytes.
    Prg,
    /// A PSID file; the bytes follow its header (and the load address, when the header leaves it at 0).
    Sid,
    /// The bytes alone. gt2reloc puts them at the player page it was given, so the caller supplies the load address.
    Bin {
        /// Where the bytes load.
        load: u16,
    },
}

/// The C64 bytes of a packed player and where they load.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Payload {
    /// Load address.
    pub load: u16,
    /// The bytes, without any file header.
    pub data: Vec<u8>,
}

/// Why a file is not a packed player.
#[derive(Debug, PartialEq, Eq)]
pub enum PayloadError {
    /// A `.prg` shorter than its load address.
    Short,
    /// The PSID header is unusable.
    Psid(PsidError),
    /// The PSID header's init or play address isn't the player's jump table at the load address.
    PsidEntries {
        /// The header's init address.
        init: u16,
        /// The header's play address.
        play: u16,
        /// The load address.
        load: u16,
    },
    /// The bytes run past `$FFFF`.
    PastEnd {
        /// Load address.
        load: u16,
        /// Byte count.
        len: usize,
    },
}

impl std::fmt::Display for PayloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PayloadError::Short => f.write_str("the program file is shorter than its load address"),
            PayloadError::Psid(e) => e.fmt(f),
            PayloadError::PsidEntries { init, play, load } => write!(
                f,
                "the PSID header's init ${init:04X} and play ${play:04X} are not the jump table at ${load:04X} and \
                 ${:04X}",
                load.wrapping_add(3)
            ),
            PayloadError::PastEnd { load, len } => {
                write!(f, "{len} bytes at ${load:04X} run past $FFFF")
            }
        }
    }
}

impl std::error::Error for PayloadError {}

impl Payload {
    /// The payload of a file packed into `container`.
    pub fn read(bytes: &[u8], container: Container) -> Result<Self, PayloadError> {
        let p = match container {
            Container::Prg => {
                let [lo, hi, rest @ ..] = bytes else {
                    return Err(PayloadError::Short);
                };
                Payload {
                    load: u16::from_le_bytes([*lo, *hi]),
                    data: rest.to_vec(),
                }
            }
            Container::Sid => {
                let psid = Psid::parse(bytes).map_err(PayloadError::Psid)?;
                let load = psid.load_address;
                if psid.init_address != load || psid.play_address != load.wrapping_add(3) {
                    return Err(PayloadError::PsidEntries {
                        init: psid.init_address,
                        play: psid.play_address,
                        load,
                    });
                }
                Payload {
                    load,
                    data: psid.data,
                }
            }
            Container::Bin { load } => Payload {
                load,
                data: bytes.to_vec(),
            },
        };
        if usize::from(p.load) + p.data.len() > 0x10000 {
            return Err(PayloadError::PastEnd {
                load: p.load,
                len: p.data.len(),
            });
        }
        Ok(p)
    }

    /// Where each `JMP abs` (`$4C`) at load+0 (init), +3 (play) and +6 (sound effect) goes; `None` for an entry that
    /// isn't one, or that runs off the end of the bytes.
    pub fn jump_table(&self) -> [Option<u16>; 3] {
        std::array::from_fn(|i| match self.data.get(3 * i..3 * i + 3) {
            Some(&[0x4C, lo, hi]) => Some(u16::from_le_bytes([lo, hi])),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A three-entry jump table and some code, at $1000.
    const PLAYER: [u8; 10] = [0x4C, 0x09, 0x10, 0x4C, 0x40, 0x10, 0x4C, 0xA0, 0x10, 0x60];

    fn psid(load_in_header: u16, init: u16, play: u16, data: &[u8]) -> Vec<u8> {
        let mut h = vec![0u8; 0x7C];
        h[0..4].copy_from_slice(b"PSID");
        h[4..6].copy_from_slice(&2u16.to_be_bytes());
        h[6..8].copy_from_slice(&0x7Cu16.to_be_bytes());
        h[8..10].copy_from_slice(&load_in_header.to_be_bytes());
        h[10..12].copy_from_slice(&init.to_be_bytes());
        h[12..14].copy_from_slice(&play.to_be_bytes());
        h[14..16].copy_from_slice(&1u16.to_be_bytes());
        h[16..18].copy_from_slice(&1u16.to_be_bytes());
        h.extend_from_slice(data);
        h
    }

    #[test]
    fn the_three_containers_give_the_same_payload() {
        let want = Payload {
            load: 0x1000,
            data: PLAYER.to_vec(),
        };
        let prg = [&[0x00, 0x10][..], &PLAYER].concat();
        assert_eq!(Payload::read(&prg, Container::Prg).unwrap(), want);
        // gt2reloc leaves the header's load address at 0 and puts it before the data.
        let sid = psid(0, 0x1000, 0x1003, &prg);
        assert_eq!(Payload::read(&sid, Container::Sid).unwrap(), want);
        let sid = psid(0x1000, 0x1000, 0x1003, &PLAYER);
        assert_eq!(Payload::read(&sid, Container::Sid).unwrap(), want);
        assert_eq!(
            Payload::read(&PLAYER, Container::Bin { load: 0x1000 }).unwrap(),
            want
        );
        assert_eq!(
            want.jump_table(),
            [Some(0x1009), Some(0x1040), Some(0x10A0)]
        );
    }

    #[test]
    fn a_jump_table_reads_only_jmp_abs() {
        let p = |data: &[u8]| Payload {
            load: 0x1000,
            data: data.to_vec(),
        };
        // No effect entry (packed without -D1): a JSR, then the bytes end inside the third entry.
        assert_eq!(
            p(&[0x4C, 0x09, 0x10, 0x20, 0x40, 0x10, 0x4C, 0xA0]).jump_table(),
            [Some(0x1009), None, None]
        );
        assert_eq!(p(&[]).jump_table(), [None; 3]);
        // JMP (ind) is $6C, not a jump table entry.
        assert_eq!(p(&[0x6C, 0x00, 0x10]).jump_table(), [None; 3]);
    }

    #[test]
    fn bad_files_are_refused() {
        assert_eq!(
            Payload::read(&[0x00], Container::Prg),
            Err(PayloadError::Short)
        );
        assert_eq!(
            Payload::read(&[0xFE, 0xFF, 0, 0, 0, 0], Container::Prg)
                .unwrap_err()
                .to_string(),
            "4 bytes at $FFFE run past $FFFF"
        );
        // Exactly to $FFFF is fine.
        assert!(Payload::read(&vec![0; 0x100], Container::Bin { load: 0xFF00 }).is_ok());
        assert!(matches!(
            Payload::read(&vec![0; 0x101], Container::Bin { load: 0xFF00 }),
            Err(PayloadError::PastEnd { len: 0x101, .. })
        ));
        let e = Payload::read(&psid(0x1000, 0x1000, 0x1006, &PLAYER), Container::Sid).unwrap_err();
        assert_eq!(
            e.to_string(),
            "the PSID header's init $1000 and play $1006 are not the jump table at $1000 and $1003"
        );
        // The init address is checked as well as the play address.
        assert!(matches!(
            Payload::read(&psid(0x1000, 0x1001, 0x1003, &PLAYER), Container::Sid),
            Err(PayloadError::PsidEntries { init: 0x1001, .. })
        ));
        assert!(matches!(
            Payload::read(b"RSID", Container::Sid),
            Err(PayloadError::Psid(PsidError::NotPsid))
        ));
    }

    #[test]
    fn psid_header_variants() {
        let want = Payload {
            load: 0x1000,
            data: PLAYER.to_vec(),
        };
        // Init 0 means the load address.
        assert_eq!(
            Payload::read(&psid(0x1000, 0, 0x1003, &PLAYER), Container::Sid).unwrap(),
            want
        );
        // A version 1 header is $76 bytes.
        let mut v1 = psid(0x1000, 0x1000, 0x1003, &[]);
        v1.truncate(0x76);
        v1[4..6].copy_from_slice(&1u16.to_be_bytes());
        v1[6..8].copy_from_slice(&0x76u16.to_be_bytes());
        v1.extend_from_slice(&PLAYER);
        assert_eq!(Payload::read(&v1, Container::Sid).unwrap(), want);
    }
}
