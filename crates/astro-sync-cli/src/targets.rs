//! The "Targets" step on the command line: the catalogue searched,
//! a target added to a night, a capture's FITS read for its target, a new
//! target outside the catalogue. Each is one call into
//! `astro_sync_core::catalogue_targets`, the call the window makes; `--json`
//! prints the core's value as-is ("CLI / GUI parity").
//!
//! Taking a target back is the window's only, for a target added in the
//! same opening of the wizard: a target already recorded in a night is
//! managed in the Journal of the website, never from Astro Sync.

use std::error::Error;
use std::path::PathBuf;

use clap::Args;

use astro_sync_core::catalogue_targets::{
    self, CatalogueTarget, FitsGuess, FitsGuessKind, NightTargetChange, PersonalTargetCreate,
};
use astro_sync_core::night_service::NightServiceTransport;

use crate::cli_night::ServiceArgs;
use crate::night::token_of;
use crate::output::print_json;

type Outcome = Result<(), Box<dyn Error>>;

#[derive(Args, Debug)]
pub struct TargetsArgs {
    #[command(flatten)]
    pub service: ServiceArgs,
    /// Text searched in the catalogue ("M31", "ngc7000", "nébuleuse"...).
    #[arg(long)]
    pub search: String,
    /// Language of the names, types and constellations (fr, en, ja...).
    #[arg(long, default_value = "fr")]
    pub locale: String,
    /// Results skipped ("Show more targets").
    #[arg(long, default_value_t = 0)]
    pub offset: u32,
    /// Results requested (8 by default, 24 at most).
    #[arg(long, default_value_t = catalogue_targets::SEARCH_PAGE)]
    pub limit: u32,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct NightTargetsArgs {
    #[command(flatten)]
    pub service: ServiceArgs,
    /// Identifier of the night online.
    #[arg(long)]
    pub session_id: String,
    /// Identifier of the target to add (catalogue or space target).
    #[arg(long)]
    pub add: String,
    /// Say the list that would be written, without sending anything.
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct FitsTargetArgs {
    /// A frame of the capture (FITS or XISF): only its header is read.
    #[arg(long)]
    pub file: PathBuf,
    /// Base URL of the service (without it: the header alone, without the catalogue).
    #[arg(long, requires = "token")]
    pub portal: Option<String>,
    /// File containing the device token.
    #[arg(long, requires = "portal")]
    pub token: Option<PathBuf>,
    /// Astro Sync data (keeps the catalogue index there).
    #[arg(long)]
    pub data: Option<PathBuf>,
    #[arg(long, default_value = "fr")]
    pub locale: String,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct PersonalTargetArgs {
    #[command(flatten)]
    pub service: ServiceArgs,
    /// Name of the target (it also names its folder).
    #[arg(long)]
    pub name: String,
    /// Label, one of the French values the service stores: Galaxie, Nébuleuse, Amas, Lune, Planète, Soleil or "Grand champ".
    #[arg(long)]
    pub label: String,
    /// Right ascension in hours (with --dec-degrees).
    #[arg(long, requires = "dec_degrees", allow_hyphen_values = true)]
    pub ra_hours: Option<f64>,
    /// Declination in degrees (with --ra-hours).
    #[arg(long, requires = "ra_hours", allow_hyphen_values = true)]
    pub dec_degrees: Option<f64>,
    /// Night to which to add it.
    #[arg(long)]
    pub session_id: Option<String>,
    /// Check the input and say what would be sent, without sending anything.
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub json: bool,
}

fn card(target: &CatalogueTarget) -> String {
    let mut line = target.designation.clone();
    if let Some(name) = &target.name {
        line.push_str(&format!(" · {name}"));
    }
    let kind: Vec<&str> = [&target.type_label, &target.constellation_label]
        .into_iter()
        .filter_map(|v| v.as_deref())
        .collect();
    if !kind.is_empty() {
        line.push_str(&format!(" — {}", kind.join(" · ")));
    }
    if let Some(magnitude) = target.magnitude {
        line.push_str(&format!(" — magnitude {magnitude:.1}"));
    }
    if let Some(size) = &target.size_label {
        line.push_str(&format!(" · {size}"));
    }
    format!("{line}  [{}]", target.id)
}

pub fn targets(args: &TargetsArgs) -> Outcome {
    let token = token_of(&args.service.token)?;
    let found = catalogue_targets::search_targets(
        &crate::words::portal_transport()?,
        &args.service.portal,
        Some(&token),
        &args.search,
        &args.locale,
        args.offset,
        args.limit,
    )?;
    if args.json {
        return print_json(&found);
    }
    println!(
        "{} result(s) of {} for “{}”",
        found.targets.len(),
        found.total,
        found.query
    );
    for target in &found.targets {
        println!("  {}", card(target));
    }
    Ok(())
}

pub fn night_targets(args: &NightTargetsArgs) -> Outcome {
    let token = token_of(&args.service.token)?;
    let transport = crate::words::portal_transport()?;
    let change = NightTargetChange::Add(args.add.clone());
    let ids = if args.dry_run {
        let detail = transport.session_detail(&args.service.portal, &token, &args.session_id)?;
        let current = catalogue_targets::night_target_ids(&detail);
        catalogue_targets::changed_target_ids(&current, &change).unwrap_or(current)
    } else {
        catalogue_targets::change_night_targets(
            &transport,
            &args.service.portal,
            &token,
            &args.session_id,
            &change,
        )?
    };
    if args.json {
        return print_json(&ids);
    }
    println!(
        "{}Targets of the night: {}",
        if args.dry_run { "(dry run) " } else { "" },
        ids.join(", ")
    );
    Ok(())
}

fn print_guess(guess: &FitsGuess) {
    println!("File: {} (only the header is read)", guess.file_name);
    let header = &guess.header;
    println!(
        "  OBJECT: {}",
        header.object.as_deref().unwrap_or("(absent)")
    );
    match header.ra_degrees.zip(header.dec_degrees) {
        Some((ra, dec)) => println!(
            "  Position: α {:.4}° · δ {:+.4}° ({:?}), field {:.1}° × {:.1}°",
            ra, dec, header.position_source, header.field_degrees[0], header.field_degrees[1]
        ),
        None => println!("  Position: (absent)"),
    }
    let verdict = match guess.kind {
        FitsGuessKind::Found => "found",
        FitsGuessKind::Hesitates => "the header hesitates",
        FitsGuessKind::Candidates => "candidates of the field",
        FitsGuessKind::UnknownObject => "object unknown to the catalogue",
        FitsGuessKind::Nothing => "neither object nor position",
    };
    println!("  Decision: {verdict}");
    if let Some(named) = &guess.named {
        println!("  → {}", card(named));
    }
    for candidate in &guess.candidates {
        println!(
            "  · {} ({:.2}° away)",
            card(&candidate.target),
            candidate.distance_degrees
        );
    }
}

pub fn fits_target(args: &FitsTargetArgs) -> Outcome {
    let (Some(portal), Some(token)) = (&args.portal, &args.token) else {
        let header = catalogue_targets::read_capture_header(&args.file)?;
        let read = catalogue_targets::header_target(&header);
        let file_name = args
            .file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let guess = catalogue_targets::decide(file_name, read, None, Vec::new());
        if args.json {
            return print_json(&guess.header);
        }
        print_guess(&guess);
        return Ok(());
    };
    let token = token_of(token)?;
    let data = args.data.clone().unwrap_or_else(std::env::temp_dir);
    let guess = catalogue_targets::guess_fits_target(
        &crate::words::portal_transport()?,
        portal,
        Some(&token),
        &args.file,
        &args.locale,
        &data.join(catalogue_targets::INDEX_FILE),
    )?;
    if args.json {
        return print_json(&guess);
    }
    print_guess(&guess);
    Ok(())
}

pub fn personal_target(args: &PersonalTargetArgs) -> Outcome {
    let spec = PersonalTargetCreate {
        name: args.name.clone(),
        label: args.label.clone(),
        ra_hours: args.ra_hours,
        dec_degrees: args.dec_degrees,
    };
    let body = catalogue_targets::personal_target_body(&spec)?;
    if args.dry_run {
        if args.json {
            return print_json(&body);
        }
        println!("(dry run) POST /v1/targets/personal {body}");
        return Ok(());
    }
    let token = token_of(&args.service.token)?;
    let created = catalogue_targets::create_personal_target(
        &crate::words::portal_transport()?,
        &args.service.portal,
        &token,
        &spec,
        args.session_id.as_deref(),
    )?;
    if args.json {
        return print_json(&created);
    }
    println!(
        "Target created: {} [{}]",
        created.target["name"].as_str().unwrap_or(""),
        created.target["id"].as_str().unwrap_or("")
    );
    if let Some(ids) = &created.night_target_ids {
        println!("Targets of the night: {}", ids.join(", "));
    }
    Ok(())
}
