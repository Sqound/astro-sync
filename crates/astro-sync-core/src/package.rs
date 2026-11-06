//! The open session-package producer.
//!
//! Writes an `astro-session-package/` directory conforming to the schema
//! (`contracts/schemas/session-package/manifest.schema.json`,
//! `README.md` in the same directory): a `manifest.json`
//! (`packageVersion: 1`, ISO 8601 UTC timestamps, package-root-relative
//! asset paths) and a matching `checksums.sha256`. The output location is
//! always chosen by the caller and is never allowed to resolve inside the
//! source folder that was scanned -- see [`produce_package`].
//!
//! `AssetKind` mirrors the manifest schema's `assetKind` enum exactly
//! (`image` / `metric-log` / `event-log` / `progression` / `other`). Task
//! 8.3's scanner only ever classifies files as image, FITS, log, or other
//! (`crate::scanner::FileClass`), so neither `event-log` nor `progression`
//! is ever emitted by [`crate::dry_run::build_dry_run_report`] itself --
//! [`append_analysis_assets`] (`event-log`) and
//! [`append_progression_assets`] (`progression`) are the two additive
//! calls that add them to an already-produced package, and a package
//! written by hand may use either kind directly too.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::analysis::AnalysisArtifacts;
use crate::analysis::siril::ProgressionRecord;
use crate::dry_run::DryRunReport;
use crate::paths::{PathValidationError, is_within, validate_relative_asset_path};
use crate::scanner::{FileClass, PACKAGE_OUTPUT_DIR_NAME};
use crate::time_util::now_iso8601_utc;

/// The manifest contract's own version. This crate only ever produces (and
/// the contract only accepts) `1`.
pub const PACKAGE_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AssetKind {
    Image,
    MetricLog,
    EventLog,
    Progression,
    Other,
}

impl AssetKind {
    /// The package subdirectory this kind of asset is written under. Not
    /// part of the contract itself (the manifest is the source of truth for
    /// where each asset actually lives), but it is how this producer lays
    /// assets out predictably.
    #[must_use]
    pub fn package_subdirectory(self) -> &'static str {
        match self {
            AssetKind::Image => "images",
            AssetKind::MetricLog => "metrics",
            AssetKind::EventLog => "events",
            AssetKind::Progression => "progressions",
            AssetKind::Other => "other",
        }
    }
}

impl From<FileClass> for AssetKind {
    fn from(class: FileClass) -> Self {
        match class {
            // FITS captures are images content-wise (light frames), even
            // though the scanner classifies them separately from
            // jpg/png/tiff/raw so a future analysis pass can find them
            // independently of the package's own asset layout.
            FileClass::Image | FileClass::Raw | FileClass::Fits | FileClass::Xisf => {
                AssetKind::Image
            }
            FileClass::Log => AssetKind::MetricLog,
            FileClass::Video | FileClass::Other => AssetKind::Other,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestAsset {
    pub path: String,
    pub kind: AssetKind,
    pub bytes: u64,
    /// When the underlying observation was captured. Always `None` here:
    /// extracting capture timestamps from EXIF/FITS headers is
    /// log/FITS analysis, out of the scope of packaging.
    pub captured_at: Option<String>,
    pub added_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

/// Descriptive-only reference to the observation night, matching the
/// manifest schema's `session` block exactly -- deliberately carrying no id
/// field, so a package can never claim to belong to a particular hosted
/// session or account by itself (`contracts/schemas/session-package/README.md`, "never trust a client-supplied tenant id").
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMeta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
}

/// Free-form, informational-only identification of the tool that wrote the
/// package.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProducerInfo {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub package_version: u32,
    pub generated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub producer: Option<ProducerInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionMeta>,
    pub assets: Vec<ManifestAsset>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageOutcome {
    pub output_dir: PathBuf,
    pub manifest_path: PathBuf,
    pub checksums_path: PathBuf,
    pub asset_count: usize,
    pub total_bytes: u64,
}

#[derive(Debug, Error)]
pub enum PackageError {
    #[error("nothing to package: the dry run found no file")]
    EmptyReport,
    #[error("output location does not exist or is not a directory: {0}")]
    OutputParentNotFound(PathBuf),
    #[error(
        "refusing to write the package inside the source folder ({output} is inside {source_root})"
    )]
    OutputInsideSource {
        output: PathBuf,
        source_root: PathBuf,
    },
    #[error("a package already exists at {0}; remove it or choose another output location")]
    PackageAlreadyExists(PathBuf),
    #[error("invalid destination path {path:?}: {source}")]
    InvalidAssetPath {
        path: String,
        source: PathValidationError,
    },
    #[error("io error at {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error(
        "analysis asset path {0} already exists in the manifest; refusing to silently overwrite a hand-provided asset with a derived one"
    )]
    AnalysisPathCollision(String),
    #[error("could not read back the manifest at {path}: {source}")]
    ManifestUnreadable {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error(
        "progression asset path {0} already exists in the manifest; refusing to silently overwrite a hand-provided asset with a derived one"
    )]
    ProgressionPathCollision(String),
    #[error("no milestone named {0:?} in this progression record")]
    UnknownMilestoneLabel(String),
}

