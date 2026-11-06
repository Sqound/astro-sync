//! Test-only helper for a unique, self-cleaning temporary directory.
//!
//! The plan's suggested dev-dependency for this (`tempfile`) is not
//! available in this task's offline build (it, and its own dependencies,
//! are not present in the local cargo registry cache, and this task runs
//! with no network access) -- see `README.md` for the recorded
//! deviation. This is the minimal stand-in: a directory under
//! [`std::env::temp_dir`], named uniquely with the process id and a
//! monotonically increasing counter, removed on drop.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

pub struct TempDir(PathBuf);

impl TempDir {
    /// Creates a fresh, empty directory under the system temp directory.
    /// `label` is only for human-readable disambiguation if one is ever
    /// left behind (e.g. by a test that panics before cleanup runs).
    pub fn new(label: &str) -> Self {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "astro-sync-core-test-{label}-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("create temp test directory");
        Self(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A synthetic FITS primary header (no pixel): each `(keyword, value)` is a
/// card, the value written as given (quote strings yourself: `'LIGHT'`),
/// closed by `END` and padded to whole 2880-byte blocks, then `pixels`
/// bytes standing for the data unit.
pub fn fits_bytes(cards: &[(&str, &str)], pixels: usize) -> Vec<u8> {
    let mut out = Vec::new();
    for (keyword, value) in cards {
        let mut card = format!("{keyword:<8}= {value}").into_bytes();
        card.resize(80, b' ');
        out.extend_from_slice(&card);
    }
    let mut end = b"END".to_vec();
    end.resize(80, b' ');
    out.extend_from_slice(&end);
    let padded = out.len().div_ceil(2880) * 2880;
    out.resize(padded, b' ');
    out.extend(std::iter::repeat_n(0_u8, pixels));
    out
}
