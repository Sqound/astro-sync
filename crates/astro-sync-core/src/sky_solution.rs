//! The sky solution of an image (`sky-solution.schema.json`).
//!
//! Computed on this computer, **always on the image that is sent** (or that
//! is online: the original fetched from the space, `night_resume::
//! fetch_original`), in this order:
//!
//! 1. **The header** ([`header_solution`], `source: header`, no computation):
//!    a solution NINA or ASIAIR already wrote, carried by the image itself (a
//!    TIFF whose description holds FITS cards) or by a `<name>.wcs` file
//!    beside it -- taken only when the header names this very image,
//!    uncropped (`NAXIS1` × `NAXIS2` = the image's size).
//! 2. **ASTAP** ([`solve_with_astap`], the default, `source: astap`), hinted
//!    by the target's position and the scale when known, given the image as
//!    an uncompressed 16-bit mono FITS whose background is levelled
//! ([`input`]: a deep stack never solved as it was).
//! 3. **astrometry.net** ([`solve_with_astrometry_net`], when the person chose
//!    it, `source: astrometry-net`).
//!
//! The solvers are external tools found by [`crate::tools`], never bundled;
//! they write only in a work folder, never next to the image: on the disk of
//! the night the image lies in, `<night>/.astro-sync-work/solve/<sha256>/`
//! ([`solve_work_root`]), else under [`SolveSetup::work_root`] (an original fetched from the space). That folder is
//! removed when the resolution ends, whatever its outcome, and `.astro-sync-work/` with it when empty. A failure is
//! said in words ([`SolveFailure::sentence`]: "Too few stars recognised in the image."), never as a code on screen.
//!
//! Sent by `PUT /v1/captures/{id}/sky-solution` with the SHA-256 of the
//! image it was computed on ([`SkySolutionTransport::put_sky_solution`]);
//! "Annotations" lists the captures still without one with
//! `GET /v1/captures?skySolution=missing` ([`list_missing`]). The
//! service never solves.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::analysis::fits::{FitsCard, FitsHeader, parse_primary_header};
use crate::analysis::sky::header_cd_matrix;
use crate::analysis::wcs::TanWcs;
use crate::external_process;
use crate::gui_bridge::{GuiBridgeError, HttpPortalGuiTransport};
use crate::local_work::CancelFlag;
use crate::night_resume::{OriginalTransport, SolveFrom, fetch_original};
use crate::raster::{ImageSize, image_size, is_fits_file, tiff_description};
use crate::service_wait::RequestFactory;
use crate::working_folder::solve_scratch_root;

pub mod input;
pub mod local;

pub use local::{LocalResult, hints_of_image, local_results};

/* ------------------------------ The solution ----------------------------- */

/// `source` of the contract (the `import` of the old portal is never
/// written by Astro Sync).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SolutionSource {
    Header,
    Astap,
    AstrometryNet,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Parity {
    /// East to the left of north, as the sky is seen.
    Normal,
    Mirrored,
}

/// The `solution` a client sends (`sky-solution.schema.json`, `solution`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkySolution {
    pub schema_version: u8,
    pub source: SolutionSource,
    pub solver_version: Option<String>,
    pub ra_deg: f64,
    pub dec_deg: f64,
    pub pixel_scale_arcsec: f64,
    pub rotation_deg: f64,
    pub field_width_deg: f64,
    pub field_height_deg: f64,
    pub parity: Parity,
    pub width: u32,
    pub height: u32,
    pub solved_at: String,
}

/// Which way a solver numbers the rows of a raster image: FITS row 1 is the
/// image's top row (`TopDown`) or its bottom row (`BottomUp`, the FITS
/// display convention).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowOrder {
    TopDown,
    BottomUp,
}

/// The rows of a header written for an exported image: the FITS display
/// convention. To prove on real exports.
pub const HEADER_ROWS: RowOrder = RowOrder::BottomUp;
/// ASTAP turns a TIFF, JPEG or PNG the FITS way up when it reads it. To
/// prove on real images.
pub const ASTAP_ROWS: RowOrder = RowOrder::BottomUp;
/// astrometry.net converts a raster image row by row from its top
/// (`an-pnmtofits`): FITS row 1 is the top row.
pub const ASTROMETRY_NET_ROWS: RowOrder = RowOrder::TopDown;

fn round_to(value: f64, decimals: i32) -> f64 {
    let factor = 10_f64.powi(decimals);
    (value * factor).round() / factor
}

