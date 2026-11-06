//! Tauri-managed state: file paths and the upload-pause flag. Neither
//! holds behavior of its own -- see `commands.rs`, which is the only place
//! that reads them.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use astro_sync_core::local_work::{CancelFlag, WorkKind};
use astro_sync_core::service_wait::{WaitControl, WakePolicy};

/// Every file this GUI's own bookkeeping lives at, resolved once from
/// Tauri's app-config directory (see `paths::resolve`). None of these are
/// ever inside a folder the user associated -- they are Astro Sync's own
/// state, not a capture.
#[derive(Debug, Clone)]
pub struct AppPaths {
    pub settings: PathBuf,
    pub device_token: PathBuf,
    pub queue: PathBuf,
    /// The cached, timestamped `GET /v1/capabilities` snapshot -- not
    /// secret, unlike `device_token`: a plain JSON file, no keyring
    /// involved.
    pub capabilities_cache: PathBuf,
    /// The space this computer is signed in to by e-mail code -- or the
    /// sentence-to-be of a sign-in the service ended
    /// (`astro_sync_core::space::SpaceRecord`). Not secret.
    pub space_record: PathBuf,
    /// The identifier of this installation of Astro Sync, announced when it
    /// signs in (`astro_sync_core::space::load_or_create_installation_id`).
    pub installation_id: PathBuf,
    /// The error reports waiting to be sent to the hosted service
    /// (`astro_sync_core::error_report`), and the random number sent beside
    /// them -- not the installation identifier above, which the sign-in gave
    /// the service. Neither is secret.
    pub error_reports: PathBuf,
    pub diagnostic_id: PathBuf,
    /// The reference star list of the measured sky background, made by
    /// `astro-sync reference-stars`. Not secret.
    pub reference_stars: PathBuf,
    /// The SHA-256 of the files already read, by (path, size, time), so an
    /// action never reads every byte of a night again.
    pub digest_cache: PathBuf,
    /// The working folder chosen, the night folders found again, the UTC
    /// offsets of the logs (`astro_sync_core::local_settings`).
    pub local_settings: PathBuf,
    /// Astro Sync's own folder: the tools downloaded, the
    /// originals fetched, the solvers' work, the progression, the
    /// stacking's scratch, the nights' ledgers, the person's pipelines
    /// (`astro_sync_core::night_steps::DataPaths`).
    pub data_dir: PathBuf,
}

impl AppPaths {
    /// The files of the hosted sign-in, as the core wants them.
    #[must_use]
    pub fn space_files(&self) -> astro_sync_core::space::SpaceFiles {
        astro_sync_core::space::SpaceFiles {
            settings: self.settings.clone(),
            device_token: self.device_token.clone(),
            record: self.space_record.clone(),
            installation_id: self.installation_id.clone(),
        }
    }
}

/// The upload queue's pause flag. `upload_all` polls it between files (via
/// `astro_sync_core::gui_bridge::drive_queue`'s `should_continue`);
/// `upload_pause`/`upload_resume` flip it. A run already in flight keeps
/// running until the current file's own outcome is known -- pausing stops
/// the *next* file, not the current transfer -- which is safe because
/// every file is independently resumable.
#[derive(Debug, Clone, Default)]
pub struct UploadControl {
    pub paused: Arc<AtomicBool>,
}

/// How this app waits for a service that sleeps when nobody uses it and
/// answers the first request late.
/// `control` is the "Annuler" switch of every wait in progress
/// (`service_cancel`); the two policies are managed state rather than
/// constants only so tests can shorten them -- production uses the defaults.
#[derive(Debug, Clone)]
pub struct ServiceWait {
    pub control: WaitControl,
    /// For a call the person is waiting on.
    pub patient: WakePolicy,
    /// For the startup capability refresh when a profile is already cached:
    /// one short, silent attempt.
    pub background: WakePolicy,
}

impl Default for ServiceWait {
    fn default() -> Self {
        Self {
            control: WaitControl::default(),
            patient: WakePolicy::default(),
            background: WakePolicy::background_refresh(),
        }
    }
}

/// A work in progress: its kind when it has one, and its stop switch.
type ArmedWork = (Option<WorkKind>, CancelFlag);

/// The stop switches of the local works in progress. Each local command arms
/// a switch of its own when it starts ([`LocalWorkControl::arm`]), so works
/// that run together -- the stacking and the progression images of a sending
/// -- never wipe out each other's « Arrêter »; `local_work_cancel` flips
/// every switch still held by a work.
#[derive(Debug, Clone, Default)]
pub struct LocalWorkControl {
    works: Arc<Mutex<Vec<ArmedWork>>>,
}

