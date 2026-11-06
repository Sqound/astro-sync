//! The multi-night plans in Astro Sync: a capability of the hosted service
//! (`analysis.progression`), **always open on a private server** (the server
//! never restricts them; Astro Sync follows the server, which it asks,
//! [`sells_multi_night`]).
//!
//! - **The masters kept** ([`IntegrationMaster`], [`read_master`],
//!   [`night_masters`]): the stacking keeps, per night, target and family,
//!   the 32-bit linear master `<night>/<target>/stacked/<family>_integration.fit`
//!   ([`crate::stacking`]); its own FITS header says what it integrates
//!   (`FILTER`, `EXPTIME` of one frame, `GAIN`, `XBINNING`, `CCD-TEMP`,
//!   `INSTRUME`, `STACKCNT`, `LIVETIME`, `ASFAMILY`), so it is read again
//!   later without any other file. The same poses stacked again under
//!   another filter leave two masters: one is active, the other stays on
//!   disk out of the plan and of the lines ([`night_masters`]).
//! - **The image a device stacked** ([`DeviceSource`], [`night_device_images`]): a night
//!   of which only the device's own stack (`<target>/device/Stacked_*.fit`)
//!   is here is a night of the plan too; its
//!   integration line is read from the image's header (`STACKCNT`, `TOTALEXP`,
//!   [`device_master::recognise`]) and a family a kept master already covers is
//!   not listed twice ([`night_sources`]).
//! - **The lines** ([`IntegrationLine`], [`integration_lines`]): what leaves
//!   for the service -- one line per family, never a file
//!   (`integration.schema.json`, `write`): the night, the target, the
//!   family, the frames, the evening, the sensor, this computer's label and
//!   its installation.
//! - **The plan of a night** ([`night_projects`]): the night's plan
//!   (`GET /v1/plans/{planId}`) says, item by item, which multi-night plan a
//!   target works for (`planItem.projectId`); a night attached by itself
//! is read by `GET /v1/projects?sessionId=`; each plan is
//!   read (`GET /v1/projects/{id}`) with the integrations the service
//!   knows. Reading never needs a capability. Attaching, detaching and
//!   creating from a night are in [`crate::multi_night_attach`].
//! - **The right** ([`MultiNightGate`]): `analysis.progression` on a
//!   service that restricts them (the hosted one, as it says itself); always
//!   open on a private server. Without
//!   it nothing is sent: the screens grey the lines and say why. It covers
//!   what Astro Sync sends to a plan (the integrations, the plan's image).
//!   The gestures on the plan itself -- attaching, detaching, creating from
//!   a night -- have their own right, `planner.mosaic` ("Advanced
//!   Planning"): [`AttachGate`].
//! - **The sending** ([`send_integrations`]): `PUT /v1/projects/{id}/integrations`,
//!   idempotent (night, target and family are the key); the lines of a night
//!   and a target are sent **complete** (`replace`): the service drops the
//!   line of a family the night no longer integrates (a night stacked again
//!   under another filter), and a service that predates `replace` is sent the
//!   earlier form (lines added or replaced, none removed). A refusal of the
//!   service (`403 multi_night_not_active`, `409 integration_outside_night`)
//!   is kept with its code and words, never an error that stops the night.
//! - **The tags of "Nights"** ([`plan_tags`]): each night with an
//!   integration in a multi-night plan, "Multi-night plan IC 1805 · 3 nights
//!   · 11 h 08" (the integrations known, not the debriefings' progress).
//!
//! The readers are tolerant: a field the service adds later breaks nothing.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::analysis::fits::read_primary_header_file;
use crate::detection::NightWindow;
use crate::device_master;
use crate::gui_bridge::{Capabilities, GuiBridgeError, HttpPortalGuiTransport};
use crate::service_wait::{RequestFactory, WaitHooks, WakePolicy};
use crate::space::{SignInAnnouncement, SpaceClient, SpaceCreation, SpaceError};
use crate::stacking::INTEGRATION_SUFFIX;
use crate::working_folder::{DEVICE_DIR, STACKED_DIR, target_folders_by_identity};

/* ------------------------------ The masters ------------------------------ */

/// One master kept on this computer, described by its own header.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationMaster {
    pub path: PathBuf,
    /// The family's label (`Ha_300s_g100`), the stem of the file.
    pub family: String,
    /// The filter as the frames name it; `None`: "No filter".
    pub filter: Option<String>,
    /// One frame's exposure, seconds.
    pub exposure_seconds: Option<f64>,
    pub gain: Option<f64>,
    pub binning: Option<u32>,
    /// The sensor temperature of the family (the median of its frames).
    pub temperature_c: Option<f64>,
    /// The frames integrated (`STACKCNT`).
    pub frame_count: u64,
    /// The integration time (`LIVETIME`, else frames × exposure), seconds.
    pub total_seconds: f64,
    /// The sensor (`INSTRUME`).
    pub sensor: Option<String>,
    /// The size of the file, bytes (the place it takes, said on screen).
    pub bytes: u64,
    /// Three colour planes (a debayered colour sensor).
    pub colour: bool,
}

/// The binning of a header (`XBINNING = 2`, or `2x2`).
fn binning_of(text: &str) -> Option<u32> {
    text.trim()
        .split(['x', 'X'])
        .next()
        .and_then(|first| first.trim().parse::<f64>().ok())
        .filter(|value| *value >= 1.0)
        .map(|value| value.round() as u32)
}