/// The contract's fields from a solved `TAN` header over an image of `size`
/// (pixels with `x` to the right and `y` downward; the centre is the
/// image's centre; `rotationDeg` the position angle of the image's up
/// direction from north towards east; `parity` normal when east is to the
/// left of north). `None` without a usable `TAN` solution.
#[must_use]
pub fn solution_from_header(
    header: &FitsHeader,
    size: ImageSize,
    rows: RowOrder,
    source: SolutionSource,
    solver_version: Option<String>,
    solved_at: &str,
) -> Option<SkySolution> {
    let wcs = TanWcs::from_header(header)?;
    let (width, height) = (f64::from(size.width), f64::from(size.height));
    // The centre of the image, in FITS pixel coordinates (pixel 1 spans
    // 0.5..1.5): the same in both row orders.
    let (ra, dec) = wcs.pixel_to_world(width / 2.0 + 0.5, height / 2.0 + 0.5);
    let [[a, b], [c, d]] = wcs.cd;
    // The image's up direction (towards row 0), in FITS pixels.
    let up_dy = match rows {
        RowOrder::TopDown => -1.0,
        RowOrder::BottomUp => 1.0,
    };
    let (xi, eta) = (b * up_dy, d * up_dy);
    let rotation = xi.atan2(eta).to_degrees().rem_euclid(360.0);
    let determinant = a * d - b * c;
    let normal = match rows {
        RowOrder::TopDown => determinant > 0.0,
        RowOrder::BottomUp => determinant < 0.0,
    };
    let scale = determinant.abs().sqrt() * 3600.0;
    let rotation = round_to(rotation, 4);
    Some(SkySolution {
        schema_version: 1,
        source,
        solver_version: solver_version
            .map(|v| v.chars().take(80).collect::<String>())
            .filter(|v| !v.trim().is_empty()),
        ra_deg: round_to(ra, 6).rem_euclid(360.0),
        dec_deg: round_to(dec, 6).clamp(-90.0, 90.0),
        pixel_scale_arcsec: round_to(scale, 5),
        rotation_deg: if rotation >= 360.0 { 0.0 } else { rotation },
        field_width_deg: round_to(width * scale / 3600.0, 5),
        field_height_deg: round_to(height * scale / 3600.0, 5),
        parity: if normal {
            Parity::Normal
        } else {
            Parity::Mirrored
        },
        width: size.width,
        height: size.height,
        solved_at: solved_at.to_string(),
    })
}

/// A header written as text: 80-column cards one after the other (a `.wcs`
/// file, a TIFF description) or one card per line, with or without `END`.
#[must_use]
pub fn parse_header_text(text: &str) -> FitsHeader {
    let lines: Vec<String> = if text.contains('\n') {
        text.lines().map(str::to_string).collect()
    } else {
        text.as_bytes()
            .chunks(80)
            .map(|chunk| String::from_utf8_lossy(chunk).into_owned())
            .collect()
    };
    let mut bytes = Vec::new();
    for line in lines {
        let line = line.trim_end_matches('\r');
        if line.trim_end() == "END" {
            break;
        }
        if !line.is_ascii() {
            continue;
        }
        let mut card = line.as_bytes().to_vec();
        card.resize(80, b' ');
        card.truncate(80);
        bytes.extend_from_slice(&card);
    }
    let mut end = b"END".to_vec();
    end.resize(80, b' ');
    bytes.extend_from_slice(&end);
    parse_primary_header(&bytes).unwrap_or_default()
}

/// ASTAP's `.ini` answer (`KEY=value` lines) as header cards; the
/// projection is ASTAP's `TAN`.
fn ini_header(text: &str) -> FitsHeader {
    let mut cards: Vec<FitsCard> = text
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| FitsCard {
            keyword: key.trim().to_ascii_uppercase(),
            value: Some(value.trim().trim_matches('\'').trim().to_string()),
            comment: None,
        })
        .collect();
    for (keyword, value) in [("CTYPE1", "RA---TAN"), ("CTYPE2", "DEC--TAN")] {
        if !cards.iter().any(|c| c.keyword == keyword) {
            cards.push(FitsCard {
                keyword: keyword.to_string(),
                value: Some(value.to_string()),
                comment: None,
            });
        }
    }
    FitsHeader { cards }
}

fn header_matches_size(header: &FitsHeader, size: ImageSize) -> bool {
    matches!(
        (header.get_i64("NAXIS1"), header.get_i64("NAXIS2")),
        (Some(w), Some(h)) if w == i64::from(size.width) && h == i64::from(size.height)
    )
}

fn header_solver(header: &FitsHeader) -> Option<String> {
    ["SWCREATE", "CREATOR", "PROGRAM"]
        .iter()
        .find_map(|key| header.get(key).map(str::to_string))
}

/// The solution already written for `image` (see the module
/// documentation), without any computation; `None` when there is none, or
/// when it was written for another size of the image (a crop, a reduction).
#[must_use]
pub fn header_solution(image: &Path, solved_at: &str) -> Option<SkySolution> {
    let size = image_size(image)?;
    let mut headers = Vec::new();
    if let Some(text) = tiff_description(image) {
        headers.push(parse_header_text(&text));
    }
    let sidecar = image.with_extension("wcs");
    if let Ok(bytes) = std::fs::read(&sidecar) {
        headers.push(parse_header_text(&String::from_utf8_lossy(&bytes)));
    }
    headers.into_iter().find_map(|header| {
        if !header_matches_size(&header, size) || header_cd_matrix(&header).is_none() {
            return None;
        }
        let solver = header_solver(&header);
        solution_from_header(
            &header,
            size,
            HEADER_ROWS,
            SolutionSource::Header,
            solver,
            solved_at,
        )
    })
}

/* ------------------------------- The solvers ----------------------------- */

