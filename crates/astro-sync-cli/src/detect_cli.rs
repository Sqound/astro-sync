//! "Detect sessions" on the command line: `detect-sessions`
//! reads the three locations and prints the proposal (`--json`: the core's
//! value as is, the one the window shows), `create-sessions` writes the
//! proposal on the service ("Create ...": `POST /v1/sessions/bulk`,
//! idempotent), `organise` describes then runs the tidying of the files.
//! Each is one call into `astro_sync_core::session_detection`, the calls the
//! window makes ("CLI / GUI parity"). Only `create-sessions` writes on the
//! service.

use std::error::Error;
use std::path::{Path, PathBuf};

use clap::{Args, ValueEnum};
use serde::de::DeserializeOwned;
use serde_json::Value;

use astro_sync_core::catalogue_targets::{self, CatalogueIndex, CatalogueTarget};
use astro_sync_core::gui_bridge::PortalGuiTransport;
use astro_sync_core::multi_night;
use astro_sync_core::now_iso8601_utc;
use astro_sync_core::service_wait::WakePolicy;
use astro_sync_core::session_detection::{
    self, AccountRights, BulkCreateOutcome, BulkCreateRequest, DetectOptions, DetectRequest,
    KnownEquipment, KnownPlace, KnownPlan, KnownSession, NightNaming, OrganiseMode,
    OrganiseOptions, Proposal, RightsRefresh, ScopeTarget, SessionScope, UserData, service,
};
use astro_sync_core::sky_solution::{self, SolveFailure, SolveHints, SolveJob, SolverChoice};
use astro_sync_core::tools::{self, DetectContext};

use crate::night::token_of;
use crate::output::print_json;

type Outcome = Result<(), Box<dyn Error>>;

