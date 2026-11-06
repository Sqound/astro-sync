//! The image of a multi-night plan: Astro Sync combines the masters kept of the
//! plan's nights into one image, on this computer, and sends it as a capture
//! of the plan.
//!
//! - **What can be combined** ([`plan_lines`]): every integration the
//!   service knows for the plan ([`crate::multi_night::NightProject`]) and
//!   every master kept here for the plan's target, found by identity in the
//!   night folders of the working folder (`.astro-sync-night.json`, then
//!   `.astro-sync-target.json`). An integration whose master is not here is
//!   "on another computer, not combined" ([`Place::OtherComputer`]):
//!   listed, never combined, never fetched (no master goes through the
//!   service).
//! - **The image of a device**: a night of which
//!   only the device's own stack is here (a Seestar's `Stacked_*.fit` in the
//!   target's `device/`) is a line of the plan too ([`PlanLine::device`]);
//!   a night with a kept master and a device image is a line with both, the
//!   master by default ([`PlanSource`]): poses on this computer give a finer
//!   image than the device's own, which is the choice offered beside it. The
//!   job names the source of each master and `plan_stack.py` converts a
//!   device image (32-bit floats, divided by 65 535) before aligning.
//! - **The frame** ([`PlanFrame`], [`plan_framing`]): the image is
//!   always made whole and kept in `stacked/`; the notches of the frame the
//!   person may keep are read from the record of the last image made with the
//!   same integrations, and the job carries the four integers of the one
//!   chosen: `plan_stack.py` cuts the rendered image after the render, nothing
//!   is stacked again, and the cut one is what leaves.
//! - **The job** ([`plan_image_job`]): the lines the person ticked; the
//!   reference night is the one with the longest integration (the longest
//!   dual-band one when the plan mixes both, [`reference_of`]); **one night
//!   only**: the image of the plan is that night's image, nothing is made
//!   and no capture of the plan is created ([`PlanImagePlan::OneNight`]).
//!   The work happens in the plan's own folder, created by Astro Sync in the
//!   working folder and named after the plan ([`plan_folder`],
//!   `.astro-sync-plan.json` its identity): the renders in `stacked/`, the
//!   scratch in `.astro-sync-work/plan-image/`, removed at the end.
//! - **The making** ([`PythonPlanImager`], `plan_stack.py`): Siril aligns
//!   the masters on the reference night (`register -2pass
//!   -transf=affine`), then `plan_combine.py` matches each master to the
//!   longest of its band and averages them weighted by their duration. A
//!   plan of a colour sensor with a dual-band integration follows the
//!   original pipeline instead (`dualband_reference.py`, the same image to
//!   the bit): the masters of each band registered from their stars and
//!   averaged, then the broadband one added to the dual-band one at 0.18;
//!   the renders chosen run on the result. The
//!   progress is the stacking's: the same events ([`StackEvent`]), Siril's
//!   journal followed.
//! - **The pipelines** ([`plan_image_jobs`]): the image of a plan
//!   is made with the pipelines chosen for the plan's target (as "Processing"
//!   leaves them, one image per pipeline: the first is the capture of the
//!   plan, the others its variants, [`plan_results_of`]);
//!   without a choice the target's defaults ([`default_plan_pipelines`]:
//!   "Natural render", and "Dual-band correction" as soon as a dual-band
//!   integration is combined). The masters are combined once per pipeline.
//! - **What leaves** ([`plan_results`], [`plan_variant_result`]): the main
//!   render as a capture of the plan (`role: "plan"`, `projectId`, `combinedNightIds`,
//!   `totalsByFilter`, `planRender`; [`ResultKind::Plan`]), behind the
//!   right of the multi-night plans ([`crate::multi_night::MultiNightGate`]):
//!   without it nothing is made.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::analysis::fits::read_primary_header_file;
use crate::analysis::sky::sensor_pixel_scale;
use crate::local_work::CancelFlag;
use crate::metadata::{
    GeneratedText, ImageQuality, MeasuredQuality, ProcessingProposal, Proposals,
};
use crate::multi_night::{
    DeviceSource, IntegrationMaster, NightProject, ServiceIntegration, same_acquisition,
    target_device_images, target_masters,
};
use crate::pipelines::{DEFAULT_PIPELINE, LINEAR_PIPELINE};
use crate::python_env;
use crate::stacking::{
    PipelineChoice, StackEvent, StackFailure, StackedRender, StackerOutput, run_script,
};
use crate::upload_queue::{EntryTarget, PlanFilterTotal, PlanUpload, ResultFile, ResultKind};
use crate::working_folder::{
    NAME_SEPARATOR, STACKED_DIR, WORK_DIR, clean_component, list_night_folders,
};

/* -------------------------------- The bands ------------------------------ */

/// What a filter lets through, as the fusion reads it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Band {
    /// A broadband filter (none, UV/IR cut, RGB, light-pollution « L-Pro »…).
    #[default]
    Broadband,
    /// A dual-band filter of a colour sensor (Hα + OIII: « L-eXtreme »,
    /// « ALP-T », « NBZ »…): its red is Hα, its green and blue OIII.
    Dualband,
}

/// The words of a dual-band filter's name (letters and digits only, lower
/// case): the names the makers give them, and « dual »/« bibande ».
pub const DUALBAND_WORDS: [&str; 13] = [
    "dual", "duo", "bibande", "extreme", "enhance", "ultimate", "alpt", "nbz", "hao3", "haoiii",
    "hoo", "triad", "quadband",
];

/// The band of a filter name ([`DUALBAND_WORDS`]; anything else, and no
/// filter, is broadband).
#[must_use]
pub fn band_of(filter: Option<&str>) -> Band {
    let name: String = filter
        .unwrap_or_default()
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_ascii_lowercase();
    if DUALBAND_WORDS.iter().any(|word| name.contains(word)) {
        Band::Dualband
    } else {
        Band::Broadband
    }
}

/* -------------------------------- The lines ------------------------------ */

/// Where an integration of the plan is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Place {
    /// The night being sent.
    ThisNight,
    /// Another night of the plan, its master on this computer.
    ThisComputer,
    /// Known to the service, its master on another computer: never combined.
    OtherComputer,
}

/// What a night's integration is made from: the master Astro Sync kept of its poses, or the image its device
/// stacked.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanSource {
    /// `<target>/stacked/<family>_integration.fit`, the stacking of the poses.
    #[default]
    Master,
    /// `<target>/device/Stacked_*.fit`, the device's own stack.
    Device,
}

impl PlanSource {
    // `serde`'s `skip_serializing_if` hands a reference.
    #[allow(clippy::trivially_copy_pass_by_ref)]
    fn is_master(&self) -> bool {
        *self == Self::Master
    }
}

/// The image a device stacked for a night of the plan, as a line carries it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceCandidate {
    pub path: PathBuf,
    /// « Seestar S50 Pro ».
    pub device: String,
    /// The poses it stacked (`STACKCNT`).
    pub frame_count: u64,
    pub total_seconds: f64,
    pub bytes: u64,
}

impl From<&DeviceSource> for DeviceCandidate {
    fn from(source: &DeviceSource) -> Self {
        Self {
            path: source.path.clone(),
            device: source.device.clone(),
            frame_count: source.frame_count,
            total_seconds: source.total_seconds,
            bytes: source.bytes,
        }
    }
}

/// One integration of the plan, as "Image of the multi-night plan" lists it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanLine {
    /// `<night id>|<family>`: what the person ticks.
    pub key: String,
    pub night_id: String,
    pub evening_date: String,
    pub filter: Option<String>,
    pub band: Band,
    pub frame_count: u64,
    pub exposure_seconds: Option<f64>,
    pub total_seconds: f64,
    pub place: Place,
    /// The computer the service names for it ("Laptop").
    pub computer_label: Option<String>,
    /// The master kept here, and its size.
    pub master: Option<PathBuf>,
    pub bytes: u64,
    /// What the job takes when the person chose nothing: the master when one is kept, else the image
    /// of the device.
    #[serde(default)]
    pub source: PlanSource,
    /// The image the device stacked for this night and family, when it is here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<DeviceCandidate>,
}

impl PlanLine {
    /// Something here can be combined: a master kept, or the image of the device.
    #[must_use]
    pub fn combinable(&self) -> bool {
        self.place != Place::OtherComputer && (self.master.is_some() || self.device.is_some())
    }

    /// The source the job takes for this line: the one asked when it is here, else the default.
    #[must_use]
    pub fn source_for(&self, asked: Option<PlanSource>) -> PlanSource {
        match asked {
            Some(PlanSource::Device) if self.device.is_some() => PlanSource::Device,
            Some(PlanSource::Master) if self.master.is_some() => PlanSource::Master,
            _ => self.source,
        }
    }

