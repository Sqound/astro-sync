//! `incidents`: "Incident list" of a target on the command line -- the
//! window's detection (`astro_sync_core::incidents`), the same readers, the
//! same thresholds. Reads the logs and the headers of the poses of the night
//! and writes nothing, nowhere: no file, no call that writes. With
//! `--session`, it also reads the signals already declared in the session
//! (`GET /v1/sessions/{id}/events`, read only) and prints the merged list;
//! there is deliberately no command that writes to the hosted service.

use std::error::Error;
use std::path::PathBuf;

use clap::Args;
use serde::Serialize;

use astro_sync_core::analysis::log_offset::ForcedLogOffsets;
use astro_sync_core::detection::NightWindow;
use astro_sync_core::incidents::{
    self, DeclaredEvents, DetectInput, Detection, IncidentKind, MergedRow, RowOrigin,
};

use crate::night::{fail, print, service};

type Outcome = Result<(), Box<dyn Error>>;

/// `incidents <night> <target>`: the incidents that the logs and the frames of a target show (nothing is written).
#[derive(Args, Debug)]
pub struct IncidentsArgs {
    /// Folder of the night.
    pub night: PathBuf,
    /// Folder of the target in the night (its frames are in `<target>/lights/`); `.`: all the frames of the night.
    pub target_folder: String,
    /// Evening of the night, YYYY-MM-DD (the night's window: local noon to local noon).
    #[arg(long)]
    pub evening_date: String,
    /// Longitude of the site, degrees east.
    #[arg(long, allow_hyphen_values = true)]
    pub longitude: Option<f64>,
    /// Offset of the displayed times, in minutes east of UTC (default: the longitude's).
    #[arg(long, allow_hyphen_values = true)]
    pub utc_offset_minutes: Option<i64>,
    /// UTC offset of the PHD2 logs (e.g. "+02:00"); absent: found from the `DATE-OBS` of the frames.
    #[arg(long)]
    pub phd2_utc_offset: Option<String>,
    /// UTC offset of the ASIAIR logs (e.g. "+02:00"); absent: found from the `DATE-OBS` of the frames.
    #[arg(long)]
    pub asiair_utc_offset: Option<String>,
    /// UTC offset of the NINA logs (e.g. "+02:00"); absent: found from the `DATE-OBS` of the frames.
    #[arg(long)]
    pub nina_utc_offset: Option<String>,
    /// Session of the service whose declared reports to read (read-only).
    #[arg(long)]
    pub session: Option<String>,
    /// Address of the service (with `--session`).
    #[arg(long, alias = "server")]
    pub portal: Option<String>,
    /// Device token file (with `--session`).
    #[arg(long, alias = "token-file")]
    pub token: Option<PathBuf>,
    /// Write the whole result as JSON.
    #[arg(long)]
    pub json: bool,
}

/// What `--json` prints: the detection, and with a session the signals read
/// and the merged list.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    detection: Detection,
    #[serde(skip_serializing_if = "Option::is_none")]
    declared: Option<DeclaredEvents>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rows: Option<Vec<MergedRow>>,
}

fn clock(instant: &str, offset_minutes: i64) -> String {
    // `2026-08-14T22:13:38Z` -> `00:13:38` at the place's clock.
    let time = instant
        .get(11..19)
        .filter(|_| instant.as_bytes().get(10) == Some(&b'T'));
    let parts: Vec<i64> = time
        .map(|t| t.split(':').filter_map(|p| p.parse().ok()).collect())
        .unwrap_or_default();
    let [hours, minutes, seconds] = parts[..] else {
        return instant.to_string();
    };
    let of_day = (hours * 3600 + minutes * 60 + seconds + offset_minutes * 60).rem_euclid(86_400);
    format!(
        "{:02}:{:02}:{:02}",
        of_day / 3600,
        (of_day % 3600) / 60,
        of_day % 60
    )
}

fn kind_label(kind: IncidentKind) -> &'static str {
    match kind {
        IncidentKind::Pause => "Pause",
        IncidentKind::Condition => "Sky condition",
        IncidentKind::EquipmentIssue => "Equipment incident",
        IncidentKind::Note => "Note",
        IncidentKind::ExposureChange => "Exposure change",
        IncidentKind::GuidingTimeouts => "Guiding settle timeouts",
        IncidentKind::SensorTemperatureDrift => "Temperature drift",
        IncidentKind::GuidingSpikes => "Guiding spikes",
    }
}

