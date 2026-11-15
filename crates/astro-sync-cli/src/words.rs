//! What the command line says when something is refused or fails, in the
//! person's language: the sentences are the ten dictionaries of the window
//! (`app/src/i18n`), looked up by the same keys, so the command line and the
//! window say the same thing the same way. A code, a Rust variant's name or
//! the service's raw English is never shown to a person.
//!
//! The language is the one the command was asked for (`--lang`), else
//! `ASTRO_SYNC_LANG`, else the language of the computer when Astro Sync has
//! a dictionary for it, else English.

use std::error::Error;

use astro_sync_core::analysis::AnalysisError;
use astro_sync_core::detection::DetectionError;
use astro_sync_core::gui_bridge::{GuiBridgeError, HttpPortalGuiTransport};
use astro_sync_core::python_env::EnvFailure;
use astro_sync_core::scanner::ScanError;
use astro_sync_core::service_wait::{ServiceEvent, ServiceWaitError, WaitHooks, WakePolicy};
use astro_sync_core::session_detection::DetectError;
use astro_sync_core::stacking::{EngineCause, EngineNotice, StackFailure};
use astro_sync_core::tools::DownloadFailure;
use astro_sync_core::uploader::{HttpUploadTransport, UploadError};
use serde_json::Value;

/// The ten dictionaries of the application, by locale.
const DICTIONARIES: [(&str, &str); 10] = [
    ("fr", include_str!("../../../app/src/i18n/fr.json")),
    ("en", include_str!("../../../app/src/i18n/en.json")),
    ("ja", include_str!("../../../app/src/i18n/ja.json")),
    ("de", include_str!("../../../app/src/i18n/de.json")),
    ("es", include_str!("../../../app/src/i18n/es.json")),
    ("it", include_str!("../../../app/src/i18n/it.json")),
    ("pt-BR", include_str!("../../../app/src/i18n/pt-BR.json")),
    ("ru", include_str!("../../../app/src/i18n/ru.json")),
    (
        "zh-Hans",
        include_str!("../../../app/src/i18n/zh-Hans.json"),
    ),
    (
        "zh-Hant",
        include_str!("../../../app/src/i18n/zh-Hant.json"),
    ),
];

/// The locale of a dictionary for `requested` (`fr`, `pt_BR`, `zh-TW`…), when
/// there is one.
#[must_use]
pub fn locale_of(requested: &str) -> Option<&'static str> {
    let normal = requested
        .split(['.', '@'])
        .next()
        .unwrap_or_default()
        .replace('_', "-");
    let lower = normal.to_ascii_lowercase();
    let exact = DICTIONARIES
        .iter()
        .find(|(locale, _)| locale.eq_ignore_ascii_case(&normal))
        .map(|(locale, _)| *locale);
    exact.or_else(|| match lower.as_str() {
        "zh-tw" | "zh-hk" | "zh-mo" => Some("zh-Hant"),
        "zh-cn" | "zh-sg" | "zh" => Some("zh-Hans"),
        "pt" | "pt-pt" => Some("pt-BR"),
        other => {
            let base = other.split('-').next().unwrap_or_default();
            DICTIONARIES
                .iter()
                .find(|(locale, _)| locale.eq_ignore_ascii_case(base))
                .map(|(locale, _)| *locale)
        }
    })
}

/// The language of this run: `--lang <x>` or `--lang=<x>` on the command line,
/// else `ASTRO_SYNC_LANG`, else the computer's, else English.
#[must_use]
pub fn requested_language() -> &'static str {
    let args: Vec<String> = std::env::args().collect();
    let mut asked: Option<String> = None;
    for (index, arg) in args.iter().enumerate() {
        if let Some(value) = arg.strip_prefix("--lang=") {
            asked = Some(value.to_string());
        } else if arg == "--lang" {
            asked = args.get(index + 1).cloned();
        }
    }
    let chosen = asked
        .into_iter()
        .chain(std::env::var("ASTRO_SYNC_LANG"))
        .chain(std::env::var("LC_ALL"))
        .chain(std::env::var("LANG"))
        .find_map(|value| locale_of(&value));
    chosen.unwrap_or("en")
}

