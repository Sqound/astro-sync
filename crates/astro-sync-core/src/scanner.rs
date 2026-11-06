//! Read-only walk of a folder: every candidate file is classified by
//! extension, hashed with SHA-256, and recorded with its exact byte size.
//! Nothing here ever opens a file for writing, renames, moves, or deletes
//! anything -- the invariant of all of Astro Sync (`crate`'s module doc).
//!
//! **Symbolic links are followed**: a night's `M31/lights` may be a link to a folder of an external
//! disk, and the frames there are the night's. A link's *target* is read
//! and hashed. A loop of links is detected and reported, never walked
//! twice; a link whose target is missing ("Link to a missing disk":
//! the disk is not plugged in) is reported and skipped. On Windows, NTFS
//! symbolic links and junctions are followed the same way; a `.lnk`
//! shortcut is an ordinary file of its own and is never followed.
//!
//! The digests can come from a [`DigestCache`] (path, size, modification
//! time), and a walk reports its progress and stops on a [`CancelFlag`]
//! (`crate::local_work`).

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use thiserror::Error;
use walkdir::WalkDir;

use crate::digest_cache::{DigestCache, FileStamp};
use crate::local_work::{CancelFlag, WorkKind, WorkProgress};
use crate::paths::to_relative_slash_string;

/// Name of the directory the package producer writes
/// (`crate::package::PACKAGE_VERSION`'s output). The scanner skips a
/// directory with this exact name wherever it appears in the walked tree,
/// so re-scanning a folder that already holds a previously produced package
/// (e.g. copied there by hand) never folds that package's own files back in
/// as if they were fresh captures.
pub const PACKAGE_OUTPUT_DIR_NAME: &str = "astro-session-package";

/// The classification buckets: conventional image formats, camera RAW files
/// (never mistaken for a picture: read as poses, their metadata by `crate::raw_header`, their pixels by Siril), videos
/// (AVI, SER…: not processed), text/CSV logs, FITS frames, XISF frames (a frame format of its own, read by `crate::xisf`),
/// and everything else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FileClass {
    Image,
    /// A camera RAW file (`cr2`, `nef`, `arw`, `dng`…): a pose, its metadata
    /// read by `crate::raw_header`.
    Raw,
    /// A video (`avi`, `ser`, `mp4`, `mov`, `mkv`): not processed here.
    Video,
    Log,
    Fits,
    Xisf,
    Other,
}

impl FileClass {
    /// Classifies a file by its extension (case-insensitive, without the
    /// leading dot). Unknown or missing extensions classify as `Other`.
    #[must_use]
    pub fn classify(extension: &str) -> Self {
        match extension.to_ascii_lowercase().as_str() {
            "jpg" | "jpeg" | "png" | "tif" | "tiff" => FileClass::Image,
            "cr2" | "cr3" | "nef" | "arw" | "dng" | "raf" | "pef" | "orf" | "rw2" => FileClass::Raw,
            "avi" | "ser" | "mp4" | "mov" | "mkv" => FileClass::Video,
            "log" | "txt" | "csv" => FileClass::Log,
            "fit" | "fits" | "fts" => FileClass::Fits,
            "xisf" => FileClass::Xisf,
            _ => FileClass::Other,
        }
    }

    /// A frame (FITS, XISF or a camera RAW file): never sent, read for its
    /// header only.
    #[must_use]
    pub fn is_frame(self) -> bool {
        matches!(self, FileClass::Fits | FileClass::Xisf | FileClass::Raw)
    }
}

/// One file found under the scanned root.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannedFile {
    /// Forward-slash path relative to the scanned root.
    pub relative_path: String,
    pub class: FileClass,
    pub bytes: u64,
    /// Lowercase hex-encoded SHA-256 digest of the file's contents.
    pub sha256: String,
}

/// What went wrong with a symbolic link met during a walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LinkProblem {
    /// The link's target does not exist: "Link to a missing disk".
    MissingTarget,
    /// The link leads back to one of its own parents.
    Loop,
}

/// A symbolic link the walk could not follow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkIssue {
    /// Forward-slash path of the link, relative to the walked root.
    pub relative_path: String,
    pub problem: LinkProblem,
    /// Where the link points, as written in the link.
    pub target: Option<String>,
}

