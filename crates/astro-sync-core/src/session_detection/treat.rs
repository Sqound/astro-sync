//! "Choose the treatments..." after "Create ...": the sessions created
//! become night folders of the working folder, so that the existing
//! treatments ("Stacking from the FITS", the image of a multi-night plan) run
//! on them as on any night.
//!
//! For each session created that holds a target to treat: the night's folder
//! is prepared as the "Folder" step does ([`prepare_night_folder`]: its
//! identity, `calibration/`, `logs/`, `<target>/lights/`), then the frames,
//! the calibration and the logs of the session are **linked** into it
//! ([`super::organise`], "Symbolic link"): the originals never move, a
//! file already in place is left, a different one is never overwritten. On
//! Windows without developer mode no link can be made: nothing is prepared,
//! and the window says why.
//!
//! The sessions are not created yet when the treatments run: `created` is the
//! provisional outcome ([`super::provisional::provisional_outcome`]), and a
//! night to create gets a provisional folder whose record carries the night's
//! proposal.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::working_folder::{
    NightFolderSpec, NightTargetSpec, WorkingFolderError, is_provisional, prepare_night_folder,
    read_night_identity, set_target_filter, write_night_identity,
};

use super::create::BulkCreateOutcome;
use super::{
    NightNaming, OrganiseError, OrganiseMode, OrganiseOptions, Proposal, organise,
    plan_organise_named,
};

/// A target of a night prepared for its treatment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TreatmentTarget {
    pub target_id: String,
    pub designation: String,
    /// Its folder in the night's folder (`stacking_run`'s `targetFolder`).
    pub folder_name: String,
    pub frames: usize,
}

/// A night of the working folder, ready for the treatments of its targets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TreatmentNight {
    pub detected_id: String,
    pub session_id: String,
    pub evening_date: String,
    pub night_folder: PathBuf,
    /// The place's longitude, when known (the night is cut at its noon).
    pub longitude: Option<f64>,
    pub targets: Vec<TreatmentTarget>,
    /// Files linked by this call; already there; left (another file there).
    pub linked: usize,
    pub already_there: usize,
    pub conflicts: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum TreatError {
    #[error(transparent)]
    WorkingFolder(#[from] WorkingFolderError),
    #[error(transparent)]
    Organise(#[from] OrganiseError),
}

impl TreatError {
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            TreatError::WorkingFolder(_) => "working_folder_error",
            TreatError::Organise(OrganiseError::SymlinkUnavailable) => "symlink_unavailable",
            TreatError::Organise(OrganiseError::LinkAcrossVolumes { .. }) => "link_across_volumes",
            TreatError::Organise(_) => "io_error",
        }
    }
}

/// The place's name the proposal chose for a session, and its longitude.
fn place_of(proposal: &Proposal, place_id: &str) -> (String, Option<f64>) {
    let Some(place) = proposal.places.iter().find(|p| p.id == place_id) else {
        return (String::new(), None);
    };
    let name = place
        .chosen
        .as_ref()
        .and_then(|chosen| place.candidates.iter().find(|c| &c.place_id == chosen))
        .map(|c| c.name.clone())
        .unwrap_or_default();
    (name, place.longitude)
}

/// The filter to record for a target of a night: the one filter the headers
/// name, else what the names around the lights say; none when the night holds
/// several.
fn assumed_filter(target: &super::DetectedTarget) -> Option<String> {
    match target.filters.as_slice() {
        [only] => Some(only.clone()),
        [] => target.filter_hint.clone(),
        _ => None,
    }
    .map(|filter| filter.trim().to_string())
    .filter(|filter| !filter.is_empty())
}

/// Prepares the nights of `created` that hold one of `target_ids` (all of
/// them when empty), in the working folder `root`.
pub fn prepare_treatments(
    proposal: &Proposal,
    created: &BulkCreateOutcome,
    target_ids: &[String],
    root: &Path,
    remembered: &[PathBuf],
    service_url: &str,
    now_utc: &str,
) -> Result<Vec<TreatmentNight>, TreatError> {
    let wanted = |id: &str| target_ids.is_empty() || target_ids.iter().any(|t| t == id);
    let mut nights = Vec::new();
    for made in &created.sessions {
        let Some(session) = proposal.sessions.iter().find(|s| s.id == made.detected_id) else {
            continue;
        };
        let targets: Vec<_> = session
            .targets
            .iter()
            .filter_map(|t| Some((t.target_id.clone()?, t)))
            .filter(|(id, _)| wanted(id))
            .collect();
        if targets.is_empty() {
            continue;
        }
        let (label, longitude) = place_of(proposal, &session.place);
        let spec = NightFolderSpec {
            night_id: made.session_id.clone(),
            service_url: service_url.to_string(),
            evening_date: session.evening_date.clone(),
            label: label.clone(),
            targets: targets
                .iter()
                .map(|(id, t)| NightTargetSpec {
                    target_id: id.clone(),
                    designation: t.designation.clone(),
                    ticked: true,
                })
                .collect(),
            follow: None,
        };
        let folder = prepare_night_folder(root, remembered, &spec, now_utc)?;
        // The filter the detection read from the names around the lights (or
        // the person answered) goes to the target folder's identity, so the
        // stacking and the plans of this night know it though the headers do
        // not: the lights themselves are never written.
        for (id, t) in &targets {
            if let (Some(filter), Some(target_folder)) = (
                assumed_filter(t),
                folder.targets.iter().find(|f| &f.target_id == id),
            ) {
                let at = folder.folder.join(&target_folder.folder_name);
                set_target_filter(&at, &filter)
                    .map_err(|source| WorkingFolderError::Io { path: at, source })?;
            }
        }
        // A night the space does not hold yet: its record carries what the
        // detection proposed, the session to come `null`.
        if is_provisional(&made.session_id)
            && let Some(mut identity) = read_night_identity(&folder.folder)
            && identity.pending.is_none()
        {
            identity.pending = Some(super::provisional::pending_night(proposal, session, &label));
            write_night_identity(&folder.folder, &identity)?;
        }
        let mut names = BTreeMap::new();
        names.insert(
            session.id.clone(),
            NightNaming {
                place: Some(label),
                session_id: Some(made.session_id.clone()),
                folder: Some(folder.folder.clone()),
            },
        );
        let plan = plan_organise_named(
            proposal,
            root,
            OrganiseMode::Symlink,
            std::slice::from_ref(&session.id),
            &names,
        )?;
        // `windows_no_link` does not stop the treatment: without developer
        // mode `organise` links by hard links (same drive), or says why not.
        let outcome = organise(&plan, &OrganiseOptions::default())?;
        nights.push(TreatmentNight {
            detected_id: session.id.clone(),
            session_id: made.session_id.clone(),
            evening_date: session.evening_date.clone(),
            night_folder: folder.folder.clone(),
            longitude,
            targets: targets
                .iter()
                .map(|(id, t)| TreatmentTarget {
                    target_id: id.clone(),
                    designation: t.designation.clone(),
                    folder_name: folder
                        .targets
                        .iter()
                        .find(|f| &f.target_id == id)
                        .map_or_else(
                            || crate::working_folder::target_folder_name(&t.designation),
                            |f| f.folder_name.clone(),
                        ),
                    frames: t.frames,
                })
                .collect(),
            linked: outcome.done,
            already_there: outcome.already_there,
            conflicts: plan.conflicts.len(),
        });
    }
    Ok(nights)
}