/// A dictionary, parsed the first time it is asked for (170 KB each).
fn dictionary(locale: &str) -> &'static Value {
    cached(locale)
}

fn cached(locale: &str) -> &'static Value {
    use std::collections::BTreeMap;
    use std::sync::Mutex;
    static CACHE: Mutex<BTreeMap<&'static str, &'static Value>> = Mutex::new(BTreeMap::new());
    let name = DICTIONARIES
        .iter()
        .find(|(known, _)| *known == locale)
        .map_or("en", |(known, _)| *known);
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cache.entry(name).or_insert_with(|| {
        let text = DICTIONARIES
            .iter()
            .find(|(known, _)| *known == name)
            .map_or("{}", |(_, text)| *text);
        Box::leak(Box::new(serde_json::from_str(text).unwrap_or(Value::Null)))
    })
}

/// The sentence of `key` in `locale` with its `{placeholders}` filled; the
/// English one when the language has none; `None` when neither does.
#[must_use]
pub fn say(locale: &str, key: &str, vars: &[(&str, &str)]) -> Option<String> {
    let raw = dictionary(locale)[key]
        .as_str()
        .or_else(|| dictionary("en")[key].as_str())?;
    let mut text = raw.to_string();
    for (name, value) in vars {
        text = text.replace(&format!("{{{name}}}"), value);
    }
    Some(text)
}

/// Whether `count` takes the singular form of the dictionaries' `.one` keys
/// in `locale` (the other keys are `.other`).
#[must_use]
pub fn is_one(locale: &str, count: u64) -> bool {
    match locale {
        "ja" | "zh-Hans" | "zh-Hant" => false,
        "fr" | "pt-BR" => count <= 1,
        "ru" => count % 10 == 1 && count % 100 != 11,
        _ => count == 1,
    }
}

/// [`say`] for a key that comes in two forms (`<base>.one`, `<base>.other`),
/// with `{count}` filled.
#[must_use]
pub fn say_plural(locale: &str, base: &str, count: u64, vars: &[(&str, &str)]) -> Option<String> {
    let form = if is_one(locale, count) {
        "one"
    } else {
        "other"
    };
    let number = count.to_string();
    let mut all = vec![("count", number.as_str())];
    all.extend_from_slice(vars);
    say(locale, &format!("{base}.{form}"), &all)
}

/// A length of time as the window writes it ("47 s", "2 min 14 s", "1 h 02 min").
#[must_use]
pub fn duration_text(locale: &str, seconds: u64) -> String {
    let two = |value: u64| format!("{value:02}");
    let (key, vars) = if seconds < 60 {
        (
            "wizard.sending.durationSeconds",
            vec![("seconds", seconds.to_string())],
        )
    } else if seconds < 3600 {
        (
            "wizard.sending.durationMinutes",
            vec![
                ("minutes", (seconds / 60).to_string()),
                ("seconds", two(seconds % 60)),
            ],
        )
    } else {
        (
            "wizard.sending.durationHours",
            vec![
                ("hours", (seconds / 3600).to_string()),
                ("minutes", two((seconds % 3600) / 60)),
            ],
        )
    };
    let vars: Vec<(&str, &str)> = vars
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect();
    say(locale, key, &vars).unwrap_or_else(|| format!("{seconds} s"))
}

/// Why a file or a call failed, in the person's language: the sentence of its
/// code in the dictionaries when it has one without a blank to fill, else the
/// text the core gave (English).
#[must_use]
pub fn reason_text(code: &str, message: &str, locale: &str) -> String {
    say(locale, &format!("errors.{code}"), &[])
        .filter(|sentence| !sentence.contains('{'))
        .unwrap_or_else(|| message.to_string())
}

/// A size on this computer as the window writes it (base 1 000, as the system
/// counts): the largest readable unit, one decimal only when it says
/// something ("46 Mo", "1,3 Go"), in the language's own units and decimal
/// mark. What the space counts is [`space_size_text`].
#[must_use]
pub fn size_text(locale: &str, bytes: u64) -> String {
    size_in(locale, bytes, 1_000)
}

