//! What Astro Sync keeps about this computer's folders and logs, beside the
//! service settings (`gui_bridge::GuiSettings`): the working folder when it
//! was changed in Settings › General, the night folders found again outside
//! it ("Find the folder again"), the digest of the `ASSISTANT-IA.md` it
//! last wrote, and the UTC offset of the PHD2 and ASIAIR logs (the window
//! used to read every log at UTC+00:00, while the command line had
//! `--phd2-utc-offset` / `--asiair-utc-offset`).
//!
//! A file of its own (`local-settings.json`, next to `settings.json`), so
//! the window's settings round trip never drops these fields. A missing
//! file is the default: the system's root, UTC for both logs.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::analysis::AnalysisOptions;
use crate::analysis::log_offset::ForcedLogOffsets;
use crate::plan_image::PlanField;
use crate::stacking::{Colour, Contrast, PolishedEngine, SoftStars};
use crate::working_folder::{RootEnvironment, default_root};

#[derive(Debug, Error)]
pub enum LocalSettingsError {
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("invalid local settings JSON at {path}: {source}")]
    Invalid {
        path: PathBuf,
        source: serde_json::Error,
    },
    /// An offset must read `+HH:MM` or `-HH:MM`.
    #[error("the UTC offset must read +HH:MM or -HH:MM, got {0:?}")]
    BadUtcOffset(String),
}

impl LocalSettingsError {
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            LocalSettingsError::Io { .. } => "io_error",
            LocalSettingsError::Invalid { .. } => "invalid_settings_file",
            LocalSettingsError::BadUtcOffset(_) => "bad_utc_offset",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalSettings {
    /// The working folder chosen in Settings › General; `None`: the
    /// system's default root.
    #[serde(default)]
    pub working_folder: Option<PathBuf>,
    /// Night folders found again outside the root, by night identifier.
    #[serde(default)]
    pub found_again: BTreeMap<String, PathBuf>,
    /// The folder the person chose to follow when two folders carry the same
    /// night ("Follow this folder"), by night identifier.
    #[serde(default)]
    pub followed: BTreeMap<String, PathBuf>,
    /// SHA-256 of the `ASSISTANT-IA.md` Astro Sync last wrote: the file is
    /// rewritten only while it still has these bytes.
    #[serde(default)]
    pub assistant_file_sha256: Option<String>,
    /// The same for the other `.md` files Astro Sync writes at the root of
    /// the working folder (`README.md`, `GUIDE-ASTRO-SYNC.<language>.md`),
    /// by file name.
    #[serde(default)]
    pub managed_files_sha256: BTreeMap<String, String>,
    /// The clock of the PHD2 logs, `+HH:MM`; `None`: found from the frames.
    #[serde(default)]
    pub phd2_utc_offset: Option<String>,
    /// The clock of the ASIAIR logs, `+HH:MM`; `None`: found from the frames.
    #[serde(default)]
    pub asiair_utc_offset: Option<String>,
    /// The clock of the NINA logs (the computer that ran NINA), `+HH:MM`;
    /// `None`: found from the frames.
    #[serde(default)]
    pub nina_utc_offset: Option<String>,
    /// The contrast of the « Finition par défaut » (the slider of
    /// "Processing"): the starting point of every pipeline of every target,
    /// which a pipeline's own finish (`stacking::PipelineChoice::finish`)
    /// replaces, and the contrast of the plan image; `none` (the master of
    /// the catalogue), `presentation` (the window's default) or `strong`.
    #[serde(default = "window_contrast")]
    pub contrast: Contrast,
    /// The colour intensity of the default finish, night and plan (the
    /// second slider of "Processing"): `none` (the window's default),
    /// `enhanced` or `strong`.
    #[serde(default)]
    pub colour: Colour,
    /// The softened stars of the default finish, night and plan (the third
    /// slider of "Processing"): `none` (the window's default), `light` or
    /// `strong`.
    #[serde(default)]
    pub soft_stars: SoftStars,
    /// The engine of the "polished render" (Settings › General;
    /// `StackJob::polished_engine`): GraXpert's AI models (the
    /// default) or Siril alone.
    #[serde(default)]
    pub polished_engine: PolishedEngine,
    /// « Image du plan multi-nuits » (Settings › General): the frame the image
    /// of a multi-night plan is cut to -- clean edges (the default), the area
    /// every night covers, or the whole field (`plan_image::PlanField`).
    #[serde(default)]
    pub plan_field: PlanField,
    /// The last folders chosen: the window of
    /// "Detect sessions" and the two boxes of the sending's step 2
    /// open on them.
    #[serde(default)]
    pub last_folders: LastFolders,
    /// Whether this installation reports its errors to the hosted service
    /// (`crate::error_report`; the sentence of the setting names what is
    /// sent). Read only on the hosted service: a private server never
    /// receives a report, whatever this says.
    ///
    /// **The default is `false` until the switch is on the screen**
    /// ([`REPORT_ERRORS_DEFAULT`]): no build reports anything while the
    /// person cannot see the setting. With the switch, the default becomes
    /// `true` (the decision: on by default on the hosted service, with an
    /// opt-out in the settings) -- that one constant changes, and the
    /// reporting's own gate (`error_report::ReportingPolicy`) opens in the
    /// same change.
    #[serde(default = "report_errors_default")]
    pub report_errors: bool,
}

/// The value of [`LocalSettings::report_errors`] for an installation that
/// never chose. See the field: `false` while the setting has no switch on
/// the screen.
pub const REPORT_ERRORS_DEFAULT: bool = false;

fn report_errors_default() -> bool {
    REPORT_ERRORS_DEFAULT
}

/// The last folders the person chose, by place of choice; `None`: none yet.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LastFolders {
    /// "Detect sessions": the FITS, the calibrations, the logs.
    #[serde(default)]
    pub fits: Option<PathBuf>,
    #[serde(default)]
    pub calibration: Option<PathBuf>,
    #[serde(default)]
    pub logs: Option<PathBuf>,
    /// The sending of a session: "Location of the session's FITS files" and
    /// "Logs of the session" (the next session proposes them).
    #[serde(default)]
    pub session_fits: Option<PathBuf>,
    #[serde(default)]
    pub session_logs: Option<PathBuf>,
}

