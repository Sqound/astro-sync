use super::*;
use crate::test_support::{TempDir, fits_bytes};
use std::cell::RefCell;

/// A master kept, as the script writes it.
pub(crate) fn write_master(
    folder: &Path,
    family: &str,
    filter: Option<&str>,
    exposure: f64,
    frames: u64,
) -> PathBuf {
    write_master_with(folder, family, filter, exposure, frames, &[])
}

/// [`write_master`] with more cards (`DATE-OBS`, the first frame's).
pub(crate) fn write_master_with(
    folder: &Path,
    family: &str,
    filter: Option<&str>,
    exposure: f64,
    frames: u64,
    extra: &[(&str, &str)],
) -> PathBuf {
    fs::create_dir_all(folder).unwrap();
    let path = folder.join(format!("{family}{INTEGRATION_SUFFIX}"));
    let mut cards: Vec<(String, String)> = vec![
        ("NAXIS".into(), "3".into()),
        ("ASFAMILY".into(), format!("'{family}'")),
        ("EXPTIME".into(), format!("{exposure:.1}")),
        ("GAIN".into(), "100".into()),
        ("XBINNING".into(), "1".into()),
        ("CCD-TEMP".into(), "-10.0".into()),
        ("INSTRUME".into(), "'ZWO ASI2600MC Pro'".into()),
        ("STACKCNT".into(), frames.to_string()),
        (
            "LIVETIME".into(),
            format!("{:.1}", exposure * frames as f64),
        ),
    ];
    if let Some(filter) = filter {
        cards.push(("FILTER".into(), format!("'{filter}'")));
    }
    cards.extend(
        extra
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string())),
    );
    let cards: Vec<(&str, &str)> = cards
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    fs::write(&path, fits_bytes(&cards, 32)).unwrap();
    path
}

/// The image a Seestar stacked, as its header says it (no pixel).
pub(crate) fn write_device_image(
    folder: &Path,
    poses: u64,
    filter: &str,
    exposure: f64,
) -> PathBuf {
    fs::create_dir_all(folder).unwrap();
    let path = folder.join(format!(
        "Stacked_{poses}_IC 1805_{exposure:.1}s_{}_20260812-061931.fit",
        filter.replace('/', "-")
    ));
    let cards: Vec<(String, String)> = vec![
        ("SIMPLE".into(), "T".into()),
        ("BITPIX".into(), "16".into()),
        ("NAXIS".into(), "3".into()),
        ("NAXIS1".into(), "2160".into()),
        ("NAXIS2".into(), "3840".into()),
        ("NAXIS3".into(), "3".into()),
        ("BZERO".into(), "32768".into()),
        ("CREATOR".into(), "'ZWO Seestar S50 Pro'".into()),
        ("FILTER".into(), format!("'{filter}'")),
        ("GAIN".into(), "100".into()),
        ("EXPTIME".into(), format!("{exposure:.1}")),
        ("STACKCNT".into(), poses.to_string()),
        ("TOTALEXP".into(), format!("{:.1}", exposure * poses as f64)),
    ];
    let cards: Vec<(&str, &str)> = cards
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    fs::write(&path, crate::test_support::fits_bytes(&cards, 32)).unwrap();
    path
}

pub(crate) fn capabilities(progression: bool) -> Capabilities {
    capabilities_with(progression, true)
}

/// The capabilities of a space: "Advanced Sync+"
/// (`analysis.progression`) and "Advanced Planning" (`planner.mosaic`)
/// are independent.
pub(crate) fn capabilities_with(progression: bool, mosaic: bool) -> Capabilities {
    serde_json::from_value(json!({
        "planner.multiple": true, "planner.mosaic": mosaic, "pointing.solve": true,
        "sync.plan_limit": 0, "gallery.private": true, "portal.invitations": true,
        "capture.variant_limit": 12, "capture.count_limit": 0, "capture.public": true,
        "analysis.logs": true, "analysis.progression": progression,
        "storage.max_file_bytes": 100, "storage.quota_bytes": 1000, "history.retention_days": 0
    }))
    .unwrap()
}

#[test]
fn a_master_kept_is_read_from_its_own_header() {
    let dir = TempDir::new("multi-night-master");
    let stacked = dir.path().join("IC 1805/stacked");
    write_master(&stacked, "Dualband_180s_g100", Some("L-eXtreme"), 180.0, 98);
    fs::write(stacked.join("IC1805_raw.tif"), b"tif").unwrap();
    fs::write(stacked.join("broken_integration.fit"), b"not a fits").unwrap();
    let masters = night_masters(dir.path(), "IC 1805");
    assert_eq!(masters.len(), 1);
    let master = &masters[0];
    assert_eq!(master.family, "Dualband_180s_g100");
    assert_eq!(master.filter.as_deref(), Some("L-eXtreme"));
    assert_eq!(master.frame_count, 98);
    assert!((master.total_seconds - 17_640.0).abs() < 1e-9);
    assert_eq!(master.binning, Some(1));
    assert_eq!(master.temperature_c, Some(-10.0));
    assert!(master.colour);
    assert!(master.bytes > 0);
}