/// Reads a master kept: `None` when the file is not one (no header, no
/// frame count).
#[must_use]
pub fn read_master(path: &Path) -> Option<IntegrationMaster> {
    let header = read_primary_header_file(path).ok()?;
    let bytes = fs::metadata(path).ok()?.len();
    let stem = path.file_name()?.to_string_lossy().into_owned();
    let family = header
        .get("ASFAMILY")
        .map(str::to_string)
        .or_else(|| stem.strip_suffix(INTEGRATION_SUFFIX).map(str::to_string))?;
    let frame_count = header
        .get_f64("STACKCNT")
        .or_else(|| header.get_f64("NCOMBINE"))
        .filter(|n| *n >= 1.0)? as u64;
    let exposure_seconds = crate::analysis::fits::exposure_seconds(&header).filter(|e| *e > 0.0);
    let total_seconds = header
        .get_f64("LIVETIME")
        .filter(|t| *t > 0.0)
        .or_else(|| exposure_seconds.map(|e| e * frame_count as f64))
        .unwrap_or(0.0);
    let text = |key: &str| {
        header
            .get(key)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    };
    Some(IntegrationMaster {
        path: path.to_path_buf(),
        family,
        filter: text("FILTER"),
        exposure_seconds,
        gain: header.get_f64("GAIN"),
        binning: text("XBINNING").as_deref().and_then(binning_of),
        temperature_c: header
            .get_f64("CCD-TEMP")
            .or_else(|| header.get_f64("SET-TEMP")),
        frame_count,
        total_seconds,
        sensor: text("INSTRUME"),
        bytes,
        colour: header.get_i64("NAXIS") == Some(3),
    })
}

/// The masters kept for one target of a night (`<target>/stacked/*_integration.fit`),
/// by family, one per set of poses ([`retain_active_masters`]).
#[must_use]
pub fn night_masters(night_folder: &Path, target_folder: &str) -> Vec<IntegrationMaster> {
    let stacked = night_folder.join(target_folder).join(STACKED_DIR);
    let Ok(entries) = fs::read_dir(&stacked) else {
        return Vec::new();
    };
    let mut masters: Vec<IntegrationMaster> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().ends_with(INTEGRATION_SUFFIX))
        })
        .filter_map(|path| read_master(&path))
        .collect();
    let assumed = crate::working_folder::read_target_filter(&night_folder.join(target_folder));
    retain_active_masters(&mut masters, assumed.as_deref());
    // A master stacked before the filter was known (the lights name none)
    // takes the one the target folder records.
    if let Some(assumed) = assumed {
        for master in masters.iter_mut().filter(|m| m.filter.is_none()) {
            master.filter = Some(assumed.clone());
        }
    }
    masters.sort_by(|a, b| a.family.cmp(&b.family));
    masters
}

/// The poses a master integrates, as far as its header tells them apart:
/// the first frame's `DATE-OBS`, one frame's exposure (ms), the gain and the
/// binning. `None` when the header has no `DATE-OBS`: such a master is never
/// taken for another one.
fn poses_of(master: &IntegrationMaster) -> Option<(String, i64, Option<i64>, Option<u32>)> {
    let header = read_primary_header_file(&master.path).ok()?;
    let date = header
        .get("DATE-OBS")
        .map(str::trim)
        .filter(|d| !d.is_empty())?
        .to_string();
    Some((
        date,
        (master.exposure_seconds? * 1000.0).round() as i64,
        master.gain.map(|g| g.round() as i64),
        master.binning,
    ))
}

/// One master per set of poses: the lights of a night that name no filter
/// can be stacked again under another one (the person corrects the filter
/// the folder names suggested), which writes a master of another family
/// beside the first. Both integrate the same poses: kept together, the night
/// would count twice in a plan and in the lines sent to the service. Of the
/// masters of the same poses, the one whose own filter is the filter the
/// target folder records now is kept (it is the person's last choice), else
/// the newest file. The others stay on disk, untouched, out of the plan.
fn retain_active_masters(masters: &mut Vec<IntegrationMaster>, recorded: Option<&str>) {
    if masters.len() < 2 {
        return;
    }
    let keys: Vec<_> = masters.iter().map(poses_of).collect();
    let modified =
        |master: &IntegrationMaster| fs::metadata(&master.path).and_then(|m| m.modified()).ok();
    let mut keep = vec![true; masters.len()];
    for (i, key) in keys.iter().enumerate() {
        let Some(key) = key else { continue };
        let same: Vec<usize> = (0..masters.len())
            .filter(|j| keys[*j].as_ref() == Some(key))
            .collect();
        if same.len() < 2 {
            continue;
        }
        let chosen = same
            .iter()
            .copied()
            .find(|j| recorded.is_some() && masters[*j].filter.as_deref() == recorded)
            .unwrap_or_else(|| {
                same.iter()
                    .copied()
                    .max_by(|a, b| {
                        modified(&masters[*a])
                            .cmp(&modified(&masters[*b]))
                            .then_with(|| masters[*a].family.cmp(&masters[*b].family))
                    })
                    .unwrap_or(i)
            });
        keep[i] = i == chosen;
    }
    let mut index = 0;
    masters.retain(|_| {
        index += 1;
        keep[index - 1]
    });
}

/// The masters of a target of the night found by its identity
/// (`.astro-sync-target.json`), whatever its folder is called.
#[must_use]
pub fn target_masters(night_folder: &Path, target_id: &str) -> Vec<IntegrationMaster> {
    target_folders_by_identity(night_folder)
        .into_iter()
        .filter(|(id, _)| id == target_id)
        .flat_map(|(_, folder)| night_masters(night_folder, &folder))
        .collect()
}

/* ------------------------ The image a device stacked ----------------------- */

/// An image a device stacked (a Seestar's `Stacked_*.fit` in `<target>/device/`) as the integration of a
/// night of a plan, described by its own header: its frames are `STACKCNT`, its time `TOTALEXP` (else
/// frames x exposure).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceSource {
    pub path: PathBuf,
    /// The file's name (`Stacked_430_M 45_30.0s_IRCUT_20261011-212040.fit`).
    pub file_name: String,
    /// The family label of the image ([`crate::device_master::DeviceImageInfo::family`]).
    pub family: String,
    /// « Seestar S50 Pro ».
    pub device: String,
    pub filter: Option<String>,
    pub exposure_seconds: Option<f64>,
    pub gain: Option<f64>,
    /// The poses it stacked (`STACKCNT`).
    pub frame_count: u64,
    pub total_seconds: f64,
    pub bytes: u64,
}

