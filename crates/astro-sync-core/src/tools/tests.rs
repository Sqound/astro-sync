use super::*;
use crate::test_support::TempDir;
use std::sync::Mutex;

#[test]
fn the_embedded_registry_is_the_one_source_of_the_tools() {
    let registry = registry();
    let ids: Vec<&str> = registry
        .dependencies
        .iter()
        .map(|e| e.id.as_str())
        .collect();
    for id in ["siril", "starnet2", "python-env", "astap", "astrometry-net"] {
        assert!(ids.contains(&id), "{id} in {ids:?}");
    }
    let siril = registry.entry("siril").unwrap();
    assert_eq!(
        siril.check.as_ref().unwrap().command,
        vec!["siril-cli", "siril"]
    );
    assert_eq!(localized(&siril.name, "pt-BR"), "Siril");
    let python = registry.entry("python-env").unwrap();
    assert_eq!(localized(&python.name, "ja"), "Python 環境");
    assert_eq!(localized(&python.name, "ko"), "Python environment");

    // A registry that breaks the format is refused whole.
    let bad = REGISTRY_JSON.replacen(
        "\"page\": \"https://siril.org/download/\"",
        "\"page\": \"https://siril.org/download/\", \"downloads\": {\"macos-arm64\": {\"url\": \"https://example.invalid/siril.dmg\", \"sha256\": \"\"}}",
        1,
    );
    assert!(parse_registry(&bad).unwrap_err().contains("SHA-256"));
    let unknown = REGISTRY_JSON.replacen(
        "\"page\": \"https://siril.org/download/\"",
        &format!("\"page\": \"https://siril.org/download/\", \"downloads\": {{\"amiga\": {{\"url\": \"https://example.invalid/s\", \"sha256\": \"{}\"}}}}", "a".repeat(64)),
        1,
    );
    assert!(
        parse_registry(&unknown)
            .unwrap_err()
            .contains("unknown system")
    );
    // The size of what the person downloads, by system, for the
    // tools with no automatic download and for the Python packages
    // ("Download (90 MB)"); a size of nothing is refused.
    for id in ["siril", "starnet2", "python-env"] {
        let entry = registry.entry(id).unwrap();
        for system in SYSTEMS {
            assert!(
                entry.sizes.get(system).is_some_and(|&b| b > 10_000_000),
                "{id} {system}"
            );
        }
        assert_eq!(
            download_bytes(entry),
            this_system().and_then(|s| entry.sizes.get(s).copied())
        );
    }
    let graxpert = registry.entry("graxpert").unwrap();
    assert_eq!(
        download_bytes(graxpert),
        this_system()
            .and_then(|s| graxpert.downloads.get(s))
            .and_then(|d| d.bytes)
    );
    let empty = REGISTRY_JSON.replacen("\"macos-arm64\": 75765778", "\"macos-arm64\": 0", 1);
    assert!(parse_registry(&empty).unwrap_err().contains("positive"));
}

#[test]
fn versions_are_read_with_the_registry_patterns() {
    let siril = r"(?i)siril[^\d\n]*(\d+(?:\.\d+)+)";
    assert_eq!(
        read_version(siril, "siril 1.4.4\n").as_deref(),
        Some("1.4.4")
    );
    assert_eq!(
        read_version(siril, "Siril-CLI 1.2.1-git (64 bits)").as_deref(),
        Some("1.2.1")
    );
    assert_eq!(read_version(siril, "nothing here 3.2"), None);
    let starnet = r"(?i)starnet2?[^\d\n]*(\d+(?:\.\d+)+)";
    assert_eq!(
        read_version(starnet, "StarNet2 v2.5.4").as_deref(),
        Some("2.5.4")
    );
    assert_eq!(
        read_version(starnet, "starnet ver 2.0.1").as_deref(),
        Some("2.0.1")
    );
    // Another shape of pattern: the first dotted number.
    assert_eq!(
        read_version("^v(.*)$", "tool 7 build v3.10.2").as_deref(),
        Some("3.10.2")
    );
    assert!(at_least("1.4.4", "1.4"));
    assert!(at_least("1.4", "1.4.0"));
    assert!(!at_least("1.3.9", "1.4"));
    assert!(at_least("2026.10.01", "2024.5"));
}