#[test]
fn the_lines_say_the_family_never_a_file() {
    let dir = TempDir::new("multi-night-lines");
    let path = write_master(dir.path(), "RGB_90s_g100", Some("RGB"), 90.0, 89);
    let master = read_master(&path).unwrap();
    let computer = Computer {
        label: Some("iMac du bureau".into()),
        installation_id: Some("inst-1".into()),
    };
    let lines = integration_lines("night-10", "ic-1805", "2026-08-10", &[master], &computer);
    let json = serde_json::to_value(&lines).unwrap();
    assert_eq!(
        json,
        json!([{
            "nightId": "night-10",
            "targetId": "ic-1805",
            "filter": "RGB",
            "exposureSeconds": 90.0,
            "gain": 100,
            "binning": 1,
            "temperatureC": -10.0,
            "frameCount": 89,
            "eveningDate": "2026-08-10",
            "sensor": "ZWO ASI2600MC Pro",
            "computerLabel": "iMac du bureau",
            "installationId": "inst-1"
        }])
    );
    assert!(!json.to_string().contains("integration.fit"));
}

#[test]
fn the_right_is_the_progression_where_the_service_sells_them_and_always_open_on_a_private_server() {
    assert!(MultiNightGate::new(Some(&capabilities(true)), true).allowed);
    // A private server never restricts them, whatever its
    // capabilities say, known or not.
    for capabilities in [Some(capabilities(true)), Some(capabilities(false)), None] {
        let private = MultiNightGate::new(capabilities.as_ref(), false);
        assert_eq!((private.allowed, private.closed), (true, None));
    }
    // A service never heard: closed, "unknown".
    assert_eq!(
        MultiNightGate::unknown().closed,
        Some(MultiNightClosed::Unknown)
    );
    assert_eq!(
        MultiNightGate::new(Some(&capabilities(false)), true).closed,
        Some(MultiNightClosed::NotInSubscription)
    );
    assert_eq!(
        MultiNightGate::new(None, true).closed,
        Some(MultiNightClosed::Unknown)
    );
}

#[test]
fn attaching_follows_planification_avancee_not_synchronisation_avancee_plus() {
    // The gestures on the plan itself are `planner.mosaic`'s.
    let planning_only = capabilities_with(false, true);
    assert!(AttachGate::new(Some(&planning_only), true).allowed);
    // "Advanced Sync+" alone does not attach...
    let sync_only = capabilities_with(true, false);
    assert_eq!(
        AttachGate::new(Some(&sync_only), true).closed,
        Some(MultiNightClosed::NotInSubscription)
    );
    // ...and Advanced Planning alone does not send integrations.
    assert!(!MultiNightGate::new(Some(&planning_only), true).allowed);
    assert!(MultiNightGate::new(Some(&sync_only), true).allowed);
    // A private server never restricts them.
    let private = AttachGate::new(Some(&capabilities_with(false, false)), false);
    assert_eq!((private.allowed, private.closed), (true, None));
    assert!(!AttachGate::unknown().allowed);
    assert_eq!(
        AttachGate::new(None, true).closed,
        Some(MultiNightClosed::Unknown)
    );
}

/// The service of the tests: a plan, its multi-night plans, a `PUT` that
/// answers what it is told to.
#[derive(Default)]
pub(crate) struct FakeService {
    pub plan: Value,
    pub projects: BTreeMap<String, Value>,
    pub put_answer: Option<Result<Value, (String, String)>>,
    pub puts: RefCell<Vec<(String, Value)>>,
    /// The plans a night belongs to by itself, by night.
    pub own: BTreeMap<String, Vec<String>>,
    /// … the answer of `GET /v1/projects/{id}/nights`, by plan.
    pub nights: BTreeMap<String, Value>,
    /// … what `POST …/nights`, `DELETE …/nights/{night}` and `POST
    /// /v1/projects` answer (`None`: `{}`).
    pub write_answer: Option<Result<Value, (String, String)>>,
    /// … the writes asked, `"POST /nights p-heart"`-style, with their body.
    pub writes: RefCell<Vec<(String, Value)>>,
    /// A service that predates `replace`: a body carrying it is refused
    /// (`400 invalid_request`, unexpected property).
    pub rejects_replace: bool,
    /// A service that keeps the lines it is sent, as the portal does
    /// (`held`): lines added or replaced by (night, target, family), the sets
    /// of `replace` complete. Its `PUT` answers the lines held.
    pub keeps_lines: bool,
    pub held: RefCell<Vec<Value>>,
}