#[derive(Args, Debug)]
pub struct DetectSessionsArgs {
    /// Folder of the FITS files (lights), required, repeatable.
    #[arg(long, required = true)]
    pub fits: Vec<PathBuf>,
    /// Folder of the calibrations (darks, flats, dark-flats, bias), repeatable.
    #[arg(long)]
    pub calibration: Vec<PathBuf>,
    /// Folder of the logs (N.I.N.A., ASIAIR, PHD2), repeatable.
    #[arg(long)]
    pub logs: Vec<PathBuf>,
    /// Do not read the sub-folders.
    #[arg(long)]
    pub no_subfolders: bool,
    /// "My places": JSON file (a list, or `{ "places": […], "defaultPlaceId" }`).
    #[arg(long)]
    pub places: Option<PathBuf>,
    /// "My equipment": JSON file (a list, or `{ "equipment": […] }`).
    #[arg(long)]
    pub equipment: Option<PathBuf>,
    /// Existing sessions and plans: JSON file `{ "sessions": […], "plans": […] }`.
    #[arg(long)]
    pub existing: Option<PathBuf>,
    /// Catalogue of the targets: JSON file (a list of targets, or the kept index).
    #[arg(long)]
    pub catalogue: Option<PathBuf>,
    /// Astro Sync data (the kept catalogue index, the installed solvers).
    #[arg(long)]
    pub data: Option<PathBuf>,
    /// Base URL of the service: places, equipment, sessions, plans and catalogue
    /// are read from it (read-only) when the files do not give them.
    #[arg(long, requires = "token")]
    pub portal: Option<String>,
    /// File containing the device token.
    #[arg(long, requires = "portal")]
    pub token: Option<PathBuf>,
    /// The account has Advanced Planning (multi-night plans).
    #[arg(long)]
    pub advanced_planning: bool,
    /// The account may have several plans per night (subscription).
    #[arg(long)]
    pub several_plans_per_night: bool,
    /// Start of the declared session (ISO 8601 UTC): detection bounded to it.
    #[arg(long, requires = "session_to")]
    pub session_from: Option<String>,
    /// End of the declared session (ISO 8601 UTC).
    #[arg(long, requires = "session_from")]
    pub session_to: Option<String>,
    /// Target of the declared session: IDENTIFIER=DESIGNATION (repeatable).
    #[arg(long = "session-target", value_name = "ID=DESIGNATION")]
    pub session_targets: Vec<String>,
    /// Filter of every light of the run (e.g. "Dualband", "UV/IR cut"): stands over the
    /// FILTER cards and over what the folder names say. Without it, a light whose header
    /// names no filter takes the one its folder names (dualband, UVIR, RGB…).
    #[arg(long, value_name = "NAME")]
    pub filter: Option<String>,
    /// Solve one frame per group without a target with the installed solver (ASTAP).
    #[arg(long, requires = "data")]
    pub solve: bool,
    /// Write the proposal to this file (for `organise`).
    #[arg(long)]
    pub output: Option<PathBuf>,
    #[arg(long, default_value = "fr")]
    pub locale: String,
    #[arg(long)]
    pub json: bool,
    #[command(flatten)]
    pub language_of_messages: crate::cli::LangArg,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ModeArg {
    /// Leave in place (default).
    Leave,
    Copy,
    Move,
    Link,
}

#[derive(Args, Debug)]
pub struct CreateSessionsArgs {
    /// The proposal written by `detect-sessions --output`.
    #[arg(long)]
    pub proposal: PathBuf,
    /// The choices of the review (JSON `{ "ticked", "places", "answers", "shifts" }`,
    /// the window's request without `proposal`); absent: what is ticked
    /// by default, the nearest place, the proposed shift.
    #[arg(long)]
    pub choices: Option<PathBuf>,
    /// Base URL of the service.
    #[arg(long)]
    pub portal: String,
    /// File containing the device token.
    #[arg(long)]
    pub token: PathBuf,
    /// Write the result to this file (for `organise --created`).
    #[arg(long)]
    pub output: Option<PathBuf>,
    /// Name of the places this call creates (without it: a word of the language, never the
    /// position of the frames). A place the review named keeps its name.
    #[arg(long, value_name = "NAME")]
    pub place_name: Option<String>,
    /// Show the body sent, without sending anything.
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub json: bool,
    #[command(flatten)]
    pub language_of_messages: crate::cli::LangArg,
}

#[derive(Args, Debug)]
pub struct TreatPrepareArgs {
    /// The proposal written by `detect-sessions --output`.
    #[arg(long)]
    pub proposal: PathBuf,
    /// The result of `import-provisional --output` (the nights to create
    /// take a provisional folder), or of `create-sessions --output`.
    #[arg(long)]
    pub created: PathBuf,
    /// The working folder.
    #[arg(long)]
    pub root: PathBuf,
    /// URL of the service (written to the identity of each night).
    #[arg(long)]
    pub portal: String,
    /// Identifier of a target to process (repeatable; none: all).
    #[arg(long = "target")]
    pub targets: Vec<String>,
}

#[derive(Args, Debug)]
pub struct ImportProvisionalArgs {
    /// The proposal written by `detect-sessions --output`.
    #[arg(long)]
    pub proposal: PathBuf,
    /// The choices of the review (as `create-sessions --choices`).
    #[arg(long)]
    pub choices: Option<PathBuf>,
    /// Write the result to this file (for `treat-prepare --created`).
    #[arg(long)]
    pub output: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct ImportSettleArgs {
    /// The working folder.
    #[arg(long)]
    pub root: PathBuf,
    /// The result of `import-provisional --output`.
    #[arg(long)]
    pub provisional: PathBuf,
    /// The result of `create-sessions --output`.
    #[arg(long)]
    pub created: PathBuf,
    /// The upload queue, whose lines follow the renamed folders.
    #[arg(long)]
    pub queue: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct ImportDiscardArgs {
    /// The working folder.
    #[arg(long)]
    pub root: PathBuf,
    /// The result of `import-provisional --output`.
    #[arg(long)]
    pub provisional: PathBuf,
}

#[derive(Args, Debug)]
pub struct OrganiseArgs {
    /// The proposal written by `detect-sessions --output`.
    #[arg(long)]
    pub proposal: PathBuf,
    /// Folder where to tidy (night / target / lights / [filter]).
    #[arg(long)]
    pub destination: PathBuf,
    #[arg(long, value_enum, default_value = "leave")]
    pub mode: ModeArg,
    /// Session to tidy (repeatable; none: all).
    #[arg(long = "session")]
    pub sessions: Vec<String>,
    /// The result of `create-sessions --output`: each tidied night takes
    /// the name `YYYY-MM-DD · <place> · <short id>` of its created session.
    #[arg(long)]
    pub created: Option<PathBuf>,
    /// Describe what would be done, without doing anything.
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub json: bool,
}

fn fail(message: impl Into<String>) -> Box<dyn Error> {
    message.into().into()
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, Box<dyn Error>> {
    let bytes =
        std::fs::read(path).map_err(|e| fail(format!("could not read {}: {e}", path.display())))?;
    serde_json::from_slice(&bytes)
        .map_err(|e| fail(format!("unreadable JSON in {}: {e}", path.display())))
}

/// A list given as is or under `key`.
fn list_of<T: DeserializeOwned>(value: &Value, key: &str) -> Result<Vec<T>, Box<dyn Error>> {
    let list = value.get(key).unwrap_or(value).clone();
    if list.is_null() {
        return Ok(Vec::new());
    }
    Ok(serde_json::from_value(list)?)
}

fn catalogue_of(args: &DetectSessionsArgs) -> Result<Vec<CatalogueTarget>, Box<dyn Error>> {
    if let Some(path) = &args.catalogue {
        let value: Value = read_json(path)?;
        if let Ok(index) = serde_json::from_value::<CatalogueIndex>(value.clone()) {
            return Ok(index.targets);
        }
        return list_of(&value, "targets");
    }
    let index_path = args
        .data
        .as_ref()
        .map(|d| d.join(catalogue_targets::INDEX_FILE));
    if let (Some(portal), Some(token)) = (&args.portal, &args.token) {
        let token = token_of(token)?;
        let kept = index_path
            .as_deref()
            .and_then(catalogue_targets::load_index);
        let (index, fresh) = catalogue_targets::refresh_index(
            &crate::words::portal_transport()?,
            portal,
            Some(&token),
            &args.locale,
            kept,
        )?;
        if fresh && let Some(path) = &index_path {
            let _ = catalogue_targets::save_index(path, &index);
        }
        return Ok(index.targets);
    }
    Ok(index_path
        .as_deref()
        .and_then(catalogue_targets::load_index)
        .map(|i| i.targets)
        .unwrap_or_default())
}

fn scope_of(args: &DetectSessionsArgs) -> Result<Option<SessionScope>, Box<dyn Error>> {
    let (Some(from), Some(to)) = (&args.session_from, &args.session_to) else {
        return Ok(None);
    };
    let targets = args
        .session_targets
        .iter()
        .map(|spec| {
            let (id, designation) = spec
                .split_once('=')
                .ok_or_else(|| fail(format!("--session-target expects ID=DESIGNATION: {spec}")))?;
            Ok(ScopeTarget {
                id: id.to_string(),
                designation: designation.to_string(),
                ..ScopeTarget::default()
            })
        })
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    Ok(Some(SessionScope {
        from: from.clone(),
        to: to.clone(),
        targets,
    }))
}

/// The rights the account holds on the service now (capabilities, and whether
/// the service sells the multi-night plans), read from it.
fn account_of_service(
    transport: &astro_sync_core::gui_bridge::HttpPortalGuiTransport,
    portal: &str,
    token: &str,
) -> AccountRights {
    let capabilities = transport.fetch_capabilities(portal, token).ok();
    let sold =
        multi_night::service_sells_multi_night(portal, WakePolicy::default()).unwrap_or(true);
    service::account_of(capabilities.as_ref(), sold)
}

/// The person's data: the files given, else the service (read only).
fn user_data(
    args: &DetectSessionsArgs,
    range: Option<(String, String)>,
) -> Result<UserData, Box<dyn Error>> {
    let mut account = AccountRights {
        advanced_planning: args.advanced_planning,
        several_plans_per_night: args.several_plans_per_night,
    };
    let mut user = UserData::default();
    if let (Some(portal), Some(token)) = (&args.portal, &args.token) {
        let token = token_of(token)?;
        let transport = crate::words::portal_transport()?;
        if !args.advanced_planning && !args.several_plans_per_night {
            account = account_of_service(&transport, portal, &token);
        }
        user = service::read_user_data(&transport, portal, &token, account, range)?;
    }
    user.account = account;
    if let Some(path) = &args.places {
        let value: Value = read_json(path)?;
        user.places = list_of::<KnownPlace>(&value, "places")?;
        if let Some(default) = value.get("defaultPlaceId").and_then(Value::as_str) {
            user.default_place_id = Some(default.to_string());
        }
    }
    if let Some(path) = &args.equipment {
        user.equipment = list_of::<KnownEquipment>(&read_json(path)?, "equipment")?;
    }
    if let Some(path) = &args.existing {
        let value: Value = read_json(path)?;
        user.sessions = list_of::<KnownSession>(&value, "sessions")?;
        user.plans = list_of::<KnownPlan>(&value, "plans")?;
    }
    Ok(user)
}

/// Right ascension as "22 h 05 min 09 s", declination as "+46° 28′ 44″".
fn sky_position_text(ra_degrees: f64, dec_degrees: f64) -> String {
    let total_seconds = (ra_degrees.rem_euclid(360.0) / 15.0 * 3600.0).round() as i64 % 86_400;
    let (hours, minutes, seconds) = (
        total_seconds / 3600,
        total_seconds % 3600 / 60,
        total_seconds % 60,
    );
    let arc_seconds = (dec_degrees.abs() * 3600.0).round() as i64;
    let sign = if dec_degrees < 0.0 { '-' } else { '+' };
    format!(
        "{hours:02} h {minutes:02} min {seconds:02} s {sign}{}° {:02}′ {:02}″",
        arc_seconds / 3600,
        arc_seconds % 3600 / 60,
        arc_seconds % 60
    )
}

/// What the solver said of the groups without a target: why it gave
/// nothing, or where it put the field when no known target is there.
fn print_solve_notes(proposal: &Proposal) {
    for target in proposal.sessions.iter().flat_map(|s| &s.targets) {
        let frame = target.solve_frame.as_deref().unwrap_or_default();
        if let Some(failure) = &target.solve_failure {
            let detail = match failure {
                SolveFailure::Failed { detail } if !detail.is_empty() => format!(" ({detail})"),
                _ => String::new(),
            };
            println!("ⓘ Plate solving of {frame}: {}{detail}", failure.sentence());
        } else if let Some((ra, dec)) = target.solved_position {
            println!(
                "ⓘ Plate solving of {frame}: field centred on {}, outside the known targets.",
                sky_position_text(ra, dec)
            );
        }
    }
}

/// What each night's poses lack in calibration: the text alone for a
/// warning, ⓘ for an information, the evening in front.
fn print_calibration_notes(proposal: &Proposal) {
    for session in &proposal.sessions {
        for note in &session.calibration_notes {
            println!(
                "{}{} (night of {})",
                if note.is_warning() { "" } else { "ⓘ " },
                note.sentence().trim_end_matches('.'),
                session.evening_date
            );
        }
    }
}

fn print_proposal(proposal: &Proposal) {
    let reading = &proposal.reading;
    println!(
        "Read: {} files · {} lights · {} calibrations · {} logs · {} nights · {} targets (read only)",
        reading.files_read,
        reading.lights,
        reading.calibration_frames,
        reading.log_files,
        reading.nights,
        reading.targets
    );
    if !proposal.multi_night.is_empty() {
        let locked = proposal.multi_night.iter().filter(|g| g.locked).count();
        println!(
            "{} multi-night plan(s) detected{}",
            proposal.multi_night.len(),
            if locked > 0 {
                format!(" ({locked} locked: Advanced Planning)")
            } else {
                " (ticked by default)".to_string()
            }
        );
    }
    for note in &proposal.notes {
        println!("ⓘ {}", note.text);
    }
    // The files that are not read: ⓘ for an
    // information, the text alone for a warning.
    for note in proposal.unread_note_sentences() {
        println!("{}{}", if note.warning { "" } else { "ⓘ " }, note.text);
    }
    for line in &proposal.lines {
        let mark = match (line.locked, line.ask, line.checked) {
            (true, _, _) => "[locked]",
            (_, true, _) => "[question]",
            (_, _, true) => "[x]",
            _ => "[ ]",
        };
        let indent = match line.kind {
            session_detection::LineKind::Target
            | session_detection::LineKind::Calibration
            | session_detection::LineKind::Log
            | session_detection::LineKind::Clock => "    ",
            _ => "",
        };
        println!("{indent}{mark} {}", line.text);
    }
    print_calibration_notes(proposal);
    print_solve_notes(proposal);
    let aside = &proposal.set_aside;
    for (label, files) in [
        ("without DATE-OBS", &aside.without_date),
        ("illisibles", &aside.unreadable),
        ("stackings ignored", &aside.stacks),
        ("calibrations not attached", &aside.calibration_unattached),
        ("logs not attached", &aside.logs_unattached),
        ("other files", &aside.other_files),
    ] {
        if !files.is_empty() {
            println!("Set aside, {label}: {}", files.len());
        }
    }
    if let Some(scoped) = &proposal.scoped {
        for target in &scoped.targets {
            println!(
                "Session · {}: {} FITS",
                target.designation,
                target.files.len()
            );
        }
        for group in &scoped.unassigned {
            println!(
                "Unassigned · {}: {} frames",
                group.designation,
                group.files.len()
            );
        }
        println!(
            "Without OBJECT or coordinates: {} · frames of other nights: {}",
            scoped.without_object.len(),
            scoped.other_nights
        );
    }
}

pub fn detect_sessions(args: &DetectSessionsArgs) -> Outcome {
    let request = DetectRequest {
        fits: args.fits.clone(),
        calibration: args.calibration.clone(),
        logs: args.logs.clone(),
        subfolders: !args.no_subfolders,
        scope: scope_of(args)?,
        filter: args.filter.clone(),
    };
    let catalogue = catalogue_of(args)?;
    let solvers = args.data.as_ref().filter(|_| args.solve).map(|data| {
        tools::solvers(&tools::detect_tools(&DetectContext::system(
            data,
            &data.join("reference-stars.bin"),
        )))
    });
    let work = std::env::temp_dir().join("astro-sync-detect-solve");
    let now = now_iso8601_utc();
    let solve = |image: &Path| -> Result<(f64, f64), SolveFailure> {
        let job = SolveJob {
            image,
            hints: SolveHints::default(),
            work_dir: &work,
            timeout: sky_solution::DEFAULT_SOLVE_TIMEOUT,
            cancel: None,
            solved_at: &now,
        };
        let solvers = solvers.as_ref().ok_or(SolveFailure::SolverMissing)?;
        let solution = sky_solution::solve_image(solvers, SolverChoice::Astap, &job)?;
        Ok((solution.ra_deg, solution.dec_deg))
    };
    let options = DetectOptions {
        solver: solvers.is_some().then_some(&solve as _),
        ..DetectOptions::default()
    };
    // The person's sessions are read for the dates found only.
    let first_pass = UserData {
        account: AccountRights::default(),
        ..user_data_without_sessions(args)?
    };
    let reading = session_detection::read_locations(&request, &options)?;
    let grouping =
        session_detection::group_frames(&reading, &request, &catalogue, &first_pass, &options)?;
    let user = user_data(args, grouping.date_range())?;
    let grouping = if user.places != first_pass.places
        || user.default_place_id != first_pass.default_place_id
    {
        session_detection::group_frames(&reading, &request, &catalogue, &user, &options)?
    } else {
        grouping
    };
    let proposal = session_detection::propose(&grouping, &user);
    if let Some(path) = &args.output {
        std::fs::write(path, serde_json::to_vec_pretty(&proposal)?)?;
    }
    if args.json {
        return print_json(&proposal);
    }
    print_proposal(&proposal);
    Ok(())
}

/// The places given as a file, for the first grouping (the night's noon may
/// be the place's).
fn user_data_without_sessions(args: &DetectSessionsArgs) -> Result<UserData, Box<dyn Error>> {
    let mut user = UserData::default();
    if let Some(path) = &args.places {
        let value: Value = read_json(path)?;
        user.places = list_of::<KnownPlace>(&value, "places")?;
        if let Some(default) = value.get("defaultPlaceId").and_then(Value::as_str) {
            user.default_place_id = Some(default.to_string());
        }
    }
    Ok(user)
}

/// The person's choices of the review, as the window sends them.
#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Choices {
    ticked: Option<Vec<String>>,
    #[serde(default)]
    places: Vec<session_detection::PlaceAnswer>,
    #[serde(default)]
    answers: Vec<session_detection::LineAnswer>,
    shifts: Option<Vec<session_detection::SessionShift>>,
}

pub fn create_sessions(args: &CreateSessionsArgs) -> Outcome {
    // The offer may have changed since the detection (a code redeemed, a plan
    // taken): the proposal follows the rights the account holds now, so a
    // multi-night plan the detection showed locked is created, ticked by
    // default, once the offer allows it.
    let account = account_of_service(
        &crate::words::portal_transport()?,
        &args.portal,
        &token_of(&args.token)?,
    );
    let request = request_of(
        &args.proposal,
        args.choices.as_deref(),
        args.place_name.as_deref(),
        Some(account),
    )?;
    if args.dry_run {
        let built = session_detection::bulk_body(&request);
        return print_json(&serde_json::json!({ "body": built.body, "skipped": built.skipped }));
    }
    let token = token_of(&args.token)?;
    let outcome = session_detection::create_sessions(
        &crate::words::portal_transport()?,
        &args.portal,
        &token,
        &request,
    )?;
    if let Some(path) = &args.output {
        std::fs::write(path, serde_json::to_vec_pretty(&outcome)?)?;
    }
    if args.json {
        return print_json(&outcome);
    }
    for made in &outcome.sessions {
        println!(
            "{} {} · {} · plan {} · {} frames{}",
            if made.action == "created" {
                "Imported"
            } else {
                "Updated"
            },
            made.evening_date,
            made.designations.join(", "),
            made.plan_action,
            made.frames,
            if made.shift_hours == 0 {
                String::new()
            } else {
                format!(" · times shifted by {} h", made.shift_hours)
            }
        );
    }
    for made in &outcome.multi_night {
        println!(
            "Multi-night plan {} · {} nights · created as “Completed”",
            made.designation,
            made.session_ids.len()
        );
    }
    if !outcome.places_created.is_empty() {
        println!("Place(s) created: {}", outcome.places_created.len());
    }
    let language = crate::words::requested_language();
    for skipped in &outcome.skipped {
        // The reason in the application's words (`detect.skip.<reason>`).
        let reason = crate::words::say(language, &format!("detect.skip.{}", skipped.reason), &[])
            .unwrap_or_else(|| skipped.reason.clone());
        println!("Not created: {} {} ({reason})", skipped.kind, skipped.id);
    }
    if outcome.replayed {
        println!("Already sent: nothing new was written.");
    }
    Ok(())
}

/// The request of the review: the proposal and the choices given (as
/// `create-sessions` reads them).
fn request_of(
    proposal: &Path,
    choices: Option<&Path>,
    place_name: Option<&str>,
    account: Option<AccountRights>,
) -> Result<BulkCreateRequest, Box<dyn Error>> {
    let mut proposal: Proposal = read_json(proposal)?;
    let mut unlocked = Vec::new();
    if let Some(account) = account {
        match proposal.refresh_rights(account) {
            RightsRefresh::Unchanged => {}
            RightsRefresh::Updated { unlocked: opened } => {
                if !opened.is_empty() {
                    eprintln!(
                        "ⓘ The offer changed since the detection: {} multi-night plan(s) now open, ticked by default.",
                        opened.len()
                    );
                }
                unlocked = opened;
            }
            RightsRefresh::Stale => {
                return Err(fail(
                    "the account's offer changed since the detection and the proposal cannot follow it: run detect-sessions again",
                ));
            }
        }
    }
    // The name of a place to create: the person's, else the word of the
    // language asked for, never its position.
    let word = crate::words::say(
        crate::words::requested_language(),
        "detect.place.defaultName",
        &[],
    );
    let place_name = place_name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .or(word)
        .unwrap_or_else(|| session_detection::create::DEFAULT_PLACE_NAME.to_string());
    let mut request = BulkCreateRequest::as_proposed_named(proposal, &place_name);
    if let Some(path) = choices {
        let choices: Choices = read_json(path)?;
        if let Some(mut ticked) = choices.ticked {
            // A plan the person could not tick (it was locked) is theirs to
            // untick now: it joins what is ticked by default.
            for id in &unlocked {
                if !ticked.contains(id) {
                    ticked.push(id.clone());
                }
            }
            request.ticked = ticked;
        }
        if let Some(shifts) = choices.shifts {
            request.shifts = shifts;
        }
        request.places = choices.places;
        request.answers = choices.answers;
    }
    Ok(request)
}

pub fn import_provisional(args: &ImportProvisionalArgs) -> Outcome {
    let request = request_of(&args.proposal, args.choices.as_deref(), None, None)?;
    let outcome = session_detection::provisional::provisional_outcome(&request);
    if let Some(path) = &args.output {
        std::fs::write(path, serde_json::to_vec_pretty(&outcome)?)?;
    }
    print_json(&outcome)
}

pub fn import_settle(args: &ImportSettleArgs) -> Outcome {
    use session_detection::provisional;
    let local: BulkCreateOutcome = read_json(&args.provisional)?;
    let created: BulkCreateOutcome = read_json(&args.created)?;
    let queue = match &args.queue {
        Some(path) => Some(astro_sync_core::upload_queue::UploadQueue::open(path)?),
        None => None,
    };
    let recorded = provisional::record_created(&args.root, &local, &created)?;
    let nights = provisional::settle_nights(&args.root, queue.as_ref())?;
    let plans = provisional::settle_plans(&args.root, &nights, queue.as_ref())?;
    print_json(&serde_json::json!({
        "recorded": recorded,
        "nights": nights,
        "plans": plans,
    }))
}

pub fn import_discard(args: &ImportDiscardArgs) -> Outcome {
    let local: BulkCreateOutcome = read_json(&args.provisional)?;
    let ids: Vec<String> = local
        .sessions
        .iter()
        .map(|s| s.session_id.clone())
        .chain(local.multi_night.iter().map(|m| m.plan_id.clone()))
        .collect();
    print_json(&session_detection::provisional::discard_provisional(
        &args.root, &ids,
    )?)
}

pub fn treat_prepare(args: &TreatPrepareArgs) -> Outcome {
    let proposal: Proposal = read_json(&args.proposal)?;
    let created: BulkCreateOutcome = read_json(&args.created)?;
    let nights = session_detection::prepare_treatments(
        &proposal,
        &created,
        &args.targets,
        &args.root,
        &[],
        &args.portal,
        &now_iso8601_utc(),
    )?;
    print_json(&nights)
}

pub fn organise(args: &OrganiseArgs) -> Outcome {
    let proposal: Proposal = read_json(&args.proposal)?;
    let mode = match args.mode {
        ModeArg::Leave => OrganiseMode::LeaveInPlace,
        ModeArg::Copy => OrganiseMode::Copy,
        ModeArg::Move => OrganiseMode::Move,
        ModeArg::Link => OrganiseMode::Symlink,
    };
    let names: std::collections::BTreeMap<String, NightNaming> = match &args.created {
        Some(path) => read_json::<BulkCreateOutcome>(path)?
            .sessions
            .into_iter()
            .map(|made| {
                (
                    made.detected_id,
                    NightNaming {
                        place: None,
                        session_id: Some(made.session_id),
                        folder: None,
                    },
                )
            })
            .collect(),
        None => std::collections::BTreeMap::new(),
    };
    let plan = session_detection::plan_organise_named(
        &proposal,
        &args.destination,
        mode,
        &args.sessions,
        &names,
    )?;
    if args.dry_run {
        if args.json {
            return print_json(&plan);
        }
        println!("(dry run) {}", plan.summary);
        for action in &plan.actions {
            println!(
                "  {:?} {} → {}",
                action.kind, action.source, action.destination
            );
        }
        for conflict in &plan.conflicts {
            println!("  left: {} ({})", conflict.source, conflict.reason);
        }
        return Ok(());
    }
    let outcome = session_detection::organise(&plan, &OrganiseOptions::default())?;
    if args.json {
        return print_json(&outcome);
    }
    println!("{}", plan.summary);
    println!(
        "Done: {} · verified: {} · sources deleted: {} · already in place: {} · failures: {}{}",
        outcome.done,
        outcome.verified,
        outcome.removed_sources,
        outcome.already_there,
        outcome.failed.len(),
        if outcome.stopped { " · stopped" } else { "" }
    );
    for failure in &outcome.failed {
        println!("  failed: {} ({})", failure.source, failure.reason);
    }
    if outcome.failed.is_empty() {
        Ok(())
    } else {
        Err(fail("the organising did not run to the end"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use astro_sync_core::session_detection::MultiNightGroup;

    /// A proposal computed for a free account: one multi-night plan, locked.
    fn locked_proposal() -> Proposal {
        let granted = AccountRights {
            advanced_planning: true,
            several_plans_per_night: true,
        };
        let mut proposal = Proposal {
            multi_night: vec![MultiNightGroup {
                id: "mn-1".into(),
                designation: "Sh2-103".into(),
                ..MultiNightGroup::default()
            }],
            account: granted,
            ..Proposal::default()
        };
        proposal.refresh_rights(AccountRights::default());
        assert!(proposal.multi_night[0].locked);
        proposal
    }

    fn write(dir: &Path, name: &str, value: &impl serde::Serialize) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
        path
    }

    #[test]
    fn a_plan_shown_locked_is_created_ticked_once_the_offer_allows_it() {
        let dir = std::env::temp_dir().join(format!("astro-cli-refresh-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = write(&dir, "proposal.json", &locked_proposal());
        let granted = AccountRights {
            advanced_planning: true,
            several_plans_per_night: true,
        };

        // The detection ran on the free account: nothing multi-night is ticked.
        let stale = request_of(&file, None, None, None).unwrap();
        assert!(!stale.ticked.contains(&"multiNight.create:mn-1".to_string()));

        // The offer was granted since: the proposal follows it.
        let request = request_of(&file, None, None, Some(granted)).unwrap();
        assert!(
            request
                .ticked
                .contains(&"multiNight.create:mn-1".to_string())
        );
        assert!(!request.proposal.multi_night[0].locked);

        // The choices written for the locked proposal are completed with it.
        let choices = write(
            &dir,
            "choices.json",
            &serde_json::json!({ "ticked": ["session.create:s-1"] }),
        );
        let request = request_of(&file, Some(&choices), None, Some(granted)).unwrap();
        assert!(request.ticked.contains(&"session.create:s-1".to_string()));
        assert!(
            request
                .ticked
                .contains(&"multiNight.create:mn-1".to_string())
        );

        // Still the free account: the plan stays locked and is never ticked.
        let free = request_of(&file, Some(&choices), None, Some(AccountRights::default())).unwrap();
        assert!(!free.ticked.contains(&"multiNight.create:mn-1".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
