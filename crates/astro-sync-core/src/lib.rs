//! `astro-sync-core` is the library crate holding Astro Sync's behavior.
//! Both `astro-sync-cli` and the Tauri app under `app/` depend on this
//! crate for every mutating action; neither ever reimplements behavior
//! itself. The non-negotiable invariant that governs every module here:
//!
//! **Astro Sync never copies, moves, renames, replaces or deletes a file
//! of the person's own** (the frames, the logs, the final images, whatever
//! they copied, moved or linked into the working folder; a link is never
//! followed). Its only writes there are, and will stay:
//!
//! - the folders of the working folder ([`working_folder`]): the root, a
//!   night's folder, `logs/`, `calibration/…/`, a ticked target's folder
//!   with `lights/` and `final/` -- created when absent, never removed;
//! - its two identity files, `.astro-sync-night.json` and
//!   `.astro-sync-target.json`, created only when absent (never replaced);
//! - `ASSISTANT-IA.md` at the root, rewritten only while it still holds the
//!   bytes Astro Sync wrote;
//! - the stacked images under `<target>/stacked/` (the stacking).
//! - the stacking's scratch under `<night>/.astro-sync-work/stacking/`
//! : hidden, Astro Sync's own, created for a stacking and removed
//!   at its end -- and skipped by every reading of the night.
//! - what its own pipelines produced, which only "Clean" of Settings
//!   removes, on the person's request and nowhere else
//!   ([`processing_files`]): the masters and the results of `stacked/`, the
//!   scratch, never a file Astro Sync did not make.
//! - the provisional night and plan folders of the import of sessions
//! ([`session_detection::provisional`]): made of links and
//!   of Astro Sync's results, renamed once the space created their session
//!   (their record rewritten, the references Astro Sync made to them with
//!   it), removed by « Abandonner l'import » -- links and Astro Sync's own
//!   files only: a file of the person's left there keeps its folder.
//!
//! Every other file is read only: the frames, the logs, the final images.
//! `read_only_invariant_tests` (below) holds the line for all of them.
//!
//! Modules:
//!
//! - [`working_folder`]: the root per system, a night folder's
//!   name and structure, the identity files, a night found by identity.
//! - [`detection`]: the rules of the night folder, read from the
//!   folders and the first header of each frame (FITS, and XISF through
//!   [`xisf`]); [`nfc`] for the names compared.
//! - [`local_work`], [`digest_cache`]: the stop switch and the
//!   progress of a long local work, and the digests kept by (path, size,
//!   time); [`local_settings`]: the working folder chosen, the night folders
//!   found again, the UTC offsets of the logs.
//! - [`night_analysis`]: the night's report v2 from its logs
//!   (NINA, PHD2, ASIAIR) and frame headers, with the measured sky
//!   background; sent so that it replaces the night's analysis, never empty.
//! - [`precheck`], [`night_online`]: before anything is sent,
//!   what is online for a night, each image's SHA-256 against it and against
//!   `POST /v1/uploads/check` (identical, new capture, variant, "will not be
//!   sent"), the four limits of `GET /v1/space/storage`, the
//!   treatments the capabilities and the tools allow.
//! - [`night_resume`]: a night's state, the step its wizard
//!   reopens at, its complements, the original of an image fetched to solve
//!   it.
//! - [`sky_solution`]: the sky solution of the image that
//!   is sent -- a header already solved, else ASTAP or astrometry.net run as
//!   external tools ([`external_process`]; [`raster`] reads an image's
//!   size) --, `PUT /v1/captures/{id}/sky-solution` and the captures still
//!   without one.
//! - [`tools`]: Settings › "Dependencies" from the
//!   dependency registry (`stacking/dependencies/registry.json`, the one
//!   source): detection, download by digest, resume.
//! - [`pipelines`], [`stacking`]: the render pipelines,
//!   and the slot of the stacking (output in `<target>/stacked/` only, never
//!   gated by a capability, the pipelines chosen and the trusted digest of a
//!   script passed to the launcher).
//! - [`progression_images`]: Siril with fixed settings,
//!   a JPEG per milestone, in Astro Sync's own data.
//! - [`assistant_prompt`]: `ASSISTANT-IA.md` and the
//!   prompt in the application's language.
//! - [`folder_files`] and [`guide_prompt`]: the
//!   `README.md` and `GUIDE-ASTRO-SYNC.<language>.md` files of the working
//!   folder, under the rule of `ASSISTANT-IA.md`, and the prompt "Ask an AI
//!   a question" in the application's language.
//! - [`device_master`]: the image a device already
//!   stacked (the Seestar's `Stacked_*.fit`), recognised from its header, found
//!   in `<target>/device/`, its families and the default source of each; the
//!   stacking renders it instead of stacking the poses again.
//! - [`night_steps`]: the steps of a night composed once
//!   for the window and the command line.
//! - [`night_service`]: a night of the space read
//!   (`GET /v1/sessions/{id}`) and created from "Nights" (`POST
//!   /v1/sessions`, `PATCH …/night`, `POST …/finish`).
//! - [`incidents`]: the "Incident list" engine -- the
//!   incidents a night's logs and frame headers show (pauses, guide star
//!   losses, guiding timeouts, exposure and temperature changes), the
//!   signals already declared in the session, and their writing (`POST
//!   /v1/sessions/{id}/events`, then `PUT /v1/sessions/{id}/rework`).
//! - [`association`]: links a hosted session id to a local folder.
//! - [`scanner`]: the read-only walk, classification, and SHA-256 hashing.
//! - [`dry_run`]: the exact file/byte/hash/destination report shown before
//!   a package is produced.
//! - [`package`]: the open session-package producer (see
//!   `contracts/schemas/session-package/`).
//! - [`upload_queue`]: the persistent, SQLite-backed queue of the results still
//!   to be uploaded (final, stacked and progression images only), and their
//!   resumable server-reported offset.
//! - [`uploader`]: the resumable-upload HTTP client
//!   (`contracts/schemas/api/uploads.md`) over an injectable
//!   transport, driving a [`upload_queue::UploadQueue`] entry from create
//!   through finalize.
//! - [`refusal`]: what Astro Sync says for each refusal the service can
//!   answer to an upload, an analysis or a pairing -- the stable code the
//!   window looks up in its three languages, and the sentence in French of
//!   the command line -- and never a raw code or the service's own English.
//! - [`canonical_json`]: sorted-key, whitespace-free JSON serialization
//! shared by the CLI's golden tests and the Tauri GUI.
//! - [`gui_bridge`]: the handful of behaviors specific to the Tauri GUI
//! that are not part of the CLI's scope -- settings, device
//!   token storage, device pairing, the portal's session list, and the
//!   upload-queue driving loop the GUI's "upload all" action reuses.
//! - [`service_wait`]: the patient HTTP client both transports send
//!   through. A service can sleep when nobody uses it and answer the
//!   first request late; a call that gets no answer because
//!   the service is starting is announced and retried within a budget, a
//!   service that cannot be reached fails at once, and only a call whose
//!   repeat is harmless is ever retried (see the module's own
//!   documentation, and `README.md`, "A sleeping hosted
//!   service").
//! - [`space`]: signing in to the hosted service by e-mail code,
//!   announcing itself as the application `sync`; what the
//!   service announces (a private server keeps its pairing), the request and
//!   verification of a code, the token kept like a pairing's, and the end of a
//!   sign-in the service pronounces (`device_detached`, `space_deleted`)
//!   without losing a folder association or a pending upload.
//! - [`analysis`]: local guiding-log/FITS-header
//!   analysis (`analysis::phd2`, `analysis::asiair`, `analysis::fits`) and
//!   the writers for `metrics/quality.csv`, `metrics/guiding.csv`, and
//!   `events/session.jsonl` (see
//!   `contracts/schemas/session-package/metrics.md`), tied
//!   together by `analysis::analyze_folder`.

