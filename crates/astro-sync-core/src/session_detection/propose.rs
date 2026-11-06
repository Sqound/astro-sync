//! Step 3: what was found, matched to the person's places, equipment,
//! sessions and plans, under the rights of the account; then the lines of
//! the review screen. Pure: the same grouping and data give the same lines.

use crate::detection::FrameType;
use crate::time_util::parse_iso8601_utc_to_unix_seconds;

use super::group::{Grouping, loose};
use super::{
    AccountRights, DetectedSession, DetectedTarget, EquipmentMatch, EquipmentProposal,
    KnownEquipment, KnownPlan, KnownSession, LineKind, PLACE_RADIUS_KM, PlaceAction,
    PlaceCandidate, PlaceProposal, Proposal, ProposalLine, ProposalNote, SessionMatch, TargetMatch,
    TargetSource, UserData, distance_km,
};

fn line(kind: LineKind, subject: &str, checked: bool, reason: &str, text: String) -> ProposalLine {
    ProposalLine {
        id: format!("{reason}:{subject}"),
        kind,
        subject: subject.to_string(),
        checked,
        locked: false,
        ask: false,
        reason: reason.to_string(),
        text,
    }
}

/// « 11 h 09 », « 6 min ».
pub(super) fn duration_text(seconds: f64) -> String {
    let minutes = (seconds / 60.0).round() as i64;
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m:02}"),
    }
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count > 1 { many } else { one })
}

fn match_places(place: &mut PlaceProposal, user: &UserData) {
    let Some((lat, lon)) = place.latitude.zip(place.longitude) else {
        place.action = PlaceAction::Choose;
        place.chosen = user.default_place_id.clone();
        return;
    };
    let mut candidates: Vec<PlaceCandidate> = user
        .places
        .iter()
        .map(|p| PlaceCandidate {
            place_id: p.id.clone(),
            name: p.name.clone(),
            distance_km: (distance_km(lat, lon, p.latitude, p.longitude) * 10.0).round() / 10.0,
        })
        .filter(|c| c.distance_km <= PLACE_RADIUS_KM)
        .collect();
    candidates.sort_by(|a, b| a.distance_km.total_cmp(&b.distance_km));
    place.chosen = candidates.first().map(|c| c.place_id.clone());
    place.action = match candidates.len() {
        0 => PlaceAction::Create,
        1 => PlaceAction::Use,
        _ => PlaceAction::UseNearest,
    };
    place.candidates = candidates;
}

fn words_of(item: &KnownEquipment) -> Vec<String> {
    [Some(&item.name), item.model.as_ref()]
        .into_iter()
        .flatten()
        .map(|w| loose(w))
        .filter(|w| w.len() >= 3)
        .collect()
}

/// Whether a header's name and an item's name are the same thing: one holds
/// the other, letters and digits only (« ZWO ASI533MC Pro » and « ASI533MC
/// Pro »).
fn names_item(header: &str, item: &KnownEquipment) -> bool {
    let header = loose(header);
    header.len() >= 3
        && words_of(item)
            .iter()
            .any(|w| header.contains(w.as_str()) || w.contains(header.as_str()))
}

/// Whether a `FILTER` name is that filter of "My equipment": the same name,
/// its first word (« Ha » for « Ha 7 nm »), or one of the names the other
/// kinds of equipment match by. Short names (« Ha », « L ») count here, where
/// a header's three letters are needed to name a camera.
fn names_filter(header: &str, item: &KnownEquipment) -> bool {
    let wanted = loose(header);
    if wanted.is_empty() {
        return false;
    }
    let first_word = item.name.split_whitespace().next().map(loose);
    loose(&item.name) == wanted
        || first_word.is_some_and(|word| word == wanted)
        || names_item(header, item)
}

