//! Rendering: `--json` prints the exact `astro-sync-core` structures the
//! Tauri GUI will also consume (`serde_json::to_string_pretty`); otherwise a
//! plain, readable table goes to stdout.

use astro_sync_core::analysis::AnalysisOutcome;
use astro_sync_core::analysis::report::SessionReport;
use astro_sync_core::analysis::siril::{ProgressionOutcome, SirilDetection, SirilStatus};
use astro_sync_core::analysis::sky_night::{FolderSkyOutcome, reason_sentence};
use astro_sync_core::dry_run::DryRunReport;
use astro_sync_core::package::PackageOutcome;
use astro_sync_core::scanner::{FileClass, ScanReport};
use astro_sync_core::upload_queue::{QueueEntry, QueueState};
use serde::Serialize;
use std::error::Error;
use std::path::Path;

pub fn print_json<T: Serialize>(value: &T) -> Result<(), Box<dyn Error>> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

/// The kind of a file as the table names it: the words are the language's,
/// the formats (`raw`, `fits`, `xisf`) are their own names.
fn class_label(class: FileClass, locale: &str) -> String {
    let (key, plain) = match class {
        FileClass::Image => ("cli.class.image", "image"),
        FileClass::Video => ("cli.class.video", "video"),
        FileClass::Log => ("cli.class.log", "log"),
        FileClass::Other => ("cli.class.other", "other"),
        FileClass::Raw => return "raw".to_string(),
        FileClass::Fits => return "fits".to_string(),
        FileClass::Xisf => return "xisf".to_string(),
    };
    crate::words::say(locale, key, &[]).unwrap_or_else(|| plain.to_string())
}

/// Shortens a SHA-256 hex digest for a readable table column; the full
/// digest is always available via `--json`.
fn short_hash(hash: &str) -> &str {
    hash.get(..12).unwrap_or(hash)
}

/// A column title or a cell, padded to `width` characters as the person sees
/// them (not bytes: the dictionaries hold wide letters).
fn pad(text: &str, width: usize, right: bool) -> String {
    let length = text.chars().count();
    let fill = " ".repeat(width.saturating_sub(length));
    if right {
        format!("{fill}{text}")
    } else {
        format!("{text}{fill}")
    }
}

pub fn print_scan_table(report: &ScanReport, locale: &str) {
    let words =
        |key: &str, vars: &[(&str, &str)]| crate::words::say(locale, key, vars).unwrap_or_default();
    println!(
        "{}",
        words(
            "cli.scan.folder",
            &[("folder", &report.root.display().to_string())]
        )
    );
    if report.files.is_empty() {
        println!("{}", words("cli.scan.none", &[]));
        return;
    }
    println!(
        "{} {} {} {}",
        pad(&words("cli.scan.path", &[]), 48, false),
        pad(&words("cli.queue.bytes", &[]), 12, true),
        pad(&words("cli.scan.class", &[]), 12, false),
        pad("SHA-256", 14, false)
    );
    let mut total_bytes = 0_u64;
    for file in &report.files {
        println!(
            "{:<48} {:>12} {} {:<14}",
            file.relative_path,
            file.bytes,
            pad(&class_label(file.class, locale), 12, false),
            short_hash(&file.sha256)
        );
        total_bytes += file.bytes;
    }
    let files = crate::words::say_plural(locale, "cli.scan.files", report.files.len() as u64, &[])
        .unwrap_or_default();
    println!(
        "--\n{}",
        words(
            "cli.scan.total",
            &[
                ("files", &files),
                ("size", &crate::words::size_text(locale, total_bytes)),
            ],
        )
    );
}

pub fn print_dry_run_table(report: &DryRunReport) {
    println!("Source folder: {}", report.source_root.display());
    if report.entries.is_empty() {
        println!("No file to send.");
        return;
    }
    println!(
        "{:<36} {:>12} {:<8} {:<36} {:<14}",
        "Source", "Bytes", "Class", "Destination", "SHA-256"
    );
    for entry in &report.entries {
        println!(
            "{:<36} {:>12} {:<8} {:<36} {:<14}",
            entry.source_relative_path,
            entry.bytes,
            class_label(entry.class, "en"),
            entry.package_relative_path,
            short_hash(&entry.sha256)
        );
    }
    println!(
        "--\n{} file(s), {} byte(s) in total -- nothing is sent without explicit approval",
        report.entries.len(),
        report.total_bytes
    );
}

