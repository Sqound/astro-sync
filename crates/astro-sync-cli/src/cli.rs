//! Argument definitions for every subcommand: the quartet (`associate`,
//! `scan`, `dry-run`, `package`), `upload` and `queue status`, and `sign-in`.
//! Each subcommand is a thin wrapper -- see `crate::commands` -- over the matching
//! `astro-sync-core` function; no subcommand here ever reimplements behavior
//! that belongs in the core crate.

use astro_sync_core::space::HOSTED_SERVICE_ORIGIN;
use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

/// The language of the sentences a refusal or a failure is said in (the ten
/// dictionaries of the application); read from the arguments by `crate::words`.
#[derive(Args, Debug, Clone, Default)]
pub struct LangArg {
    /// Language of the messages, the tables and the names of the tools (en, fr, ja, de, es, it,
    /// pt-BR, ru, zh-Hans, zh-Hant); defaults to ASTRO_SYNC_LANG, then the computer's language,
    /// then English.
    #[arg(long = "lang", value_name = "LANGUAGE")]
    pub lang: Option<String>,
}

#[derive(Parser, Debug)]
#[command(
    name = "astro-sync",
    version,
    about = "Astro Sync desktop bridge (CLI)"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Link the identifier of a hosted run to a local folder.
    Associate(AssociateArgs),
    /// Walk a folder read-only (classification, sizes, SHA-256); with --detect, the rules of a night folder.
    Scan(crate::cli_night::ScanArgs),
    /// Compute the dry-run report: what would be sent, without transferring anything.
    DryRun(FolderCommandArgs),
    /// Produce the open package of a night (`astro-session-package/`) in a chosen location.
    Package(PackageArgs),
    /// Send the pending files (resumable protocol, automatic resume).
    Upload(UploadArgs),
    /// Sign in to the hosted service with an e-mail address and a six-digit code, without a password.
    SignIn(SignInArgs),
    /// Connect this computer to a private server with the eight-character code its account page shows.
    Pair(PairArgs),
    /// Operations on the persistent upload queue.
    Queue(QueueArgs),
    /// Analyse locally the guiding logs (PHD2, ASIAIR) and FITS headers of a folder.
    Analyze(AnalyzeArgs),
    /// Generate the night's combined analysis report and write it to a JSON file.
    Report(ReportArgs),
    /// Siril operations (detecting the installation).
    Siril(SirilArgs),
    /// Generate and run a Siril progression for an acquisition family.
    Progression(ProgressionArgs),
    /// Measure the night's sky background on the solved frames of a folder, and send it (free).
    Sky(SkyArgs),
    /// Build the reference star list of the sky background: Tycho-2 (downloaded with your agreement) or a copy of Hipparcos or Tycho-2.
    ReferenceStars(ReferenceStarsArgs),
    /// The working folder (Documents/Astro Sync) and its ASSISTANT-IA.md.
    WorkingFolder(crate::cli_night::WorkingFolderArgs),
    /// The nights of the working folder (by their identity file).
    Nights(crate::cli_night::NightsArgs),
    /// The "Folder" step: the folder of a night, with only the ticked targets.
    NightFolder(crate::cli_night::NightFolderArgs),
    /// The summary: what becomes of each image, the service's answer, the four limits (nothing is sent).
    Check(crate::cli_night::CheckArgs),
    /// Send a night or its additions: only the results leave (final or stacked images, report, sky solutions).
    SendNight(crate::cli_night::SendNightArgs),
    /// Analyse the logs of a night and send the report alone (the raw logs stay).
    SendLogs(crate::cli_night::SendLogsArgs),
    /// Compute the sky solution of an image (header, ASTAP, astrometry.net) and send it.
    Solve(crate::cli_night::SolveArgs),
    /// Send the title and description of the images of captures already online in the ten
    /// languages, recomputed from what Astro Sync recorded beside its renders.
    ImageTexts(crate::cli_night::ImageTextsArgs),
    /// The progression images of a target (Siril, fixed settings), in the Astro Sync data.
    ProgressionImages(crate::cli_night::ProgressionImagesArgs),
    /// Settings › Dependencies: the tools of the registry, their state, their download.
    Tools(crate::cli_night::ToolsArgs),
    /// Measure the quality of a linear master (width and roundness of the stars), like the window does.
    MeasureMaster(crate::cli_night::MeasureMasterArgs),
    /// Measure each frame of a target (stars, sky background) and flag those that stray from the rest of the night.
    PoseStats(crate::pose_cli::PoseStatsArgs),
    /// The incidents of a target: pauses (flip, gap between frames), loss of the guide star, guiding waits, exposure, temperature, spikes; with --session, the list merged with the declared reports (nothing is written).
    Incidents(crate::incidents_cli::IncidentsArgs),
    /// The rendering pipelines of the stacking.
    ListPipelines(crate::cli_night::ListPipelinesArgs),
    /// The stack of a target in <target>/stacked/ (chosen pipelines), the master of each family kept.
    Stack(crate::cli_night::StackArgs),
    /// The integrations of a night for a multi-night plan: lines, never a file (Advanced Sync+).
    Integrations(crate::plan_cli::IntegrationsArgs),
    /// The image of a multi-night plan: the masters of the nights aligned and combined here (HaRGB), then the rendering.
    PlanImage(crate::plan_cli::PlanImageArgs),
    /// The progression of a multi-night plan: its image after each night (the capture is already
    /// online). The same options as `plan-image`; `plan-image --progression` makes it with the
    /// images.
    PlanProgression(crate::plan_cli::PlanImageArgs),
    /// Attach a night to a multi-night plan (or detach it, or create a plan with it); search for a plan by its target's name.
    PlanAttach(crate::plan_cli::PlanAttachArgs),
    /// The prompt of "Ask an AI assistant", in the language of the application.
    AssistantPrompt(crate::cli_night::AssistantPromptArgs),
    /// The "Targets" step: search for a target in the catalogue (8 results, best placed first).
    Targets(crate::targets::TargetsArgs),
    /// The "Targets" step: add a target to a night (the other targets stay).
    NightTargets(crate::targets::NightTargetsArgs),
    /// The "Targets" step: read the header of a frame (FITS, XISF) to derive the target; nothing from the file is sent.
    FitsTarget(crate::targets::FitsTargetArgs),
    /// The "Targets" step: create an off-catalogue target in the space, and add it to a night.
    PersonalTarget(crate::targets::PersonalTargetArgs),
    /// "Detect sessions": read the FITS files, calibrations and logs of a folder and propose nights, sessions, plans and multi-night plans; nothing is written.
    DetectSessions(crate::detect_cli::DetectSessionsArgs),
    /// "Create ...": write to the service what `detect-sessions` proposed (plans, "Executed" sessions, multi-night plans), in one idempotent call.
    CreateSessions(crate::detect_cli::CreateSessionsArgs),
    /// "Start the treatments": the created sessions become nights of the working folder, their frames linked (never moved).
    TreatPrepare(crate::detect_cli::TreatPrepareArgs),
    /// Tidy the files of the detected sessions: leave in place, copy, move (checked by SHA-256) or link.
    Organise(crate::detect_cli::OrganiseArgs),
    /// What "Create …" would make of the proposal, computed here: a provisional identifier (`local-…`) per session and plan to create; nothing is written.
    ImportProvisional(crate::detect_cli::ImportProvisionalArgs),
    /// After the creation: the provisional folders take the name of their session (and of their plan), their references follow.
    ImportSettle(crate::detect_cli::ImportSettleArgs),
    /// "Abandon the import": the provisional folders removed (Astro Sync's links and results only).
    ImportDiscard(crate::detect_cli::ImportDiscardArgs),
}

