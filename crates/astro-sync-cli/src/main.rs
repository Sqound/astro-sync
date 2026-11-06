//! Thin command-line entry point over `astro_sync_core`. `associate`,
//! `scan`, `dry-run`, and `package`, plus `upload` and `queue status` and
//! `sign-in` (the hosted service's e-mail-code sign-in), each call
//! straight into the core crate and print either a readable table or, with
//! `--json`, the same JSON structure the Tauri GUI will consume.

mod cli;
mod cli_night;
mod commands;
mod detect_cli;
mod findings;
mod incidents_cli;
mod night;
mod output;
mod plan_cli;
mod pose_cli;
mod targets;
mod words;

use astro_sync_core::build_info;
use clap::{CommandFactory, FromArgMatches};
use cli::{Cli, Command};
use std::process::ExitCode;

fn main() -> ExitCode {
    // The version this program states to the portal on every request.
    astro_sync_core::client_identity::install(env!("CARGO_PKG_VERSION"));
    let cli = parse_cli();
    let result = match &cli.command {
        Command::Associate(args) => commands::associate(args),
        Command::Scan(args) => night::scan(args, commands::scan),
        Command::DryRun(args) => commands::dry_run(args),
        Command::Package(args) => commands::package(args),
        Command::Upload(args) => commands::upload(args),
        Command::SignIn(args) => commands::sign_in(args),
        Command::Pair(args) => commands::pair(args),
        Command::Queue(args) => commands::queue(&args.command),
        Command::Analyze(args) => commands::analyze(args),
        Command::Report(args) => commands::report(args),
        Command::Siril(args) => commands::siril(&args.command),
        Command::Progression(args) => commands::progression(args),
        Command::Sky(args) => commands::sky(args),
        Command::ReferenceStars(args) => commands::reference_stars(args),
        Command::WorkingFolder(args) => night::working_folder(args),
        Command::Nights(args) => night::nights(args),
        Command::NightFolder(args) => night::night_folder(args),
        Command::Check(args) => night::check(args),
        Command::SendNight(args) => night::send_night(args),
        Command::SendLogs(args) => night::send_logs(args),
        Command::Solve(args) => night::solve(args),
        Command::ImageTexts(args) => night::image_texts(args),
        Command::ProgressionImages(args) => night::progression_images(args),
        Command::Tools(args) => night::tools(args),
        Command::ListPipelines(args) => night::list_pipelines(args),
        Command::Stack(args) => night::stack(args),
        Command::MeasureMaster(args) => night::measure_master(args),
        Command::PoseStats(args) => pose_cli::pose_stats(args),
        Command::Incidents(args) => incidents_cli::incidents(args),
        Command::Integrations(args) => plan_cli::integrations(args),
        Command::PlanImage(args) => plan_cli::plan_image(args),
        Command::PlanProgression(args) => plan_cli::plan_progression(args),
        Command::PlanAttach(args) => plan_cli::plan_attach(args),
        Command::AssistantPrompt(args) => night::assistant_prompt(args),
        Command::Targets(args) => targets::targets(args),
        Command::NightTargets(args) => targets::night_targets(args),
        Command::FitsTarget(args) => targets::fits_target(args),
        Command::PersonalTarget(args) => targets::personal_target(args),
        Command::DetectSessions(args) => detect_cli::detect_sessions(args),
        Command::CreateSessions(args) => detect_cli::create_sessions(args),
        Command::TreatPrepare(args) => detect_cli::treat_prepare(args),
        Command::Organise(args) => detect_cli::organise(args),
        Command::ImportProvisional(args) => detect_cli::import_provisional(args),
        Command::ImportSettle(args) => detect_cli::import_settle(args),
        Command::ImportDiscard(args) => detect_cli::import_discard(args),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let locale = words::requested_language();
            let sentence = words::error_text(error.as_ref(), locale);
            eprintln!("{sentence}");
            ExitCode::FAILURE
        }
    }
}

/// `astro-sync --version` prints `astro-sync 1.0.0 (6aa28f767, 2026-10-13)`:
/// the version, then the commit and day stamped at build time, the same
/// `build_info` the window shows in its menu.
fn parse_cli() -> Cli {
    let version = build_info::build_info(env!("CARGO_PKG_VERSION")).cli_version();
    // One string for the life of the process: clap keeps a `'static` version.
    let version: &'static str = Box::leak(version.into_boxed_str());
    let mut matches = Cli::command().version(version).get_matches();
    match Cli::from_arg_matches_mut(&mut matches) {
        Ok(cli) => cli,
        Err(error) => error.exit(),
    }
}