    /// The line as it would be with `source` taken: its totals and its file are the source's.
    fn taking(&self, source: PlanSource) -> (Self, PathBuf) {
        let mut line = self.clone();
        let path = match (source, &self.device) {
            (PlanSource::Device, Some(device)) => {
                line.frame_count = device.frame_count;
                line.total_seconds = device.total_seconds;
                line.bytes = device.bytes;
                device.path.clone()
            }
            _ => self.master.clone().unwrap_or_default(),
        };
        line.source = source;
        (line, path)
    }
}

/// The night being sent, to tell it from the others.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentNight {
    pub night_id: String,
    pub evening_date: String,
    pub folder: PathBuf,
}

fn same_family(master: &IntegrationMaster, line: &ServiceIntegration) -> bool {
    let close = |a: Option<f64>, b: Option<f64>| match (a, b) {
        (Some(a), Some(b)) => (a - b).abs() < 1e-3,
        (None, None) => true,
        _ => false,
    };
    master.filter == line.filter
        && close(master.exposure_seconds, Some(line.exposure_seconds))
        && close(master.gain.map(f64::round), line.gain.map(f64::round))
        && master.binning.unwrap_or(1) == line.binning.unwrap_or(1)
}

/// The evening a night folder is named by (`YYYY-MM-DD · …`).
fn evening_of_folder(name: &str) -> String {
    name.split(NAME_SEPARATOR)
        .next()
        .unwrap_or_default()
        .to_string()
}

fn line_of(
    night_id: &str,
    evening_date: &str,
    master: &IntegrationMaster,
    device: Option<&DeviceSource>,
    place: Place,
) -> PlanLine {
    PlanLine {
        key: format!("{night_id}|{}", master.family),
        night_id: night_id.to_string(),
        evening_date: evening_date.to_string(),
        filter: master.filter.clone(),
        band: band_of(master.filter.as_deref()),
        frame_count: master.frame_count,
        exposure_seconds: master.exposure_seconds,
        total_seconds: master.total_seconds,
        place,
        computer_label: None,
        master: Some(master.path.clone()),
        bytes: master.bytes,
        source: PlanSource::Master,
        device: device.map(DeviceCandidate::from),
    }
}

/// A line whose only source is the image its device stacked.
fn device_line(
    night_id: &str,
    evening_date: &str,
    device: &DeviceSource,
    place: Place,
) -> PlanLine {
    PlanLine {
        key: format!("{night_id}|{}", device.family),
        night_id: night_id.to_string(),
        evening_date: evening_date.to_string(),
        filter: device.filter.clone(),
        band: band_of(device.filter.as_deref()),
        frame_count: device.frame_count,
        exposure_seconds: device.exposure_seconds,
        total_seconds: device.total_seconds,
        place,
        computer_label: None,
        master: None,
        bytes: device.bytes,
        source: PlanSource::Device,
        device: Some(DeviceCandidate::from(device)),
    }
}

/// What a night of the plan has here for the target: the masters kept and the images its device stacked.
type NightHere = (String, Vec<IntegrationMaster>, Vec<DeviceSource>);

fn device_matches(device: &DeviceSource, integration: &ServiceIntegration) -> bool {
    same_acquisition(
        (
            device.filter.as_deref(),
            device.exposure_seconds,
            device.gain,
        ),
        (
            integration.filter.as_deref(),
            Some(integration.exposure_seconds),
            integration.gain,
        ),
    )
}

/// The integrations of the plan: those the service knows, each found here
/// or not, and the masters kept here the service does not know yet (the
/// night being sent), the earliest night first. A night's integration is
/// found as the master Astro Sync kept of its poses, as the image its device
/// stacked, or as both (the master is the default source).
#[must_use]
pub fn plan_lines(
    root: &Path,
    project: &NightProject,
    current: Option<&CurrentNight>,
) -> Vec<PlanLine> {
    lines_of(root, project, current, &[])
}

/// The integrations of a multi-night plan the space does not hold yet: the
/// import's review proposed it, its nights are the import's own night
/// folders (`nights`, by their provisional identifier), and the masters
/// kept there are its lines, the earliest night first. Nothing is asked of
/// the service.
#[must_use]
pub fn local_plan_lines(root: &Path, project: &NightProject, nights: &[String]) -> Vec<PlanLine> {
    lines_of(root, project, None, nights)
}

fn lines_of(
    root: &Path,
    project: &NightProject,
    current: Option<&CurrentNight>,
    local: &[String],
) -> Vec<PlanLine> {
    let known: Vec<&str> = project
        .integrations
        .iter()
        .map(|i| i.night_id.as_str())
        .chain(local.iter().map(String::as_str))
        .collect();
    // The masters and the device images kept here, by night.
    let mut here: BTreeMap<String, NightHere> = BTreeMap::new();
    for entry in list_night_folders(root).unwrap_or_default() {
        let is_current = current.is_some_and(|c| c.night_id == entry.night_id);
        if !is_current && !known.contains(&entry.night_id.as_str()) {
            continue;
        }
        here.insert(
            entry.night_id.clone(),
            (
                evening_of_folder(&entry.name),
                target_masters(&entry.folder, &project.target_id),
                target_device_images(&entry.folder, &project.target_id),
            ),
        );
    }
    if let Some(current) = current {
        here.entry(current.night_id.clone()).or_insert_with(|| {
            (
                current.evening_date.clone(),
                target_masters(&current.folder, &project.target_id),
                target_device_images(&current.folder, &project.target_id),
            )
        });
    }
    let place_of = |night_id: &str| {
        if current.is_some_and(|c| c.night_id == night_id) {
            Place::ThisNight
        } else {
            Place::ThisComputer
        }
    };
    let mut used: Vec<PathBuf> = Vec::new();
    let mut used_devices: Vec<PathBuf> = Vec::new();
    let mut lines = Vec::new();
    for integration in project
        .integrations
        .iter()
        .filter(|i| !i.is_declared_without_frames())
    {
        let mine = here.get(&integration.night_id);
        let found = mine.and_then(|(_, masters, _)| {
            masters
                .iter()
                .find(|m| same_family(m, integration) && !used.contains(&m.path))
        });
        let device = mine.and_then(|(_, _, devices)| {
            devices
                .iter()
                .find(|d| device_matches(d, integration) && !used_devices.contains(&d.path))
        });
        let place = place_of(&integration.night_id);
        let made = match (found, device) {
            (Some(master), device) => {
                used.push(master.path.clone());
                Some(line_of(
                    &integration.night_id,
                    &integration.evening_date,
                    master,
                    device,
                    place,
                ))
            }
            (None, Some(device)) => Some(device_line(
                &integration.night_id,
                &integration.evening_date,
                device,
                place,
            )),
            (None, None) => None,
        };
        if let Some(device) = device {
            used_devices.push(device.path.clone());
        }
        match made {
            Some(mut line) => {
                line.computer_label = integration.computer_label.clone();
                lines.push(line);
            }
            None => lines.push(PlanLine {
                key: format!(
                    "{}|{}",
                    integration.night_id,
                    integration.filter.as_deref().unwrap_or("nofilter")
                ),
                night_id: integration.night_id.clone(),
                evening_date: integration.evening_date.clone(),
                filter: integration.filter.clone(),
                band: band_of(integration.filter.as_deref()),
                frame_count: integration.frame_count,
                exposure_seconds: Some(integration.exposure_seconds),
                total_seconds: integration.seconds(),
                place: Place::OtherComputer,
                computer_label: integration.computer_label.clone(),
                master: None,
                bytes: 0,
                source: PlanSource::Master,
                device: None,
            }),
        }
    }
    for (night_id, (evening, masters, devices)) in &here {
        for master in masters.iter().filter(|m| !used.contains(&m.path)) {
            let device = devices
                .iter()
                .find(|d| d.same_family_as(master) && !used_devices.contains(&d.path));
            if let Some(device) = device {
                used_devices.push(device.path.clone());
            }
            lines.push(line_of(
                night_id,
                evening,
                master,
                device,
                place_of(night_id),
            ));
        }
        for device in devices.iter().filter(|d| !used_devices.contains(&d.path)) {
            lines.push(device_line(night_id, evening, device, place_of(night_id)));
        }
    }
    lines.sort_by(|a, b| {
        (&a.evening_date, &a.night_id, &a.filter).cmp(&(&b.evening_date, &b.night_id, &b.filter))
    });
    lines
}

