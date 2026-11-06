//! The person's data read from the service, **read only**, for
//! [`super::propose`]: "My places" (`GET /v1/places`), "My equipment"
//! (`GET /v1/equipment`), the plans (`GET /v1/plans`) and the sessions of
//! the dates detected (`GET /v1/sessions`, then each one's detail), and the
//! rights of the account from its capabilities. The readers are tolerant: a
//! field the service does not send is left empty, never an error.

use std::sync::Arc;

use serde_json::Value;

use crate::catalogue_targets::night_target_ids;
use crate::gui_bridge::{Capabilities, GuiBridgeError, HttpPortalGuiTransport, PortalGuiTransport};
use crate::multi_night::AttachGate;
use crate::night_service::NightServiceTransport;
use crate::service_wait::RequestFactory;
use crate::working_folder::evening_date_of;

use super::{
    AccountRights, KnownEquipment, KnownPlace, KnownPlan, KnownSession, KnownSlot, UserData,
};

/// The reads this module needs beyond the existing transports.
pub trait UserDataTransport {
    /// `GET {base}/api{path}` with the device's token, its JSON.
    fn read(&self, base_url: &str, token: &str, path: &str) -> Result<Value, GuiBridgeError>;
}

impl UserDataTransport for HttpPortalGuiTransport {
    fn read(&self, base_url: &str, token: &str, path: &str) -> Result<Value, GuiBridgeError> {
        let url = format!("{}/api{path}", base_url.trim_end_matches('/'));
        let token = token.to_owned();
        let build: RequestFactory =
            Arc::new(move |client| client.get(url.clone()).bearer_auth(&token));
        self.read_json(&build)
    }
}

fn text(value: &Value, pointers: &[&str]) -> Option<String> {
    pointers.iter().find_map(|p| {
        value
            .pointer(p)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    })
}

fn number(value: &Value, pointers: &[&str]) -> Option<f64> {
    pointers.iter().find_map(|p| {
        let v = value.pointer(p)?;
        v.as_f64().or_else(|| v.as_str()?.trim().parse().ok())
    })
}

fn rows<'a>(value: &'a Value, key: &str) -> Vec<&'a Value> {
    value
        .get(key)
        .or(Some(value))
        .and_then(Value::as_array)
        .map(|a| a.iter().collect())
        .unwrap_or_default()
}

/// `GET /v1/places`'s answer.
#[must_use]
pub fn places_of(answer: &Value) -> (Vec<KnownPlace>, Option<String>) {
    let places = rows(answer, "places")
        .into_iter()
        .filter_map(|p| {
            Some(KnownPlace {
                id: text(p, &["/id"])?,
                name: text(p, &["/name"]).unwrap_or_default(),
                latitude: number(p, &["/latitude"])?,
                longitude: number(p, &["/longitude"])?,
                time_zone: text(p, &["/timeZone"]),
                utc_offset_minutes: None,
            })
        })
        .collect();
    let default = text(
        answer,
        &["/defaultPlaceId", "/defaultPlace/id", "/default/id"],
    );
    (places, default)
}

/// `GET /v1/equipment`'s answer: the space's own items and the default rows.
#[must_use]
pub fn equipment_of(answer: &Value) -> Vec<KnownEquipment> {
    rows(answer, "equipment")
        .into_iter()
        .chain(rows(answer, "defaults"))
        .filter(|e| e.get("archivedAt").is_none_or(Value::is_null))
        .filter_map(|e| {
            Some(KnownEquipment {
                id: text(e, &["/id"])?,
                name: text(e, &["/name"]).unwrap_or_default(),
                kind: text(e, &["/type"]).unwrap_or_default(),
                manufacturer: text(e, &["/manufacturer"]),
                model: text(e, &["/model"]),
                focal_length_mm: number(
                    e,
                    &[
                        "/specifications/focalLength",
                        "/specifications/focalLengthMm",
                        "/specifications/focal_length",
                        "/planner/focalLength",
                    ],
                ),
            })
        })
        .collect()
}

/// `GET /v1/plans`'s answer (the plan's document under `payload`, or flat).
#[must_use]
pub fn plans_of(answer: &Value) -> Vec<KnownPlan> {
    rows(answer, "plans")
        .into_iter()
        .filter_map(|p| {
            let doc = p.get("payload").unwrap_or(p);
            let items: Vec<&Value> = doc
                .get("items")
                .and_then(Value::as_array)
                .map(|a| a.iter().collect())
                .unwrap_or_default();
            Some(KnownPlan {
                id: text(p, &["/id"]).or_else(|| text(doc, &["/id"]))?,
                name: text(doc, &["/name"]).unwrap_or_default(),
                evening_date: text(doc, &["/date"])?.chars().take(10).collect(),
                // A plan the device closed (`closedAs`: executed, partial,
                // abandoned, archived) keeps the resource status `active`
                // until its debriefing: its closure is what it is.
                status: text(doc, &["/closedAs"])
                    .or_else(|| text(p, &["/status"]))
                    .or_else(|| text(doc, &["/status"]))
                    .unwrap_or_default(),
                target_ids: items
                    .iter()
                    .filter_map(|i| text(i, &["/targetId", "/target/id"]))
                    .collect(),
                designations: items
                    .iter()
                    .filter_map(|i| text(i, &["/target/designation", "/designation"]))
                    .collect(),
                session_id: text(p, &["/sessionId"]).or_else(|| text(doc, &["/sessionId"])),
            })
        })
        .collect()
}