/// Builds the manifest a package for `report` would contain, using
/// `generated_at` for the manifest's own timestamp and every asset's
/// `addedAt` (a package is written in one atomic pass, so every asset
/// shares the moment the package itself was generated). Pure and
/// filesystem-free: exposed separately from [`produce_package`] so a caller
/// that only needs the exact manifest shape -- the CLI/GUI golden tests
/// (`crates/astro-sync-cli/tests/golden.rs`), or a future preview feature --
/// does not have to read the wall clock or write real files to get it.
#[must_use]
pub fn build_manifest(
    report: &DryRunReport,
    generated_at: String,
    session: Option<SessionMeta>,
    producer: Option<ProducerInfo>,
) -> Manifest {
    let assets = report
        .entries
        .iter()
        .map(|entry| ManifestAsset {
            path: entry.package_relative_path.clone(),
            kind: entry.asset_kind,
            bytes: entry.bytes,
            captured_at: None,
            added_at: generated_at.clone(),
            mime_type: guess_mime_type(&entry.package_relative_path),
        })
        .collect();
    Manifest {
        package_version: PACKAGE_VERSION,
        generated_at,
        producer,
        session,
        assets,
    }
}

/// Writes `<output_parent>/astro-session-package/` from a dry-run report:
/// copies every source file into the appropriate subdirectory, then writes
/// `manifest.json` and `checksums.sha256`. Reads every source file exactly
/// once and never writes into the source folder -- refuses outright if
/// `output_parent` (or the package directory itself) would resolve to
/// somewhere inside `report.source_root`.
pub fn produce_package(
    report: &DryRunReport,
    output_parent: &Path,
    session: Option<SessionMeta>,
    producer: Option<ProducerInfo>,
) -> Result<PackageOutcome, PackageError> {
    if report.entries.is_empty() {
        return Err(PackageError::EmptyReport);
    }
    if !output_parent.is_dir() {
        return Err(PackageError::OutputParentNotFound(
            output_parent.to_path_buf(),
        ));
    }

    let output_dir = output_parent.join(PACKAGE_OUTPUT_DIR_NAME);
    let output_inside_source = is_within(&report.source_root, output_parent).unwrap_or(false)
        || is_within(&report.source_root, &output_dir).unwrap_or(false);
    if output_inside_source {
        return Err(PackageError::OutputInsideSource {
            output: output_parent.to_path_buf(),
            source_root: report.source_root.clone(),
        });
    }
    if output_dir.exists() {
        return Err(PackageError::PackageAlreadyExists(output_dir));
    }

    // Validate every destination path before touching the filesystem: one
    // bad entry must not leave a half-written package behind.
    for entry in &report.entries {
        validate_relative_asset_path(&entry.package_relative_path).map_err(|source| {
            PackageError::InvalidAssetPath {
                path: entry.package_relative_path.clone(),
                source,
            }
        })?;
    }

    fs::create_dir_all(&output_dir).map_err(|source| PackageError::Io {
        path: output_dir.clone(),
        source,
    })?;

    let generated_at = now_iso8601_utc();
    let mut checksums = String::new();

    for entry in &report.entries {
        let destination = output_dir.join(&entry.package_relative_path);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|source| PackageError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let source_path = report.source_root.join(&entry.source_relative_path);
        fs::copy(&source_path, &destination).map_err(|source| PackageError::Io {
            path: source_path.clone(),
            source,
        })?;

        checksums.push_str(&format!(
            "{}  {}\n",
            entry.sha256, entry.package_relative_path
        ));
    }

    let manifest = build_manifest(report, generated_at, session, producer);
    let manifest_json =
        serde_json::to_string_pretty(&manifest).expect("manifest always serializes") + "\n";
    let manifest_path = output_dir.join("manifest.json");
    fs::write(&manifest_path, manifest_json).map_err(|source| PackageError::Io {
        path: manifest_path.clone(),
        source,
    })?;

    let checksums_path = output_dir.join("checksums.sha256");
    fs::write(&checksums_path, checksums).map_err(|source| PackageError::Io {
        path: checksums_path.clone(),
        source,
    })?;

    Ok(PackageOutcome {
        total_bytes: report.total_bytes,
        asset_count: report.entries.len(),
        output_dir,
        manifest_path,
        checksums_path,
    })
}

