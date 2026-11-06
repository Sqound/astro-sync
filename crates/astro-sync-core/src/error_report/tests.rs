use super::*;
use crate::client_identity;
use crate::test_support::TempDir;
use std::cell::RefCell;
use std::io::{Read, Write};

const ON: ReportingPolicy = ReportingPolicy {
    switch_on_screen: true,
};
const HOSTED: &str = "https://portal.planastro.space";

fn files_in(dir: &TempDir) -> ErrorReportFiles {
    ErrorReportFiles {
        settings: dir.path().join("settings.json"),
        local_settings: dir.path().join("local-settings.json"),
        queue: dir.path().join("error-reports.json"),
        diagnostic_id: dir.path().join("diagnostic-id"),
    }
}

/// The settings files of a computer on `service` (none: the hosted default)
/// whose person chose `report`.
fn configure(files: &ErrorReportFiles, service: Option<&str>, report: Option<bool>) {
    let settings = gui_bridge::GuiSettings {
        portal_url: service.map(str::to_string),
        device_label: Some("Ordinateur de Camille".to_string()),
        locale: None,
    };
    gui_bridge::save_settings(&files.settings, &settings).unwrap();
    let mut local = local_settings::LocalSettings::default();
    if let Some(report) = report {
        local.set_report_errors(report);
    }
    local_settings::save(&files.local_settings, &local).unwrap();
}

fn panic_error(message: &str) -> RawError<'_> {
    RawError {
        kind: ErrorKind::Panic,
        message,
        stack: Some("at upload_queue.rs:120:9 (astro-sync-core)"),
    }
}

fn queued(files: &ErrorReportFiles) -> Vec<ErrorReport> {
    load_queue(&files.queue).reports
}

#[derive(Default)]
struct Fake {
    answers: RefCell<Vec<Delivery>>,
    seen: RefCell<Vec<(String, String, serde_json::Value)>>,
}

impl ErrorReportTransport for Fake {
    fn deliver(&self, service_url: &str, token: &str, body: &serde_json::Value) -> Delivery {
        self.seen
            .borrow_mut()
            .push((service_url.to_string(), token.to_string(), body.clone()));
        let mut answers = self.answers.borrow_mut();
        if answers.is_empty() {
            Delivery::Accepted
        } else {
            answers.remove(0)
        }
    }
}

#[test]
fn nothing_is_queued_while_the_switch_is_not_on_the_screen_or_the_setting_is_off() {
    let dir = TempDir::new("error-report-gates");
    let files = files_in(&dir);
    // The person chose "yes" on the hosted service, the build has no switch.
    configure(&files, Some(HOSTED), Some(true));
    let closed = ReportingPolicy {
        switch_on_screen: false,
    };
    assert_eq!(
        record(&files, closed, "1.0.0", panic_error("boom")).unwrap(),
        Recorded::NotAllowed
    );
    // The default of the code is "no".
    configure(&files, Some(HOSTED), None);
    assert_eq!(
        record(&files, ON, "1.0.0", panic_error("boom")).unwrap(),
        Recorded::NotAllowed
    );
    configure(&files, Some(HOSTED), Some(false));
    assert_eq!(
        record(&files, ON, "1.0.0", panic_error("boom")).unwrap(),
        Recorded::NotAllowed
    );
    assert!(!files.queue.exists());
    // No settings files at all: no.
    let bare = files_in(&TempDir::new("error-report-bare"));
    assert!(!reporting_allowed(&bare, ON));
}