/// The reference of a set of lines: the longest integration (its night is
/// the one the others are aligned on). When the plan mixes dual-band and
/// broadband integrations, the reference is the longest **dual-band** one:
/// the original pipeline keeps the dual-band master as the structure of the
/// image and adds the broadband one to it (`dualband_reference.py`), so its
/// frame is the frame of the image of the plan; what the broadband nights do
/// not reach is left black. Without that mix (broadband only, dual-band only)
/// it is the longest one. This is the one place the reference is chosen.
#[must_use]
pub fn reference_of(lines: &[&PlanLine]) -> Option<usize> {
    let dual = lines.iter().any(|l| l.band == Band::Dualband);
    let broad = lines.iter().any(|l| l.band != Band::Dualband);
    let mixed = dual && broad;
    lines
        .iter()
        .enumerate()
        .filter(|(_, line)| !mixed || line.band == Band::Dualband)
        .max_by(|a, b| {
            a.1.total_seconds
                .total_cmp(&b.1.total_seconds)
                .then(b.1.evening_date.cmp(&a.1.evening_date))
        })
        .map(|(index, _)| index)
}

/// Each filter's total over the lines, in the order the filters come.
#[must_use]
pub fn totals_by_filter(lines: &[&PlanLine]) -> Vec<PlanFilterTotal> {
    let mut totals: Vec<PlanFilterTotal> = Vec::new();
    for line in lines {
        match totals.iter_mut().find(|t| t.filter == line.filter) {
            Some(total) => {
                total.frame_count += line.frame_count;
                total.total_seconds += line.total_seconds;
            }
            None => totals.push(PlanFilterTotal {
                filter: line.filter.clone(),
                frame_count: line.frame_count,
                total_seconds: line.total_seconds,
            }),
        }
    }
    totals
}

/* ------------------------------ The plan folder ---------------------------- */

/// The identity of a plan's folder.
pub const PLAN_IDENTITY_FILE: &str = ".astro-sync-plan.json";

/// The scratch of the image of a plan, under `.astro-sync-work/`.
pub const PLAN_SCRATCH_DIR: &str = "plan-image";

/// `.astro-sync-plan.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanIdentity {
    pub format: u32,
    pub project_id: String,
    pub target_id: String,
    pub name: String,
    /// A plan the import proposed before the space created it: `project_id`
    /// is provisional. Absent from every other plan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<PendingPlan>,
}

/// A multi-night plan the import's review proposed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingPlan {
    /// The plan the space created: `null` until the « Récapitulatif » of the
    /// import creates it; set before the folder is renamed.
    pub pending_project_id: Option<String>,
    /// The folder as it was named when the plan was created.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<PathBuf>,
    /// The proposal's id of the group of nights (`MultiNightGroup::id`).
    pub group_id: String,
}

/// Rewrites a plan folder's identity, beside it then renamed over it.
pub fn write_plan_identity(folder: &Path, identity: &PlanIdentity) -> std::io::Result<()> {
    let path = folder.join(PLAN_IDENTITY_FILE);
    let beside = folder.join(format!("{PLAN_IDENTITY_FILE}.tmp"));
    fs::write(
        &beside,
        serde_json::to_string_pretty(identity).map_err(std::io::Error::other)? + "\n",
    )?;
    fs::rename(&beside, &path)
}

/// Reads a plan folder's identity.
#[must_use]
pub fn read_plan_identity(folder: &Path) -> Option<PlanIdentity> {
    let text = fs::read_to_string(folder.join(PLAN_IDENTITY_FILE)).ok()?;
    serde_json::from_str(&text).ok()
}

/// The name of a plan's folder: the plan's name (its target, « IC 1805 »)
/// and the short identifier of the plan.
#[must_use]
pub fn plan_folder_name(name: &str, project_id: &str) -> String {
    let short = crate::working_folder::short_id(project_id);
    format!(
        "{}{NAME_SEPARATOR}{}",
        clean_component(name),
        clean_component(&short)
    )
}

/// The folder of a plan in the working folder, found by its identity, whatever it is called; `None` while
/// the plan's image was never made here (nothing is created).
#[must_use]
pub fn find_plan_folder(root: &Path, project_id: &str) -> Option<PathBuf> {
    fs::read_dir(root)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.is_dir() && read_plan_identity(path).is_some_and(|id| id.project_id == project_id)
        })
}

/// The folder of the plan in the working folder, found by its identity, or
/// created (with its identity) the first time.
pub fn plan_folder(root: &Path, project: &NightProject) -> std::io::Result<PathBuf> {
    if let Some(found) = find_plan_folder(root, &project.project_id) {
        return Ok(found);
    }
    let folder = root.join(plan_folder_name(&project.name, &project.project_id));
    fs::create_dir_all(&folder)?;
    let identity = PlanIdentity {
        format: 1,
        project_id: project.project_id.clone(),
        target_id: project.target_id.clone(),
        name: project.name.clone(),
        pending: None,
    };
    fs::write(
        folder.join(PLAN_IDENTITY_FILE),
        serde_json::to_string_pretty(&identity).map_err(std::io::Error::other)? + "\n",
    )?;
    Ok(folder)
}

/* --------------------------------- The job -------------------------------- */

/// One master combined.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanJobMaster {
    pub path: PathBuf,
    pub night_id: String,
    pub evening_date: String,
    pub filter: Option<String>,
    pub band: Band,
    pub frame_count: u64,
    pub total_seconds: f64,
    /// Where the file comes from: a master Astro Sync kept (the default, not written), or the image a
    /// device stacked, which `plan_stack.py` converts to a master before aligning.
    #[serde(default, skip_serializing_if = "PlanSource::is_master")]
    pub source: PlanSource,
}

/// The frame of the image of a plan the person keeps: four integers of the whole
/// image, a notch of the slider (`framings` of the record of an earlier image).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanFrame {
    pub top: u32,
    pub left: u32,
    pub height: u32,
    pub width: u32,
}

/// The rectangle the image of a plan was cut to, on the frame of its master
/// (`crop` of the record, `plan_crop.py`): the largest axis-aligned rectangle
/// every night covers. A milestone of the progression is given it, so that it
/// is cut the same way as the image of the plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanCrop {
    pub top: u32,
    pub left: u32,
    pub height: u32,
    pub width: u32,
    pub frame_height: u32,
    pub frame_width: u32,
}

/// The master whose frame the image of a plan is on (`frameMaster` of the
/// record): the longest dual-band one, or the reference. A milestone whose
/// own nights give it another frame is registered on this one first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanFrameMaster {
    pub path: PathBuf,
    #[serde(default, skip_serializing_if = "PlanSource::is_master")]
    pub source: PlanSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub night_id: Option<String>,
}

/// The frame the image of a plan is cut to (« Image du plan multi-nuits »,
/// Settings › General; `planField` of the job, `plan_crop.py`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanField {
    /// « Bords nets » (the default): the largest axis-aligned rectangle
    /// without a pixel no night covers; the parts fewer nights cover stay.
    #[default]
    Clean,
    /// « Zone commune à toutes les nuits »: the largest rectangle every night
    /// covers (one exposure everywhere, a smaller field).
    Common,
    /// « Tout le champ »: nothing is cut (irregular edges).
    Whole,
}

impl PlanField {
    /// The default, not written in a job.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        *self == PlanField::Clean
    }
}

/// What the person chose beyond the integrations ticked: which source each line takes, and the frame kept.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlanMaking {
    /// The source asked for a line (its key); a line not named, or whose source is not here, takes its default.
    pub sources: BTreeMap<String, PlanSource>,
    /// The frame kept; `None` (or the whole image) keeps the whole image.
    pub frame: Option<PlanFrame>,
    /// « Image du plan multi-nuits » (Settings › General): the frame the image
    /// and its milestones are cut to ([`PlanImageJob::plan_field`]).
    pub plan_field: PlanField,
}

/// One image of a plan to make: what `plan_stack.py` reads (`--job`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanImageJob {
    pub format: u32,
    pub project_id: String,
    pub target_id: String,
    pub plan_dir: PathBuf,
    /// `<plan>/stacked/`.
    pub out: PathBuf,
    /// `<plan>/.astro-sync-work/plan-image/`.
    pub work: PathBuf,
    /// The stem of the files (`IC_1805_plan`).
    pub name: String,
    pub masters: Vec<PlanJobMaster>,
    /// The index of the reference master ([`reference_of`]).
    pub reference: usize,
    /// The frame to cut from the whole image once it is rendered; `None`: the whole image leaves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<PlanFrame>,
    /// The frame the image is cut to: clean edges (the default, not written),
    /// the area every night covers, or the whole field.
    #[serde(default, skip_serializing_if = "PlanField::is_clean")]
    pub plan_field: PlanField,
    /// A milestone of the progression: the rectangle of the image of the plan
    /// to cut, the same for every milestone ([`crate::plan_progression`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crop: Option<PlanCrop>,
    /// A milestone of the progression: the master whose frame the image of
    /// the plan is on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_master: Option<PlanFrameMaster>,
    /// The render chosen: one pipeline, or "No processing" ([`LINEAR_PIPELINE`]).
    pub pipeline: PipelineChoice,
    /// The contrast of presentation: the image is also written with
    /// the curve (`<name>_<id>_presentation.tif`); the plain one stays the
    /// image of the plan.
    #[serde(default, skip_serializing_if = "crate::stacking::Contrast::is_none")]
    pub contrast: crate::stacking::Contrast,
    /// The colour intensity and the softened stars of that same image (the
    /// finish beside the contrast; the extra image is written when any is asked).
    #[serde(default, flatten)]
    pub retouch: crate::stacking::Retouch,
    pub pipelines_dirs: Vec<PathBuf>,
    pub siril: Option<PathBuf>,
    pub starnet: Option<PathBuf>,
    /// GraXpert for the "polished render": `graxpert` of the job file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graxpert: Option<PathBuf>,
    /// The pipeline's own name (English) for a pipeline the service has no
    /// word for -- one the person added --, kept for the processing's steps;
    /// never part of the job file.
    #[serde(skip)]
    pub pipeline_label: Option<String>,
    /// The title and the description of the render in the ten languages
    /// (the service's `variantGeneratedTitle` / `variantGeneratedDescription`,
    /// [`crate::metadata::generated_render_texts`]); never part of the job
    /// file.
    #[serde(skip)]
    pub render_texts: (Option<GeneratedText>, Option<GeneratedText>),
    /// Also write the image as a JPEG (the milestone of the progression of a
    /// plan: light, where the PNG of the same image weighs fifteen megabytes).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub jpeg: bool,
}