impl MultiNightTransport for FakeService {
    fn plan(&self, _: &str, _: &str, _: &str) -> Result<Value, GuiBridgeError> {
        Ok(self.plan.clone())
    }
    fn project(&self, _: &str, _: &str, project_id: &str) -> Result<Value, GuiBridgeError> {
        self.projects
            .get(project_id)
            .cloned()
            .ok_or_else(|| GuiBridgeError::Refused {
                code: "project_not_found".into(),
                message: "The project was not found.".into(),
            })
    }
    fn projects(&self, _: &str, _: &str) -> Result<Value, GuiBridgeError> {
        Ok(json!({
            "projects": self.projects.values().cloned().collect::<Vec<_>>()
        }))
    }
    fn projects_of_night(
        &self,
        _: &str,
        _: &str,
        session_id: &str,
    ) -> Result<Value, GuiBridgeError> {
        let ids = self.own.get(session_id).cloned().unwrap_or_default();
        Ok(json!({
            "projects": ids
                .iter()
                .filter_map(|id| self.projects.get(id).cloned())
                .collect::<Vec<_>>()
        }))
    }
    fn project_nights(&self, _: &str, _: &str, project_id: &str) -> Result<Value, GuiBridgeError> {
        if !self.projects.contains_key(project_id) {
            return Err(GuiBridgeError::Refused {
                code: "project_not_found".into(),
                message: "The project was not found.".into(),
            });
        }
        Ok(self
            .nights
            .get(project_id)
            .cloned()
            .unwrap_or_else(|| json!({ "projectId": project_id, "nights": [], "candidates": [] })))
    }
    fn attach_nights(
        &self,
        _: &str,
        _: &str,
        project_id: &str,
        body: &Value,
    ) -> Result<Value, GuiBridgeError> {
        self.write("POST", &format!("/projects/{project_id}/nights"), body)
    }
    fn detach_night(
        &self,
        _: &str,
        _: &str,
        project_id: &str,
        night_id: &str,
    ) -> Result<Value, GuiBridgeError> {
        self.write(
            "DELETE",
            &format!("/projects/{project_id}/nights/{night_id}"),
            &Value::Null,
        )
    }
    fn create_project(&self, _: &str, _: &str, body: &Value) -> Result<Value, GuiBridgeError> {
        self.write("POST", "/projects", body)
    }
    fn put_integrations(
        &self,
        _: &str,
        _: &str,
        project_id: &str,
        body: &Value,
    ) -> Result<Value, GuiBridgeError> {
        self.puts
            .borrow_mut()
            .push((project_id.to_string(), body.clone()));
        if self.rejects_replace && body.get("replace").is_some() {
            return Err(GuiBridgeError::Refused {
                code: "invalid_request".into(),
                message: "Unexpected property \"replace\".".into(),
            });
        }
        if self.keeps_lines {
            return Ok(self.keep(body));
        }
        match self.put_answer.clone().unwrap_or_else(|| Ok(json!({}))) {
            Ok(answer) => Ok(answer),
            Err((code, message)) => Err(GuiBridgeError::Refused { code, message }),
        }
    }
}

impl FakeService {
    /// What the portal does with the body of a `PUT …/integrations`.
    fn keep(&self, body: &Value) -> Value {
        let key = |line: &Value| {
            [
                "nightId",
                "targetId",
                "filter",
                "exposureSeconds",
                "gain",
                "binning",
                "temperatureC",
            ]
            .map(|name| line[name].to_string())
            .join("|")
        };
        let lines = body["integrations"].as_array().cloned().unwrap_or_default();
        let mut held = self.held.borrow_mut();
        for line in &lines {
            held.retain(|other| key(other) != key(line));
            held.push(line.clone());
        }
        for set in body["replace"].as_array().into_iter().flatten() {
            held.retain(|other| {
                other["nightId"] != set["nightId"]
                    || other["targetId"] != set["targetId"]
                    || lines.iter().any(|line| key(line) == key(other))
            });
        }
        json!({ "integrations": *held })
    }

    fn write(&self, method: &str, path: &str, body: &Value) -> Result<Value, GuiBridgeError> {
        self.writes
            .borrow_mut()
            .push((format!("{method} {path}"), body.clone()));
        match self.write_answer.clone().unwrap_or_else(|| Ok(json!({}))) {
            Ok(answer) => Ok(answer),
            Err((code, message)) => Err(GuiBridgeError::Refused { code, message }),
        }
    }
}

pub(crate) fn integration(
    night: &str,
    date: &str,
    filter: &str,
    exposure: f64,
    frames: u64,
    computer: &str,
) -> Value {
    json!({
        "id": format!("i-{night}"), "projectId": "p-heart", "nightId": night,
        "targetId": "ic-1805", "filter": filter, "exposureSeconds": exposure,
        "gain": 100, "binning": 1, "temperatureC": -10, "frameCount": frames,
        "totalSeconds": exposure * frames as f64, "eveningDate": date,
        "sensor": "IMX571", "computerLabel": computer,
        "installationId": if computer == "Portable" { "inst-2" } else { "inst-1" },
        "createdAt": "2026-08-10T10:00:00Z", "updatedAt": "2026-08-10T10:00:00Z"
    })
}