/// Adds analysis output (`crate::analysis::analyze_folder`'s
/// [`AnalysisArtifacts`]) to a package [`produce_package`] already wrote:
/// writes whichever of `quality.csv`/`guiding.csv`/`session.jsonl` is present
/// under `metrics/`/`events/`, recomputes `checksums.sha256`, and rewrites
/// `manifest.json` with one new `metric-log`/`event-log` asset entry per file
/// added. Kept as a separate call over an already-produced package (rather than
/// a parameter on [`produce_package`] itself) so the package producer stays
/// agnostic of analysis specifics, and so every existing caller of
/// `produce_package` (four parameters, unchanged) is unaffected.
///
/// Refuses with [`PackageError::AnalysisPathCollision`] if the package
/// already lists an asset at one of those three fixed paths -- e.g. the
/// scanned source folder itself already contained a hand-written
/// `quality.csv` that [`produce_package`] copied through as a plain
/// `metric-log` asset. A derived file must never silently replace one a
/// person or another tool supplied. When none of the three fields is
/// `Some`, this is a no-op that returns `outcome` unchanged (not an
/// error): calling it with an empty [`AnalysisArtifacts`] is a valid,
/// if pointless, thing to do.
pub fn append_analysis_assets(
    outcome: &PackageOutcome,
    analysis: &AnalysisArtifacts,
) -> Result<PackageOutcome, PackageError> {
    let manifest_bytes = fs::read(&outcome.manifest_path).map_err(|source| PackageError::Io {
        path: outcome.manifest_path.clone(),
        source,
    })?;
    let mut manifest: Manifest = serde_json::from_slice(&manifest_bytes).map_err(|source| {
        PackageError::ManifestUnreadable {
            path: outcome.manifest_path.clone(),
            source,
        }
    })?;
    let mut checksums =
        fs::read_to_string(&outcome.checksums_path).map_err(|source| PackageError::Io {
            path: outcome.checksums_path.clone(),
            source,
        })?;

    let slots: [(Option<&String>, &str, AssetKind, &str); 3] = [
        (
            analysis.quality_csv.as_ref(),
            "metrics/quality.csv",
            AssetKind::MetricLog,
            "text/csv",
        ),
        (
            analysis.guiding_csv.as_ref(),
            "metrics/guiding.csv",
            AssetKind::MetricLog,
            "text/csv",
        ),
        (
            analysis.session_jsonl.as_ref(),
            "events/session.jsonl",
            AssetKind::EventLog,
            "application/jsonl",
        ),
    ];

    let generated_at = now_iso8601_utc();
    let mut added_bytes = 0_u64;
    let mut added_count = 0_usize;

    for (content, relative_path, kind, mime_type) in slots {
        let Some(content) = content else { continue };
        if manifest
            .assets
            .iter()
            .any(|asset| asset.path == relative_path)
        {
            return Err(PackageError::AnalysisPathCollision(
                relative_path.to_string(),
            ));
        }

        let destination = outcome.output_dir.join(relative_path);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|source| PackageError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        fs::write(&destination, content).map_err(|source| PackageError::Io {
            path: destination.clone(),
            source,
        })?;

        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        let sha256 = format!("{:x}", hasher.finalize());
        checksums.push_str(&format!("{sha256}  {relative_path}\n"));

        manifest.assets.push(ManifestAsset {
            path: relative_path.to_string(),
            kind,
            bytes: content.len() as u64,
            captured_at: None,
            added_at: generated_at.clone(),
            mime_type: Some(mime_type.to_string()),
        });

        added_bytes += content.len() as u64;
        added_count += 1;
    }

    if added_count == 0 {
        return Ok(outcome.clone());
    }

    let manifest_json =
        serde_json::to_string_pretty(&manifest).expect("manifest always serializes") + "\n";
    fs::write(&outcome.manifest_path, manifest_json).map_err(|source| PackageError::Io {
        path: outcome.manifest_path.clone(),
        source,
    })?;
    fs::write(&outcome.checksums_path, checksums).map_err(|source| PackageError::Io {
        path: outcome.checksums_path.clone(),
        source,
    })?;

    Ok(PackageOutcome {
        output_dir: outcome.output_dir.clone(),
        manifest_path: outcome.manifest_path.clone(),
        checksums_path: outcome.checksums_path.clone(),
        asset_count: outcome.asset_count + added_count,
        total_bytes: outcome.total_bytes + added_bytes,
    })
}

