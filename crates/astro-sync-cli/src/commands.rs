//! One handler per subcommand, each a thin call into `astro_sync_core`.

use std::error::Error;
use std::fmt;
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use astro_sync_core::analysis::log_offset::{self, ForcedLogOffsets};
use astro_sync_core::analysis::photometry::{
    ReferenceStarList, parse_hipparcos_main, parse_tycho2,
};
use astro_sync_core::analysis::reference_catalogue::{self, TYCHO2, Tycho2Consent};
use astro_sync_core::analysis::siril::{self, ProgressionSettings};
use astro_sync_core::analysis::sky_night::{self, SkyOptions};
use astro_sync_core::analysis::{self, AnalysisOptions};
use astro_sync_core::gui_bridge::{
    DriveOptions, GuiBridgeError, HttpPortalGuiTransport, QueueDriveEvent, StopReason, drive_queue,
    upload_place_sky_measure,
};
use astro_sync_core::service_wait::{ServiceEvent, WaitHooks, WakePolicy};
use astro_sync_core::space::{
    self, DeviceIdentity, SignInAnnouncement, SignInEnd, SpaceClient, SpaceError,
};
use astro_sync_core::upload_queue::UploadQueue;
use astro_sync_core::uploader::{HttpUploadTransport, UploadEntryResult, UploadTarget};
use astro_sync_core::{association, dry_run, package, scanner};

use crate::cli::{
    AnalyzeArgs, AssociateArgs, FolderCommandArgs, FolderSource, PackageArgs, PairArgs,
    ProgressionArgs, QueueCommand, QueueStatusArgs, ReferenceStarsArgs, ReportArgs, SignInArgs,
    SirilCommand, SirilDetectArgs, SkyArgs, UploadArgs,
};
use crate::output;

/// A CLI-level argument-combination problem (as opposed to an
/// `astro-sync-core` error), e.g. neither `--folder` nor `--session-id` was
/// given.
#[derive(Debug)]
struct ArgError(String);

impl fmt::Display for ArgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Error for ArgError {}

/// Resolves the folder `scan`/`dry-run`/`package` operate on: either the
/// directly given `--folder`, or the folder of a previously saved
/// association looked up by `--config` + `--session-id`.
fn resolve_folder(source: &FolderSource) -> Result<PathBuf, Box<dyn Error>> {
    if let Some(folder) = &source.folder {
        return Ok(folder.clone());
    }
    match (&source.config, &source.session_id) {
        (Some(config), Some(session_id)) => {
            let store = association::load_store(config)?;
            store
                .find(session_id)
                .map(|found| found.folder.clone())
                .ok_or_else(|| {
                    Box::new(ArgError(format!(
                        "no association for the run {session_id:?} in {}",
                        config.display()
                    ))) as Box<dyn Error>
                })
        }
        _ => Err(Box::new(ArgError(
            "provide --folder, or --config and --session-id".to_string(),
        ))),
    }
}

pub fn associate(args: &AssociateArgs) -> Result<(), Box<dyn Error>> {
    let association = match (&args.folder, &args.new_folder) {
        (Some(_), Some(_)) => {
            return Err(Box::new(ArgError(
                "provide either --folder or --new-folder (not both)".to_string(),
            )));
        }
        (Some(folder), None) => association::Association::existing(&args.session_id, folder)?,
        (None, Some(new_folder)) => {
            let parent = args
                .parent
                .as_ref()
                .ok_or_else(|| ArgError("--new-folder requiert --parent".to_string()))?;
            association::Association::propose_new(&args.session_id, parent, new_folder)?
        }
        (None, None) => {
            return Err(Box::new(ArgError(
                "provide --folder (an existing folder) or --new-folder and --parent (a new folder)"
                    .to_string(),
            )));
        }
    };

    let store = association::save_association(&args.config, &association)?;

    if args.json {
        output::print_json(&association)?;
    } else {
        println!(
            "Association saved: run {:?} -> {} ({})",
            association.session_id,
            association.folder.display(),
            if association.proposed {
                "new folder"
            } else {
                "existing folder"
            }
        );
        println!("Configuration file: {}", args.config.display());
        println!("Associations saved in total: {}", store.associations.len());
    }
    Ok(())
}

pub fn scan(args: &FolderCommandArgs) -> Result<(), Box<dyn Error>> {
    let folder = resolve_folder(&args.folder_source)?;
    let report = scanner::scan_folder(&folder)?;
    if args.json {
        output::print_json(&report)?;
    } else {
        output::print_scan_table(&report, crate::words::requested_language());
    }
    Ok(())
}

pub fn dry_run(args: &FolderCommandArgs) -> Result<(), Box<dyn Error>> {
    let folder = resolve_folder(&args.folder_source)?;
    let scan = scanner::scan_folder(&folder)?;
    let report = dry_run::build_package_report(&scan);
    if args.json {
        output::print_json(&report)?;
    } else {
        output::print_dry_run_table(&report);
    }
    Ok(())
}

pub fn package(args: &PackageArgs) -> Result<(), Box<dyn Error>> {
    let folder = resolve_folder(&args.folder_source)?;
    let scan = scanner::scan_folder(&folder)?;
    let report = dry_run::build_package_report(&scan);

    let session = (args.session_label.is_some()
        || args.session_started_at.is_some()
        || args.session_finished_at.is_some())
    .then(|| package::SessionMeta {
        label: args.session_label.clone(),
        started_at: args.session_started_at.clone(),
        finished_at: args.session_finished_at.clone(),
    });
    let producer = Some(package::ProducerInfo {
        name: args.producer_name.clone(),
        version: Some(
            args.producer_version
                .clone()
                .unwrap_or_else(|| astro_sync_core::version().to_string()),
        ),
    });

    let outcome = package::produce_package(&report, &args.output, session, producer)?;

    if args.json {
        output::print_json(&outcome)?;
    } else {
        output::print_package_outcome(&outcome);
    }
    Ok(())
}