/// The result of a read-only scan: every candidate file under `root`,
/// sorted by relative path for a deterministic, diffable report, and the
/// links that could not be followed (absent when there are none).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanReport {
    pub root: PathBuf,
    pub files: Vec<ScannedFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<LinkIssue>,
}

#[derive(Debug, Error)]
pub enum ScanError {
    #[error("folder does not exist or is not a directory: {0}")]
    NotADirectory(PathBuf),
    #[error("io error at {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("could not walk the folder: {0}")]
    Walk(#[from] walkdir::Error),
    /// The person stopped the work (`crate::local_work::CancelFlag`).
    #[error("the scan was stopped")]
    Cancelled,
}

fn is_hidden(entry: &walkdir::DirEntry) -> bool {
    entry.file_name().to_str().is_some_and(|name| {
        name.starts_with('.')
            // The two folders every Windows drive carries at its root: the
            // second one refuses to be read, which would fail the whole walk
            // of a night kept at the root of an external disk.
            || name.eq_ignore_ascii_case("$RECYCLE.BIN")
            || name.eq_ignore_ascii_case("System Volume Information")
    })
}

fn is_package_output_dir(entry: &walkdir::DirEntry) -> bool {
    entry.file_type().is_dir() && entry.file_name().to_str() == Some(PACKAGE_OUTPUT_DIR_NAME)
}

/// One file met by [`walk_folder`], before any byte of it is read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkedFile {
    /// Forward-slash path relative to the walked root.
    pub relative_path: String,
    /// The path to open (under the root, possibly through a link).
    pub path: PathBuf,
    pub class: FileClass,
    pub stamp: FileStamp,
    /// The file, or one of its parent folders, is reached through a link.
    pub through_link: bool,
}

/// Every candidate file under a root, and the links that could not be
/// followed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FolderWalk {
    pub files: Vec<WalkedFile>,
    pub links: Vec<LinkIssue>,
}

/// Walks `root` read-only, following symbolic links, without reading any
/// file: hidden entries (dotfiles and dot-directories, so Astro Sync's own
/// `.astro-sync-*.json`) and any `astro-session-package` output directory
/// are skipped. Sorted by relative path.
pub fn walk_folder(root: &Path, cancel: Option<&CancelFlag>) -> Result<FolderWalk, ScanError> {
    walk_folder_reporting(root, cancel, None)
}

/// How often a walk that reports says where it is.
const WALK_REPORT_EVERY: Duration = Duration::from_millis(100);

/// [`walk_folder`] that says where it is: `report(found, folder)` is called at
/// most every 100 ms with the number of files met so far and the folder being
/// walked (absolute). It is what the window shows
/// while the files are searched, before any header is read (a walk reads no
/// byte, so there is no total to measure).
pub fn walk_folder_reporting(
    root: &Path,
    cancel: Option<&CancelFlag>,
    report: Option<&dyn Fn(u64, &Path)>,
) -> Result<FolderWalk, ScanError> {
    if !root.is_dir() {
        return Err(ScanError::NotADirectory(root.to_path_buf()));
    }
    let mut walk = FolderWalk::default();
    let walker = WalkDir::new(root)
        .follow_links(true)
        .into_iter()
        .filter_entry(|entry| {
            entry.depth() == 0 || (!is_hidden(entry) && !is_package_output_dir(entry))
        });

    let mut last_report = Instant::now()
        .checked_sub(WALK_REPORT_EVERY)
        .unwrap_or_else(Instant::now);
    for entry in walker {
        if cancel.is_some_and(CancelFlag::is_cancelled) {
            return Err(ScanError::Cancelled);
        }
        if let Some(report) = report
            && last_report.elapsed() >= WALK_REPORT_EVERY
        {
            if let Ok(entry) = &entry {
                let folder = if entry.file_type().is_dir() {
                    entry.path()
                } else {
                    entry.path().parent().unwrap_or(root)
                };
                report(walk.files.len() as u64, folder);
                last_report = Instant::now();
            }
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                if let Some(issue) = link_issue(root, &error) {
                    walk.links.push(issue);
                    continue;
                }
                return Err(ScanError::Walk(error));
            }
        };
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .expect("walked entries are always under the scanned root");
        let metadata = fs::metadata(path).map_err(|source| ScanError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
        walk.files.push(WalkedFile {
            relative_path: to_relative_slash_string(relative),
            path: path.to_path_buf(),
            class: FileClass::classify(extension),
            stamp: FileStamp::from_metadata(&metadata),
            through_link: through_link(root, relative),
        });
    }
    walk.files
        .sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    walk.links
        .sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Ok(walk)
}