/// Why no solution came out: said in words, never as a code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "code",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum SolveFailure {
    /// No solver is installed ("Settings › Dependencies").
    SolverMissing,
    /// The image is not a FITS, TIFF, JPEG or PNG the solver can read.
    UnreadableImage,
    TooFewStars,
    /// The stars were found but matched no field of the star catalogue.
    NotSolved,
    /// ASTAP says its installed star database does not suit a field of this
    /// size (« use G05 database », « Wide field image », « Database limit
    /// for this FOV »): another database is needed ("Settings ›
    /// Dependencies").
    OutsideDatabase,
    /// ASTAP's star database is missing.
    NoDatabase,
    /// astrometry.net is here and has no index file to match the stars
    /// against (WEB.17b.5): "Settings › Dependencies" says where to put them.
    NoIndex,
    Timeout {
        seconds: u64,
    },
    Cancelled,
    /// Any other end, with the solver's last words for the details (ⓘ).
    Failed {
        detail: String,
    },
}

impl SolveFailure {
    /// The words of the "Annotations" row (English; the window's
    /// dictionaries carry the ten languages).
    #[must_use]
    pub fn sentence(&self) -> String {
        match self {
            SolveFailure::SolverMissing => "No solver is installed: sky solutions cannot be computed here.".to_string(),
            SolveFailure::UnreadableImage => "The solver cannot read this image.".to_string(),
            SolveFailure::TooFewStars => "Too few stars recognised in the image.".to_string(),
            SolveFailure::NotSolved => "The stars of the image match no field of the catalogue.".to_string(),
            SolveFailure::OutsideDatabase => "Field outside the installed indexes: ASTAP’s star database does not cover a field of this size.".to_string(),
            SolveFailure::NoDatabase => "ASTAP’s star database cannot be found.".to_string(),
            SolveFailure::NoIndex => "astrometry.net has no index files: install them in Settings › Dependencies.".to_string(),
            SolveFailure::Timeout { seconds } => {
                format!("No solution after {seconds} s: solving was stopped.")
            }
            SolveFailure::Cancelled => "Solving stopped.".to_string(),
            SolveFailure::Failed { .. } => "The solver stopped without a solution.".to_string(),
        }
    }

    /// The stable code the window looks up.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            SolveFailure::SolverMissing => "solver_missing",
            SolveFailure::UnreadableImage => "unreadable_image",
            SolveFailure::TooFewStars => "too_few_stars",
            SolveFailure::NotSolved => "not_solved",
            SolveFailure::OutsideDatabase => "outside_database",
            SolveFailure::NoDatabase => "no_star_database",
            SolveFailure::NoIndex => "no_index_files",
            SolveFailure::Timeout { .. } => "solve_timeout",
            SolveFailure::Cancelled => "cancelled",
            SolveFailure::Failed { .. } => "solve_failed",
        }
    }
}

/// What helps a solver: the target's position and the image's scale.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SolveHints {
    pub ra_deg: Option<f64>,
    pub dec_deg: Option<f64>,
    pub pixel_scale_arcsec: Option<f64>,
}

/// A solver found by [`crate::tools`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SolverTool {
    pub path: PathBuf,
    pub version: Option<String>,
    /// ASTAP's star database folder, when it is not beside ASTAP.
    #[serde(default)]
    pub database: Option<PathBuf>,
}

/// One resolution: the image, its size, the hints, where the solver may
/// write (Astro Sync's data), how long it may take.
#[derive(Debug, Clone)]
pub struct SolveJob<'a> {
    pub image: &'a Path,
    pub hints: SolveHints,
    pub work_dir: &'a Path,
    pub timeout: Duration,
    pub cancel: Option<&'a CancelFlag>,
    pub solved_at: &'a str,
}

/// The default time a solver gets for one image.
pub const DEFAULT_SOLVE_TIMEOUT: Duration = Duration::from_secs(180);

fn prepare_work_dir(work_dir: &Path, base: &str) -> Result<PathBuf, SolveFailure> {
    std::fs::create_dir_all(work_dir).map_err(|error| SolveFailure::Failed {
        detail: error.to_string(),
    })?;
    let output = work_dir.join(base);
    for extension in ["wcs", "ini", "log", "solved", "axy"] {
        let _ = std::fs::remove_file(output.with_extension(extension));
    }
    Ok(output)
}

fn ended(end: &external_process::ProcessEnd, timeout: Duration) -> Option<SolveFailure> {
    if end.cancelled {
        return Some(SolveFailure::Cancelled);
    }
    if end.timed_out {
        return Some(SolveFailure::Timeout {
            seconds: timeout.as_secs(),
        });
    }
    None
}

fn last_words(end: &external_process::ProcessEnd) -> String {
    let text = format!("{}\n{}", end.stdout, end.stderr);
    text.lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default()
        .chars()
        .take(300)
        .collect()
}

