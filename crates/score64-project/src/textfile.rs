//! What every project text file shares (ADR-014 §4): positions for error messages, and atomic, synced writes.

use std::path::{Path, PathBuf};

/// The 1-based line and column of byte `offset` in `text`.
pub(crate) fn line_col(text: &str, offset: usize) -> (usize, usize) {
    let before = &text[..offset.min(text.len())];
    let line = before.matches('\n').count() + 1;
    let col = before
        .rfind('\n')
        .map_or(before.len(), |i| before.len() - i - 1)
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
