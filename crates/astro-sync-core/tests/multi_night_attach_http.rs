//! Attaching a night to a multi-night plan against the real HTTP transport
//! and a fake service: the routes, the verbs, the body, the refusals the
//! service answers in clear.

#[path = "support/fake_service.rs"]
mod fake_service;

use astro_sync_core::gui_bridge::{Capabilities, HttpPortalGuiTransport};
use astro_sync_core::multi_night::{
    AttachGate, IntegrationLine, IntegrationsSent, MultiNightClosed, MultiNightGate,
    night_projects, project_of, send_integrations, service_sells_multi_night,
    service_sells_multi_night_kept,
};
use astro_sync_core::multi_night_attach::{
    NewPlanPlace, NewPlanTarget, NightPlanChange, PlanCreated, attach_night,
    create_plan_from_night, detach_night, proposed_plans, search_plans,
};
use astro_sync_core::service_wait::WakePolicy;
use fake_service::{FakeService, Reply, Wake, error_reply};

const HEART: &str = r#"{"id":"p-heart","revision":1,"status":"active","target":{"targetId":"ic-1805","name":"IC 1805","commonName":"Nébuleuse du Cœur"},"goalMinutes":720}"#;
const NIGHTS: &str = r#"{"projectId":"p-heart","nights":[{"sessionId":"n-09","planId":"plan-09","planName":"Nuit du 9 août","eveningDate":"2026-08-09","sessionStatus":"finished","link":"plan","integrationCount":2,"integrationSeconds":3600}],"candidates":[{"sessionId":"n-10","planId":null,"planName":null,"eveningDate":"2026-08-10","sessionStatus":"finished","link":null,"integrationCount":0,"integrationSeconds":0,"projectId":null}]}"#;

/// Advanced Planning alone (`analysis.progression` off) attaches.
fn open() -> AttachGate {
    let capabilities: Capabilities = serde_json::from_str(
        r#"{"planner.multiple":true,"planner.mosaic":true,"pointing.solve":true,"sync.plan_limit":0,"gallery.private":true,"portal.invitations":true,"capture.variant_limit":12,"capture.count_limit":0,"capture.public":true,"analysis.logs":true,"analysis.progression":false,"storage.max_file_bytes":100,"storage.quota_bytes":1000,"history.retention_days":0}"#,
    )
    .unwrap();
    AttachGate::new(Some(&capabilities), true)
}