/// ASTAP's command line on `image` (the file written for it, or the image
/// itself), writing its answer and its log to `output` (`<output>.wcs`,
/// `<output>.ini`, `<output>.log`), never next to the image. The field is
/// the picture's height times the scale when the scale is known; with a
/// position and no scale ASTAP searches the field's size itself (`-fov 0`).
#[must_use]
pub fn astap_args(
    image: &Path,
    hints: &SolveHints,
    size: ImageSize,
    output: &Path,
    database: Option<&Path>,
) -> Vec<String> {
    let mut args = vec![
        "-f".to_string(),
        image.display().to_string(),
        "-o".to_string(),
        output.display().to_string(),
        "-z".to_string(),
        "0".to_string(),
        "-log".to_string(),
    ];
    let positioned = matches!((hints.ra_deg, hints.dec_deg), (Some(_), Some(_)));
    match (hints.ra_deg, hints.dec_deg) {
        (Some(ra), Some(dec)) => {
            args.extend([
                "-ra".to_string(),
                format!("{:.6}", ra / 15.0),
                "-spd".to_string(),
                format!("{:.6}", dec + 90.0),
                "-r".to_string(),
                "30".to_string(),
            ]);
        }
        _ => args.extend(["-r".to_string(), "180".to_string()]),
    }
    if let Some(scale) = hints.pixel_scale_arcsec {
        args.extend([
            "-fov".to_string(),
            format!("{:.4}", scale * f64::from(size.height) / 3600.0),
        ]);
    } else if positioned {
        args.extend(["-fov".to_string(), "0".to_string()]);
    }
    if let Some(database) = database {
        args.extend(["-d".to_string(), database.display().to_string()]);
    }
    args
}

/// Whether ASTAP's answer or log says its star database does not suit the
/// field (too wide for it, or too shallow for so small a field).
fn astap_database_does_not_cover(output: &Path) -> bool {
    let text = ["ini", "log"]
        .iter()
        .filter_map(|extension| std::fs::read_to_string(output.with_extension(extension)).ok())
        .collect::<Vec<_>>()
        .join("\n")
        .to_ascii_lowercase();
    text.contains("use g05 database")
        || text.contains("use w08 database")
        || text.contains("wide field image")
        || text.contains("could not find w08")
        || text.contains("database limit for this fov")
}

/// The failure ASTAP's `ERROR=` line names, if any.
fn astap_error(output: &Path) -> Option<SolveFailure> {
    let ini = std::fs::read_to_string(output.with_extension("ini")).ok()?;
    let error = ini
        .lines()
        .find_map(|line| line.strip_prefix("ERROR="))?
        .to_ascii_lowercase();
    Some(if error.contains("star database") {
        SolveFailure::NoDatabase
    } else if error.contains("reading image") || error.contains("unsupported") {
        SolveFailure::UnreadableImage
    } else {
        SolveFailure::Failed {
            detail: error.trim().chars().take(300).collect(),
        }
    })
}

/// One run of ASTAP on `image` (exit codes: 0 solved, 1 no solution, 2 too
/// few stars, 16 unreadable image, 32/33 star database missing or
/// unreadable), within `deadline`.
fn run_astap(
    tool: &SolverTool,
    job: &SolveJob<'_>,
    image: &Path,
    hints: &SolveHints,
    size: ImageSize,
    deadline: Instant,
) -> Result<SkySolution, SolveFailure> {
    let output = prepare_work_dir(job.work_dir, "astap")?;
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(SolveFailure::Timeout {
            seconds: job.timeout.as_secs(),
        });
    }
    let mut command = Command::new(&tool.path);
    command.args(astap_args(
        image,
        hints,
        size,
        &output,
        tool.database.as_deref(),
    ));
    let end = external_process::run(command, remaining, job.cancel)
        .map_err(|_| SolveFailure::SolverMissing)?;
    if let Some(failure) = ended(&end, job.timeout) {
        return Err(failure);
    }
    // ASTAP ends with 1 (« no solution ») when it could not even read the
    // image (a Deflate TIFF: « Error reading image file. »): its `ERROR=`
    // line says which.
    let unsolved = || {
        if let Some(failure) = astap_error(&output) {
            failure
        } else if astap_database_does_not_cover(&output) {
            SolveFailure::OutsideDatabase
        } else {
            SolveFailure::NotSolved
        }
    };
    match end.code() {
        Some(0) => {}
        Some(1) => return Err(unsolved()),
        Some(2) => return Err(SolveFailure::TooFewStars),
        Some(16) => return Err(SolveFailure::UnreadableImage),
        Some(32 | 33) => return Err(SolveFailure::NoDatabase),
        _ => {
            return Err(SolveFailure::Failed {
                detail: last_words(&end),
            });
        }
    }
    let version = tool
        .version
        .as_ref()
        .map(|version| format!("ASTAP {version}"));
    let from_wcs = std::fs::read(output.with_extension("wcs"))
        .ok()
        .map(|bytes| parse_header_text(&String::from_utf8_lossy(&bytes)));
    let from_ini = std::fs::read_to_string(output.with_extension("ini"))
        .ok()
        .map(|text| ini_header(&text));
    if from_ini
        .as_ref()
        .and_then(|ini| ini.get("PLTSOLVD"))
        .is_some_and(|solved| solved.trim().eq_ignore_ascii_case("F"))
    {
        return Err(unsolved());
    }
    [from_wcs, from_ini]
        .into_iter()
        .flatten()
        .find_map(|header| {
            solution_from_header(
                &header,
                size,
                ASTAP_ROWS,
                SolutionSource::Astap,
                version.clone(),
                job.solved_at,
            )
        })
        .ok_or(SolveFailure::NotSolved)
}