/// Adds selected Siril progression milestone images
/// (`analysis::siril::run_progression`'s [`ProgressionRecord`]) to a
/// package [`produce_package`] already wrote: copies each selected
/// milestone's output file under `progressions/`, writes
/// `progressions/progression.json` (the exact record: Siril version,
/// script, settings, acquisition family, every milestone), recomputes
/// `checksums.sha256`, and rewrites `manifest.json` with one new
/// `progression` asset per selected image plus one `other` asset for the
/// record itself.
///
/// Only the milestones named in `selected_milestone_labels` are ever read
/// from disk -- never every milestone `record` describes unconditionally
/// -- matching the rule that Astro Sync sends the approved report,
/// selected derived progression images, and final processed images: the
/// person chooses which progression images leave the device, exactly like
/// `append_analysis_assets` never assumes every possible analysis file is
/// wanted. An empty `selected_milestone_labels` is a no-op (returns
/// `outcome` unchanged, including no `progression.json`), the same
/// "calling this with nothing to add is valid, not an error" rule
/// `append_analysis_assets` already follows -- a person may choose to skip
/// sending any progression image at all.
///
/// Kept as a separate call over an already-produced package (rather than a
/// parameter on [`produce_package`] itself), for the same reason
/// `append_analysis_assets` is: the package producer stays agnostic of
/// analysis specifics, and every existing caller of `produce_package` is
/// unaffected.
///
/// Refuses with [`PackageError::UnknownMilestoneLabel`] if a requested
/// label is not one `record.milestones` actually has, and with
/// [`PackageError::ProgressionPathCollision`] if the package already lists
/// an asset at the destination path a selected milestone or the record
/// would be written to -- a derived file must never silently replace one a
/// person or another tool supplied, exactly as for analysis assets.
pub fn append_progression_assets(
    outcome: &PackageOutcome,
    record: &ProgressionRecord,
    selected_milestone_labels: &[String],
) -> Result<PackageOutcome, PackageError> {
    if selected_milestone_labels.is_empty() {
        return Ok(outcome.clone());
    }

    let manifest_bytes = fs::read(&outcome.manifest_path).map_err(|source| PackageError::Io {
        path: outcome.manifest_path.clone(),
        source,
    })?;
    let mut manifest: Manifest = serde_json::from_slice(&manifest_bytes).map_err(|source| {
        PackageError::ManifestUnreadable {
            path: outcome.manifest_path.clone(),
            source,
        }
    })?;
    let mut checksums =
        fs::read_to_string(&outcome.checksums_path).map_err(|source| PackageError::Io {
            path: outcome.checksums_path.clone(),
            source,
        })?;

    let generated_at = now_iso8601_utc();
    let mut added_bytes = 0_u64;
    let mut added_count = 0_usize;

    for label in selected_milestone_labels {
        let milestone = record
            .milestones
            .iter()
            .find(|milestone| &milestone.label == label)
            .ok_or_else(|| PackageError::UnknownMilestoneLabel(label.clone()))?;

        let source_path = Path::new(&milestone.output_path);
        let file_name = source_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(label.as_str());
        let relative_path = format!("progressions/{file_name}");
        let content = fs::read(source_path).map_err(|source| PackageError::Io {
            path: source_path.to_path_buf(),
            source,
        })?;
        add_one_progression_file(
            outcome,
            &mut manifest,
            &mut checksums,
            &relative_path,
            &content,
            AssetKind::Progression,
            guess_mime_type(&relative_path),
            &generated_at,
        )?;
        added_count += 1;
        added_bytes += content.len() as u64;
    }

    let record_json =
        serde_json::to_string_pretty(record).expect("ProgressionRecord always serializes") + "\n";
    add_one_progression_file(
        outcome,
        &mut manifest,
        &mut checksums,
        "progressions/progression.json",
        record_json.as_bytes(),
        AssetKind::Other,
        Some("application/json".to_string()),
        &generated_at,
    )?;
    added_count += 1;
    added_bytes += record_json.len() as u64;

    let manifest_json =
        serde_json::to_string_pretty(&manifest).expect("manifest always serializes") + "\n";
    fs::write(&outcome.manifest_path, manifest_json).map_err(|source| PackageError::Io {
        path: outcome.manifest_path.clone(),
        source,
    })?;
    fs::write(&outcome.checksums_path, checksums).map_err(|source| PackageError::Io {
        path: outcome.checksums_path.clone(),
        source,
    })?;

    Ok(PackageOutcome {
        output_dir: outcome.output_dir.clone(),
        manifest_path: outcome.manifest_path.clone(),
        checksums_path: outcome.checksums_path.clone(),
        asset_count: outcome.asset_count + added_count,
        total_bytes: outcome.total_bytes + added_bytes,
    })
}