fn match_equipment(item: &mut EquipmentProposal, user: &UserData) {
    let of_kind = |kind: &'static str| user.equipment.iter().filter(move |e| e.kind == kind);
    let found = |e: &KnownEquipment, by: &str| EquipmentMatch {
        equipment_id: e.id.clone(),
        name: e.name.clone(),
        by: by.to_string(),
    };
    if let Some(camera) = &item.instrume {
        item.camera = of_kind("camera")
            .find(|e| names_item(camera, e))
            .map(|e| found(e, "name"));
    }
    if let Some(telescop) = &item.telescop {
        item.mount = of_kind("mount")
            .find(|e| names_item(telescop, e))
            .map(|e| found(e, "name"));
        item.telescope = of_kind("telescope")
            .find(|e| names_item(telescop, e))
            .map(|e| found(e, "name"));
    }
    // The filters the frames name, among the filters of "My equipment".
    item.filter_matches.clear();
    for name in &item.filters {
        if let Some(filter) = of_kind("filter").find(|e| names_filter(name, e))
            && !item
                .filter_matches
                .iter()
                .any(|m| m.equipment_id == filter.id)
        {
            item.filter_matches.push(found(filter, "name"));
        }
    }
    if item.telescope.is_none()
        && let Some(focal) = item.focal_length_mm
    {
        item.telescope = of_kind("telescope")
            .find(|e| {
                e.focal_length_mm
                    .is_some_and(|f| (f - focal).abs() <= 0.03 * focal)
            })
            .map(|e| found(e, "focal-length"));
    }
}

fn holds_target(slot_id: Option<&str>, slot_name: Option<&str>, target: &DetectedTarget) -> bool {
    let by_id = slot_id.is_some() && slot_id == target.target_id.as_deref();
    let by_name = slot_name.is_some_and(|n| {
        loose(n) == loose(&target.designation)
            || target.objects.iter().any(|o| loose(o) == loose(n))
    });
    by_id || by_name
}

fn overlaps(session: &KnownSession, target: &DetectedTarget) -> bool {
    let (Some(start), Some(end)) = (
        parse_iso8601_utc_to_unix_seconds(&target.start),
        parse_iso8601_utc_to_unix_seconds(&target.end),
    ) else {
        return false;
    };
    session.targets.iter().any(|slot| {
        match (
            slot.start
                .as_deref()
                .and_then(parse_iso8601_utc_to_unix_seconds),
            slot.end
                .as_deref()
                .and_then(parse_iso8601_utc_to_unix_seconds),
        ) {
            (Some(a), Some(b)) => a < end && start < b,
            _ => false,
        }
    })
}

fn plan_holds(plan: &KnownPlan, target: &DetectedTarget) -> bool {
    plan.target_ids
        .iter()
        .any(|id| Some(id.as_str()) == target.target_id.as_deref())
        || plan
            .designations
            .iter()
            .any(|d| holds_target(None, Some(d), target))
}

/// What a detected session meets: an existing session of that night (at the
/// same place when both are known), else a plan of that night.
fn match_session(session: &mut DetectedSession, user: &UserData, place: Option<&str>) {
    let existing = user.sessions.iter().find(|s| {
        s.evening_date == session.evening_date
            && (s.place_id.is_none() || place.is_none() || s.place_id.as_deref() == place)
    });
    if let Some(existing) = existing {
        session.existing = SessionMatch::Existing {
            session_id: existing.id.clone(),
        };
        for target in &mut session.targets {
            let inside = existing.targets.iter().any(|slot| {
                holds_target(
                    slot.target_id.as_deref(),
                    slot.designation.as_deref(),
                    target,
                )
            });
            // The filter the session declares is the one of its frames; a
            // target without any is not given one (nothing is supposed).
            if target.filters.is_empty() {
                let declared = existing
                    .targets
                    .iter()
                    .find(|slot| {
                        holds_target(
                            slot.target_id.as_deref(),
                            slot.designation.as_deref(),
                            target,
                        )
                    })
                    .and_then(|slot| slot.filter.clone());
                target.filters.extend(declared);
            }
            target.existing = if inside {
                TargetMatch::InSession
            } else if overlaps(existing, target) {
                TargetMatch::Overlap
            } else {
                TargetMatch::AddToSession
            };
        }
        return;
    }
    let plans: Vec<&KnownPlan> = user
        .plans
        .iter()
        .filter(|p| p.evening_date == session.evening_date)
        .collect();
    if let Some(plan) = plans.iter().find(|p| p.is_pending()) {
        let shared = session.targets.iter().any(|t| plan_holds(plan, t));
        session.existing = if shared {
            SessionMatch::ReusePlan {
                plan_id: plan.id.clone(),
                plan_name: plan.name.clone(),
            }
        } else {
            SessionMatch::ReplacePlan {
                plan_id: plan.id.clone(),
                plan_name: plan.name.clone(),
            }
        };
        return;
    }
    if !user.account.several_plans_per_night
        && let Some(plan) = plans.first()
    {
        session.existing = SessionMatch::PlanExists {
            plan_id: plan.id.clone(),
            plan_name: plan.name.clone(),
        };
    }
}