/// ASTAP on `job.image` ([`input`]): the image is written as an uncompressed
/// 16-bit mono FITS with the position and the scale, levelled first, plain
/// when the levelled one finds no field; an image this module does not decode
/// goes as it is. The scale is the hints', else the image's header's, else
/// that of a FITS of the same size beside it.
pub fn solve_with_astap(
    tool: &SolverTool,
    job: &SolveJob<'_>,
) -> Result<SkySolution, SolveFailure> {
    let size = image_size(job.image).ok_or(SolveFailure::UnreadableImage)?;
    let deadline = Instant::now() + job.timeout;
    let decoded = input::read_luminance(job.image)
        .ok()
        .filter(|(luminance, _)| {
            (luminance.width, luminance.height) == (size.width as usize, size.height as usize)
        });
    let mut hints = job.hints;
    if hints.pixel_scale_arcsec.is_none() {
        let header = decoded.as_ref().and_then(|(_, header)| header.as_ref());
        hints.pixel_scale_arcsec = input::scale_hint(job.image, header, size);
    }
    let Some((luminance, _)) = decoded else {
        return run_astap(tool, job, job.image, &hints, size, deadline);
    };
    let cards = input::InputCards {
        ra_deg: hints.ra_deg,
        dec_deg: hints.dec_deg,
        pixel_scale_arcsec: hints.pixel_scale_arcsec,
    };
    std::fs::create_dir_all(job.work_dir).map_err(|error| SolveFailure::Failed {
        detail: error.to_string(),
    })?;
    let mut unsolved: Option<SolveFailure> = None;
    for preparation in [input::Preparation::Levelled, input::Preparation::Plain] {
        let written =
            input::write_input(&luminance, job.work_dir, preparation, &cards).map_err(|error| {
                SolveFailure::Failed {
                    detail: error.to_string(),
                }
            })?;
        let attempt = run_astap(tool, job, &written.path, &hints, size, deadline);
        let _ = std::fs::remove_file(&written.path);
        match attempt {
            Ok(solution) => return Ok(solution),
            // Stars that match nothing say more than too few stars.
            Err(failure @ (SolveFailure::NotSolved | SolveFailure::TooFewStars)) => {
                if unsolved != Some(SolveFailure::NotSolved) {
                    unsolved = Some(failure);
                }
            }
            Err(other) => return Err(other),
        }
    }
    Err(unsolved.unwrap_or(SolveFailure::NotSolved))
}

/// `solve-field`'s command line for `job`: every product in the work
/// folder, none but the `.wcs` and the `.solved` mark kept.
#[must_use]
pub fn astrometry_net_args(job: &SolveJob<'_>) -> Vec<String> {
    let mut args: Vec<String> = [
        "--overwrite",
        "--no-plots",
        "--no-remove-lines",
        "--dir",
        &job.work_dir.display().to_string(),
        "--out",
        "astrometry",
        "--new-fits",
        "none",
        "--corr",
        "none",
        "--match",
        "none",
        "--rdls",
        "none",
        "--index-xyls",
        "none",
        "--temp-axy",
        "--cpulimit",
        &job.timeout.as_secs().max(1).to_string(),
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect();
    if let Some(scale) = job.hints.pixel_scale_arcsec {
        args.extend([
            "--scale-units".to_string(),
            "arcsecperpix".to_string(),
            "--scale-low".to_string(),
            format!("{:.4}", scale * 0.8),
            "--scale-high".to_string(),
            format!("{:.4}", scale * 1.2),
        ]);
    }
    if let (Some(ra), Some(dec)) = (job.hints.ra_deg, job.hints.dec_deg) {
        args.extend([
            "--ra".to_string(),
            format!("{ra:.6}"),
            "--dec".to_string(),
            format!("{dec:.6}"),
            "--radius".to_string(),
            "10".to_string(),
        ]);
    }
    args.push(job.image.display().to_string());
    args
}

/// Whether `solve-field` stopped for want of index files: its engine says
/// « You must list at least one index in the config file » when the
/// configuration lists none, and warns « failed to open index directory »
/// when the folder it names is not there.
fn no_index_files(end: &external_process::ProcessEnd) -> bool {
    let text = format!("{}\n{}", end.stdout, end.stderr).to_ascii_lowercase();
    text.contains("list at least one index")
        || text.contains("failed to open index directory")
        || text.contains("no index files")
}

/// astrometry.net's `solve-field` on `job.image` (the person chose it; its
/// indices are theirs to install).
pub fn solve_with_astrometry_net(
    tool: &SolverTool,
    job: &SolveJob<'_>,
) -> Result<SkySolution, SolveFailure> {
    let size = image_size(job.image).ok_or(SolveFailure::UnreadableImage)?;
    let output = prepare_work_dir(job.work_dir, "astrometry")?;
    let mut command = Command::new(&tool.path);
    command.args(astrometry_net_args(job));
    // `--cpulimit` is the solver's own limit; the run gets a margin over it.
    let limit = job.timeout + Duration::from_secs(30);
    let end = external_process::run(command, limit, job.cancel)
        .map_err(|_| SolveFailure::SolverMissing)?;
    if let Some(failure) = ended(&end, job.timeout) {
        return Err(failure);
    }
    let wcs = std::fs::read(output.with_extension("wcs")).ok();
    let Some(wcs) = wcs else {
        return Err(if end.succeeded() {
            SolveFailure::NotSolved
        } else if no_index_files(&end) {
            SolveFailure::NoIndex
        } else {
            SolveFailure::Failed {
                detail: last_words(&end),
            }
        });
    };
    let version = tool
        .version
        .as_ref()
        .map(|version| format!("astrometry.net {version}"));
    // A FITS pose keeps its own row order (row 1 at the bottom); astrometry.net
    // numbers the rows of a raster image from its top.
    let rows = if is_fits_file(job.image) {
        RowOrder::BottomUp
    } else {
        ASTROMETRY_NET_ROWS
    };
    solution_from_header(
        &parse_header_text(&String::from_utf8_lossy(&wcs)),
        size,
        rows,
        SolutionSource::AstrometryNet,
        version,
        job.solved_at,
    )
    .ok_or(SolveFailure::NotSolved)
}

/// The solver the person uses (Settings; ASTAP by default).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SolverChoice {
    #[default]
    Astap,
    AstrometryNet,
}