/// A size the space counts (its quota, what is left, its ceiling, the files
/// sent), written as [`size_text`] but in base 1 024, as the service and the
/// portal count it: a quota of 150 x 1 024 x 1 024 bytes reads "150 MB".
#[must_use]
pub fn space_size_text(locale: &str, bytes: u64) -> String {
    size_in(locale, bytes, 1_024)
}

fn size_in(locale: &str, bytes: u64, unit: u64) -> String {
    let comma = matches!(locale, "fr" | "de" | "es" | "it" | "pt-BR" | "ru");
    let value = |amount: f64| {
        let floored = (amount * 10.0 + 1e-6).floor() / 10.0;
        let text = if (floored - floored.round()).abs() < 1e-9 {
            format!("{floored:.0}")
        } else {
            format!("{floored:.1}")
        };
        if comma { text.replace('.', ",") } else { text }
    };
    let (kilo, mega, giga) = (unit, unit * unit, unit * unit * unit);
    let (key, amount) = if bytes >= giga {
        ("common.gigabytes", value(bytes as f64 / giga as f64))
    } else if bytes >= mega {
        ("common.megabytes", value(bytes as f64 / mega as f64))
    } else if bytes >= kilo {
        ("common.kilobytes", value(bytes as f64 / kilo as f64))
    } else {
        ("common.bytes", bytes.to_string())
    };
    say(locale, key, &[("value", &amount)]).unwrap_or(amount)
}

/// What the commands that talk to the service say while it wakes (the
/// README's "A sleeping hosted service"): "The service is starting…" on the
/// standard error after 4 s of silence, then that it answered; the standard
/// output stays a clean result.
#[must_use]
pub fn wake_hooks(locale: &'static str) -> WaitHooks {
    WaitHooks::with_events(move |event| match event {
        ServiceEvent::Starting => {
            if let Some(line) = say(locale, "service.starting", &[]) {
                eprintln!("{line}");
            }
        }
        ServiceEvent::Ready => {
            if let Some(line) = say(locale, "service.ready", &[]) {
                eprintln!("{line}");
            }
        }
        ServiceEvent::Stopped => {}
    })
}

/// The transport of the commands that read and write the service, patient
/// with a sleeping one and saying so.
pub fn portal_transport() -> Result<HttpPortalGuiTransport, GuiBridgeError> {
    HttpPortalGuiTransport::with_options(WakePolicy::default(), wake_hooks(requested_language()))
}

/// [`portal_transport`] for the resumable uploads.
pub fn upload_transport(portal: &str) -> Result<HttpUploadTransport, UploadError> {
    HttpUploadTransport::with_options(
        portal,
        WakePolicy::default(),
        wake_hooks(requested_language()),
    )
}

/// The key of the dictionary for the code a stacking script failed with (the
/// window's table, `wizardSummary.ts`).
fn stacking_failure_key(code: &str) -> Option<&'static str> {
    Some(match code {
        "bad_input"
        | "folder_missing"
        | "frames_outside_lights"
        | "output_inside_input"
        | "output_is_final"
        | "work_not_empty" => "bad_input",
        "too_few_lights" => "too_few_lights",
        "device_stretched" => "device_stretched",
        "device_unreadable" => "device_unreadable",
        "script_not_trusted" => "script_not_trusted",
        "unknown_pipeline" => "unknown_pipeline",
        "siril_missing" => "siril_missing",
        "starnet_missing" => "starnet_missing",
        "tool_missing" => "tool_missing_here",
        "registration_failed" => "registration_failed",
        "calibration_failed" | "integration_failed" => "siril_failed",
        "starnet_failed" => "starnet_failed",
        "render_failed" | "script_failed" | "tool_failed" => "tool_failed",
        _ => return None,
    })
}