/// Runs local log/FITS analysis over `args.folder` (read-only: PHD2 guide
/// logs, ASIAIR Autorun logs, FITS headers) and writes
/// `metrics/quality.csv`, `metrics/guiding.csv`, and `events/session.jsonl`
/// under `args.out` -- the same relative layout the open session package
/// uses, but written directly rather than through a full `package`
/// (`astro-session-package/manifest.json`+`checksums.sha256`) run. Wiring
/// `analyze`'s output into an actual package (`package::append_analysis_assets`)
/// is CLI-level plumbing left for a later task; the core capability is
/// already exercised directly in `astro-sync-core`'s own tests.
pub fn analyze(args: &AnalyzeArgs) -> Result<(), Box<dyn Error>> {
    if !args.out.is_dir() {
        return Err(Box::new(ArgError(format!(
            "the output folder {} does not exist",
            args.out.display()
        ))));
    }

    let options = AnalysisOptions {
        phd2_utc_offset: args.phd2_utc_offset.clone(),
        asiair_utc_offset: args.asiair_utc_offset.clone(),
        nina_utc_offset: args.nina_utc_offset.clone(),
    };
    let outcome = analysis::analyze_folder(&args.folder, &options)?;

    let metrics_dir = args.out.join("metrics");
    let events_dir = args.out.join("events");
    std::fs::create_dir_all(&metrics_dir)?;
    std::fs::create_dir_all(&events_dir)?;
    std::fs::write(metrics_dir.join("quality.csv"), &outcome.quality_csv)?;
    std::fs::write(metrics_dir.join("guiding.csv"), &outcome.guiding_csv)?;
    std::fs::write(events_dir.join("session.jsonl"), &outcome.session_jsonl)?;

    if args.json {
        output::print_json(&outcome)?;
    } else {
        output::print_analyze_summary(&outcome, &args.out);
    }
    Ok(())
}

/// `report --folder … --out report.json`: the same combined
/// session-analysis report the GUI's Analysis screen generates
/// (`analysis::report::generate_report`, which itself runs
/// `analysis::analyze_folder`) and uploads via
/// `POST /v1/sessions/{id}/analyses`, written here to a chosen file
/// instead -- a thin CLI wrapper, exactly like `analyze`, `--out` names
/// the file itself (its parent directory must already exist; the file
/// itself is created or overwritten). Rendered canonically
/// (`render_report_json`: sorted keys, no insignificant whitespace, the
/// exact bytes a real upload would send) so the CLI and the GUI never
/// diverge on what "the report" byte-for-byte is.
pub fn report(args: &ReportArgs) -> Result<(), Box<dyn Error>> {
    let parent_missing = args
        .out
        .parent()
        .is_some_and(|parent| !parent.as_os_str().is_empty() && !parent.is_dir());
    if parent_missing {
        return Err(Box::new(ArgError(format!(
            "the folder {} does not exist",
            args.out.parent().unwrap().display()
        ))));
    }

    // The clock of the logs is found from the frames unless given,
    // as `send-logs` does, and said: guiding read with the wrong clock is
    // attributed to the wrong frames.
    let forced = ForcedLogOffsets {
        phd2: args.phd2_utc_offset.clone(),
        asiair: args.asiair_utc_offset.clone(),
        nina: args.nina_utc_offset.clone(),
    };
    let (outcome, clock) = log_offset::analyze_folder_detecting(&args.folder, &forced, None)?;
    let mut report = analysis::report::report_from_outcome_with(
        &outcome,
        &astro_sync_core::pose_stats::record::load(&args.folder),
    );
    report.log_clock = clock.as_ref().map(analysis::report::LogClock::from);
    let rendered = analysis::report::render_report_json(&report);
    std::fs::write(&args.out, rendered.as_bytes())?;

    if args.json {
        output::print_json(&report)?;
        // The report on the standard output stays a pure JSON document.
        crate::night::print_logs_clock_to(&clock, &mut std::io::stderr());
    } else {
        output::print_report_summary(&report, &args.out);
        crate::night::print_logs_clock_to(&clock, &mut std::io::stdout());
    }
    Ok(())
}

/// `siril detect`: tries a manually-selected path first, then the OS's
/// standard install locations, then `PATH`, and reports the first candidate
/// that runs and answers `--version`, verified against the compiled-in
/// compatible-version whitelist. Never a hard error: "Siril is not
/// installed" is a normal, reportable outcome, not a CLI failure.
pub fn siril(command: &SirilCommand) -> Result<(), Box<dyn Error>> {
    match command {
        SirilCommand::Detect(args) => siril_detect(args),
    }
}

fn siril_detect(args: &SirilDetectArgs) -> Result<(), Box<dyn Error>> {
    let candidates = siril::candidate_locations(args.siril_path.as_deref());
    let detection = siril::detect(&candidates);
    if args.json {
        output::print_json(&detection)?;
    } else {
        output::print_siril_detection(&detection);
    }
    Ok(())
}