impl DeviceSource {
    /// The image as a master of the plan's night: the line the service gets, the totals the plan weighs.
    #[must_use]
    pub fn as_master(&self) -> IntegrationMaster {
        IntegrationMaster {
            path: self.path.clone(),
            family: self.family.clone(),
            filter: self.filter.clone(),
            exposure_seconds: self.exposure_seconds,
            gain: self.gain,
            binning: None,
            temperature_c: None,
            frame_count: self.frame_count,
            total_seconds: self.total_seconds,
            sensor: Some(self.device.clone()),
            bytes: self.bytes,
            colour: true,
        }
    }

    /// Whether this image integrates the same family as `master` (the filter, the exposure and the gain; a
    /// header of the device does not always say the binning).
    #[must_use]
    pub fn same_family_as(&self, master: &IntegrationMaster) -> bool {
        same_acquisition(
            (self.filter.as_deref(), self.exposure_seconds, self.gain),
            (
                master.filter.as_deref(),
                master.exposure_seconds,
                master.gain,
            ),
        )
    }
}

/// The same filter, the same exposure (to the millisecond) and the same gain (to the unit); a missing
/// exposure or gain on both sides is the same.
pub(crate) fn same_acquisition(
    left: (Option<&str>, Option<f64>, Option<f64>),
    right: (Option<&str>, Option<f64>, Option<f64>),
) -> bool {
    let close = |a: Option<f64>, b: Option<f64>| match (a, b) {
        (Some(a), Some(b)) => (a - b).abs() < 1e-3,
        (None, None) => true,
        _ => false,
    };
    left.0 == right.0
        && close(left.1, right.1)
        && close(left.2.map(f64::round), right.2.map(f64::round))
}

/// The images of a known device in the `device/` folder of a target of a night, the biggest of each family
/// (Astro Sync never combines two images of a device: the Seestar cuts its stack when the target moves, and
/// the bigger one is the night), by family. Reads headers only.
#[must_use]
pub fn night_device_images(night_folder: &Path, target_folder: &str) -> Vec<DeviceSource> {
    let folder = night_folder.join(target_folder).join(DEVICE_DIR);
    let Ok(entries) = fs::read_dir(&folder) else {
        return Vec::new();
    };
    let mut biggest: BTreeMap<String, DeviceSource> = BTreeMap::new();
    let mut names: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect();
    names.sort();
    for path in names {
        let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
            continue;
        };
        let fits = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "fit" | "fits" | "fts"));
        if !fits || !path.is_file() {
            continue;
        }
        let Ok(header) = read_primary_header_file(&path) else {
            continue;
        };
        let Some(info) = device_master::recognise(&header, &name) else {
            continue;
        };
        let bytes = fs::metadata(&path).map_or(0, |m| m.len());
        let total_seconds = info
            .total_seconds
            .filter(|t| *t > 0.0)
            .or_else(|| info.exposure_seconds.map(|e| e * info.poses as f64))
            .unwrap_or(0.0);
        let source = DeviceSource {
            path,
            file_name: name,
            family: info.family.clone(),
            device: info.device,
            filter: info.filter,
            exposure_seconds: info.exposure_seconds,
            gain: info.gain,
            frame_count: info.poses,
            total_seconds,
            bytes,
        };
        let replace = biggest
            .get(&info.family)
            .is_none_or(|kept| source.frame_count > kept.frame_count);
        if replace {
            biggest.insert(info.family, source);
        }
    }
    biggest.into_values().collect()
}

/// The device images of a target of the night found by its identity, whatever its folder is called.
#[must_use]
pub fn target_device_images(night_folder: &Path, target_id: &str) -> Vec<DeviceSource> {
    target_folders_by_identity(night_folder)
        .into_iter()
        .filter(|(id, _)| id == target_id)
        .flat_map(|(_, folder)| night_device_images(night_folder, &folder))
        .collect()
}

/// What a night integrates for a plan: the masters kept, and the image of its device for each family no kept
/// master covers (the integration line of a night that was never stacked here, read from the header).
#[must_use]
pub fn night_sources(
    masters: &[IntegrationMaster],
    devices: &[DeviceSource],
) -> Vec<IntegrationMaster> {
    let mut sources = masters.to_vec();
    sources.extend(
        devices
            .iter()
            .filter(|device| !masters.iter().any(|m| device.same_family_as(m)))
            .map(DeviceSource::as_master),
    );
    sources
}

/// One family of a target's lights as "The night's integration" shows it
/// before (and after) the stacking: what will be integrated, and the master
/// kept for it if there is one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FamilyPreview {
    /// The stacking's label of the family (`Ha_300s_g100`).
    pub label: String,
    pub filter: Option<String>,
    /// What its filter lets through (the card's "Dual-band" or
    /// "Broadband", and the pipelines recommended for it).
    #[serde(default)]
    pub band: crate::plan_image::Band,
    pub exposure_seconds: Option<f64>,
    pub gain: Option<f64>,
    pub binning: Option<String>,
    /// The temperature most of its frames share, whole degrees.
    pub temperature_c: Option<i64>,
    /// The frames inside the night's window.
    pub frame_count: u64,
    pub total_seconds: f64,
    /// The master kept, when the family was stacked already.
    pub master: Option<IntegrationMaster>,
}

/// What "The night's integration" says of one target.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationPreview {
    pub target_folder: String,
    pub families: Vec<FamilyPreview>,
    /// The lights outside the night's window, left out of the integration.
    pub outside: u64,
}

