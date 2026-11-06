//! Attaching a night to a multi-night plan from Astro Sync: the night's page
//! offers "Attach to a multi-night plan", a window of the plans of the night's
//! target with a search, "Create a multi-night plan" for the night, and
//! "Detach".
//!
//! - **The choices** ([`proposed_plans`]): every multi-night plan of the
//!   space, each asked `GET /v1/projects/{id}/nights`, which says whether the
//!   night is in it already ([`ChoiceState::Current`]), may join it
//!   ([`ChoiceState::Available`], or [`ChoiceState::Moves`] from another
//!   plan: a night belongs to one plan at most) or is of another target
//!   ([`ChoiceState::OtherTarget`], not proposed). A night with no target
//!   recorded is offered every plan, and the service judges.
//! - **The search** ([`search_plans`]): by the name of a plan's target (its
//!   designation, its common name) -- a plan has no other name -- whatever
//!   the accents, the case and the spaces; it finds the plans that were not
//!   proposed.
//! - **The gestures** ([`attach_night`], [`detach_night`],
//!   [`create_plan_from_night`]): `POST` / `DELETE
//!   /v1/projects/{id}/nights`, `POST /v1/projects` with `fromSessionIds`.
//!   Under the right of the plans themselves (`planner.mosaic`,
//!   "Advanced Planning", always open on a private server: [`AttachGate`]);
//!   without it nothing is asked.
//!   A refusal of the service (`multi_night_not_active`, `target_mismatch`,
//!   `capability_required`, `session_not_found`…) is an outcome with its
//!   code and words, never an error.
//!
//! Reading never needs a capability, on any service. Nothing is written on
//! disk.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::gui_bridge::GuiBridgeError;
use crate::multi_night::{AttachGate, MultiNightClosed, MultiNightTransport};

/// The plans of the space whose nights are read to build the choices.
pub const MAX_READ: usize = 40;
/// The plans a search returns.
pub const MAX_FOUND: usize = 20;
/// The hours goal of a plan created from a night: 10 h, said as such and
/// changed in the planner.
pub const DEFAULT_GOAL_MINUTES: u64 = 600;
/// The minimum altitude and the minimum distance to the Moon (degrees) of a
/// plan created here: the planner's own defaults.
pub const DEFAULT_MIN_ALTITUDE: f64 = 30.0;
pub const DEFAULT_MOON_SEPARATION: f64 = 25.0;

/* ------------------------------- The choices ------------------------------ */

/// What the night can do with one plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChoiceState {
    /// The night may join it.
    Available,
    /// The night is in it already.
    Current,
    /// The night is in another plan; joining this one moves it.
    Moves,
    /// A plan of another target: the service would refuse
    /// (`target_mismatch`).
    OtherTarget,
}

/// One multi-night plan as the window lists it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanChoice {
    pub project_id: String,
    /// The plan's target as it names it (« IC 1805 »).
    pub name: String,
    pub common_name: Option<String>,
    /// The catalogue target or the framing the plan works for.
    pub target_id: String,
    pub goal_minutes: Option<u64>,
    /// The nights in the plan, run or planned.
    pub night_count: u64,
    /// The integrations of those nights, seconds.
    pub total_seconds: f64,
    pub state: ChoiceState,
    /// With [`ChoiceState::Moves`]: the plan the night is in now.
    pub from_project_id: Option<String>,
    pub from_name: Option<String>,
}

/// What the window proposes for a night.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanChoices {
    /// The plans proposed: the night's current plan first.
    pub plans: Vec<PlanChoice>,
    /// The night has no target recorded: every plan is proposed.
    pub no_target: bool,
    /// The plans of the space (the search is over all of them).
    pub total: u64,
}

/// The plans of the space that are not archived (`GET /v1/projects` lists
/// archived ones too).
fn list_plans<T: MultiNightTransport>(
    transport: &T,
    base_url: &str,
    token: &str,
) -> Result<Vec<Value>, GuiBridgeError> {
    let list = transport.projects(base_url, token)?;
    Ok(list
        .get("projects")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter(|p| p.get("id").and_then(Value::as_str).is_some())
                .filter(|p| p.get("status").and_then(Value::as_str) != Some("archived"))
                .cloned()
                .collect()
        })
        .unwrap_or_default())
}

fn text(value: &Value, pointer: &str) -> Option<String> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
}

fn sessions_of(rows: Option<&Value>) -> impl Iterator<Item = &Value> {
    rows.and_then(Value::as_array)
        .map(|rows| rows.iter())
        .into_iter()
        .flatten()
}