/// A session's detail (`GET /v1/sessions/{id}`) as the engine needs it.
#[must_use]
pub fn session_of(
    id: &str,
    plan_id: Option<String>,
    started_at: &str,
    detail: &Value,
) -> KnownSession {
    let evening = text(
        detail,
        &[
            "/night/eveningDate/value",
            "/night/date/value",
            "/night/date",
            "/session/eveningDate",
        ],
    )
    .or_else(|| evening_date_of(started_at, None))
    .unwrap_or_default();
    KnownSession {
        id: id.to_string(),
        plan_id,
        evening_date: evening.chars().take(10).collect(),
        place_id: text(
            detail,
            &[
                "/night/placeId/value",
                "/night/location/id",
                "/session/placeId",
            ],
        ),
        targets: slots_of(detail),
    }
}

/// The targets of a session's detail and their slots: the times observed
/// (`startedAt` / `finishedAt`), else the plan's slot on the clock
/// (`slot`); a removed target holds no slot. A detail without `targets` (an
/// older service) gives the night's target ids alone, without slots.
fn slots_of(detail: &Value) -> Vec<KnownSlot> {
    let Some(targets) = detail.get("targets").and_then(Value::as_array) else {
        return night_target_ids(detail)
            .into_iter()
            .map(|id| KnownSlot {
                target_id: Some(id),
                ..KnownSlot::default()
            })
            .collect();
    };
    targets
        .iter()
        .filter(|t| t.get("state").and_then(Value::as_str) != Some("removed"))
        .filter_map(|t| {
            Some(KnownSlot {
                target_id: Some(text(t, &["/targetId"])?),
                designation: text(t, &["/label"]),
                start: text(t, &["/startedAt", "/slot/start"]),
                end: text(t, &["/finishedAt", "/slot/end"]),
                filter: text(t, &["/filter"]),
            })
        })
        .collect()
}

/// The rights the proposal follows: "Advanced Planning" as Astro Sync
/// already gates it ([`AttachGate`]), several plans a night from
/// `planner.multiple` (always on a private server, which sells nothing).
#[must_use]
pub fn account_of(capabilities: Option<&Capabilities>, sold: bool) -> AccountRights {
    AccountRights {
        advanced_planning: AttachGate::new(capabilities, sold).allowed,
        several_plans_per_night: !sold || capabilities.is_some_and(|c| c.planner_multiple),
    }
}

/// Everything, read from the service: the sessions only of the evenings
/// `range` (inclusive, `YYYY-MM-DD`, a day of margin each side).
pub fn read_user_data<T>(
    transport: &T,
    base_url: &str,
    token: &str,
    account: AccountRights,
    range: Option<(String, String)>,
) -> Result<UserData, GuiBridgeError>
where
    T: UserDataTransport + PortalGuiTransport + NightServiceTransport,
{
    let (places, default_place_id) = places_of(&transport.read(base_url, token, "/v1/places")?);
    let equipment = equipment_of(&transport.read(base_url, token, "/v1/equipment")?);
    let plans = plans_of(&transport.read(base_url, token, "/v1/plans")?);
    let mut sessions = Vec::new();
    if let Some((first, last)) = range {
        let day = |date: &str, days: i64| {
            crate::time_util::parse_iso8601_utc_to_unix_seconds(&format!("{date}T00:00:00Z"))
                .map(|t| crate::time_util::format_iso8601_utc_millis(t + days as f64 * 86_400.0))
        };
        let (from, to) = (day(&first, -1), day(&last, 2));
        let mut since: Option<String> = None;
        loop {
            let page = transport.list_sessions(base_url, token, since.as_deref(), Some(100))?;
            for summary in &page.sessions {
                let inside = match (&from, &to) {
                    (Some(from), Some(to)) => {
                        summary.started_at.as_str() >= from.as_str()
                            && summary.started_at.as_str() < to.as_str()
                    }
                    _ => true,
                };
                if !inside {
                    continue;
                }
                let detail = transport.session_detail(base_url, token, &summary.id)?;
                sessions.push(session_of(
                    &summary.id,
                    summary.plan_id.clone(),
                    &summary.started_at,
                    &detail,
                ));
            }
            if !page.has_more || page.next_since.is_none() || page.sessions.is_empty() {
                break;
            }
            since = page.next_since;
        }
    }
    Ok(UserData {
        places,
        default_place_id,
        equipment,
        sessions,
        plans,
        account,
    })
}
