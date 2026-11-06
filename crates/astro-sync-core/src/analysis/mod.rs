//! Local log/FITS analysis: parsers for guiding logs and FITS headers
//! ([`phd2`], [`asiair`], [`fits`]), the writers for the open session
//! package's `metrics/quality.csv`, `metrics/guiding.csv`, and
//! `events/session.jsonl`
//! (`contracts/schemas/session-package/metrics.md`), and
//! [`analyze_folder`], which ties all of the above together over a real
//! folder for `astro-sync-cli`'s `analyze` subcommand. [`siril`] is the
//! separate, optional Siril orchestration built on top of
//! [`fits::AcquisitionFamily`]'s grouping.
//!
//! **Evidence rule.** Every value this module ever writes into a report
//! traces to a value one of the parsers actually read (a header keyword, a
//! CSV column, a log line) -- an aggregate like `rmsTotalArcsec` is a
//! root-mean-square of directly recorded offsets, in one physical unit
//! (arcseconds), not a synthesized rating. **No composite "quality score"
//! is ever computed anywhere in this crate.** `analysis_never_produces_a_composite_score`
//! (below) checks this holds for the actual rendered output (CSV headers
//! and every JSON key `events/session.jsonl` can carry), not just for the
//! source as currently written, precisely so a future edit that quietly
//! adds a `score`/`rating`/`grade` field fails the test suite.

pub mod acquisition_steps;
pub mod asiair;
mod csv_writer;
pub mod ephemeris;
pub mod fits;
pub mod guiding_series;
pub mod guiding_trace;
mod jsonl_writer;
pub mod log_offset;
pub mod nina;
pub mod phd2;
pub mod photometry;
pub mod reference_catalogue;
pub mod report;
pub mod siril;
pub mod sky;
pub mod sky_night;
#[cfg(test)]
pub(crate) mod sky_synthetic;
pub mod wcs;

pub use csv_writer::{
    GUIDING_CSV_HEADER, GuidingRow, QUALITY_CSV_HEADER, QualityRow, write_guiding_csv,
    write_quality_csv,
};
pub use jsonl_writer::{SessionEvent, write_session_jsonl};

use crate::scanner::{self, FileClass};
use crate::time_util::parse_iso8601_utc_to_unix_seconds;
use serde_json::json;
use std::collections::HashMap;
use std::fmt;
use std::path::Path;

#[derive(Debug)]
pub enum AnalysisError {
    Scan(scanner::ScanError),
    Io {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    Phd2 {
        path: String,
        source: phd2::Phd2Error,
    },
    Asiair {
        path: String,
        source: asiair::AsiairError,
    },
    Nina {
        path: String,
        source: nina::NinaError,
    },
}

impl fmt::Display for AnalysisError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AnalysisError::Scan(source) => write!(f, "{source}"),
            AnalysisError::Io { path, source } => {
                write!(f, "io error at {}: {source}", path.display())
            }
            AnalysisError::Phd2 { path, source } => {
                write!(f, "PHD2 guide log {path}: {source}")
            }
            AnalysisError::Asiair { path, source } => {
                write!(f, "ASIAIR autorun log {path}: {source}")
            }
            AnalysisError::Nina { path, source } => {
                write!(f, "NINA log {path}: {source}")
            }
        }
    }
}

impl std::error::Error for AnalysisError {}

#[derive(Debug, Clone)]
pub struct AnalysisOptions {
    /// Fixed UTC offset every PHD2 guide log found in the folder is read
    /// with -- see `phd2::Phd2Options`.
    pub phd2_utc_offset: String,
    /// Fixed UTC offset every ASIAIR Autorun log found in the folder is
    /// read with -- see `asiair::AsiairOptions`.
    pub asiair_utc_offset: String,
    /// Fixed UTC offset every NINA log found in the folder is read with
    /// (NINA writes the computer's local time) -- see `nina::NinaOptions`.
    pub nina_utc_offset: String,
}

impl Default for AnalysisOptions {
    fn default() -> Self {
        Self {
            phd2_utc_offset: "+00:00".to_string(),
            asiair_utc_offset: "+00:00".to_string(),
            nina_utc_offset: "+00:00".to_string(),
        }
    }
}

/// One frame whose first header was read, with what the report v2 needs
/// beyond the acquisition record: its type, its `OBJECT`, its pixel scale,
/// and whether it is a light frame (a calibration frame makes neither a
/// step nor a quality row of the report).
#[derive(Debug, Clone, PartialEq)]
pub struct AnalysedFrame {
    pub path: String,
    pub record: fits::FitsAcquisitionRecord,
    pub object: Option<String>,
    pub pixel_scale_arcsec: Option<f64>,
    pub is_light: bool,
}

/// The three optional files this crate can hand to
/// `crate::package::append_analysis_assets`, ready to be written verbatim.
/// Each is independently optional: a caller who only ran a FITS-only
/// analysis (no guiding log found) still gets a `quality.csv`, with no
/// `guiding.csv` alongside it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AnalysisArtifacts {
    pub quality_csv: Option<String>,
    pub guiding_csv: Option<String>,
    pub session_jsonl: Option<String>,
}

/// One PHD2 guiding session's own recorded boundary
/// (`"Guiding Begins at "` / `"Guiding Ends at "`, `phd2::GuidingSession`),
/// kept alongside the file it was read from -- `collect_phd2` below folds a
/// session's samples/events into flat lists and would otherwise lose both.
/// `analysis::report` reads this to state a session's chronology
/// `startedAtUtc`/`endedAtUtc` as a directly recorded value rather than a
/// derived minimum/maximum over guiding samples.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuidingSessionBounds {
    pub source_file: String,
    pub started_at_utc: Option<String>,
    pub ended_at_utc: Option<String>,
    /// The guide camera's scale this section declares (`Pixel scale = 6.88
    /// arc-sec/px`): the report v3 says it, so a reader can show pixels.
    #[serde(skip)]
    pub pixel_scale_arcsec_per_px: Option<f64>,
}

