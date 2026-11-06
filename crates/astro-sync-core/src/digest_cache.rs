//! The SHA-256 of a file, kept by (path, size, modification time) so it is
//! computed once (without it, every action redid the walk and all the
//! hashes). A night weighs
//! tens of gigabytes: the "Files" step, the summary, the sending and the resume
//! all need the digests, and each used to read every byte again.
//!
//! A row is trusted only while the file keeps its exact size and
//! modification time (nanoseconds where the system has them); any change
//! and the file is read again. The cache lives in Astro Sync's own data
//! folder (`digest-cache.sqlite`), never in the person's tree. Losing it
//! costs only time.

use rusqlite::{Connection, OptionalExtension, params};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DigestCacheError {
    #[error("could not open the digest cache at {path}: {source}")]
    Open {
        path: PathBuf,
        source: rusqlite::Error,
    },
    #[error("digest cache error: {0}")]
    Database(#[from] rusqlite::Error),
}

/// What identifies one state of a file: its size and its modification time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStamp {
    pub size: u64,
    /// Nanoseconds since the Unix epoch (0 when the system gives none).
    pub modified_ns: i64,
}

impl FileStamp {
    /// The stamp of `path` (links followed).
    pub fn of(path: &Path) -> std::io::Result<Self> {
        Ok(Self::from_metadata(&fs::metadata(path)?))
    }

    #[must_use]
    pub fn from_metadata(metadata: &fs::Metadata) -> Self {
        let modified_ns = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |duration| {
                i64::try_from(duration.as_nanos()).unwrap_or(i64::MAX)
            });
        Self {
            size: metadata.len(),
            modified_ns,
        }
    }
}

/// The cache. Shareable between threads (one connection behind a lock: a
/// lookup is a few microseconds next to reading a file).
pub struct DigestCache {
    connection: Mutex<Connection>,
}

impl std::fmt::Debug for DigestCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DigestCache")
    }
}

const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS digests (
    path TEXT PRIMARY KEY,
    size INTEGER NOT NULL,
    modified_ns INTEGER NOT NULL,
    sha256 TEXT NOT NULL
)";

impl DigestCache {
    /// Opens (or creates) the cache file at `path`.
    pub fn open(path: &Path) -> Result<Self, DigestCacheError> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            let _ = fs::create_dir_all(parent);
        }
        let connection = Connection::open(path).map_err(|source| DigestCacheError::Open {
            path: path.to_path_buf(),
            source,
        })?;
        connection.execute(SCHEMA, [])?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    /// A cache that lives as long as the value (tests, a one-off command).
    pub fn in_memory() -> Result<Self, DigestCacheError> {
        let connection = Connection::open_in_memory()?;
        connection.execute(SCHEMA, [])?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    fn key(path: &Path) -> String {
        // The file the path leads to: a night reached through a link and the
        // same night reached directly share their digests.
        fs::canonicalize(path)
            .unwrap_or_else(|_| path.to_path_buf())
            .to_string_lossy()
            .into_owned()
    }

    /// The digest recorded for `path` in exactly the state `stamp`, if any.
    pub fn get(&self, path: &Path, stamp: FileStamp) -> Result<Option<String>, DigestCacheError> {
        let connection = self.connection.lock().expect("digest cache lock");
        let found = connection
            .query_row(
                "SELECT sha256 FROM digests WHERE path = ?1 AND size = ?2 AND modified_ns = ?3",
                params![
                    Self::key(path),
                    i64::try_from(stamp.size).unwrap_or(i64::MAX),
                    stamp.modified_ns
                ],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        Ok(found)
    }

    /// Records the digest of `path` in the state `stamp` (replacing any
    /// older state of the same file).
    pub fn put(&self, path: &Path, stamp: FileStamp, sha256: &str) -> Result<(), DigestCacheError> {
        let connection = self.connection.lock().expect("digest cache lock");
        connection.execute(
            "INSERT INTO digests (path, size, modified_ns, sha256) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(path) DO UPDATE SET size = excluded.size,
                 modified_ns = excluded.modified_ns, sha256 = excluded.sha256",
            params![
                Self::key(path),
                i64::try_from(stamp.size).unwrap_or(i64::MAX),
                stamp.modified_ns,
                sha256
            ],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    #[test]
    fn a_digest_is_found_only_for_the_same_size_and_time() {
        let dir = TempDir::new("digest-cache-stamp");
        let file = dir.path().join("final.tif");
        fs::write(&file, b"image").unwrap();
        let cache = DigestCache::open(&dir.path().join("cache.sqlite")).unwrap();
        let stamp = FileStamp::of(&file).unwrap();
        assert_eq!(cache.get(&file, stamp).unwrap(), None);
        cache.put(&file, stamp, "abc").unwrap();
        assert_eq!(cache.get(&file, stamp).unwrap().as_deref(), Some("abc"));
        let changed = FileStamp {
            size: stamp.size + 1,
            ..stamp
        };
        assert_eq!(cache.get(&file, changed).unwrap(), None);
        let touched = FileStamp {
            modified_ns: stamp.modified_ns + 1,
            ..stamp
        };
        assert_eq!(cache.get(&file, touched).unwrap(), None);
    }

    #[test]
    fn the_cache_survives_a_reopening() {
        let dir = TempDir::new("digest-cache-reopen");
        let file = dir.path().join("a.fits");
        fs::write(&file, b"x").unwrap();
        let stamp = FileStamp::of(&file).unwrap();
        let path = dir.path().join("cache.sqlite");
        DigestCache::open(&path)
            .unwrap()
            .put(&file, stamp, "d1")
            .unwrap();
        let reopened = DigestCache::open(&path).unwrap();
        assert_eq!(reopened.get(&file, stamp).unwrap().as_deref(), Some("d1"));
    }
}