impl PlanImageJob {
    /// The finish of the image: the one the pipeline was chosen with (the
    /// contrast of the wizard, a slider value included) when it has one,
    /// else `default` (the settings of « Traitements »). Same rule as the
    /// night's `--finish`.
    pub fn apply_finish(&mut self, default: crate::stacking::PipelineFinish) {
        let finish = self.pipeline.finish.filter(|_| !self.pipeline.is_linear());
        let finish = finish.unwrap_or(default);
        self.contrast = finish.contrast;
        self.retouch = finish.retouch;
    }

    /// The job file beside the scratch (`.astro-sync-work/plan-image.job.json`).
    #[must_use]
    pub fn job_file(&self) -> PathBuf {
        self.work.with_extension("job.json")
    }

    /// The band of each master, in the order they are combined.
    #[cfg(test)]
    pub(crate) fn band_of_masters(&self) -> Vec<Band> {
        self.masters.iter().map(|m| m.band).collect()
    }

    /// The nights combined, the earliest first.
    #[must_use]
    pub fn night_ids(&self) -> Vec<String> {
        let mut nights: Vec<(&str, &str)> = self
            .masters
            .iter()
            .map(|m| (m.evening_date.as_str(), m.night_id.as_str()))
            .collect();
        nights.sort_unstable();
        nights.dedup();
        nights.into_iter().map(|(_, id)| id.to_string()).collect()
    }
}

/// What the person's choice gives.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum PlanImagePlan {
    /// Two nights or more: an image to make.
    Job(Box<PlanImageJob>),
    /// One night only: the image of the plan is that night's image; nothing
    /// is made, no capture of the plan is created.
    OneNight { night_id: String },
    /// Nothing combinable is ticked.
    Nothing,
}

/// The stem of the plan's files: its name, spaces as `_`, then `_plan`.
#[must_use]
pub fn plan_file_stem(name: &str) -> String {
    let cleaned: String = clean_component(name)
        .chars()
        .map(|c| if c.is_whitespace() { '_' } else { c })
        .collect();
    format!("{cleaned}_plan")
}

/// The job of the lines `chosen` (their keys) among `lines`: only the lines
/// whose master is here are combined. `plan_dir`: [`plan_folder`].
#[must_use]
pub fn plan_image_job(
    project: &NightProject,
    lines: &[PlanLine],
    chosen: &[String],
    plan_dir: &Path,
    pipeline: Option<PipelineChoice>,
) -> PlanImagePlan {
    plan_image_job_with(
        project,
        lines,
        chosen,
        plan_dir,
        pipeline,
        &PlanMaking::default(),
    )
}

/// [`plan_image_job`] with what the person chose beyond the ticks: the source of each line (the master
/// Astro Sync kept, or the image of the device) and the frame to keep.
#[must_use]
pub fn plan_image_job_with(
    project: &NightProject,
    lines: &[PlanLine],
    chosen: &[String],
    plan_dir: &Path,
    pipeline: Option<PipelineChoice>,
    making: &PlanMaking,
) -> PlanImagePlan {
    plan_image_job_of(project, lines, chosen, plan_dir, pipeline, making, false)
}

/// [`plan_image_job_with`]; `single_night`: a single night is combined all the
/// same (alone, aligned on itself) instead of being said to be the night's own
/// image -- the first milestone of the progression of a plan.
pub(crate) fn plan_image_job_of(
    project: &NightProject,
    lines: &[PlanLine],
    chosen: &[String],
    plan_dir: &Path,
    pipeline: Option<PipelineChoice>,
    making: &PlanMaking,
    single_night: bool,
) -> PlanImagePlan {
    plan_image_job_in_run(
        project,
        lines,
        chosen,
        plan_dir,
        pipeline,
        making,
        single_night,
        &[],
    )
}

/// [`plan_image_job_of`] for one pipeline of a run that makes several: the
/// name of the files is the run's (`run`: the pipelines of the whole run,
/// this one included), so that every image of one run carries one number
/// ([`plan_run_name`]).
#[allow(clippy::too_many_arguments)]
fn plan_image_job_in_run(
    project: &NightProject,
    lines: &[PlanLine],
    chosen: &[String],
    plan_dir: &Path,
    pipeline: Option<PipelineChoice>,
    making: &PlanMaking,
    single_night: bool,
    run: &[PipelineChoice],
) -> PlanImagePlan {
    // Each line as it is taken: its totals and its file are those of its source.
    let picked: Vec<(PlanLine, PathBuf)> = lines
        .iter()
        .filter(|l| chosen.contains(&l.key) && l.combinable())
        .map(|l| l.taking(l.source_for(making.sources.get(&l.key).copied())))
        .collect();
    let mut nights: Vec<&str> = picked.iter().map(|(l, _)| l.night_id.as_str()).collect();
    nights.sort_unstable();
    nights.dedup();
    match nights.as_slice() {
        [] => return PlanImagePlan::Nothing,
        [one] if !single_night => {
            return PlanImagePlan::OneNight {
                night_id: (*one).to_string(),
            };
        }
        _ => {}
    }
    let pipeline = pipeline.unwrap_or_else(|| PipelineChoice::new(DEFAULT_PIPELINE));
    let out = plan_dir.join(STACKED_DIR);
    let stem = plan_file_stem(&project.name);
    let mut suffixes: Vec<String> = run.iter().map(file_suffix_of).collect();
    if !suffixes.contains(&file_suffix_of(&pipeline)) {
        suffixes.push(file_suffix_of(&pipeline));
    }
    let combined: Vec<(String, Option<String>, u64)> = picked
        .iter()
        .map(|(line, _)| (line.night_id.clone(), line.filter.clone(), line.frame_count))
        .collect();
    let name = plan_run_name(&out, &stem, &suffixes, &combined);
    let refs: Vec<&PlanLine> = picked.iter().map(|(line, _)| line).collect();
    PlanImagePlan::Job(Box::new(PlanImageJob {
        format: 1,
        project_id: project.project_id.clone(),
        target_id: project.target_id.clone(),
        plan_dir: plan_dir.to_path_buf(),
        out,
        work: plan_dir.join(WORK_DIR).join(PLAN_SCRATCH_DIR),
        name,
        reference: reference_of(&refs).unwrap_or(0),
        frame: making.frame,
        plan_field: making.plan_field,
        crop: None,
        frame_master: None,
        masters: picked
            .iter()
            .map(|(line, path)| PlanJobMaster {
                path: path.clone(),
                night_id: line.night_id.clone(),
                evening_date: line.evening_date.clone(),
                filter: line.filter.clone(),
                band: line.band,
                frame_count: line.frame_count,
                total_seconds: line.total_seconds,
                source: line.source,
            })
            .collect(),
        pipeline,
        contrast: crate::stacking::Contrast::None,
        retouch: crate::stacking::Retouch::default(),
        pipelines_dirs: Vec::new(),
        siril: None,
        starnet: None,
        graxpert: None,
        pipeline_label: None,
        render_texts: (None, None),
        jpeg: false,
    }))
}

/// The part of an image's file name that says its render: the pipeline's
/// identifier, `linear` for "No processing".
fn file_suffix_of(pipeline: &PipelineChoice) -> String {
    if pipeline.is_linear() {
        LINEAR_PIPELINE.to_string()
    } else {
        pipeline.pipeline_id.clone()
    }
}