pub fn print_package_outcome(outcome: &PackageOutcome) {
    println!("Package written: {}", outcome.output_dir.display());
    println!("  manifest.json    : {}", outcome.manifest_path.display());
    println!("  checksums.sha256 : {}", outcome.checksums_path.display());
    println!(
        "  {} file(s), {} byte(s) in total",
        outcome.asset_count, outcome.total_bytes
    );
    println!("The source folder was not modified.");
}

/// What `sky` prints: the list, the poses kept, each night's value, and why
/// the others gave none.
pub fn print_sky_summary(outcome: &FolderSkyOutcome, folder: &Path) {
    println!("Sky background measured: {}", folder.display());
    if outcome.reference_stars == 0 {
        println!("  Reference star list: absent (option --stars)");
    } else {
        println!("  Reference star list: {} stars", outcome.reference_stars);
    }
    let kept = outcome.poses.iter().filter(|pose| pose.kept).count();
    println!(
        "  Frames: {} read, {} kept; {} dark(s) or bias",
        outcome.poses.len(),
        kept,
        outcome.calibration_frames
    );
    for night in &outcome.nights {
        println!(
            "  Night of {}: {:.2} mag/arcsec² (median of {} frames, method {})",
            night.night, night.sqm, night.pose_count, night.method_version
        );
    }
    if outcome.nights.is_empty() {
        println!("  No value: nothing is guessed.");
    }
    let mut reasons: Vec<(&str, usize)> = Vec::new();
    for reason in outcome
        .poses
        .iter()
        .filter_map(|pose| pose.reason.as_deref())
    {
        match reasons.iter_mut().find(|(code, _)| *code == reason) {
            Some((_, count)) => *count += 1,
            None => reasons.push((reason, 1)),
        }
    }
    for (code, count) in reasons {
        println!(
            "  {count} frame(s) without a value: {}",
            reason_sentence(code).unwrap_or(code)
        );
    }
}

pub fn print_analyze_summary(outcome: &AnalysisOutcome, out: &Path) {
    println!("Analysis written to: {}", out.display());
    println!(
        "  metrics/quality.csv  : {} data row(s)",
        outcome.quality_csv.lines().count().saturating_sub(1)
    );
    println!(
        "  metrics/guiding.csv  : {} data row(s)",
        outcome.guiding_csv.lines().count().saturating_sub(1)
    );
    println!(
        "  events/session.jsonl : {} event(s)",
        outcome.session_jsonl.lines().count()
    );
    println!(
        "Logs recognised: {} PHD2, {} ASIAIR, {} FITS header(s)",
        outcome.phd2_files_parsed, outcome.asiair_files_parsed, outcome.fits_files_parsed
    );
    if !outcome.acquisition_families.is_empty() {
        println!("Acquisition families:");
        for family in &outcome.acquisition_families {
            println!("  {} -- {} frame(s)", family.key, family.frame_paths.len());
        }
    }
}

pub fn print_report_summary(report: &SessionReport, out: &Path) {
    println!("Report written to: {}", out.display());
    println!("  report version       : {}", report.report_version);
    println!("  generated at         : {}", report.generated_at_utc);
    println!(
        "  logs recognised      : {} PHD2, {} ASIAIR, {} NINA, {} FITS or XISF header(s)",
        report.source.phd2_files_parsed,
        report.source.asiair_files_parsed,
        report.source.nina_files_parsed,
        report.source.fits_files_parsed
    );
    if let Some(scale) = report.pixel_scale_arcsec {
        println!("  scale                : {scale} ″/px");
    }
    println!("  quality              : {} frame(s)", report.quality.len());
    match &report.guiding_trace {
        Some(trace) => println!(
            "  guiding              : {} sample(s) in {} point(s) of {} s",
            trace.samples,
            trace.points.len(),
            trace.interval_seconds
        ),
        None => println!("  guiding              : none"),
    }
    println!(
        "  events               : {} event(s)",
        report.timeline.events.len()
    );
    println!(
        "  guiding per slice    : {} slice(s) of 2 min",
        report.guiding_series.len()
    );
    if !report.acquisition_steps.is_empty() {
        println!("Acquisition steps:");
        for step in &report.acquisition_steps {
            println!(
                "  {} -- {} frame(s){}",
                step.key,
                step.frame_count,
                step.target_hint
                    .as_deref()
                    .map(|target| format!(" · {target}"))
                    .unwrap_or_default()
            );
        }
    }
    if report.is_empty() {
        println!("This report holds nothing: it would not be sent.");
    }
}

