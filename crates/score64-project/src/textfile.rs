//! What every project text file shares (ADR-014 §4): the schema version, names that are safe on every system,
//! positions for error messages, and atomic, synced writes.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The layout version every project file is read and written in.
pub(crate) const SCHEMA: u32 = 1;

/// A file's `schema`: absent means 1, and a newer one is an error rather than a partial read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub(crate) struct Schema;

impl TryFrom<u32> for Schema {
    type Error = String;
    fn try_from(v: u32) -> Result<Self, String> {
        if v == SCHEMA {
            Ok(Schema)
        } else {
            Err(format!(
                "schema {v} is not supported; this build reads {SCHEMA}"
            ))
        }
    }
}

impl From<Schema> for u32 {
    fn from(_: Schema) -> u32 {
        SCHEMA
    }
}

/// Whether `name` is a Windows device name (`con`, `prn`, `aux`, `nul`, `com0`–`com9`, `lpt0`–`lpt9`), in any case
/// and with any extension (`nul.json`): Windows can't create a file or folder of that name.
pub(crate) fn is_device_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or("").to_ascii_lowercase();
    matches!(stem.as_str(), "con" | "prn" | "aux" | "nul")
        || ((stem.starts_with("com") || stem.starts_with("lpt"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit())
}

/// The 1-based line and column (in characters) of byte `offset` in `text`.
pub(crate) fn line_col(text: &str, offset: usize) -> (usize, usize) {
    let before = &text[..text.floor_char_boundary(offset)];
    let line = before.matches('\n').count() + 1;
    let col = before[before.rfind('\n').map_or(0, |i| i + 1)..]
        .chars()
        .count()
        + 1;
    (line, col)
}

/// The lines of every `header` (such as `[[decision]]`), in order: the n-th item's header is the n-th.
pub(crate) fn headers(text: &str, header: &str) -> Vec<usize> {
    text.lines()
        .enumerate()
        .filter(|(_, l)| l.trim_start().starts_with(header))
        .map(|(i, _)| i + 1)
        .collect()
}

/// Write `text` at `path`: to a temporary file beside it (`<name>.<pid>-<n>.part`, so two writers never share one),
/// synced to disk, then renamed over the target. A crash or power loss leaves the old file or the new one, never half
/// of one; on Windows the rename fails while another process holds the target open. The temporary file is removed
/// when anything fails. The error names the file it concerns.
pub(crate) fn write_atomic(path: &Path, text: &str) -> Result<(), (PathBuf, std::io::Error)> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let mut part = path.as_os_str().to_owned();
    part.push(format!(
        ".{}-{}.part",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let part = PathBuf::from(part);
    let written = (|| {
        let mut f = std::fs::File::create(&part).map_err(|e| (part.clone(), e))?;
        f.write_all(text.as_bytes())
            .map_err(|e| (part.clone(), e))?;
        f.sync_all().map_err(|e| (part.clone(), e))?;
        std::fs::rename(&part, path).map_err(|e| (path.to_path_buf(), e))
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    written
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_write_leaves_the_old_file_and_no_part_file() {
        let dir = std::env::temp_dir().join(format!("score64-textfile-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // The target is a non-empty folder, so the rename over it fails.
        let target = dir.join("track.toml");
        std::fs::create_dir_all(target.join("inside")).unwrap();
        let (path, _) = write_atomic(&target, "x = 1\n").unwrap_err();
        assert_eq!(path, target);
        assert!(target.join("inside").is_dir());
        let left: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(
            left,
            [std::ffi::OsString::from("track.toml")],
            "no .part file left"
        );
        // And a good write replaces the old content whole.
        let file = dir.join("a.toml");
        write_atomic(&file, "old\n").unwrap();
        write_atomic(&file, "new\n").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "new\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn device_names_in_any_case_and_with_any_extension() {
        for name in [
            "con",
            "CON",
            "Nul.json",
            "aux.tar.gz",
            "com1",
            "LPT9.txt",
            "prn",
        ] {
            assert!(is_device_name(name), "{name}");
        }
        for name in ["console", "com10", "com", "lpt", "conn.txt", "main"] {
            assert!(!is_device_name(name), "{name}");
        }
    }
}
