//! Step 1: the three locations read, headers only (never a pixel), one
//! record per physical file.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::analysis::fits;
use crate::analysis::sky_night::{header_site, parse_angle};
use crate::catalogue_targets::header_target;
use crate::detection::{self, FrameType, LogKind};
use crate::device_master::{self, DeviceImageInfo};
use crate::local_work::{WorkKind, WorkProgress};
use crate::scanner::{FileClass, LinkIssue, ScanError, WalkedFile, walk_folder_reporting};
use crate::time_util::parse_iso8601_utc_to_unix_seconds;

use super::band::{self, BandGuess};
use super::{DetectError, DetectOptions, DetectRequest, FoundSoFar};

/// Which of the three locations a file was met in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Location {
    Fits,
    Calibration,
    Logs,
}

/// What one frame's first header says.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FrameRecord {
    /// The path to open (as walked, possibly through a link).
    pub path: PathBuf,
    /// The folders between the location's root (its name included) and the
    /// file: what the folder's name may say (`observation_M65`, `dark`).
    pub folders: Vec<String>,
    pub in_calibration_location: bool,
    pub kind: Option<FrameType>,
    pub kind_from_folder: bool,
    pub stacked: bool,
    /// The image a known device stacked: a stacked
    /// image all the same (never a pose), but named and offered as the master
    /// of its target instead of being set aside.
    pub device: Option<DeviceImageInfo>,
    pub object: Option<String>,
    pub ra_degrees: Option<f64>,
    pub dec_degrees: Option<f64>,
    pub field_degrees: [f64; 2],
    pub date_obs: Option<f64>,
    /// `date_obs` is the camera clock as written, its time zone unknown (a
    /// RAW file whose metadata names none): the grouping brings it to UTC
    /// with the place of the night ([`super::group`]).
    pub date_zone_unknown: bool,
    pub exposure_seconds: Option<f64>,
    pub filter: Option<String>,
    pub gain: Option<f64>,
    pub temperature_c: Option<f64>,
    pub site: Option<(f64, f64)>,
    pub telescop: Option<String>,
    pub instrume: Option<String>,
    /// `CREATOR`: the software or device that wrote the file (a Seestar
    /// writes its own name there).
    pub creator: Option<String>,
    pub focal_length_mm: Option<f64>,
    pub altitude_deg: Option<f64>,
    pub azimuth_deg: Option<f64>,
    pub lst_hours: Option<f64>,
    pub bayer: Option<String>,
    /// The band the names around a light say, read only for a light whose
    /// header names no filter.
    pub band_guess: Option<BandGuess>,
}

impl FrameRecord {
    pub fn display(&self) -> String {
        self.path.to_string_lossy().into_owned()
    }

    /// Whether a smart telescope took the frame, one that calibrates its
    /// own sub-frames on board (Seestar, Stellina, Dwarf, Vespera,
    /// eVscope…): nothing to attach, so the person is not told that darks
    /// or flats are missing.
    pub fn calibrates_itself(&self) -> bool {
        [&self.creator, &self.instrume, &self.telescop]
            .into_iter()
            .flatten()
            .any(|name| {
                let name = name.to_lowercase();
                ["seestar", "stellina", "dwarf", "vespera", "evscope"]
                    .iter()
                    .any(|device| name.contains(device))
            })
    }

    /// The instrument a multi-night group is kept to: the camera and the
    /// focal length.
    pub fn instrument(&self) -> String {
        let camera = self.instrume.as_deref().unwrap_or("?");
        match self.focal_length_mm {
            Some(focal) => format!("{camera} · {focal:.0} mm"),
            None => camera.to_string(),
        }
    }
}

/// A log met in one of the locations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogRecord {
    pub path: PathBuf,
    pub kind: LogKind,
}

