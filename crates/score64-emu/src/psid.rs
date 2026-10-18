//! PSID files, as written by gt2reloc. Only what the helper needs: where the data loads and where init and play are.

/// A parsed PSID file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Psid {
    /// Header version (1–4).
    pub version: u16,
    /// Where the C64 data loads.
    pub load_address: u16,
    /// Init routine (A = subtune).
    pub init_address: u16,
    /// Play routine, called once per frame.
    pub play_address: u16,
    /// Number of subtunes.
    pub songs: u16,
    /// Default subtune, 1-based.
    pub start_song: u16,
    /// Song name from the header.
    pub name: String,
    /// The C64 data, without the load address.
    pub data: Vec<u8>,
}

/// Why a file is not a PSID the helper can run.
#[derive(Debug, PartialEq, Eq)]
pub enum PsidError {
    /// Too short or wrong magic.
    NotPsid,
    /// RSID files expect a real C64 environment (KERNAL, CIA, VIC interrupts), which the helper does not emulate.
    Rsid,
    /// The header is inconsistent.
    Malformed(&'static str),
    /// Play address 0: the tune installs its own interrupt, which the helper does not emulate.
    InstallsOwnInterrupt,
}

impl std::fmt::Display for PsidError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PsidError::NotPsid => f.write_str("not a PSID file"),
            PsidError::Rsid => f.write_str("RSID files are not supported (they need a full C64)"),
            PsidError::Malformed(why) => write!(f, "malformed PSID header: {why}"),
            PsidError::InstallsOwnInterrupt => f.write_str(
                "play address is 0: the tune installs its own interrupt, which is not emulated",
            ),
        }
    }
}

impl std::error::Error for PsidError {}

fn be16(bytes: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([bytes[at], bytes[at + 1]])
}

impl Psid {
    /// Parse a PSID file.
    pub fn parse(bytes: &[u8]) -> Result<Self, PsidError> {
        if bytes.len() < 0x76 {
            return Err(PsidError::NotPsid);
        }
        match &bytes[0..4] {
            b"PSID" => {}
            b"RSID" => return Err(PsidError::Rsid),
            _ => return Err(PsidError::NotPsid),
        }
        let version = be16(bytes, 0x04);
        if !(1..=4).contains(&version) {
            return Err(PsidError::Malformed("unknown version"));
        }
        let data_offset = usize::from(be16(bytes, 0x06));
        if data_offset > bytes.len() || data_offset < 0x76 {
            return Err(PsidError::Malformed("data offset out of range"));
        }
        let mut data = &bytes[data_offset..];
        let mut load_address = be16(bytes, 0x08);
        if load_address == 0 {
            if data.len() < 2 {
                return Err(PsidError::Malformed("missing load address"));
            }
            load_address = u16::from_le_bytes([data[0], data[1]]);
            data = &data[2..];
        }
        let init_address = match be16(bytes, 0x0A) {
            0 => load_address,
            a => a,
        };
        let play_address = be16(bytes, 0x0C);
        if play_address == 0 {
            return Err(PsidError::InstallsOwnInterrupt);
        }
        let name_bytes = &bytes[0x16..0x36];
        let name_end = name_bytes
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(name_bytes.len());
        Ok(Self {
            version,
            load_address,
            init_address,
            play_address,
            songs: be16(bytes, 0x0E),
            start_song: be16(bytes, 0x10),
            name: name_bytes[..name_end]
                .iter()
                .map(|&b| char::from(b))
                .collect(),
            data: data.to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(magic: &[u8; 4], load: u16, init: u16, play: u16) -> Vec<u8> {
        let mut h = vec![0u8; 0x7C];
        h[0..4].copy_from_slice(magic);
        h[4..6].copy_from_slice(&2u16.to_be_bytes());
        h[6..8].copy_from_slice(&0x7Cu16.to_be_bytes());
        h[8..10].copy_from_slice(&load.to_be_bytes());
        h[10..12].copy_from_slice(&init.to_be_bytes());
        h[12..14].copy_from_slice(&play.to_be_bytes());
        h[14..16].copy_from_slice(&1u16.to_be_bytes());
        h[16..18].copy_from_slice(&1u16.to_be_bytes());
        h[0x16..0x1A].copy_from_slice(b"Test");
        h
    }

    #[test]
    fn load_address_from_data() {
        let mut f = header(b"PSID", 0, 0x1000, 0x1003);
        f.extend_from_slice(&[0x00, 0x10, 0x4C, 0x00, 0x10]);
        let p = Psid::parse(&f).unwrap();
        assert_eq!(
            (p.load_address, p.init_address, p.play_address),
            (0x1000, 0x1000, 0x1003)
        );
        assert_eq!(p.data, [0x4C, 0x00, 0x10]);
        assert_eq!(p.name, "Test");
    }

    #[test]
    fn rejects_rsid_and_own_interrupt() {
        assert_eq!(
            Psid::parse(&header(b"RSID", 0x1000, 0x1000, 0)),
            Err(PsidError::Rsid)
        );
        assert_eq!(
            Psid::parse(&header(b"PSID", 0x1000, 0x1000, 0)),
            Err(PsidError::InstallsOwnInterrupt)
        );
    }
}
