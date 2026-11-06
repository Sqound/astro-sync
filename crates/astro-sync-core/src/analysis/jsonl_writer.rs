//! Writer for `events/session.jsonl`: one compact JSON object per line
//! (`type`, `timestampUtc`, `source`, `data`), per the shape approved in
//! `contracts/schemas/session-package/metrics.md`. Reuses
//! [`crate::canonical_json`] (sorted keys, no insignificant whitespace) so
//! this writer does not grow its own key-ordering logic, and so two
//! renderings of the same event are byte-identical -- the same property
//! the CLI/GUI golden tests already rely on for other reports.

use crate::canonical_json::canonical_json;
use serde::Serialize;
use serde_json::Value;

/// One event bound for `events/session.jsonl`. `event_type` is a short,
/// stable identifier (`"dither"`, `"star-lost"`, `"pause"`, `"autofocus"`,
/// ...; the exact vocabulary is enumerated in `metrics.md`), `source` is
/// `"phd2"` or `"asiair"`, and `data` carries whatever fields that event
/// type recorded (e.g. `{"statusText": "SNR too low"}` for `star-lost`) --
/// an empty object when the event itself carries no extra data.
///
/// Derives `Serialize` (`analysis::report`) with the exact same field
/// shape [`write_session_jsonl`] already renders (`type`/
/// `timestampUtc`/`source`/`data`) -- purely additive, this writer's own
/// canonical-JSON rendering below is unchanged.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    pub timestamp_utc: String,
    pub source: String,
    pub data: Value,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionEventRecord<'a> {
    #[serde(rename = "type")]
    event_type: &'a str,
    timestamp_utc: &'a str,
    source: &'a str,
    data: &'a Value,
}

/// Renders every event as one canonical-JSON object per line, LF-separated,
/// with a trailing newline when `events` is non-empty. An empty slice
/// renders as an empty string: `events/session.jsonl` is an optional asset
/// (`contracts/schemas/session-package/README.md`), and an
/// empty file is a valid (if useless) instance of "no events recorded".
#[must_use]
pub fn write_session_jsonl(events: &[SessionEvent]) -> String {
    let mut out = String::new();
    for event in events {
        let record = SessionEventRecord {
            event_type: &event.event_type,
            timestamp_utc: &event.timestamp_utc,
            source: &event.source,
            data: &event.data,
        };
        out.push_str(&canonical_json(&record));
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_empty_event_list_renders_as_an_empty_string() {
        assert_eq!(write_session_jsonl(&[]), "");
    }

    #[test]
    fn one_event_renders_as_one_canonical_json_line() {
        let events = vec![SessionEvent {
            event_type: "star-lost".to_string(),
            timestamp_utc: "2026-08-07T14:39:00.000Z".to_string(),
            source: "phd2".to_string(),
            data: json!({"statusText": "SNR too low"}),
        }];
        let jsonl = write_session_jsonl(&events);
        assert_eq!(
            jsonl,
            "{\"data\":{\"statusText\":\"SNR too low\"},\"source\":\"phd2\",\"timestampUtc\":\"2026-08-07T14:39:00.000Z\",\"type\":\"star-lost\"}\n"
        );
    }

    #[test]
    fn an_event_with_no_extra_data_carries_an_empty_object() {
        let events = vec![SessionEvent {
            event_type: "pause".to_string(),
            timestamp_utc: "2026-08-07T14:30:00.000Z".to_string(),
            source: "phd2".to_string(),
            data: json!({}),
        }];
        let jsonl = write_session_jsonl(&events);
        assert!(jsonl.contains("\"data\":{}"));
    }

    #[test]
    fn multiple_events_are_lf_separated_with_one_trailing_newline() {
        let events = vec![
            SessionEvent {
                event_type: "dither".to_string(),
                timestamp_utc: "2026-08-07T14:10:00.000Z".to_string(),
                source: "asiair".to_string(),
                data: json!({}),
            },
            SessionEvent {
                event_type: "autofocus".to_string(),
                timestamp_utc: "2026-08-07T14:11:00.000Z".to_string(),
                source: "asiair".to_string(),
                data: json!({"temperatureC": 24.0}),
            },
        ];
        let jsonl = write_session_jsonl(&events);
        let lines: Vec<&str> = jsonl.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(!jsonl.contains('\r'));
        assert!(jsonl.ends_with('\n'));
        assert!(!jsonl.trim_end_matches('\n').ends_with('\n'));
    }

    #[test]
    fn each_line_is_valid_standalone_json() {
        let events = vec![SessionEvent {
            event_type: "meridian-flip".to_string(),
            timestamp_utc: "2026-08-07T15:00:00.000Z".to_string(),
            source: "asiair".to_string(),
            data: json!({}),
        }];
        let jsonl = write_session_jsonl(&events);
        for line in jsonl.lines() {
            let value: Value = serde_json::from_str(line).unwrap();
            assert_eq!(value["type"], "meridian-flip");
        }
    }
}