/// The families of a target of the night, their frames inside `window`,
/// the largest first (the stacking's grouping: temperatures merged).
#[must_use]
pub fn integration_preview(
    night_folder: &Path,
    target_folder: &str,
    window: Option<&NightWindow>,
) -> IntegrationPreview {
    let (found, outside) =
        crate::progression_images::target_families_within(night_folder, target_folder, window);
    let masters = night_masters(night_folder, target_folder);
    let mut families: Vec<(FamilyPreview, Vec<(i64, u64)>)> = Vec::new();
    for family in found {
        let label = crate::stacking::family_label(&family);
        let frames = family.frame_paths.len() as u64;
        let index = match families.iter().position(|(f, _)| f.label == label) {
            Some(index) => index,
            None => {
                families.push((
                    FamilyPreview {
                        label: label.clone(),
                        filter: family.filter.clone(),
                        band: crate::plan_image::band_of(family.filter.as_deref()),
                        exposure_seconds: family.exposure_seconds,
                        gain: family.gain,
                        binning: family.binning.clone(),
                        temperature_c: None,
                        frame_count: 0,
                        total_seconds: 0.0,
                        master: masters.iter().find(|m| m.family == label).cloned(),
                    },
                    Vec::new(),
                ));
                families.len() - 1
            }
        };
        let (preview, temperatures) = &mut families[index];
        preview.frame_count += frames;
        preview.total_seconds += frames as f64 * family.exposure_seconds.unwrap_or(0.0);
        if let Some(t) = family.temperature_class_c {
            temperatures.push((t, frames));
        }
    }
    let mut families: Vec<FamilyPreview> = families
        .into_iter()
        .map(|(mut preview, temperatures)| {
            preview.temperature_c = temperatures
                .iter()
                .max_by_key(|(t, n)| (*n, -*t))
                .map(|(t, _)| *t);
            preview
        })
        .collect();
    families.sort_by(|a, b| {
        b.frame_count
            .cmp(&a.frame_count)
            .then(a.label.cmp(&b.label))
    });
    IntegrationPreview {
        target_folder: target_folder.to_string(),
        families,
        outside: outside.len() as u64,
    }
}

/* ------------------------------- The lines ------------------------------- */

/// This computer, as a line names it: where the master is kept.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Computer {
    /// The label of the paired device ("office iMac").
    pub label: Option<String>,
    /// Astro Sync's installation on this computer.
    pub installation_id: Option<String>,
}

/// One line sent to the service (`integration.schema.json`, `write`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationLine {
    pub night_id: String,
    pub target_id: String,
    pub filter: Option<String>,
    pub exposure_seconds: f64,
    pub gain: Option<i64>,
    pub binning: Option<u32>,
    #[serde(rename = "temperatureC")]
    pub temperature_c: Option<f64>,
    pub frame_count: u64,
    pub evening_date: String,
    pub sensor: Option<String>,
    pub computer_label: Option<String>,
    pub installation_id: Option<String>,
}

fn bounded(text: Option<&str>, max: usize) -> Option<String> {
    text.map(str::trim)
        .filter(|t| !t.is_empty())
        .map(|t| t.chars().take(max).collect())
}

/// The lines of one target of a night, one per master (a master without an
/// exposure cannot say its time: it is left out).
#[must_use]
pub fn integration_lines(
    night_id: &str,
    target_id: &str,
    evening_date: &str,
    masters: &[IntegrationMaster],
    computer: &Computer,
) -> Vec<IntegrationLine> {
    masters
        .iter()
        .filter_map(|master| {
            Some(IntegrationLine {
                night_id: night_id.to_string(),
                target_id: target_id.to_string(),
                filter: bounded(master.filter.as_deref(), 40),
                exposure_seconds: master.exposure_seconds?,
                gain: master.gain.map(|g| g.round() as i64),
                binning: master.binning.filter(|b| (1..=16).contains(b)),
                temperature_c: master
                    .temperature_c
                    .filter(|t| (-100.0..=100.0).contains(t))
                    .map(|t| (t * 10.0).round() / 10.0),
                frame_count: master.frame_count,
                evening_date: evening_date.to_string(),
                sensor: bounded(master.sensor.as_deref(), 80),
                computer_label: bounded(computer.label.as_deref(), 80),
                installation_id: bounded(computer.installation_id.as_deref(), 100),
            })
        })
        .collect()
}

/* -------------------------------- The right ------------------------------- */

/// Whether a service restricts the multi-night plans (a private server never
/// does; Astro Sync follows the server): the
/// service that creates a space for any address — its
/// `GET /v1/service` announces `signIn.method: "email-code"` with
/// `spaceCreation: "open"` —, the hosted one, whose space's capabilities
/// then decide. Any other service (pairing, the owner's address alone, no
/// sign-in) is a private server, where the multi-night plans are always
/// open. Asked of the service, never guessed from its address.
#[must_use]
pub fn sells_multi_night(announcement: &SignInAnnouncement) -> bool {
    matches!(
        announcement,
        SignInAnnouncement::EmailCode {
            space_creation: SpaceCreation::Open,
            ..
        }
    )
}

/// Asks the service at `portal_url` whether it restricts the multi-night plans
/// ([`sells_multi_night`], `GET /v1/service`), waiting as `policy` says.
pub fn service_sells_multi_night(portal_url: &str, policy: WakePolicy) -> Result<bool, SpaceError> {
    let client = SpaceClient::with_options(portal_url, policy, WaitHooks::silent())?;
    Ok(sells_multi_night(&client.announcement()?))
}

/// What this computer last heard a service say about selling the
/// multi-night plans (`<state>/service-sells-multi-night.json`), so that a
/// service out of reach keeps the answer it gave.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SoldRecord {
    portal_url: String,
    sells_multi_night: bool,
}

fn same_portal(left: &str, right: &str) -> bool {
    left.trim()
        .trim_end_matches('/')
        .eq_ignore_ascii_case(right.trim().trim_end_matches('/'))
}