/// The result of analyzing one folder: the three rendered files, the
/// acquisition families the FITS headers were grouped into, a count of how many
/// files of each kind were actually recognized and parsed (`0` for all three is
/// a valid, if uninteresting, outcome -- an empty or unrelated folder is not an
/// error), and the same rows and events pre-render
/// (`quality_rows`/`guiding_rows`/`events`) plus each PHD2 session's own
/// recorded start/end (`guiding_session_bounds`) -- exactly what
/// `analysis::report::generate_report` needs to compose a combined JSON report
/// without re-parsing the folder itself.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisOutcome {
    pub quality_csv: String,
    pub guiding_csv: String,
    pub session_jsonl: String,
    pub acquisition_families: Vec<fits::AcquisitionFamily>,
    pub phd2_files_parsed: usize,
    pub asiair_files_parsed: usize,
    pub fits_files_parsed: usize,
    pub quality_rows: Vec<QualityRow>,
    pub guiding_rows: Vec<GuidingRow>,
    pub events: Vec<SessionEvent>,
    pub guiding_session_bounds: Vec<GuidingSessionBounds>,
    /// NINA logs read, every frame whose header was read (in the order of
    /// `quality_rows`), and the NINA logs themselves (frames and target
    /// windows) for the report v2's steps and target hints.
    pub nina_files_parsed: usize,
    /// The guiding samples that count for the guiding statistics (the slices
    /// of the report, the RMS over a frame's exposure): `guiding_rows`
    /// without the frames taken while the guider settled after a dither,
    /// the first one after a lost star, those PHD2 gave an error code and
    /// those inside a meridian flip -- the excursions PHD2 itself leaves out.
    /// The raw rows all stay in `guiding_rows`.
    #[serde(skip)]
    pub steady_guiding_rows: Vec<GuidingRow>,
    /// The start of every light exposure the ASIAIR logs name, in the clock
    /// they were read with (`log_offset` finds that clock with them).
    #[serde(skip)]
    pub asiair_exposure_starts: Vec<f64>,
    /// The meridian flips the ASIAIR logs tell, `(begin, end)` in Unix
    /// seconds: the markers of the report v3's trace.
    #[serde(skip)]
    pub meridian_flip_spans: Vec<(f64, f64)>,
    #[serde(skip)]
    pub frames: Vec<AnalysedFrame>,
    #[serde(skip)]
    pub nina_logs: Vec<nina::NinaLog>,
}

impl AnalysisOutcome {
    /// Packages the three rendered files as [`AnalysisArtifacts`] for
    /// `crate::package::append_analysis_assets`. All three are always
    /// `Some` here (even a header-only CSV or an empty JSONL string is a
    /// valid, meaningful artifact -- "no events were recorded" is
    /// evidence, not an absence of it); a caller that wants to omit one
    /// entirely (e.g. no guiding log was ever found) can build
    /// [`AnalysisArtifacts`] directly instead of going through this.
    #[must_use]
    pub fn as_package_artifacts(&self) -> AnalysisArtifacts {
        AnalysisArtifacts {
            quality_csv: Some(self.quality_csv.clone()),
            guiding_csv: Some(self.guiding_csv.clone()),
            session_jsonl: Some(self.session_jsonl.clone()),
        }
    }
}

pub(crate) fn looks_like_phd2_guide_log(contents: &str) -> bool {
    contents.contains("Guiding Begins at ") || contents.contains("Frame,Time,mount")
}

pub(crate) fn looks_like_asiair_autorun_log(contents: &str) -> bool {
    contents.lines().take(20).any(|line| {
        line.len() >= 19
            && line.as_bytes().get(4) == Some(&b'/')
            && line.as_bytes().get(7) == Some(&b'/')
            && line.contains('[')
    })
}

/// The ASIAIR also writes each Autorun log a second time in Chinese
/// (`Autorun_Log_<stamp>_CHN.txt`, first line "日志开始于"): the same
/// events with other words, which the English reader does not recognise
/// (an autofocus without its temperature, a failed result that never
/// failed). It is a translation of its sibling, not another journal:
/// never read for events.
pub(crate) fn is_asiair_translation(file_name: &str, contents: &str) -> bool {
    let stem = file_name
        .rsplit_once('.')
        .map_or(file_name, |(stem, _)| stem)
        .to_ascii_lowercase();
    (stem.starts_with("autorun_log") && stem.ends_with("_chn"))
        || contents.lines().next().is_some_and(|first| {
            first
                .trim_start_matches('\u{feff}')
                .starts_with("日志开始于")
        })
}

fn root_mean_square(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let sum_of_squares: f64 = values.iter().map(|v| v * v).sum();
    Some((sum_of_squares / values.len() as f64).sqrt())
}

/// A guide frame that passed the structural exclusions (settling, lost star,
/// error code), waiting for the night's noise level to settle the frames of
/// the minutes that follow a dither.
struct GuideCandidate {
    row: GuidingRow,
    sample: NumericSample,
    /// (section ordinal in the analysis, dither ordinal in the section).
    recovery: Option<(usize, u32)>,
}

/// How far from the night's median offset (in robust standard deviations of
/// the night's calm frames, per axis) the guider must be to count as back
/// from a dither.
const RECOVERY_SIGMAS: f64 = 3.0;
/// The tolerance never goes under this, in arcseconds.
const RECOVERY_FLOOR_ARCSEC: f64 = 0.5;