#[test]
fn a_private_server_never_gets_a_report() {
    let dir = TempDir::new("error-report-private");
    let files = files_in(&dir);
    for private in [
        "https://portal.example.test",
        "http://192.168.1.20:8787",
        "http://localhost:8787/",
        "https://portal.planastro.space.example.test",
    ] {
        configure(&files, Some(private), Some(true));
        assert!(!reporting_allowed(&files, ON), "{private}");
        assert_eq!(
            record(&files, ON, "1.0.0", panic_error("boom")).unwrap(),
            Recorded::NotAllowed,
            "{private}"
        );
    }
    // Reports queued on the hosted service, then the computer moves to a
    // private server: the queue is emptied and nothing is sent.
    configure(&files, Some(HOSTED), Some(true));
    record(&files, ON, "1.0.0", panic_error("boom")).unwrap();
    assert_eq!(queued(&files).len(), 1);
    configure(&files, Some("https://portal.example.test"), Some(true));
    let fake = Fake::default();
    assert_eq!(
        flush(&files, ON, Some("tok"), &fake).unwrap(),
        Flushed::NotAllowed
    );
    assert!(fake.seen.borrow().is_empty());
    assert!(!files.queue.exists());
}

#[test]
fn the_hosted_service_is_the_default_and_an_address_with_a_slash_is_the_same() {
    let dir = TempDir::new("error-report-hosted");
    let files = files_in(&dir);
    for service in [None, Some(HOSTED), Some("https://portal.planastro.space/")] {
        configure(&files, service, Some(true));
        assert!(reporting_allowed(&files, ON), "{service:?}");
    }
}

#[test]
fn a_report_holds_the_listed_fields_and_nothing_personal() {
    let dir = TempDir::new("error-report-content");
    let files = files_in(&dir);
    configure(&files, Some(HOSTED), Some(true));
    let message = "failed to open /Us\u{65}rs/camille/Astro/M31 2026-10-12/light_001.fits, \
                   for camille@example.test at 35.6895, 139.6917 on 192.168.1.20 \
                   with Io { name: \"Nuit M31\" }";
    record(&files, ON, "1.0.0", panic_error(message)).unwrap();
    let reports = queued(&files);
    assert_eq!(reports.len(), 1);
    let report = &reports[0];
    assert_eq!(
        report.message,
        "failed to open [path], for [e-mail] at [coord], [coord] on [réseau] with Io { name: \"…\" }"
    );
    assert_eq!(report.kind, ErrorKind::Panic);
    assert_eq!(report.app_version, "1.0.0");
    assert_eq!(report.os, space::current_platform());
    assert_eq!(report.arch, std::env::consts::ARCH);
    assert!(is_hex_id(&report.id));
    assert!(report.at.ends_with('Z'));
    // The exact keys of a report, and of the request.
    let json = serde_json::to_value(report).unwrap();
    let mut keys: Vec<&str> = json
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "appVersion",
            "arch",
            "at",
            "id",
            "kind",
            "message",
            "os",
            "osVersion",
            "stack"
        ]
    );
    let body = request_body("0123456789abcdef0123456789abcdef", &reports);
    let mut keys: Vec<&str> = body
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, ["application", "diagnosticId", "reports"]);
    // Nothing of the settings, of the computer's name or of a path anywhere.
    let all = body.to_string();
    for personal in [
        "camille",
        "Camille",
        "Users",
        "M31",
        "Ordinateur",
        "192.168",
        "35.68",
    ] {
        assert!(!all.contains(personal), "{personal} in {all}");
    }
}

#[test]
fn the_queue_is_bounded_and_the_same_failure_is_queued_once() {
    let dir = TempDir::new("error-report-queue");
    let files = files_in(&dir);
    configure(&files, Some(HOSTED), Some(true));
    assert_eq!(
        record(&files, ON, "1.0.0", panic_error("same")).unwrap(),
        Recorded::Queued(1)
    );
    assert_eq!(
        record(&files, ON, "1.0.0", panic_error("same")).unwrap(),
        Recorded::AlreadyQueued
    );
    for n in 0..30 {
        record(
            &files,
            ON,
            "1.0.0",
            panic_error(&format!("failure number {n}")),
        )
        .unwrap();
    }
    let reports = queued(&files);
    assert_eq!(reports.len(), QUEUE_LIMIT);
    // The oldest went first.
    assert_eq!(reports[0].message, "failure number 10");
    assert_eq!(reports[QUEUE_LIMIT - 1].message, "failure number 29");
}