/// `progression --folder … --family … --out …`: finds the named acquisition
/// family in `folder` (reusing `analysis::analyze_folder`'s own FITS-header
/// grouping), then delegates to `siril::detect_and_run_progression` -- the
/// one place (shared with the GUI's Progression screen) that decides
/// whether a detected Siril is compatible enough to run at all. A missing
/// or incompatible Siril, or any failure while running it, is reported as
/// an "unavailable" progression and still exits successfully: the
/// failure policy is that this can never block whatever else the person is
/// doing with the folder (scanning, packaging, uploading), so this command
/// never turns that into a nonzero exit code. Only a genuine argument
/// problem (folder/family/out not found) is a hard error here.
pub fn progression(args: &ProgressionArgs) -> Result<(), Box<dyn Error>> {
    if !args.out.is_dir() {
        return Err(Box::new(ArgError(format!(
            "the output folder {} does not exist",
            args.out.display()
        ))));
    }

    let analysis_outcome = analysis::analyze_folder(&args.folder, &AnalysisOptions::default())?;
    let family = analysis_outcome
        .acquisition_families
        .iter()
        .find(|family| family.key == args.family)
        .ok_or_else(|| {
            ArgError(format!(
                "no acquisition family {:?} in {} (families found: {})",
                args.family,
                args.folder.display(),
                analysis_outcome
                    .acquisition_families
                    .iter()
                    .map(|family| family.key.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })?;

    let working_dir = args.out.join("siril-work").join(&family.key);
    let outcome = siril::detect_and_run_progression(
        &args.folder,
        &working_dir,
        family,
        args.siril_path.as_deref(),
        &ProgressionSettings::default(),
    );

    if args.json {
        output::print_json(&outcome)?;
    } else {
        output::print_progression_outcome(&outcome);
    }
    Ok(())
}

/// Reads a device token from a file -- never from an argument, so it never
/// appears in a process listing or shell history.
fn read_token_file(path: &Path) -> Result<String, Box<dyn Error>> {
    let raw = std::fs::read_to_string(path).map_err(|source| {
        ArgError(format!(
            "could not read the token ({}): {source}",
            path.display()
        ))
    })?;
    let token = raw.trim().to_string();
    if token.is_empty() {
        return Err(Box::new(ArgError(format!(
            "the token file {} is empty",
            path.display()
        ))));
    }
    Ok(token)
}

/// What the command line says while the hosted service starts. A service
/// can sleep when nobody uses it and take a while to wake: say so
/// instead of appearing stuck, and instead of reporting "no answer".
const SERVICE_STARTING: &str = "The service is starting. It may take a minute.";
const SERVICE_READY: &str = "The service answered. Sending resumes.";

/// `sky`: the night's measured sky background, read on this computer from
/// the solved light frames of `--folder`; with `--portal`/`--token`, the
/// night's one value goes to the service (`POST /v1/place-sky-measures`,
/// free). The frames never leave the computer.
pub fn sky(args: &SkyArgs) -> Result<(), Box<dyn Error>> {
    let stars = match &args.stars {
        Some(path) => Some(ReferenceStarList::read(path)?),
        None => None,
    };
    let options = SkyOptions {
        stars,
        site: args
            .site_latitude
            .zip(args.site_longitude)
            .map(|(latitude, longitude)| sky_night::Site {
                latitude,
                longitude,
            }),
        pedestal_adu: args.pedestal,
    };
    let outcome = sky_night::measure_folder_sky(&args.folder, &options)?;

    let uploaded = match (&args.portal, &args.token) {
        (Some(portal), Some(token_path)) => {
            let night = match &args.night {
                Some(wanted) => outcome.nights.iter().find(|night| &night.night == wanted),
                None if outcome.nights.len() > 1 => {
                    return Err(Box::new(ArgError(
                        "this folder holds several measured nights: choose one with --night"
                            .to_string(),
                    )));
                }
                None => outcome.nights.first(),
            };
            let Some(night) = night else {
                if !args.json {
                    output::print_sky_summary(&outcome, &args.folder);
                }
                return Err(Box::new(ArgError(
                    "no night value to send: nothing was sent".to_string(),
                )));
            };
            let token = read_token_file(token_path)?;
            let hooks = WaitHooks::with_events(|event| match event {
                ServiceEvent::Starting => eprintln!("{SERVICE_STARTING}"),
                ServiceEvent::Ready | ServiceEvent::Stopped => {}
            });
            let transport = HttpPortalGuiTransport::with_options(WakePolicy::default(), hooks)?;
            let body = sky_night::upload_body(night, args.session_id.as_deref());
            match upload_place_sky_measure(&transport, portal, &token, &body) {
                Ok(uploaded) => Some(uploaded),
                Err(GuiBridgeError::Refused { code, message }) => {
                    let sentence = astro_sync_core::refusal::plain_sentence(&code)
                        .map_or(message, str::to_string);
                    return Err(Box::new(ArgError(sentence)));
                }
                Err(error) => return Err(Box::new(error)),
            }
        }
        _ => None,
    };

    if args.json {
        output::print_json(&serde_json::json!({
            "measure": outcome,
            "uploaded": uploaded,
        }))?;
    } else {
        output::print_sky_summary(&outcome, &args.folder);
        if let Some(uploaded) = uploaded {
            println!(
                "Value sent for the night of {} ({}){}",
                uploaded.measure.night,
                uploaded.measure.place_id,
                if uploaded.replaced {
                    ": it replaces the value already recorded."
                } else {
                    "."
                }
            );
        }
    }
    Ok(())
}

/// `reference-stars`: writes the compact reference list of `sky` from a
/// copy of `hip_main.dat` (Hipparcos) or `tyc2.dat` (Tycho-2).
pub fn reference_stars(args: &ReferenceStarsArgs) -> Result<(), Box<dyn Error>> {
    if args.download_tycho2 {
        return download_tycho2_list(args);
    }
    let (source, parse): (&Path, fn(&str) -> Vec<_>) = match (&args.hipparcos, &args.tycho2) {
        (Some(path), _) => (path, parse_hipparcos_main),
        (None, Some(path)) => (path, parse_tycho2),
        (None, None) => {
            return Err(Box::new(ArgError(
                "provide --hipparcos or --tycho2".to_string(),
            )));
        }
    };
    let text = std::fs::read_to_string(source)?;
    let stars: Vec<_> = parse(&text)
        .into_iter()
        .filter(|star| star.v_mag <= args.faintest)
        .collect();
    let list = ReferenceStarList::new(stars);
    std::fs::write(&args.out, list.to_bytes())?;
    println!(
        "{} reference stars written to {}",
        list.len(),
        args.out.display()
    );
    Ok(())
}

/// The agreement the command line asks for before it fetches Tycho-2: it
/// names the catalogue, the source, the size and the credit.
fn tycho2_agreement() -> String {
    format!(
        "Download {} from VizieR (CDS, {}): about thirty requests, then a list of about {} MB kept on this computer. Credit: {}. Add --accept-download to accept; without it nothing is downloaded.",
        TYCHO2.name, TYCHO2.vizier_table, TYCHO2.list_megabytes, TYCHO2.credit,
    )
}

/// `reference-stars --download-tycho2 --accept-download`.
fn download_tycho2_list(args: &ReferenceStarsArgs) -> Result<(), Box<dyn Error>> {
    if !args.accept_download {
        return Err(Box::new(ArgError(tycho2_agreement())));
    }
    let list = reference_catalogue::download_tycho2(
        Tycho2Consent::given(),
        args.faintest,
        &mut |done, total| eprintln!("Tycho-2: band {done} of {total}"),
    )?;
    std::fs::write(&args.out, list.to_bytes())?;
    println!(
        "{} reference stars (Tycho-2) written to {}. Credit to display: {}.",
        list.len(),
        args.out.display(),
        TYCHO2.credit
    );
    Ok(())
}

pub fn upload(args: &UploadArgs) -> Result<(), Box<dyn Error>> {
    // Standard error, not standard output: `--json` keeps standard output
    // a clean machine-readable result.
    upload_with(
        args,
        WakePolicy::default(),
        DriveOptions::command_line(),
        crate::words::requested_language(),
        Arc::new(|line: &str| eprintln!("{line}")),
    )
}

/// [`upload`] with the waiting policy, the waits between two tries and the
/// place the service's state is said made explicit, so a test can shorten the
/// waits and read what was said.
///
/// Everything it says is said in `locale`, with the sentences of the window.
///
/// The loop is the window's own ([`drive_queue`]), so the command line and the
/// window never disagree on what a refusal does: a file the service refuses
/// for good is set aside and the next one goes on; a full storage stops the
/// run with every file kept; a failure that will pass (a service that fails,
/// a network that is down, too many uploads open in the space) is waited out,
/// longer each time, up to the number of new tries the command line allows --
/// then the file stays queued for the next run, unmarked.
fn upload_with(
    args: &UploadArgs,
    policy: WakePolicy,
    options: DriveOptions,
    locale: &'static str,
    say: Arc<dyn Fn(&str) + Send + Sync>,
) -> Result<(), Box<dyn Error>> {
    let token = read_token_file(&args.token)?;
    let queue = UploadQueue::open(&args.queue)?;
    let words =
        |key: &str, vars: &[(&str, &str)]| crate::words::say(locale, key, vars).unwrap_or_default();

    if let Some(folder) = &args.folder {
        let scan = scanner::scan_folder(folder)?;
        let report = dry_run::build_dry_run_report(&scan);
        let enqueued = queue.enqueue_from_dry_run(&report)?;
        if !args.json {
            println!(
                "{}",
                words(
                    "cli.upload.queueUpdated",
                    &[
                        ("added", &enqueued.inserted.to_string()),
                        ("known", &enqueued.already_known.to_string()),
                    ],
                )
            );
        }
    }

    let waiting = Arc::clone(&say);
    let hooks = WaitHooks::with_events(move |event| match event {
        ServiceEvent::Starting => {
            waiting(&service_line(locale, "service.starting", SERVICE_STARTING))
        }
        ServiceEvent::Ready => waiting(&service_line(locale, "service.ready", SERVICE_READY)),
        ServiceEvent::Stopped => {}
    });
    let transport = HttpUploadTransport::with_options(&args.portal, policy, hooks)?;
    let target = UploadTarget {
        session_id: args.session_id.clone(),
        plan_id: args.plan_id.clone(),
        target_ids: args.target_ids.clone(),
    };

    let mut results = Vec::new();
    // The service said this computer is no longer attached to a space: no
    // other file can be sent either, and none of them is at fault.
    let mut sign_in_ended: Option<SignInEnd> = None;
    let mut current_file = PathBuf::new();
    let print_lines = !args.json;
    drive_queue(
        &transport,
        &queue,
        &token,
        &target,
        &options,
        || true,
        |event| match event {
            QueueDriveEvent::Started {
                source_path, bytes, ..
            } => {
                if print_lines {
                    println!(
                        "{} ({})",
                        words(
                            "queue.startedNotice",
                            &[("file", &source_path.display().to_string())]
                        ),
                        crate::words::size_text(locale, bytes)
                    );
                }
                current_file = source_path;
            }
            QueueDriveEvent::Progress { sent, total, .. } => {
                if print_lines {
                    print!(
                        "\r  {} / {}",
                        crate::words::size_text(locale, sent),
                        crate::words::size_text(locale, total)
                    );
                    let _ = std::io::stdout().flush();
                }
            }
            QueueDriveEvent::Completed {
                sha256,
                capture_id,
                deduplicated,
                skipped,
                replaced,
            } => {
                let note = astro_sync_core::metadata::skipped_sentence(&skipped);
                if print_lines {
                    let mut line = words(
                        "queue.completedNotice",
                        &[
                            ("file", &current_file.display().to_string()),
                            ("captureId", &capture_id),
                        ],
                    );
                    if deduplicated {
                        line = format!("{line} {}", words("uploads.row.deduplicated", &[]));
                    }
                    println!("\r  {line}");
                    if let Some(note) = &note {
                        println!("  {note}");
                    }
                    if replaced {
                        println!("  {}", words("queue.replacedPhoto", &[]));
                    }
                }
                let mut result = UploadEntryResult::sent(sha256, capture_id, deduplicated);
                result.skipped = skipped;
                result.note = note;
                result.replaced = replaced;
                results.push(result);
            }
            QueueDriveEvent::Refused {
                sha256,
                code,
                message,
            } => {
                if print_lines {
                    let reason = crate::words::reason_text(&code, &message, locale);
                    println!(
                        "\r  {}",
                        words(
                            "queue.refusedNotice",
                            &[
                                ("file", &current_file.display().to_string()),
                                ("reason", &reason),
                            ],
                        )
                    );
                }
                results.push(UploadEntryResult::failed(sha256, code, message));
            }
            QueueDriveEvent::Held {
                sha256,
                code,
                message,
            } => {
                if print_lines {
                    let reason = crate::words::reason_text(&code, &message, locale);
                    println!("\r  {}", words("queue.heldNotice", &[("reason", &reason)]));
                    println!("  {}", words("cli.upload.runAgain", &[]));
                }
                results.push(UploadEntryResult::failed(sha256, code, message));
            }
            QueueDriveEvent::Waiting {
                code,
                message,
                retry_in_seconds,
                attempt,
                ..
            } => {
                let reason = crate::words::reason_text(&code, &message, locale);
                say(&words(
                    "cli.upload.waiting",
                    &[
                        ("reason", &reason),
                        (
                            "wait",
                            &crate::words::duration_text(locale, retry_in_seconds),
                        ),
                        ("attempt", &attempt.to_string()),
                    ],
                ));
            }
            QueueDriveEvent::Failed {
                sha256,
                code,
                message,
            } => {
                if let Some(end) = SignInEnd::from_code(&code) {
                    sign_in_ended = Some(end);
                    if print_lines {
                        println!("\r  {}", describe_sign_in_ended(end, locale));
                    }
                } else if print_lines {
                    let reason = crate::words::reason_text(&code, &message, locale);
                    println!(
                        "\r  {}",
                        words(
                            "queue.failedNoticeKnown",
                            &[
                                ("file", &current_file.display().to_string()),
                                ("reason", &reason),
                            ],
                        )
                    );
                }
                results.push(UploadEntryResult::failed(sha256, code, message));
            }
            QueueDriveEvent::Stopped { reason, .. } => {
                if print_lines && reason == StopReason::NoProgress {
                    eprintln!(
                        "{} ({})",
                        words("queue.stoppedNoProgress", &[]),
                        current_file.display()
                    );
                }
            }
        },
    )?;

    if args.json {
        output::print_json(&results)?;
    } else if results.is_empty() {
        println!("{}", words("queue.stoppedQueueEmpty", &[]));
    }
    match sign_in_ended {
        // A failure of the command, not a line among others: a script must
        // know that nothing more can be sent until the person signs in again.
        Some(end) => Err(Box::new(ArgError(describe_sign_in_ended(end, locale)))),
        None => Ok(()),
    }
}

/// The sentence of the service's state in `locale` (the English one when the
/// dictionaries have none).
fn service_line(locale: &str, key: &str, fallback: &str) -> String {
    crate::words::say(locale, key, &[]).unwrap_or_else(|| fallback.to_string())
}

/// Why this computer can no longer send, and the command that signs it in again.
fn describe_sign_in_ended(end: SignInEnd, locale: &str) -> String {
    let reason = crate::words::reason_text(end.code(), end.plain_message(), locale);
    let again = crate::words::say(locale, "cli.upload.signInAgain", &[])
        .unwrap_or_else(|| "Command: astro-sync sign-in --token <token file>.".to_string());
    format!("{reason} {again}")
}

const SIGN_IN_SERVICE_READY: &str = "The service answered.";

/// The terminal a sign-in talks through: where a line is read from, where the
/// result goes (standard output) and where everything else goes (standard
/// error, so `--json` keeps standard output a clean machine-readable
/// result). A prompt is only shown to someone who is there to read it.
struct Console<'a> {
    input: &'a mut dyn BufRead,
    out: &'a mut dyn Write,
    err: &'a mut dyn Write,
    interactive: bool,
}

impl Console<'_> {
    /// One line, trimmed; `None` at the end of the input.
    fn ask(&mut self, prompt: &str) -> Result<Option<String>, Box<dyn Error>> {
        if self.interactive {
            write!(self.err, "{prompt}")?;
            self.err.flush()?;
        }
        let mut line = String::new();
        if self.input.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        Ok(Some(line.trim().to_string()))
    }

    fn note(&mut self, line: &str) -> Result<(), Box<dyn Error>> {
        writeln!(self.err, "{line}")?;
        Ok(())
    }
}

/// The language of the message the service sends, from the environment
/// (`fr_FR.UTF-8` reads as `fr`); the service reads anything unknown as `en`.
fn environment_locale() -> String {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .filter_map(|name| std::env::var(name).ok())
        .find(|value| !value.is_empty() && value != "C" && value != "POSIX")
        .unwrap_or_else(|| "en".to_string())
}

/// `sign-in`: signs in to the hosted service by
/// e-mail address and code, and keeps the device token in a file only its
/// owner can read -- the file `upload --token` reads. The code is read from
/// the keyboard or from standard input, never from an argument.
pub fn sign_in(args: &SignInArgs) -> Result<(), Box<dyn Error>> {
    let stdin = std::io::stdin();
    let interactive = stdin.is_terminal();
    let mut input = stdin.lock();
    let mut out = std::io::stdout();
    let mut err = std::io::stderr();
    sign_in_with(
        args,
        WakePolicy::default(),
        Arc::new(|line: &str| eprintln!("{line}")),
        &mut Console {
            input: &mut input,
            out: &mut out,
            err: &mut err,
            interactive,
        },
    )
}

/// [`sign_in`] with the waiting policy, the place the service's state is said
/// and the terminal made explicit, so a test can shorten the wait, script
/// what is typed and read what is said.
fn sign_in_with(
    args: &SignInArgs,
    policy: WakePolicy,
    say: Arc<dyn Fn(&str) + Send + Sync>,
    console: &mut Console<'_>,
) -> Result<(), Box<dyn Error>> {
    let installation_file = args.installation_id.clone().unwrap_or_else(|| {
        args.token
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
            .join("installation-id")
    });
    let device_name = args
        .device_name
        .clone()
        .unwrap_or_else(space::default_device_name);
    let device = DeviceIdentity::at(&installation_file, &device_name)?;
    let locale = args.language.clone().unwrap_or_else(environment_locale);

    let hooks = WaitHooks::with_events(move |event| match event {
        ServiceEvent::Starting => say(SERVICE_STARTING),
        ServiceEvent::Ready => say(SIGN_IN_SERVICE_READY),
        ServiceEvent::Stopped => {}
    });
    let client = SpaceClient::with_options(&args.portal, policy, hooks)?;

    // What the service says about how a person signs in decides everything.
    match client.announcement()? {
        SignInAnnouncement::EmailCode { .. } => {}
        SignInAnnouncement::Pairing => {
            return Err(Box::new(ArgError(
                "this service is a private server: it does not sign in by e-mail. \
                 Connect this computer with the eight-character code your server shows \
                 (Astro Sync, “Astro Portal space” tab), then give the token file to `upload --token`."
                    .to_string(),
            )));
        }
        SignInAnnouncement::None => {
            return Err(Box::new(ArgError(
                "this service accepts no sign-in: its private API is switched off.".to_string(),
            )));
        }
        SignInAnnouncement::Unsupported { .. } => {
            return Err(Box::new(SpaceError::IncompatibleApiVersion));
        }
    }

    let email = match &args.email {
        Some(email) => email.clone(),
        None => console.ask("E-mail address: ")?.unwrap_or_default(),
    };
    if email.trim().is_empty() {
        return Err(Box::new(ArgError(
            "no e-mail address: give --email, or type it (or send it on the standard input)."
                .to_string(),
        )));
    }

    let mut address = email.clone();
    if !args.verify_only {
        let requested = client.request_code(&email, &locale, &device)?;
        address = requested.email.clone();
        console.note(&format!(
            "A six-digit code was just sent to {address}. It is valid for {} minutes, once only, and only the last message received counts. Look in your junk mail too.",
            requested.expires_in_seconds / 60
        ))?;
        console.note(
            "The button in the message does not open Astro Sync: on a computer it only shows the code.",
        )?;
        if args.send_only {
            if args.json {
                writeln!(console.out, "{}", serde_json::to_string_pretty(&requested)?)?;
            } else {
                writeln!(
                    console.out,
                    "Code sent to {address}. Sending a new one will be possible in {} seconds. \
                     Then give the code on the standard input, with --verify-only.",
                    requested.resend_after_seconds
                )?;
            }
            return Ok(());
        }
    }

    let signed_in = loop {
        let Some(typed) = console.ask("Sign-in code: ")? else {
            return Err(Box::new(ArgError(
                "no code received on the standard input. The code is never an argument: type it, or send it on the standard input (with --verify-only if the code was already sent)."
                    .to_string(),
            )));
        };
        match client.verify_code(&address, &typed, &device) {
            Ok(signed_in) => break signed_in,
            // At a keyboard a wrong code is asked for again: it stays valid
            // and the person has a few tries. Anywhere else it is a failure.
            Err(error)
                if console.interactive
                    && (matches!(error, SpaceError::InvalidCode)
                        || error
                            .refusal()
                            .is_some_and(|refusal| refusal.code == "sign_in_code_incorrect")) =>
            {
                console.note(&format!("{error}"))?;
            }
            Err(error) => return Err(Box::new(error)),
        }
    };

    space::write_token_file(&args.token, signed_in.device_token.expose())?;

    if args.json {
        let result = serde_json::json!({
            "created": signed_in.created,
            "space": &signed_in.space,
            "device": &signed_in.device,
            "tokenFile": args.token,
        });
        writeln!(console.out, "{}", serde_json::to_string_pretty(&result)?)?;
    } else {
        writeln!(
            console.out,
            "Signed in to the space {} ({}).",
            signed_in.space.email,
            if signed_in.created {
                "just created"
            } else {
                "already existing"
            }
        )?;
        writeln!(
            console.out,
            "This computer is registered under the name “{}”.",
            signed_in.device.name
        )?;
        writeln!(
            console.out,
            "Device token stored in {} (readable by you only): give it to `upload --token`.",
            args.token.display()
        )?;
        writeln!(
            console.out,
            "Your space on the web: {} (same e-mail address).",
            signed_in.space.web_url
        )?;
    }
    Ok(())
}

/// `pair`: exchanges the code a private server shows ("Pair a new device")
/// for a device token, kept in a file only its owner can read -- the file
/// `upload --token` reads. The code is asked for on the keyboard, or read from
/// the standard input; it is never an argument, so it stays out of the
/// terminal history.
pub fn pair(args: &PairArgs) -> Result<(), Box<dyn Error>> {
    let stdin = std::io::stdin();
    let interactive = stdin.is_terminal();
    let mut input = stdin.lock();
    let mut out = std::io::stdout();
    let mut err = std::io::stderr();
    pair_with(
        args,
        &crate::words::portal_transport()?,
        &mut Console {
            input: &mut input,
            out: &mut out,
            err: &mut err,
            interactive,
        },
    )
}

/// [`pair`] with the transport and the terminal made explicit.
fn pair_with(
    args: &PairArgs,
    transport: &HttpPortalGuiTransport,
    console: &mut Console<'_>,
) -> Result<(), Box<dyn Error>> {
    let Some(typed) = console.ask("Pairing code: ")? else {
        return Err(Box::new(ArgError(
            "no pairing code received on the standard input. The code is never an argument: type it, or send it on the standard input."
                .to_string(),
        )));
    };
    let label = args
        .device_name
        .clone()
        .unwrap_or_else(space::default_device_name);
    let token = astro_sync_core::gui_bridge::claim_device(transport, &args.portal, &typed, &label)?;
    space::write_token_file(&args.token, &token)?;
    if args.json {
        let result = serde_json::json!({
            "deviceName": label,
            "tokenFile": args.token,
        });
        writeln!(console.out, "{}", serde_json::to_string_pretty(&result)?)?;
    } else {
        writeln!(
            console.out,
            "This computer is registered under the name “{label}”."
        )?;
        writeln!(
            console.out,
            "Device token stored in {} (readable by you only): give it to `upload --token`.",
            args.token.display()
        )?;
    }
    Ok(())
}

pub fn queue(args: &QueueCommand) -> Result<(), Box<dyn Error>> {
    match args {
        QueueCommand::Status(args) => queue_status(args),
    }
}

fn queue_status(args: &QueueStatusArgs) -> Result<(), Box<dyn Error>> {
    let queue = UploadQueue::open(&args.queue)?;
    let entries = queue.list()?;
    if args.json {
        output::print_json(&entries)?;
    } else {
        output::print_queue_table(&entries, crate::words::requested_language());
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../astro-sync-core/tests/support/fake_service.rs"]
mod fake_service;

#[cfg(test)]
#[path = "../../astro-sync-core/tests/support/contract_replay.rs"]
mod contract_replay;

#[cfg(test)]
#[path = "sign_in_tests.rs"]
mod sign_in_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use astro_sync_core::upload_queue::QueueState;
    use std::net::TcpListener;
    use std::sync::Mutex;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    #[test]
    fn tycho_2_is_never_downloaded_without_the_agreement_that_names_it() {
        let out = unique_temp_dir("tycho2-agreement").join("reference-stars.bin");
        let args = ReferenceStarsArgs {
            hipparcos: None,
            tycho2: None,
            download_tycho2: true,
            accept_download: false,
            faintest: 12.5,
            out: out.clone(),
        };
        let error = reference_stars(&args).expect_err("refused");
        let message = error.to_string();
        for word in ["Tycho-2", "VizieR", "I/259", "25 MB", "--accept-download"] {
            assert!(message.contains(word), "{word}: {message}");
        }
        assert!(!out.exists(), "nothing was fetched or written");
    }

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "astro-sync-cli-test-{label}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Regression test: a first implementation of the `upload` loop kept
    /// calling `next_pending` forever whenever a transient network failure
    /// left the entry's state unchanged (correct at the queue/uploader
    /// level -- the entry must stay resumable -- but wrong at the CLI
    /// loop's level, which must not spin on it). This binds a port, drops
    /// the listener so the connection is refused immediately (fast,
    /// deterministic, no real network needed), and asserts `upload`
    /// returns promptly with the entry still `queued` rather than hanging.
    #[test]
    fn upload_stops_instead_of_looping_when_the_portal_is_unreachable() {
        let dir = unique_temp_dir("upload-unreachable");
        let folder = dir.join("night");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("frame.tif"), b"test-bytes").unwrap();
        let token_path = dir.join("token.txt");
        std::fs::write(&token_path, "fake-token").unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let args = UploadArgs {
            queue: dir.join("queue.sqlite"),
            portal: format!("http://127.0.0.1:{port}"),
            token: token_path,
            folder: Some(folder),
            session_id: None,
            plan_id: None,
            target_ids: Vec::new(),
            json: true,
            language_of_messages: crate::cli::LangArg::default(),
        };

        upload(&args).unwrap();

        let queue = UploadQueue::open(&args.queue).unwrap();
        let entries = queue.list().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].state, QueueState::Queued);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The same shape as production, on a scale of hundreds of milliseconds.
    fn fast_policy() -> WakePolicy {
        WakePolicy {
            total_budget: Duration::from_millis(3_000),
            attempt_timeout: Duration::from_millis(500),
            write_timeout: Duration::from_millis(600),
            chunk_timeout: Duration::from_millis(1_500),
            retry_pause: Duration::from_millis(40),
            notice_after: Duration::from_millis(120),
            connect_timeout: Duration::from_millis(500),
        }
    }

    fn upload_args_for(dir: &Path, portal: String) -> UploadArgs {
        let folder = dir.join("night");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("frame.tif"), vec![5_u8; 250]).unwrap();
        let token_path = dir.join("token.txt");
        std::fs::write(&token_path, "fake-token").unwrap();
        UploadArgs {
            queue: dir.join("queue.sqlite"),
            portal,
            token: token_path,
            folder: Some(folder),
            session_id: None,
            plan_id: None,
            target_ids: Vec::new(),
            json: true,
            language_of_messages: crate::cli::LangArg::default(),
        }
    }

    /// What the person reading the terminal sees while the service wakes
    /// from a sleep: that it is starting, then that it answered -- once
    /// each -- and the file is sent exactly once.
    #[test]
    fn upload_says_the_service_is_starting_then_sends_the_file_once() {
        use fake_service::{FakeService, UploadModel, Wake, error_reply};

        let dir = unique_temp_dir("upload-wake");
        let model = Arc::new(UploadModel::default());
        let app_model = Arc::clone(&model);
        let service =
            FakeService::start(Wake::Held(Duration::from_millis(1_200)), move |request| {
                app_model
                    .reply(request, 100)
                    .unwrap_or_else(|| error_reply(404, "not_found", "no such route"))
            });
        let args = upload_args_for(&dir, service.base_url());
        let said: Arc<Mutex<Vec<String>>> = Arc::default();
        let sink = Arc::clone(&said);

        upload_with(
            &args,
            fast_policy(),
            DriveOptions::command_line(),
            "en",
            Arc::new(move |line: &str| sink.lock().unwrap().push(line.to_string())),
        )
        .unwrap();

        assert_eq!(
            *said.lock().unwrap(),
            [SERVICE_STARTING.to_string(), SERVICE_READY.to_string()]
        );
        let queue = UploadQueue::open(&args.queue).unwrap();
        assert_eq!(queue.list().unwrap()[0].state, QueueState::Completed);
        assert_eq!(model.sessions_created(), 1);
        assert_eq!(model.captures_created(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A service that never wakes in time ends the command without a hang:
    /// the file stays queued for the next run, and the person was told the
    /// service was starting (the reason for the wait), not left in silence.
    #[test]
    fn upload_gives_up_cleanly_when_the_service_does_not_wake_in_time() {
        use fake_service::{FakeService, Wake, error_reply};

        let dir = unique_temp_dir("upload-wake-timeout");
        let service = FakeService::start(Wake::Held(Duration::from_secs(30)), |_| {
            error_reply(404, "not_found", "no such route")
        });
        let args = upload_args_for(&dir, service.base_url());
        let said: Arc<Mutex<Vec<String>>> = Arc::default();
        let sink = Arc::clone(&said);
        let policy = WakePolicy {
            total_budget: Duration::from_millis(1_000),
            ..fast_policy()
        };

        let started = std::time::Instant::now();
        upload_with(
            &args,
            policy,
            DriveOptions::command_line(),
            "en",
            Arc::new(move |line: &str| sink.lock().unwrap().push(line.to_string())),
        )
        .unwrap();

        assert!(started.elapsed() < Duration::from_millis(3_000));
        assert_eq!(*said.lock().unwrap(), [SERVICE_STARTING.to_string()]);
        let queue = UploadQueue::open(&args.queue).unwrap();
        assert_eq!(queue.list().unwrap()[0].state, QueueState::Queued);
        assert_eq!(service.app_hits("POST", "/api/v1/uploads"), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// One file, `frame.tif` (250 bytes), or the files named, in a folder.
    fn args_with_files(dir: &Path, portal: String, names: &[&str]) -> UploadArgs {
        let folder = dir.join("night");
        std::fs::create_dir_all(&folder).unwrap();
        for name in names {
            std::fs::write(folder.join(name), name.as_bytes().repeat(40)).unwrap();
        }
        let token_path = dir.join("token.txt");
        std::fs::write(&token_path, "fake-token").unwrap();
        UploadArgs {
            queue: dir.join("queue.sqlite"),
            portal,
            token: token_path,
            folder: Some(folder),
            session_id: None,
            plan_id: None,
            target_ids: Vec::new(),
            json: true,
            language_of_messages: crate::cli::LangArg::default(),
        }
    }

    fn states_by_name(args: &UploadArgs) -> Vec<(String, QueueState)> {
        let mut states: Vec<(String, QueueState)> = UploadQueue::open(&args.queue)
            .unwrap()
            .list()
            .unwrap()
            .into_iter()
            .map(|entry| {
                (
                    entry
                        .source_path
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                    entry.state,
                )
            })
            .collect();
        states.sort_by(|a, b| a.0.cmp(&b.0));
        states
    }

    /// How many creations of an upload reached the application (the
    /// finalize routes share the prefix of the create).
    fn creations_seen(service: &fake_service::FakeService) -> usize {
        service.app_hits("POST", "/api/v1/uploads") - service.app_hits("POST", "/api/v1/uploads/")
    }

    fn quick_options(max_replays: u32) -> DriveOptions {
        DriveOptions {
            backoff: astro_sync_core::gui_bridge::Backoff::new(vec![Duration::from_millis(5)]),
            max_replays: Some(max_replays),
            replay_when_unanswered: false,
        }
    }

    /// A space that already keeps too many uploads open: the command says
    /// it will try again, waits (the wait is said, with the number of the
    /// try), tries again by itself, and sends the file -- never marking it.
    #[test]
    fn upload_waits_and_tries_again_by_itself_when_the_space_has_too_many_uploads_open() {
        use fake_service::{FakeService, UploadModel, Wake, error_reply};

        let dir = unique_temp_dir("upload-sessions-limit");
        let model = Arc::new(UploadModel::default());
        let app_model = Arc::clone(&model);
        let refused = Arc::new(Mutex::new(0_u32));
        let counted = Arc::clone(&refused);
        let service = FakeService::start(Wake::Instant, move |request| {
            if request.method == "POST" && request.path == "/api/v1/uploads" {
                let mut refused = counted.lock().unwrap();
                if *refused < 3 {
                    *refused += 1;
                    return error_reply(
                        409,
                        "upload_sessions_limit_reached",
                        "Raw service sentence about open uploads.",
                    );
                }
            }
            app_model
                .reply(request, 100)
                .unwrap_or_else(|| error_reply(404, "not_found", "no such route"))
        });
        let args = args_with_files(&dir, service.base_url(), &["frame.tif"]);
        let said: Arc<Mutex<Vec<String>>> = Arc::default();
        let sink = Arc::clone(&said);

        upload_with(
            &args,
            fast_policy(),
            quick_options(5),
            "en",
            Arc::new(move |line: &str| sink.lock().unwrap().push(line.to_string())),
        )
        .unwrap();

        let said = said.lock().unwrap().clone();
        assert_eq!(said.len(), 3, "one line per wait: {said:?}");
        for (index, line) in said.iter().enumerate() {
            assert!(
                line.contains("Too many uploads are in progress in your space")
                    && line.contains("has not failed")
                    && line.contains(&format!("attempt {}", index + 1)),
                "{line}"
            );
            assert!(
                !line.contains("upload_sessions_limit_reached")
                    && !line.contains("Raw service sentence"),
                "never the raw code nor the service's English: {line}"
            );
        }
        assert_eq!(
            states_by_name(&args),
            [("frame.tif".to_string(), QueueState::Completed)]
        );
        // 3 refusals, then the create that went through: no more.
        assert_eq!(creations_seen(&service), 4);
        assert_eq!(model.captures_created(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The command line stops waiting after the number of new tries it is
    /// allowed, and leaves the file queued: unmarked, for the next run.
    #[test]
    fn upload_gives_up_waiting_after_its_tries_and_leaves_the_file_queued() {
        use fake_service::{FakeService, Wake, error_reply};

        let dir = unique_temp_dir("upload-sessions-limit-forever");
        let service = FakeService::start(Wake::Instant, |request| {
            if request.method == "GET" {
                return fake_service::identity_reply();
            }
            error_reply(
                409,
                "upload_sessions_limit_reached",
                "Raw service sentence about open uploads.",
            )
        });
        let args = args_with_files(&dir, service.base_url(), &["frame.tif"]);
        let said: Arc<Mutex<Vec<String>>> = Arc::default();
        let sink = Arc::clone(&said);

        upload_with(
            &args,
            fast_policy(),
            quick_options(2),
            "en",
            Arc::new(move |line: &str| sink.lock().unwrap().push(line.to_string())),
        )
        .unwrap();

        assert_eq!(said.lock().unwrap().len(), 2, "two new tries, said");
        assert_eq!(
            states_by_name(&args),
            [("frame.tif".to_string(), QueueState::Queued)]
        );
        assert_eq!(
            creations_seen(&service),
            3,
            "the first try and the two new ones, not a loop"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
