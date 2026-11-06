//! The night's analysis, sent (option "Logs" of
//! "Processing" and the "Logs" tab).
//!
//! [`analyze_night`] reads a night's folder on this computer: the report v3
//! from its logs (NINA, PHD2, ASIAIR) and the headers of its frames
//! ([`crate::analysis::report`]), and the "Measured sky background" computed
//! with it from the solved light frames when the reference list (Tycho-2,
//! Settings › Dependencies) is there ([`crate::analysis::sky_night`]).
//!
//! [`send_night_analysis`] sends them: the report to `POST
//! /v1/sessions/{id}/analyses`, which **replaces** the night's analysis (one
//! per night), then the night's sky value to `POST
//! /v1/place-sky-measures` (free, no capability). A report holding nothing is
//! **never sent** (`analysis_empty`): the analysis in place is kept. The raw
//! logs and the frames never leave the computer; only the values do.

use serde::Serialize;
use std::path::Path;

use crate::analysis::log_offset::{self, ForcedLogOffsets, LogsUtcOffset};
use crate::analysis::report::{
    LogClock, SessionReport, generate_report_with, render_report_json, report_from_outcome_with,
};
use crate::analysis::sky_night::{self, NightSkyMeasure, SkyOptions};
use crate::analysis::{AnalysisError, AnalysisOptions};
use crate::gui_bridge::{
    GuiBridgeError, PortalGuiTransport, SkyMeasureTransport, UploadedAnalysis, UploadedSkyMeasure,
    upload_analysis, upload_place_sky_measure,
};

/// What a night's folder gives, ready to be sent.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NightAnalysis {
    pub report: SessionReport,
    /// The night's measured sky background, when the solved light frames and
    /// the reference list give one.
    pub sky_measure: Option<NightSkyMeasure>,
    /// Why there is none: `no_reference_stars` (the list is not downloaded),
    /// `no_measure` (no solved frame of the night could be measured).
    pub sky_missing_reason: Option<String>,
    /// The clocks the logs were read with, and whether Astro Sync found them
    /// (`detected`) or the person gave them; `None` when the folder holds no
    /// PHD2, ASIAIR or NINA log, or the report was made with fixed options
    /// ([`analyze_night`]).
    pub logs_utc_offset: Option<LogsUtcOffset>,
}

impl NightAnalysis {
    /// Whether there is nothing to send (the report is empty and no sky
    /// value was measured).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.report.is_empty() && self.sky_measure.is_none()
    }
}

/// Reads `folder` (read only) into the report v2 and, when `stars` is given,
/// the night's sky value. The sky measure reads pixels: a caller runs this off
/// the window's thread.
pub fn analyze_night(
    folder: &Path,
    options: &AnalysisOptions,
    sky: Option<SkyOptions>,
) -> Result<NightAnalysis, AnalysisError> {
    analyze_night_with(folder, options, sky, None)
}

/// [`analyze_night`], telling `progress` which file the report
/// reads.
pub fn analyze_night_with(
    folder: &Path,
    options: &AnalysisOptions,
    sky: Option<SkyOptions>,
    progress: Option<crate::local_work::ProgressSink<'_>>,
) -> Result<NightAnalysis, AnalysisError> {
    let report = generate_report_with(folder, options, progress)?;
    let (sky_measure, sky_missing_reason) = measure_sky(folder, sky);
    Ok(NightAnalysis {
        report,
        sky_measure,
        sky_missing_reason,
        logs_utc_offset: None,
    })
}

/// [`analyze_night_with`] for the logs whose clock is not UTC: the offset of
/// each kind of log is `forced` by the person, or **found** from the frames'
/// `DATE-OBS` ([`log_offset`]) and said in [`NightAnalysis::logs_utc_offset`].
/// The command line and the window both read a night this way.
pub fn analyze_night_detecting(
    folder: &Path,
    forced: &ForcedLogOffsets,
    sky: Option<SkyOptions>,
) -> Result<NightAnalysis, AnalysisError> {
    analyze_night_detecting_with(folder, forced, sky, None)
}

/// [`analyze_night_detecting`], telling `progress` which file the report
/// reads.
pub fn analyze_night_detecting_with(
    folder: &Path,
    forced: &ForcedLogOffsets,
    sky: Option<SkyOptions>,
    progress: Option<crate::local_work::ProgressSink<'_>>,
) -> Result<NightAnalysis, AnalysisError> {
    let (outcome, logs_utc_offset) =
        log_offset::analyze_folder_detecting(folder, forced, progress)?;
    let mut report = report_from_outcome_with(&outcome, &crate::pose_stats::record::load(folder));
    // The report says which clocks its logs were read with.
    report.log_clock = logs_utc_offset.as_ref().map(LogClock::from);
    let (sky_measure, sky_missing_reason) = measure_sky(folder, sky);
    Ok(NightAnalysis {
        report,
        sky_measure,
        sky_missing_reason,
        logs_utc_offset,
    })
}