/// Shared write-one-file step for [`append_progression_assets`]: refuses a
/// collision against an asset path the manifest already lists, writes the
/// bytes under `outcome.output_dir`, appends the checksum line, and pushes
/// the manifest asset entry. Mutates `manifest`/`checksums` in place so the
/// caller can call this once per selected milestone plus once for the
/// record itself without repeating any of this bookkeeping.
#[allow(clippy::too_many_arguments)]
fn add_one_progression_file(
    outcome: &PackageOutcome,
    manifest: &mut Manifest,
    checksums: &mut String,
    relative_path: &str,
    content: &[u8],
    kind: AssetKind,
    mime_type: Option<String>,
    added_at: &str,
) -> Result<(), PackageError> {
    if manifest
        .assets
        .iter()
        .any(|asset| asset.path == relative_path)
    {
        return Err(PackageError::ProgressionPathCollision(
            relative_path.to_string(),
        ));
    }

    let destination = outcome.output_dir.join(relative_path);
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|source| PackageError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    fs::write(&destination, content).map_err(|source| PackageError::Io {
        path: destination.clone(),
        source,
    })?;

    let mut hasher = Sha256::new();
    hasher.update(content);
    let sha256 = format!("{:x}", hasher.finalize());
    checksums.push_str(&format!("{sha256}  {relative_path}\n"));

    manifest.assets.push(ManifestAsset {
        path: relative_path.to_string(),
        kind,
        bytes: content.len() as u64,
        captured_at: None,
        added_at: added_at.to_string(),
        mime_type,
    });

    Ok(())
}

