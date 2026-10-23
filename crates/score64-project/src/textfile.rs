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

/// Whether `name` is a Windows device name (`con`, `prn`, `aux`, `nul`, `conin$`, `conout$`, `com0`–`com9`,
/// `lpt0`–`lpt9`, and `com`/`lpt` with `¹`, `²` or `³`), in any case and with any extension (`nul.json`): Windows
/// can't create a file or folder of that name.
pub(crate) fn is_device_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or("").to_ascii_lowercase();
    let numbered = |prefix: &str| {
        stem.strip_prefix(prefix).is_some_and(|n| {
            let mut c = n.chars();
            matches!(
                (c.next(), c.next()),
                (Some('0'..='9' | '¹' | '²' | '³'), None)
            )
        })
    };
    matches!(
        stem.as_str(),
        "con" | "prn" | "aux" | "nul" | "conin$" | "conout$"
    ) || numbered("com")
        || numbered("lpt")
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

/// A fresh, uniquely named temporary file beside `path` (`<name>.<pid>-<n>.part`, so two writers never share one)
/// holding `text`, synced to disk. Removed again if writing it fails.
fn write_part(path: &Path, text: &str) -> Result<PathBuf, (PathBuf, std::io::Error)> {
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
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&part)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()
    })();
    match written {
        Ok(()) => Ok(part),
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err((part, e))
        }
    }
}

/// Sync `path`'s folder, so a rename or link into it survives a power loss (Unix; elsewhere a no-op).
fn sync_parent(path: &Path) {
    #[cfg(unix)]
    if let Some(dir) = path.parent() {
        let dir = if dir.as_os_str().is_empty() {
            Path::new(".")
        } else {
            dir
        };
        if let Ok(d) = std::fs::File::open(dir) {
            let _ = d.sync_all();
        }
    }
}

/// Write `text` at `path`: to a temporary file beside it, synced to disk, then renamed over the target, which keeps
/// its permissions. A crash or power loss leaves the old file or the new one, never half of one; on Windows the
/// rename fails while another process holds the target open. The temporary file is removed when anything fails. The
/// error names the file it concerns.
pub(crate) fn write_atomic(path: &Path, text: &str) -> Result<(), (PathBuf, std::io::Error)> {
    let part = write_part(path, text)?;
    let done = (|| {
        if let Ok(old) = std::fs::metadata(path) {
            std::fs::set_permissions(&part, old.permissions()).map_err(|e| (part.clone(), e))?;
        }
        std::fs::rename(&part, path).map_err(|e| (path.to_path_buf(), e))
    })();
    if done.is_err() {
        let _ = std::fs::remove_file(&part);
    } else {
        sync_parent(path);
    }
    done
}

/// Create `path` holding `text`, atomically like [`write_atomic`], but only if nothing is there: the temporary file
/// is hard-linked into place, which fails with `AlreadyExists` even when another writer creates the file meanwhile.
/// Where hard links aren't supported, it falls back to checking first.
pub(crate) fn create_atomic(path: &Path, text: &str) -> Result<(), (PathBuf, std::io::Error)> {
    let part = write_part(path, text)?;
    let linked = match std::fs::hard_link(&part, path) {
        Err(e) if e.kind() == std::io::ErrorKind::Unsupported => {
            if path.exists() {
                Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    "already exists",
                ))
            } else {
                std::fs::rename(&part, path)
            }
        }
        other => other,
    };
    let _ = std::fs::remove_file(&part);
    linked.map_err(|e| (path.to_path_buf(), e))?;
    sync_parent(path);
    Ok(())
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
    fn creating_never_replaces_a_file() {
        let dir = std::env::temp_dir().join(format!("score64-create-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("score64.toml");
        create_atomic(&file, "first\n").unwrap();
        let (path, e) = create_atomic(&file, "second\n").unwrap_err();
        assert_eq!(
            (path, e.kind()),
            (file.clone(), std::io::ErrorKind::AlreadyExists)
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "first\n");
        let left: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(
            left,
            [std::ffi::OsString::from("score64.toml")],
            "no .part file left"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_rewrite_keeps_the_files_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("score64-perm-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("track.toml");
        write_atomic(&file, "a\n").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o640)).unwrap();
        write_atomic(&file, "b\n").unwrap();
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o640
        );
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
            "conin$",
            "CONOUT$.log",
            "com¹",
            "lpt³.txt",
        ] {
            assert!(is_device_name(name), "{name}");
        }
        for name in [
            "console", "com10", "com", "lpt", "conn.txt", "main", "conin", "com⁴",
        ] {
            assert!(!is_device_name(name), "{name}");
        }
    }
}