#[test]
fn a_queue_file_with_something_unsafe_in_it_is_not_sent() {
    let dir = TempDir::new("error-report-hand-edited");
    let files = files_in(&dir);
    configure(&files, Some(HOSTED), Some(true));
    record(&files, ON, "1.0.0", panic_error("clean one")).unwrap();
    let mut queue = load_queue(&files.queue);
    let mut dirty = queue.reports[0].clone();
    dirty.id = "ffffffffffffffffffffffffffffffff".to_string();
    dirty.message = "read /Us\u{65}rs/camille/Astro".to_string();
    queue.reports.push(dirty);
    save_queue(&files.queue, &queue).unwrap();
    let fake = Fake::default();
    flush(&files, ON, Some("tok"), &fake).unwrap();
    let seen = fake.seen.borrow();
    let sent = seen[0].2["reports"].as_array().unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["message"], "clean one");
}

#[test]
fn a_flush_sends_to_the_service_in_force_with_the_token_and_the_random_identifier() {
    let dir = TempDir::new("error-report-flush");
    let files = files_in(&dir);
    configure(&files, Some(HOSTED), Some(true));
    for n in 0..13 {
        record(&files, ON, "1.0.0", panic_error(&format!("failure {n}"))).unwrap();
    }
    // No token: the reports wait.
    let fake = Fake::default();
    assert_eq!(flush(&files, ON, None, &fake).unwrap(), Flushed::NoToken);
    assert_eq!(
        flush(&files, ON, Some(""), &fake).unwrap(),
        Flushed::NoToken
    );
    assert!(fake.seen.borrow().is_empty());
    assert_eq!(queued(&files).len(), 13);
    // With the token: two requests of 10 and 3, then nothing waits.
    assert_eq!(
        flush(&files, ON, Some("device-token"), &fake).unwrap(),
        Flushed::Done {
            left: 13,
            waiting: 0
        }
    );
    let seen = fake.seen.borrow();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].0, HOSTED);
    assert_eq!(seen[0].1, "device-token");
    assert_eq!(seen[0].2["application"], "astro-sync");
    assert_eq!(seen[0].2["reports"].as_array().unwrap().len(), 10);
    assert_eq!(seen[1].2["reports"].as_array().unwrap().len(), 3);
    let id = seen[0].2["diagnosticId"].as_str().unwrap();
    assert!(is_hex_id(id));
    assert_eq!(seen[1].2["diagnosticId"], id);
    // Not the identifier of the sign-in, and the same one next time.
    assert!(!id.starts_with("astrosync-"));
    assert_eq!(diagnostic_id(&files.diagnostic_id).unwrap(), id);
    assert!(!files.queue.exists());
    assert_eq!(
        flush(&files, ON, Some("device-token"), &fake).unwrap(),
        Flushed::Nothing
    );
}

#[test]
fn a_report_not_delivered_waits_and_one_refused_for_good_is_dropped() {
    let dir = TempDir::new("error-report-delivery");
    let files = files_in(&dir);
    configure(&files, Some(HOSTED), Some(true));
    record(&files, ON, "1.0.0", panic_error("first")).unwrap();
    let fake = Fake::default();
    fake.answers.borrow_mut().push(Delivery::Retry);
    assert_eq!(
        flush(&files, ON, Some("tok"), &fake).unwrap(),
        Flushed::Done {
            left: 0,
            waiting: 1
        }
    );
    // One attempt only: no loop.
    assert_eq!(fake.seen.borrow().len(), 1);
    assert_eq!(queued(&files).len(), 1);
    fake.answers.borrow_mut().push(Delivery::Refused);
    assert_eq!(
        flush(&files, ON, Some("tok"), &fake).unwrap(),
        Flushed::Done {
            left: 1,
            waiting: 0
        }
    );
    assert!(!files.queue.exists());
}