fn target_words(session: &DetectedSession) -> String {
    session
        .targets
        .iter()
        .map(|t| t.designation.clone())
        .collect::<Vec<_>>()
        .join(", ")
}

fn kind_word(kind: FrameType) -> &'static str {
    match kind {
        FrameType::Light => "lights",
        FrameType::Dark => "darks",
        FrameType::Flat => "flats",
        FrameType::DarkFlat => "dark-flats",
        FrameType::Bias => "bias",
    }
}

fn seconds_text(seconds: f64) -> String {
    if seconds >= 1.0 {
        let rounded = (seconds * 10.0).round() / 10.0;
        if rounded.fract() == 0.0 {
            format!("{rounded:.0} s")
        } else {
            format!("{rounded:.1} s")
        }
    } else {
        format!("{:.0} ms", seconds * 1000.0)
    }
}

/// The lines, in the order of the review screen: places, equipment, nights
/// (each with its targets, calibration, logs and clock), multi-night plans.
#[allow(clippy::too_many_lines)]
fn lines_of(proposal: &Proposal) -> Vec<ProposalLine> {
    let mut lines = Vec::new();
    for place in &proposal.places {
        let chosen = place
            .candidates
            .iter()
            .find(|c| Some(&c.place_id) == place.chosen.as_ref());
        let (checked, reason, text) = match (place.action, chosen) {
            (PlaceAction::Use, Some(c)) => (
                true,
                "place.known",
                format!("Place: {} ({} km away)", c.name, format_km(c.distance_km)),
            ),
            (PlaceAction::UseNearest, Some(c)) => (
                true,
                "place.nearest",
                format!(
                    "Place: {} ({} km away), the nearest of {} places within 10 km",
                    c.name,
                    format_km(c.distance_km),
                    place.candidates.len()
                ),
            ),
            (PlaceAction::Choose, _) => (
                false,
                "place.choose",
                "Place: the frames name none. Choose a place before creating.".to_string(),
            ),
            _ => (
                true,
                "place.create",
                format!(
                    "Unknown place ({:.4}°, {:.4}°): create this place, or choose one",
                    place.latitude.unwrap_or_default(),
                    place.longitude.unwrap_or_default()
                ),
            ),
        };
        let mut row = line(LineKind::Place, &place.id, checked, reason, text);
        row.ask = place.action == PlaceAction::Choose;
        lines.push(row);
    }
    for item in &proposal.equipment {
        let header = [
            item.telescop.clone(),
            item.instrume.clone(),
            item.focal_length_mm.map(|f| format!("{f:.0} mm")),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
        let known: Vec<String> = [&item.telescope, &item.camera, &item.mount]
            .into_iter()
            .flatten()
            .map(|m| m.name.clone())
            .collect();
        let row = if known.is_empty() {
            line(
                LineKind::Equipment,
                &item.id,
                false,
                "equipment.choose",
                format!("Equipment from the headers: {header}. Choose it in “My equipment”."),
            )
        } else {
            line(
                LineKind::Equipment,
                &item.id,
                true,
                "equipment.known",
                format!("Equipment: {} (headers: {header})", known.join(", ")),
            )
        };
        lines.push(row);
    }
    for session in &proposal.sessions {
        let frames: usize = session.targets.iter().map(|t| t.frames).sum();
        let seconds: f64 = session.targets.iter().map(|t| t.exposure_seconds).sum();
        let what = format!(
            "Night of {}: {} ({}, {})",
            session.evening_date,
            target_words(session),
            plural(frames, "frame", "frames"),
            duration_text(seconds)
        );
        let row = match &session.existing {
            SessionMatch::Create => line(
                LineKind::Session,
                &session.id,
                true,
                "session.create",
                format!("{what}: “Executed” session and its one-night plan"),
            ),
            SessionMatch::Existing { .. } => line(
                LineKind::Session,
                &session.id,
                true,
                "session.existing",
                format!("{what}: the session already exists, the frames are attached to it"),
            ),
            SessionMatch::ReusePlan { plan_name, .. } => line(
                LineKind::Session,
                &session.id,
                true,
                "plan.reuse",
                format!(
                    "{what}: the planned “{plan_name}” is reused and will be set to “Executed”"
                ),
            ),
            SessionMatch::ReplacePlan { plan_name, .. } => line(
                LineKind::Session,
                &session.id,
                true,
                "plan.replace",
                format!(
                    "{what}: replace “{plan_name}” with {}",
                    target_words(session)
                ),
            ),
            SessionMatch::PlanExists { plan_name, .. } => {
                let mut row = line(
                    LineKind::Session,
                    &session.id,
                    false,
                    "plan.freeRuleExisting",
                    format!(
                        "{what}: without Advanced Planning or a subscription, a plan exists on this date (“{plan_name}”): none will be created there"
                    ),
                );
                row.locked = true;
                row
            }
        };
        lines.push(row);
        for target in &session.targets {
            let subject = format!("{}/{}", session.id, target.key);
            let found = match &target.source {
                TargetSource::Object => "OBJECT".to_string(),
                TargetSource::Coordinates { distance_degrees } => {
                    format!("coordinates, {distance_degrees:.2}° away")
                }
                TargetSource::Folder { folder } => format!("folder name “{folder}”"),
                TargetSource::PlateSolved { .. } => "plate solving of a frame".to_string(),
                TargetSource::UnknownObject => "OBJECT, not in the catalogue".to_string(),
                TargetSource::ToSolve => "nothing".to_string(),
            };
            let detail = format!(
                "{}: {}, {} ({found})",
                target.designation,
                plural(target.frames, "frame", "frames"),
                duration_text(target.exposure_seconds)
            );
            let mut row = match (&target.source, &target.existing) {
                (TargetSource::ToSolve, _) => line(
                    LineKind::Target,
                    &subject,
                    false,
                    "target.toSolve",
                    format!(
                        "{}, no known target: plate solving to do on a frame ({})",
                        plural(target.frames, "frame", "frames"),
                        target.solve_frame.clone().unwrap_or_default()
                    ),
                ),
                (_, TargetMatch::InSession) => line(
                    LineKind::Target,
                    &subject,
                    true,
                    "target.inSession",
                    format!("{detail}: already in the session, frames attached"),
                ),
                (_, TargetMatch::AddToSession) => line(
                    LineKind::Target,
                    &subject,
                    true,
                    "target.addToSession",
                    format!("Add {} to the session ({detail})", target.designation),
                ),
                (_, TargetMatch::Overlap) => {
                    let mut row = line(
                        LineKind::Target,
                        &subject,
                        false,
                        "target.overlap",
                        format!(
                            "{detail}: its time slot overlaps that of another target of the session. Do not add it, or add {} to the session?",
                            target.designation
                        ),
                    );
                    row.ask = true;
                    row
                }
                (TargetSource::UnknownObject, _) => line(
                    LineKind::Target,
                    &subject,
                    true,
                    "target.createPersonal",
                    format!("{detail}: create “{}” as a new target", target.designation),
                ),
                _ => line(LineKind::Target, &subject, true, "target.detected", detail),
            };
            if matches!(session.existing, SessionMatch::PlanExists { .. }) {
                row.checked = false;
                row.locked = true;
            }
            lines.push(row);
        }
        if let Some(clock) = &session.clock {
            if let Some(shift) = clock.proposed_shift_hours {
                lines.push(line(
                    LineKind::Clock,
                    &session.id,
                    true,
                    "clock.shift",
                    format!(
                        "Suspect clock ({}): shift the times by {}{} h, applied to the session only, never to the files",
                        clock
                            .reasons
                            .iter()
                            .map(|r| match r.code {
                                super::ClockReasonCode::SunUp => "Sun up at the written time",
                                super::ClockReasonCode::TargetBelowHorizon => "target below the horizon",
                                super::ClockReasonCode::AltitudeDisagrees => "header altitude differs",
                                super::ClockReasonCode::LstDisagrees => "header sidereal time differs",
                            })
                            .collect::<Vec<_>>()
                            .join(", "),
                        if shift > 0 { "+" } else { "−" },
                        shift.abs()
                    ),
                ));
            } else if clock.suspect {
                lines.push(line(
                    LineKind::Clock,
                    &session.id,
                    false,
                    "clock.suspect",
                    "Suspect clock: no shift by whole hours makes the night consistent".to_string(),
                ));
            } else if clock.place == super::PlaceUsed::None {
                lines.push(line(
                    LineKind::Clock,
                    &session.id,
                    false,
                    "clock.needsPlace",
                    "Clock: checked once the place is chosen".to_string(),
                ));
            }
        }
        for attached in &session.calibration {
            let Some(set) = proposal
                .calibration_sets
                .iter()
                .find(|s| s.id == attached.set_id)
            else {
                continue;
            };
            let mut parts = vec![format!("{} {}", set.frames, kind_word(attached.kind))];
            if let Some(e) = set.exposure_seconds {
                parts.push(seconds_text(e));
            }
            if let Some(g) = set.gain {
                parts.push(format!("gain {g:.0}"));
            }
            if let Some(t) = set.temperature_c {
                parts.push(format!("{t:.0} °C").replace('-', "−"));
            }
            if let Some(f) = &set.filter {
                parts.push(f.clone());
            }
            if let Some(start) = &set.start {
                parts.push(start[..10].to_string());
            }
            lines.push(line(
                LineKind::Calibration,
                &format!("{}/{}", session.id, set.id),
                true,
                "calibration.attached",
                format!("Calibration: {}", parts.join(" · ")),
            ));
        }
        for attached in &session.logs {
            let Some(log) = proposal.logs.iter().find(|l| l.id == attached.log_id) else {
                continue;
            };
            let name = log.path.rsplit(['/', '\\']).next().unwrap_or_default();
            let targets: Vec<String> = attached
                .targets
                .iter()
                .filter_map(|key| session.targets.iter().find(|t| &t.key == key))
                .map(|t| t.designation.clone())
                .collect();
            lines.push(line(
                LineKind::Log,
                &format!("{}/{}", session.id, log.id),
                true,
                "log.attached",
                format!(
                    "Log {}: {} ({}, time {})",
                    log.kind.to_uppercase(),
                    name,
                    targets.join(", "),
                    log.utc_offset.clone().unwrap_or_default()
                ),
            ));
        }
    }
    for group in &proposal.multi_night {
        let nights = group
            .nights
            .iter()
            .map(|n| n.evening_date.clone())
            .collect::<Vec<_>>()
            .join(", ");
        let text = format!(
            "Multi-night plan {}: {} ({nights}), goal = total acquired {}, created as “Completed”",
            group.designation,
            plural(group.nights.len(), "night", "nights"),
            duration_text(group.goal_seconds)
        );
        let row = if group.locked {
            let mut row = line(
                LineKind::MultiNight,
                &group.id,
                false,
                "multiNight.locked",
                format!("{text}. Unlock the feature: Advanced Planning"),
            );
            row.locked = true;
            row
        } else {
            line(
                LineKind::MultiNight,
                &group.id,
                true,
                "multiNight.create",
                text,
            )
        };
        lines.push(row);
    }
    lines
}

fn format_km(km: f64) -> String {
    format!("{km:.1}")
}

/// The remarks about the account's rights (`account.*`), above the nights.
fn account_notes(proposal: &mut Proposal) {
    proposal
        .notes
        .retain(|note| !note.key.starts_with("account."));
    let account = proposal.account;
    if !account.advanced_planning && !account.several_plans_per_night {
        proposal.notes.push(ProposalNote {
            key: "account.free".to_string(),
            text: "Without a subscription or Advanced Planning: one plan per night, and multi-night plans stay locked. See the offers on the web.".to_string(),
        });
    } else if !account.advanced_planning && !proposal.multi_night.is_empty() {
        proposal.notes.push(ProposalNote {
            key: "account.multiNightLocked".to_string(),
            text: "Multi-night plans need Advanced Planning: they are shown locked.".to_string(),
        });
    }
}

/// What [`Proposal::refresh_rights`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RightsRefresh {
    /// The proposal was computed for these rights already.
    Unchanged,
    /// The multi-night plans were unlocked or locked and the lines made again;
    /// `unlocked` are the ids of the multi-night lines that are now open and
    /// ticked by default (the person never had the choice of them).
    Updated { unlocked: Vec<String> },
    /// The rights changed in a way the proposal cannot follow without the
    /// person's plans: detect the sessions again.
    Stale,
}