/// Everything read.
#[derive(Debug, Clone, Default)]
pub struct Reading {
    pub frames: Vec<FrameRecord>,
    pub logs: Vec<LogRecord>,
    pub unreadable: Vec<String>,
    /// Stacked images whose header cannot be read (named so).
    pub stacks: Vec<String>,
    /// Text files no reader knows (ASIAIR's `_CHN` renditions among them).
    pub other_files: Vec<String>,
    pub links: Vec<LinkIssue>,
    pub files_read: usize,
    /// Videos (AVI, SER…) met in the folders: not processed, not read, so
    /// not counted in `files_read`.
    pub videos: Vec<String>,
    /// The text files a capture program wrote beside a video (ZWO's
    /// `<video>.avi.txt`): they follow their video.
    pub video_captures: Vec<String>,
    /// Camera RAW files (CR2, NEF, ARW, DNG, RAF, PEF, CR3…) whose metadata
    /// cannot be read (a variant, a damaged file): not read, said as such
    /// (a RAW file that can be read is a pose).
    pub raw_files: Vec<String>,
}

/// The type a folder's name says, the deepest first (`DARK`, `dark_flat`,
/// `flats`, `lights`); `None` when no folder says one.
pub fn folder_kind(folders: &[String]) -> Option<FrameType> {
    folders.iter().rev().find_map(|folder| {
        let word: String = folder
            .chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect();
        match word.as_str() {
            "dark" | "darks" => Some(FrameType::Dark),
            "flat" | "flats" => Some(FrameType::Flat),
            "darkflat" | "darkflats" | "flatdark" | "flatdarks" => Some(FrameType::DarkFlat),
            "bias" | "biases" | "offset" | "offsets" => Some(FrameType::Bias),
            "light" | "lights" => Some(FrameType::Light),
            _ => None,
        }
    })
}

/// A stacked image rather than a frame: a stacking key of the header, or a
/// name or folder that says so (the ASIAIR's `Stacked…` Live stacks).
fn is_stacked(header: &fits::FitsHeader, file_name: &str, folders: &[String]) -> bool {
    let counted = ["STACKCNT", "NCOMBINE", "STACKED", "NSTACK"]
        .iter()
        .any(|key| header.get_f64(key).is_some_and(|n| n > 1.0));
    counted || named_stacked(file_name, folders)
}

fn named_stacked(file_name: &str, folders: &[String]) -> bool {
    let lowered = file_name.to_lowercase();
    let named = lowered.starts_with("stacked") || lowered.starts_with("master");
    let in_folder = folders.iter().any(|f| {
        matches!(
            f.to_lowercase().as_str(),
            "stacks" | "stack" | "masters" | "master"
        )
    });
    named || in_folder
}