#[derive(Args, Debug)]
pub struct AssociateArgs {
    /// Identifier of the hosted run.
    #[arg(long)]
    pub session_id: String,
    /// JSON configuration file where the association is persisted (never in the source folder).
    #[arg(long)]
    pub config: PathBuf,
    /// Existing folder to associate as is.
    #[arg(long)]
    pub folder: Option<PathBuf>,
    /// Name of the new folder to create (used with --parent).
    #[arg(long)]
    pub new_folder: Option<String>,
    /// Parent folder chosen by the user for --new-folder.
    #[arg(long)]
    pub parent: Option<PathBuf>,
    /// JSON output (machine-readable) instead of the default human-readable message.
    #[arg(long)]
    pub json: bool,
}

/// How `scan`, `dry-run`, and `package` locate the folder to operate on:
/// either a direct `--folder`, or a previously saved association looked up
/// by `--config` + `--session-id`.
#[derive(Args, Debug)]
pub struct FolderSource {
    /// Folder to process directly.
    #[arg(long)]
    pub folder: Option<PathBuf>,
    /// Configuration file containing saved associations (with --session-id).
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// Identifier of a run already associated in --config.
    #[arg(long)]
    pub session_id: Option<String>,
}

#[derive(Args, Debug)]
pub struct FolderCommandArgs {
    #[command(flatten)]
    pub folder_source: FolderSource,
    /// JSON output (machine-readable) instead of the default human-readable table.
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct PackageArgs {
    #[command(flatten)]
    pub folder_source: FolderSource,
    /// Location (already existing) where to write astro-session-package/; never in the source folder.
    #[arg(long)]
    pub output: PathBuf,
    /// Descriptive label of the observing night (never an identifier).
    #[arg(long)]
    pub session_label: Option<String>,
    #[arg(long)]
    pub session_started_at: Option<String>,
    #[arg(long)]
    pub session_finished_at: Option<String>,
    /// Name of the producing tool, for information only.
    #[arg(long, default_value = "astro-sync-cli")]
    pub producer_name: String,
    #[arg(long)]
    pub producer_version: Option<String>,
    /// JSON output (machine-readable) instead of the default human-readable summary.
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct UploadArgs {
    /// SQLite database file of the queue (created on first use).
    #[arg(long)]
    pub queue: PathBuf,
    /// Base URL of the service (the hosted service, or the address of your private server: e.g. https://astro.example).
    #[arg(long)]
    pub portal: String,
    /// File containing the device token -- never an argument that gets displayed.
    #[arg(long)]
    pub token: PathBuf,
    /// Folder to scan and enqueue before sending (optional:
    /// without this option, `upload` only resumes the existing queue).
    #[arg(long)]
    pub folder: Option<PathBuf>,
    /// Identifier of the hosted run to associate with the sent files.
    #[arg(long)]
    pub session_id: Option<String>,
    /// Plan identifier to associate.
    #[arg(long)]
    pub plan_id: Option<String>,
    /// Target identifier to associate (repeatable).
    #[arg(long = "target-id")]
    pub target_ids: Vec<String>,
    /// JSON output (machine-readable) instead of the default human-readable summary.
    #[arg(long)]
    pub json: bool,
    #[command(flatten)]
    pub language_of_messages: crate::cli::LangArg,
}

/// `pair`: the code a private server shows on its account page ("Pair a new
/// device"), exchanged for the device token `upload --token` reads. The way in
/// for a private server that signs in with a password; one that signs in by
/// e-mail code is reached with `sign-in`.
#[derive(Args, Debug)]
pub struct PairArgs {
    /// Base URL of the private server (e.g. https://astro.example).
    #[arg(long)]
    pub portal: String,
    /// File where to store the device token (readable by you alone): the one `upload --token` reads.
    #[arg(long)]
    pub token: PathBuf,
    /// Name of this computer in the server's devices (defaults to "Astro Sync — " followed by the machine name).
    #[arg(long)]
    pub device_name: Option<String>,
    /// JSON output (machine-readable) instead of the default human-readable summary. The token never appears in it.
    #[arg(long)]
    pub json: bool,
    #[command(flatten)]
    pub language_of_messages: LangArg,
}

/// `sign-in`: the sign-in of the hosted service -- an e-mail address, then
/// the code of six digits received by e-mail -- announcing itself as the application `sync`. `associate` stays for private
/// servers; the token this command keeps is the file `upload --token` reads.
#[derive(Args, Debug)]
pub struct SignInArgs {
    /// Base URL of the service (defaults to the hosted service).
    #[arg(long, default_value = HOSTED_SERVICE_ORIGIN)]
    pub portal: String,
    /// File where to store the device token (readable by you alone): the one `upload --token` reads.
    #[arg(long)]
    pub token: PathBuf,
    /// E-mail address. Without this option, it is asked for (or read from standard input).
    #[arg(long)]
    pub email: Option<String>,
    /// Language of the message received: fr, en or ja (defaults to the environment's, otherwise en).
    #[arg(long)]
    pub language: Option<String>,
    /// Name of this computer in "My devices" (defaults to "Astro Sync — " followed by the machine name).
    #[arg(long)]
    pub device_name: Option<String>,
    /// File of this installation's identifier (defaults to `installation-id`, next to the token file).
    #[arg(long)]
    pub installation_id: Option<PathBuf>,
    /// Make the code be sent, then stop, without waiting for anything (usable without a screen: the
    /// code is then given with `--verify-only`).
    #[arg(long, conflicts_with = "verify_only")]
    pub send_only: bool,
    /// Send nothing: read the code already received from standard input (never as an argument, so
    /// that it does not stay in the terminal history) and verify it.
    #[arg(long)]
    pub verify_only: bool,
    /// JSON output (machine-readable) instead of the default human-readable summary. The token never appears in it.
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct QueueArgs {
    #[command(subcommand)]
    pub command: QueueCommand,
}

#[derive(Subcommand, Debug)]
pub enum QueueCommand {
    /// List the state of each file known to the queue.
    Status(QueueStatusArgs),
}

#[derive(Args, Debug)]
pub struct QueueStatusArgs {
    /// SQLite database file of the queue.
    #[arg(long)]
    pub queue: PathBuf,
    /// JSON output (machine-readable) instead of the default human-readable table.
    #[arg(long)]
    pub json: bool,
    #[command(flatten)]
    pub language_of_messages: LangArg,
}

#[derive(Args, Debug)]
pub struct AnalyzeArgs {
    /// Folder to analyse (PHD2/ASIAIR logs, FITS headers); read-only.
    #[arg(long)]
    pub folder: PathBuf,
    /// Already existing folder where to write metrics/quality.csv, metrics/guiding.csv
    /// and events/session.jsonl.
    #[arg(long)]
    pub out: PathBuf,
    /// Fixed UTC offset of the timestamps of the PHD2 logs found (e.g. "+09:00").
    #[arg(long, default_value = "+00:00")]
    pub phd2_utc_offset: String,
    /// Fixed UTC offset of the timestamps of the ASIAIR logs found (e.g. "+09:00").
    #[arg(long, default_value = "+00:00")]
    pub asiair_utc_offset: String,
    /// Fixed UTC offset of the timestamps of the NINA logs found (the time of the
    /// computer that ran NINA, e.g. "+09:00").
    #[arg(long, default_value = "+00:00")]
    pub nina_utc_offset: String,
    /// JSON output (machine-readable, `analysis::AnalysisOutcome`) instead of the human-readable summary.
    #[arg(long)]
    pub json: bool,
}

/// `report --folder … --out report.json`: the same combined
/// session-analysis report the GUI's Analysis screen generates and uploads
/// (`analysis::report::generate_report`), written to a chosen file rather
/// than uploaded -- lets the report be produced, inspected, or fed to
/// another tool from the CLI/scripts without the GUI or a paired portal.
#[derive(Args, Debug)]
pub struct ReportArgs {
    /// Folder to analyse (PHD2/ASIAIR logs, FITS headers); read-only.
    #[arg(long)]
    pub folder: PathBuf,
    /// File where to write the JSON report (created or replaced; the parent folder must exist).
    #[arg(long)]
    pub out: PathBuf,
    /// UTC offset of the timestamps of the PHD2 logs (e.g. "+09:00"); without this
    /// option, the time is found from the frames, like `send-logs`.
    #[arg(long)]
    pub phd2_utc_offset: Option<String>,
    /// UTC offset of the timestamps of the ASIAIR logs (e.g. "+09:00"); without this
    /// option, the time is found from the frames, like `send-logs`.
    #[arg(long)]
    pub asiair_utc_offset: Option<String>,
    /// UTC offset of the timestamps of the NINA logs (the time of the computer that
    /// ran NINA, e.g. "+09:00"); without this option, the time is found from the frames,
    /// like `send-logs`.
    #[arg(long)]
    pub nina_utc_offset: Option<String>,
    /// Also print the report (JSON, machine-readable) on standard output.
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct SirilArgs {
    #[command(subcommand)]
    pub command: SirilCommand,
}

#[derive(Subcommand, Debug)]
pub enum SirilCommand {
    /// Detect Siril (standard locations, manual selection, version check).
    Detect(SirilDetectArgs),
}

#[derive(Args, Debug)]
pub struct SirilDetectArgs {
    /// Path manually chosen for the Siril executable; tried before the
    /// standard locations and the PATH.
    #[arg(long)]
    pub siril_path: Option<PathBuf>,
    /// JSON output (machine-readable, `analysis::siril::SirilDetection`) instead of the human-readable summary.
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct ProgressionArgs {
    /// Source folder (read-only) containing the frames of the chosen family.
    #[arg(long)]
    pub folder: PathBuf,
    /// Key of the acquisition family to stack (see `analyze`), e.g.
    /// "Ha_300s_g100_1x1_t-10C".
    #[arg(long)]
    pub family: String,
    /// Already existing folder where to write the isolated working directory and the progression.
    #[arg(long)]
    pub out: PathBuf,
    /// Path manually chosen for the Siril executable; tried before the
    /// standard locations and the PATH.
    #[arg(long)]
    pub siril_path: Option<PathBuf>,
    /// JSON output (machine-readable, `analysis::siril::ProgressionOutcome`) instead of the human-readable summary.
    #[arg(long)]
    pub json: bool,
}

/// `sky --folder …`: the measured sky background of the night, from the
/// solved light frames of a folder, read on this computer; with `--portal`
/// and `--token`, the night's one value is sent to the service
/// (`POST /v1/place-sky-measures`, free). The frames never leave the
/// computer.
#[derive(Args, Debug)]
pub struct SkyArgs {
    /// Folder of the frames (read-only); its darks and bias serve as the pedestal.
    #[arg(long)]
    pub folder: PathBuf,
    /// Reference star list (made by `reference-stars`).
    #[arg(long)]
    pub stars: Option<PathBuf>,
    /// Latitude of the site, in degrees, for frames whose header gives none.
    #[arg(long, requires = "site_longitude", allow_hyphen_values = true)]
    pub site_latitude: Option<f64>,
    /// Longitude of the site, in degrees (positive east), with --site-latitude.
    #[arg(long, requires = "site_latitude", allow_hyphen_values = true)]
    pub site_longitude: Option<f64>,
    /// Sensor pedestal per pixel, in ADU, if you know it (otherwise: header, dark or bias of the folder).
    #[arg(long)]
    pub pedestal: Option<f64>,
    /// Base URL of the service: with --token, sends the night's value.
    #[arg(long, requires = "token")]
    pub portal: Option<String>,
    /// File containing the device token (the one from `sign-in` or from the association).
    #[arg(long, requires = "portal")]
    pub token: Option<PathBuf>,
    /// Run of the night: the site and the date then come from its plan.
    #[arg(long)]
    pub session_id: Option<String>,
    /// The night to send (YYYY-MM-DD) when the folder contains several.
    #[arg(long)]
    pub night: Option<String>,
    /// JSON output (machine-readable) instead of the human-readable summary.
    #[arg(long)]
    pub json: bool,
}

/// `reference-stars`: the compact reference list of `sky`, made from a copy
/// of the Hipparcos main catalogue (`hip_main.dat`, CDS I/239) or of Tycho-2
/// (`tyc2.dat`, CDS I/259) the person downloaded, or, with `--download-tycho2
/// --accept-download`, from Tycho-2 fetched from VizieR (CDS I/259).
#[derive(Args, Debug)]
pub struct ReferenceStarsArgs {
    /// The hip_main.dat file of the Hipparcos catalogue.
    #[arg(
        long,
        conflicts_with_all = ["tycho2", "download_tycho2"],
        required_unless_present_any = ["tycho2", "download_tycho2"]
    )]
    pub hipparcos: Option<PathBuf>,
    /// The tyc2.dat file of the Tycho-2 catalogue.
    #[arg(long, conflicts_with = "download_tycho2")]
    pub tycho2: Option<PathBuf>,
    /// Download Tycho-2 from VizieR (CDS, I/259): about 25 MB once the list is made.
    #[arg(long)]
    pub download_tycho2: bool,
    /// Your agreement to the download of Tycho-2 (without it, nothing is downloaded).
    #[arg(long, requires = "download_tycho2")]
    pub accept_download: bool,
    /// Only keep the stars brighter than this V magnitude.
    #[arg(long, default_value_t = 12.5)]
    pub faintest: f64,
    /// File to write.
    #[arg(long)]
    pub out: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_command_is_well_formed() {
        Cli::command().debug_assert();
    }

    #[test]
    fn reference_stars_offers_the_tycho_2_download_only_with_its_agreement_flag() {
        let cli = Cli::parse_from([
            "astro-sync",
            "reference-stars",
            "--download-tycho2",
            "--accept-download",
            "--out",
            "/tmp/reference-stars.bin",
        ]);
        match cli.command {
            Command::ReferenceStars(args) => {
                assert!(args.download_tycho2 && args.accept_download);
                assert_eq!(args.faintest, 12.5);
            }
            other => panic!("unexpected command: {other:?}"),
        }
        // The agreement flag alone, or the download beside a file, is refused.
        for arguments in [
            vec!["--accept-download", "--out", "/x"],
            vec!["--download-tycho2", "--hipparcos", "/h", "--out", "/x"],
            vec!["--download-tycho2", "--tycho2", "/t", "--out", "/x"],
            vec!["--out", "/x"],
        ] {
            let mut full = vec!["astro-sync", "reference-stars"];
            full.extend(arguments);
            assert!(Cli::try_parse_from(full).is_err());
        }
        // The files still work as before.
        assert!(
            Cli::try_parse_from([
                "astro-sync",
                "reference-stars",
                "--tycho2",
                "/t",
                "--out",
                "/x"
            ])
            .is_ok()
        );
    }

    #[test]
    fn sky_parses_a_measure_and_an_upload() {
        let cli = Cli::parse_from([
            "astro-sync",
            "sky",
            "--folder",
            "/tmp/night",
            "--stars",
            "/tmp/stars.bin",
            "--site-latitude",
            "45.0",
            "--site-longitude",
            "-30.0",
            "--portal",
            "https://astro.example",
            "--token",
            "/tmp/token",
            "--session-id",
            "session-1",
        ]);
        match cli.command {
            Command::Sky(args) => {
                assert_eq!(args.site_longitude, Some(-30.0));
                assert_eq!(args.portal.as_deref(), Some("https://astro.example"));
                assert_eq!(args.session_id.as_deref(), Some("session-1"));
                assert!(!args.json);
            }
            other => panic!("unexpected command: {other:?}"),
        }
        // A portal without a token, or a latitude without a longitude, is refused.
        assert!(
            Cli::try_parse_from([
                "astro-sync",
                "sky",
                "--folder",
                "/x",
                "--portal",
                "https://a.b"
            ])
            .is_err()
        );
        assert!(
            Cli::try_parse_from([
                "astro-sync",
                "sky",
                "--folder",
                "/x",
                "--site-latitude",
                "4"
            ])
            .is_err()
        );
        assert!(Cli::try_parse_from(["astro-sync", "reference-stars", "--out", "/x"]).is_err());
    }

    #[test]
    fn associate_parses_an_existing_folder() {
        let cli = Cli::parse_from([
            "astro-sync",
            "associate",
            "--session-id",
            "session-1",
            "--config",
            "/tmp/astro-sync.json",
            "--folder",
            "/tmp/night",
        ]);
        match cli.command {
            Command::Associate(args) => {
                assert_eq!(args.session_id, "session-1");
                assert_eq!(args.folder, Some(PathBuf::from("/tmp/night")));
                assert_eq!(args.new_folder, None);
            }
            other => panic!("expected Associate, got {other:?}"),
        }
    }

    #[test]
    fn dry_run_subcommand_is_kebab_case() {
        let cli = Cli::parse_from(["astro-sync", "dry-run", "--folder", "/tmp/night"]);
        assert!(matches!(cli.command, Command::DryRun(_)));
    }

    #[test]
    fn package_defaults_the_producer_name() {
        let cli = Cli::parse_from([
            "astro-sync",
            "package",
            "--folder",
            "/tmp/night",
            "--output",
            "/tmp/out",
        ]);
        match cli.command {
            Command::Package(args) => assert_eq!(args.producer_name, "astro-sync-cli"),
            other => panic!("expected Package, got {other:?}"),
        }
    }

    #[test]
    fn upload_parses_repeated_target_ids_and_the_token_file_path() {
        let cli = Cli::parse_from([
            "astro-sync",
            "upload",
            "--queue",
            "/tmp/queue.sqlite",
            "--portal",
            "https://portal.example",
            "--token",
            "/tmp/token.txt",
            "--target-id",
            "m31",
            "--target-id",
            "m42",
        ]);
        match cli.command {
            Command::Upload(args) => {
                assert_eq!(args.queue, PathBuf::from("/tmp/queue.sqlite"));
                assert_eq!(args.portal, "https://portal.example");
                assert_eq!(args.token, PathBuf::from("/tmp/token.txt"));
                assert_eq!(args.target_ids, vec!["m31".to_string(), "m42".to_string()]);
                assert_eq!(args.folder, None);
            }
            other => panic!("expected Upload, got {other:?}"),
        }
    }

    #[test]
    fn sign_in_defaults_to_the_hosted_service_and_never_takes_the_code_as_an_argument() {
        let cli = Cli::parse_from([
            "astro-sync",
            "sign-in",
            "--token",
            "/tmp/token.txt",
            "--email",
            "vous@exemple.fr",
        ]);
        match cli.command {
            Command::SignIn(args) => {
                assert_eq!(args.portal, HOSTED_SERVICE_ORIGIN);
                assert_eq!(args.token, PathBuf::from("/tmp/token.txt"));
                assert_eq!(args.email.as_deref(), Some("vous@exemple.fr"));
                assert!(!args.send_only && !args.verify_only && !args.json);
            }
            other => panic!("expected SignIn, got {other:?}"),
        }
        // There is no `--code`: it would stay in the shell history.
        let attempt = Cli::try_parse_from([
            "astro-sync",
            "sign-in",
            "--token",
            "/tmp/token.txt",
            "--code",
            "481207",
        ]);
        assert!(attempt.is_err(), "a code cannot be given as an argument");
        // The two halves of the headless use exclude each other.
        let both = Cli::try_parse_from([
            "astro-sync",
            "sign-in",
            "--token",
            "/tmp/token.txt",
            "--send-only",
            "--verify-only",
        ]);
        assert!(both.is_err());
    }

    #[test]
    fn analyze_defaults_the_utc_offsets_to_zero() {
        let cli = Cli::parse_from([
            "astro-sync",
            "analyze",
            "--folder",
            "/tmp/night",
            "--out",
            "/tmp/out",
        ]);
        match cli.command {
            Command::Analyze(args) => {
                assert_eq!(args.folder, PathBuf::from("/tmp/night"));
                assert_eq!(args.out, PathBuf::from("/tmp/out"));
                assert_eq!(args.phd2_utc_offset, "+00:00");
                assert_eq!(args.asiair_utc_offset, "+00:00");
                assert_eq!(args.nina_utc_offset, "+00:00");
                assert!(!args.json);
            }
            other => panic!("expected Analyze, got {other:?}"),
        }
    }

    #[test]
    fn report_defaults_the_utc_offsets_to_zero() {
        let cli = Cli::parse_from([
            "astro-sync",
            "report",
            "--folder",
            "/tmp/night",
            "--out",
            "/tmp/report.json",
        ]);
        match cli.command {
            Command::Report(args) => {
                assert_eq!(args.folder, PathBuf::from("/tmp/night"));
                assert_eq!(args.out, PathBuf::from("/tmp/report.json"));
                // None given: the clock of the logs is found from the frames.
                assert_eq!(args.phd2_utc_offset, None);
                assert_eq!(args.asiair_utc_offset, None);
                assert_eq!(args.nina_utc_offset, None);
                assert!(!args.json);
            }
            other => panic!("expected Report, got {other:?}"),
        }
    }

    #[test]
    fn queue_status_is_a_nested_subcommand() {
        let cli = Cli::parse_from([
            "astro-sync",
            "queue",
            "status",
            "--queue",
            "/tmp/queue.sqlite",
            "--json",
        ]);
        match cli.command {
            Command::Queue(QueueArgs {
                command: QueueCommand::Status(args),
            }) => {
                assert_eq!(args.queue, PathBuf::from("/tmp/queue.sqlite"));
                assert!(args.json);
            }
            other => panic!("expected Queue(Status), got {other:?}"),
        }
    }

    #[test]
    fn siril_detect_is_a_nested_subcommand_with_an_optional_manual_path() {
        let cli = Cli::parse_from([
            "astro-sync",
            "siril",
            "detect",
            "--siril-path",
            "/opt/siril/bin/siril-cli",
            "--json",
        ]);
        match cli.command {
            Command::Siril(SirilArgs {
                command: SirilCommand::Detect(args),
            }) => {
                assert_eq!(
                    args.siril_path,
                    Some(PathBuf::from("/opt/siril/bin/siril-cli"))
                );
                assert!(args.json);
            }
            other => panic!("expected Siril(Detect), got {other:?}"),
        }
    }

    #[test]
    fn progression_requires_folder_family_and_out() {
        let cli = Cli::parse_from([
            "astro-sync",
            "progression",
            "--folder",
            "/tmp/night",
            "--family",
            "Ha_300s_g100_1x1_t-10C",
            "--out",
            "/tmp/out",
        ]);
        match cli.command {
            Command::Progression(args) => {
                assert_eq!(args.folder, PathBuf::from("/tmp/night"));
                assert_eq!(args.family, "Ha_300s_g100_1x1_t-10C");
                assert_eq!(args.out, PathBuf::from("/tmp/out"));
                assert_eq!(args.siril_path, None);
                assert!(!args.json);
            }
            other => panic!("expected Progression, got {other:?}"),
        }
    }
}