#[cfg(unix)]
#[test]
fn the_rows_say_what_is_present_missing_or_too_old() {
    use crate::external_process::fake_tool;
    let dir = TempDir::new("tools-detect");
    let bin = dir.path().join("bin");
    fs::create_dir_all(&bin).unwrap();
    fake_tool(&bin, "siril-cli", "echo 'siril 1.4.4'");
    fake_tool(&bin, "starnet2", "echo 'StarNet2 v1.0.0'");
    fake_tool(&bin, "astap_cli", "exit 0");
    let data = dir.path().join("data");
    let database = data.join("tools/astap-database");
    fs::create_dir_all(&database).unwrap();
    fs::write(database.join("d50_0101.1476"), b"stars").unwrap();
    let stars = data.join("reference-stars.bin");
    let context = DetectContext {
        data_dir: data.clone(),
        reference_stars: stars.clone(),
        search_dirs: vec![bin.clone()],
        known_paths: BTreeMap::new(),
        probe_timeout: Duration::from_secs(10),
        stacking: None,
    };
    let rows = detect_tools(&context);
    let state = |id: &str| row(&rows, id).unwrap().state;
    assert_eq!(state("siril"), ToolState::Present);
    assert_eq!(
        row(&rows, "siril").unwrap().version.as_deref(),
        Some("1.4.4")
    );
    assert_eq!(
        row(&rows, "siril").unwrap().progression_compatible,
        Some(true)
    );
    assert_eq!(state("starnet2"), ToolState::Outdated);
    assert_eq!(state("python-env"), ToolState::Missing);
    // No Python in the range on this computer: python.org only.
    assert_eq!(row(&rows, "python-env").unwrap().blocked_by, vec!["python"]);
    assert!(!row(&rows, "python-env").unwrap().installable);
    assert_eq!(state("astap"), ToolState::Present);
    assert_eq!(state("astap-database"), ToolState::Present);
    assert_eq!(state("astrometry-net"), ToolState::Missing);
    assert_eq!(state("tycho-2"), ToolState::Missing);
    assert_eq!(state("stacking-pipeline"), ToolState::Missing);
    assert_eq!(
        row(&rows, "stacking-pipeline").unwrap().blocked_by,
        vec!["python"]
    );
    assert!(!row(&rows, "astrometry-net").unwrap().required);
    assert!(row(&rows, "astap").unwrap().required);
    let order: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
    let astap = order.iter().position(|id| *id == "astap").unwrap();
    assert_eq!(order[astap + 1], "astap-database");

    let present = tools_present(&rows);
    assert!(present.astap && present.siril && !present.stacking_pipeline);
    let solvers = solvers(&rows);
    assert_eq!(
        solvers.astap.as_ref().unwrap().database.as_deref(),
        Some(database.as_path())
    );
    assert!(solvers.astrometry_net.is_none());
    assert_eq!(siril_for_progression(&rows).unwrap().1, "1.4.4");

    // The stacking's environment made and checked: the pipeline is
    // present while Siril is; without Siril it waits for it.
    let stacking = dir.path().join("stacking");
    fs::create_dir_all(&stacking).unwrap();
    for file in ["astro_sync_stack.py", "check_environment.py"] {
        fs::write(stacking.join(file), b"").unwrap();
    }
    fs::write(stacking.join("requirements.txt"), "numpy==2.5.1\n").unwrap();
    let env_bin = data.join("stacking-env/bin");
    fs::create_dir_all(&env_bin).unwrap();
    fake_tool(&env_bin, "python3", "exit 0");
    fs::write(
        data.join("stacking-env/astro-sync-env.json"),
        format!(
            r#"{{"format": 1, "pythonVersion": "3.13.7", "requirementsSha256": "{:x}"}}"#,
            Sha256::digest(b"numpy==2.5.1\n")
        ),
    )
    .unwrap();
    let with_env = DetectContext {
        stacking: StackingResources::find(&stacking),
        ..context.clone()
    };
    let rows = detect_tools(&with_env);
    let pipeline = row(&rows, "stacking-pipeline").unwrap();
    assert_eq!(pipeline.state, ToolState::Present);
    assert_eq!(pipeline.version.as_deref(), Some("3.13.7"));
    assert!(pipeline.blocked_by.is_empty());
    assert!(tools_present(&rows).stacking_pipeline);
    fs::remove_file(bin.join("siril-cli")).unwrap();
    let rows = detect_tools(&with_env);
    let pipeline = row(&rows, "stacking-pipeline").unwrap();
    assert_eq!(pipeline.state, ToolState::Missing);
    assert_eq!(pipeline.blocked_by, vec!["siril"]);
    assert!(!tools_present(&rows).stacking_pipeline);
    fake_tool(&bin, "siril-cli", "echo 'siril 1.4.4'");

    // The list downloaded, an old Siril the progression does not know.
    fs::write(&stars, b"ASRSTAR1").unwrap();
    fake_tool(&bin, "siril-cli", "echo 'siril 1.3.0'");
    let rows = detect_tools(&context);
    assert_eq!(row(&rows, "tycho-2").unwrap().state, ToolState::Present);
    assert_eq!(row(&rows, "siril").unwrap().state, ToolState::Outdated);
    assert!(!tools_present(&rows).siril);
}