/// The contrast the window starts with: "Presentation 2.0".
#[must_use]
pub fn window_contrast() -> Contrast {
    Contrast::Presentation
}

impl Default for LocalSettings {
    fn default() -> Self {
        Self {
            working_folder: None,
            found_again: BTreeMap::new(),
            followed: BTreeMap::new(),
            assistant_file_sha256: None,
            managed_files_sha256: BTreeMap::new(),
            phd2_utc_offset: None,
            asiair_utc_offset: None,
            nina_utc_offset: None,
            contrast: window_contrast(),
            colour: Colour::None,
            soft_stars: SoftStars::None,
            polished_engine: PolishedEngine::default(),
            plan_field: PlanField::default(),
            last_folders: LastFolders::default(),
            report_errors: REPORT_ERRORS_DEFAULT,
        }
    }
}

impl LocalSettings {
    /// The working folder in force: the chosen one, else the system's.
    #[must_use]
    pub fn root(&self, env: &RootEnvironment) -> Option<PathBuf> {
        self.working_folder.clone().or_else(|| default_root(env))
    }

    /// The digest of the text Astro Sync last wrote in the `.md` file
    /// `name` of the working folder (`folder_files`).
    #[must_use]
    pub fn managed_file_sha256(&self, name: &str) -> Option<&str> {
        self.managed_files_sha256.get(name).map(String::as_str)
    }

    /// Keeps (or forgets, when `sha256` is none) the digest of the text
    /// Astro Sync wrote in the `.md` file `name`.
    pub fn record_managed_file(&mut self, name: &str, sha256: Option<String>) {
        match sha256 {
            Some(sha256) => {
                self.managed_files_sha256.insert(name.to_string(), sha256);
            }
            None => {
                self.managed_files_sha256.remove(name);
            }
        }
    }

    /// The folders found again, for `working_folder::locate_night`.
    #[must_use]
    pub fn remembered_folders(&self) -> Vec<PathBuf> {
        self.found_again.values().cloned().collect()
    }

    /// Sets both offsets, after checking each reads `+HH:MM` / `-HH:MM`.
    pub fn set_utc_offsets(
        &mut self,
        phd2: Option<&str>,
        asiair: Option<&str>,
    ) -> Result<(), LocalSettingsError> {
        let phd2 = check_offset(phd2)?;
        let asiair = check_offset(asiair)?;
        self.phd2_utc_offset = phd2;
        self.asiair_utc_offset = asiair;
        Ok(())
    }

    /// Turns the reporting of errors on or off (the switch of the settings).
    pub fn set_report_errors(&mut self, report: bool) {
        self.report_errors = report;
    }