/// The Heart: a plan of three nights (sheet § 2).
pub(crate) fn heart() -> Value {
    json!({
        "id": "p-heart", "revision": 3, "status": "active",
        "target": { "targetId": "ic-1805", "name": "IC 1805", "commonName": "Nébuleuse du Cœur" },
        "goalMinutes": 720,
        "integrations": [
            integration("n-12", "2026-08-12", "UV/IR-cut", 120.0, 120, "iMac du bureau"),
            integration("n-09", "2026-08-09", "L-eXtreme", 180.0, 98, "iMac du bureau"),
            integration("n-10", "2026-08-10", "RGB", 90.0, 89, "iMac du bureau"),
        ],
        "planCapture": null
    })
}

#[test]
fn the_plan_of_a_night_names_its_multi_night_plans() {
    let service = FakeService {
        plan: json!({
            "id": "plan-12",
            "payload": {
                "date": "2026-08-12",
                "items": [
                    { "id": "a", "targetId": "ic-1805", "projectId": "p-heart" },
                    { "id": "b", "targetId": "ic-1805", "projectId": "p-heart" },
                    { "id": "c", "targetId": "m-31" },
                    { "id": "d", "targetId": "m-33", "projectId": "p-gone" }
                ]
            }
        }),
        projects: BTreeMap::from([("p-heart".to_string(), heart())]),
        ..FakeService::default()
    };
    let night = night_projects(&service, "https://s", "t", Some("plan-12"), None).unwrap();
    assert_eq!(night.evening_date.as_deref(), Some("2026-08-12"));
    // The plan the service no longer knows is left out, never an error.
    assert_eq!(night.projects.len(), 1);
    let heart = &night.projects[0];
    assert_eq!(
        (heart.name.as_str(), heart.target_id.as_str()),
        ("IC 1805", "ic-1805")
    );
    assert_eq!(heart.night_count, 3);
    // 4 h 54 + 2 h 14 + 4 h 00 = 11 h 08.
    assert_eq!((heart.total_seconds / 60.0).round(), 668.0);
    let dates: Vec<&str> = heart
        .integrations
        .iter()
        .map(|i| i.evening_date.as_str())
        .collect();
    assert_eq!(dates, vec!["2026-08-09", "2026-08-10", "2026-08-12"]);
    // A night without a plan has no multi-night plan.
    assert!(
        night_projects(&service, "https://s", "t", None, None)
            .unwrap()
            .projects
            .is_empty()
    );
}

#[test]
fn a_night_attached_by_itself_reads_its_plan_by_its_own_id() {
    // The night of the 10th has no plan: no plan item names the multi-night
    // plan, the service says it by `GET /v1/projects?sessionId=`.
    let service = FakeService {
        projects: BTreeMap::from([("p-heart".to_string(), heart())]),
        own: BTreeMap::from([("n-10".to_string(), vec!["p-heart".to_string()])]),
        ..FakeService::default()
    };
    let night = night_projects(&service, "https://s", "t", None, Some("n-10")).unwrap();
    assert_eq!(night.evening_date, None);
    assert_eq!(night.projects.len(), 1);
    let own = &night.projects[0];
    // The night's target is the plan's.
    assert_eq!(
        (own.project_id.as_str(), own.target_id.as_str()),
        ("p-heart", "ic-1805")
    );
    assert_eq!(own.night_count, 3);

    // A night that is in no plan by itself has none.
    let none = night_projects(&service, "https://s", "t", None, Some("n-99")).unwrap();
    assert!(none.projects.is_empty());

    // A plan named by an item and by the night is listed once.
    let both = FakeService {
        plan: json!({ "payload": { "date": "2026-08-12", "items": [
            { "targetId": "ic-1805", "projectId": "p-heart" }
        ]}}),
        projects: BTreeMap::from([("p-heart".to_string(), heart())]),
        own: BTreeMap::from([("n-12".to_string(), vec!["p-heart".to_string()])]),
        ..FakeService::default()
    };
    let night = night_projects(&both, "u", "t", Some("plan-12"), Some("n-12")).unwrap();
    assert_eq!(night.projects.len(), 1);
    assert_eq!(night.evening_date.as_deref(), Some("2026-08-12"));
}

#[test]
fn a_service_that_refuses_the_night_filter_says_nothing_of_it() {
    struct Refusing(FakeService);
    impl MultiNightTransport for Refusing {
        fn plan(&self, a: &str, b: &str, c: &str) -> Result<Value, GuiBridgeError> {
            self.0.plan(a, b, c)
        }
        fn project(&self, a: &str, b: &str, c: &str) -> Result<Value, GuiBridgeError> {
            self.0.project(a, b, c)
        }
        fn projects(&self, a: &str, b: &str) -> Result<Value, GuiBridgeError> {
            self.0.projects(a, b)
        }
        fn projects_of_night(&self, _: &str, _: &str, _: &str) -> Result<Value, GuiBridgeError> {
            Err(GuiBridgeError::Refused {
                code: "not_found".into(),
                message: "no".into(),
            })
        }
        fn project_nights(&self, a: &str, b: &str, c: &str) -> Result<Value, GuiBridgeError> {
            self.0.project_nights(a, b, c)
        }
        fn attach_nights(
            &self,
            a: &str,
            b: &str,
            c: &str,
            d: &Value,
        ) -> Result<Value, GuiBridgeError> {
            self.0.attach_nights(a, b, c, d)
        }
        fn detach_night(
            &self,
            a: &str,
            b: &str,
            c: &str,
            d: &str,
        ) -> Result<Value, GuiBridgeError> {
            self.0.detach_night(a, b, c, d)
        }
        fn create_project(&self, a: &str, b: &str, c: &Value) -> Result<Value, GuiBridgeError> {
            self.0.create_project(a, b, c)
        }
        fn put_integrations(
            &self,
            a: &str,
            b: &str,
            c: &str,
            d: &Value,
        ) -> Result<Value, GuiBridgeError> {
            self.0.put_integrations(a, b, c, d)
        }
    }
    let service = Refusing(FakeService::default());
    let night = night_projects(&service, "u", "t", None, Some("n-10")).unwrap();
    assert!(night.projects.is_empty());
}