#[test]
fn the_window_reads_proposes_attaches_detaches_and_creates_through_the_routes() {
    let service = FakeService::start(Wake::Instant, |request| {
        match (
            request.method.as_str(),
            request.path.split('?').next().unwrap_or_default(),
        ) {
            ("GET", "/api/v1/projects") => Reply::Json(200, format!(r#"{{"projects":[{HEART}]}}"#)),
            ("GET", "/api/v1/projects/p-heart/nights") => Reply::Json(200, NIGHTS.to_string()),
            ("GET", "/api/v1/projects/p-heart") => Reply::Json(200, HEART.to_string()),
            ("POST", "/api/v1/projects/p-heart/nights") => Reply::Json(200, NIGHTS.to_string()),
            ("DELETE", "/api/v1/projects/p-heart/nights/n-10") => {
                Reply::Json(200, NIGHTS.to_string())
            }
            ("DELETE", "/api/v1/projects/p-heart/nights/n-11") => error_reply(
                404,
                "night_not_found",
                "The space holds neither a night nor a plan of that id.",
            ),
            ("POST", "/api/v1/projects/p-other/nights") => error_reply(
                409,
                "target_mismatch",
                "This night is not of the plan's target.",
            ),
            ("POST", "/api/v1/projects") => Reply::Json(
                201,
                r#"{"id":"p-new","target":{"targetId":"ic-1805","name":"IC 1805"},"goalMinutes":600,"nights":[{"sessionId":"n-10"}]}"#.to_string(),
            ),
            _ => error_reply(404, "not_found", "no"),
        }
    });
    let transport = HttpPortalGuiTransport::new().unwrap();
    let base = service.base_url();

    // The window opens: the plan of the target, the night a candidate.
    let choices = proposed_plans(&transport, &base, "tok", "n-10", true).unwrap();
    assert_eq!(choices.plans.len(), 1);
    assert_eq!(choices.plans[0].project_id, "p-heart");
    assert_eq!(choices.plans[0].night_count, 1);
    assert_eq!(choices.plans[0].total_seconds, 3600.0);
    // … and the search finds it by the common name, accents or not.
    let found = search_plans(&transport, &base, "tok", Some("n-10"), "coeur").unwrap();
    assert_eq!(found.len(), 1);

    // One click attaches: the night, by its id, with the device's token.
    let done = attach_night(&transport, &base, "tok", "p-heart", "n-10", open()).unwrap();
    assert_eq!(
        done,
        NightPlanChange::Attached {
            project_id: "p-heart".into(),
            nights: 1
        }
    );
    let post = service
        .requests()
        .into_iter()
        .find(|r| r.method == "POST" && r.path == "/api/v1/projects/p-heart/nights")
        .unwrap();
    assert_eq!(post.json(), serde_json::json!({ "sessionIds": ["n-10"] }));
    assert_eq!(post.header("authorization"), Some("Bearer tok"));

    // "Cancel" detaches; a night that is not there is the service's
    // `night_not_found`, said in clear.
    let undone = detach_night(&transport, &base, "tok", "p-heart", "n-10", open()).unwrap();
    assert!(matches!(undone, NightPlanChange::Detached { .. }));
    assert_eq!(
        service.app_hits("DELETE", "/api/v1/projects/p-heart/nights/n-10"),
        1
    );
    let missing = detach_night(&transport, &base, "tok", "p-heart", "n-11", open()).unwrap();
    assert_eq!(
        missing,
        NightPlanChange::Refused {
            project_id: "p-heart".into(),
            code: "night_not_found".into(),
            message: "The space holds neither a night nor a plan of that id.".into()
        }
    );

    // The service refuses a plan of another target.
    let refused = attach_night(&transport, &base, "tok", "p-other", "n-10", open()).unwrap();
    assert!(matches!(
        refused,
        NightPlanChange::Refused { ref code, .. } if code == "target_mismatch"
    ));

    // A plan created for the night: one POST, the night in `fromSessionIds`.
    let target = NewPlanTarget {
        target_id: "ic-1805".into(),
        name: "IC 1805".into(),
        common_name: None,
    };
    let place = NewPlanPlace {
        name: "Col des Mélèzes".into(),
        latitude: 44.9,
        longitude: 6.4,
    };
    let created = create_plan_from_night(
        &transport,
        &base,
        "tok",
        "n-10",
        &target,
        Some(&place),
        open(),
    )
    .unwrap();
    assert_eq!(
        created,
        PlanCreated::Created {
            project_id: "p-new".into(),
            name: "IC 1805".into(),
            goal_minutes: 600,
            nights: 1
        }
    );
    let creations = service
        .requests()
        .into_iter()
        .filter(|r| r.method == "POST" && r.path == "/api/v1/projects")
        .collect::<Vec<_>>();
    assert_eq!(creations.len(), 1);
    assert_eq!(
        creations[0].json()["fromSessionIds"],
        serde_json::json!(["n-10"])
    );

    // Without the right nothing more reaches the service.
    let before = service.requests().len();
    let closed = AttachGate::unknown();
    assert_eq!(
        attach_night(&transport, &base, "tok", "p-heart", "n-10", closed).unwrap(),
        NightPlanChange::Closed {
            project_id: "p-heart".into(),
            closed: MultiNightClosed::Unknown
        }
    );
    assert_eq!(service.requests().len(), before);
}

#[test]
fn a_night_attached_by_itself_is_read_through_the_session_filter() {
    let service = FakeService::start(Wake::Instant, |request| {
        match (
            request.method.as_str(),
            request.path.split('?').next().unwrap_or_default(),
        ) {
            ("GET", "/api/v1/projects") => Reply::Json(200, format!(r#"{{"projects":[{HEART}]}}"#)),
            ("GET", "/api/v1/projects/p-heart") => Reply::Json(200, HEART.to_string()),
            _ => error_reply(404, "not_found", "no"),
        }
    });
    let transport = HttpPortalGuiTransport::new().unwrap();
    let night = night_projects(&transport, &service.base_url(), "tok", None, Some("n-10")).unwrap();
    assert_eq!(night.projects.len(), 1);
    assert_eq!(night.projects[0].project_id, "p-heart");
    assert_eq!(night.projects[0].target_id, "ic-1805");
    // The filter is the query `sessionId=` of the list route.
    let list = service
        .requests()
        .into_iter()
        .find(|r| r.path.starts_with("/api/v1/projects?") && r.method == "GET")
        .unwrap();
    assert!(list.path.contains("sessionId=n-10"), "{}", list.path);
}

/// Astro Sync asks the service whether it sells the
/// multi-night plans (`GET /v1/service`) instead of guessing from its
/// address: a service that creates a space for any address sells them; a
/// private server (pairing, its owner's address alone, an announcement
/// older than the field) never restricts them. The answer is kept for when
/// the service cannot be reached.
#[test]
fn whether_a_service_sells_the_multi_night_plans_is_asked_of_it_and_kept() {
    for (sign_in, sells) in [
        (
            r#","signIn":{"method":"email-code","spaceCreation":"open"}"#,
            true,
        ),
        (
            r#","signIn":{"method":"email-code","spaceCreation":"closed"}"#,
            false,
        ),
        (r#","signIn":{"method":"pairing"}"#, false),
        ("", false),
    ] {
        let body =
            format!(r#"{{"service":"astro-portal","apiVersion":1,"version":"1.0.0"{sign_in}}}"#);
        let service =
            FakeService::start(Wake::Instant, move |request| match request.path.as_str() {
                "/api/v1/service" => Reply::Json(200, body.clone()),
                _ => error_reply(404, "not_found", "no"),
            });
        assert_eq!(
            service_sells_multi_night(&service.base_url(), WakePolicy::background_refresh())
                .unwrap(),
            sells,
            "{sign_in}"
        );
    }

    // Kept: the answer heard is written, and read again while the service
    // cannot be reached; another address was never heard.
    let folder = std::env::temp_dir().join(format!(
        "astro-sync-sells-multi-night-{}",
        std::process::id()
    ));
    let record = folder.join("service-sells-multi-night.json");
    let service = FakeService::start(Wake::Instant, |request| {
        match request.path.as_str() {
        "/api/v1/service" => Reply::Json(
            200,
            r#"{"service":"astro-portal","apiVersion":1,"version":"1.0.0","signIn":{"method":"pairing"}}"#
                .to_string(),
        ),
        _ => error_reply(404, "not_found", "no"),
    }
    });
    let base = service.base_url();
    assert_eq!(
        service_sells_multi_night_kept(&base, WakePolicy::background_refresh(), &record),
        Some(false)
    );
    drop(service);
    assert_eq!(
        service_sells_multi_night_kept(&base, WakePolicy::background_refresh(), &record),
        Some(false)
    );
    assert_eq!(
        service_sells_multi_night_kept(
            "http://127.0.0.1:9/",
            WakePolicy::background_refresh(),
            &record
        ),
        None
    );
    let _ = std::fs::remove_dir_all(&folder);
}

/// Synchronisation avancée+ on (`analysis.progression`).
fn sending() -> MultiNightGate {
    let capabilities: Capabilities = serde_json::from_str(
        r#"{"planner.multiple":true,"planner.mosaic":true,"pointing.solve":true,"sync.plan_limit":0,"gallery.private":true,"portal.invitations":true,"capture.variant_limit":12,"capture.count_limit":0,"capture.public":true,"analysis.logs":true,"analysis.progression":true,"storage.max_file_bytes":100,"storage.quota_bytes":1000,"history.retention_days":0}"#,
    )
    .unwrap();
    MultiNightGate::new(Some(&capabilities), true)
}

fn night_line(filter: &str) -> IntegrationLine {
    IntegrationLine {
        night_id: "n-12".into(),
        target_id: "ic-1805".into(),
        filter: Some(filter.into()),
        exposure_seconds: 180.0,
        gain: Some(100),
        binning: Some(1),
        temperature_c: Some(-10.0),
        frame_count: 100,
        evening_date: "2026-08-12".into(),
        sensor: None,
        computer_label: None,
        installation_id: None,
    }
}

#[test]
fn the_lines_of_a_night_go_complete_through_the_route_and_an_older_service_gets_the_earlier_form() {
    let project = project_of(&serde_json::from_str(HEART).unwrap(), "ic-1805").unwrap();
    let transport = HttpPortalGuiTransport::new().unwrap();
    let path = "/api/v1/projects/p-heart/integrations";

    // A service that knows `replace`: one `PUT`, the set named.
    let current = FakeService::start(Wake::Instant, |request| {
        if request.method == "PUT" && request.path == "/api/v1/projects/p-heart/integrations" {
            Reply::Json(200, r#"{"integrations":[]}"#.to_string())
        } else {
            error_reply(404, "not_found", "no")
        }
    });
    let sent = send_integrations(
        &transport,
        &current.base_url(),
        "tok",
        &project,
        &[night_line("Broadband")],
        sending(),
    )
    .unwrap();
    assert!(matches!(sent, IntegrationsSent::Sent { lines: 1, .. }));
    let puts: Vec<_> = current
        .requests()
        .into_iter()
        .filter(|r| r.method == "PUT")
        .collect();
    assert_eq!(puts.len(), 1);
    assert_eq!(puts[0].path, path);
    assert_eq!(puts[0].header("authorization"), Some("Bearer tok"));
    assert_eq!(
        puts[0].json()["replace"],
        serde_json::json!([{ "nightId": "n-12", "targetId": "ic-1805" }])
    );
    assert_eq!(puts[0].json()["integrations"][0]["filter"], "Broadband");

    // A service that predates it refuses the property: the lines go again
    // without it, and the night is sent all the same.
    let older = FakeService::start(Wake::Instant, |request| {
        if request.method != "PUT" {
            return error_reply(404, "not_found", "no");
        }
        if request.json().get("replace").is_some() {
            error_reply(400, "invalid_request", "Unexpected property replace.")
        } else {
            Reply::Json(200, r#"{"integrations":[]}"#.to_string())
        }
    });
    let sent = send_integrations(
        &transport,
        &older.base_url(),
        "tok",
        &project,
        &[night_line("Broadband")],
        sending(),
    )
    .unwrap();
    assert!(matches!(sent, IntegrationsSent::Sent { lines: 1, .. }));
    let puts: Vec<_> = older
        .requests()
        .into_iter()
        .filter(|r| r.method == "PUT")
        .collect();
    assert_eq!(puts.len(), 2);
    assert!(puts[0].json().get("replace").is_some());
    assert!(puts[1].json().get("replace").is_none());
    assert_eq!(puts[1].json()["integrations"][0]["filter"], "Broadband");
}