/// The nights, filters and poses a record of an earlier image says it
/// combined (`masters` of `<name>_plan.json`), in a fixed order.
fn recorded_masters(record: &Path) -> Option<Vec<(String, Option<String>, u64)>> {
    let record: serde_json::Value = serde_json::from_str(&fs::read_to_string(record).ok()?).ok()?;
    let mut masters: Vec<(String, Option<String>, u64)> = record["masters"]
        .as_array()?
        .iter()
        .map(|m| {
            (
                m["nightId"].as_str().unwrap_or_default().to_string(),
                m["filter"].as_str().map(str::to_string),
                m["frameCount"].as_u64().unwrap_or(0),
            )
        })
        .collect();
    masters.sort();
    Some(masters)
}

/// The name that every image of one run shares (`IC_1805_plan`,
/// `IC_1805_plan-2`...): **one run, one number**, whatever the pipelines it
/// makes. A run of the same nights, filters and poses as an earlier one takes
/// its name again and rewrites its files; otherwise the first number nothing
/// uses yet (`suffixes`: the renders of the run, the files of which
/// `<name>_<suffix>.tif` say a name is used).
#[must_use]
pub fn plan_run_name(
    out: &Path,
    stem: &str,
    suffixes: &[String],
    masters: &[(String, Option<String>, u64)],
) -> String {
    let mut wanted = masters.to_vec();
    wanted.sort();
    let mut n = 1_u32;
    loop {
        let name = if n == 1 {
            stem.to_string()
        } else {
            format!("{stem}-{n}")
        };
        let record = out.join(record_file_name(&name));
        let used = record.exists()
            || suffixes
                .iter()
                .any(|suffix| out.join(format!("{name}_{suffix}.tif")).exists());
        if !used || recorded_masters(&record).is_some_and(|recorded| recorded == wanted) {
            return name;
        }
        n += 1;
    }
}

/// What a plan's target starts with when no pipeline was chosen for it
/// (the defaults of "Processing"): "Natural render"
/// ([`DEFAULT_PIPELINE`]), and "Dual-band correction" (`dualband-correction`)
/// too as soon as a dual-band integration is combined.
#[must_use]
pub fn default_plan_pipelines(lines: &[PlanLine], chosen: &[String]) -> Vec<PipelineChoice> {
    let dual = lines
        .iter()
        .any(|l| chosen.contains(&l.key) && l.combinable() && l.band == Band::Dualband);
    let mut ids = vec![DEFAULT_PIPELINE];
    if dual {
        ids.push("dualband-correction");
    }
    ids.into_iter().map(PipelineChoice::new).collect()
}

/// The jobs of the image of a plan, one per pipeline chosen for its target
/// (the image takes the pipelines of the target, not a render of its own). A
/// pipeline asked twice is made once; none asked: the target's
/// defaults ([`default_plan_pipelines`]). Several jobs, or exactly one
/// [`PlanImagePlan::OneNight`] / [`PlanImagePlan::Nothing`] when there is
/// nothing to combine.
#[must_use]
pub fn plan_image_jobs(
    project: &NightProject,
    lines: &[PlanLine],
    chosen: &[String],
    plan_dir: &Path,
    pipelines: &[PipelineChoice],
) -> Vec<PlanImagePlan> {
    plan_image_jobs_with(
        project,
        lines,
        chosen,
        plan_dir,
        pipelines,
        &PlanMaking::default(),
    )
}

/// [`plan_image_jobs`] with the source of each line and the frame the person chose.
#[must_use]
pub fn plan_image_jobs_with(
    project: &NightProject,
    lines: &[PlanLine],
    chosen: &[String],
    plan_dir: &Path,
    pipelines: &[PipelineChoice],
    making: &PlanMaking,
) -> Vec<PlanImagePlan> {
    let mut wanted: Vec<PipelineChoice> = Vec::new();
    for pipeline in pipelines {
        if !wanted.iter().any(|w| w.pipeline_id == pipeline.pipeline_id) {
            wanted.push(pipeline.clone());
        }
    }
    if wanted.is_empty() {
        wanted = default_plan_pipelines(lines, chosen);
    }
    let mut plans = Vec::new();
    for pipeline in &wanted {
        match plan_image_job_in_run(
            project,
            lines,
            chosen,
            plan_dir,
            Some(pipeline.clone()),
            making,
            false,
            &wanted,
        ) {
            job @ PlanImagePlan::Job(_) => plans.push(job),
            nothing_to_make => return vec![nothing_to_make],
        }
    }
    plans
}

/* ------------------------------- The frame -------------------------------- */

/// The suffix `plan_stack.py` gives the image cut to the frame kept, before the extension.
pub const FRAMED_SUFFIX: &str = "_framed";

/// The share of the whole image below the full exposure from which the
/// choice of a frame is offered ("nothing under 1 %").
pub const FRAMING_OFFERED_FROM: f64 = 0.01;

fn is_framed_tiff(file: &str) -> bool {
    let path = Path::new(file);
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "tif" | "tiff"))
        && path
            .file_stem()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.ends_with(FRAMED_SUFFIX))
}

/// One stop of the slider of the frame, as `plan_framing.py` measured it on the image (`coverage.framings` of
/// the record).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FramingNotch {
    pub top: u32,
    pub left: u32,
    pub height: u32,
    pub width: u32,
    /// The share of the whole image (0 to 1).
    pub share_of_frame: f64,
    /// The share of this frame below the full exposure (0 to 1).
    pub degraded_share: f64,
    /// The lowest exposure found in it, seconds, and the dual-band exposure there (0: no Hα, no OIII).
    pub lowest_seconds: u64,
    pub lowest_dual_band_seconds: u64,
    /// `cropped`, `mark`, `between` or `whole`.
    pub kind: String,
    /// The mark: the largest frame with at most 5 % of its pixels degraded.
    pub recommended: bool,
    /// The whole image, the default.
    pub default: bool,
}

impl FramingNotch {
    /// The four integers a job carries.
    #[must_use]
    pub fn frame(&self) -> PlanFrame {
        PlanFrame {
            top: self.top,
            left: self.left,
            height: self.height,
            width: self.width,
        }
    }
}

/// What the screen offers for the frame of the image of a plan, read from the record of the last image made with
/// the same integrations. The screen recomputes nothing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanFraming {
    /// The record it was read from (`M_45_plan.json`).
    pub record: String,
    pub frame_height: u32,
    pub frame_width: u32,
    /// The share of the whole image below the full exposure (0 to 1).
    pub degraded_share: f64,
    /// The share without dual-band data, 0 unless the plan mixes the bands (0 to 1).
    pub no_dual_band_share: f64,
    /// A dual-band integration is combined: the Hα and OIII are mentioned.
    pub has_dual_band: bool,
    /// The exposure of the best covered pixels, seconds, and its dual-band part.
    pub full_seconds: u64,
    pub full_dual_band_seconds: u64,
    pub recommended_degraded_share: f64,
    /// The corners (`[x, y]`, pixels of the whole image) of the area every integration covers: what the outline
    /// leaves unhatched on the picture. Empty when the record has none.
    pub full_polygon: Vec<[u32; 2]>,
    /// Cropped first, the whole image last.
    pub notches: Vec<FramingNotch>,
    /// The choice is offered: at least [`FRAMING_OFFERED_FROM`] of the image is below the full exposure and there is a
    /// smaller frame to take.
    pub offered: bool,
    /// A small picture of the whole image (`data:image/png;base64,…`), for the outline.
    pub preview: Option<String>,
}