/// The proposal followed to the rights the account holds now, without reading
/// the folders again (see [`Proposal::refresh_rights`]).
pub(super) fn refresh_rights(proposal: &mut Proposal, account: AccountRights) -> RightsRefresh {
    if proposal.account == account {
        return RightsRefresh::Unchanged;
    }
    // "One plan per night" decided the sessions that meet a plan of their
    // date: that match needs the person's plans, read again by a new
    // detection.
    if proposal.account.several_plans_per_night != account.several_plans_per_night
        && proposal
            .sessions
            .iter()
            .any(|s| matches!(s.existing, SessionMatch::PlanExists { .. }))
    {
        return RightsRefresh::Stale;
    }
    let before: Vec<bool> = proposal.multi_night.iter().map(|g| g.locked).collect();
    proposal.account = account;
    for group in &mut proposal.multi_night {
        group.locked = !account.advanced_planning;
    }
    account_notes(proposal);
    proposal.lines = lines_of(proposal);
    let unlocked = proposal
        .multi_night
        .iter()
        .zip(before)
        .filter(|(group, was_locked)| *was_locked && !group.locked)
        .map(|(group, _)| format!("multiNight.create:{}", group.id))
        .collect();
    RightsRefresh::Updated { unlocked }
}

/// Step 3: the grouping matched to the person's data, and the lines.
#[must_use]
pub fn propose(grouping: &Grouping, user: &UserData) -> Proposal {
    let mut proposal = grouping.proposal.clone();
    for place in &mut proposal.places {
        match_places(place, user);
    }
    for item in &mut proposal.equipment {
        match_equipment(item, user);
    }
    let chosen_place = |id: &str| {
        proposal
            .places
            .iter()
            .find(|p| p.id == id)
            .and_then(|p| p.chosen.clone())
    };
    let places: Vec<Option<String>> = proposal
        .sessions
        .iter()
        .map(|s| chosen_place(&s.place))
        .collect();
    for (session, place) in proposal.sessions.iter_mut().zip(places) {
        match_session(session, user, place.as_deref());
        session.filter_ask = session.targets.iter().any(|t| t.filters.is_empty());
        // A guess is only worth showing where the review asks.
        if !session.filter_ask {
            session.filter_guess = None;
        }
    }
    proposal.account = user.account;
    for group in &mut proposal.multi_night {
        group.locked = !user.account.advanced_planning;
    }
    account_notes(&mut proposal);
    proposal.lines = lines_of(&proposal);
    proposal
}