mod nfc_table;
mod paths;
#[cfg(test)]
mod test_support;
mod time_util;

pub mod analysis;
pub mod assistant_prompt;
pub mod association;
pub mod build_info;
pub mod canonical_json;
pub mod catalogue_targets;
pub mod client_identity;
pub mod detection;
pub mod device_master;
pub mod digest_cache;
pub mod dry_run;
pub mod error_report;
pub mod external_process;
pub mod folder_files;
pub mod gui_bridge;
pub mod guide_prompt;
pub mod image_texts;
pub mod incidents;
pub mod local_settings;
pub mod local_work;
pub mod master_quality;
pub mod metadata;
pub mod multi_night;
pub mod multi_night_attach;
pub mod nfc;
pub mod night_analysis;
pub mod night_online;
pub mod night_resume;
pub mod night_service;
pub mod night_steps;
pub mod package;
pub mod pipeline_trust;
pub mod pipelines;
pub mod plan_image;
pub mod plan_progression;
pub mod pose_stats;
pub mod precheck;
pub mod processing_files;
pub mod progression_images;
pub mod python_env;
pub mod raster;
pub mod raw_header;
pub mod refusal;
pub mod scanner;
pub mod service_wait;
pub mod session_detection;
pub mod siril_progress;
pub mod sky_after_upload;
pub mod sky_solution;
pub mod space;
pub mod stacking;
pub mod standing;
pub mod tools;
pub mod upload_queue;
pub mod uploader;
pub mod working_folder;
pub mod xisf;