/// Asks the service ([`service_sells_multi_night`]) and keeps its answer at
/// `record`; when it cannot be asked, the answer it last gave for the same
/// address, else `None` (never asked: the gates stay closed, "unknown").
#[must_use]
pub fn service_sells_multi_night_kept(
    portal_url: &str,
    policy: WakePolicy,
    record: &Path,
) -> Option<bool> {
    match service_sells_multi_night(portal_url, policy) {
        Ok(sold) => {
            let kept = SoldRecord {
                portal_url: portal_url.trim().to_string(),
                sells_multi_night: sold,
            };
            if let Some(parent) = record.parent() {
                let _ = fs::create_dir_all(parent);
            }
            if let Ok(json) = serde_json::to_string_pretty(&kept) {
                let _ = fs::write(record, json);
            }
            Some(sold)
        }
        Err(_) => fs::read_to_string(record)
            .ok()
            .and_then(|raw| serde_json::from_str::<SoldRecord>(&raw).ok())
            .filter(|kept| same_portal(&kept.portal_url, portal_url))
            .map(|kept| kept.sells_multi_night),
    }
}

/// Why the multi-night plans are closed here (never on a private server:
/// it never restricts them).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MultiNightClosed {
    /// The capability of the gate is not granted by the service that sells
    /// them (`analysis.progression` for [`MultiNightGate`], `planner.mosaic`
    /// for [`AttachGate`]): "See the offers".
    NotInSubscription,
    /// The capabilities, or whether the service sells them, are not known
    /// yet.
    Unknown,
}

/// Why a gate is closed, given whether its capability is granted (`None`:
/// the capabilities are not known yet) and whether the service sells the
/// multi-night plans. A service that does not sell them never closes it.
fn closed_reason(granted: Option<bool>, sold: bool) -> Option<MultiNightClosed> {
    if !sold {
        return None;
    }
    match granted {
        None => Some(MultiNightClosed::Unknown),
        Some(false) => Some(MultiNightClosed::NotInSubscription),
        Some(true) => None,
    }
}

/// Whether Astro Sync sends the integrations and makes the plan's image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MultiNightGate {
    pub allowed: bool,
    pub closed: Option<MultiNightClosed>,
}

impl MultiNightGate {
    /// `analysis.progression` on a service that sells the multi-night plans
    /// (`sold`, [`sells_multi_night`]); always open on a private server.
    #[must_use]
    pub fn new(capabilities: Option<&Capabilities>, sold: bool) -> Self {
        let closed = closed_reason(capabilities.map(|c| c.analysis_progression), sold);
        Self {
            allowed: closed.is_none(),
            closed,
        }
    }

    /// Closed while the service has not said whether it sells the
    /// multi-night plans (unreachable, and never asked before).
    #[must_use]
    pub fn unknown() -> Self {
        Self {
            allowed: false,
            closed: Some(MultiNightClosed::Unknown),
        }
    }
}

/// Whether Astro Sync attaches a night to a multi-night plan, detaches it,
/// or creates a plan from it: these are gestures on the plan itself, so the
/// right is `planner.mosaic` ("Advanced Planning"), not
/// `analysis.progression`; always
/// open on a private server. Same shape as [`MultiNightGate`] on the
/// screens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachGate {
    pub allowed: bool,
    pub closed: Option<MultiNightClosed>,
}

impl AttachGate {
    /// `planner.mosaic` on a service that sells the multi-night plans;
    /// always open on a private server.
    #[must_use]
    pub fn new(capabilities: Option<&Capabilities>, sold: bool) -> Self {
        let closed = closed_reason(capabilities.map(|c| c.planner_mosaic), sold);
        Self {
            allowed: closed.is_none(),
            closed,
        }
    }

    /// Closed while the service has not said whether it sells the
    /// multi-night plans (unreachable, and never asked before).
    #[must_use]
    pub fn unknown() -> Self {
        Self {
            allowed: false,
            closed: Some(MultiNightClosed::Unknown),
        }
    }
}

/* ------------------------------ The service ------------------------------ */

/// One integration as the service lists it (`integration.schema.json`,
/// `integration`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceIntegration {
    #[serde(default)]
    pub id: String,
    pub night_id: String,
    pub target_id: String,
    #[serde(default)]
    pub filter: Option<String>,
    /// `0` for a declared line whose poses are not known (the Journal
    /// reads them with `frameCount` 0 and `exposureSeconds` 0,
    /// `total_seconds` being the time declared).
    #[serde(default)]
    pub exposure_seconds: f64,
    #[serde(default)]
    pub gain: Option<f64>,
    #[serde(default)]
    pub binning: Option<u32>,
    #[serde(default, rename = "temperatureC")]
    pub temperature_c: Option<f64>,
    #[serde(default)]
    pub frame_count: u64,
    #[serde(default)]
    pub total_seconds: f64,
    pub evening_date: String,
    #[serde(default)]
    pub sensor: Option<String>,
    #[serde(default)]
    pub computer_label: Option<String>,
    #[serde(default)]
    pub installation_id: Option<String>,
    /// `declared` (written by the service from the plan and the debriefing)
    /// or `measured` (written by an Astro Sync); absent = measured.
    #[serde(default)]
    pub source: Option<String>,
}

impl ServiceIntegration {
    /// A line the service declared without knowing a single pose: no frame,
    /// no exposure -- a promise of time, not an integration any computer
    /// holds, so there is nothing to combine nor to name a computer for.
    #[must_use]
    pub fn is_declared_without_frames(&self) -> bool {
        self.source.as_deref() == Some("declared")
            && self.frame_count == 0
            && self.exposure_seconds <= 0.0
    }

    /// The seconds integrated (the service's, else frames × exposure).
    #[must_use]
    pub fn seconds(&self) -> f64 {
        if self.total_seconds > 0.0 {
            self.total_seconds
        } else {
            self.frame_count as f64 * self.exposure_seconds
        }
    }
}

/// A multi-night plan a target of the night works for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NightProject {
    pub project_id: String,
    /// The night's target that works for it (its plan item).
    pub target_id: String,
    /// The plan's target as it names it (« IC 1805 »).
    pub name: String,
    pub common_name: Option<String>,
    /// The integrations the service knows, the earliest night first.
    pub integrations: Vec<ServiceIntegration>,
    pub total_seconds: f64,
    pub night_count: u64,
    /// The image of the plan already online (`planCapture`), as the service
    /// wrote it.
    pub plan_capture: Option<Value>,
}

