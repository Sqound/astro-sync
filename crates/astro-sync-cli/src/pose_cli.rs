//! `pose-stats`: "Frame measurement" of a target on the command line -- the
//! window's measure (`astro_sync_core::pose_stats`), the same script, the same
//! judgement. Reads the frames, writes nothing but the scratch list of the batch
//! (removed at the end).

use std::error::Error;
use std::path::PathBuf;

use clap::Args;

use astro_sync_core::pose_stats::{self, MeasureRequest, PoseEvent, PythonMeasurer, Reason};
use astro_sync_core::python_env;
use astro_sync_core::tools::{self, ToolState};

type Outcome = Result<(), Box<dyn Error>>;

/// `pose-stats <night> <target>`: measure each frame (stars, sky background) and flag those that stray.
#[derive(Args, Debug)]
pub struct PoseStatsArgs {
    /// Folder of the night.
    pub night: PathBuf,
    /// Folder of the target in the night (its frames are in `<target>/lights/`).
    pub target_folder: String,
    /// Astro Sync data (stacking-env/ environment).
    #[arg(long)]
    pub data: PathBuf,
    /// Folder of the stacking scripts (stacking).
    #[arg(long)]
    pub stacking: PathBuf,
    /// Evening of the night, YYYY-MM-DD (the night's window: local noon to local noon).
    #[arg(long)]
    pub evening_date: String,
    /// Longitude of the site, degrees east.
    #[arg(long, allow_hyphen_values = true)]
    pub longitude: Option<f64>,
    /// Latitude of the site, degrees north (to say "dawn").
    #[arg(long, allow_hyphen_values = true)]
    pub latitude: Option<f64>,
    /// Offset of the displayed times, in minutes east of UTC (default: the longitude's).
    #[arg(long, allow_hyphen_values = true)]
    pub utc_offset_minutes: Option<i64>,
    /// Write the whole result as JSON.
    #[arg(long)]
    pub json: bool,
}

fn reason_text(reason: &Reason) -> String {
    match reason {
        Reason::Dawn { factor } => format!("sky background {factor} times higher: dawn"),
        Reason::Sky { factor } => format!("sky background {factor} times higher"),
        Reason::Clouds => "few stars: probable cloud".to_string(),
        Reason::Trailed => "elongated stars".to_string(),
        Reason::Wide => "stars wider than the rest of the night".to_string(),
    }
}

fn clock(unix: Option<f64>, offset_minutes: i64) -> String {
    let Some(unix) = unix else {
        return "--:--".to_string();
    };
    let seconds = unix as i64 + offset_minutes * 60;
    let of_day = seconds.rem_euclid(86_400);
    format!("{:02}:{:02}", of_day / 3600, (of_day % 3600) / 60)
}

pub fn pose_stats(args: &PoseStatsArgs) -> Outcome {
    let python = python_env::env_python(&args.data);
    let script = args.stacking.join("pose_stats.py");
    // Siril converts the camera RAW poses before they are measured.
    let siril = tools::detect_tools(&crate::night::detect_context(
        &args.data,
        Some(args.stacking.as_path()),
    ))
    .into_iter()
    .find(|row| row.id == "siril" && row.state == ToolState::Present)
    .and_then(|row| row.path);
    let measurer = PythonMeasurer::for_night(&args.night, python, script).with_siril(siril);
    let request = MeasureRequest {
        night_folder: args.night.clone(),
        target_folder: args.target_folder.clone(),
        evening_date: args.evening_date.clone(),
        longitude: args.longitude,
        latitude: args.latitude,
        resume: Vec::new(),
    };
    let mut seen = 0usize;
    let stats = pose_stats::measure_target(&request, &measurer, None, &mut |event| {
        if let PoseEvent::Neighbours { count, planned } = event {
            eprintln!("{count} neighbours of flagged frames added ({planned} frames planned)");
        } else {
            seen += 1;
            if seen.is_multiple_of(25) {
                eprintln!("{seen} frames measured");
            }
        }
    })
    .map_err(|reason| format!("measure impossible: {reason}"))?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&stats)?);
        return Ok(());
    }
    let offset = args
        .utc_offset_minutes
        .unwrap_or_else(|| (args.longitude.unwrap_or(0.0) / 15.0).round() as i64 * 60);
    println!(
        "{} frames measured of {} ({} planned), {} flagged.",
        stats.measured_count(),
        stats.total,
        stats.planned,
        stats.flagged_count()
    );
    for family in &stats.families {
        println!(
            "\nFamily {} \u{b7} {} frames, {} measured{}",
            family.label,
            family.total,
            family.measured.len(),
            if family.judged {
                ""
            } else {
                " \u{b7} no median (fewer than ten frames)"
            }
        );
        let m = &family.medians;
        println!(
            "  medians: width {} px · eccentricity {} · stars {} · background {} ADU",
            m.fwhm_pixels.map_or("—".into(), |v| format!("{v:.1}")),
            m.eccentricity.map_or("—".into(), |v| format!("{v:.2}")),
            m.star_count.map_or("—".into(), |v| format!("{v:.0}")),
            m.sky_background.map_or("—".into(), |v| format!("{v:.0}")),
        );
        if let Some(missing) = family.shape_missing {
            println!("  {missing} frames without a width (fewer than twenty stars)");
        }
        if let Some(shape) = &family.family_shape {
            println!(
                "  family width: {:.1} px (median of {} stars read over {} frames){}",
                shape.fwhm_pixels,
                shape.stars,
                shape.poses,
                shape
                    .eccentricity
                    .map_or(String::new(), |v| format!(" · eccentricity {v:.2}"))
            );
        }
        for flagged in &family.flagged {
            let pose = family.measured.iter().find(|p| p.path == flagged.path);
            if let Some(pose) = pose {
                println!(
                    "  no {:>3}  {}  fwhm {:>4}  ecc {:>4}  stars {:>3}  bkg {:>6}  {}  {}",
                    pose.index + 1,
                    clock(pose.unix, offset),
                    pose.fwhm_pixels.map_or("—".into(), |v| format!("{v:.1}")),
                    pose.eccentricity.map_or("—".into(), |v| format!("{v:.2}")),
                    pose.star_count.map_or("—".into(), |v| v.to_string()),
                    pose.sky_background
                        .map_or("—".into(), |v| format!("{v:.0}")),
                    reason_text(&flagged.reason),
                    // The name `stack --leave-out` takes.
                    flagged
                        .path
                        .file_name()
                        .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
                );
            }
        }
    }
    Ok(())
}