/// One plan (a row of the list) as a choice for the night `session_id`,
/// from its `GET …/nights`; `None` night: the state is [`ChoiceState::Available`].
fn choice_of(project: &Value, nights: Option<&Value>, session_id: Option<&str>) -> PlanChoice {
    let id = text(project, "/id").unwrap_or_default();
    let name = text(project, "/target/name").unwrap_or_else(|| id.clone());
    let own = nights.and_then(|n| n.get("nights"));
    let candidates = nights.and_then(|n| n.get("candidates"));
    let is_night = |row: &Value| {
        session_id.is_some() && row.get("sessionId").and_then(Value::as_str) == session_id
    };
    let (state, from_project_id) = match (session_id, nights) {
        (Some(_), Some(_)) => {
            if sessions_of(own).any(is_night) {
                (ChoiceState::Current, None)
            } else if let Some(row) = sessions_of(candidates).find(|row| is_night(row)) {
                match row.get("projectId").and_then(Value::as_str) {
                    Some(other) if other != id => (ChoiceState::Moves, Some(other.to_string())),
                    _ => (ChoiceState::Available, None),
                }
            } else {
                (ChoiceState::OtherTarget, None)
            }
        }
        _ => (ChoiceState::Available, None),
    };
    PlanChoice {
        project_id: id,
        name,
        common_name: text(project, "/target/commonName"),
        target_id: crate::multi_night::project_target_id(project),
        goal_minutes: project.get("goalMinutes").and_then(Value::as_u64),
        night_count: sessions_of(own).count() as u64,
        total_seconds: sessions_of(own)
            .filter_map(|row| row.get("integrationSeconds").and_then(Value::as_f64))
            .sum(),
        state,
        from_project_id,
        from_name: None,
    }
}

/// Reads `GET …/nights` of each plan and builds its choice for the night;
/// a plan the service no longer knows is left out.
fn read_choices<T: MultiNightTransport>(
    transport: &T,
    base_url: &str,
    token: &str,
    plans: &[Value],
    session_id: Option<&str>,
) -> Result<Vec<PlanChoice>, GuiBridgeError> {
    let mut choices = Vec::new();
    for project in plans {
        let id = text(project, "/id").unwrap_or_default();
        let nights = match transport.project_nights(base_url, token, &id) {
            Ok(nights) => Some(nights),
            Err(GuiBridgeError::Refused { .. }) => continue,
            Err(error) => return Err(error),
        };
        choices.push(choice_of(project, nights.as_ref(), session_id));
    }
    // "Moves the night of the plan ...": the plan the night is in, by its name.
    let names: Vec<(String, String)> = plans
        .iter()
        .filter_map(|p| Some((text(p, "/id")?, text(p, "/target/name")?)))
        .collect();
    for choice in &mut choices {
        if let Some(from) = &choice.from_project_id {
            choice.from_name = names
                .iter()
                .find(|(id, _)| id == from)
                .map(|(_, name)| name.clone());
        }
    }
    Ok(choices)
}

/// The plans proposed for a night: those of its target (the service says it
/// by the night's candidates), the current plan first. `has_target`: the
/// night has a target recorded; without one every plan is proposed.
pub fn proposed_plans<T: MultiNightTransport>(
    transport: &T,
    base_url: &str,
    token: &str,
    session_id: &str,
    has_target: bool,
) -> Result<PlanChoices, GuiBridgeError> {
    let plans = list_plans(transport, base_url, token)?;
    let total = plans.len() as u64;
    let read: Vec<Value> = plans.into_iter().take(MAX_READ).collect();
    let all = read_choices(transport, base_url, token, &read, Some(session_id))?;
    let mut proposed: Vec<PlanChoice> = all
        .iter()
        .filter(|choice| choice.state != ChoiceState::OtherTarget)
        .cloned()
        .collect();
    let mut no_target = false;
    if proposed.is_empty() && !has_target && !all.is_empty() {
        // The night names no target: every plan is offered, the service
        // judges (`409 target_mismatch`, said in clear).
        no_target = true;
        proposed = all
            .into_iter()
            .map(|choice| PlanChoice {
                state: ChoiceState::Available,
                ..choice
            })
            .collect();
    }
    proposed.sort_by(|a, b| {
        (a.state != ChoiceState::Current)
            .cmp(&(b.state != ChoiceState::Current))
            .then_with(|| fold(&a.name).cmp(&fold(&b.name)))
    });
    Ok(PlanChoices {
        plans: proposed,
        no_target,
        total,
    })
}