/// The multi-night side of a night: its evening (the plan's date) and the
/// plans its targets work for.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NightMultiNight {
    /// The night's evening as the service dates it (the plan's `date`).
    pub evening_date: Option<String>,
    pub projects: Vec<NightProject>,
}

/// The service's routes of the multi-night plans, by a device token.
pub trait MultiNightTransport {
    fn plan(&self, base_url: &str, token: &str, plan_id: &str) -> Result<Value, GuiBridgeError>;
    fn project(
        &self,
        base_url: &str,
        token: &str,
        project_id: &str,
    ) -> Result<Value, GuiBridgeError>;
    fn projects(&self, base_url: &str, token: &str) -> Result<Value, GuiBridgeError>;
    /// `GET /v1/projects?sessionId=`: the multi-night plans a night belongs
    /// to, whether its plan's item names them or the night itself does.
    fn projects_of_night(
        &self,
        base_url: &str,
        token: &str,
        session_id: &str,
    ) -> Result<Value, GuiBridgeError>;
    /// `GET /v1/projects/{id}/nights`: the nights of a plan and the
    /// candidates that may join it.
    fn project_nights(
        &self,
        base_url: &str,
        token: &str,
        project_id: &str,
    ) -> Result<Value, GuiBridgeError>;
    /// `POST /v1/projects/{id}/nights` (`{ sessionIds }`): all or none.
    fn attach_nights(
        &self,
        base_url: &str,
        token: &str,
        project_id: &str,
        body: &Value,
    ) -> Result<Value, GuiBridgeError>;
    /// `DELETE /v1/projects/{id}/nights/{nightId}`: idempotent.
    fn detach_night(
        &self,
        base_url: &str,
        token: &str,
        project_id: &str,
        night_id: &str,
    ) -> Result<Value, GuiBridgeError>;
    /// `POST /v1/projects`: a multi-night plan created, "from" of the
    /// nights named by `fromSessionIds`.
    fn create_project(
        &self,
        base_url: &str,
        token: &str,
        body: &Value,
    ) -> Result<Value, GuiBridgeError>;
    fn put_integrations(
        &self,
        base_url: &str,
        token: &str,
        project_id: &str,
        body: &Value,
    ) -> Result<Value, GuiBridgeError>;
}

fn route(base_url: &str, path: &str) -> String {
    format!("{}/api{path}", base_url.trim_end_matches('/'))
}

impl MultiNightTransport for HttpPortalGuiTransport {
    fn plan(&self, base_url: &str, token: &str, plan_id: &str) -> Result<Value, GuiBridgeError> {
        let url = route(base_url, &format!("/v1/plans/{plan_id}"));
        let token = token.to_owned();
        let build: RequestFactory =
            Arc::new(move |client| client.get(url.clone()).bearer_auth(&token));
        self.read_json(&build)
    }

    fn project(
        &self,
        base_url: &str,
        token: &str,
        project_id: &str,
    ) -> Result<Value, GuiBridgeError> {
        let url = route(base_url, &format!("/v1/projects/{project_id}"));
        let token = token.to_owned();
        let build: RequestFactory =
            Arc::new(move |client| client.get(url.clone()).bearer_auth(&token));
        self.read_json(&build)
    }

    fn projects(&self, base_url: &str, token: &str) -> Result<Value, GuiBridgeError> {
        let url = route(base_url, "/v1/projects");
        let token = token.to_owned();
        let build: RequestFactory =
            Arc::new(move |client| client.get(url.clone()).bearer_auth(&token));
        self.read_json(&build)
    }

    fn projects_of_night(
        &self,
        base_url: &str,
        token: &str,
        session_id: &str,
    ) -> Result<Value, GuiBridgeError> {
        let url = route(base_url, "/v1/projects");
        let token = token.to_owned();
        let session_id = session_id.to_owned();
        let build: RequestFactory = Arc::new(move |client| {
            client
                .get(url.clone())
                .query(&[("sessionId", session_id.as_str())])
                .bearer_auth(&token)
        });
        self.read_json(&build)
    }

    fn project_nights(
        &self,
        base_url: &str,
        token: &str,
        project_id: &str,
    ) -> Result<Value, GuiBridgeError> {
        let url = route(base_url, &format!("/v1/projects/{project_id}/nights"));
        let token = token.to_owned();
        let build: RequestFactory =
            Arc::new(move |client| client.get(url.clone()).bearer_auth(&token));
        self.read_json(&build)
    }

    fn attach_nights(
        &self,
        base_url: &str,
        token: &str,
        project_id: &str,
        body: &Value,
    ) -> Result<Value, GuiBridgeError> {
        // All or none, and attaching a night already attached changes
        // nothing: harmless to repeat.
        let url = route(base_url, &format!("/v1/projects/{project_id}/nights"));
        let token = token.to_owned();
        let body = body.clone();
        let build: RequestFactory =
            Arc::new(move |client| client.post(url.clone()).bearer_auth(&token).json(&body));
        self.read_json(&build)
    }

    fn detach_night(
        &self,
        base_url: &str,
        token: &str,
        project_id: &str,
        night_id: &str,
    ) -> Result<Value, GuiBridgeError> {
        // Idempotent by the contract.
        let url = route(
            base_url,
            &format!("/v1/projects/{project_id}/nights/{night_id}"),
        );
        let token = token.to_owned();
        let build: RequestFactory =
            Arc::new(move |client| client.delete(url.clone()).bearer_auth(&token));
        self.read_json(&build)
    }

    fn create_project(
        &self,
        base_url: &str,
        token: &str,
        body: &Value,
    ) -> Result<Value, GuiBridgeError> {
        // A creation is sent once (the plan's `id` is fixed by the caller).
        let url = route(base_url, "/v1/projects");
        self.write_json_once(base_url, &|client| {
            client.post(url.clone()).bearer_auth(token).json(body)
        })
    }