/// The solvers present on this computer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Solvers {
    pub astap: Option<SolverTool>,
    pub astrometry_net: Option<SolverTool>,
}

/// The whole resolution of one image: its header first, then the solver
/// chosen (ASTAP when astrometry.net is chosen but missing, and the other
/// way round).
pub fn solve_image(
    solvers: &Solvers,
    choice: SolverChoice,
    job: &SolveJob<'_>,
) -> Result<SkySolution, SolveFailure> {
    if let Some(solution) = header_solution(job.image, job.solved_at) {
        return Ok(solution);
    }
    let astap = solvers.astap.as_ref().map(|tool| {
        (
            tool,
            solve_with_astap as fn(&SolverTool, &SolveJob<'_>) -> _,
        )
    });
    let astrometry = solvers.astrometry_net.as_ref().map(|tool| {
        (
            tool,
            solve_with_astrometry_net as fn(&SolverTool, &SolveJob<'_>) -> _,
        )
    });
    let chosen = match choice {
        SolverChoice::Astap => astap.or(astrometry),
        SolverChoice::AstrometryNet => astrometry.or(astap),
    };
    let (tool, solve) = chosen.ok_or(SolveFailure::SolverMissing)?;
    solve(tool, job)
}

/* ------------------------------- The service ----------------------------- */

/// `PUT /v1/captures/{id}/sky-solution`'s body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkySolutionPut {
    pub sha256: String,
    pub solution: SkySolution,
}

/// Its answer (`written`): the solution stored, as the service says it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WrittenSkySolution {
    pub sky_solution: serde_json::Value,
    pub created: bool,
    pub changed: bool,
}

/// A target of a listed capture (`capture.schema.json`, `listedTarget`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListedTarget {
    pub id: String,
    #[serde(default)]
    pub designation: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub ra_hours: Option<f64>,
    #[serde(default)]
    pub dec_degrees: Option<f64>,
}

/// A capture whose main image has no sky solution (`GET
/// /v1/captures?skySolution=missing`), from the phone, the web or a night.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MissingCapture {
    pub id: String,
    #[serde(default)]
    pub session_id: Option<String>,
    pub original_name: String,
    pub sha256: String,
    pub byte_size: u64,
    #[serde(default)]
    pub main_image_id: Option<String>,
    /// `phone`, `web`, `astro-sync`, `stacking`.
    #[serde(default)]
    pub main_image_source: Option<String>,
    #[serde(default)]
    pub night_date: Option<String>,
    #[serde(default)]
    pub gallery_label: Option<String>,
    #[serde(default)]
    pub thumbnail: Option<bool>,
    #[serde(default)]
    pub targets: Vec<ListedTarget>,
}