pub use canonical_json::canonical_json;
pub use time_util::{DETERMINISTIC_TIME_ENV_VAR, now_iso8601_utc};

/// The crate's own semantic version, exposed so the CLI and GUI can report
/// which core they were built against without duplicating the version
/// string from `Cargo.toml`.
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The crate's version is the workspace's (`Cargo.toml` at the root of
    /// the repository), the one the release tag carries.
    #[test]
    fn version_matches_cargo_toml() {
        let workspace = include_str!("../../../Cargo.toml");
        let section = workspace
            .split("[workspace.package]")
            .nth(1)
            .expect("a [workspace.package] section");
        let line = section
            .lines()
            .map(str::trim)
            .find(|line| line.starts_with("version"))
            .expect("a version line");
        let expected = line.split('"').nth(1).expect("a quoted version");
        assert_eq!(version(), expected);
    }

    #[test]
    fn version_is_never_empty() {
        assert!(!version().is_empty());
    }
}

/// The read-only enforcement test, kept for every file that is not Astro Sync's
/// own: running a scan, a dry-run, a full package production, the "Folder"
/// step, the detection and the analysis over a night must never change a single
/// file's bytes or modification time, and must add nothing but folders and the
/// two identity files.
#[cfg(test)]
mod read_only_invariant_tests {
    use crate::{
        analysis, detection, dry_run, package, scanner, test_support::TempDir, working_folder,
    };
    use sha2::{Digest, Sha256};
    use std::fs;
    use std::path::Path;
    use std::time::SystemTime;

    #[derive(Debug, PartialEq)]
    struct FileSnapshot {
        bytes: u64,
        modified: SystemTime,
        sha256: String,
    }

    fn snapshot(root: &Path, relative_paths: &[&str]) -> Vec<(String, FileSnapshot)> {
        relative_paths
            .iter()
            .map(|relative| {
                let path = root.join(relative);
                let metadata = fs::metadata(&path).unwrap();
                let contents = fs::read(&path).unwrap();
                let mut hasher = Sha256::new();
                hasher.update(&contents);
                let snapshot = FileSnapshot {
                    bytes: metadata.len(),
                    modified: metadata.modified().unwrap(),
                    sha256: format!("{:x}", hasher.finalize()),
                };
                ((*relative).to_string(), snapshot)
            })
            .collect()
    }