/// Why a stacking gave no image, in words (the window's sentences): never the
/// name of a variant or a code.
#[must_use]
pub fn stack_failure_text(failure: &StackFailure, locale: &str) -> String {
    let plain = |key: &str| say(locale, &format!("errors.stacking.{key}"), &[]);
    let sentence = match failure {
        StackFailure::NotInSubscription => plain("not_in_subscription"),
        StackFailure::ToolMissing => say(locale, "cli.stack.tool_missing", &[]),
        StackFailure::OneRenderOnly { .. } => plain("one_render_only"),
        StackFailure::NoLights => plain("no_lights"),
        StackFailure::Cancelled => plain("cancelled"),
        StackFailure::OutsideStacked { .. } => plain("outside_stacked"),
        StackFailure::NotEnoughSpace {
            needed,
            available,
            folder,
        } => say(
            locale,
            "errors.stacking.not_enough_space",
            &[
                ("folder", folder),
                ("needed", &size_text(locale, *needed)),
                ("available", &size_text(locale, *available)),
            ],
        ),
        StackFailure::Failed { code, logs, .. } => match stacking_failure_key(code) {
            Some(key @ ("siril_failed" | "registration_failed" | "tool_failed")) => match logs {
                Some(logs) => say(locale, &format!("errors.stacking.{key}"), &[("logs", logs)]),
                None => plain(&format!("{key}_no_logs")),
            },
            // The window sends to « Réglages › Dépendances »; the command
            // line has its own commands.
            Some(key @ ("siril_missing" | "starnet_missing" | "tool_missing_here")) => {
                say(locale, &format!("cli.stack.{key}"), &[])
            }
            Some("unknown_pipeline") => say(locale, "cli.stack.unknown_pipeline", &[]),
            Some(key) => plain(key),
            None => say(locale, "errors.stacking_failed", &[]),
        },
    };
    sentence.unwrap_or_else(|| {
        say(locale, "errors.stacking_failed", &[]).unwrap_or_else(|| "stacking failed".into())
    })
}