/// The base64 of `bytes` (RFC 4648, padded): the picture of the preview goes to the window as a data URL.
pub(crate) fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecordMaster {
    night_id: String,
    #[serde(default)]
    filter: Option<String>,
    #[serde(default)]
    source: PlanSource,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecordFullCoverage {
    frame_height: u32,
    frame_width: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecordCoverage {
    full_coverage: RecordFullCoverage,
    #[serde(default)]
    no_dual_band_share: f64,
    #[serde(default)]
    degraded_share: f64,
    #[serde(default)]
    has_dual_band: bool,
    #[serde(default)]
    full_seconds: u64,
    #[serde(default)]
    full_dual_band_seconds: u64,
    #[serde(default)]
    recommended_degraded_share: f64,
    #[serde(default)]
    full_polygon: Option<Vec<[u32; 2]>>,
    #[serde(default)]
    framings: Vec<FramingNotch>,
}

#[derive(Deserialize)]
struct RecordCropMode {
    #[serde(default)]
    mode: Option<PlanField>,
}

#[derive(Deserialize)]
struct PlanRecord {
    #[serde(default)]
    masters: Vec<RecordMaster>,
    /// The frame the image was cut to; a record without one was made whole.
    #[serde(default)]
    crop: Option<RecordCropMode>,
    #[serde(default)]
    coverage: Option<RecordCoverage>,
    #[serde(default)]
    preview: Option<String>,
}

/// The integrations a job combines, as a record names them: night, filter and source, in a fixed order.
fn combined_set(
    items: impl Iterator<Item = (String, Option<String>, PlanSource)>,
) -> Vec<(String, Option<String>, PlanSource)> {
    let mut set: Vec<_> = items.collect();
    set.sort();
    set
}

/// The frames on offer for the image of the plan made of `chosen` (the keys of the integrations ticked, `making`
/// the sources taken and the frame the image is cut to, [`PlanField`]): read from the record of the last image
/// made **with the same integrations, cut the same way** in
/// `<plan>/stacked/` (the newest), its picture beside it. `None` before the first image of that set is made (the
/// first image is always whole), or when the record says nothing of the frame.
#[must_use]
pub fn plan_framing(
    plan_dir: &Path,
    lines: &[PlanLine],
    chosen: &[String],
    making: &PlanMaking,
) -> Option<PlanFraming> {
    let wanted = combined_set(
        lines
            .iter()
            .filter(|l| chosen.contains(&l.key) && l.combinable())
            .map(|l| {
                let source = l.source_for(making.sources.get(&l.key).copied());
                (l.night_id.clone(), l.filter.clone(), source)
            }),
    );
    if wanted.is_empty() {
        return None;
    }
    let stacked = plan_dir.join(STACKED_DIR);
    let mut best: Option<(std::time::SystemTime, PathBuf, PlanRecord)> = None;
    for entry in fs::read_dir(&stacked).ok()?.filter_map(Result::ok) {
        let path = entry.path();
        let is_record = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with("_plan.json"));
        if !is_record {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(record) = serde_json::from_str::<PlanRecord>(&text) else {
            continue;
        };
        let made = combined_set(
            record
                .masters
                .iter()
                .map(|m| (m.night_id.clone(), m.filter.clone(), m.source)),
        );
        // The notches are measured on the image as it was cut: only an image cut the same way offers them.
        let field = record
            .crop
            .as_ref()
            .map_or(Some(PlanField::Whole), |crop| crop.mode);
        if made != wanted || record.coverage.is_none() || field != Some(making.plan_field) {
            continue;
        }
        let at = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::UNIX_EPOCH);
        if best.as_ref().is_none_or(|(newest, _, _)| at > *newest) {
            best = Some((at, path, record));
        }
    }
    let (_, path, record) = best?;
    let coverage = record.coverage?;
    let preview = record
        .preview
        .filter(|name| !name.contains(['/', '\\']))
        .and_then(|name| fs::read(stacked.join(name)).ok())
        .map(|bytes| format!("data:image/png;base64,{}", base64(&bytes)));
    let offered = coverage.degraded_share >= FRAMING_OFFERED_FROM && coverage.framings.len() >= 2;
    Some(PlanFraming {
        record: path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        frame_height: coverage.full_coverage.frame_height,
        frame_width: coverage.full_coverage.frame_width,
        degraded_share: coverage.degraded_share,
        no_dual_band_share: coverage.no_dual_band_share,
        has_dual_band: coverage.has_dual_band,
        full_seconds: coverage.full_seconds,
        full_dual_band_seconds: coverage.full_dual_band_seconds,
        recommended_degraded_share: coverage.recommended_degraded_share,
        full_polygon: coverage.full_polygon.unwrap_or_default(),
        notches: coverage.framings,
        offered,
        preview,
    })
}

/* -------------------------------- The making ------------------------------ */

/// The slot. The real one is [`PythonPlanImager`]; tests give a fake.
pub trait PlanImager {
    fn make(
        &self,
        job: &PlanImageJob,
        cancel: &CancelFlag,
        on_event: &mut dyn FnMut(&StackEvent),
    ) -> Result<Vec<StackerOutput>, StackFailure>;
}

/// `plan_stack.py` in the stacking environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PythonPlanImager {
    pub python: PathBuf,
    /// `plan_stack.py`, beside `astro_sync_stack.py`.
    pub script: PathBuf,
    pub timeout: Duration,
}

impl PlanImager for PythonPlanImager {
    fn make(
        &self,
        job: &PlanImageJob,
        cancel: &CancelFlag,
        on_event: &mut dyn FnMut(&StackEvent),
    ) -> Result<Vec<StackerOutput>, StackFailure> {
        let io = |e: std::io::Error| StackFailure::Failed {
            code: "io_error".to_string(),
            message: e.to_string(),
            logs: None,
        };
        let file = job.job_file();
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent).map_err(io)?;
        }
        let text = serde_json::to_string_pretty(job).map_err(|e| io(std::io::Error::other(e)))?;
        fs::write(&file, text).map_err(io)?;
        let mut command = python_env::script_command(&self.python, &self.script);
        command.arg("--job").arg(&file).arg("--json");
        let result = run_script(command, &job.work, self.timeout, cancel, on_event);
        let _ = fs::remove_file(&file);
        result
    }
}

/// What makes the image of a plan possible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanImageGate {
    /// The multi-night plans are open here ([`crate::multi_night::MultiNightGate`]).
    pub allowed: bool,
    /// The pipeline row of "Dependencies" is present (Siril and Python).
    pub tool_present: bool,
    /// `analysis.logs`: the quality of the image is read from the record of
    /// the combination and leaves with the image; without it nothing is
    /// built.
    pub image_quality: bool,
}

/// An image of the job beside the plan's own: the same render with a
/// contrast and/or a finish, and what made it (its steps say the treatment).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanFinish {
    pub render: StackedRender,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub processing: Option<ProcessingProposal>,
}

/// The image of a plan, made.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanImageOutcome {
    pub project_id: String,
    pub target_id: String,
    /// The render (a 16-bit TIFF) that leaves as the capture of the plan: the
    /// job's image without a finish.
    pub render: StackedRender,
    /// The other 16-bit TIFFs the job wrote: the same render with its contrast
    /// and its finish (colours, softened stars). Each leaves as an image of
    /// the capture, named after its treatment.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub finishes: Vec<PlanFinish>,
    pub other_files: Vec<PathBuf>,
    pub combined_night_ids: Vec<String>,
    pub totals_by_filter: Vec<PlanFilterTotal>,
    /// `natural`, `dualband`, `polished`, `hoo` or `none` (the service's words).
    pub plan_render: Option<String>,
    /// The quality of the combined linear image, read on its stars before the
    /// render (`quality` of `<name>_plan.json`) -- the image of the plan's
    /// own, in arc seconds when the reference master says its pixel scale.
    /// `None` without `analysis.logs`, or when fewer than twenty stars could
    /// be read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_quality: Option<ImageQuality>,
    /// What made the image, proposed with it -- « Astro Sync 0.9.1 · Siril
    /// 1.4.4 », and the combination method (`weighted_mean`), the render
    /// and the nights in the steps. Without it the service reads the
    /// `Software` tag of the TIFF.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub processing: Option<ProcessingProposal>,
    /// The title of the render in the ten languages (« Dual-band
    /// correction »), proposed as the generated name of the image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated_title: Option<GeneratedText>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated_description: Option<GeneratedText>,
}

/// The file that holds the record of a plan's combination in `stacked/`:
/// `<name>_plan.json`, and `<name>.json` when the name already ends in `_plan`
/// (the name of a plan's image does), the rule of `plan_stack.py`.
#[must_use]
pub fn record_file_name(name: &str) -> String {
    if name.ends_with("_plan") {
        format!("{name}.json")
    } else {
        format!("{name}_plan.json")
    }
}

/// The rectangle the image a job made was cut to and the master whose frame it
/// is on, read from its record (`crop` and `frameMaster` of `<name>_plan.json`
/// in the job's `out`). `None` before the image is made, or for a record
/// written before the crop. The rectangle is the whole frame when nothing was
/// cut (the whole field kept, or nothing to cut): a milestone given it cuts
/// nothing either.
#[must_use]
pub fn recorded_crop(job: &PlanImageJob) -> Option<(PlanCrop, PlanFrameMaster)> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Record {
        crop: Option<PlanCrop>,
        frame_master: Option<PlanFrameMaster>,
    }
    let text = fs::read_to_string(job.out.join(record_file_name(&job.name))).ok()?;
    let record = serde_json::from_str::<Record>(&text).ok()?;
    Some((record.crop?, record.frame_master?))
}

/// The quality of the image of a plan: what `plan_stack.py` measured on the
/// combined linear master (`quality` of the record it writes), and the pixel
/// scale of the reference master (the others are aligned on its frame).
fn plan_image_quality(job: &PlanImageJob) -> Option<ImageQuality> {
    #[derive(Deserialize)]
    struct Record {
        #[serde(default)]
        quality: Option<MeasuredQuality>,
    }
    let text = fs::read_to_string(job.out.join(record_file_name(&job.name))).ok()?;
    let measure = serde_json::from_str::<Record>(&text).ok()?.quality?;
    let scale = job
        .masters
        .get(job.reference)
        .and_then(|master| read_primary_header_file(&master.path).ok())
        .and_then(|header| sensor_pixel_scale(&header))
        .map(|scale| scale.arcsec_per_pixel);
    ImageQuality::from_measure(&measure, scale)
}