    fn put_integrations(
        &self,
        base_url: &str,
        token: &str,
        project_id: &str,
        body: &Value,
    ) -> Result<Value, GuiBridgeError> {
        // A `PUT` of the same lines replaces them: harmless to repeat.
        let url = route(base_url, &format!("/v1/projects/{project_id}/integrations"));
        let token = token.to_owned();
        let body = body.clone();
        let build: RequestFactory =
            Arc::new(move |client| client.put(url.clone()).bearer_auth(&token).json(&body));
        self.read_json(&build)
    }
}

/// The plan of a project as the service answers it (`project.schema.json`,
/// `resource`): its name and the integrations it knows.
#[must_use]
pub fn project_of(project: &Value, target_id: &str) -> Option<NightProject> {
    let project_id = project.get("id")?.as_str()?.to_string();
    let target = project.get("target");
    let name = target
        .and_then(|t| t.get("name"))
        .and_then(Value::as_str)
        .unwrap_or(target_id)
        .to_string();
    let common_name = target
        .and_then(|t| t.get("commonName"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let integrations: Vec<ServiceIntegration> = project
        .get("integrations")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| serde_json::from_value(row.clone()).ok())
                .collect()
        })
        .unwrap_or_default();
    Some(project_summary(
        project_id,
        target_id.to_string(),
        name,
        common_name,
        integrations,
        project.get("planCapture").filter(|v| !v.is_null()).cloned(),
    ))
}

fn project_summary(
    project_id: String,
    target_id: String,
    name: String,
    common_name: Option<String>,
    mut integrations: Vec<ServiceIntegration>,
    plan_capture: Option<Value>,
) -> NightProject {
    integrations.sort_by(|a, b| {
        (&a.evening_date, &a.night_id, &a.filter).cmp(&(&b.evening_date, &b.night_id, &b.filter))
    });
    let total_seconds = integrations.iter().map(ServiceIntegration::seconds).sum();
    let nights: std::collections::BTreeSet<&str> =
        integrations.iter().map(|i| i.night_id.as_str()).collect();
    NightProject {
        project_id,
        target_id,
        name,
        common_name,
        night_count: nights.len() as u64,
        total_seconds,
        integrations,
        plan_capture,
    }
}

/// The (target, multi-night plan) pairs of a plan's items, in their order.
#[must_use]
pub fn plan_projects(plan: &Value) -> Vec<(String, String)> {
    let mut pairs: Vec<(String, String)> = Vec::new();
    let items = plan
        .pointer("/payload/items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for item in items {
        let target = item
            .get("targetId")
            .or_else(|| item.pointer("/target/id"))
            .and_then(Value::as_str);
        let project = item.get("projectId").and_then(Value::as_str);
        if let (Some(target), Some(project)) = (target, project) {
            let pair = (target.to_string(), project.to_string());
            if !pairs.contains(&pair) {
                pairs.push(pair);
            }
        }
    }
    pairs
}

/// The target a multi-night plan works for: the catalogue object
/// (`target.targetId`), else the saved framing (`target.framingId`).
#[must_use]
pub fn project_target_id(project: &Value) -> String {
    project
        .pointer("/target/targetId")
        .or_else(|| project.pointer("/target/framingId"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// The ids of the multi-night plans a night belongs to by itself
/// (`GET /v1/projects?sessionId=`: a night with no plan item of the plan's
/// target carries the link itself). A service that does not know the route
/// (an older one) says nothing: the night has none here.
fn own_project_ids<T: MultiNightTransport>(
    transport: &T,
    base_url: &str,
    token: &str,
    session_id: &str,
) -> Result<Vec<String>, GuiBridgeError> {
    let list = match transport.projects_of_night(base_url, token, session_id) {
        Ok(list) => list,
        Err(GuiBridgeError::Refused { .. }) => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    Ok(list
        .get("projects")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|p| p.get("id").and_then(Value::as_str).map(str::to_string))
                .collect()
        })
        .unwrap_or_default())
}

/// The multi-night side of a night: the plans its plan's items name
/// (`planItem.projectId`, `plan_id`) and those it belongs to by itself
/// (`session_id`: a night attached without a plan item of the target, or run
/// without a plan). A plan the service no longer knows is left out, never an
/// error.
pub fn night_projects<T: MultiNightTransport>(
    transport: &T,
    base_url: &str,
    token: &str,
    plan_id: Option<&str>,
    session_id: Option<&str>,
) -> Result<NightMultiNight, GuiBridgeError> {
    let mut evening_date = None;
    let mut pairs: Vec<(String, String)> = Vec::new();
    if let Some(plan_id) = plan_id {
        let plan = transport.plan(base_url, token, plan_id)?;
        evening_date = plan
            .pointer("/payload/date")
            .and_then(Value::as_str)
            .map(str::to_string);
        pairs = plan_projects(&plan);
    }
    let mut projects = Vec::new();
    let mut known: Vec<String> = Vec::new();
    for (target_id, project_id) in pairs {
        known.push(project_id.clone());
        match transport.project(base_url, token, &project_id) {
            Ok(project) => projects.extend(project_of(&project, &target_id)),
            Err(GuiBridgeError::Refused { .. }) => {}
            Err(error) => return Err(error),
        }
    }
    if let Some(session_id) = session_id {
        for project_id in own_project_ids(transport, base_url, token, session_id)? {
            if known.contains(&project_id) {
                continue;
            }
            match transport.project(base_url, token, &project_id) {
                Ok(project) => {
                    let target_id = project_target_id(&project);
                    projects.extend(project_of(&project, &target_id));
                }
                Err(GuiBridgeError::Refused { .. }) => {}
                Err(error) => return Err(error),
            }
        }
    }
    Ok(NightMultiNight {
        evening_date,
        projects,
    })
}

/// What became of the lines of one multi-night plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "result",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum IntegrationsSent {
    /// The service took them: the plan as it now stands.
    Sent {
        project_id: String,
        lines: u64,
        project: NightProject,
    },
    /// Nothing sent: the right is missing here (said before any request).
    Closed {
        project_id: String,
        closed: MultiNightClosed,
    },
    /// No master of the night for the plan's target: nothing to send.
    Nothing { project_id: String },
    /// The service refused (`multi_night_not_active`,
    /// `integration_outside_night`, `project_not_found`…): its code and
    /// words, shown as they are.
    Refused {
        project_id: String,
        code: String,
        message: String,
    },
}