/// Whether `relative` (under `root`) passes through a symbolic link.
fn through_link(root: &Path, relative: &Path) -> bool {
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component);
        if fs::symlink_metadata(&current).is_ok_and(|m| m.file_type().is_symlink()) {
            return true;
        }
    }
    false
}

/// A walk error that is a link we cannot follow: a loop, or a link whose
/// target is missing. Anything else is a real error.
fn link_issue(root: &Path, error: &walkdir::Error) -> Option<LinkIssue> {
    let path = error.path()?;
    let relative = path.strip_prefix(root).map_or_else(
        |_| path.to_string_lossy().into_owned(),
        to_relative_slash_string,
    );
    let target = fs::read_link(path)
        .ok()
        .map(|target| target.to_string_lossy().into_owned());
    if error.loop_ancestor().is_some() {
        return Some(LinkIssue {
            relative_path: relative,
            problem: LinkProblem::Loop,
            target,
        });
    }
    let is_link = fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink());
    if is_link && fs::metadata(path).is_err() {
        return Some(LinkIssue {
            relative_path: relative,
            problem: LinkProblem::MissingTarget,
            target,
        });
    }
    None
}

/// What a scan may use beside the folder: a digest cache, a stop switch and
/// a progress sink.
#[derive(Default, Clone, Copy)]
pub struct ScanOptions<'a> {
    pub cache: Option<&'a DigestCache>,
    pub cancel: Option<&'a CancelFlag>,
    pub progress: Option<&'a (dyn Fn(&WorkProgress) + Sync)>,
    /// Hash only the files this returns `true` for (the others get an empty
    /// digest). `None`: every file.
    pub hash_only: Option<&'a (dyn Fn(&WalkedFile) -> bool + Sync)>,
}

/// Walks `root` read-only and returns every candidate file: hidden entries
/// (dotfiles and dot-directories) and any `astro-session-package` output
/// directory are skipped: everything else is classified, sized, and hashed.
pub fn scan_folder(root: &Path) -> Result<ScanReport, ScanError> {
    scan_folder_with(root, &ScanOptions::default())
}

/// [`scan_folder`] with a digest cache, a stop switch and a progress sink.
pub fn scan_folder_with(root: &Path, options: &ScanOptions<'_>) -> Result<ScanReport, ScanError> {
    let walk = walk_folder(root, options.cancel)?;
    let bytes_total: u64 = walk.files.iter().map(|file| file.stamp.size).sum();
    let total = walk.files.len() as u64;
    let mut files = Vec::with_capacity(walk.files.len());
    let mut bytes_done = 0_u64;
    for (index, file) in walk.files.iter().enumerate() {
        if let Some(progress) = options.progress {
            progress(&WorkProgress {
                kind: WorkKind::Scan,
                done: index as u64,
                total,
                bytes_done,
                bytes_total,
                current: Some(file.relative_path.clone()),
            });
        }
        let wanted = options.hash_only.is_none_or(|keep| keep(file));
        let sha256 = if wanted {
            digest_of(file, options.cache, options.cancel)?
        } else {
            String::new()
        };
        bytes_done += file.stamp.size;
        files.push(ScannedFile {
            relative_path: file.relative_path.clone(),
            class: file.class,
            bytes: file.stamp.size,
            sha256,
        });
    }
    if let Some(progress) = options.progress {
        progress(&WorkProgress {
            kind: WorkKind::Scan,
            done: total,
            total,
            bytes_done,
            bytes_total,
            current: None,
        });
    }
    Ok(ScanReport {
        root: root.to_path_buf(),
        files,
        links: walk.links,
    })
}