pub fn print_siril_detection(detection: &SirilDetection) {
    match detection.status {
        SirilStatus::Compatible => println!(
            "Siril detected: {} (version {}, compatible)",
            detection.path.as_deref().unwrap_or("?"),
            detection.version.as_deref().unwrap_or("?")
        ),
        SirilStatus::IncompatibleVersion => println!(
            "Siril detected: {} (version {}, not in the allowlist: {})",
            detection.path.as_deref().unwrap_or("?"),
            detection.version.as_deref().unwrap_or("?"),
            detection.compatible_versions.join(", ")
        ),
        SirilStatus::NotFound => println!(
            "Siril not found (allowlist of compatible versions: {})",
            detection.compatible_versions.join(", ")
        ),
    }
}

pub fn print_progression_outcome(outcome: &ProgressionOutcome) {
    match outcome {
        ProgressionOutcome::Available(record) => {
            println!("Progression generated with Siril {}", record.siril_version);
            println!("  script: {}", record.script_path);
            println!(
                "  family: {} ({} milestone(s))",
                record.acquisition_family.key,
                record.milestones.len()
            );
            for milestone in &record.milestones {
                println!(
                    "  milestone {} -- {} frame(s) -- {}",
                    milestone.index, milestone.frame_count, milestone.output_path
                );
            }
            println!(
                "  display scale (final stack): {}x{} px",
                record.final_stack_display_scale.width_pixels,
                record.final_stack_display_scale.height_pixels
            );
        }
        ProgressionOutcome::Unavailable {
            reason,
            script_path,
            ..
        } => {
            println!("Progression unavailable: {reason}");
            if !script_path.is_empty() {
                println!("  script kept for reference: {script_path}");
            }
        }
    }
}

fn state_label(state: QueueState, locale: &str) -> String {
    let (key, plain) = match state {
        QueueState::Queued => ("cli.queue.state.queued", "queued"),
        QueueState::Uploading => ("cli.queue.state.uploading", "uploading"),
        QueueState::Completed => ("cli.queue.state.completed", "completed"),
        QueueState::Failed => ("cli.queue.state.failed", "failed"),
        QueueState::Refused => ("cli.queue.state.refused", "refused"),
        QueueState::Held => ("cli.queue.state.held", "on hold"),
    };
    crate::words::say(locale, key, &[]).unwrap_or_else(|| plain.to_string())
}

/// Why a file was set aside, in plain words: the sentence of its code in the
/// person's language when Astro Sync has one, else the text the queue kept.
fn reason_text(entry: &QueueEntry, locale: &str) -> Option<String> {
    let kept = entry.last_error.clone().unwrap_or_default();
    entry
        .last_error_code
        .as_deref()
        .map(|code| {
            crate::words::say(locale, &format!("errors.{code}"), &[])
                .filter(|sentence| !sentence.contains('{'))
                .or_else(|| astro_sync_core::refusal::plain_sentence(code).map(str::to_string))
                .unwrap_or_else(|| kept.clone())
        })
        .or_else(|| entry.last_error.clone())
}

pub fn print_queue_table(entries: &[QueueEntry], locale: &str) {
    let words =
        |key: &str, vars: &[(&str, &str)]| crate::words::say(locale, key, vars).unwrap_or_default();
    if entries.is_empty() {
        println!("{}", words("cli.queue.empty", &[]));
        return;
    }
    println!(
        "{} {} {} {} {}",
        pad(&words("cli.queue.file", &[]), 40, false),
        pad(&words("cli.queue.bytes", &[]), 12, true),
        pad(&words("cli.queue.sent", &[]), 12, true),
        pad(&words("cli.queue.state", &[]), 16, false),
        pad("SHA-256", 14, false)
    );
    for entry in entries {
        println!(
            "{:<40} {:>12} {:>12} {} {:<14}",
            entry.source_path.display(),
            entry.size,
            entry.offset,
            pad(&state_label(entry.state, locale), 16, false),
            short_hash(&entry.sha256)
        );
        if let Some(reason) = reason_text(entry, locale) {
            println!("  {}", words("cli.queue.lastError", &[("reason", &reason)]));
        }
    }
}