/// The most lines one `PUT` carries.
const MAX_LINES_PER_SENDING: usize = 200;

/// The body of a `PUT …/integrations` that sends `lines` complete: the
/// (night, target) sets they belong to are named in `replace`, so the service
/// removes the lines of those sets that are not among them any more. `None`
/// when they do not fit one `PUT`: a truncated set must never be sent as a
/// complete one.
fn complete_body(lines: &[IntegrationLine]) -> Option<Value> {
    if lines.len() > MAX_LINES_PER_SENDING {
        return None;
    }
    let mut sets: Vec<Value> = Vec::new();
    for line in lines {
        let set = json!({ "nightId": line.night_id, "targetId": line.target_id });
        if !sets.contains(&set) {
            sets.push(set);
        }
    }
    Some(json!({ "integrations": lines, "replace": sets }))
}

/// Sends the lines of one multi-night plan (`PUT …/integrations`), when the
/// gate allows it. The lines are the **complete** set of their night and
/// target (`replace`, api/v1.md « Integrations of a multi-night plan »): what
/// the plan holds for them and is not among them is removed. A service that
/// predates `replace` refuses the body (`400 invalid_request`): the lines are
/// then sent again in the earlier form, which removes nothing. More than 200
/// lines are sent in that form too (the first 200), never as a complete set.
///
/// An empty `lines` sends nothing ([`IntegrationsSent::Nothing`]) even though
/// the service accepts an empty set: a master folder that cannot be read (a
/// disk that is not mounted) is not a night without masters, and would
/// otherwise wipe the plan.
pub fn send_integrations<T: MultiNightTransport>(
    transport: &T,
    base_url: &str,
    token: &str,
    project: &NightProject,
    lines: &[IntegrationLine],
    gate: MultiNightGate,
) -> Result<IntegrationsSent, GuiBridgeError> {
    let project_id = project.project_id.clone();
    if let Some(closed) = gate.closed {
        return Ok(IntegrationsSent::Closed { project_id, closed });
    }
    if lines.is_empty() {
        return Ok(IntegrationsSent::Nothing { project_id });
    }
    let earlier =
        json!({ "integrations": lines.iter().take(MAX_LINES_PER_SENDING).collect::<Vec<_>>() });
    let answer = match complete_body(lines) {
        Some(body) => match transport.put_integrations(base_url, token, &project_id, &body) {
            // A service that predates `replace` refuses the property.
            Err(GuiBridgeError::Refused { code, .. }) if code == "invalid_request" => {
                transport.put_integrations(base_url, token, &project_id, &earlier)
            }
            other => other,
        },
        None => transport.put_integrations(base_url, token, &project_id, &earlier),
    };
    match answer {
        Ok(answer) => {
            let integrations: Vec<ServiceIntegration> = answer
                .get("integrations")
                .and_then(Value::as_array)
                .map(|rows| {
                    rows.iter()
                        .filter_map(|row| serde_json::from_value(row.clone()).ok())
                        .collect()
                })
                .unwrap_or_default();
            Ok(IntegrationsSent::Sent {
                project_id: project_id.clone(),
                lines: lines.len().min(MAX_LINES_PER_SENDING) as u64,
                project: project_summary(
                    project_id,
                    project.target_id.clone(),
                    project.name.clone(),
                    project.common_name.clone(),
                    integrations,
                    project.plan_capture.clone(),
                ),
            })
        }
        Err(GuiBridgeError::Refused { code, message }) => Ok(IntegrationsSent::Refused {
            project_id,
            code,
            message,
        }),
        Err(error) => Err(error),
    }
}

/* -------------------------------- The tags ------------------------------- */

/// The tag of a night of a multi-night plan in "Nights".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanTag {
    pub project_id: String,
    pub name: String,
    pub night_count: u64,
    pub total_seconds: f64,
}

/// The tags of the nights, by night: every multi-night plan of the space
/// (`GET /v1/projects`), each read with its integrations and with the nights
/// attached to it (`GET /v1/projects/{id}/nights`: a night attached by itself
/// has no integration yet and is tagged all the same). A plan without a night
/// tags none.
pub fn plan_tags<T: MultiNightTransport>(
    transport: &T,
    base_url: &str,
    token: &str,
) -> Result<BTreeMap<String, Vec<PlanTag>>, GuiBridgeError> {
    let list = transport.projects(base_url, token)?;
    let mut tags: BTreeMap<String, Vec<PlanTag>> = BTreeMap::new();
    let ids: Vec<String> = list
        .get("projects")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|p| p.get("id").and_then(Value::as_str).map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    for id in ids {
        let Ok(project) = transport.project(base_url, token, &id) else {
            continue;
        };
        let Some(summary) = project_of(&project, "") else {
            continue;
        };
        // The nights run (a `sessionId`), attached since or planned from the
        // plan; best effort, like the project's own read.
        let attached: Vec<String> = transport
            .project_nights(base_url, token, &id)
            .ok()
            .and_then(|answer| answer.get("nights").and_then(Value::as_array).cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|night| {
                night
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .collect();
        let mut night_ids: std::collections::BTreeSet<&str> = summary
            .integrations
            .iter()
            .map(|i| i.night_id.as_str())
            .collect();
        night_ids.extend(attached.iter().map(String::as_str));
        let tag = PlanTag {
            project_id: summary.project_id.clone(),
            name: summary.name.clone(),
            night_count: night_ids.len() as u64,
            total_seconds: summary.total_seconds,
        };
        for night_id in night_ids {
            let night = tags.entry(night_id.to_string()).or_default();
            if !night.iter().any(|t| t.project_id == tag.project_id) {
                night.push(tag.clone());
            }
        }
    }
    Ok(tags)
}

#[cfg(test)]
pub(crate) mod tests;