#[test]
fn a_night_attached_by_itself_is_tagged_without_an_integration() {
    let service = FakeService {
        projects: BTreeMap::from([(
            "p-m31".to_string(),
            json!({ "id": "p-m31", "target": { "name": "M31" }, "integrations": [] }),
        )]),
        nights: BTreeMap::from([(
            "p-m31".to_string(),
            json!({ "projectId": "p-m31", "candidates": [], "nights": [
                { "sessionId": "n-20", "planId": null, "link": "night" },
                { "sessionId": null, "planId": "plan-21", "link": "plan" }
            ]}),
        )]),
        ..FakeService::default()
    };
    let tags = plan_tags(&service, "u", "t").unwrap();
    // The night run is tagged; the plan not run yet is not a night.
    assert_eq!(
        tags.keys().map(String::as_str).collect::<Vec<_>>(),
        vec!["n-20"]
    );
    assert_eq!(tags["n-20"][0].night_count, 1);
    assert_eq!(tags["n-20"][0].total_seconds, 0.0);
}

#[test]
fn the_lines_leave_only_with_the_right_and_a_refusal_is_said_in_clear() {
    let dir = TempDir::new("multi-night-send");
    let master = read_master(&write_master(
        dir.path(),
        "UV-IR-cut_120s_g100",
        Some("UV/IR-cut"),
        120.0,
        120,
    ))
    .unwrap();
    let lines = integration_lines(
        "n-12",
        "ic-1805",
        "2026-08-12",
        &[master],
        &Computer::default(),
    );
    let project = project_of(&heart(), "ic-1805").unwrap();
    let open = MultiNightGate::new(Some(&capabilities(true)), true);

    // Without the right: nothing is asked of the service.
    let service = FakeService::default();
    let closed = MultiNightGate::new(Some(&capabilities(false)), true);
    let sent = send_integrations(&service, "u", "t", &project, &lines, closed).unwrap();
    assert!(matches!(sent, IntegrationsSent::Closed { .. }));
    // Nor while the service was never heard.
    let sent = send_integrations(
        &service,
        "u",
        "t",
        &project,
        &lines,
        MultiNightGate::unknown(),
    )
    .unwrap();
    assert_eq!(
        sent,
        IntegrationsSent::Closed {
            project_id: "p-heart".into(),
            closed: MultiNightClosed::Unknown
        }
    );
    assert!(service.puts.borrow().is_empty());

    // With it: one `PUT` of the lines, the plan as it now stands.
    let service = FakeService {
        put_answer: Some(Ok(json!({ "integrations": [
            integration("n-09", "2026-08-09", "L-eXtreme", 180.0, 98, "iMac du bureau"),
            integration("n-12", "2026-08-12", "UV/IR-cut", 120.0, 120, "iMac du bureau"),
        ]}))),
        ..FakeService::default()
    };
    let sent = send_integrations(&service, "u", "t", &project, &lines, open).unwrap();
    let IntegrationsSent::Sent {
        lines: n, project, ..
    } = sent
    else {
        panic!("{sent:?}")
    };
    assert_eq!((n, project.night_count), (1, 2));
    let puts = service.puts.borrow();
    assert_eq!(puts[0].0, "p-heart");
    assert_eq!(puts[0].1["integrations"][0]["frameCount"], 120);

    // The service's refusal keeps its code and its words.
    for code in ["multi_night_not_active", "integration_outside_night"] {
        let service = FakeService {
            put_answer: Some(Err((code.to_string(), format!("words of {code}")))),
            ..FakeService::default()
        };
        let sent = send_integrations(
            &service,
            "u",
            "t",
            &project_of(&heart(), "ic-1805").unwrap(),
            &lines,
            open,
        )
        .unwrap();
        assert_eq!(
            sent,
            IntegrationsSent::Refused {
                project_id: "p-heart".into(),
                code: code.into(),
                message: format!("words of {code}"),
            }
        );
    }
}