pub fn incidents(args: &IncidentsArgs) -> Outcome {
    let window = NightWindow::of_evening(&args.evening_date, args.longitude)
        .ok_or_else(|| fail(format!("unreadable evening date: {}", args.evening_date)))?;
    let input = DetectInput {
        night_folder: args.night.clone(),
        target_folder: Some(args.target_folder.clone()),
        window: Some(window),
        log_offsets: ForcedLogOffsets {
            phd2: args.phd2_utc_offset.clone(),
            asiair: args.asiair_utc_offset.clone(),
            nina: args.nina_utc_offset.clone(),
        },
    };
    let detection = incidents::detect_incidents(&input);

    // The signals already declared: a read, and only with a session.
    let (declared, rows) = match args.session.as_deref() {
        None => (None, None),
        Some(session) => {
            let (portal, token) = service(args.portal.as_ref(), args.token.as_ref())?;
            let transport = crate::words::portal_transport()?;
            let read = incidents::read_declared_events(&transport, &portal, &token, session)
                .map_err(|error| fail(format!("could not read the signals: {error}")))?;
            let rows = incidents::merge_rows(session, &read.events, &detection.incidents);
            (Some(read), Some(rows))
        }
    };

    let offset = args
        .utc_offset_minutes
        .unwrap_or_else(|| (args.longitude.unwrap_or(0.0) / 15.0).round() as i64 * 60);
    let report = Report {
        detection,
        declared,
        rows,
    };
    print(args.json, &report, || print_text(&report, offset))
}

fn print_text(report: &Report, offset: i64) {
    let detection = &report.detection;
    println!(
        "{} frames read, logs: {} ASIAIR, {} PHD2, {} NINA. {} incidents detected.",
        detection.poses,
        detection.logs.asiair,
        detection.logs.phd2,
        detection.logs.nina,
        detection.incidents.len()
    );
    if let Some(offsets) = &detection.logs_utc_offset {
        println!(
            "Log clock (UTC): ASIAIR {}, PHD2 {}, NINA {}{}.",
            offsets.asiair.as_deref().unwrap_or("—"),
            offsets.phd2.as_deref().unwrap_or("—"),
            offsets.nina.as_deref().unwrap_or("—"),
            if offsets.detected { " (found)" } else { "" }
        );
    }
    for problem in &detection.problems {
        println!("Problem: {} — {}", problem.code, problem.detail);
    }
    for incident in &detection.incidents {
        let span = match &incident.end {
            Some(end) => format!(
                "{} → {}",
                clock(&incident.start, offset),
                clock(end, offset)
            ),
            None => clock(&incident.start, offset),
        };
        println!(
            "\n{span}  {} ({})\n  key      {}\n  text     {}\n  evidence {:?} {} {}\n  payload  {}",
            kind_label(incident.event_type),
            incident.code,
            incident.id,
            incident.description,
            incident.evidence.source,
            incident.evidence.code,
            incident.evidence.params,
            incident.payload
        );
    }
    for unproposed in &detection.unproposed {
        println!(
            "\nDetected, not proposed: {} × {} {}",
            unproposed.count, unproposed.code, unproposed.params
        );
    }
    if let (Some(declared), Some(rows)) = (&report.declared, &report.rows) {
        println!(
            "\nSignals of the session: {} read{}.",
            declared.events.len(),
            if declared.truncated {
                " (incomplete read)"
            } else {
                ""
            }
        );
        for row in rows {
            let origin = match (row.origin, row.already_added) {
                (RowOrigin::Declared, _) => "declared",
                (RowOrigin::Detected, true) => "detected, already added",
                (RowOrigin::Detected, false) => "detected",
            };
            println!(
                "  {}  {:<22} {:<20} {}{}",
                clock(&row.at, offset),
                kind_label(row.kind),
                origin,
                row.text.as_deref().unwrap_or(""),
                row.near
                    .as_ref()
                    .map_or(String::new(), |near| format!("  [seems to go with {near}]"))
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_time_is_shown_at_the_place_s_clock() {
        assert_eq!(clock("2026-08-14T22:13:38Z", 120), "00:13:38");
        assert_eq!(clock("2026-08-14T22:13:38Z", 0), "22:13:38");
        assert_eq!(clock("not a time", 120), "not a time");
    }
}