/// Why a download of a tool stopped, in the window's sentences
/// (`deps.line.*`): what happened and whether anything is installed.
#[must_use]
pub fn download_failure_text(failure: &DownloadFailure, locale: &str) -> String {
    let kept = say(locale, "deps.line.kept", &[]);
    let joined = |line: Option<String>| {
        [line, kept.clone()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ")
    };
    let line = match failure {
        DownloadFailure::NoAutomaticDownload => say(locale, "deps.line.noAutomatic", &[]),
        DownloadFailure::Interrupted {
            kept_bytes, total, ..
        } => {
            let done = size_text(locale, *kept_bytes);
            return joined(match total {
                Some(total) => say(
                    locale,
                    "deps.line.interrupted",
                    &[("done", &done), ("total", &size_text(locale, *total))],
                ),
                None => say(locale, "deps.line.interruptedPlain", &[("done", &done)]),
            });
        }
        DownloadFailure::Cancelled { kept_bytes } => {
            let done = size_text(locale, *kept_bytes);
            return joined(say(
                locale,
                "deps.line.interruptedPlain",
                &[("done", &done)],
            ));
        }
        DownloadFailure::DigestMismatch => say(locale, "deps.line.digest", &[]),
        DownloadFailure::Extraction { .. } => say(locale, "deps.line.extraction", &[]),
        DownloadFailure::Io { .. } => say(locale, "deps.line.io", &[]),
    };
    line.unwrap_or_else(|| format!("{failure:?}"))
}

/// Why the Python environment's installation stopped, in the window's
/// sentences (`deps.env.*`).
#[must_use]
pub fn env_failure_text(failure: &EnvFailure, locale: &str) -> String {
    let (line, keeps) = match failure {
        EnvFailure::NoPython { first, last } => (
            say(
                locale,
                "deps.env.noPython",
                &[("first", first), ("last", last)],
            ),
            false,
        ),
        EnvFailure::NoResources => (say(locale, "deps.env.noResources", &[]), false),
        EnvFailure::Venv { .. } => (say(locale, "deps.env.venv", &[]), false),
        EnvFailure::Packages { .. } => (say(locale, "deps.env.packages", &[]), true),
        EnvFailure::Check { problems, .. } if !problems.is_empty() => (
            say(
                locale,
                "deps.env.check",
                &[("problems", &problems.join(", "))],
            ),
            false,
        ),
        EnvFailure::Check { .. } => (say(locale, "deps.env.checkPlain", &[]), false),
        EnvFailure::Cancelled => (say(locale, "deps.env.cancelled", &[]), true),
    };
    let Some(line) = line else {
        return format!("{failure:?}");
    };
    let keep = if keeps {
        say(locale, "deps.env.kept", &[])
    } else {
        None
    };
    [Some(line), keep]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ")
}

/// What the stacking did when it fell back on another engine than the one
/// asked for (GraXpert, then Siril, then a polynomial): the cause, then what
/// did the work instead, in the person's language.
#[must_use]
pub fn engine_notice_text(notice: &EngineNotice, locale: &str) -> String {
    let cause = notice
        .causes
        .iter()
        .filter_map(|cause| {
            let key = match cause {
                EngineCause::GraxpertMissing => "graxpertMissing",
                EngineCause::GraxpertNoLibrary => "graxpertNoLibrary",
                EngineCause::GraxpertFailed => "graxpertFailed",
                EngineCause::SirilMissing => "sirilMissing",
                EngineCause::SirilFailed => "sirilFailed",
            };
            say(locale, &format!("cli.stack.engine.{key}"), &[])
        })
        .collect::<Vec<_>>()
        .join(" ");
    let used = say(
        locale,
        if notice.used == "siril" {
            "cli.stack.engine.usedSiril"
        } else {
            "cli.stack.engine.usedPolynomial"
        },
        &[],
    )
    .unwrap_or_default();
    say(
        locale,
        "cli.stack.engineNotice",
        &[("cause", &cause), ("used", &used)],
    )
    .unwrap_or_else(|| format!("{cause} {used}"))
}

/// The code a failure carries, when it is one of Astro Sync's own.
fn code_of(error: &(dyn Error + 'static)) -> Option<&'static str> {
    if let Some(error) = error.downcast_ref::<GuiBridgeError>() {
        return Some(error.code());
    }
    if let Some(error) = error.downcast_ref::<UploadError>() {
        return Some(error.code());
    }
    if let Some(error) = error.downcast_ref::<ServiceWaitError>() {
        return Some(error.code());
    }
    if let Some(error) = error.downcast_ref::<DetectionError>() {
        return Some(error.code());
    }
    if let Some(error) = error.downcast_ref::<DetectError>() {
        return Some(match error.code() {
            "folder_missing" => "folder_not_found",
            other => other,
        });
    }
    if let Some(ScanError::NotADirectory(_)) = error.downcast_ref::<ScanError>() {
        return Some("folder_not_found");
    }
    if let Some(AnalysisError::Scan(ScanError::NotADirectory(_))) =
        error.downcast_ref::<AnalysisError>()
    {
        return Some("folder_not_found");
    }
    None
}

/// The code of a refusal the service answered, whatever the command that
/// asked.
fn refused_code<'a>(error: &'a (dyn Error + 'static)) -> Option<&'a str> {
    match error.downcast_ref::<GuiBridgeError>() {
        Some(GuiBridgeError::Refused { code, .. }) => Some(code),
        _ => match error.downcast_ref::<UploadError>() {
            Some(UploadError::Refused { code, .. }) => Some(code),
            _ => None,
        },
    }
}

/// What a person reads for `error`: the sentence of its code in their
/// language when Astro Sync has one, else the error as it is (the command
/// line's own English sentences).
#[must_use]
pub fn error_text(error: &(dyn Error + 'static), locale: &str) -> String {
    let mut current: Option<&(dyn Error + 'static)> = Some(error);
    while let Some(candidate) = current {
        if refused_code(candidate) == Some("target_not_found")
            && let Some(sentence) = say(locale, "cli.targetNotFound", &[])
        {
            return sentence;
        }
        if let Some(code) = code_of(candidate)
            && let Some(sentence) = say(locale, &format!("errors.{code}"), &[])
            && !sentence.contains('{')
        {
            return sentence;
        }
        current = candidate.source();
    }
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_locale_is_found_from_the_way_a_computer_names_it() {
        assert_eq!(locale_of("fr_FR.UTF-8"), Some("fr"));
        assert_eq!(locale_of("ja_JP.UTF-8"), Some("ja"));
        assert_eq!(locale_of("pt_BR"), Some("pt-BR"));
        assert_eq!(locale_of("zh_TW"), Some("zh-Hant"));
        assert_eq!(locale_of("zh-CN"), Some("zh-Hans"));
        assert_eq!(locale_of("C"), None);
        assert_eq!(locale_of("sv"), None);
    }

    #[test]
    fn every_dictionary_parses_and_says_the_network_error() {
        for (locale, _) in DICTIONARIES {
            let sentence = say(locale, "errors.network_error", &[]).unwrap();
            assert!(sentence.chars().count() > 10, "{locale}");
        }
        assert_ne!(
            say("ja", "errors.network_error", &[]),
            say("fr", "errors.network_error", &[])
        );
    }

    #[test]
    fn the_placeholders_are_filled() {
        let sentence = say(
            "fr",
            "wizard.summary.refused.fileTooLarge",
            &[("file", "a.tif"), ("size", "46 Mo"), ("max", "25 Mo")],
        )
        .unwrap();
        assert!(sentence.contains("a.tif") && sentence.contains("46 Mo"));
        assert!(!sentence.contains('{'));
    }

    #[test]
    fn what_the_space_counts_is_written_in_base_1024_as_the_portal_writes_it() {
        // The quota of a space: 150 x 1 024 x 1 024 bytes, « 150 MB » on the
        // portal.
        assert_eq!(space_size_text("en", 157_286_400), "150 MB");
        assert_eq!(
            space_size_text("fr", 157_286_400).replace('\u{a0}', " "),
            "150 Mo"
        );
        assert_eq!(space_size_text("en", 1_536), "1.5 KB");
        // On this computer, the system's base 1 000 stays.
        assert_eq!(size_text("en", 157_286_400), "157.2 MB");
    }
}

#[cfg(test)]
mod failure_text_tests {
    use super::*;

    #[test]
    fn a_refused_download_says_why_nothing_is_installed() {
        for (locale, _) in DICTIONARIES {
            let text = download_failure_text(&DownloadFailure::DigestMismatch, locale);
            assert_ne!(text, "DigestMismatch", "{locale}");
            assert!(text.chars().count() > 15, "{locale}");
        }
        let digest_line = say("fr", "deps.line.digest", &[]).expect("fr digest line");
        assert!(
            download_failure_text(&DownloadFailure::DigestMismatch, "fr").contains(&digest_line)
        );
        let cut = download_failure_text(
            &DownloadFailure::Interrupted {
                kept_bytes: 184_000_000,
                total: Some(410_000_000),
                detail: String::new(),
            },
            "en",
        );
        assert!(cut.contains("184 MB") && cut.contains("410 MB"), "{cut}");
        assert!(
            env_failure_text(
                &EnvFailure::Packages {
                    detail: String::new()
                },
                "fr"
            )
            .contains(&say("fr", "deps.env.kept", &[]).expect("fr kept line"))
        );
    }

    #[test]
    fn a_fallback_of_the_engine_says_its_cause_and_the_engine_that_worked() {
        let notice = EngineNotice {
            family: None,
            used: "siril".to_string(),
            causes: vec![EngineCause::GraxpertNoLibrary],
        };
        let text = engine_notice_text(&notice, "fr");
        assert!(
            text.contains("onnxruntime") && text.contains("Siril"),
            "{text}"
        );
        assert!(!text.contains('{'));
        for (locale, _) in DICTIONARIES {
            assert!(
                !engine_notice_text(&notice, locale).contains('{'),
                "{locale}"
            );
        }
    }

    #[test]
    fn a_failed_alignment_is_not_a_generic_failure_of_siril() {
        let failure = StackFailure::Failed {
            code: "registration_failed".to_string(),
            message: String::new(),
            logs: Some("/data/logs".to_string()),
        };
        let text = stack_failure_text(&failure, "en");
        assert!(
            text.contains("align") && text.contains("/data/logs"),
            "{text}"
        );
        let missing = StackFailure::Failed {
            code: "siril_missing".to_string(),
            message: String::new(),
            logs: None,
        };
        assert!(stack_failure_text(&missing, "en").contains("astro-sync tools"));
    }
}