#[test]
fn each_night_of_a_plan_is_tagged_with_its_integrations() {
    let service = FakeService {
        projects: BTreeMap::from([
            ("p-heart".to_string(), heart()),
            (
                "p-empty".to_string(),
                json!({ "id": "p-empty", "target": { "name": "M31" }, "integrations": [] }),
            ),
        ]),
        ..FakeService::default()
    };
    let tags = plan_tags(&service, "u", "t").unwrap();
    assert_eq!(
        tags.keys().map(String::as_str).collect::<Vec<_>>(),
        vec!["n-09", "n-10", "n-12"]
    );
    let tag = &tags["n-10"][0];
    assert_eq!((tag.name.as_str(), tag.night_count), ("IC 1805", 3));
    assert_eq!((tag.total_seconds / 60.0).round(), 668.0);
}

#[test]
fn the_integration_of_a_night_is_previewed_by_family_with_its_master() {
    use crate::detection::NightWindow;
    let dir = TempDir::new("multi-night-preview");
    let lights = dir.path().join("IC 1805/lights");
    fs::create_dir_all(&lights).unwrap();
    let frame = |name: &str, temperature: &str, date: &str| {
        let bytes = fits_bytes(
            &[
                ("IMAGETYP", "'LIGHT'"),
                ("FILTER", "'UV/IR-cut'"),
                ("EXPTIME", "120.0"),
                ("GAIN", "100"),
                ("XBINNING", "1"),
                ("CCD-TEMP", temperature),
                ("DATE-OBS", &format!("'{date}'")),
            ],
            0,
        );
        fs::write(lights.join(name), bytes).unwrap();
    };
    frame("a.fits", "-10.0", "2026-08-12T21:00:00");
    frame("b.fits", "-10.0", "2026-08-12T23:00:00");
    frame("c.fits", "-9.0", "2026-08-13T01:00:00");
    frame("late.fits", "-10.0", "2026-08-13T20:15:00");
    let window = NightWindow::of_evening("2026-08-12", Some(6.0)).unwrap();
    let preview = integration_preview(dir.path(), "IC 1805", Some(&window));
    assert_eq!(preview.outside, 1);
    assert_eq!(preview.families.len(), 1);
    let family = &preview.families[0];
    assert_eq!(family.label, "UV_IR-cut_120s_g100");
    assert_eq!((family.frame_count, family.total_seconds), (3, 360.0));
    assert_eq!(family.temperature_c, Some(-10));
    // The band the card of the target says ("Broadband").
    assert_eq!(family.band, crate::plan_image::Band::Broadband);
    assert!(family.master.is_none());
    write_master(
        &dir.path().join("IC 1805/stacked"),
        "UV_IR-cut_120s_g100",
        Some("UV/IR-cut"),
        120.0,
        3,
    );
    let again = integration_preview(dir.path(), "IC 1805", Some(&window));
    assert!(
        again.families[0]
            .master
            .as_ref()
            .is_some_and(|m| m.bytes > 0)
    );
}

#[test]
fn a_master_stacked_without_a_filter_takes_the_one_the_target_folder_records() {
    let dir = TempDir::new("multi-night-assumed-filter");
    let night = dir.path().join("2026-08-07 · Col · n-07");
    let target = night.join("Sh2-103");
    fs::create_dir_all(target.join("stacked")).unwrap();
    fs::write(
        target.join(crate::working_folder::TARGET_IDENTITY_FILE),
        r#"{"format":1,"targetId":"sharpless-103"}"#,
    )
    .unwrap();
    // As the stacking left it before the filter was known: no FILTER card.
    write_master(
        &target.join("stacked"),
        "nofilter_180s_g100",
        None,
        180.0,
        120,
    );
    assert_eq!(night_masters(&night, "Sh2-103")[0].filter, None);

    crate::working_folder::set_target_filter(&target, "Dualband").unwrap();
    let masters = target_masters(&night, "sharpless-103");
    assert_eq!(masters[0].filter.as_deref(), Some("Dualband"));
    // A master that names its own filter keeps it.
    write_master(
        &target.join("stacked"),
        "UVIR_180s_g100",
        Some("UV/IR cut"),
        180.0,
        89,
    );
    let kept: Vec<_> = night_masters(&night, "Sh2-103")
        .into_iter()
        .map(|m| m.filter)
        .collect();
    assert!(kept.contains(&Some("UV/IR cut".to_string())));
    assert!(kept.contains(&Some("Dualband".to_string())));
}