    /// Every file under `root`, hidden ones included, sorted.
    fn every_file(root: &Path) -> Vec<String> {
        let mut files: Vec<String> = walkdir::WalkDir::new(root)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file())
            .map(|entry| {
                entry
                    .path()
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        files.sort();
        files
    }

    #[test]
    fn scan_dry_run_and_package_never_modify_a_source_file() {
        let source = TempDir::new("read-only-source");
        let relative_paths = [
            "frame-0001.fits",
            "quality.csv",
            "notes.txt",
            "raw.dat",
            "sub/frame-0002.fits",
        ];
        fs::create_dir(source.path().join("sub")).unwrap();
        fs::write(source.path().join("frame-0001.fits"), vec![7_u8; 128]).unwrap();
        fs::write(source.path().join("quality.csv"), b"fwhm,rms\n1.2,0.4\n").unwrap();
        fs::write(source.path().join("notes.txt"), b"observing notes").unwrap();
        fs::write(source.path().join("raw.dat"), vec![3_u8; 40]).unwrap();
        fs::write(source.path().join("sub/frame-0002.fits"), vec![9_u8; 64]).unwrap();

        let before = snapshot(source.path(), &relative_paths);

        let scan = scanner::scan_folder(source.path()).unwrap();
        assert_eq!(scan.files.len(), 5);
        let report = dry_run::build_dry_run_report(&scan);
        let output_parent = TempDir::new("read-only-output");
        let outcome = package::produce_package(&report, output_parent.path(), None, None).unwrap();
        assert_eq!(outcome.asset_count, 5);

        let after = snapshot(source.path(), &relative_paths);
        assert_eq!(
            before, after,
            "scanning, dry-running, and producing a package must never change a source file's bytes or modification time"
        );
    }

    #[test]
    fn the_working_folder_writes_only_folders_and_its_identity_files() {
        let root = TempDir::new("read-only-working-folder");
        let spec = working_folder::NightFolderSpec {
            night_id: "7f3a2c91-invariant".to_string(),
            service_url: "https://service.example".to_string(),
            evening_date: "2026-10-07".to_string(),
            label: "Col des Étoiles".to_string(),
            targets: vec![working_folder::NightTargetSpec {
                target_id: "t-m31".to_string(),
                designation: "M31".to_string(),
                ticked: true,
            }],
            follow: None,
        };
        // The person's files, put in before the "Folder" step: in the
        // folder named as Astro Sync would name it, and in the target's.
        let night = root.path().join("2026-10-07 · Col des Étoiles · 7f3a2c");
        let theirs = [
            "M31/lights/Light_0001.fits",
            "M31/final/M31.tif",
            "logs/PHD2_GuideLog_2026-10-07.txt",
            "notes.txt",
        ];
        for relative in theirs {
            let path = night.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, relative.as_bytes()).unwrap();
        }
        let before = snapshot(&night, &theirs);
        let files_before = every_file(root.path());

        let now = "2026-10-08T09:00:00Z";
        let outcome = working_folder::prepare_night_folder(root.path(), &[], &spec, now).unwrap();
        assert_eq!(outcome.folder, night);
        working_folder::prepare_night_folder(root.path(), &[], &spec, now).unwrap();
        scanner::scan_folder(&night).unwrap();
        let input = detection::DetectionInput {
            evening_date: Some("2026-10-07".to_string()),
            night_started_at: Some("2026-10-07T20:00:00Z".to_string()),
            longitude: None,
            targets: vec![detection::DetectionTarget {
                target_id: "t-m31".to_string(),
                designation: "M31".to_string(),
                aliases: Vec::new(),
                ticked: true,
                online_final: false,
                stacking_planned: false,
            }],
        };
        detection::detect_night(&night, &input, &detection::DetectOptions::default()).unwrap();
        analysis::analyze_folder(&night, &analysis::AnalysisOptions::default()).unwrap();

        assert_eq!(
            before,
            snapshot(&night, &theirs),
            "the person's files are untouched"
        );
        let mut expected = files_before;
        let prefix = night
            .strip_prefix(root.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        expected.push(format!("{prefix}/.astro-sync-night.json"));
        expected.push(format!("{prefix}/M31/.astro-sync-target.json"));
        expected.sort();
        assert_eq!(
            every_file(root.path()),
            expected,
            "nothing but the two identity files is added"
        );
    }
}
