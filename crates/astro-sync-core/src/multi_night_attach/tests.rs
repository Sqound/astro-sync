use super::*;
use crate::multi_night::tests::{FakeService, capabilities_with};
use std::collections::BTreeMap;

/// Advanced Planning alone (no Advanced Sync+) attaches.
fn open() -> AttachGate {
    AttachGate::new(Some(&capabilities_with(false, true)), true)
}

fn project(id: &str, designation: &str, common: Option<&str>, target: &str) -> Value {
    let mut target = json!({ "targetId": target, "name": designation });
    if let Some(common) = common {
        target["commonName"] = json!(common);
    }
    json!({ "id": id, "revision": 1, "status": "active", "target": target, "goalMinutes": 720 })
}

fn nights(project: &str, own: &[&str], candidates: &[(&str, Option<&str>)]) -> Value {
    json!({
        "projectId": project,
        "nights": own.iter().map(|s| json!({
            "sessionId": s, "planId": null, "link": "night", "integrationSeconds": 3600.0
        })).collect::<Vec<_>>(),
        "candidates": candidates.iter().map(|(s, p)| json!({
            "sessionId": s, "planId": null, "link": null, "projectId": p, "integrationSeconds": 0.0
        })).collect::<Vec<_>>(),
    })
}

/// The space: IC 1805 (the night n-10 is a candidate), IC 1805 again with the
/// night in it, M31 (another target: the night is in neither list) and an
/// archived plan.
fn space() -> FakeService {
    FakeService {
        projects: BTreeMap::from([
            (
                "p-heart".to_string(),
                project("p-heart", "IC 1805", Some("Nébuleuse du Cœur"), "ic-1805"),
            ),
            (
                "p-soul".to_string(),
                project("p-soul", "IC 1848", Some("Nébuleuse de l’Âme"), "ic-1848"),
            ),
            ("p-m31".to_string(), project("p-m31", "M31", None, "m-31")),
            (
                "p-old".to_string(),
                json!({ "id": "p-old", "status": "archived", "target": { "targetId": "ic-1805", "name": "IC 1805" } }),
            ),
        ]),
        nights: BTreeMap::from([
            (
                "p-heart".to_string(),
                nights("p-heart", &["n-09"], &[("n-10", None)]),
            ),
            (
                "p-soul".to_string(),
                nights("p-soul", &["n-12"], &[("n-10", Some("p-heart"))]),
            ),
            ("p-m31".to_string(), nights("p-m31", &["n-30"], &[])),
            ("p-old".to_string(), nights("p-old", &[], &[("n-10", None)])),
        ]),
        ..FakeService::default()
    }
}

#[test]
fn the_plans_proposed_are_those_of_the_nights_target() {
    let service = space();
    let choices = proposed_plans(&service, "u", "t", "n-10", true).unwrap();
    // The archived plan and the plan of another target are not proposed.
    let ids: Vec<&str> = choices
        .plans
        .iter()
        .map(|c| c.project_id.as_str())
        .collect();
    assert_eq!(ids, vec!["p-heart", "p-soul"]);
    assert_eq!(choices.total, 3);
    assert!(!choices.no_target);
    let heart = &choices.plans[0];
    assert_eq!(heart.state, ChoiceState::Available);
    assert_eq!((heart.night_count, heart.total_seconds), (1, 3600.0));
    assert_eq!(heart.goal_minutes, Some(720));
    assert_eq!(heart.common_name.as_deref(), Some("Nébuleuse du Cœur"));
    // IC 1848 would take the night from IC 1805, whose name is said.
    let soul = &choices.plans[1];
    assert_eq!(soul.state, ChoiceState::Moves);
    assert_eq!(soul.from_project_id.as_deref(), Some("p-heart"));
    assert_eq!(soul.from_name.as_deref(), Some("IC 1805"));
}

#[test]
fn the_current_plan_comes_first_and_is_not_offered_again() {
    let mut service = space();
    service.nights.insert(
        "p-soul".to_string(),
        nights("p-soul", &["n-10", "n-12"], &[]),
    );
    service.nights.insert(
        "p-heart".to_string(),
        nights("p-heart", &["n-09"], &[("n-10", Some("p-soul"))]),
    );
    let choices = proposed_plans(&service, "u", "t", "n-10", true).unwrap();
    let states: Vec<(&str, ChoiceState)> = choices
        .plans
        .iter()
        .map(|c| (c.project_id.as_str(), c.state))
        .collect();
    assert_eq!(
        states,
        vec![
            ("p-soul", ChoiceState::Current),
            ("p-heart", ChoiceState::Moves)
        ]
    );
}