#[test]
fn a_night_stacked_again_under_another_filter_keeps_one_active_master() {
    let dir = TempDir::new("multi-night-restacked");
    let night = dir.path().join("2026-08-12 · 18930f");
    let target = night.join("Sh2-103");
    let stacked = target.join("stacked");
    fs::create_dir_all(&stacked).unwrap();
    fs::write(
        target.join(crate::working_folder::TARGET_IDENTITY_FILE),
        r#"{"format":1,"targetId":"sharpless-103","filter":"Dual-band"}"#,
    )
    .unwrap();
    let first_frame = [("DATE-OBS", "'2026-08-12T22:47:09.915989'")];
    // The lights name no filter: stacked first as dual-band, then again as
    // broadband once the person changed the filter of the night.
    let old = write_master_with(
        &stacked,
        "Dual-band_180s_g100",
        Some("Dual-band"),
        180.0,
        100,
        &first_frame,
    );
    let at = |hours: u64| {
        std::time::SystemTime::UNIX_EPOCH
            + std::time::Duration::from_secs(1_790_000_000 + hours * 3600)
    };
    fs::File::options()
        .write(true)
        .open(&old)
        .unwrap()
        .set_modified(at(0))
        .unwrap();
    let new = write_master_with(
        &stacked,
        "Broadband_180s_g100",
        Some("Broadband"),
        180.0,
        100,
        &first_frame,
    );
    fs::File::options()
        .write(true)
        .open(&new)
        .unwrap()
        .set_modified(at(100))
        .unwrap();
    // Another family of the same night (other poses, another first frame)
    // is a master of its own.
    write_master_with(
        &stacked,
        "Dual-band_300s_g100",
        Some("Dual-band"),
        300.0,
        20,
        &[("DATE-OBS", "'2026-08-13T01:10:00'")],
    );

    let families =
        |masters: Vec<IntegrationMaster>| masters.into_iter().map(|m| m.family).collect::<Vec<_>>();
    // The filter the folder records is the person's last choice.
    crate::working_folder::set_target_filter(&target, "Broadband").unwrap();
    assert_eq!(
        families(night_masters(&night, "Sh2-103")),
        ["Broadband_180s_g100", "Dual-band_300s_g100"]
    );
    let lines = integration_lines(
        "night-12",
        "sharpless-103",
        "2026-08-12",
        &target_masters(&night, "sharpless-103"),
        &Computer::default(),
    );
    assert_eq!(lines.len(), 2);
    assert_eq!(
        lines.iter().map(|l| l.frame_count).sum::<u64>(),
        120,
        "the night counts once"
    );
    assert!(
        lines
            .iter()
            .all(|l| l.filter.as_deref() != Some("Dual-band") || l.exposure_seconds == 300.0)
    );
    // Back to dual-band: the other master is the active one, nothing on disk changed.
    crate::working_folder::set_target_filter(&target, "Dual-band").unwrap();
    assert_eq!(
        families(night_masters(&night, "Sh2-103")),
        ["Dual-band_180s_g100", "Dual-band_300s_g100"]
    );
    assert!(old.is_file() && new.is_file());
    // A folder that records a filter neither master names: the newest wins.
    crate::working_folder::set_target_filter(&target, "L-eXtreme").unwrap();
    assert_eq!(
        families(night_masters(&night, "Sh2-103")),
        ["Broadband_180s_g100", "Dual-band_300s_g100"]
    );
}

fn line_of(night: &str, target: &str, filter: &str, exposure: f64) -> IntegrationLine {
    IntegrationLine {
        night_id: night.to_string(),
        target_id: target.to_string(),
        filter: Some(filter.to_string()),
        exposure_seconds: exposure,
        gain: Some(100),
        binning: Some(1),
        temperature_c: Some(-10.0),
        frame_count: 100,
        evening_date: "2026-08-12".to_string(),
        sensor: None,
        computer_label: None,
        installation_id: None,
    }
}

#[test]
fn the_lines_of_a_night_are_sent_complete_and_an_older_service_gets_the_earlier_form() {
    let project = project_of(&heart(), "ic-1805").unwrap();
    let open = MultiNightGate::new(Some(&capabilities(true)), true);

    // The lines of a night and a target name their set in `replace`; a set
    // is named once, whatever the number of its lines, and two nights are
    // two sets.
    let service = FakeService::default();
    let lines = [
        line_of("n-12", "ic-1805", "UV/IR-cut", 120.0),
        line_of("n-12", "ic-1805", "UV/IR-cut", 300.0),
        line_of("n-13", "ic-1805", "UV/IR-cut", 120.0),
    ];
    send_integrations(&service, "u", "t", &project, &lines, open).unwrap();
    let puts = service.puts.borrow();
    assert_eq!(puts.len(), 1);
    assert_eq!(puts[0].1["integrations"].as_array().unwrap().len(), 3);
    assert_eq!(
        puts[0].1["replace"],
        json!([
            { "nightId": "n-12", "targetId": "ic-1805" },
            { "nightId": "n-13", "targetId": "ic-1805" }
        ])
    );
    drop(puts);

    // A service that predates `replace` refuses the property: the same
    // lines are sent again in the earlier form, which removes nothing.
    let older = FakeService {
        rejects_replace: true,
        put_answer: Some(Ok(json!({ "integrations": [] }))),
        ..FakeService::default()
    };
    let sent = send_integrations(&older, "u", "t", &project, &lines, open).unwrap();
    assert!(matches!(sent, IntegrationsSent::Sent { lines: 3, .. }));
    let puts = older.puts.borrow();
    assert_eq!(puts.len(), 2);
    assert!(puts[0].1.get("replace").is_some());
    assert!(puts[1].1.get("replace").is_none());
    assert_eq!(puts[1].1["integrations"].as_array().unwrap().len(), 3);
    drop(puts);

    // An `invalid_request` of the earlier form too is the service's word,
    // kept as it is.
    let refusing = FakeService {
        put_answer: Some(Err(("invalid_request".into(), "words".into()))),
        ..FakeService::default()
    };
    let sent = send_integrations(&refusing, "u", "t", &project, &lines, open).unwrap();
    assert_eq!(
        sent,
        IntegrationsSent::Refused {
            project_id: "p-heart".into(),
            code: "invalid_request".into(),
            message: "words".into()
        }
    );
    assert_eq!(refusing.puts.borrow().len(), 2);

    // Other refusals are not retried.
    let outside = FakeService {
        put_answer: Some(Err(("integration_outside_night".into(), "words".into()))),
        ..FakeService::default()
    };
    send_integrations(&outside, "u", "t", &project, &lines, open).unwrap();
    assert_eq!(outside.puts.borrow().len(), 1);

    // More lines than one `PUT` carries are never sent as a complete set: a
    // truncated set would remove the lines left out.
    let many: Vec<IntegrationLine> = (0..201)
        .map(|n| line_of("n-12", "ic-1805", "UV/IR-cut", 10.0 + f64::from(n)))
        .collect();
    let big = FakeService::default();
    send_integrations(&big, "u", "t", &project, &many, open).unwrap();
    let puts = big.puts.borrow();
    assert_eq!(puts.len(), 1);
    assert!(puts[0].1.get("replace").is_none());
    assert_eq!(puts[0].1["integrations"].as_array().unwrap().len(), 200);
}