fn guess_mime_type(package_relative_path: &str) -> Option<String> {
    let extension = package_relative_path
        .rsplit('.')
        .next()?
        .to_ascii_lowercase();
    let mime = match extension.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "tif" | "tiff" => "image/tiff",
        "fits" | "fit" | "fts" => "image/fits",
        "csv" => "text/csv",
        "txt" | "log" => "text/plain",
        _ => return None,
    };
    Some(mime.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::fits::AcquisitionFamily;
    use crate::analysis::siril::{DisplayScale, MilestoneOutcome, ProgressionSettings};
    use crate::dry_run::build_dry_run_report;
    use crate::scanner::scan_folder;
    use crate::test_support::TempDir;
    use std::fs;

    fn fixture_with_two_files() -> (TempDir, DryRunReport) {
        let source = TempDir::new("package-fixture");
        fs::write(source.path().join("frame-0001.fits"), vec![1_u8; 16]).unwrap();
        fs::write(source.path().join("quality.csv"), b"fwhm,rms\n1.2,0.4\n").unwrap();
        let scan = scan_folder(source.path()).unwrap();
        let report = build_dry_run_report(&scan);
        (source, report)
    }

    #[test]
    fn produces_a_manifest_and_checksums_covering_every_asset() {
        let (source, report) = fixture_with_two_files();
        let output_parent = TempDir::new("package-output");

        let outcome = produce_package(&report, output_parent.path(), None, None).unwrap();

        assert_eq!(outcome.asset_count, 2);
        assert!(outcome.manifest_path.is_file());
        assert!(outcome.checksums_path.is_file());
        assert!(outcome.output_dir.join("images/frame-0001.fits").is_file());
        assert!(outcome.output_dir.join("metrics/quality.csv").is_file());

        let manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&outcome.manifest_path).unwrap()).unwrap();
        assert_eq!(manifest["packageVersion"], 1);
        assert_eq!(manifest["assets"].as_array().unwrap().len(), 2);

        let checksums = fs::read_to_string(&outcome.checksums_path).unwrap();
        assert_eq!(checksums.lines().count(), 2);
        assert!(checksums.contains("  images/frame-0001.fits"));

        let _ = &source; // keep the fixture alive for the whole assertion block
    }

    /// Structural check against the shape
    /// `contracts/schemas/session-package/manifest.schema.json`
    /// requires: this crate has no dependency on a JSON Schema validator, so
    /// this test parses the produced `manifest.json` as generic JSON and
    /// re-checks the required keys and the "no traversal in any asset path"
    /// rule directly, independently of the typed `Manifest`/`ManifestAsset`
    /// structs that produced it.
    #[test]
    fn produced_manifest_satisfies_the_schemas_structural_requirements() {
        let (_source, report) = fixture_with_two_files();
        let output_parent = TempDir::new("package-structural-check");
        let outcome = produce_package(&report, output_parent.path(), None, None).unwrap();

        let manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&outcome.manifest_path).unwrap()).unwrap();

        for required_key in ["packageVersion", "generatedAt", "assets"] {
            assert!(
                manifest.get(required_key).is_some(),
                "manifest.json is missing required key {required_key:?}"
            );
        }
        assert_eq!(manifest["packageVersion"], 1);
        assert!(manifest["generatedAt"].as_str().unwrap().ends_with('Z'));

        let assets = manifest["assets"].as_array().unwrap();
        assert!(!assets.is_empty());
        for asset in assets {
            for required_key in ["path", "kind", "bytes", "addedAt"] {
                assert!(
                    asset.get(required_key).is_some(),
                    "asset {asset:?} is missing required key {required_key:?}"
                );
            }
            let path = asset["path"].as_str().unwrap();
            assert_eq!(
                validate_relative_asset_path(path),
                Ok(()),
                "asset path {path:?} must be a safe, traversal-free relative path"
            );
            assert!(matches!(
                asset["kind"].as_str().unwrap(),
                "image" | "metric-log" | "event-log" | "progression" | "other"
            ));
        }
    }

    #[test]
    fn refuses_when_the_output_would_land_inside_the_source_folder() {
        let (source, report) = fixture_with_two_files();
        let result = produce_package(&report, source.path(), None, None);
        assert!(matches!(
            result,
            Err(PackageError::OutputInsideSource { .. })
        ));
    }

    #[test]
    fn refuses_when_the_output_would_land_inside_a_subfolder_of_the_source() {
        let (source, report) = fixture_with_two_files();
        let nested = source.path().join("nested-output");
        fs::create_dir(&nested).unwrap();
        let result = produce_package(&report, &nested, None, None);
        assert!(matches!(
            result,
            Err(PackageError::OutputInsideSource { .. })
        ));
    }

    #[test]
    fn refuses_an_empty_report() {
        let output_parent = TempDir::new("package-empty-report");
        let empty = DryRunReport {
            source_root: TempDir::new("package-empty-source").path().to_path_buf(),
            entries: Vec::new(),
            total_bytes: 0,
        };
        let refused = produce_package(&empty, output_parent.path(), None, None).unwrap_err();
        assert!(matches!(refused, PackageError::EmptyReport));
        // C3: said in plain English, the language of the command line.
        assert_eq!(
            refused.to_string(),
            "nothing to package: the dry run found no file"
        );
    }

    #[test]
    fn refuses_when_a_package_already_exists_at_the_output_location() {
        let (_source, report) = fixture_with_two_files();
        let output_parent = TempDir::new("package-already-exists");
        fs::create_dir(output_parent.path().join(PACKAGE_OUTPUT_DIR_NAME)).unwrap();
        assert!(matches!(
            produce_package(&report, output_parent.path(), None, None),
            Err(PackageError::PackageAlreadyExists(_))
        ));
    }

    #[test]
    fn rejects_a_dry_run_entry_whose_destination_path_traverses_out_of_the_package() {
        let (_source, mut report) = fixture_with_two_files();
        report.entries[0].package_relative_path = "images/../../etc/passwd".to_string();
        let output_parent = TempDir::new("package-traversal");
        let result = produce_package(&report, output_parent.path(), None, None);
        assert!(matches!(result, Err(PackageError::InvalidAssetPath { .. })));
        // Nothing should have been written: validation runs before any copy.
        assert!(!output_parent.path().join(PACKAGE_OUTPUT_DIR_NAME).exists());
    }

    #[test]
    fn asset_kind_maps_fits_and_conventional_images_to_the_images_directory() {
        assert_eq!(AssetKind::from(FileClass::Image), AssetKind::Image);
        assert_eq!(AssetKind::from(FileClass::Fits), AssetKind::Image);
        assert_eq!(AssetKind::from(FileClass::Log), AssetKind::MetricLog);
        assert_eq!(AssetKind::from(FileClass::Other), AssetKind::Other);
    }

    #[test]
    fn session_and_producer_are_included_when_provided() {
        let (_source, report) = fixture_with_two_files();
        let output_parent = TempDir::new("package-session-meta");
        let session = SessionMeta {
            label: Some("2026-09-20 — target".to_string()),
            started_at: Some("2026-09-20T18:30:00Z".to_string()),
            finished_at: Some("2026-09-21T02:10:00Z".to_string()),
        };
        let producer = ProducerInfo {
            name: "astro-sync-cli".to_string(),
            version: Some("0.1.0".to_string()),
        };
        let outcome =
            produce_package(&report, output_parent.path(), Some(session), Some(producer)).unwrap();
        let manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&outcome.manifest_path).unwrap()).unwrap();
        assert!(manifest["session"]["id"].is_null()); // no id field ever exists to populate
        assert_eq!(manifest["session"]["label"], "2026-09-20 — target");
        assert_eq!(manifest["producer"]["name"], "astro-sync-cli");
    }

    fn fixture_with_one_fits_file() -> (TempDir, DryRunReport) {
        let source = TempDir::new("package-fixture-analysis");
        fs::write(source.path().join("frame-0001.fits"), vec![1_u8; 16]).unwrap();
        let scan = scan_folder(source.path()).unwrap();
        let report = build_dry_run_report(&scan);
        (source, report)
    }

    #[test]
    fn append_analysis_assets_adds_metric_log_and_event_log_entries() {
        let (_source, report) = fixture_with_one_fits_file();
        let output_parent = TempDir::new("package-analysis-append");
        let outcome = produce_package(&report, output_parent.path(), None, None).unwrap();

        let analysis = AnalysisArtifacts {
            quality_csv: Some("timestampUtc\n".to_string()),
            guiding_csv: Some("timestampUtc\n".to_string()),
            session_jsonl: Some("{\"type\":\"dither\"}\n".to_string()),
        };
        let updated = append_analysis_assets(&outcome, &analysis).unwrap();

        assert_eq!(updated.asset_count, outcome.asset_count + 3);
        assert!(updated.output_dir.join("metrics/quality.csv").is_file());
        assert!(updated.output_dir.join("metrics/guiding.csv").is_file());
        assert!(updated.output_dir.join("events/session.jsonl").is_file());

        let manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&updated.manifest_path).unwrap()).unwrap();
        let assets = manifest["assets"].as_array().unwrap();
        assert!(
            assets
                .iter()
                .any(|a| a["path"] == "metrics/quality.csv" && a["kind"] == "metric-log")
        );
        assert!(
            assets
                .iter()
                .any(|a| a["path"] == "metrics/guiding.csv" && a["kind"] == "metric-log")
        );
        assert!(
            assets
                .iter()
                .any(|a| a["path"] == "events/session.jsonl" && a["kind"] == "event-log")
        );

        let checksums = fs::read_to_string(&updated.checksums_path).unwrap();
        assert!(checksums.contains("  metrics/quality.csv"));
        assert!(checksums.contains("  metrics/guiding.csv"));
        assert!(checksums.contains("  events/session.jsonl"));
    }

    #[test]
    fn append_analysis_assets_refuses_to_overwrite_an_existing_asset_path() {
        // `fixture_with_two_files` already places a hand-written
        // `quality.csv` in the source folder, which `produce_package`
        // copies through to `metrics/quality.csv` as a plain `metric-log`
        // asset -- exactly the collision this function must refuse.
        let (_source, report) = fixture_with_two_files();
        let output_parent = TempDir::new("package-analysis-collision");
        let outcome = produce_package(&report, output_parent.path(), None, None).unwrap();

        let analysis = AnalysisArtifacts {
            quality_csv: Some("timestampUtc\n".to_string()),
            guiding_csv: None,
            session_jsonl: None,
        };
        let result = append_analysis_assets(&outcome, &analysis);
        assert!(matches!(
            &result,
            Err(PackageError::AnalysisPathCollision(path)) if path == "metrics/quality.csv"
        ));
        // Nothing should have been written on the collision path.
        assert!(
            !output_parent
                .path()
                .join(PACKAGE_OUTPUT_DIR_NAME)
                .join("events/session.jsonl")
                .exists()
        );
    }

    #[test]
    fn append_analysis_assets_is_a_no_op_when_nothing_is_provided() {
        let (_source, report) = fixture_with_one_fits_file();
        let output_parent = TempDir::new("package-analysis-noop");
        let outcome = produce_package(&report, output_parent.path(), None, None).unwrap();
        let updated = append_analysis_assets(&outcome, &AnalysisArtifacts::default()).unwrap();
        assert_eq!(updated.asset_count, outcome.asset_count);
        assert_eq!(updated.total_bytes, outcome.total_bytes);
    }

    /// A `ProgressionRecord` with two milestone files actually written to
    /// disk under `working_dir` -- `append_progression_assets` reads each
    /// selected milestone's bytes from `MilestoneOutcome::output_path`, so
    /// the fixture needs real files there, not just struct data.
    fn sample_progression_record(working_dir: &Path) -> ProgressionRecord {
        let milestone_01 = working_dir.join("milestone_01.fit");
        let milestone_02 = working_dir.join("milestone_02.fit");
        fs::write(&milestone_01, b"fake-fits-bytes-milestone-01").unwrap();
        fs::write(&milestone_02, b"fake-fits-bytes-milestone-02-longer").unwrap();

        ProgressionRecord {
            siril_version: "1.2.1".to_string(),
            script: "requires 1.2.1\n".to_string(),
            script_path: working_dir.join("progression.ssf").display().to_string(),
            settings: ProgressionSettings::default(),
            acquisition_family: AcquisitionFamily {
                key: "Ha_300s_g100_1x1_t-10C".to_string(),
                filter: Some("Ha".to_string()),
                exposure_seconds: Some(300.0),
                gain: Some(100.0),
                binning: Some("1x1".to_string()),
                temperature_class_c: Some(-10),
                camera: None,
                frame_paths: vec!["frame-0001.fits".to_string(), "frame-0002.fits".to_string()],
            },
            milestones: vec![
                MilestoneOutcome {
                    index: 1,
                    frame_count: 1,
                    label: "milestone_01".to_string(),
                    output_path: milestone_01.display().to_string(),
                },
                MilestoneOutcome {
                    index: 2,
                    frame_count: 2,
                    label: "milestone_02".to_string(),
                    output_path: milestone_02.display().to_string(),
                },
            ],
            final_stack_display_scale: DisplayScale {
                width_pixels: 1024,
                height_pixels: 768,
            },
            generated_at: "2026-09-27T12:00:00Z".to_string(),
        }
    }

    #[test]
    fn append_progression_assets_adds_only_the_selected_milestone_and_the_record() {
        let (_source, report) = fixture_with_one_fits_file();
        let output_parent = TempDir::new("package-progression-append");
        let outcome = produce_package(&report, output_parent.path(), None, None).unwrap();

        let working_dir = TempDir::new("package-progression-working");
        let record = sample_progression_record(working_dir.path());

        let updated =
            append_progression_assets(&outcome, &record, &["milestone_02".to_string()]).unwrap();

        // One selected milestone image plus the record itself.
        assert_eq!(updated.asset_count, outcome.asset_count + 2);
        assert!(
            updated
                .output_dir
                .join("progressions/milestone_02.fit")
                .is_file()
        );
        assert!(
            !updated
                .output_dir
                .join("progressions/milestone_01.fit")
                .exists(),
            "an unselected milestone must never be copied into the package"
        );
        assert!(
            updated
                .output_dir
                .join("progressions/progression.json")
                .is_file()
        );

        let manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&updated.manifest_path).unwrap()).unwrap();
        let assets = manifest["assets"].as_array().unwrap();
        assert!(assets.iter().any(
            |a| a["path"] == "progressions/milestone_02.fit" && a["kind"] == "progression"
        ));
        assert!(
            assets
                .iter()
                .any(|a| a["path"] == "progressions/progression.json" && a["kind"] == "other")
        );

        let recorded_json: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(updated.output_dir.join("progressions/progression.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(recorded_json["sirilVersion"], "1.2.1");
        assert_eq!(recorded_json["milestones"].as_array().unwrap().len(), 2);

        let checksums = fs::read_to_string(&updated.checksums_path).unwrap();
        assert!(checksums.contains("  progressions/milestone_02.fit"));
        assert!(checksums.contains("  progressions/progression.json"));
        assert!(!checksums.contains("milestone_01.fit"));
    }

    #[test]
    fn append_progression_assets_is_a_no_op_when_nothing_is_selected() {
        let (_source, report) = fixture_with_one_fits_file();
        let output_parent = TempDir::new("package-progression-noop");
        let outcome = produce_package(&report, output_parent.path(), None, None).unwrap();

        let working_dir = TempDir::new("package-progression-noop-working");
        let record = sample_progression_record(working_dir.path());

        let updated = append_progression_assets(&outcome, &record, &[]).unwrap();
        assert_eq!(updated.asset_count, outcome.asset_count);
        assert_eq!(updated.total_bytes, outcome.total_bytes);
        assert!(!updated.output_dir.join("progressions").exists());
    }

    #[test]
    fn append_progression_assets_refuses_an_unknown_milestone_label() {
        let (_source, report) = fixture_with_one_fits_file();
        let output_parent = TempDir::new("package-progression-unknown-label");
        let outcome = produce_package(&report, output_parent.path(), None, None).unwrap();

        let working_dir = TempDir::new("package-progression-unknown-label-working");
        let record = sample_progression_record(working_dir.path());

        let result = append_progression_assets(&outcome, &record, &["milestone_99".to_string()]);
        assert!(matches!(
            result,
            Err(PackageError::UnknownMilestoneLabel(label)) if label == "milestone_99"
        ));
    }

    #[test]
    fn append_progression_assets_refuses_to_overwrite_an_already_added_milestone() {
        let (_source, report) = fixture_with_one_fits_file();
        let output_parent = TempDir::new("package-progression-collision");
        let outcome = produce_package(&report, output_parent.path(), None, None).unwrap();

        let working_dir = TempDir::new("package-progression-collision-working");
        let record = sample_progression_record(working_dir.path());

        let once =
            append_progression_assets(&outcome, &record, &["milestone_01".to_string()]).unwrap();
        let result = append_progression_assets(&once, &record, &["milestone_01".to_string()]);
        assert!(matches!(
            &result,
            Err(PackageError::ProgressionPathCollision(path)) if path == "progressions/milestone_01.fit"
        ));
    }
}