/// A text as the search compares it: lower case, accents removed, only
/// letters and digits (« NGC 7000 » = « ngc7000 », « Cœur » = « coeur »).
#[must_use]
pub fn fold(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars().flat_map(char::to_lowercase) {
        match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' => out.push('a'),
            'ç' | 'ć' | 'č' => out.push('c'),
            'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ė' | 'ę' => out.push('e'),
            'ì' | 'í' | 'î' | 'ï' | 'ī' => out.push('i'),
            'ñ' | 'ń' => out.push('n'),
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' => out.push('o'),
            'ù' | 'ú' | 'û' | 'ü' | 'ū' => out.push('u'),
            'ý' | 'ÿ' => out.push('y'),
            'ś' | 'š' => out.push('s'),
            'ž' | 'ź' | 'ż' => out.push('z'),
            'ł' => out.push('l'),
            'œ' => out.push_str("oe"),
            'æ' => out.push_str("ae"),
            'ß' => out.push_str("ss"),
            c if c.is_alphanumeric() => out.push(c),
            _ => {}
        }
    }
    out
}

/// Whether a plan answers to `query` (already folded): its target's
/// designation, its common name or its target's identifier.
fn matches(project: &Value, folded_query: &str) -> bool {
    ["/target/name", "/target/commonName", "/target/targetId"]
        .iter()
        .filter_map(|pointer| text(project, pointer))
        .any(|candidate| fold(&candidate).contains(folded_query))
}

/// The plans whose target answers to `query` (by name, whatever the accents,
/// the case and the spaces), among all those of the space, the first
/// [`MAX_FOUND`]. With `session_id` each says what the night can do with it.
/// An empty query finds none.
pub fn search_plans<T: MultiNightTransport>(
    transport: &T,
    base_url: &str,
    token: &str,
    session_id: Option<&str>,
    query: &str,
) -> Result<Vec<PlanChoice>, GuiBridgeError> {
    let folded = fold(query);
    if folded.is_empty() {
        return Ok(Vec::new());
    }
    let found: Vec<Value> = list_plans(transport, base_url, token)?
        .into_iter()
        .filter(|project| matches(project, &folded))
        .take(MAX_FOUND)
        .collect();
    let mut choices = read_choices(transport, base_url, token, &found, session_id)?;
    choices.sort_by_key(|choice| fold(&choice.name));
    Ok(choices)
}

/* ------------------------------- The gestures ----------------------------- */

/// What became of an attachment or a detachment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "result",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum NightPlanChange {
    /// The night is in the plan; `nights`: the plan's nights now.
    Attached { project_id: String, nights: u64 },
    /// The night is out of the plan.
    Detached { project_id: String, nights: u64 },
    /// Nothing asked: the right is missing here (said before any request).
    Closed {
        project_id: String,
        closed: MultiNightClosed,
    },
    /// The service refused: its code and words, shown as they are.
    Refused {
        project_id: String,
        code: String,
        message: String,
    },
}

fn nights_of(answer: &Value) -> u64 {
    answer
        .get("nights")
        .and_then(Value::as_array)
        .map_or(0, |nights| nights.len() as u64)
}

/// Attaches the night to the plan (`POST /v1/projects/{id}/nights`), under
/// the right of the multi-night plans. A night of another plan is moved.
pub fn attach_night<T: MultiNightTransport>(
    transport: &T,
    base_url: &str,
    token: &str,
    project_id: &str,
    session_id: &str,
    gate: AttachGate,
) -> Result<NightPlanChange, GuiBridgeError> {
    let project_id = project_id.to_string();
    if let Some(closed) = gate.closed {
        return Ok(NightPlanChange::Closed { project_id, closed });
    }
    let body = json!({ "sessionIds": [session_id] });
    match transport.attach_nights(base_url, token, &project_id, &body) {
        Ok(answer) => Ok(NightPlanChange::Attached {
            nights: nights_of(&answer),
            project_id,
        }),
        Err(GuiBridgeError::Refused { code, message }) => Ok(NightPlanChange::Refused {
            project_id,
            code,
            message,
        }),
        Err(error) => Err(error),
    }
}

/// Detaches the night from the plan (`DELETE /v1/projects/{id}/nights/{night}`),
/// under the same right. Idempotent.
pub fn detach_night<T: MultiNightTransport>(
    transport: &T,
    base_url: &str,
    token: &str,
    project_id: &str,
    session_id: &str,
    gate: AttachGate,
) -> Result<NightPlanChange, GuiBridgeError> {
    let project_id = project_id.to_string();
    if let Some(closed) = gate.closed {
        return Ok(NightPlanChange::Closed { project_id, closed });
    }
    match transport.detach_night(base_url, token, &project_id, session_id) {
        Ok(answer) => Ok(NightPlanChange::Detached {
            nights: nights_of(&answer),
            project_id,
        }),
        Err(GuiBridgeError::Refused { code, message }) => Ok(NightPlanChange::Refused {
            project_id,
            code,
            message,
        }),
        Err(error) => Err(error),
    }
}

/* -------------------------------- The creation ---------------------------- */