/// A server: the file, cut after `cut` bytes on the first request.
struct FakeServer {
    file: Vec<u8>,
    cut: Mutex<Option<usize>>,
    asked_from: Mutex<Vec<u64>>,
    honours_range: bool,
}

struct CutReader {
    bytes: Vec<u8>,
    at: usize,
    cut: Option<usize>,
}

impl Read for CutReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if self.cut.is_some_and(|cut| self.at >= cut) {
            return Err(std::io::Error::other("connection reset"));
        }
        let end = self
            .cut
            .map_or(self.bytes.len(), |cut| cut.min(self.bytes.len()))
            .min(self.at + out.len().min(1000));
        let n = end - self.at;
        out[..n].copy_from_slice(&self.bytes[self.at..end]);
        self.at = end;
        Ok(n)
    }
}

impl ToolFetcher for FakeServer {
    fn fetch(&self, _url: &str, from: u64) -> Result<Fetched, String> {
        self.asked_from.lock().unwrap().push(from);
        let start = if self.honours_range { from } else { 0 };
        let cut = self.cut.lock().unwrap().take();
        Ok(Fetched {
            start,
            total: Some(self.file.len() as u64),
            body: Box::new(CutReader {
                bytes: self.file[start as usize..].to_vec(),
                at: 0,
                cut,
            }),
        })
    }
}

fn offer(file: &[u8]) -> Download {
    Download {
        url: "https://downloads.example.invalid/astap/astap_cli_2026.10.zip?x=1".to_string(),
        sha256: format!("{:x}", Sha256::digest(file)),
        bytes: Some(file.len() as u64),
    }
}