    /// Sets the clock of the NINA logs, checked like the two others.
    pub fn set_nina_utc_offset(&mut self, nina: Option<&str>) -> Result<(), LocalSettingsError> {
        self.nina_utc_offset = check_offset(nina)?;
        Ok(())
    }

    /// The offsets the person gave, `None` where unset: Astro Sync finds those
    /// from the frames when it reads a night's logs
    /// ([`crate::analysis::log_offset`]).
    #[must_use]
    pub fn forced_log_offsets(&self) -> ForcedLogOffsets {
        ForcedLogOffsets {
            phd2: self.phd2_utc_offset.clone(),
            asiair: self.asiair_utc_offset.clone(),
            nina: self.nina_utc_offset.clone(),
        }
    }

    /// The analysis options these settings give (UTC where unset).
    #[must_use]
    pub fn analysis_options(&self) -> AnalysisOptions {
        let default = AnalysisOptions::default();
        AnalysisOptions {
            phd2_utc_offset: self
                .phd2_utc_offset
                .clone()
                .unwrap_or(default.phd2_utc_offset),
            asiair_utc_offset: self
                .asiair_utc_offset
                .clone()
                .unwrap_or(default.asiair_utc_offset),
            nina_utc_offset: self
                .nina_utc_offset
                .clone()
                .unwrap_or(default.nina_utc_offset),
        }
    }
}

/// `+HH:MM` / `-HH:MM`, or nothing (UTC).
fn check_offset(offset: Option<&str>) -> Result<Option<String>, LocalSettingsError> {
    match offset.map(str::trim).filter(|o| !o.is_empty()) {
        None => Ok(None),
        Some(offset) => crate::time_util::parse_fixed_offset_seconds(offset)
            .map(|_| Some(offset.to_string()))
            .ok_or_else(|| LocalSettingsError::BadUtcOffset(offset.to_string())),
    }
}