#[test]
fn a_night_stacked_again_under_another_filter_is_retired_from_the_plan() {
    use crate::night_steps::send_night_integrations;
    let dir = TempDir::new("multi-night-retired");
    let night = dir.path().join("2026-08-12 · 18930f");
    let target = night.join("Sh2-103");
    let stacked = target.join("stacked");
    fs::create_dir_all(&stacked).unwrap();
    fs::write(
        target.join(crate::working_folder::TARGET_IDENTITY_FILE),
        r#"{"format":1,"targetId":"sharpless-103"}"#,
    )
    .unwrap();
    // The same poses stacked under two filters; another family beside them.
    let first_frame = [("DATE-OBS", "'2026-08-12T22:47:09.915989'")];
    for (family, filter) in [
        ("Dual-band_180s_g100", "Dual-band"),
        ("Broadband_180s_g100", "Broadband"),
    ] {
        write_master_with(&stacked, family, Some(filter), 180.0, 100, &first_frame);
    }
    write_master_with(
        &stacked,
        "Dual-band_300s_g100",
        Some("Dual-band"),
        300.0,
        20,
        &[("DATE-OBS", "'2026-08-13T01:10:00'")],
    );
    let plan = project_of(
        &json!({ "id": "p-sh2", "target": { "targetId": "sharpless-103", "name": "Sh2-103" }, "integrations": [] }),
        "sharpless-103",
    )
    .unwrap();
    let multi_night = NightMultiNight {
        evening_date: Some("2026-08-12".to_string()),
        projects: vec![plan],
    };
    let open = MultiNightGate::new(Some(&capabilities(true)), true);
    let send = |service: &FakeService| {
        send_night_integrations(
            service,
            "u",
            "t",
            "night-12",
            &night,
            &multi_night,
            "2026-08-12",
            &Computer::default(),
            open,
        )
        .unwrap()
    };
    let filters = |service: &FakeService| {
        let mut held: Vec<(String, f64)> = service
            .held
            .borrow()
            .iter()
            .map(|line| {
                (
                    line["filter"].as_str().unwrap().to_string(),
                    line["exposureSeconds"].as_f64().unwrap(),
                )
            })
            .collect();
        held.sort_by(|a, b| a.partial_cmp(b).unwrap());
        held
    };

    let service = FakeService {
        keeps_lines: true,
        ..FakeService::default()
    };
    let older = FakeService {
        keeps_lines: true,
        rejects_replace: true,
        ..FakeService::default()
    };
    // Stacked as dual-band, sent.
    crate::working_folder::set_target_filter(&target, "Dual-band").unwrap();
    send(&service);
    send(&older);
    let dual = vec![
        ("Dual-band".to_string(), 180.0),
        ("Dual-band".to_string(), 300.0),
    ];
    assert_eq!(filters(&service), dual);
    assert_eq!(filters(&older), dual);

    // The person corrects the filter: the night is stacked again as
    // broadband, the dual-band master of the same poses leaves the plan.
    crate::working_folder::set_target_filter(&target, "Broadband").unwrap();
    let sent = send(&service);
    assert!(matches!(sent[0], IntegrationsSent::Sent { lines: 2, .. }));
    // Only the lines the night still carries are held; nothing is doubled.
    assert_eq!(
        filters(&service),
        vec![
            ("Broadband".to_string(), 180.0),
            ("Dual-band".to_string(), 300.0)
        ]
    );
    // A service that predates `replace` cannot retire it (the earlier form
    // removes nothing): that limit is the older service's.
    send(&older);
    assert_eq!(filters(&older).len(), 3);
    // Sent again: nothing changes.
    send(&service);
    assert_eq!(service.held.borrow().len(), 2);
}