/// The target of a plan created from a night.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewPlanTarget {
    pub target_id: String,
    pub name: String,
    pub common_name: Option<String>,
}

/// The place a plan created from a night inherits: the night's own.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewPlanPlace {
    pub name: String,
    pub latitude: f64,
    pub longitude: f64,
}

/// The place of a night as the service holds it (`GET /v1/sessions/{id}`:
/// `night.place.value`), when it names a place with coordinates.
#[must_use]
pub fn place_of_detail(detail: &Value) -> Option<NewPlanPlace> {
    let place = detail.pointer("/night/place/value")?;
    let name: String = text(place, "/name")?.chars().take(300).collect();
    let latitude = place.get("latitude")?.as_f64()?;
    let longitude = place.get("longitude")?.as_f64()?;
    ((-90.0..=90.0).contains(&latitude) && (-180.0..=180.0).contains(&longitude)).then_some(
        NewPlanPlace {
            name,
            latitude,
            longitude,
        },
    )
}

/// What is missing to create a plan from the night.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NewPlanMissing {
    /// The night has no place with coordinates: the plan cannot inherit it.
    Place,
    /// No target to work for.
    Target,
}

/// What became of the creation of a plan from a night.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "result",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum PlanCreated {
    /// The plan exists, the night attached to it.
    Created {
        project_id: String,
        name: String,
        goal_minutes: u64,
        nights: u64,
    },
    /// Nothing asked: the right is missing here.
    Closed { closed: MultiNightClosed },
    /// Nothing asked: the night lacks what the plan inherits.
    Missing { missing: NewPlanMissing },
    /// The service refused: its code and words.
    Refused { code: String, message: String },
}

/// A random UUID (version 4) for the plan: the creation is sent once, with
/// an identifier fixed by the caller.
fn fresh_uuid() -> String {
    let mut bytes = [0u8; 16];
    let _ = getrandom::getrandom(&mut bytes);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// The body of `POST /v1/projects` for a plan "from" of one night
/// (`project.schema.json`, `create`).
#[must_use]
pub fn plan_body(
    project_id: &str,
    target: &NewPlanTarget,
    place: &NewPlanPlace,
    goal_minutes: u64,
    session_id: &str,
) -> Value {
    let mut target_value = json!({ "targetId": target.target_id, "name": target.name });
    if let Some(common) = target
        .common_name
        .as_deref()
        .filter(|c| !c.trim().is_empty())
    {
        target_value["commonName"] = json!(common);
    }
    json!({
        "id": project_id,
        "target": target_value,
        "goalMinutes": goal_minutes.clamp(30, 6000),
        "place": {
            "name": place.name,
            "latitude": place.latitude,
            "longitude": place.longitude,
        },
        "equipment": {},
        "constraints": {
            "minAltitude": DEFAULT_MIN_ALTITUDE,
            "moonSeparationMinimum": DEFAULT_MOON_SEPARATION,
            "moonSeparationMinimumByFilter": {},
        },
        "fromSessionIds": [session_id],
    })
}

/// Creates a blank multi-night plan for the night's target and attaches the
/// night to it (`POST /v1/projects` with `fromSessionIds`): the target is
/// the night's, the place the night's, the goal [`DEFAULT_GOAL_MINUTES`].
/// The service checks the night first and creates nothing unless it can be
/// attached.
pub fn create_plan_from_night<T: MultiNightTransport>(
    transport: &T,
    base_url: &str,
    token: &str,
    session_id: &str,
    target: &NewPlanTarget,
    place: Option<&NewPlanPlace>,
    gate: AttachGate,
) -> Result<PlanCreated, GuiBridgeError> {
    if let Some(closed) = gate.closed {
        return Ok(PlanCreated::Closed { closed });
    }
    if target.target_id.trim().is_empty() || target.name.trim().is_empty() {
        return Ok(PlanCreated::Missing {
            missing: NewPlanMissing::Target,
        });
    }
    let Some(place) = place else {
        return Ok(PlanCreated::Missing {
            missing: NewPlanMissing::Place,
        });
    };
    let project_id = fresh_uuid();
    let body = plan_body(&project_id, target, place, DEFAULT_GOAL_MINUTES, session_id);
    match transport.create_project(base_url, token, &body) {
        Ok(answer) => Ok(PlanCreated::Created {
            project_id: text(&answer, "/id").unwrap_or(project_id),
            name: text(&answer, "/target/name").unwrap_or_else(|| target.name.clone()),
            goal_minutes: answer
                .get("goalMinutes")
                .and_then(Value::as_u64)
                .unwrap_or(DEFAULT_GOAL_MINUTES),
            nights: nights_of(&answer),
        }),
        Err(GuiBridgeError::Refused { code, message }) => {
            Ok(PlanCreated::Refused { code, message })
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests;
