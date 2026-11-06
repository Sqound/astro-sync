//! What Astro Sync knows how to say for the refusals the service answers
//! outside the sign-in (the sign-in's own are in [`crate::space`]): an
//! upload, a local analysis, the pairing of a private server, the night's
//! measured sky background.
//!
//! The contract names them (`contracts/schemas/api/v1.md`,
//! « Isolation between spaces », « Uploads and captures », and
//! `uploads.md`). Every one has
//!
//! - a **stable code**, the one the service sends, that the interface looks
//!   up as `errors.<code>` in the person's language
//!   (`app/src/i18n/{fr,en,ja}.json`);
//! - a **sentence in English** for the command line, which says what happened
//!   and what the person can do. It is the same text as the English
//!   dictionary of the application: a test reads `en.json` and fails when
//!   the two differ, so the command line and the window never say the same
//!   thing two ways.
//!
//! What a refusal *does* to the queue is decided elsewhere
//! ([`crate::uploader::UploadError::verdict`]): a refusal for good leaves the
//! queue, a refusal that needs the person holds it, a refusal that only asks
//! for patience is tried again later, by itself -- none of them is ever
//! sent again in a loop.

/// Codes the service sends, or that Astro Sync derives from the status when
/// the service names nothing it knows, with their English sentence.
///
/// A sentence says the reason and what the person can do; the wrapper around
/// it (the queue's notice, the command line's line) says what became of the
/// file.
const KNOWN: [(&str, &str); 20] = [
    (
        "capture_content_refused",
        "This file is an SVG or HTML document: the service does not accept it. If it was meant to be an image, export it as JPEG, PNG or TIFF, then add it again.",
    ),
    (
        "capture_limit_reached",
        "Not added: your free space has reached the number of captures it keeps. Delete a capture, or choose a storage offer, then start sending again.",
    ),
    (
        "storage_quota_reached",
        "Hosted storage is not available or is full for your space. Start sending again as soon as your space has room again, for example with a plan that gives more.",
    ),
    (
        "upload_sessions_limit_reached",
        "Too many uploads are in progress in your space. This file has not failed: it will resume by itself as soon as an upload finishes.",
    ),
    (
        "file_too_large",
        "This file is too large for the hosted storage of your space.",
    ),
    (
        "payload_too_large",
        "This upload is larger than the service accepts. It will not be sent again as it is: make it smaller (a smaller folder, fewer logs at a time), then try again.",
    ),
    (
        "session_limit_reached",
        "Your space has reached a limit of the service: this upload will not be repeated. It is the maximum number of observation nights; nothing already in your space is lost. This technical limit is very wide: if you reach it, contact support.",
    ),
    (
        "session_event_limit_reached",
        "Your space has reached a limit of the service: this upload will not be repeated. This observation night already holds the maximum number of events: the whole batch was refused, nothing was written, and nothing already recorded is lost. Open a new execution for the rest of the night.",
    ),
    (
        "entitlement_read_only",
        "The subscription of your Astro Portal space has ended: sending new files and new analyses is suspended, but everything already in your space stays readable. Start sending again as soon as the subscription is renewed.",
    ),
    (
        "analysis_not_entitled",
        "The service does not keep analysis reports: this one was not sent. The analysis itself stays here, on this computer.",
    ),
    (
        "place_not_found",
        "None of your places is within 500 m of where these frames were taken: the night’s value was not sent. Add this place to your places, on the phone or on the web, then send it again.",
    ),
    (
        "device_limit_reached",
        "This space has reached its maximum number of devices. Detach a device you no longer use from “My devices”, in the mobile application or on the web, then ask for a new code: this one has already been used.",
    ),
    (
        "upload_refused",
        "The service refused this file and will not accept it as it is. It will not be sent again: update Astro Sync, then add the file again if you want to try once more.",
    ),
    (
        "service_error",
        "The service cannot handle the request right now: it has a problem, or it is very busy. This file has not failed: it stays in the queue and sending will go on where it stopped.",
    ),
    (
        "capture_variant_limit_reached",
        "Not added: this capture already holds as many variants as your space allows.",
    ),
    (
        "variant_capture_pending",
        "Waiting: this image is a variant of the capture another image of this sending creates. It will leave as soon as that image is sent; send again.",
    ),
    (
        "project_not_found",
        "This plan does not exist in this space: import the night again from Sessions.",
    ),
    (
        "variant_capture_refused",
        "This file was not sent: import the night again from Sessions.",
    ),
    (
        "not_a_result",
        "Not sent: Astro Sync only sends results (a final or stacked image, a progression image). The FITS frames, the calibration and the raw logs stay on this computer.",
    ),
    (
        "analysis_empty",
        "Nothing to send: this night’s logs and frames give no step, no measure and no guiding. The analysis already online is kept.",
    ),
];

/// Every code of the catalogue, in the order of the contract's sentences.
pub fn known_codes() -> impl Iterator<Item = &'static str> {
    KNOWN.iter().map(|(code, _)| *code)
}

/// The stable code for a refusal code the service sent, when this Astro Sync
/// has a sentence for it. The service's own codes are the ones the interface
/// looks up; `upload_refused` and `service_error` are Astro Sync's, for a
/// refusal it does not know (`4xx`) and a failure of the service (`5xx`).
#[must_use]
pub fn stable_code(code: &str) -> Option<&'static str> {
    KNOWN
        .iter()
        .find(|(known, _)| *known == code)
        .map(|(known, _)| *known)
}