impl MissingCapture {
    /// The hints its first positioned target gives.
    #[must_use]
    pub fn hints(&self) -> SolveHints {
        self.targets
            .iter()
            .find_map(|t| Some((t.ra_hours? * 15.0, t.dec_degrees?)))
            .map(|(ra, dec)| SolveHints {
                ra_deg: Some(ra),
                dec_deg: Some(dec),
                pixel_scale_arcsec: None,
            })
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MissingPage {
    captures: Vec<MissingCapture>,
    #[serde(default)]
    next_since: Option<String>,
    #[serde(default)]
    has_more: Option<bool>,
}

/// The two routes, behind a trait so tests never touch the network.
pub trait SkySolutionTransport {
    fn put_sky_solution(
        &self,
        base_url: &str,
        token: &str,
        capture_id: &str,
        body: &SkySolutionPut,
    ) -> Result<WrittenSkySolution, GuiBridgeError>;

    /// One page of `GET /v1/captures?skySolution=missing`, from `since`.
    fn missing_page(
        &self,
        base_url: &str,
        token: &str,
        since: Option<&str>,
    ) -> Result<serde_json::Value, GuiBridgeError>;
}

impl SkySolutionTransport for HttpPortalGuiTransport {
    /// Idempotent by digest (`api/v1.md`): repeated freely while the service
    /// wakes.
    fn put_sky_solution(
        &self,
        base_url: &str,
        token: &str,
        capture_id: &str,
        body: &SkySolutionPut,
    ) -> Result<WrittenSkySolution, GuiBridgeError> {
        let url = format!(
            "{}/api/v1/captures/{capture_id}/sky-solution",
            base_url.trim_end_matches('/')
        );
        let token = token.to_owned();
        let body = body.clone();
        let build: RequestFactory =
            Arc::new(move |client| client.put(url.clone()).bearer_auth(&token).json(&body));
        self.read_json(&build)
    }

    fn missing_page(
        &self,
        base_url: &str,
        token: &str,
        since: Option<&str>,
    ) -> Result<serde_json::Value, GuiBridgeError> {
        let url = format!("{}/api/v1/captures", base_url.trim_end_matches('/'));
        let token = token.to_owned();
        let mut query = vec![
            ("skySolution", "missing".to_string()),
            ("limit", "100".to_string()),
        ];
        if let Some(since) = since {
            query.push(("since", since.to_string()));
        }
        let build: RequestFactory =
            Arc::new(move |client| client.get(url.clone()).bearer_auth(&token).query(&query));
        self.read_json(&build)
    }
}

/// Every capture without a sky solution, page after page.
pub fn list_missing<T: SkySolutionTransport>(
    transport: &T,
    base_url: &str,
    token: &str,
) -> Result<Vec<MissingCapture>, GuiBridgeError> {
    let mut captures = Vec::new();
    let mut since: Option<String> = None;
    // A bound on the pages: a service that always says « more » never holds
    // the window.
    for _ in 0..100 {
        let page: MissingPage =
            serde_json::from_value(transport.missing_page(base_url, token, since.as_deref())?)
                .map_err(|error| GuiBridgeError::InvalidResponse(error.to_string()))?;
        captures.extend(page.captures);
        match (page.has_more, page.next_since) {
            (Some(true), Some(next)) if since.as_deref() != Some(next.as_str()) => {
                since = Some(next);
            }
            _ => break,
        }
    }
    Ok(captures)
}

/// Sends the solution of the image `sha256` of capture `capture_id`.
pub fn send_solution<T: SkySolutionTransport>(
    transport: &T,
    base_url: &str,
    token: &str,
    capture_id: &str,
    sha256: &str,
    solution: &SkySolution,
) -> Result<WrittenSkySolution, GuiBridgeError> {
    transport.put_sky_solution(
        base_url,
        token,
        capture_id,
        &SkySolutionPut {
            sha256: sha256.to_ascii_lowercase(),
            solution: solution.clone(),
        },
    )
}

/* ------------------------- One capture, end to end ----------------------- */

impl MissingCapture {
    /// Where its main image is solved from: the file of a night's folder
    /// with the same digest (`here`, by SHA-256), else its original, fetched
    /// from the space.
    #[must_use]
    pub fn solve_from(&self, here: &std::collections::BTreeMap<String, PathBuf>) -> SolveFrom {
        let image_id = self.main_image_id.clone().unwrap_or_default();
        match here.get(&self.sha256.to_ascii_lowercase()) {
            Some(path) => SolveFrom::Local {
                capture_id: self.id.clone(),
                image_id,
                sha256: self.sha256.clone(),
                path: path.clone(),
            },
            None => SolveFrom::Original {
                capture_id: self.id.clone(),
                image_id,
                sha256: self.sha256.clone(),
                original_name: self.original_name.clone(),
                byte_size: self.byte_size,
            },
        }
    }
}

/// What became of one image.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(
    tag = "outcome",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum SolveOutcome {
    /// Solved and stored by the service.
    Sent {
        capture_id: String,
        solution: SkySolution,
        written: WrittenSkySolution,
    },
    /// No solution: the row says why, and can be tried again.
    Failed {
        capture_id: String,
        failure: SolveFailure,
        message: String,
    },
}

/// Where the work of one resolution goes.
#[derive(Debug, Clone)]
pub struct SolveSetup<'a> {
    pub solvers: &'a Solvers,
    pub choice: SolverChoice,
    /// The originals fetched (`<data>/originals`).
    pub originals_dir: &'a Path,
    /// The solvers' work when the image is not a file of a night: the
    /// night's `.astro-sync-work/solve/` when the caller knows the night,
    /// else `<data>/solve`.
    pub work_root: &'a Path,
    pub timeout: Duration,
    pub cancel: Option<&'a CancelFlag>,
    pub now: &'a str,
}

/// Solves the image `source` names (fetching its original when it is not
/// here) and sends the solution with that image's digest. A network failure
/// is an error; a resolution that finds nothing is [`SolveOutcome::Failed`].
pub fn solve_and_send<T: SkySolutionTransport + OriginalTransport>(
    transport: &T,
    base_url: &str,
    token: &str,
    source: &SolveFrom,
    hints: SolveHints,
    setup: &SolveSetup<'_>,
) -> Result<SolveOutcome, GuiBridgeError> {
    let (capture_id, sha256) = match source {
        SolveFrom::Local {
            capture_id, sha256, ..
        }
        | SolveFrom::Original {
            capture_id, sha256, ..
        } => (capture_id.clone(), sha256.clone()),
    };
    let image = fetch_original(transport, base_url, token, source, setup.originals_dir)?;
    let root = solve_work_root(source, setup);
    // One attempt on a file: an image of the working folder points where its
    // master says (a very wide field cannot be solved blind): that first, the
    // target's position where the master is silent.
    let attempt = |file: &Path, folder: &str| {
        let hints = match source {
            SolveFrom::Local { .. } => hints_of_image(file).completed_by(hints),
            SolveFrom::Original { .. } => hints,
        };
        let work = root.join(folder);
        let job = SolveJob {
            image: file,
            hints,
            work_dir: &work,
            timeout: setup.timeout,
            cancel: setup.cancel,
            solved_at: setup.now,
        };
        let solved = solve_image(setup.solvers, setup.choice, &job);
        // The work of this resolution ends here, whatever came of it.
        remove_solve_work(&work);
        solved
    };
    let folder = sha256.to_ascii_lowercase();
    // A finish (contrast, colours, softened stars) changes the pixels, never
    // the geometry: its twin without it, of the same size, says the same sky.
    // The solvers fail on stars the finish took out, so the twin's solution
    // is the one the finished image takes (same WCS, sent with its digest).
    let twin = match source {
        SolveFrom::Local { path, .. } => finish_twin(path),
        SolveFrom::Original { .. } => None,
    };
    let solved = match &twin {
        Some(twin) => attempt(twin, &format!("{folder}-twin")).or_else(|failure| {
            if matches!(failure, SolveFailure::Cancelled) {
                Err(failure)
            } else {
                attempt(&image, &folder)
            }
        }),
        None => attempt(&image, &folder),
    };
    match solved {
        Ok(solution) => {
            let written =
                send_solution(transport, base_url, token, &capture_id, &sha256, &solution)?;
            Ok(SolveOutcome::Sent {
                capture_id,
                solution,
                written,
            })
        }
        Err(failure) => Ok(SolveOutcome::Failed {
            capture_id,
            message: failure.sentence(),
            failure,
        }),
    }
}

/// The same image without its contrast, colours and softened stars -- the file
/// of the same name minus `_strong`, `_colour-40`, `_stars-85`... beside it --,
/// when `image` carries a finish and that file is there with the same size in
/// pixels. `None` for an image without a finish, or whose twin is not here.
#[must_use]
pub fn finish_twin(image: &Path) -> Option<PathBuf> {
    let stem = image.file_stem()?.to_str()?;
    let extension = image.extension()?;
    let mut rest = stem;
    if let Some((head, value)) = rest.rsplit_once("_stars-")
        && value.parse::<crate::stacking::SoftStars>().is_ok()
    {
        rest = head;
    }
    if let Some((head, value)) = rest.rsplit_once("_colour-")
        && value.parse::<crate::stacking::Colour>().is_ok()
    {
        rest = head;
    }
    if let Some((head, value)) = rest.rsplit_once("_contrast-")
        && value.parse::<crate::stacking::Contrast>().is_ok()
    {
        rest = head;
    } else if let Some(head) = rest
        .strip_suffix("_strong")
        .or_else(|| rest.strip_suffix("_presentation"))
    {
        rest = head;
    }
    if rest.len() == stem.len() || rest.is_empty() {
        return None;
    }
    let twin = image.with_file_name(format!("{rest}.{}", extension.to_str()?));
    (twin != image && twin.is_file() && image_size(&twin)? == image_size(image)?).then_some(twin)
}

/// Where one resolution works: on the disk of the night the image lies in
/// (`<night>/.astro-sync-work/solve/`), else [`SolveSetup::work_root`].
#[must_use]
pub fn solve_work_root(source: &SolveFrom, setup: &SolveSetup<'_>) -> PathBuf {
    match source {
        SolveFrom::Local { path, .. } => {
            crate::working_folder::night_folder_of(path).map(|n| solve_scratch_root(&n))
        }
        SolveFrom::Original { .. } => None,
    }
    .unwrap_or_else(|| setup.work_root.to_path_buf())
}

/// Removes the work folder of one resolution: with its empty parents up to
/// `.astro-sync-work/` on a night's disk, alone in Astro Sync's data.
fn remove_solve_work(work: &Path) {
    if work
        .components()
        .any(|part| part.as_os_str() == crate::working_folder::WORK_DIR)
    {
        crate::working_folder::remove_scratch(work);
    } else {
        let _ = std::fs::remove_dir_all(work);
    }
}

#[cfg(test)]
mod tests;