/// Loads the settings at `path`; a missing file is the default.
pub fn load(path: &Path) -> Result<LocalSettings, LocalSettingsError> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(LocalSettings::default());
        }
        Err(source) => {
            return Err(LocalSettingsError::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    serde_json::from_str(&raw).map_err(|source| LocalSettingsError::Invalid {
        path: path.to_path_buf(),
        source,
    })
}

/// Saves the settings at `path`, whole (written to a side file, then
/// renamed over: never half a file). These are Astro Sync's own files, in
/// its settings folder, never the person's tree.
pub fn save(path: &Path, settings: &LocalSettings) -> Result<(), LocalSettingsError> {
    let io = |source| LocalSettingsError::Io {
        path: path.to_path_buf(),
        source,
    };
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(io)?;
    }
    let json = serde_json::to_string_pretty(settings).expect("local settings serialize");
    let side = path.with_extension("json.part");
    fs::write(&side, json).map_err(io)?;
    fs::rename(&side, path).map_err(io)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;
    use crate::working_folder::HostSystem;

    #[test]
    fn a_missing_file_is_the_default_and_a_saved_one_comes_back() {
        let dir = TempDir::new("local-settings");
        let path = dir.path().join("local-settings.json");
        assert_eq!(load(&path).unwrap(), LocalSettings::default());
        let mut settings = LocalSettings::default();
        settings.set_utc_offsets(Some("+09:00"), None).unwrap();
        settings
            .found_again
            .insert("night-1".to_string(), dir.path().join("moved"));
        save(&path, &settings).unwrap();
        assert_eq!(load(&path).unwrap(), settings);
        assert!(!dir.path().join("local-settings.json.part").exists());
    }

    /// Reporting errors is off until the switch is on the screen, a file written
    /// before the field loads with that default, and a choice is kept.
    #[test]
    fn reporting_errors_is_off_by_default_and_a_choice_is_kept() {
        let dir = TempDir::new("local-settings-report-errors");
        let path = dir.path().join("local-settings.json");
        assert!(!LocalSettings::default().report_errors);
        fs::write(&path, r#"{"phd2UtcOffset":"+09:00"}"#).unwrap();
        let mut settings = load(&path).unwrap();
        assert_eq!(settings.report_errors, REPORT_ERRORS_DEFAULT);
        settings.set_report_errors(true);
        save(&path, &settings).unwrap();
        let written = fs::read_to_string(&path).unwrap();
        assert!(written.contains("\"reportErrors\": true"), "{written}");
        assert!(load(&path).unwrap().report_errors);
    }

    /// The contrast is "Presentation" until chosen, is kept
    /// with the other settings, and a file written before the option still
    /// loads; so is the engine of the "polished render" (GraXpert by default).
    #[test]
    fn the_contrast_and_the_polished_engine_have_their_defaults_and_are_kept() {
        let dir = TempDir::new("local-settings-contrast");
        let path = dir.path().join("local-settings.json");
        assert_eq!(LocalSettings::default().contrast, Contrast::Presentation);
        fs::write(&path, r#"{"phd2UtcOffset":"+09:00"}"#).unwrap();
        let mut settings = load(&path).unwrap();
        assert_eq!(settings.contrast, Contrast::Presentation);
        assert_eq!(settings.polished_engine, PolishedEngine::GraxpertAi);
        assert_eq!(settings.colour, Colour::None);
        assert_eq!(settings.soft_stars, SoftStars::None);
        settings.contrast = Contrast::None;
        settings.colour = Colour::Strong;
        settings.soft_stars = SoftStars::Light;
        settings.polished_engine = PolishedEngine::Siril;
        save(&path, &settings).unwrap();
        let written = fs::read_to_string(&path).unwrap();
        assert!(written.contains("\"contrast\": \"none\""), "{written}");
        assert!(written.contains("\"colour\": 40"), "{written}");
        assert!(written.contains("\"softStars\": 50"), "{written}");
        assert!(
            written.contains("\"polishedEngine\": \"siril\""),
            "{written}"
        );
        assert_eq!(load(&path).unwrap().contrast, Contrast::None);
        assert_eq!(load(&path).unwrap().colour, Colour::Strong);
        assert_eq!(load(&path).unwrap().soft_stars, SoftStars::Light);
        assert_eq!(load(&path).unwrap().polished_engine, PolishedEngine::Siril);
        fs::write(&path, r#"{"contrast":"extreme"}"#).unwrap();
        assert!(
            load(&path).is_err(),
            "an unknown word is not read as a choice"
        );
    }

    #[test]
    fn the_utc_offsets_are_checked_and_feed_the_analysis() {
        let mut settings = LocalSettings::default();
        assert_eq!(
            settings
                .set_utc_offsets(Some("UTC+9"), None)
                .unwrap_err()
                .code(),
            "bad_utc_offset"
        );
        assert_eq!(settings.phd2_utc_offset, None, "nothing kept on a refusal");
        settings
            .set_utc_offsets(Some("+09:00"), Some("-02:00"))
            .unwrap();
        let options = settings.analysis_options();
        assert_eq!(options.phd2_utc_offset, "+09:00");
        assert_eq!(options.asiair_utc_offset, "-02:00");
        settings.set_nina_utc_offset(Some("+01:00")).unwrap();
        assert_eq!(settings.analysis_options().nina_utc_offset, "+01:00");
        assert!(settings.set_nina_utc_offset(Some("1h")).is_err());
        assert_eq!(settings.nina_utc_offset.as_deref(), Some("+01:00"));
        settings.set_utc_offsets(None, Some(" ")).unwrap();
        assert_eq!(settings.analysis_options().asiair_utc_offset, "+00:00");
    }

    /// G1: an offset left empty in the settings is found from the frames (the
    /// command line does the same with an option left out); one given is forced.
    #[test]
    fn an_offset_left_empty_is_found_and_one_given_is_forced() {
        let mut settings = LocalSettings::default();
        assert_eq!(settings.forced_log_offsets(), ForcedLogOffsets::default());
        settings.set_utc_offsets(Some("+02:00"), None).unwrap();
        let forced = settings.forced_log_offsets();
        assert_eq!(forced.phd2.as_deref(), Some("+02:00"));
        assert_eq!(forced.asiair, None);
        assert_eq!(forced.nina, None);
    }

    #[test]
    fn the_root_is_the_chosen_one_else_the_systems() {
        let env = RootEnvironment {
            system: HostSystem::MacOs,
            home: Some(PathBuf::from("/maison/camille")),
            xdg_documents_dir: None,
            xdg_user_dirs: None,
            windows_documents: None,
        };
        let mut settings = LocalSettings::default();
        assert_eq!(
            settings.root(&env),
            Some(PathBuf::from("/maison/camille/Documents/Astro Sync"))
        );
        settings.working_folder = Some(PathBuf::from("/disque/Nuits"));
        assert_eq!(settings.root(&env), Some(PathBuf::from("/disque/Nuits")));
    }
}
