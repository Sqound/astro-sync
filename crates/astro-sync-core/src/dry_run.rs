//! The dry-run report: the exact list of files, bytes, hashes, and
//! destination visibility that producing a package *would* send -- built
//! straight from a [`ScanReport`], with no filesystem access of its own.
//!
//! "Destination visibility" here means exactly where and as what kind of
//! asset each source file would land inside the produced
//! `astro-session-package/` (its [`AssetKind`] and its package-relative
//! path): the plan calls for showing this before anything is written, and
//! nothing beyond this module's pure computation is needed to know it,
//! since [`crate::package::produce_package`] lays assets out the same way.
//! Nothing is transferred by building this report; there is no network
//! access anywhere in this crate.

use serde::Serialize;
use std::path::{Path, PathBuf};

use crate::package::AssetKind;
use crate::scanner::{FileClass, ScanReport};
use crate::upload_queue::result_kind_of;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DryRunEntry {
    /// Forward-slash path relative to the source folder.
    pub source_relative_path: String,
    pub class: FileClass,
    pub asset_kind: AssetKind,
    /// Where this file would land inside the produced package, relative to
    /// the package root (e.g. `"images/2026-09-20/frame-0001.fits"`).
    pub package_relative_path: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DryRunReport {
    pub source_root: PathBuf,
    pub entries: Vec<DryRunEntry>,
    pub total_bytes: u64,
}

/// Builds the dry-run report a scan would produce a package from. Entries
/// keep the scan's order (sorted by source-relative path).
#[must_use]
pub fn build_dry_run_report(scan: &ScanReport) -> DryRunReport {
    let entries: Vec<DryRunEntry> = scan
        .files
        .iter()
        .map(|file| {
            let asset_kind = AssetKind::from(file.class);
            let package_relative_path = format!(
                "{}/{}",
                asset_kind.package_subdirectory(),
                file.relative_path
            );
            DryRunEntry {
                source_relative_path: file.relative_path.clone(),
                class: file.class,
                asset_kind,
                package_relative_path,
                bytes: file.bytes,
                sha256: file.sha256.clone(),
            }
        })
        .collect();
    let total_bytes = entries.iter().map(|entry| entry.bytes).sum();
    DryRunReport {
        source_root: scan.root.clone(),
        entries,
        total_bytes,
    }
}

/// The report of the **package**, which keeps only what can leave the
/// computer: the results the upload queue takes
/// ([`crate::upload_queue::result_kind_of`]) -- a final or stacked image --
/// and never a FITS, a XISF, a raw camera file, a calibration frame or a raw
/// log. A package that listed them could not be imported: the portal links
/// each of its files to a file already online, and those never go online
/// (`asset_not_uploaded`). The simulation and the package come from this
/// one list, so the screen, `dry-run --json` and the package agree.
/// [`build_dry_run_report`] stays the whole inventory of the folder.
#[must_use]
pub fn build_package_report(scan: &ScanReport) -> DryRunReport {
    let mut report = build_dry_run_report(scan);
    report
        .entries
        .retain(|entry| result_kind_of(Path::new(&entry.source_relative_path)).is_ok());
    report.total_bytes = report.entries.iter().map(|entry| entry.bytes).sum();
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::scan_folder;
    use crate::test_support::TempDir;
    use std::fs;

    #[test]
    fn maps_every_scanned_file_to_a_destination_and_sums_the_total_bytes() {
        let dir = TempDir::new("dry-run-basic");
        fs::write(dir.path().join("frame.fits"), vec![0_u8; 10]).unwrap();
        fs::write(dir.path().join("quality.csv"), vec![0_u8; 5]).unwrap();
        fs::write(dir.path().join("raw.dat"), vec![0_u8; 3]).unwrap();

        let scan = scan_folder(dir.path()).unwrap();
        let report = build_dry_run_report(&scan);

        assert_eq!(report.entries.len(), 3);
        assert_eq!(report.total_bytes, 18);
        assert_eq!(report.source_root, dir.path());

        let fits = report
            .entries
            .iter()
            .find(|e| e.source_relative_path == "frame.fits")
            .unwrap();
        assert_eq!(fits.asset_kind, AssetKind::Image);
        assert_eq!(fits.package_relative_path, "images/frame.fits");

        let csv = report
            .entries
            .iter()
            .find(|e| e.source_relative_path == "quality.csv")
            .unwrap();
        assert_eq!(csv.asset_kind, AssetKind::MetricLog);
        assert_eq!(csv.package_relative_path, "metrics/quality.csv");

        let other = report
            .entries
            .iter()
            .find(|e| e.source_relative_path == "raw.dat")
            .unwrap();
        assert_eq!(other.asset_kind, AssetKind::Other);
        assert_eq!(other.package_relative_path, "other/raw.dat");
    }

    #[test]
    fn preserves_subdirectory_structure_in_the_destination_path() {
        let dir = TempDir::new("dry-run-nested");
        fs::create_dir(dir.path().join("2026-09-20")).unwrap();
        fs::write(dir.path().join("2026-09-20/frame-0001.fits"), vec![0_u8; 1]).unwrap();

        let scan = scan_folder(dir.path()).unwrap();
        let report = build_dry_run_report(&scan);

        assert_eq!(
            report.entries[0].package_relative_path,
            "images/2026-09-20/frame-0001.fits"
        );
    }

    #[test]
    fn an_empty_folder_produces_an_empty_report() {
        let dir = TempDir::new("dry-run-empty");
        let scan = scan_folder(dir.path()).unwrap();
        let report = build_dry_run_report(&scan);
        assert!(report.entries.is_empty());
        assert_eq!(report.total_bytes, 0);
    }

    /// A real night's package holds its results only.
    #[test]
    fn the_package_report_keeps_the_results_and_no_frame_nor_raw_log() {
        let dir = TempDir::new("dry-run-package");
        for (relative, bytes) in [
            ("target 1/final/image.tif", 40),
            ("target 1/final/image.jpg", 10),
            ("target 1/stacked/stack.png", 30),
            ("target 1/lights/pose-1/Light_001.fits", 500),
            ("target 1/lights/pose-1/preview.jpg", 5),
            ("calibration/darks/dark.fit", 200),
            ("logs/Autorun_Log.txt", 8),
            ("logs/PHD2_GuideLog.txt", 9),
            ("notes.txt", 2),
            ("capture.xisf", 70),
        ] {
            let path = dir.path().join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, vec![0_u8; bytes]).unwrap();
        }
        let scan = scan_folder(dir.path()).unwrap();
        assert_eq!(build_dry_run_report(&scan).entries.len(), 10);
        let package = build_package_report(&scan);
        let kept: Vec<&str> = package
            .entries
            .iter()
            .map(|e| e.source_relative_path.as_str())
            .collect();
        assert_eq!(
            kept,
            vec![
                "target 1/final/image.jpg",
                "target 1/final/image.tif",
                "target 1/stacked/stack.png"
            ]
        );
        assert_eq!(package.total_bytes, 80);
        assert!(
            package
                .entries
                .iter()
                .all(|e| e.asset_kind == AssetKind::Image)
        );
    }
}
