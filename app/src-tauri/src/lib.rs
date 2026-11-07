// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[cfg(test)]
mod build_info_tests;
mod commands;
mod commands_detect;
mod commands_incidents;
mod commands_master;
mod commands_multi_night;
mod commands_night;
mod commands_poses;
mod commands_processing;
mod commands_sky;
mod commands_targets;
#[cfg(test)]
mod detect_commands_tests;
mod dialogs;
mod error;
mod error_reports;
#[cfg(test)]
mod local_work_tests;
#[cfg(test)]
mod night_commands_tests;
mod paths;
#[cfg(test)]
mod processing_commands_tests;
#[cfg(test)]
mod sky_tests;
#[cfg(test)]
mod space_tests;
mod state;
#[cfg(test)]
mod wake_tests;

use state::{LocalWorkControl, ServiceWait, UploadControl};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(UploadControl::default())
        .manage(ServiceWait::default())
        .manage(LocalWorkControl::default())
        .manage(commands_sky::SkyAfterUpload)
        .setup(|app| {
            // A resolution the last run left going is over.
            commands_sky::end_interrupted(app.handle());
            // What the application says it is to the portal (the version of
            // the window) must be known before the first request goes out.
            astro_sync_core::client_identity::install(&app.package_info().version.to_string());
            // The reports of errors: a stop of the program is queued, and
            // what waits from the last run is sent (both only where the
            // reporting is allowed -- see `error_reports`).
            error_reports::install_panic_hook(app.handle());
            error_reports::flush_in_background(app.handle().clone());
            // A provisional folder whose session was created and whose rename
            // the last run left half done is finished.
            let handle = app.handle().clone();
            std::thread::spawn(move || commands_detect::reconcile_provisional(&handle));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_build_info,
            commands::settings_get,
            commands::settings_set,
            commands::device_status,
            commands::device_pair,
            commands::device_unpair,
            commands::service_announcement,
            commands::space_status,
            commands::space_request_code,
            commands::space_verify_code,
            commands::space_refresh,
            commands::space_rename,
            commands::space_detach,
            commands::capabilities,
            commands::list_sessions,
            commands::choose_folder,
            commands::scan_folder,
            commands::dry_run_folder,
            commands::produce_package,
            commands::enqueue_folder,
            commands::queue_status,
            commands::upload_next,
            commands::upload_all,
            commands::upload_pause,
            commands::upload_resume,
            commands::upload_requeue_missing,
            commands::upload_resend,
            commands::queue_forget_before_link,
            commands_sky::sky_retry,
            commands::service_cancel,
            commands::local_work_cancel,
            commands::local_work_cancel_kind,
            commands::local_settings_get,
            error_reports::error_report_record,
            commands::local_settings_set_utc_offsets,
            commands::local_settings_set_contrast,
            commands::local_settings_set_colour,
            commands::local_settings_set_soft_stars,
            commands::local_settings_set_polished_engine,
            commands::local_settings_set_plan_field,
            commands::local_settings_set_renders_upload,
            commands::local_settings_set_last_folders,
            commands::download_reference_stars,
            commands_night::working_folder_get,
            commands_night::working_folder_prepare,
            commands_night::working_folder_replace_assistant_file,
            commands_night::working_folder_set,
            commands_night::guide_write,
            commands_night::managed_file_replace,
            commands_night::guide_prompt,
            commands_night::night_folders_list,
            commands_night::night_locate,
            commands_night::night_found_again,
            commands_night::night_follow,
            commands_night::night_folder_plan,
            commands_night::night_folder_prepare,
            commands_night::night_detect,
            commands_night::night_fingerprint,
            commands_night::night_online,
            commands_night::night_view,
            commands_night::night_precheck,
            commands_night::night_enqueue,
            commands_night::night_analysis,
            commands_night::night_send_logs,
            commands_night::night_record_sending,
            commands_night::night_solve,
            commands_night::sky_missing,
            commands_night::sky_solve_capture,
            commands_night::tools_detect,
            commands_night::tools_download,
            commands_night::tools_install_all,
            commands_night::tools_verify,
            commands_night::python_env_install,
            commands_processing::python_env_reinstall,
            commands_processing::processing_files_scan,
            commands_processing::processing_files_clean,
            commands_night::pipelines_list,
            commands_night::pipeline_sections,
            commands_night::pipeline_trust_set,
            commands_night::stacking_run,
            commands_night::stacking_frame,
            commands_night::progression_families,
            commands_night::progression_images_run,
            commands_night::progression_images_candidates,
            commands_night::assistant_prompt,
            commands_night::night_detail,
            commands_night::night_create,
            commands_targets::targets_search,
            commands_targets::night_targets_change,
            commands_targets::fits_choose,
            commands_master::master_measure,
            commands_master::master_measure_current,
            commands_master::master_measure_forget,
            commands_incidents::incidents_apply,
            commands_incidents::incidents_declared,
            commands_incidents::incidents_detect,
            commands_incidents::incidents_event_ids,
            commands_poses::pose_stats_plan,
            commands_poses::pose_stats_run,
            commands_targets::fits_target_guess,
            commands_targets::personal_target_create,
            commands_night::folder_open,
            commands_night::pipelines_folder_open,
            commands_multi_night::night_multi_night,
            commands_multi_night::night_integration_preview,
            commands_multi_night::night_integrations_send,
            commands_multi_night::plan_image_run,
            commands_multi_night::plan_image_run_local,
            commands_multi_night::plan_framing_read,
            commands_multi_night::multi_night_tags,
            commands_multi_night::night_attachment,
            commands_multi_night::night_plan_choices,
            commands_multi_night::night_plan_search,
            commands_multi_night::night_plan_attach,
            commands_multi_night::night_plan_detach,
            commands_multi_night::night_plan_create,
            commands_detect::sessions_user_data,
            commands_detect::sessions_detect,
            commands_detect::sessions_refresh_rights,
            commands_detect::sessions_organise_plan,
            commands_detect::sessions_organise,
            commands_detect::sessions_create_bulk,
            commands_detect::space_storage_get,
            commands_detect::detect_treatments_prepare,
            commands_detect::sessions_provisional,
            commands_detect::detect_import_create,
            commands_detect::detect_import_plans,
            commands_detect::detect_import_discard,
            commands_detect::detect_images_send,
            commands_detect::result_file_open,
            commands_detect::result_file_reveal,
            commands_detect::result_file_preview,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