impl LocalWorkControl {
    /// A new switch, armed, for a work that starts now. The switches of the
    /// works that have finished (nobody holds them but this list) are
    /// dropped on the way.
    pub fn arm(&self) -> CancelFlag {
        self.arm_as(None)
    }

    /// [`LocalWorkControl::arm`] for a work of a known kind, which
    /// [`LocalWorkControl::cancel_kind`] can stop without the others.
    pub fn arm_kind(&self, kind: WorkKind) -> CancelFlag {
        self.arm_as(Some(kind))
    }

    fn arm_as(&self, kind: Option<WorkKind>) -> CancelFlag {
        let mut works = self.works.lock().unwrap_or_else(|e| e.into_inner());
        works.retain(|(_, flag)| flag.holders() > 1);
        let flag = CancelFlag::new();
        works.push((kind, flag.clone()));
        flag
    }

    /// Whether a work of this kind is running now (its switch is still held
    /// by the work: the finished ones are forgotten on the way).
    pub fn is_running(&self, kind: WorkKind) -> bool {
        let mut works = self.works.lock().unwrap_or_else(|e| e.into_inner());
        works.retain(|(_, flag)| flag.holders() > 1);
        works.iter().any(|(of, _)| *of == Some(kind))
    }

    /// « Arrêter »: every local work in progress is asked to stop. A work
    /// that starts afterwards is not concerned.
    pub fn cancel_all(&self) {
        let mut works = self.works.lock().unwrap_or_else(|e| e.into_inner());
        works.retain(|(_, flag)| flag.holders() > 1);
        for (_, flag) in works.iter() {
            flag.cancel();
        }
    }

    /// « Arrêter l'empilement »: the works of this kind in progress are
    /// asked to stop, the others go on.
    pub fn cancel_kind(&self, kind: WorkKind) {
        let mut works = self.works.lock().unwrap_or_else(|e| e.into_inner());
        works.retain(|(_, flag)| flag.holders() > 1);
        for (_, flag) in works.iter().filter(|(of, _)| *of == Some(kind)) {
            flag.cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::LocalWorkControl;
    use astro_sync_core::local_work::WorkKind;

    #[test]
    fn stopping_the_stacking_leaves_the_other_works_running() {
        let control = LocalWorkControl::default();
        let stacking = control.arm_kind(WorkKind::Stacking);
        let progression = control.arm_kind(WorkKind::Progression);
        let unnamed = control.arm();
        control.cancel_kind(WorkKind::Stacking);
        assert!(stacking.is_cancelled());
        assert!(!progression.is_cancelled() && !unnamed.is_cancelled());
        // A stacking that starts afterwards (the next target) is not
        // concerned by a stop that came before it.
        let next = control.arm_kind(WorkKind::Stacking);
        assert!(!next.is_cancelled());
        // « Arrêter » still stops everything.
        control.cancel_all();
        assert!(progression.is_cancelled() && unnamed.is_cancelled() && next.is_cancelled());
    }

    #[test]
    fn a_work_is_running_while_something_holds_its_switch() {
        let control = LocalWorkControl::default();
        assert!(!control.is_running(WorkKind::Stacking));
        let stacking = control.arm_kind(WorkKind::Stacking);
        assert!(control.is_running(WorkKind::Stacking));
        assert!(!control.is_running(WorkKind::Progression));
        drop(stacking);
        assert!(!control.is_running(WorkKind::Stacking));
    }

    #[test]
    fn a_work_that_starts_never_wipes_out_the_stop_of_a_work_running_with_it() {
        let control = LocalWorkControl::default();
        let stacking = control.arm();
        control.cancel_all();
        assert!(stacking.is_cancelled());
        // The progression images start while the stacking is stopping.
        let progression = control.arm();
        assert!(
            stacking.is_cancelled(),
            "the second work reset the stop of the first"
        );
        assert!(
            !progression.is_cancelled(),
            "a work that starts after « Arrêter » is not stopped"
        );
    }

    #[test]
    fn arreter_stops_every_work_in_progress_and_forgets_the_finished_ones() {
        let control = LocalWorkControl::default();
        let first = control.arm();
        let second = control.arm();
        let finished = control.arm();
        drop(finished);
        control.cancel_all();
        assert!(first.is_cancelled() && second.is_cancelled());
        assert_eq!(control.works.lock().unwrap().len(), 2);
        // Once the first has finished too, the next arming forgets it.
        drop(first);
        let third = control.arm();
        assert!(!third.is_cancelled());
        assert_eq!(control.works.lock().unwrap().len(), 2);
    }
}