fn text(header: &fits::FitsHeader, key: &str) -> Option<String> {
    header
        .get(key)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

fn record_of(
    header: &fits::FitsHeader,
    file: &WalkedFile,
    folders: Vec<String>,
    location: Location,
) -> FrameRecord {
    let acquisition = fits::extract_acquisition_record(header);
    let target = header_target(header);
    let file_name = file
        .relative_path
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_string();
    let header_kind = header
        .get("IMAGETYP")
        .or_else(|| header.get("FRAME"))
        .or_else(|| header.get("FRAMETYP"))
        .and_then(detection::normalize_image_type);
    // In a calibration folder the folder is right: the ASIAIR writes `Light`
    // on darks taken in its « Light » mode. A light without any type in the
    // folder of FITS is a light (the Vespera writes none).
    let by_folder = folder_kind(&folders).filter(|kind| *kind != FrameType::Light);
    let (kind, kind_from_folder) = match (by_folder, header_kind) {
        (Some(folder), header) => (Some(folder), header != Some(folder)),
        (None, Some(header)) => (Some(header), false),
        (None, None) if location == Location::Fits => (Some(FrameType::Light), false),
        (None, None) => (None, false),
    };
    let stacked = is_stacked(header, &file_name, &folders);
    let device = if stacked && location == Location::Fits {
        device_master::recognise(header, &file_name)
    } else {
        None
    };
    FrameRecord {
        path: file.path.clone(),
        folders,
        in_calibration_location: location == Location::Calibration,
        kind,
        kind_from_folder,
        stacked,
        device,
        object: target.object,
        ra_degrees: target.ra_degrees,
        dec_degrees: target.dec_degrees,
        field_degrees: target.field_degrees,
        date_obs: acquisition
            .date_obs_utc
            .as_deref()
            .and_then(parse_iso8601_utc_to_unix_seconds),
        date_zone_unknown: acquisition.date_zone_unknown,
        exposure_seconds: acquisition.exposure_seconds,
        filter: acquisition
            .filter
            .map(|f| f.trim().to_string())
            .filter(|f| !f.is_empty()),
        gain: acquisition.gain,
        temperature_c: acquisition.sensor_temp_c.or_else(|| header.get_f64("TEMP")),
        site: header_site(header).map(|site| (site.latitude, site.longitude)),
        telescop: text(header, "TELESCOP"),
        instrume: text(header, "INSTRUME"),
        creator: text(header, "CREATOR"),
        focal_length_mm: fits::focal_length_mm(header),
        altitude_deg: header.get("OBJCTALT").and_then(parse_angle),
        azimuth_deg: header.get("OBJCTAZ").and_then(parse_angle),
        lst_hours: header.get("LST").and_then(parse_angle),
        bayer: text(header, "BAYERPAT"),
        band_guess: None,
    }
}

/// The notes a person may keep beside a night (`SESSION.md`, then
/// `README.md`): their first heading is read for a band word.
const NOTE_FILES: [&str; 2] = ["SESSION.md", "README.md"];

/// Most of a note that is read (a heading is in its first lines).
const NOTE_READ_BYTES: u64 = 8 * 1024;

/// The headings of the notes beside a frame and above it, up to the
/// location's root (`depth` folders, the root included), nearest first, as
/// `(file name, heading)`. A `README.md` of the root itself is the archive's,
/// not a night's: it is not read. Read once per folder.
fn notes_of(
    path: &Path,
    depth: usize,
    cache: &mut HashMap<PathBuf, Vec<(String, String)>>,
) -> Vec<(String, String)> {
    let mut found = Vec::new();
    for (level, folder) in path.ancestors().skip(1).take(depth).enumerate() {
        let at_root = level + 1 == depth;
        let notes = cache.entry(folder.to_path_buf()).or_insert_with(|| {
            NOTE_FILES
                .iter()
                .filter_map(|name| {
                    let mut text = String::new();
                    std::fs::File::open(folder.join(name))
                        .ok()?
                        .take(NOTE_READ_BYTES)
                        .read_to_string(&mut text)
                        .ok()?;
                    band::first_heading(&text).map(|heading| ((*name).to_string(), heading))
                })
                .collect()
        });
        found.extend(
            notes
                .iter()
                .filter(|(name, _)| !(at_root && depth > 1 && name == "README.md"))
                .cloned(),
        );
    }
    found
}

/// The folders from the location's root (its own name included) down to
/// the file.
fn folders_of(root: &Path, relative: &str) -> Vec<String> {
    let mut folders: Vec<String> = root
        .file_name()
        .map(|n| vec![n.to_string_lossy().into_owned()])
        .unwrap_or_default();
    let parts: Vec<&str> = relative.split('/').collect();
    folders.extend(
        parts[..parts.len().saturating_sub(1)]
            .iter()
            .map(|p| (*p).to_string()),
    );
    folders
}

/// Whether a text file is the description a capture program wrote for a
/// video of the same folder (`Lune.avi.txt` or `Lune.txt` beside
/// `Lune.avi`), whatever the case. `videos` holds the lower-cased relative
/// paths of the walk's videos.
fn describes_video(relative_path: &str, videos: &HashSet<String>) -> bool {
    let lowered = relative_path.to_lowercase();
    let Some(without_text) = lowered
        .strip_suffix(".txt")
        .or_else(|| lowered.strip_suffix(".log"))
    else {
        return false;
    };
    // `Lune.avi.txt` (the video's whole name) or `Lune.txt` (its stem).
    videos.contains(without_text)
        || videos.iter().any(|video| {
            video
                .rsplit_once('.')
                .is_some_and(|(name, _)| name == without_text)
        })
}

/// The running counts of "Found so far".
#[derive(Default)]
struct Tally {
    nights: BTreeSet<i64>,
    targets: BTreeSet<String>,
    places: BTreeSet<(i64, i64)>,
    instruments: BTreeSet<(String, String)>,
    lights: usize,
    calibration_files: usize,
    calibration_sets: BTreeSet<(u8, i64, i64)>,
    calibration_kinds: BTreeSet<u8>,
    log_files: usize,
    log_kinds: BTreeSet<u8>,
    to_solve: BTreeSet<String>,
}

impl Tally {
    fn frame(&mut self, frame: &FrameRecord) {
        if frame.stacked {
            return;
        }
        match frame.kind {
            Some(FrameType::Light) => {
                self.lights += 1;
                let longitude = frame.site.map_or(0.0, |(_, lon)| lon);
                if let Some(t) = frame.date_obs {
                    // The evening: the day of the mean solar noon before.
                    let local = t + longitude / 15.0 * 3600.0 - 12.0 * 3600.0;
                    self.nights.insert((local / 86_400.0).floor() as i64);
                }
                match (&frame.object, frame.ra_degrees.zip(frame.dec_degrees)) {
                    (Some(object), _) => {
                        let key: String = object
                            .chars()
                            .filter(|c| c.is_alphanumeric())
                            .flat_map(char::to_lowercase)
                            .collect();
                        self.targets.insert(key);
                    }
                    (None, Some((ra, dec))) => {
                        self.targets
                            .insert(format!("@{:.0}:{:.0}", ra.round(), dec.round()));
                    }
                    (None, None) => {
                        self.to_solve.insert(frame.folders.join("/"));
                    }
                }
                if let Some((lat, lon)) = frame.site {
                    self.places
                        .insert(((lat * 10.0).round() as i64, (lon * 10.0).round() as i64));
                }
                self.instruments.insert((
                    frame.instrume.clone().unwrap_or_default(),
                    frame.telescop.clone().unwrap_or_default(),
                ));
            }
            Some(kind) => {
                let rank = match kind {
                    FrameType::Dark => 0,
                    FrameType::Flat => 1,
                    FrameType::DarkFlat => 2,
                    _ => 3,
                };
                self.calibration_files += 1;
                self.calibration_kinds.insert(rank);
                self.calibration_sets.insert((
                    rank,
                    (frame.exposure_seconds.unwrap_or(0.0) * 1000.0).round() as i64,
                    frame.gain.unwrap_or(-1.0).round() as i64,
                ));
            }
            None => {}
        }
    }

    fn log(&mut self, kind: LogKind) {
        self.log_files += 1;
        self.log_kinds.insert(match kind {
            LogKind::Nina => 0,
            LogKind::Asiair => 1,
            _ => 2,
        });
    }

    fn found(&self) -> FoundSoFar {
        const CALIBRATION: [&str; 4] = ["dark", "flat", "dark-flat", "bias"];
        const LOGS: [&str; 3] = ["nina", "asiair", "phd2"];
        FoundSoFar {
            nights: self.nights.len(),
            targets: self.targets.len(),
            places: self.places.len(),
            instruments: self.instruments.len(),
            lights: self.lights,
            calibration_files: self.calibration_files,
            calibration_sets: self.calibration_sets.len(),
            calibration_kinds: self
                .calibration_kinds
                .iter()
                .map(|k| CALIBRATION[*k as usize].to_string())
                .collect(),
            log_files: self.log_files,
            log_kinds: self
                .log_kinds
                .iter()
                .map(|k| LOGS[*k as usize].to_string())
                .collect(),
            to_solve: self.to_solve.len(),
        }
    }
}

/// How often "Found so far" is reported, in files.
const FOUND_EVERY: usize = 25;

/// The folders as they are on disk (links resolved), to tell where a file
/// lives whatever path reached it.
fn canonical_roots(roots: &[PathBuf]) -> Vec<PathBuf> {
    roots
        .iter()
        .map(|root| std::fs::canonicalize(root).unwrap_or_else(|_| root.clone()))
        .collect()
}

/// Reads the three locations: the FITS first, then the calibration, then
/// the logs. A file is read once, in the location it was first met in,
/// except a file met from the FITS through a link (a night's `dark` link)
/// that lives in a calibration folder and in no FITS folder: it is read as
/// calibration. A folder given to several locations (the same folder for
/// the FITS and the calibration) is thus read as FITS: its lights are
/// lights, with the band their names say. Logs met in the folders of FITS
/// (a night's `logs` link) are read too.
pub fn read_locations(
    request: &DetectRequest,
    options: &DetectOptions<'_>,
) -> Result<Reading, DetectError> {
    if request.fits.is_empty() {
        return Err(DetectError::NoFitsFolder);
    }
    let roots: Vec<(Location, &PathBuf)> = request
        .fits
        .iter()
        .map(|p| (Location::Fits, p))
        .chain(
            request
                .calibration
                .iter()
                .map(|p| (Location::Calibration, p)),
        )
        .chain(request.logs.iter().map(|p| (Location::Logs, p)))
        .collect();
    let fits_roots = canonical_roots(&request.fits);
    let calibration_roots = canonical_roots(&request.calibration);
    let only_in_calibration = |canonical: &Path| {
        calibration_roots
            .iter()
            .any(|root| canonical.starts_with(root))
            && !fits_roots.iter().any(|root| canonical.starts_with(root))
    };

    let mut reading = Reading::default();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    // Where in `todo` each file to read is, by its path on disk.
    let mut queued: HashMap<PathBuf, usize> = HashMap::new();
    let mut todo: Vec<(Location, &PathBuf, WalkedFile)> = Vec::new();
    // Files met by the walks of the locations done so far: the count the
    // window shows while the files are searched is the whole one.
    let mut walked_before = 0_u64;
    for (location, root) in roots {
        // Before the first header is read the window would say nothing (and
        // « 0 sur 0 »): the walk says how many files it met and where.
        let report = |found: u64, folder: &std::path::Path| {
            if let Some(progress) = options.progress {
                progress(&WorkProgress {
                    kind: WorkKind::Scan,
                    done: walked_before + found,
                    total: 0,
                    bytes_done: 0,
                    bytes_total: 0,
                    current: Some(folder.to_string_lossy().into_owned()),
                });
            }
        };
        let walk =
            walk_folder_reporting(root, options.cancel, Some(&report)).map_err(
                |error| match error {
                    ScanError::Cancelled => DetectError::Cancelled,
                    other => other.into(),
                },
            )?;
        walked_before += walk.files.len() as u64;
        reading.links.extend(walk.links);
        // The videos of this walk, by folder and name: the text files a
        // capture program wrote beside them follow them.
        let videos: HashSet<String> = walk
            .files
            .iter()
            .filter(|file| file.class == FileClass::Video)
            .map(|file| file.relative_path.to_lowercase())
            .collect();
        for file in walk.files {
            if !request.subfolders && file.relative_path.contains('/') {
                continue;
            }
            // Not readable here: said, never opened.
            if file.class == FileClass::Video
                || (file.class == FileClass::Log && describes_video(&file.relative_path, &videos))
            {
                let canonical =
                    std::fs::canonicalize(&file.path).unwrap_or_else(|_| file.path.clone());
                if seen.insert(canonical) {
                    let shown = file.path.display().to_string();
                    match file.class {
                        FileClass::Video => reading.videos.push(shown),
                        _ => reading.video_captures.push(shown),
                    }
                }
                continue;
            }
            let wanted = match location {
                Location::Logs => file.class == FileClass::Log,
                _ => file.class.is_frame() || file.class == FileClass::Log,
            };
            if !wanted {
                continue;
            }
            let canonical = std::fs::canonicalize(&file.path).unwrap_or_else(|_| file.path.clone());
            if seen.insert(canonical.clone()) {
                queued.insert(canonical, todo.len());
                todo.push((location, root, file));
            } else if location == Location::Calibration
                && let Some(&index) = queued.get(&canonical)
                && todo[index].0 == Location::Fits
                && only_in_calibration(&canonical)
            {
                // A calibration frame first met through a night's link: read
                // where it lives, as calibration.
                todo[index] = (location, root, file);
            }
        }
    }

    let total = todo.len() as u64;
    let bytes_total: u64 = todo.iter().map(|(_, _, f)| f.stamp.size).sum();
    let mut bytes_done = 0_u64;
    let mut tally = Tally::default();
    let mut notes: HashMap<PathBuf, Vec<(String, String)>> = HashMap::new();
    for (index, (location, root, file)) in todo.into_iter().enumerate() {
        if options.cancel.is_some_and(|c| c.is_cancelled()) {
            return Err(DetectError::Cancelled);
        }
        if let Some(progress) = options.progress {
            progress(&WorkProgress {
                kind: WorkKind::Headers,
                done: index as u64,
                total,
                bytes_done,
                bytes_total,
                // The whole path (the window shows it under the bar).
                current: Some(file.path.to_string_lossy().into_owned()),
            });
        }
        if let Some(found) = options.found
            && index % FOUND_EVERY == 0
            && index > 0
        {
            found(&tally.found());
        }
        bytes_done += file.stamp.size;
        reading.files_read += 1;
        if file.class == FileClass::Log {
            match detection::log_kind(&file) {
                LogKind::Unknown => reading.other_files.push(file.path.display().to_string()),
                kind => {
                    tally.log(kind);
                    reading.logs.push(LogRecord {
                        path: file.path.clone(),
                        kind,
                    });
                }
            }
            continue;
        }
        let folders = folders_of(root, &file.relative_path);
        match detection::read_frame_header(&file.path, file.class) {
            Some(header) => {
                let mut record = record_of(&header, &file, folders.clone(), location);
                // Whatever location found it: a light read from a folder
                // also given as calibration is a light all the same.
                if record.kind == Some(FrameType::Light) && record.filter.is_none() {
                    let name = file.relative_path.rsplit('/').next().unwrap_or_default();
                    let beside = notes_of(&file.path, folders.len(), &mut notes);
                    record.band_guess = band::guess_for_frame(&folders, name, &beside);
                }
                tally.frame(&record);
                reading.frames.push(record);
            }
            None => {
                let name = file.relative_path.rsplit('/').next().unwrap_or_default();
                let list = if file.class == FileClass::Raw {
                    // A RAW variant the reader does not know: said, not read
                    // (and not counted among the files read).
                    reading.files_read -= 1;
                    &mut reading.raw_files
                } else if named_stacked(name, &folders) {
                    &mut reading.stacks
                } else {
                    &mut reading.unreadable
                };
                list.push(file.path.display().to_string());
            }
        }
    }
    if let Some(progress) = options.progress {
        progress(&WorkProgress {
            kind: WorkKind::Headers,
            done: total,
            total,
            bytes_done,
            bytes_total,
            current: None,
        });
    }
    if let Some(found) = options.found {
        found(&tally.found());
    }
    Ok(reading)
}