/// A number as the steps write it: no trailing zeros, at most `decimals`.
fn short_number(value: f64, decimals: usize) -> String {
    let text = format!("{value:.decimals$}");
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        text
    }
}

/// The numbers of a JSON array, « 1.012/0.998 ».
fn numbers_of(value: Option<&serde_json::Value>, decimals: usize) -> Option<String> {
    let list: Vec<String> = value?
        .as_array()?
        .iter()
        .filter_map(serde_json::Value::as_f64)
        .map(|n| short_number(n, decimals))
        .collect();
    (!list.is_empty()).then(|| list.join("/"))
}

/// What made the image of a plan: the software (Astro Sync's version, and
/// Siril's when it aligned the masters -- the line the script recorded in
/// `tools.siril`) and the steps, in the order they ran, each with its
/// parameters as `<name>_plan.json` records them: the pipeline of the render,
/// the alignment, the level match by aperture (radius, scale of each
/// channel), the combination (method, nights, the weight of each), the share
/// of the frame no night covers, the contrast, the frames. The contract keeps
/// `processing.steps` as one text of 500 characters at most (the first steps
/// are the ones that matter, so they come first). Without the record the
/// method is not said.
pub fn plan_processing(job: &PlanImageJob, render: &StackedRender) -> ProcessingProposal {
    plan_processing_of(job, &render.pipeline_id, render.contrast, render.retouch)
}

fn plan_processing_of(
    job: &PlanImageJob,
    pipeline_id: &str,
    contrast: crate::stacking::Contrast,
    retouch: crate::stacking::Retouch,
) -> ProcessingProposal {
    let record: serde_json::Value = fs::read_to_string(job.out.join(record_file_name(&job.name)))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(serde_json::Value::Null);
    let siril = record["tools"]["siril"].as_str().map(str::to_string);
    // Siril aligned the masters whenever the job named it.
    let software = crate::metadata::software_label(siril.as_deref(), job.siril.is_some());
    let combination = &record["combination"];
    // The pipeline by name: the service's own word for the render (`natural`,
    // `dualband`, ...) when it has one -- the same in every language --, else
    // the pipeline's name; the id follows in brackets.
    let name = plan_render_of(pipeline_id)
        .or_else(|| job.pipeline_label.clone())
        .filter(|name| name != pipeline_id);
    let mut steps = vec![match name {
        Some(name) => format!("pipeline {name} ({pipeline_id})"),
        None => format!("pipeline {pipeline_id}"),
    }];
    if job.masters.len() > 1 {
        // A plan combined on its dual-band master is registered from the
        // stars of the masters (astroalign), not by Siril.
        steps.push(
            if combination["method"].as_str() == Some("dualband_reference") {
                "alignment stars (astroalign)"
            } else {
                "alignment Siril 2-pass affine"
            }
            .to_string(),
        );
    }
    // The scale brought to each master, night after night.
    let levels: Vec<String> = combination["levelMatch"]
        .as_array()
        .map(|list| {
            list.iter()
                .map(|level| {
                    let method = level["method"].as_str().unwrap_or("aperture");
                    let mut text = method.to_string();
                    if let Some(radius) = level["apertureRadius"].as_f64() {
                        text += &format!(" r={} px", short_number(radius, 1));
                    }
                    if let Some(scales) = numbers_of(level.get("scales"), 3) {
                        text += &format!(" scales {scales}");
                    }
                    text
                })
                .collect()
        })
        .unwrap_or_default();
    if !levels.is_empty() {
        steps.push(format!("level match {}", levels.join("; ")));
    }
    let nights = job.night_ids().len();
    let mut combined = combination["method"]
        .as_str()
        .filter(|m| !m.is_empty())
        .unwrap_or("combination")
        .to_string();
    if nights > 0 {
        combined += &format!(" {nights} nights");
    }
    let weights: Vec<f64> = combination["masters"]
        .as_array()
        .map(|list| list.iter().filter_map(|m| m["weight"].as_f64()).collect())
        .unwrap_or_default();
    if !weights.is_empty() {
        let weights: Vec<String> = weights.into_iter().map(|w| short_number(w, 3)).collect();
        combined += &format!(" weights {}", weights.join("/"));
    }
    steps.push(combined);
    if let Some(share) = combination["noData"].as_f64().filter(|share| *share > 0.0) {
        steps.push(format!("no data {} %", short_number(share * 100.0, 1)));
    }
    // The cut of the frame: to the edges with data, or to the area every
    // night covers; what is kept, and what each edge lost.
    let crop = &record["crop"];
    if crop["applied"].as_bool() == Some(true) {
        let number = |value: &serde_json::Value| value.as_u64().unwrap_or(0);
        let lost = &crop["lost"];
        let to = if crop["mode"].as_str() == Some("common") {
            "common area"
        } else {
            "edges with data"
        };
        steps.push(format!(
            "crop to {to} {}x{} px of {}x{}, kept {} % (edges lost top/left/bottom/right {}/{}/{}/{} px)",
            number(&crop["width"]),
            number(&crop["height"]),
            number(&crop["frameWidth"]),
            number(&crop["frameHeight"]),
            short_number(crop["keptShare"].as_f64().unwrap_or(1.0) * 100.0, 1),
            number(&lost["top"]),
            number(&lost["left"]),
            number(&lost["bottom"]),
            number(&lost["right"]),
        ));
    }
    // The treatment *this* image carries, never the one the job asked for: the
    // image of the plan has none, its twins have theirs.
    if !contrast.is_none() {
        steps.push(format!("contrast {}", contrast.token()));
    }
    steps.extend(crate::metadata::retouch_steps(retouch));
    let frames: u64 = job.masters.iter().map(|m| m.frame_count).sum();
    if frames > 0 {
        steps.push(format!("{frames} lights"));
    }
    crate::metadata::processing_proposal(&software, &steps)
}

/// The service's word for a render: `raw` is "Natural render"
/// (`natural`), `dualband-correction` "Dual-band correction" (`dualband`),
/// `polished` "polished render" (`polished`), `hoo` "HOO palette" (`hoo`),
/// "No processing" `none`; a pipeline the person added says nothing.
#[must_use]
pub fn plan_render_of(pipeline_id: &str) -> Option<String> {
    match pipeline_id {
        "raw" => Some("natural".to_string()),
        "dualband-correction" => Some("dualband".to_string()),
        "polished" => Some("polished".to_string()),
        "hoo" => Some("hoo".to_string()),
        LINEAR_PIPELINE => Some("none".to_string()),
        _ => None,
    }
}

fn remove_plan_scratch(job: &PlanImageJob) {
    let _ = fs::remove_file(job.job_file());
    let _ = fs::remove_dir_all(&job.work);
    let _ = fs::remove_dir(job.plan_dir.join(WORK_DIR));
}