/// The SHA-256 of a walked file: from the cache when it knows the file in
/// this exact state, else read (and recorded there).
pub fn digest_of(
    file: &WalkedFile,
    cache: Option<&DigestCache>,
    cancel: Option<&CancelFlag>,
) -> Result<String, ScanError> {
    if let Some(cache) = cache
        && let Ok(Some(found)) = cache.get(&file.path, file.stamp)
    {
        return Ok(found);
    }
    let sha256 = hash_file(&file.path, cancel)?;
    if let Some(cache) = cache {
        // A cache that cannot be written only costs time.
        let _ = cache.put(&file.path, file.stamp, &sha256);
    }
    Ok(sha256)
}

/// The SHA-256 of one file (lowercase hexadecimal), read in chunks.
pub fn file_sha256(path: &Path) -> Result<String, ScanError> {
    hash_file(path, None)
}

fn hash_file(path: &Path, cancel: Option<&CancelFlag>) -> Result<String, ScanError> {
    let io_error = |source| ScanError::Io {
        path: path.to_path_buf(),
        source,
    };
    let mut file = fs::File::open(path).map_err(io_error)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        if cancel.is_some_and(CancelFlag::is_cancelled) {
            return Err(ScanError::Cancelled);
        }
        let read = file.read(&mut buffer).map_err(io_error)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;
    use std::fs;

    #[test]
    fn classify_covers_every_named_extension_and_defaults_to_other() {
        for ext in ["jpg", "jpeg", "png", "tif", "tiff"] {
            assert_eq!(FileClass::classify(ext), FileClass::Image, "{ext}");
            assert_eq!(
                FileClass::classify(&ext.to_ascii_uppercase()),
                FileClass::Image,
                "{ext} (uppercase)"
            );
        }
        for ext in ["log", "txt", "csv"] {
            assert_eq!(FileClass::classify(ext), FileClass::Log, "{ext}");
        }
        for ext in ["fit", "fits", "fts"] {
            assert_eq!(FileClass::classify(ext), FileClass::Fits, "{ext}");
        }
        assert_eq!(FileClass::classify("xisf"), FileClass::Xisf);
        assert_eq!(FileClass::classify("XISF"), FileClass::Xisf);
        assert!(FileClass::Xisf.is_frame() && FileClass::Fits.is_frame());
        assert!(!FileClass::Image.is_frame());
        assert_eq!(FileClass::classify("lnk"), FileClass::Other);
        for ext in [
            "cr2", "cr3", "nef", "arw", "dng", "raf", "pef", "orf", "rw2",
        ] {
            assert_eq!(FileClass::classify(ext), FileClass::Raw, "{ext}");
            assert_eq!(
                FileClass::classify(&ext.to_ascii_uppercase()),
                FileClass::Raw,
                "{ext} (uppercase)"
            );
        }
        for ext in ["avi", "ser", "mp4", "mov", "mkv", "AVI", "SER"] {
            assert_eq!(FileClass::classify(ext), FileClass::Video, "{ext}");
        }
        assert!(FileClass::Raw.is_frame() && !FileClass::Video.is_frame());
        assert_eq!(FileClass::classify(""), FileClass::Other);
    }

    /// Known SHA-256 test vectors (NIST/RFC-standard strings), so the
    /// hashing path is checked against values that do not depend on this
    /// implementation's own hashing to be correct.
    #[test]
    fn hashes_match_known_sha256_vectors() {
        let dir = TempDir::new("scanner-hash-vectors");
        let cases = [
            (
                "",
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            ),
            (
                "abc",
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            ),
        ];
        for (index, (content, _)) in cases.iter().enumerate() {
            fs::write(dir.path().join(format!("case-{index}.txt")), content).unwrap();
        }
        let report = scan_folder(dir.path()).unwrap();
        for (index, (_, expected)) in cases.iter().enumerate() {
            let file = report
                .files
                .iter()
                .find(|f| f.relative_path == format!("case-{index}.txt"))
                .unwrap();
            assert_eq!(&file.sha256, expected);
        }
    }

    #[test]
    fn scan_ignores_hidden_files_and_the_package_output_directory() {
        let dir = TempDir::new("scanner-ignores");
        fs::write(dir.path().join("visible.jpg"), b"data").unwrap();
        fs::write(dir.path().join(".hidden.jpg"), b"data").unwrap();
        fs::create_dir(dir.path().join(".hidden-dir")).unwrap();
        fs::write(dir.path().join(".hidden-dir/inner.jpg"), b"data").unwrap();
        fs::create_dir(dir.path().join(PACKAGE_OUTPUT_DIR_NAME)).unwrap();
        fs::write(
            dir.path()
                .join(PACKAGE_OUTPUT_DIR_NAME)
                .join("manifest.json"),
            b"{}",
        )
        .unwrap();

        let report = scan_folder(dir.path()).unwrap();
        let paths: Vec<&str> = report
            .files
            .iter()
            .map(|f| f.relative_path.as_str())
            .collect();
        assert_eq!(paths, vec!["visible.jpg"]);
    }

    #[test]
    fn scan_reports_bytes_and_is_sorted_by_relative_path() {
        let dir = TempDir::new("scanner-sorted");
        fs::create_dir(dir.path().join("b")).unwrap();
        fs::write(dir.path().join("b/frame.fits"), vec![0_u8; 10]).unwrap();
        fs::write(dir.path().join("a.log"), vec![0_u8; 3]).unwrap();

        let report = scan_folder(dir.path()).unwrap();
        let paths: Vec<&str> = report
            .files
            .iter()
            .map(|f| f.relative_path.as_str())
            .collect();
        assert_eq!(paths, vec!["a.log", "b/frame.fits"]);
        assert_eq!(report.files[0].bytes, 3);
        assert_eq!(report.files[0].class, FileClass::Log);
        assert_eq!(report.files[1].bytes, 10);
        assert_eq!(report.files[1].class, FileClass::Fits);
    }

    #[test]
    fn the_system_folders_of_a_windows_drive_are_not_walked() {
        let dir = TempDir::new("scanner-windows-system-folders");
        fs::create_dir_all(dir.path().join("System Volume Information")).unwrap();
        fs::write(dir.path().join("System Volume Information/x.fits"), b"x").unwrap();
        fs::create_dir_all(dir.path().join("$RECYCLE.BIN")).unwrap();
        fs::write(dir.path().join("$RECYCLE.BIN/y.fits"), b"y").unwrap();
        fs::write(dir.path().join("a.fits"), b"a").unwrap();

        let report = scan_folder(dir.path()).unwrap();
        let paths: Vec<&str> = report
            .files
            .iter()
            .map(|f| f.relative_path.as_str())
            .collect();
        assert_eq!(paths, vec!["a.fits"]);
    }

    #[test]
    fn scan_rejects_a_path_that_is_not_a_directory() {
        let dir = TempDir::new("scanner-not-a-dir");
        let file = dir.path().join("not-a-dir.txt");
        fs::write(&file, b"x").unwrap();
        assert!(matches!(
            scan_folder(&file),
            Err(ScanError::NotADirectory(_))
        ));
    }
    #[cfg(unix)]
    #[test]
    fn a_linked_folder_is_followed_and_its_target_hashed() {
        let outside = TempDir::new("scanner-link-target");
        fs::write(outside.path().join("frame-0001.fits"), b"abc").unwrap();
        let night = TempDir::new("scanner-link-night");
        fs::create_dir(night.path().join("M31")).unwrap();
        std::os::unix::fs::symlink(outside.path(), night.path().join("M31/lights")).unwrap();

        let report = scan_folder(night.path()).unwrap();
        assert_eq!(report.files.len(), 1);
        assert_eq!(report.files[0].relative_path, "M31/lights/frame-0001.fits");
        assert_eq!(
            report.files[0].sha256,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert!(report.links.is_empty());
        let walk = walk_folder(night.path(), None).unwrap();
        assert!(walk.files[0].through_link);
    }

    #[cfg(unix)]
    #[test]
    fn a_loop_of_links_and_a_missing_target_are_reported_not_walked() {
        let night = TempDir::new("scanner-link-loop");
        fs::create_dir_all(night.path().join("M31/lights")).unwrap();
        fs::write(night.path().join("M31/lights/a.fits"), b"x").unwrap();
        // A link back to a parent: a loop.
        std::os::unix::fs::symlink(night.path().join("M31"), night.path().join("M31/lights/up"))
            .unwrap();
        // A link to a disk that is not plugged in.
        std::os::unix::fs::symlink(
            night.path().join("absent-disk/Light"),
            night.path().join("logs"),
        )
        .unwrap();

        let report = scan_folder(night.path()).unwrap();
        let paths: Vec<&str> = report
            .files
            .iter()
            .map(|f| f.relative_path.as_str())
            .collect();
        assert_eq!(paths, vec!["M31/lights/a.fits"]);
        assert_eq!(report.links.len(), 2);
        let missing = report
            .links
            .iter()
            .find(|l| l.relative_path == "logs")
            .unwrap();
        assert_eq!(missing.problem, LinkProblem::MissingTarget);
        assert!(
            missing
                .target
                .as_deref()
                .unwrap()
                .ends_with("absent-disk/Light")
        );
        let looped = report
            .links
            .iter()
            .find(|l| l.relative_path == "M31/lights/up")
            .unwrap();
        assert_eq!(looped.problem, LinkProblem::Loop);
    }

    #[test]
    fn a_second_scan_takes_the_digests_from_the_cache() {
        let dir = TempDir::new("scanner-cache");
        fs::write(dir.path().join("final.tif"), b"abc").unwrap();
        let cache = DigestCache::in_memory().unwrap();
        let options = ScanOptions {
            cache: Some(&cache),
            ..ScanOptions::default()
        };
        let first = scan_folder_with(dir.path(), &options).unwrap();
        // The cache now answers for the file: plant a value only the cache
        // could give, to prove the bytes are not read again.
        let walk = walk_folder(dir.path(), None).unwrap();
        cache
            .put(&walk.files[0].path, walk.files[0].stamp, "from-cache")
            .unwrap();
        let second = scan_folder_with(dir.path(), &options).unwrap();
        assert_eq!(first.files[0].sha256.len(), 64);
        assert_eq!(second.files[0].sha256, "from-cache");
        // A changed file is read again.
        fs::write(dir.path().join("final.tif"), b"abcd").unwrap();
        let third = scan_folder_with(dir.path(), &options).unwrap();
        assert_eq!(
            third.files[0].sha256,
            "88d4266fd4e6338d13b845fcf289579d209c897823b9217da3e161936f031589"
        );
    }

    #[test]
    fn a_scan_stops_on_the_switch_and_reports_its_progress() {
        let dir = TempDir::new("scanner-cancel");
        for index in 0..3 {
            fs::write(dir.path().join(format!("f{index}.fits")), b"data").unwrap();
        }
        let seen = std::sync::Mutex::new(Vec::new());
        let progress = |p: &WorkProgress| seen.lock().unwrap().push((p.done, p.total));
        let report = scan_folder_with(
            dir.path(),
            &ScanOptions {
                progress: Some(&progress),
                ..ScanOptions::default()
            },
        )
        .unwrap();
        assert_eq!(report.files.len(), 3);
        assert_eq!(*seen.lock().unwrap(), vec![(0, 3), (1, 3), (2, 3), (3, 3)]);

        let cancel = CancelFlag::new();
        cancel.cancel();
        let stopped = scan_folder_with(
            dir.path(),
            &ScanOptions {
                cancel: Some(&cancel),
                ..ScanOptions::default()
            },
        );
        assert!(matches!(stopped, Err(ScanError::Cancelled)));
    }

    #[test]
    fn a_walk_says_how_many_files_it_met_and_in_which_folder() {
        let dir = TempDir::new("scanner-walk-report");
        let nested = dir.path().join("Autorun").join("Light");
        fs::create_dir_all(&nested).unwrap();
        for index in 0..5 {
            fs::write(nested.join(format!("f{index}.fits")), b"data").unwrap();
        }
        let seen = std::sync::Mutex::new(Vec::new());
        let report = |found: u64, folder: &Path| {
            seen.lock().unwrap().push((found, folder.to_path_buf()));
        };
        let walk = walk_folder_reporting(dir.path(), None, Some(&report)).unwrap();
        assert_eq!(walk.files.len(), 5);
        let seen = seen.lock().unwrap();
        // At most every 100 ms: a short walk reports once, at once, from the root.
        assert!(!seen.is_empty() && seen.len() <= 2, "{seen:?}");
        assert_eq!(seen[0], (0, dir.path().to_path_buf()));
        assert!(seen.iter().all(|(found, _)| *found <= 5));

        // Without a sink, the same walk.
        assert_eq!(walk_folder(dir.path(), None).unwrap(), walk);
    }
}