fn median(values: &mut [f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    Some(if values.len() % 2 == 1 {
        values[middle]
    } else {
        (values[middle - 1] + values[middle]) / 2.0
    })
}

/// Median and robust standard deviation (1.4826 x the median absolute
/// deviation) of a series.
fn robust_location_scale(values: &[f64]) -> Option<(f64, f64)> {
    let center = median(&mut values.to_vec())?;
    let mut deviations: Vec<f64> = values.iter().map(|v| (v - center).abs()).collect();
    Some((center, 1.4826 * median(&mut deviations)?))
}

/// Beyond this many robust standard deviations from the night's median
/// offset a frame is a wrong star or a jump of the lock, not tracking.
const SPIKE_SIGMAS: f64 = 20.0;
/// The spike limit never goes under this, in arcseconds.
const SPIKE_FLOOR_ARCSEC: f64 = 10.0;

/// The frames that count for the guiding statistics: a frame of the recovery
/// after a dither is left out until the guider is first back within the noise
/// of the night (then it and the following ones count), and so is a frame
/// whose offset is far beyond that noise (the guider took another star, or
/// the lock jumped: PHD2 flags none of them).
fn resolve_recoveries(candidates: Vec<GuideCandidate>) -> (Vec<NumericSample>, Vec<GuidingRow>) {
    let calm = |pick: fn(&NumericSample) -> Option<f64>| -> Vec<f64> {
        candidates
            .iter()
            .filter(|c| c.recovery.is_none())
            .filter_map(|c| pick(&c.sample))
            .collect()
    };
    let ra = robust_location_scale(&calm(|s| s.ra_offset_arcsec));
    let dec = robust_location_scale(&calm(|s| s.dec_offset_arcsec));
    let within = |value: Option<f64>, reference: Option<(f64, f64)>, sigmas: f64, floor: f64| match (
        value, reference,
    ) {
        (Some(value), Some((center, scale))) => {
            (value - center).abs() <= (sigmas * scale).max(floor)
        }
        _ => true,
    };
    let mut back: std::collections::HashSet<(usize, u32)> = std::collections::HashSet::new();
    let mut samples = Vec::new();
    let mut rows = Vec::new();
    for candidate in candidates {
        let (sample_ra, sample_dec) = (
            candidate.sample.ra_offset_arcsec,
            candidate.sample.dec_offset_arcsec,
        );
        if !(within(sample_ra, ra, SPIKE_SIGMAS, SPIKE_FLOOR_ARCSEC)
            && within(sample_dec, dec, SPIKE_SIGMAS, SPIKE_FLOOR_ARCSEC))
        {
            continue;
        }
        if let Some(key) = candidate.recovery
            && !back.contains(&key)
        {
            if within(sample_ra, ra, RECOVERY_SIGMAS, RECOVERY_FLOOR_ARCSEC)
                && within(sample_dec, dec, RECOVERY_SIGMAS, RECOVERY_FLOOR_ARCSEC)
            {
                back.insert(key);
            } else {
                continue;
            }
        }
        samples.push(candidate.sample);
        rows.push(candidate.row);
    }
    (samples, rows)
}

/// One raw guiding sample kept for exposure-window correlation: the
/// parsed Unix timestamp alongside the already-built `GuidingRow` (whose
/// own `timestamp_utc` is the formatted string, not directly comparable as
/// a number).
struct NumericSample {
    unix_seconds: f64,
    ra_offset_arcsec: Option<f64>,
    dec_offset_arcsec: Option<f64>,
}

/// Analyzes every recognizable log or FITS file directly under `root`
/// (recursively, via `crate::scanner::walk_folder`'s read-only walk, which
/// hashes nothing and follows links) and
/// renders the three open-package files. Unrecognized text files (an
/// observing-notes `.txt`, an unrelated `.csv`) are silently skipped --
/// only a file this crate positively recognizes as a PHD2 guide log or an
/// ASIAIR Autorun log, and then fails to parse, is a hard error, since
/// that combination means real guiding data could not be read rather than
/// "this file was never guiding data to begin with". A FITS file whose
/// header cannot be parsed is likewise skipped rather than failing the
/// whole run: a folder of light frames is exactly the kind of place a
/// stray non-FITS or corrupt file turns up, and analysis of everything
/// else should not be blocked by it.
pub fn analyze_folder(
    root: &Path,
    options: &AnalysisOptions,
) -> Result<AnalysisOutcome, AnalysisError> {
    analyze_folder_with(root, options, None)
}

/// [`analyze_folder`], telling `progress` which file it reads, one report
/// per log or frame read (`done` of `total` of the files it reads; the
/// images and other files it skips are not counted).
pub fn analyze_folder_with(
    root: &Path,
    options: &AnalysisOptions,
    progress: Option<crate::local_work::ProgressSink<'_>>,
) -> Result<AnalysisOutcome, AnalysisError> {
    let scan = scanner::walk_folder(root, None).map_err(AnalysisError::Scan)?;
    let readable = scan
        .files
        .iter()
        .filter(|file| {
            matches!(
                file.class,
                FileClass::Log | FileClass::Fits | FileClass::Xisf | FileClass::Raw
            )
        })
        .count() as u64;
    let mut read = 0_u64;

    let mut guiding_rows: Vec<GuidingRow> = Vec::new();
    let mut guide_candidates: Vec<GuideCandidate> = Vec::new();
    let mut guiding_sections = 0_usize;
    let mut flip_spans: Vec<(f64, f64)> = Vec::new();
    let mut asiair_exposure_starts: Vec<f64> = Vec::new();
    let mut events: Vec<SessionEvent> = Vec::new();
    let mut fits_records: Vec<(String, fits::FitsAcquisitionRecord)> = Vec::new();
    let mut guiding_session_bounds: Vec<GuidingSessionBounds> = Vec::new();
    let mut phd2_files_parsed = 0_usize;
    let mut asiair_files_parsed = 0_usize;
    let mut fits_files_parsed = 0_usize;
    let mut nina_files_parsed = 0_usize;
    let mut frames: Vec<AnalysedFrame> = Vec::new();
    let mut nina_logs: Vec<nina::NinaLog> = Vec::new();

    for file in &scan.files {
        let full_path = root.join(&file.relative_path);
        if let Some(sink) = progress
            && matches!(
                file.class,
                FileClass::Log | FileClass::Fits | FileClass::Xisf | FileClass::Raw
            )
        {
            read += 1;
            sink(&crate::local_work::WorkProgress {
                kind: crate::local_work::WorkKind::Analysis,
                done: read,
                total: readable,
                bytes_done: 0,
                bytes_total: 0,
                current: Some(file.relative_path.clone()),
            });
        }
        match file.class {
            FileClass::Log => {
                let Ok(contents) = std::fs::read_to_string(&full_path) else {
                    continue; // not valid UTF-8 text: not a log this crate reads
                };
                if looks_like_phd2_guide_log(&contents) {
                    let log = phd2::parse_phd2_guide_log(
                        &contents,
                        &phd2::Phd2Options {
                            utc_offset: options.phd2_utc_offset.clone(),
                        },
                    )
                    .map_err(|source| AnalysisError::Phd2 {
                        path: file.relative_path.clone(),
                        source,
                    })?;
                    phd2_files_parsed += 1;
                    for session in &log.sessions {
                        guiding_session_bounds.push(GuidingSessionBounds {
                            source_file: file.relative_path.clone(),
                            started_at_utc: session.started_at_utc.clone(),
                            ended_at_utc: session.ended_at_utc.clone(),
                            pixel_scale_arcsec_per_px: session.pixel_scale_arcsec_per_px,
                        });
                    }
                    collect_phd2(
                        &log,
                        &mut guiding_rows,
                        &mut guide_candidates,
                        &mut guiding_sections,
                        &mut events,
                    );
                } else if is_asiair_translation(
                    file.relative_path.rsplit('/').next().unwrap_or_default(),
                    &contents,
                ) {
                    // The Chinese rendition of a sibling log: no event of its own.
                } else if looks_like_asiair_autorun_log(&contents) {
                    let log = asiair::parse_asiair_autorun_log(
                        &contents,
                        &asiair::AsiairOptions {
                            utc_offset: options.asiair_utc_offset.clone(),
                        },
                    )
                    .map_err(|source| AnalysisError::Asiair {
                        path: file.relative_path.clone(),
                        source,
                    })?;
                    asiair_files_parsed += 1;
                    flip_spans.extend(meridian_flip_spans(&log));
                    asiair_exposure_starts.extend(
                        log.exposure_start_events
                            .iter()
                            .filter_map(|e| parse_iso8601_utc_to_unix_seconds(&e.timestamp_utc)),
                    );
                    collect_asiair(&log, &mut events);
                } else if crate::detection::looks_like_nina_log(&contents) {
                    let log = nina::parse_nina_log(
                        &contents,
                        &nina::NinaOptions {
                            utc_offset: options.nina_utc_offset.clone(),
                        },
                    )
                    .map_err(|source| AnalysisError::Nina {
                        path: file.relative_path.clone(),
                        source,
                    })?;
                    nina_files_parsed += 1;
                    events.extend(log.events.iter().cloned());
                    nina_logs.push(log);
                }
            }
            FileClass::Fits | FileClass::Xisf | FileClass::Raw => {
                // The first header only: never the pixels of a frame for
                // an analysis that reads its keywords.
                if let Some(header) = crate::detection::read_frame_header(&full_path, file.class) {
                    let record = fits::extract_acquisition_record(&header);
                    let frame = crate::detection::frame_record(&header);
                    frames.push(AnalysedFrame {
                        path: file.relative_path.clone(),
                        record: record.clone(),
                        object: frame.object,
                        pixel_scale_arcsec: acquisition_steps::header_pixel_scale(&header),
                        is_light: is_light_frame(&file.relative_path, frame.image_type),
                    });
                    fits_records.push((file.relative_path.clone(), record));
                    fits_files_parsed += 1;
                }
            }
            FileClass::Image | FileClass::Video | FileClass::Other => {}
        }
    }

    events.sort_by(|a, b| a.timestamp_utc.cmp(&b.timestamp_utc));

    flip_spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (mut numeric_samples, mut steady_guiding_rows) = resolve_recoveries(guide_candidates);
    // The guider is re-acquiring the star around a meridian flip: those
    // frames say nothing of the tracking either.
    if !flip_spans.is_empty() {
        let inside = |at: f64| flip_spans.iter().any(|(from, to)| at >= *from && at <= *to);
        numeric_samples.retain(|sample| !inside(sample.unix_seconds));
        steady_guiding_rows.retain(|row| {
            parse_iso8601_utc_to_unix_seconds(&row.timestamp_utc).is_none_or(|at| !inside(at))
        });
    }

    let families = fits::group_into_families(&fits_records);
    let mut family_key_by_path: HashMap<&str, &str> = HashMap::new();
    for family in &families {
        for path in &family.frame_paths {
            family_key_by_path.insert(path.as_str(), family.key.as_str());
        }
    }

    let quality_rows: Vec<QualityRow> = fits_records
        .iter()
        .map(|(path, record)| {
            build_quality_row(path, record, &family_key_by_path, &numeric_samples)
        })
        .collect();

    Ok(AnalysisOutcome {
        quality_csv: write_quality_csv(&quality_rows),
        guiding_csv: write_guiding_csv(&guiding_rows),
        session_jsonl: write_session_jsonl(&events),
        acquisition_families: families,
        phd2_files_parsed,
        asiair_files_parsed,
        fits_files_parsed,
        quality_rows,
        guiding_rows,
        events,
        guiding_session_bounds,
        nina_files_parsed,
        steady_guiding_rows,
        asiair_exposure_starts,
        meridian_flip_spans: flip_spans,
        frames,
        nina_logs,
    })
}

/// The folders of a night that never hold a pose: the calibration frames,
/// what the stacking makes (`stacked/`, its scratch), the person's own final
/// images (`final/`) and the images a device stacked itself (`device/`: a
/// Seestar `Stacked_*.fit` carries the end of the night as its `DATE-OBS`, a
/// « pose » that would close every list of incidents with a gap that never
/// happened).
const NOT_POSES_FOLDERS: [&str; 5] = [
    "calibration",
    "stacked",
    "final",
    "device",
    ".astro-sync-work",
];

/// A light frame: a file under one of [`NOT_POSES_FOLDERS`] is never one,
/// whatever its header says (darks taken as « Light » by a capture program
/// stay darks, the master of a stacking is not a pose); elsewhere its header
/// says so, or it says nothing (sheet § 3: a frame without `IMAGETYP` takes
/// the type of its folder).
fn is_light_frame(relative_path: &str, image_type: Option<crate::detection::FrameType>) -> bool {
    let outside = relative_path.split(['/', '\\']).any(|segment| {
        NOT_POSES_FOLDERS
            .iter()
            .any(|folder| segment.eq_ignore_ascii_case(folder))
    });
    if outside {
        return false;
    }
    match image_type {
        Some(kind) => kind == crate::detection::FrameType::Light,
        None => true,
    }
}

fn build_quality_row(
    path: &str,
    record: &fits::FitsAcquisitionRecord,
    family_key_by_path: &HashMap<&str, &str>,
    numeric_samples: &[NumericSample],
) -> QualityRow {
    let (rms_total, rms_ra, rms_dec, sample_count) =
        match (record.date_obs_utc.as_deref(), record.exposure_seconds) {
            (Some(date_obs), Some(exposure_seconds)) => {
                match parse_iso8601_utc_to_unix_seconds(date_obs) {
                    Some(start) => {
                        let end = start + exposure_seconds;
                        let (ra, dec): (Vec<f64>, Vec<f64>) = numeric_samples
                            .iter()
                            .filter(|sample| {
                                sample.unix_seconds >= start && sample.unix_seconds <= end
                            })
                            .filter_map(|sample| {
                                Some((sample.ra_offset_arcsec?, sample.dec_offset_arcsec?))
                            })
                            .unzip();
                        let rms_ra = root_mean_square(&ra);
                        let rms_dec = root_mean_square(&dec);
                        let rms_total = match (rms_ra, rms_dec) {
                            (Some(ra), Some(dec)) => Some(ra.hypot(dec)),
                            _ => None,
                        };
                        (rms_total, rms_ra, rms_dec, Some(ra.len() as u64))
                    }
                    None => (None, None, None, None),
                }
            }
            _ => (None, None, None, None),
        };

    QualityRow {
        timestamp_utc: record.date_obs_utc.clone().unwrap_or_default(),
        acquisition_family: family_key_by_path
            .get(path)
            .map_or_else(String::new, |k| (*k).to_string()),
        filter: record.filter.clone(),
        exposure_seconds: record.exposure_seconds,
        gain: record.gain,
        iso: record.iso,
        binning: record.binning.clone(),
        sensor_temp_c: record.sensor_temp_c,
        rms_total_arcsec: rms_total,
        rms_ra_arcsec: rms_ra,
        rms_dec_arcsec: rms_dec,
        guiding_sample_count: sample_count,
        sky_background_adu: record.sky_background_adu,
        fwhm_pixels: record.fwhm_pixels,
        eccentricity: record.eccentricity,
        source_path: path.to_string(),
    }
}

/// The spans of the meridian flips an ASIAIR log tells: each
/// `[Meridian Flip|Begin]` up to the next `[Meridian Flip|End]` (a flip whose
/// end was never logged stops at its begin).
fn meridian_flip_spans(log: &asiair::AsiairAutorunLog) -> Vec<(f64, f64)> {
    let ends: Vec<f64> = log
        .meridian_flip_end_events
        .iter()
        .filter_map(|end| parse_iso8601_utc_to_unix_seconds(&end.timestamp_utc))
        .collect();
    log.meridian_flip_begin_events
        .iter()
        .filter_map(|begin| parse_iso8601_utc_to_unix_seconds(&begin.timestamp_utc))
        .map(|from| {
            let to = ends
                .iter()
                .copied()
                .filter(|end| *end >= from)
                .fold(f64::INFINITY, f64::min);
            (from, if to.is_finite() { to } else { from })
        })
        .collect()
}

fn collect_phd2(
    log: &phd2::Phd2Log,
    guiding_rows: &mut Vec<GuidingRow>,
    candidates: &mut Vec<GuideCandidate>,
    sections: &mut usize,
    events: &mut Vec<SessionEvent>,
) {
    for session in &log.sessions {
        *sections += 1;
        let section = *sections;
        for sample in &session.samples {
            let row = GuidingRow {
                timestamp_utc: sample.timestamp_utc.clone(),
                source: "phd2".to_string(),
                frame: sample.frame.map(u64::from),
                elapsed_seconds: Some(sample.elapsed_seconds),
                ra_offset_arcsec: Some(sample.ra_offset_arcsec),
                dec_offset_arcsec: Some(sample.dec_offset_arcsec),
                ra_duration_ms: sample.ra_duration_ms,
                dec_duration_ms: sample.dec_duration_ms,
                star_mass: sample.star_mass,
                snr: sample.snr,
                error_code: Some(sample.error_code),
                ra_direction: sample.ra_direction,
                dec_direction: sample.dec_direction,
            };
            // A frame taken while the guider settles, re-acquires the star
            // or failed is a row of the guiding file, never a point of its
            // statistics.
            if sample.exclusion.is_none() {
                candidates.push(GuideCandidate {
                    sample: NumericSample {
                        unix_seconds: parse_iso8601_utc_to_unix_seconds(&sample.timestamp_utc)
                            .unwrap_or(f64::NAN),
                        ra_offset_arcsec: Some(sample.ra_offset_arcsec),
                        dec_offset_arcsec: Some(sample.dec_offset_arcsec),
                    },
                    row: row.clone(),
                    recovery: sample.dither_recovery.map(|dither| (section, dither)),
                });
            }
            guiding_rows.push(row);
        }
        for settle in &session.settle_events {
            let outcome = match settle.outcome {
                phd2::SettleOutcome::Started => "started",
                phd2::SettleOutcome::Complete => "complete",
                phd2::SettleOutcome::Failed => "failed",
            };
            events.push(SessionEvent {
                event_type: "dither-settle".to_string(),
                timestamp_utc: settle.timestamp_utc.clone(),
                source: "phd2".to_string(),
                data: json!({"outcome": outcome, "precedingFrame": settle.preceding_frame}),
            });
        }
        // The events the night's guiding summary counts: a dither, and
        // a settling that failed ("settling timed out").
        for dither in &session.dither_events {
            events.push(SessionEvent {
                event_type: "dither".to_string(),
                timestamp_utc: dither.timestamp_utc.clone(),
                source: "phd2".to_string(),
                data: json!({"precedingFrame": dither.preceding_frame}),
            });
        }
        for settle in &session.settle_events {
            if settle.outcome == phd2::SettleOutcome::Failed {
                events.push(SessionEvent {
                    event_type: "settle-timeout".to_string(),
                    timestamp_utc: settle.timestamp_utc.clone(),
                    source: "phd2".to_string(),
                    data: json!({"precedingFrame": settle.preceding_frame}),
                });
            }
        }
        for star_lost in &session.star_lost_events {
            events.push(SessionEvent {
                event_type: "star-lost".to_string(),
                timestamp_utc: star_lost.timestamp_utc.clone(),
                source: "phd2".to_string(),
                data: json!({"statusText": star_lost.status_text, "precedingFrame": star_lost.preceding_frame}),
            });
        }
    }
    for pause in &log.pauses {
        let kind = match pause.kind {
            phd2::PauseKind::ExplicitMarker => "explicit-marker",
            phd2::PauseKind::SessionGap => "session-gap",
        };
        events.push(SessionEvent {
            event_type: "pause".to_string(),
            timestamp_utc: pause.paused_at_utc.clone(),
            source: "phd2".to_string(),
            data: json!({"kind": kind}),
        });
        if let Some(resumed_at) = &pause.resumed_at_utc {
            events.push(SessionEvent {
                event_type: "resume".to_string(),
                timestamp_utc: resumed_at.clone(),
                source: "phd2".to_string(),
                data: json!({"kind": kind}),
            });
        }
    }
}

fn collect_asiair(log: &asiair::AsiairAutorunLog, events: &mut Vec<SessionEvent>) {
    for dither in &log.dither_events {
        events.push(SessionEvent {
            event_type: "dither".to_string(),
            timestamp_utc: dither.timestamp_utc.clone(),
            source: "asiair".to_string(),
            data: json!({}),
        });
    }
    for settle in &log.settle_events {
        events.push(SessionEvent {
            event_type: "settle".to_string(),
            timestamp_utc: settle.timestamp_utc.clone(),
            source: "asiair".to_string(),
            data: json!({}),
        });
    }
    for timeout in &log.settle_timeout_events {
        events.push(SessionEvent {
            event_type: "settle-timeout".to_string(),
            timestamp_utc: timeout.timestamp_utc.clone(),
            source: "asiair".to_string(),
            data: json!({}),
        });
    }
    for autofocus in &log.autofocus_events {
        events.push(SessionEvent {
            event_type: "autofocus".to_string(),
            timestamp_utc: autofocus.timestamp_utc.clone(),
            source: "asiair".to_string(),
            data: json!({"temperatureC": autofocus.temperature_c}),
        });
    }
    for result in &log.autofocus_result_events {
        events.push(SessionEvent {
            event_type: "autofocus-result".to_string(),
            timestamp_utc: result.timestamp_utc.clone(),
            source: "asiair".to_string(),
            data: json!({"succeeded": result.succeeded}),
        });
    }
    for flip in &log.meridian_flip_events {
        events.push(SessionEvent {
            event_type: "meridian-flip".to_string(),
            timestamp_utc: flip.timestamp_utc.clone(),
            source: "asiair".to_string(),
            data: json!({}),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;
    use std::fs;

    /// Same synthetic-header builder as `fits::tests` (duplicated locally:
    /// `fits`'s copy is private to its own `#[cfg(test)]` module).
    fn build_fits_header(cards: &[(&str, &str)]) -> Vec<u8> {
        const CARD_LEN: usize = 80;
        const BLOCK_LEN: usize = 2880;
        let mut out = Vec::new();
        for (keyword, value) in cards {
            let card = format!("{keyword:<8}= {value}");
            let mut bytes = card.into_bytes();
            bytes.resize(CARD_LEN, b' ');
            out.extend_from_slice(&bytes);
        }
        let mut end_card = b"END".to_vec();
        end_card.resize(CARD_LEN, b' ');
        out.extend_from_slice(&end_card);
        let remainder = out.len() % BLOCK_LEN;
        if remainder != 0 {
            out.resize(out.len() + (BLOCK_LEN - remainder), b' ');
        }
        out
    }

    const PHD2_LOG: &str = "Guiding Begins at 2026-08-07 22:38:00\nPixel scale = 2.00 arc-sec/px\nFrame,Time,mount,dx,dy,RARawDistance,DECRawDistance,RAGuideDistance,DECGuideDistance,RADuration,RADirection,DECDuration,DECDirection,XStep,YStep,StarMass,SNR,ErrorCode\n1,1.000,Mount,0,0,0.100,0.100,0,0,0,,0,,0,0,30000,12,0\n2,3.000,Mount,0,0,0.200,0.100,0,0,0,,0,,0,0,30000,12,0\nINFO: SETTLING STATE CHANGE, Settling started\nINFO: SETTLING STATE CHANGE, Settling complete\nGuiding Ends at 2026-08-07 23:00:00\n";

    const ASIAIR_LOG: &str = "2026/08/07 22:39:00 [Guide] Dither\n2026/08/07 22:39:10 [Guide] Dither Settle\n2026/08/07 22:40:00 [AutoFocus|Begin] Run AF when temperature changed 2 degrees, 27.0\u{2103} changed to 24.0\u{2103}, exposure 3.0s Bin1, temperature 24.0\u{2103}\n2026/08/07 22:41:00 [AutoFocus|End] Auto focus succeeded\n";

    fn write_fixture_folder() -> TempDir {
        let dir = TempDir::new("analysis-folder");
        fs::write(dir.path().join("PHD2_GuideLog_2026-08-07.txt"), PHD2_LOG).unwrap();
        fs::write(dir.path().join("Autorun_Log_2026-08-07.txt"), ASIAIR_LOG).unwrap();
        fs::write(
            dir.path().join("frame-0001.fits"),
            build_fits_header(&[
                ("EXPTIME", "300.0"),
                ("FILTER", "'Ha      '"),
                ("GAIN", "100"),
                ("CCD-TEMP", "-10.0"),
                ("DATE-OBS", "'2026-08-07T22:38:00'"),
                ("XBINNING", "1"),
                ("YBINNING", "1"),
            ]),
        )
        .unwrap();
        fs::write(dir.path().join("notes.txt"), "just some observing notes").unwrap();
        dir
    }

    /// The ASIAIR writes every Autorun log a second time in
    /// Chinese; its autofocus has no temperature the reader knows and its
    /// result never says « succeeded ».
    const ASIAIR_LOG_CHN: &str = "日志开始于 2026/08/07 22:39:00\n2026/08/07 22:39:00 [Guide] 抖动\n2026/08/07 22:40:00 [AutoFocus|Begin] 温度变化2度，27.0\u{2103}变为24.0\u{2103}，曝光 3.0s Bin1\n2026/08/07 22:41:00 [AutoFocus|End] 自动对焦成功\n";

    #[test]
    fn the_chinese_rendition_of_an_asiair_log_adds_no_event() {
        let alone = write_fixture_folder();
        let alone_events = analyze_folder(alone.path(), &AnalysisOptions::default())
            .unwrap()
            .session_jsonl;

        let both = write_fixture_folder();
        fs::write(
            both.path().join("Autorun_Log_2026-08-07_CHN.txt"),
            ASIAIR_LOG_CHN,
        )
        .unwrap();
        let outcome = analyze_folder(both.path(), &AnalysisOptions::default()).unwrap();
        assert_eq!(
            outcome.asiair_files_parsed, 1,
            "the translation is not parsed"
        );
        assert_eq!(outcome.session_jsonl, alone_events);
        let autofocus = outcome
            .session_jsonl
            .lines()
            .filter(|l| l.contains("\"autofocus\""))
            .count();
        assert_eq!(autofocus, 1);
        assert!(!outcome.session_jsonl.contains("\"succeeded\":false"));

        // Under another name, its first line still gives it away.
        let renamed = write_fixture_folder();
        fs::write(renamed.path().join("copie-de-journal.txt"), ASIAIR_LOG_CHN).unwrap();
        let outcome = analyze_folder(renamed.path(), &AnalysisOptions::default()).unwrap();
        assert_eq!(outcome.session_jsonl, alone_events);
    }

    #[test]
    fn a_translation_is_told_by_its_name_or_its_first_line() {
        assert!(is_asiair_translation("Autorun_Log_2026-08-07_CHN.txt", ""));
        assert!(is_asiair_translation("autorun_log_x_chn.TXT", "x"));
        assert!(is_asiair_translation(
            "notes.txt",
            "日志开始于 2026/08/07 22:39:00\n"
        ));
        assert!(!is_asiair_translation(
            "Autorun_Log_2026-08-07.txt",
            ASIAIR_LOG
        ));
        assert!(!is_asiair_translation("PHD2_GuideLog_CHN.txt", PHD2_LOG));
    }

    #[test]
    fn analyzes_a_mixed_folder_end_to_end() {
        let dir = write_fixture_folder();
        let outcome = analyze_folder(dir.path(), &AnalysisOptions::default()).unwrap();

        assert_eq!(outcome.phd2_files_parsed, 1);
        assert_eq!(outcome.asiair_files_parsed, 1);
        assert_eq!(outcome.fits_files_parsed, 1);
        assert_eq!(outcome.acquisition_families.len(), 1);

        assert!(outcome.guiding_csv.starts_with(GUIDING_CSV_HEADER));
        assert_eq!(outcome.guiding_csv.lines().count(), 3); // header + 2 samples

        assert!(outcome.quality_csv.starts_with(QUALITY_CSV_HEADER));
        let quality_data_line = outcome.quality_csv.lines().nth(1).unwrap();
        assert!(quality_data_line.contains("Ha_300s_g100_1x1_t-10C"));
        // The exposure spans [22:38:00, 22:43:00); both guide samples
        // (22:38:01 and 22:38:03) fall inside it.
        assert!(quality_data_line.contains(",2,")); // guidingSampleCount

        let event_types: Vec<String> = outcome
            .session_jsonl
            .lines()
            .map(|line| {
                let value: serde_json::Value = serde_json::from_str(line).unwrap();
                value["type"].as_str().unwrap().to_string()
            })
            .collect();
        assert!(event_types.iter().any(|t| t == "dither-settle"));
        assert!(event_types.iter().any(|t| t == "dither"));
        assert!(event_types.iter().any(|t| t == "settle"));
        assert!(event_types.iter().any(|t| t == "autofocus"));
        assert!(event_types.iter().any(|t| t == "autofocus-result"));
    }

    #[test]
    fn an_empty_folder_produces_header_only_files_and_no_error() {
        let dir = TempDir::new("analysis-empty-folder");
        let outcome = analyze_folder(dir.path(), &AnalysisOptions::default()).unwrap();
        assert_eq!(outcome.phd2_files_parsed, 0);
        assert_eq!(outcome.asiair_files_parsed, 0);
        assert_eq!(outcome.fits_files_parsed, 0);
        assert_eq!(outcome.quality_csv, format!("{QUALITY_CSV_HEADER}\n"));
        assert_eq!(outcome.guiding_csv, format!("{GUIDING_CSV_HEADER}\n"));
        assert_eq!(outcome.session_jsonl, "");
    }

    #[test]
    fn an_unrelated_text_file_is_silently_skipped() {
        let dir = TempDir::new("analysis-unrelated-text");
        fs::write(dir.path().join("readme.txt"), "nothing to see here").unwrap();
        let outcome = analyze_folder(dir.path(), &AnalysisOptions::default()).unwrap();
        assert_eq!(outcome.phd2_files_parsed, 0);
        assert_eq!(outcome.asiair_files_parsed, 0);
    }

    #[test]
    fn a_recognized_but_malformed_phd2_log_is_a_hard_error() {
        let dir = TempDir::new("analysis-malformed-phd2");
        fs::write(
            dir.path().join("PHD2_GuideLog_bad.txt"),
            "Guiding Begins at not-a-timestamp\n",
        )
        .unwrap();
        let result = analyze_folder(dir.path(), &AnalysisOptions::default());
        assert!(matches!(result, Err(AnalysisError::Phd2 { .. })));
    }

    #[test]
    fn as_package_artifacts_always_carries_all_three_files() {
        let dir = write_fixture_folder();
        let outcome = analyze_folder(dir.path(), &AnalysisOptions::default()).unwrap();
        let artifacts = outcome.as_package_artifacts();
        assert!(artifacts.quality_csv.is_some());
        assert!(artifacts.guiding_csv.is_some());
        assert!(artifacts.session_jsonl.is_some());
    }

    /// The evidence rule, checked against real rendered output rather than
    /// against the source text (which legitimately discusses "score" in
    /// comments explaining why there isn't one): every CSV column name and
    /// every JSON key reachable from `events/session.jsonl` must not be (or
    /// contain) `score`, `rating`, or `grade`, case-insensitively. A future
    /// change that quietly adds a composite rating field -- to a CSV header,
    /// to an event's `data` object, at any nesting depth -- fails this test.
    #[test]
    fn what_a_device_stacked_or_a_stacking_made_is_never_a_pose() {
        use crate::detection::FrameType;
        // A pose of the target, with or without its header saying so.
        assert!(is_light_frame(
            "M45/lights/Light_001.fit",
            Some(FrameType::Light)
        ));
        assert!(is_light_frame("M45/lights/Light_001.fit", None));
        // The image the Seestar stacked, put in `device/`: its `DATE-OBS`
        // is the end of the night, it is no pose.
        assert!(!is_light_frame(
            "M45/device/Stacked_430_M 45_30.0s_IRCUT_20261011-061931.fit",
            Some(FrameType::Light)
        ));
        assert!(!is_light_frame("M45/Device/Stacked_8.fit", None));
        // The other folders that never hold a pose.
        for folder in ["calibration", "stacked", "final", ".astro-sync-work"] {
            assert!(
                !is_light_frame(&format!("M45/{folder}/a.fit"), None),
                "{folder}"
            );
        }
    }

    #[test]
    fn analysis_never_produces_a_composite_score() {
        const FORBIDDEN: [&str; 3] = ["score", "rating", "grade"];

        for header in [QUALITY_CSV_HEADER, GUIDING_CSV_HEADER] {
            for column in header.split(',') {
                let lower = column.to_ascii_lowercase();
                assert!(
                    !FORBIDDEN.iter().any(|f| lower.contains(f)),
                    "CSV column {column:?} looks like a composite score"
                );
            }
        }

        let dir = write_fixture_folder();
        let outcome = analyze_folder(dir.path(), &AnalysisOptions::default()).unwrap();
        for line in outcome.session_jsonl.lines() {
            let value: serde_json::Value = serde_json::from_str(line).unwrap();
            assert_no_forbidden_keys(&value, &FORBIDDEN);
        }
    }

    fn assert_no_forbidden_keys(value: &serde_json::Value, forbidden: &[&str]) {
        match value {
            serde_json::Value::Object(map) => {
                for (key, nested) in map {
                    let lower = key.to_ascii_lowercase();
                    assert!(
                        !forbidden.iter().any(|f| lower.contains(f)),
                        "JSON key {key:?} looks like a composite score"
                    );
                    assert_no_forbidden_keys(nested, forbidden);
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    assert_no_forbidden_keys(item, forbidden);
                }
            }
            _ => {}
        }
    }
}