/// Makes the image of a plan under its rules (see the module
/// documentation). The scratch is gone when this returns, whatever the
/// outcome.
pub fn run_plan_image(
    imager: &dyn PlanImager,
    job: &PlanImageJob,
    gate: PlanImageGate,
    cancel: &CancelFlag,
    on_event: &mut dyn FnMut(&StackEvent),
) -> Result<PlanImageOutcome, StackFailure> {
    if !gate.allowed {
        return Err(StackFailure::NotInSubscription);
    }
    if !gate.tool_present {
        return Err(StackFailure::ToolMissing);
    }
    let failed = |code: &str, message: String| StackFailure::Failed {
        code: code.to_string(),
        message,
        logs: None,
    };
    // A scratch a hard stop left behind goes first.
    remove_plan_scratch(job);
    fs::create_dir_all(&job.out).map_err(|e| failed("io_error", e.to_string()))?;
    let made = imager.make(job, cancel, on_event);
    remove_plan_scratch(job);
    let outputs = made?;
    let stacked = fs::canonicalize(&job.out).map_err(|e| failed("io_error", e.to_string()))?;
    let mut renders: Vec<StackedRender> = Vec::new();
    let mut other_files = Vec::new();
    // With a frame kept, the image that leaves is the cut one (`<file>_framed.tif`); the whole image stays in
    // `stacked/`.
    let framed = job.frame.is_some() && outputs.iter().any(|o| is_framed_tiff(&o.file));
    for output in outputs {
        let path = job.out.join(&output.file);
        let inside = !output.file.contains(['/', '\\'])
            && fs::canonicalize(&path).is_ok_and(|p| p.parent() == Some(stacked.as_path()));
        if !inside {
            return Err(StackFailure::OutsideStacked { file: output.file });
        }
        let is_tiff = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "tif" | "tiff"));
        if !is_tiff || (framed && !is_framed_tiff(&output.file)) {
            other_files.push(path);
            continue;
        }
        let (sha256, size) = crate::stacking::sha256_and_size(&path)
            .map_err(|e| failed("io_error", e.to_string()))?;
        renders.push(StackedRender {
            pipeline_id: output.pipeline,
            contrast: output.contrast,
            retouch: output.retouch,
            path,
            sha256,
            size,
            family: None,
        });
    }
    // The image of the plan is the render without a curve nor a finish (the
    // script writes it first); the others are its twins, one per treatment.
    let main = renders
        .iter()
        .position(|r| r.contrast.is_none() && r.retouch.is_none())
        .unwrap_or(0);
    if renders.is_empty() {
        return Err(failed(
            "no_output",
            "the image of the plan was not written".into(),
        ));
    }
    let render = renders.remove(main);
    let finishes: Vec<PlanFinish> = renders
        .into_iter()
        .map(|render| PlanFinish {
            processing: Some(plan_processing(job, &render)),
            render,
        })
        .collect();
    let image_quality = if gate.image_quality {
        plan_image_quality(job)
    } else {
        None
    };
    let lines: Vec<PlanLine> = job
        .masters
        .iter()
        .map(|m| PlanLine {
            key: String::new(),
            night_id: m.night_id.clone(),
            evening_date: m.evening_date.clone(),
            filter: m.filter.clone(),
            band: m.band,
            frame_count: m.frame_count,
            exposure_seconds: None,
            total_seconds: m.total_seconds,
            place: Place::ThisComputer,
            computer_label: None,
            master: None,
            bytes: 0,
            source: m.source,
            device: None,
        })
        .collect();
    let refs: Vec<&PlanLine> = lines.iter().collect();
    let processing = Some(plan_processing(job, &render));
    Ok(PlanImageOutcome {
        project_id: job.project_id.clone(),
        target_id: job.target_id.clone(),
        plan_render: plan_render_of(&render.pipeline_id),
        render,
        finishes,
        other_files,
        combined_night_ids: job.night_ids(),
        totals_by_filter: totals_by_filter(&refs),
        image_quality,
        processing,
        generated_title: job.render_texts.0.clone(),
        generated_description: job.render_texts.1.clone(),
    })
}

/// The image of the plan as results to enqueue: the capture of the plan
/// (`role: "plan"`, [`ResultKind::Plan`]), never of a night, and its other
/// treatments (contrast, finish) as images of that capture. The capture
/// carries everything Astro Sync proposes for it (quality, how it was made)
/// and the title of its render; every image carries its own title, named
/// after its treatment, and its own steps.
#[must_use]
pub fn plan_results(outcome: &PlanImageOutcome) -> Vec<ResultFile> {
    let mut results = vec![plan_result(
        outcome,
        &outcome.render,
        outcome.processing.clone(),
        true,
        None,
    )];
    results.extend(plan_finish_results(
        outcome,
        &format!(
            "{}{}",
            crate::uploader::VARIANT_OF_SHA256,
            outcome.render.sha256
        ),
    ));
    results
}

/// Another render of the same plan, sent as images of the capture the first
/// one creates (`first_sha256` is that image's digest, resolved at the
/// upload): the render itself and each of its treatments, with their title,
/// description, quality and steps, and nothing of the capture -- how the
/// *capture* was made is the main image's to say, and the service keeps each
/// image's own steps to show them when it becomes the main one.
#[must_use]
pub fn plan_variant_results(outcome: &PlanImageOutcome, first_sha256: &str) -> Vec<ResultFile> {
    plan_images_of(
        outcome,
        &format!("{}{first_sha256}", crate::uploader::VARIANT_OF_SHA256),
    )
}

/// Every image of `outcome` as an image of the capture `variant_of` names
/// (the identifier of a capture online, or `sha256:<digest>` of the image of
/// the same sending that creates it).
fn plan_images_of(outcome: &PlanImageOutcome, variant_of: &str) -> Vec<ResultFile> {
    let mut results = vec![plan_result(
        outcome,
        &outcome.render,
        outcome.processing.clone(),
        false,
        Some(variant_of.to_string()),
    )];
    results.extend(plan_finish_results(outcome, variant_of));
    results
}

/// The treatments of the job beside its main render, as images of the capture
/// `variant_of` names.
fn plan_finish_results(outcome: &PlanImageOutcome, variant_of: &str) -> Vec<ResultFile> {
    outcome
        .finishes
        .iter()
        .map(|finish| {
            plan_result(
                outcome,
                &finish.render,
                finish.processing.clone(),
                false,
                Some(variant_of.to_string()),
            )
        })
        .collect()
}

fn plan_result(
    outcome: &PlanImageOutcome,
    render: &StackedRender,
    processing: Option<ProcessingProposal>,
    creates_capture: bool,
    variant_of: Option<String>,
) -> ResultFile {
    // Its name says its treatment: the render's title, then the contrast, the
    // colours and the softened stars it carries (ten languages).
    let title = outcome
        .generated_title
        .as_ref()
        .map(|base| crate::metadata::finished_title(base, render.contrast, render.retouch));
    let proposals = Proposals {
        processing: processing.clone().filter(|_| creates_capture),
        variant_processing: processing,
        image_quality: outcome.image_quality.clone(),
        variant_generated_title: title,
        variant_generated_description: outcome.generated_description.clone(),
        ..Proposals::default()
    };
    ResultFile {
        path: render.path.clone(),
        sha256: render.sha256.clone(),
        size: render.size,
        kind: ResultKind::Plan,
        target: EntryTarget {
            session_id: None,
            target_ids: vec![outcome.target_id.clone()],
            variant_of,
            source: Some("stacking".to_string()),
            metadata: Some(proposals).filter(|proposals| !proposals.is_empty()),
            frame_count: None,
            plan: Some(PlanUpload {
                project_id: outcome.project_id.clone(),
                combined_night_ids: outcome.combined_night_ids.clone(),
                totals_by_filter: outcome.totals_by_filter.clone(),
                plan_render: outcome.plan_render.clone(),
            }),
        },
    }
}

/// What [`run_plan_images`] made: every image made, and why the next one was
/// not (the ones made stay made, whatever follows).
#[derive(Debug)]
pub struct PlanImagesRun {
    pub outcomes: Vec<PlanImageOutcome>,
    pub failure: Option<StackFailure>,
}

/// Makes the image of a plan with each of its jobs, one after the other;
/// stops at the first that fails.
pub fn run_plan_images(
    imager: &dyn PlanImager,
    jobs: &[&PlanImageJob],
    gate: PlanImageGate,
    cancel: &CancelFlag,
    on_event: &mut dyn FnMut(&StackEvent),
) -> PlanImagesRun {
    let mut run = PlanImagesRun {
        outcomes: Vec::new(),
        failure: None,
    };
    for job in jobs {
        match run_plan_image(imager, job, gate, cancel, on_event) {
            Ok(outcome) => run.outcomes.push(outcome),
            Err(failure) => {
                run.failure = Some(failure);
                break;
            }
        }
    }
    run
}

/// The images of a plan as results to enqueue: **one capture per plan**. The
/// main image -- the render of the pipeline `main`, else the first one made --
/// is the capture; every other render, and every contrast or finish of any
/// render, is an image of that capture (a variant, named after its
/// treatment), exactly as the renders of a night.
///
/// `capture`: the capture of the plan the service already holds (its
/// identifier). Then nothing creates a capture: every image is added to it
/// as a variant, the main image included.
#[must_use]
pub fn plan_results_of(
    outcomes: &[PlanImageOutcome],
    main: Option<&str>,
    capture: Option<&str>,
) -> Vec<ResultFile> {
    if let Some(capture) = capture {
        // The main render goes first: it is the one that would be the capture.
        let first = main_outcome(outcomes, main);
        return first
            .into_iter()
            .chain(
                outcomes
                    .iter()
                    .filter(|o| first.is_none_or(|f| !std::ptr::eq(*o, f))),
            )
            .flat_map(|outcome| plan_images_of(outcome, capture))
            .collect();
    }
    let Some(first) = main_outcome(outcomes, main) else {
        return Vec::new();
    };
    let mut results = plan_results(first);
    results.extend(
        outcomes
            .iter()
            .filter(|outcome| !std::ptr::eq(*outcome, first))
            .flat_map(|outcome| plan_variant_results(outcome, &first.render.sha256)),
    );
    results
}

/// The outcome whose render is the capture's main image: the pipeline `main`
/// names, else the first one made.
fn main_outcome<'a>(
    outcomes: &'a [PlanImageOutcome],
    main: Option<&str>,
) -> Option<&'a PlanImageOutcome> {
    main.and_then(|id| outcomes.iter().find(|o| o.render.pipeline_id == id))
        .or_else(|| outcomes.first())
}

#[cfg(test)]
mod tests;