#[test]
fn a_night_with_no_target_is_offered_every_plan_and_the_service_judges() {
    let service = space();
    // n-99 is in no list: every plan reads "other target" for it.
    let choices = proposed_plans(&service, "u", "t", "n-99", false).unwrap();
    assert!(choices.no_target);
    assert_eq!(choices.plans.len(), 3);
    assert!(
        choices
            .plans
            .iter()
            .all(|c| c.state == ChoiceState::Available)
    );
    // With a target recorded and no plan of it: nothing is proposed, and the
    // window says so (the search stays).
    let none = proposed_plans(&service, "u", "t", "n-99", true).unwrap();
    assert!(none.plans.is_empty() && !none.no_target);
    // A space without plan: nothing, not even a flag.
    let empty = proposed_plans(&FakeService::default(), "u", "t", "n-10", false).unwrap();
    assert!(empty.plans.is_empty() && !empty.no_target && empty.total == 0);
}

#[test]
fn the_search_finds_a_plan_by_the_name_of_its_target_whatever_the_accents() {
    assert_eq!(fold("Nébuleuse du Cœur"), "nebuleuseducoeur");
    assert_eq!(fold("NGC 7000"), fold("ngc7000"));
    let service = space();
    let found = |query: &str| -> Vec<String> {
        search_plans(&service, "u", "t", Some("n-10"), query)
            .unwrap()
            .into_iter()
            .map(|c| c.project_id)
            .collect()
    };
    assert_eq!(found("coeur"), vec!["p-heart"]);
    assert_eq!(found("ic 18"), vec!["p-heart", "p-soul"]);
    assert_eq!(found("AME"), vec!["p-soul"]);
    // A plan of another target is found too, said so (the night is not in
    // its lists); an archived plan is not.
    let m31 = search_plans(&service, "u", "t", Some("n-10"), "m31").unwrap();
    assert_eq!(m31.len(), 1);
    assert_eq!(m31[0].state, ChoiceState::OtherTarget);
    assert!(found("zzz").is_empty());
    // An empty query finds nothing, and asks nothing of the service.
    assert!(found("  ").is_empty());
    // Without a night (the command line) every plan is simply available.
    let bare = search_plans(&service, "u", "t", None, "m31").unwrap();
    assert_eq!(bare[0].state, ChoiceState::Available);
}

#[test]
fn attaching_asks_the_service_only_with_the_right_and_a_refusal_is_said_in_clear() {
    // Without the right, or before the service was heard: nothing is asked.
    let service = FakeService::default();
    for (gate, closed) in [
        (
            // Advanced Sync+ alone does not attach.
            AttachGate::new(Some(&capabilities_with(true, false)), true),
            MultiNightClosed::NotInSubscription,
        ),
        (AttachGate::unknown(), MultiNightClosed::Unknown),
    ] {
        let attach = attach_night(&service, "u", "t", "p-heart", "n-10", gate).unwrap();
        assert_eq!(
            attach,
            NightPlanChange::Closed {
                project_id: "p-heart".into(),
                closed
            }
        );
        let detach = detach_night(&service, "u", "t", "p-heart", "n-10", gate).unwrap();
        assert!(matches!(detach, NightPlanChange::Closed { .. }));
    }
    assert!(service.writes.borrow().is_empty());

    // With it: one POST of the night, the plan's nights as it now stands.
    let service = FakeService {
        write_answer: Some(Ok(nights("p-heart", &["n-09", "n-10"], &[]))),
        ..FakeService::default()
    };
    let done = attach_night(&service, "u", "t", "p-heart", "n-10", open()).unwrap();
    assert_eq!(
        done,
        NightPlanChange::Attached {
            project_id: "p-heart".into(),
            nights: 2
        }
    );
    let detached = detach_night(&service, "u", "t", "p-heart", "n-10", open()).unwrap();
    assert_eq!(
        detached,
        NightPlanChange::Detached {
            project_id: "p-heart".into(),
            nights: 2
        }
    );
    let writes = service.writes.borrow();
    assert_eq!(writes[0].0, "POST /projects/p-heart/nights");
    assert_eq!(writes[0].1, json!({ "sessionIds": ["n-10"] }));
    assert_eq!(writes[1].0, "DELETE /projects/p-heart/nights/n-10");

    // The service's refusals keep their codes and words.
    for (code, status) in [
        ("multi_night_not_active", 403),
        ("target_mismatch", 409),
        ("session_not_found", 404),
        ("project_not_found", 404),
    ] {
        let service = FakeService {
            write_answer: Some(Err((code.to_string(), format!("words of {code} {status}")))),
            ..FakeService::default()
        };
        for change in [
            attach_night(&service, "u", "t", "p-heart", "n-10", open()).unwrap(),
            detach_night(&service, "u", "t", "p-heart", "n-10", open()).unwrap(),
        ] {
            assert_eq!(
                change,
                NightPlanChange::Refused {
                    project_id: "p-heart".into(),
                    code: code.into(),
                    message: format!("words of {code} {status}"),
                }
            );
        }
    }
}