/// The sentence in English for a known code.
#[must_use]
pub fn plain_sentence(code: &str) -> Option<&'static str> {
    KNOWN
        .iter()
        .find(|(known, _)| *known == code)
        .map(|(_, sentence)| *sentence)
}

/// The stable code to show for an answer of `status` whose code is `code`:
/// the service's own when Astro Sync knows it, else one derived from the
/// status -- `upload_refused` for a refusal (`4xx`), `service_error` for a
/// failure of the service (`5xx`) or a service too busy (`408`, `429`),
/// `service_starting` for the one `5xx` the contract names -- and `None` for
/// anything else, which keeps its caller's generic code.
#[must_use]
pub fn code_for_answer(status: u16, code: &str) -> Option<&'static str> {
    if let Some(known) = stable_code(code) {
        return Some(known);
    }
    match status {
        500..=599 if code == "service_starting" => Some("service_starting"),
        // A failure of the service, or a service too busy to answer
        // (`408`, `429`): it will pass.
        408 | 429 | 500..=599 => Some("service_error"),
        400..=499 => Some("upload_refused"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_known_code_has_a_plain_sentence() {
        assert_eq!(known_codes().count(), KNOWN.len());
        for (code, sentence) in KNOWN {
            assert_eq!(stable_code(code), Some(code));
            assert_eq!(plain_sentence(code), Some(sentence));
            assert!(
                !sentence.contains(code) && !sentence.contains("HTTP"),
                "{code}: a sentence is plain words, never the technical code"
            );
            assert!(
                sentence.chars().count() > 40,
                "{code}: a sentence says the reason and what to do"
            );
        }
    }

    #[test]
    fn the_status_gives_a_code_to_a_refusal_nobody_knows_yet() {
        assert_eq!(
            code_for_answer(415, "capture_content_refused"),
            Some("capture_content_refused")
        );
        assert_eq!(
            code_for_answer(422, "something_new"),
            Some("upload_refused")
        );
        assert_eq!(
            code_for_answer(500, "internal_error"),
            Some("service_error")
        );
        assert_eq!(code_for_answer(429, "rate_limited"), Some("service_error"));
        assert_eq!(
            code_for_answer(503, "service_starting"),
            Some("service_starting")
        );
        assert_eq!(code_for_answer(302, "moved"), None);
    }

    /// A plan of another space is refused `404 project_not_found`, and the
    /// variants of the same capture are refused in cascade
    /// (`variant_capture_refused`): both say to import the night again, never
    /// « update Astro Sync ».
    #[test]
    fn a_plan_of_another_space_says_to_import_the_night_again() {
        for (status, code) in [(404, "project_not_found"), (400, "variant_capture_refused")] {
            assert_eq!(code_for_answer(status, code), Some(code));
            let sentence = plain_sentence(code).unwrap();
            assert!(sentence.contains("import the night again from Sessions"));
            assert!(!sentence.to_lowercase().contains("update"));
        }
    }

    /// The sentence of the fourth capture of a free space is said in each of
    /// the ten languages of the window, with the way out (delete a capture,
    /// or choose a storage offer) and neither its raw code nor a
    /// placeholder.
    #[test]
    fn the_capture_limit_is_said_in_the_ten_languages() {
        let mut seen = std::collections::BTreeSet::new();
        for locale in [
            "fr", "en", "ja", "de", "es", "it", "pt-BR", "zh-Hans", "zh-Hant", "ru",
        ] {
            let words = dictionary(locale);
            let sentence = words["errors.capture_limit_reached"]
                .as_str()
                .unwrap_or_else(|| panic!("{locale}.json has no errors.capture_limit_reached"));
            assert!(
                !sentence.contains("capture_limit") && !sentence.contains(['{', '}']),
                "{locale}: {sentence}"
            );
            assert!(sentence.chars().count() > 20, "{locale}: {sentence}");
            seen.insert(sentence.to_string());
        }
        assert_eq!(seen.len(), 10, "one sentence of its own in each language");
        assert_eq!(
            stable_code("capture_limit_reached"),
            Some("capture_limit_reached")
        );
        assert_eq!(
            code_for_answer(409, "capture_limit_reached"),
            Some("capture_limit_reached")
        );
    }

    fn dictionary(locale: &str) -> serde_json::Value {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../app/src/i18n/{locale}.json"));
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap()
    }

    /// The command line and the window say the same thing: the English of
    /// this catalogue is the English dictionary's, word for word -- and the
    /// window has a sentence of its own for the code in every language.
    #[test]
    fn the_command_line_says_what_the_dictionaries_say() {
        let english = dictionary("en");
        for (code, sentence) in KNOWN {
            let key = format!("errors.{code}");
            assert_eq!(
                english[&key].as_str(),
                Some(sentence),
                "{key} in en.json must be the sentence of the command line"
            );
        }
        // The ten languages of the window, each with a sentence of its own
        // for every code (including `capture_limit_reached`).
        for locale in [
            "fr", "ja", "de", "es", "it", "pt-BR", "zh-Hans", "zh-Hant", "ru",
        ] {
            let other = dictionary(locale);
            // A sentence of an ideographic language says as much in fewer
            // characters.
            let shortest = if matches!(locale, "ja" | "zh-Hans" | "zh-Hant") {
                10
            } else {
                20
            };
            for code in known_codes() {
                let key = format!("errors.{code}");
                assert!(
                    other[&key]
                        .as_str()
                        .is_some_and(|text| text.chars().count() > shortest),
                    "{locale}.json has no sentence for {key}"
                );
            }
        }
    }
}