#[test]
fn a_download_is_checked_by_its_digest_and_resumed_after_a_failure() {
    let dir = TempDir::new("tools-download");
    let file: Vec<u8> = (0..10_000_u32).map(|i| (i % 251) as u8).collect();
    let server = FakeServer {
        file: file.clone(),
        cut: Mutex::new(Some(4_000)),
        asked_from: Mutex::new(Vec::new()),
        honours_range: true,
    };
    let download = offer(&file);
    let mut seen = Vec::new();
    let failure = download_tool(
        dir.path(),
        "astap",
        Some(&download),
        &server,
        None,
        &mut |p| seen.push(p.bytes),
    )
    .unwrap_err();
    match failure {
        DownloadFailure::Interrupted {
            kept_bytes, total, ..
        } => {
            assert_eq!(kept_bytes, 4_000);
            assert_eq!(total, Some(10_000));
        }
        other => panic!("{other:?}"),
    }
    let part = dir.path().join("tools/astap/astap_cli_2026.10.zip.part");
    assert_eq!(fs::metadata(&part).unwrap().len(), 4_000);

    let done = download_tool(
        dir.path(),
        "astap",
        Some(&download),
        &server,
        None,
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(*server.asked_from.lock().unwrap(), vec![0, 4_000]);
    assert_eq!(fs::read(&done.path).unwrap(), file);
    assert!(!part.exists());
    assert!(!done.already_there);

    // Asked again: the file is there with its digest, nothing is fetched.
    let again = download_tool(
        dir.path(),
        "astap",
        Some(&download),
        &server,
        None,
        &mut |_| {},
    )
    .unwrap();
    assert!(again.already_there);
    assert_eq!(server.asked_from.lock().unwrap().len(), 2);
}

#[test]
fn a_file_that_is_not_the_registrys_is_refused_and_a_system_without_download_has_the_page() {
    let dir = TempDir::new("tools-download-refused");
    let server = FakeServer {
        file: b"something else entirely".to_vec(),
        cut: Mutex::new(None),
        asked_from: Mutex::new(Vec::new()),
        honours_range: false,
    };
    let download = offer(b"the real file");
    assert_eq!(
        download_tool(
            dir.path(),
            "siril",
            Some(&download),
            &server,
            None,
            &mut |_| {}
        )
        .unwrap_err(),
        DownloadFailure::DigestMismatch
    );
    assert!(
        fs::read_dir(dir.path().join("tools/siril"))
            .unwrap()
            .next()
            .is_none()
    );
    assert_eq!(
        download_tool(dir.path(), "siril", None, &server, None, &mut |_| {}).unwrap_err(),
        DownloadFailure::NoAutomaticDownload
    );
    let cancel = CancelFlag::new();
    cancel.cancel();
    assert!(matches!(
        download_tool(
            dir.path(),
            "siril",
            Some(&offer(b"x")),
            &server,
            Some(&cancel),
            &mut |_| {}
        )
        .unwrap_err(),
        DownloadFailure::Cancelled { .. }
    ));
}

#[test]
fn install_all_takes_the_required_missing_rows_with_a_download() {
    let mut rows = detect_tools(&DetectContext {
        data_dir: TempDir::new("tools-install-all").path().to_path_buf(),
        reference_stars: PathBuf::from("/nonexistent/reference-stars.bin"),
        search_dirs: Vec::new(),
        known_paths: BTreeMap::new(),
        probe_timeout: Duration::from_secs(1),
        stacking: None,
    });
    // The registry gives ASTAP and its star database a download on each
    // system it knows: here, both are to install, the program first.
    let ids: Vec<&str> = to_install(&rows).iter().map(|r| r.id.as_str()).collect();
    if this_system().is_some() {
        assert_eq!(ids, vec!["astap", "astap-database"]);
    } else {
        assert!(ids.is_empty());
    }
    for row in &mut rows {
        if row.id == "astap" || row.id == "astrometry-net" {
            row.download = Some(offer(b"x"));
        }
    }
    let ids: Vec<&str> = to_install(&rows).iter().map(|r| r.id.as_str()).collect();
    assert_eq!(
        ids,
        if this_system().is_some() {
            vec!["astap", "astap-database"]
        } else {
            vec!["astap"]
        }
    );
    assert_eq!(
        file_name_of("https://a.invalid/dir/ASTAP%20setup.exe?y"),
        "ASTAP20setup.exe"
    );
}

/* ------------------- ASTAP and its star database: download, install --------- */

fn zip_of(files: &[(&str, &[u8], u32)]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes, mode) in files {
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(*mode);
        writer.start_file(*name, options).unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn serving(file: &[u8]) -> FakeServer {
    FakeServer {
        file: file.to_vec(),
        cut: Mutex::new(None),
        asked_from: Mutex::new(Vec::new()),
        honours_range: true,
    }
}

fn empty_context(data: &Path) -> DetectContext {
    DetectContext {
        data_dir: data.to_path_buf(),
        reference_stars: PathBuf::from("/nonexistent/reference-stars.bin"),
        search_dirs: Vec::new(),
        known_paths: BTreeMap::new(),
        probe_timeout: Duration::from_secs(1),
        stacking: None,
    }
}

#[test]
fn the_registry_pins_astap_and_its_star_database_on_every_system() {
    let registry = registry();
    for id in ["astap", "astap-database"] {
        let entry = registry.entry(id).unwrap();
        assert_eq!(entry.page, "https://www.hnsky.org/astap.htm");
        for system in SYSTEMS {
            let download = entry.downloads.get(system).unwrap_or_else(|| {
                panic!("{id} has no download for {system}");
            });
            // Official source only, https, a ZIP, a digest and a size.
            assert!(
                download
                    .url
                    .starts_with("https://downloads.sourceforge.net/project/astap-program/"),
                "{}",
                download.url
            );
            assert!(is_archive(&download.url), "{}", download.url);
            assert_eq!(download.sha256.len(), 64);
            assert!(download.bytes.is_some_and(|bytes| bytes > 100_000));
        }
    }
    // The program is the command-line solver of each system; the star
    // database is the same D20 file everywhere.
    let astap = registry.entry("astap").unwrap();
    assert!(astap.downloads["macos-arm64"].url.ends_with("macOS_M1.zip"));
    assert!(
        astap.downloads["macos-x64"]
            .url
            .ends_with("macOS_x86_64.zip")
    );
    assert!(astap.downloads["windows-x64"].url.ends_with("win64.zip"));
    assert!(
        astap.downloads["linux-x64"]
            .url
            .ends_with("Linux_amd64.zip")
    );
    let database = registry.entry("astap-database").unwrap();
    assert!(database.downloads.values().all(|download| {
        download
            .url
            .ends_with("/star_databases/d20_star_database.zip")
            && download.sha256 == database.downloads["macos-arm64"].sha256
    }));
    assert_eq!(astap.licence, "MPL-2.0");
}

#[test]
fn astap_installed_in_the_users_folder_is_among_the_windows_places() {
    let local = std::ffi::OsString::from("LOCAL");
    let user = PathBuf::from("LOCAL").join("Programs").join("astap");
    let places = astap_windows_places(Some(local.clone()));
    // The installer's folder first, then the user's, the command line first.
    assert_eq!(
        places,
        vec![
            PathBuf::from(r"C:\Program Files\astap").join("astap_cli.exe"),
            PathBuf::from(r"C:\Program Files\astap").join("astap.exe"),
            user.join("astap_cli.exe"),
            user.join("astap.exe"),
        ]
    );
    assert_eq!(astap_user_folder(Some(local)), Some(user));
    // Without `%LOCALAPPDATA%` (or empty), only the installer's folder.
    assert_eq!(astap_windows_places(None).len(), 2);
    assert_eq!(astap_user_folder(Some(std::ffi::OsString::new())), None);
}

#[test]
fn astap_is_unpacked_into_its_tools_folder_and_found_there() {
    let dir = TempDir::new("tools-install-astap");
    let program = format!("astap_cli{}", std::env::consts::EXE_SUFFIX);
    let archive = zip_of(&[(&program, b"#!/bin/sh\necho astap\n", 0o755)]);
    let download = offer(&archive);
    let server = serving(&archive);
    let before = detect_tools(&empty_context(dir.path()));
    assert_eq!(row(&before, "astap").unwrap().state, ToolState::Missing);

    let done = download_and_install(
        dir.path(),
        "astap",
        Some(&download),
        &server,
        None,
        &mut |_| {},
    )
    .unwrap();
    let folder = dir.path().join("tools/astap");
    assert_eq!(done.path, folder);
    assert!(folder.join(&program).is_file());
    // The archive is deleted once unpacked; no staging folder is left.
    assert!(!folder.join("astap_cli_2026.10.zip").exists());
    assert!(!folder.join(".installing").exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(folder.join(&program))
            .unwrap()
            .permissions()
            .mode();
        assert_ne!(mode & 0o100, 0, "the program is executable");
    }

    // Detection finds the program in Astro Sync's own folder.
    let after = detect_tools(&empty_context(dir.path()));
    let astap = row(&after, "astap").unwrap();
    assert_eq!(astap.state, ToolState::Present);
    assert_eq!(astap.path.as_deref(), Some(folder.join(&program).as_path()));
}

#[test]
fn the_star_database_is_unpacked_whole_and_seen_present() {
    let dir = TempDir::new("tools-install-database");
    let archive = zip_of(&[
        ("d20_0101.1476", b"stars one", 0o644),
        ("d20_0201.1476", b"stars two", 0o644),
    ]);
    let download = offer(&archive);
    let before = detect_tools(&empty_context(dir.path()));
    assert_eq!(
        row(&before, "astap-database").unwrap().state,
        ToolState::Missing
    );
    download_and_install(
        dir.path(),
        "astap-database",
        Some(&download),
        &serving(&archive),
        None,
        &mut |_| {},
    )
    .unwrap();
    let folder = dir.path().join("tools/astap-database");
    assert_eq!(
        fs::read(folder.join("d20_0201.1476")).unwrap(),
        b"stars two"
    );
    let after = detect_tools(&empty_context(dir.path()));
    let database = row(&after, "astap-database").unwrap();
    assert_eq!(database.state, ToolState::Present);
    assert_eq!(database.path.as_deref(), Some(folder.as_path()));
}

#[test]
fn a_file_with_the_wrong_digest_installs_nothing() {
    let dir = TempDir::new("tools-install-refused");
    let archive = zip_of(&[("astap_cli", b"program", 0o755)]);
    let mut download = offer(&archive);
    download.sha256 = "0".repeat(64);
    let failure = download_and_install(
        dir.path(),
        "astap",
        Some(&download),
        &serving(&archive),
        None,
        &mut |_| {},
    )
    .unwrap_err();
    assert_eq!(failure, DownloadFailure::DigestMismatch);
    assert!(!dir.path().join("tools/astap/astap_cli").exists());
}

#[test]
fn an_archive_that_cannot_be_unpacked_is_a_failure_to_retry_not_a_file_to_install() {
    let dir = TempDir::new("tools-install-broken");
    let archive = zip_of(&[("../outside", b"escape", 0o644), ("astap_cli", b"x", 0o755)]);
    let download = offer(&archive);
    let failure = download_and_install(
        dir.path(),
        "astap",
        Some(&download),
        &serving(&archive),
        None,
        &mut |_| {},
    )
    .unwrap_err();
    assert!(
        matches!(failure, DownloadFailure::Extraction { .. }),
        "{failure:?}"
    );
    assert!(!dir.path().join("tools/outside").exists());
    assert!(!dir.path().join("outside").exists());
    // Nothing half-unpacked is where detection looks.
    assert!(!dir.path().join("tools/astap/astap_cli").exists());
    assert!(!dir.path().join("tools/astap/.installing").exists());
    // The checked archive is kept for the next try, but the row says
    // « missing » (its "Install"), never « downloaded, to install ».
    let rows = detect_tools(&empty_context(dir.path()));
    assert_eq!(row(&rows, "astap").unwrap().state, ToolState::Missing);
}

/// GraXpert, for the "polished render" -- in the registry with the
/// real files of its 3.0.2 release, read from the log line it prints, and
/// found where its Linux archive is unpacked (one folder down).
#[cfg(unix)]
#[test]
fn graxpert_is_in_the_registry_and_found_where_its_archive_is_unpacked() {
    use crate::external_process::fake_tool;
    let registry = registry();
    let graxpert = registry.entry("graxpert").unwrap();
    assert_eq!(graxpert.kind, DependencyKind::Tool);
    assert_eq!(graxpert.min_version.as_deref(), Some("3.0"));
    assert_eq!(
        graxpert.check.as_ref().unwrap().command,
        vec!["graxpert", "GraXpert"]
    );
    for system in SYSTEMS {
        let download = &graxpert.downloads[system];
        assert!(
            download
                .url
                .starts_with("https://github.com/Steffenhir/GraXpert/releases/download/3.0.2/"),
            "{}",
            download.url
        );
        assert_eq!(download.sha256.len(), 64);
        assert!(download.bytes.is_some_and(|bytes| bytes > 100_000_000));
    }
    // Only the Linux file is a ZIP Astro Sync unpacks; the .dmg and the .msi
    // are left for the person to open, as their installers ask.
    assert!(is_archive(&graxpert.downloads["linux-x64"].url));
    assert!(!is_archive(&graxpert.downloads["macos-arm64"].url));
    assert!(!is_archive(&graxpert.downloads["windows-x64"].url));
    assert!(graxpert.licence.starts_with("GPL-3.0-only"));
    assert!(graxpert.licence.contains("CC-BY-NC-SA-4.0"));
    let pattern = &graxpert.check.as_ref().unwrap().version_pattern;
    assert_eq!(
        read_version(
            pattern,
            "2026-10-14 23:33:20,156 MainProcess root INFO     GraXpert version: 3.0.2 release: Umbriel"
        )
        .as_deref(),
        Some("3.0.2")
    );

    let dir = TempDir::new("tools-graxpert");
    let data = dir.path().join("data");
    let unpacked = data.join("tools/graxpert/GraXpert-linux");
    fs::create_dir_all(&unpacked).unwrap();
    fake_tool(
        &unpacked,
        "GraXpert",
        "echo '2026-10-14 MainProcess root INFO     GraXpert version: 3.0.2 release: Umbriel'",
    );
    let mut known_paths = BTreeMap::new();
    known_paths.insert("graxpert".to_string(), vec![unpacked.join("GraXpert")]);
    let context = DetectContext {
        data_dir: data.clone(),
        reference_stars: data.join("reference-stars.bin"),
        search_dirs: Vec::new(),
        known_paths,
        probe_timeout: Duration::from_secs(10),
        stacking: None,
    };
    let rows = detect_tools(&context);
    let found = row(&rows, "graxpert").unwrap();
    assert_eq!(found.state, ToolState::Present);
    assert_eq!(found.version.as_deref(), Some("3.0.2"));
    assert_eq!(
        found.path.as_deref(),
        Some(unpacked.join("GraXpert").as_path())
    );
    // Optional: "Install all" never fetches it.
    assert!(!found.required);
    // The places of this system include the unpacked archive.
    assert!(graxpert_install_locations(&data).contains(&unpacked.join("GraXpert")));
}