fn heart_target() -> NewPlanTarget {
    NewPlanTarget {
        target_id: "ic-1805".into(),
        name: "IC 1805".into(),
        common_name: Some("Nébuleuse du Cœur".into()),
    }
}

fn col() -> NewPlanPlace {
    NewPlanPlace {
        name: "Col des Mélèzes".into(),
        latitude: 44.9,
        longitude: 6.4,
    }
}

#[test]
fn a_plan_created_from_a_night_inherits_its_target_and_its_place() {
    let service = FakeService {
        write_answer: Some(Ok(json!({
            "id": "p-new", "target": { "name": "IC 1805" }, "goalMinutes": 600,
            "nights": [{ "sessionId": "n-10", "planId": null, "link": "night" }]
        }))),
        ..FakeService::default()
    };
    let made = create_plan_from_night(
        &service,
        "u",
        "t",
        "n-10",
        &heart_target(),
        Some(&col()),
        open(),
    )
    .unwrap();
    assert_eq!(
        made,
        PlanCreated::Created {
            project_id: "p-new".into(),
            name: "IC 1805".into(),
            goal_minutes: 600,
            nights: 1
        }
    );
    let writes = service.writes.borrow();
    assert_eq!(writes[0].0, "POST /projects");
    let body = &writes[0].1;
    assert_eq!(body["fromSessionIds"], json!(["n-10"]));
    assert_eq!(
        body["target"],
        json!({ "targetId": "ic-1805", "name": "IC 1805", "commonName": "Nébuleuse du Cœur" })
    );
    assert_eq!(body["goalMinutes"], 600);
    assert_eq!(body["place"]["name"], "Col des Mélèzes");
    assert_eq!(body["constraints"]["minAltitude"], 30.0);
    // The plan's identifier is a UUID of version 4, fixed before the request.
    let id = body["id"].as_str().unwrap();
    assert_eq!(id.len(), 36);
    assert_eq!(&id[14..15], "4");
    assert!("89ab".contains(&id[19..20]));
}

#[test]
fn a_plan_is_not_asked_for_without_the_right_the_place_or_the_target() {
    let service = FakeService::default();
    let closed = AttachGate::new(Some(&capabilities_with(true, false)), true);
    assert_eq!(
        create_plan_from_night(
            &service,
            "u",
            "t",
            "n",
            &heart_target(),
            Some(&col()),
            closed
        )
        .unwrap(),
        PlanCreated::Closed {
            closed: MultiNightClosed::NotInSubscription
        }
    );
    assert_eq!(
        create_plan_from_night(&service, "u", "t", "n", &heart_target(), None, open()).unwrap(),
        PlanCreated::Missing {
            missing: NewPlanMissing::Place
        }
    );
    let nameless = NewPlanTarget {
        target_id: String::new(),
        name: "x".into(),
        common_name: None,
    };
    assert_eq!(
        create_plan_from_night(&service, "u", "t", "n", &nameless, Some(&col()), open()).unwrap(),
        PlanCreated::Missing {
            missing: NewPlanMissing::Target
        }
    );
    assert!(service.writes.borrow().is_empty());

    // The service's refusal: the target of another night, the capability.
    for code in [
        "target_mismatch",
        "capability_required",
        "multi_night_not_active",
    ] {
        let service = FakeService {
            write_answer: Some(Err((code.to_string(), "words".to_string()))),
            ..FakeService::default()
        };
        let made = create_plan_from_night(
            &service,
            "u",
            "t",
            "n",
            &heart_target(),
            Some(&col()),
            open(),
        )
        .unwrap();
        assert_eq!(
            made,
            PlanCreated::Refused {
                code: code.into(),
                message: "words".into()
            }
        );
    }
}

#[test]
fn the_place_of_a_night_needs_its_coordinates() {
    let detail = json!({ "night": { "place": { "value": {
        "name": " Col des Mélèzes ", "latitude": 44.9, "longitude": 6.4
    }}}});
    assert_eq!(place_of_detail(&detail), Some(col()));
    for place in [
        json!({ "name": "Col" }),
        json!({ "name": "", "latitude": 1.0, "longitude": 1.0 }),
        json!({ "name": "Col", "latitude": 91.0, "longitude": 1.0 }),
    ] {
        assert_eq!(
            place_of_detail(&json!({ "night": { "place": { "value": place } } })),
            None
        );
    }
    assert_eq!(
        place_of_detail(&json!({ "night": { "place": { "value": null } } })),
        None
    );
}