fn measure_sky(
    folder: &Path,
    sky: Option<SkyOptions>,
) -> (Option<NightSkyMeasure>, Option<String>) {
    match sky {
        Some(sky_options) if sky_options.stars.is_some() => {
            match sky_night::measure_folder_sky(folder, &sky_options) {
                Ok(outcome) => match outcome.nights.into_iter().next() {
                    Some(measure) => (Some(measure), None),
                    None => (None, Some("no_measure".to_string())),
                },
                Err(_) => (None, Some("no_measure".to_string())),
            }
        }
        _ => (None, Some("no_reference_stars".to_string())),
    }
}

/// What became of the night's sky value at the sending.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum SkyMeasureSending {
    /// Stored for the night's place (`replaced` when an earlier value was).
    Sent { stored: UploadedSkyMeasure },
    /// The service did not keep it (`place_not_found`: no place of the space
    /// near the frames); the analysis is sent all the same.
    Refused { code: String },
    /// No value to send.
    NotMeasured,
}

/// What [`send_night_analysis`] did.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NightAnalysisSent {
    /// `None` when the report was empty and only the sky value was sent.
    pub analysis: Option<UploadedAnalysis>,
    pub sky_measure: SkyMeasureSending,
    /// The clocks of the logs the report was read with (see
    /// [`NightAnalysis::logs_utc_offset`]), said again with the sending.
    pub logs_utc_offset: Option<LogsUtcOffset>,
}

/// The local refusal of an empty report: the service's own code, so the
/// window says it with the same sentence.
fn empty_refusal() -> GuiBridgeError {
    GuiBridgeError::Refused {
        code: "analysis_empty".to_string(),
        message: "The report holds no step, no measure, no guiding and no event: nothing was sent."
            .to_string(),
    }
}

/// Sends the night's analysis of `session_id`: the report (which replaces
/// the one in place), then the sky value. Nothing is sent when both are
/// empty (`analysis_empty`); an empty report with a sky value sends the
/// value alone, and the analysis in place is kept. A refusal of the report
/// stops before the sky value; a refusal of the sky value is kept in
/// [`SkyMeasureSending::Refused`] and does not undo the analysis.
pub fn send_night_analysis<T: PortalGuiTransport + SkyMeasureTransport>(
    transport: &T,
    portal_url: &str,
    token: &str,
    session_id: &str,
    night: &NightAnalysis,
) -> Result<NightAnalysisSent, GuiBridgeError> {
    if night.is_empty() {
        return Err(empty_refusal());
    }
    let analysis = if night.report.is_empty() {
        None
    } else {
        let body: serde_json::Value = serde_json::from_str(&render_report_json(&night.report))
            .map_err(|error| GuiBridgeError::InvalidResponse(error.to_string()))?;
        Some(upload_analysis(
            transport, portal_url, token, session_id, &body,
        )?)
    };
    let sky_measure = match &night.sky_measure {
        None => SkyMeasureSending::NotMeasured,
        Some(measure) => {
            let body = sky_night::upload_body(measure, Some(session_id));
            match upload_place_sky_measure(transport, portal_url, token, &body) {
                Ok(stored) => SkyMeasureSending::Sent { stored },
                Err(GuiBridgeError::Refused { code, .. }) => SkyMeasureSending::Refused { code },
                Err(other) => return Err(other),
            }
        }
    };
    Ok(NightAnalysisSent {
        analysis,
        sky_measure,
        logs_utc_offset: night.logs_utc_offset.clone(),
    })
}

/// Refuses a report body the window hands back if it holds nothing: the
/// check of [`send_night_analysis`] for the report alone (the "Logs"
/// tab sends the report it showed).
pub fn ensure_not_empty(report: &serde_json::Value) -> Result<(), GuiBridgeError> {
    let none = |key: &str| {
        report
            .get(key)
            .and_then(serde_json::Value::as_array)
            .is_none_or(Vec::is_empty)
    };
    let no_event = report
        .get("timeline")
        .and_then(|t| t.get("events"))
        .and_then(serde_json::Value::as_array)
        .is_none_or(Vec::is_empty);
    let no_trace = report
        .get("guidingTrace")
        .and_then(|t| t.get("points"))
        .and_then(serde_json::Value::as_array)
        .is_none_or(Vec::is_empty);
    if none("acquisitionSteps")
        && none("quality")
        && none("guiding")
        && no_trace
        && none("guidingSeries")
        && no_event
    {
        return Err(empty_refusal());
    }
    Ok(())
}