#[test]
fn the_random_identifier_is_32_hex_characters_and_is_drawn_again_when_the_file_is_gone() {
    let dir = TempDir::new("error-report-id");
    let path = dir.path().join("diagnostic-id");
    let first = diagnostic_id(&path).unwrap();
    assert!(is_hex_id(&first));
    assert_eq!(diagnostic_id(&path).unwrap(), first);
    fs::write(&path, "not an identifier").unwrap();
    let second = diagnostic_id(&path).unwrap();
    assert!(is_hex_id(&second));
    assert_ne!(second, first);
    fs::remove_file(&path).unwrap();
    assert_ne!(diagnostic_id(&path).unwrap(), second);
}

#[test]
fn the_version_of_the_system_is_read_from_the_files_it_keeps() {
    let plist = "<dict>\n<key>ProductName</key>\n<string>macOS</string>\n\
                 <key>ProductVersion</key>\n\t<string>15.1</string>\n</dict>";
    assert_eq!(
        plist_value(plist, "ProductVersion").as_deref(),
        Some("15.1")
    );
    assert_eq!(plist_value(plist, "Missing"), None);
    let release = "NAME=\"Ubuntu\"\nVERSION_ID=\"22.04\"\n";
    assert_eq!(
        os_release_value(release, "VERSION_ID").as_deref(),
        Some("22.04")
    );
    assert!(is_version("15.1") && is_version("22.04") && is_version("1.0.0+91"));
    assert!(!is_version("") && !is_version("a b") && !is_version(&"1".repeat(41)));
    let system = System::current();
    assert!(["macos", "windows", "linux"].contains(&system.os.as_str()));
    assert!(system.os_version.as_deref().is_none_or(is_version));
}

/// The HTTP transport posts to the route of the service with the device
/// token, and maps the answers.
#[test]
fn the_http_transport_posts_the_batch_and_maps_the_answers() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let mut requests = Vec::new();
        for status in [
            "201 Created",
            "422 Unprocessable Entity",
            "429 Too Many Requests",
            "503 Service Unavailable",
        ] {
            let (mut stream, _) = listener.accept().unwrap();
            let mut received = Vec::new();
            let mut buffer = [0_u8; 4096];
            loop {
                let read = stream.read(&mut buffer).unwrap();
                received.extend_from_slice(&buffer[..read]);
                let text = String::from_utf8_lossy(&received).to_string();
                if let Some(split) = text.find("\r\n\r\n") {
                    let length = text[..split]
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length: ")
                                .map(|v| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if received.len() >= split + 4 + length {
                        break;
                    }
                }
                if read == 0 {
                    break;
                }
            }
            requests.push(String::from_utf8_lossy(&received).to_string());
            let answer =
                format!("HTTP/1.1 {status}\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{{}}");
            stream.write_all(answer.as_bytes()).unwrap();
        }
        requests
    });
    let transport = HttpErrorReportTransport::new().unwrap();
    let body = request_body("0123456789abcdef0123456789abcdef", &[]);
    let service = format!("http://{address}/");
    let answers: Vec<Delivery> = (0..4)
        .map(|_| transport.deliver(&service, "device-token", &body))
        .collect();
    assert_eq!(
        answers,
        [
            Delivery::Accepted,
            Delivery::Refused,
            Delivery::Retry,
            Delivery::Retry
        ]
    );
    let requests = server.join().unwrap();
    let first = requests[0].to_ascii_lowercase();
    assert!(
        first.starts_with("post /api/v1/diagnostics/errors http/1.1"),
        "{first}"
    );
    assert!(
        first.contains("authorization: bearer device-token"),
        "{first}"
    );
    assert!(first.contains("content-type: application/json"), "{first}");
    assert!(first.contains("\"application\":\"astro-sync\""), "{first}");
    assert!(
        first.contains(&format!(
            "x-planastro-client: {}",
            client_identity::value().to_ascii_lowercase()
        )),
        "{first}"
    );
}

#[test]
fn an_unreachable_service_is_a_later_not_a_failure() {
    let transport = HttpErrorReportTransport::new().unwrap();
    let body = request_body("0123456789abcdef0123456789abcdef", &[]);
    assert_eq!(
        transport.deliver("http://127.0.0.1:1", "tok", &body),
        Delivery::Retry
    );
}
