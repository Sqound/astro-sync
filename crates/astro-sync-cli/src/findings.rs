//! What the detection of a night folder found, in the person's language: the
//! sentences are the window's (`wizard.files.*` in the ten dictionaries),
//! composed from the finding's key and parameters the way the window does.
//! A finding the window composes with a longer grammar (the darks that are
//! missing among others, the frames of another night, the files outside the
//! structure) keeps the core's English sentence.

use astro_sync_core::detection::Finding;
use serde_json::Value;

use crate::words::{say, say_plural};

/// `120` → "120", `0.5` → "0.5".
fn plain_number(value: f64) -> String {
    if (value - value.round()).abs() < 1e-9 {
        format!("{}", value.round() as i64)
    } else {
        format!("{value:.3}").trim_end_matches('0').to_string()
    }
}

/// The finding as one line in `locale`.
#[must_use]
pub fn sentence(finding: &Finding, locale: &str) -> String {
    composed(finding, locale).unwrap_or_else(|| finding.sentence())
}

fn composed(finding: &Finding, locale: &str) -> Option<String> {
    let text = |name: &str| {
        finding
            .params
            .get(name)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let count = finding
        .params
        .get("count")
        .and_then(Value::as_u64)
        .filter(|count| *count > 0)
        .unwrap_or(finding.files.len().max(1) as u64);
    let plural = |base: &str, vars: &[(&str, &str)]| say_plural(locale, base, count, vars);
    match finding.key {
        "detection.finalMissing" => say(locale, "wizard.files.finalMissing", &[]),
        "detection.finalFromDevice" => {
            let restack = finding
                .params
                .get("restackFrames")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            if restack > 0 {
                say(
                    locale,
                    "wizard.files.finalFromDeviceAndFrames",
                    &[("count", &restack.to_string())],
                )
            } else {
                say(locale, "wizard.files.finalFromDevice", &[])
            }
        }
        "detection.framesMissing" => say(locale, "wizard.files.framesMissing", &[]),
        "detection.noLogs" => say(locale, "wizard.files.noLogs", &[]),
        "detection.finalFitsRefused" => say(locale, "wizard.files.finalFitsRefused", &[]),
        "detection.targetEmpty" => Some(format!(
            "{}{}{}{}.",
            say(locale, "wizard.files.targetEmptyLead", &[])?,
            text("lightsFolder"),
            say(locale, "wizard.files.targetEmptyOr", &[])?,
            text("folder")
        )),
        "detection.framesMisfiled" => {
            let kind = Some(text("kind"))
                .filter(|kind| !kind.is_empty())
                .unwrap_or_else(|| "flat".into());
            let head = plural(&format!("wizard.files.misfiled.{kind}"), &[])?;
            let (from, to) = (text("from"), text("to"));
            if to.is_empty() {
                return Some(format!(
                    "{head} {from}{}",
                    say(locale, "wizard.files.misfiledToLights", &[])?
                ));
            }
            let link = if finding.params.get("fromLights").and_then(Value::as_bool) == Some(true) {
                plural(
                    "wizard.files.misfiledHeader",
                    &[("header", &text("header"))],
                )?
            } else {
                say(locale, "wizard.files.misfiledMove", &[])?
            };
            Some(format!("{head} {from}{link}{to}."))
        }
        "detection.calibrationHeaderDiffers" => {
            let kind = Some(text("kind"))
                .filter(|kind| !kind.is_empty())
                .unwrap_or_else(|| "dark".into());
            let kind_name = plural(&format!("wizard.files.calibrationKind.{kind}"), &[])?;
            let tail = plural(
                "wizard.files.calibrationHeaderDiffersTail",
                &[("header", &text("header")), ("kind", &kind_name)],
            )?;
            Some(format!(
                "{} {}{tail}",
                plural("wizard.files.calibrationHeaderDiffers", &[])?,
                text("folder")
            ))
        }
        "detection.framesOtherTarget" => {
            let object = text("object");
            Some(format!(
                "{} {}{}{object}/lights/.",
                plural("wizard.files.otherTarget", &[("object", &object)])?,
                text("folder"),
                say(locale, "wizard.files.otherTargetHeader", &[])?
            ))
        }
        "detection.flatsMissing" => {
            let none = finding.params.get("none").and_then(Value::as_bool) == Some(true);
            let filter = finding.params.get("filter").and_then(Value::as_str);
            let key = match (filter.is_some(), none) {
                (true, true) => "wizard.files.noFlatAtAll",
                (true, false) => "wizard.files.noFlat",
                (false, true) => "wizard.files.noFlatAtAllUnfiltered",
                (false, false) => "wizard.files.noFlatUnfiltered",
            };
            let sentence = say(locale, key, &[("filter", filter.unwrap_or_default())])?;
            Some(format!("{}: {sentence}", text("target")))
        }
        "detection.darksMissing"
            if finding.params.get("none").and_then(Value::as_bool) == Some(true) =>
        {
            let seconds = finding
                .params
                .get("exposureSeconds")
                .and_then(Value::as_f64)
                .map(|value| {
                    say(
                        locale,
                        "common.secondsShort",
                        &[("count", &plain_number(value))],
                    )
                })
                .unwrap_or_default()?;
            let mut settings = seconds;
            if let Some(gain) = finding.params.get("gain").and_then(Value::as_f64) {
                settings = format!(
                    "{settings}{}{}",
                    say(locale, "detect.join", &[])?,
                    say(
                        locale,
                        "wizard.files.gain",
                        &[("gain", &plain_number(gain))]
                    )?
                );
            }
            say(
                locale,
                "wizard.files.noDark",
                &[("settings", &settings), ("target", &text("target"))],
            )
        }
        "detection.linkTargetMissing" => Some(format!(
            "{} {}",
            say(locale, "wizard.files.linkMissing", &[])?,
            text("path")
        )),
        "detection.linkLoop" => Some(format!(
            "{} {}",
            say(locale, "wizard.files.linkLoop", &[])?,
            text("path")
        )),
        _ => None,
    }
}
