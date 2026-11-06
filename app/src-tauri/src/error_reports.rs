//! The reports of errors to the hosted service: the glue between the
//! application and `astro_sync_core::error_report`, which holds every rule
//! (what a report says, the queue, the gates, the sending).
//!
//! **Nothing is reported while the setting has no switch on the screen.**
//! [`SWITCH_ON_SCREEN`] is `false` until Settings carries "send the error
//! logs of the application"; the change that adds the switch sets it to
//! `true` and turns the default of `report_errors` to `true`
//! (`local_settings::REPORT_ERRORS_DEFAULT`). Until then the panic hook and
//! the command below are in place and do nothing.

use std::panic::{catch_unwind, AssertUnwindSafe};

use astro_sync_core::error_report::{
    self, scrub, ErrorKind, ErrorReportFiles, HttpErrorReportTransport, RawError, ReportingPolicy,
};
use astro_sync_core::gui_bridge;
use tauri::{AppHandle, Runtime};

use crate::error::CommandError;
use crate::paths;
use crate::state::AppPaths;

/// Whether the setting is on the screen (see the module).
pub const SWITCH_ON_SCREEN: bool = false;

const POLICY: ReportingPolicy = ReportingPolicy {
    switch_on_screen: SWITCH_ON_SCREEN,
};

fn files_of(paths: &AppPaths) -> ErrorReportFiles {
    ErrorReportFiles {
        settings: paths.settings.clone(),
        local_settings: paths.local_settings.clone(),
        queue: paths.error_reports.clone(),
        diagnostic_id: paths.diagnostic_id.clone(),
    }
}

/// Queues the stop of the program when it happens. The previous hook (the
/// message on the standard error) still runs first. A failure of the report
/// itself is swallowed: reporting must never be the reason for a second stop.
pub fn install_panic_hook<R: Runtime>(app: &AppHandle<R>) {
    let Ok(paths) = paths::resolve(app) else {
        return;
    };
    let files = files_of(&paths);
    let version = app.package_info().version.to_string();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        previous(info);
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let payload = info.payload();
            let message = payload
                .downcast_ref::<&str>()
                .map(|text| (*text).to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "panic".to_string());
            let stack = info
                .location()
                .map(|at| scrub::panic_location(at.file(), at.line(), at.column()));
            let _ = error_report::record(
                &files,
                POLICY,
                &version,
                RawError {
                    kind: ErrorKind::Panic,
                    message: &message,
                    stack: stack.as_deref(),
                },
            );
        }));
    }));
}

/// Sends what waits, on a thread of its own: the application does not wait
/// for the service.
pub fn flush_in_background<R: Runtime>(app: AppHandle<R>) {
    if !SWITCH_ON_SCREEN {
        return;
    }
    std::thread::spawn(move || {
        let _ = flush_now(&app);
    });
}

fn flush_now<R: Runtime>(app: &AppHandle<R>) -> Result<(), CommandError> {
    let paths = paths::resolve(app)?;
    let files = files_of(&paths);
    // The keychain is asked for the token only where a report may be sent.
    let token = if error_report::reporting_allowed(&files, POLICY) {
        gui_bridge::load_settings(&paths.settings)
            .ok()
            .and_then(|settings| settings.portal_url)
            .and_then(|portal| {
                gui_bridge::load_device_token(&paths.device_token, &portal)
                    .ok()
                    .flatten()
            })
    } else {
        None
    };
    let transport =
        HttpErrorReportTransport::new().map_err(|error| CommandError::other(error.to_string()))?;
    error_report::flush(&files, POLICY, token.as_deref(), &transport)
        .map_err(|error| CommandError::other(error.to_string()))?;
    Ok(())
}

/// An uncaught error of the window (`window.onerror`, an unhandled promise):
/// queued if the reporting is allowed, then what waits is sent. The text is
/// cleaned here, in the core, whatever the window sends.
#[tauri::command]
pub fn error_report_record<R: Runtime>(
    app: AppHandle<R>,
    message: String,
    stack: Option<String>,
) -> Result<(), CommandError> {
    let paths = paths::resolve(&app)?;
    let version = app.package_info().version.to_string();
    let recorded = error_report::record(
        &files_of(&paths),
        POLICY,
        &version,
        RawError {
            kind: ErrorKind::Ui,
            message: &message,
            stack: stack.as_deref(),
        },
    )
    .map_err(|error| CommandError::other(error.to_string()))?;
    if matches!(recorded, error_report::Recorded::Queued(_)) {
        flush_in_background(app.clone());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wake_tests::{fast, harness, serialized};

    /// While the setting has no switch on the screen, an installation whose
    /// settings file says "report" (written by hand) queues and sends
    /// nothing: not a window error, not a stop of the program.
    #[test]
    fn nothing_is_queued_while_the_switch_is_not_on_the_screen() {
        let _guard = serialized();
        let harness = harness(fast());
        // On the hosted service, with the setting on.
        harness.pair_with("https://portal.planastro.space");
        std::fs::write(
            harness.config_dir.join("local-settings.json"),
            r#"{"reportErrors":true}"#,
        )
        .unwrap();
        const { assert!(!SWITCH_ON_SCREEN, "the switch is not in the settings yet") };
        error_report_record(
            harness.app.handle().clone(),
            "Cannot read properties of undefined".to_string(),
            Some("render@tauri://localhost/assets/index.js:1:2".to_string()),
        )
        .unwrap();
        assert!(!harness.config_dir.join("error-reports.json").exists());
        assert!(!harness.config_dir.join("diagnostic-id").exists());
        // The panic hook does the same.
        install_panic_hook(harness.app.handle());
        let _ = std::panic::catch_unwind(|| panic!("boom"));
        let _ = std::panic::take_hook();
        assert!(!harness.config_dir.join("error-reports.json").exists());
    }
}
