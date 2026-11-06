//! `sign-in` at the level of the command's own function: what is asked, what
//! is said, what is written, for every answer the contract records -- played
//! by the fake service that replays them
//! (`astro-sync-core/tests/support/contract_replay.rs`), with the waiting
//! policy shortened and what is typed scripted. The real binary, its exit
//! codes and its standard input are exercised by `tests/sign_in_cli.rs`.

use std::io::Cursor;
use std::sync::Mutex;
use std::time::Duration;

use super::contract_replay::{Replay, exchange};
use super::fake_service::{FakeService, Wake, identity_reply};
use super::*;

const TOKEN_OF_ASTRO_SYNC: &str = "QJR3D-0HcwGBdSXJ8IpOIuINTTIPAl2OHFyn1hIeyUQ";
const CODE_OF_ASTRO_SYNC: &str = "730418";

fn unique_temp_dir(label: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "astro-sync-cli-signin-{label}-{}-{nanos}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The same shape as production, on a scale of hundreds of milliseconds.
fn fast_policy() -> WakePolicy {
    WakePolicy {
        total_budget: Duration::from_millis(3_000),
        attempt_timeout: Duration::from_millis(500),
        write_timeout: Duration::from_millis(600),
        chunk_timeout: Duration::from_millis(1_500),
        retry_pause: Duration::from_millis(40),
        notice_after: Duration::from_millis(120),
        connect_timeout: Duration::from_millis(500),
    }
}

fn args_for(dir: &Path, portal: String) -> SignInArgs {
    SignInArgs {
        portal,
        token: dir.join("secrets/token.txt"),
        email: None,
        language: Some("fr".to_string()),
        device_name: Some("Astro Sync \u{2014} MacBook Pro".to_string()),
        installation_id: None,
        send_only: false,
        verify_only: false,
        json: false,
    }
}

fn upload_args_for(dir: &Path, portal: String) -> UploadArgs {
    let folder = dir.join("night");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("frame.tif"), vec![5_u8; 250]).unwrap();
    let token = dir.join("token.txt");
    std::fs::write(&token, TOKEN_OF_ASTRO_SYNC).unwrap();
    UploadArgs {
        queue: dir.join("queue.sqlite"),
        portal,
        token,
        folder: Some(folder),
        session_id: None,
        plan_id: None,
        target_ids: Vec::new(),
        json: true,
        language_of_messages: crate::cli::LangArg::default(),
    }
}

struct Session {
    said: Arc<Mutex<Vec<String>>>,
    out: Vec<u8>,
    err: Vec<u8>,
}

impl Session {
    fn stdout(&self) -> String {
        String::from_utf8_lossy(&self.out).into_owned()
    }

    fn stderr(&self) -> String {
        String::from_utf8_lossy(&self.err).into_owned()
    }

    /// Everything the person could read, anywhere.
    fn everything_shown(&self) -> String {
        format!(
            "{}\n{}\n{}",
            self.stdout(),
            self.stderr(),
            self.said.lock().unwrap().join("\n")
        )
    }
}

/// Runs `sign-in` as `typed` (what is on the keyboard or on standard input).
fn run(
    args: &SignInArgs,
    policy: WakePolicy,
    typed: &str,
    interactive: bool,
) -> (Result<(), String>, Session) {
    let said: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink = Arc::clone(&said);
    let mut input = Cursor::new(typed.as_bytes().to_vec());
    let mut session = Session {
        said,
        out: Vec::new(),
        err: Vec::new(),
    };
    let result = {
        let mut console = Console {
            input: &mut input,
            out: &mut session.out,
            err: &mut session.err,
            interactive,
        };
        sign_in_with(
            args,
            policy,
            Arc::new(move |line: &str| sink.lock().unwrap().push(line.to_string())),
            &mut console,
        )
    };
    (result.map_err(|error| error.to_string()), session)
}

#[test]
fn a_person_at_the_keyboard_signs_in_with_an_address_and_a_code() {
    let dir = unique_temp_dir("keyboard");
    let replay = Replay::start(&["request-code-address-with-space", "verify-code-astro-sync"]);
    let args = args_for(&dir, replay.base_url());

    let (result, session) = run(
        &args,
        fast_policy(),
        &format!("vous@exemple.fr\n{CODE_OF_ASTRO_SYNC}\n"),
        true,
    );

    result.expect("the sign-in succeeds");
    replay.assert_played_exactly();
    assert_eq!(
        std::fs::read_to_string(&args.token).unwrap(),
        TOKEN_OF_ASTRO_SYNC,
        "the token is in the token file, the very file `upload --token` reads"
    );
    // The identifier of the installation sits next to it, and is the same one
    // at the next sign-in.
    let installation = dir.join("secrets/installation-id");
    let first = std::fs::read_to_string(&installation).unwrap();
    assert!(first.starts_with("astrosync-"));
    let out = session.stdout();
    assert!(
        out.contains("Signed in to the space vous@exemple.fr (already existing)."),
        "{out}"
    );
    assert!(out.contains("Astro Sync \u{2014} MacBook Pro"), "{out}");
    assert!(out.contains("https://portal.example.test"), "{out}");
    let err = session.stderr();
    assert!(
        err.contains("E-mail address: ") && err.contains("Sign-in code: "),
        "{err}"
    );
    assert!(err.contains("vous@exemple.fr"), "{err}");
    assert!(
        err.contains("does not open Astro Sync") && err.contains("only shows the code"),
        "the screen says what the link of the message does on a computer: {err}"
    );
    // Neither the token nor the code is ever shown.
    let shown = session.everything_shown();
    assert!(
        !shown.contains(TOKEN_OF_ASTRO_SYNC) && !shown.contains(CODE_OF_ASTRO_SYNC),
        "{shown}"
    );
    // A second sign-in keeps the identifier of the installation.
    let replay = Replay::start(&["request-code-address-with-space", "verify-code-astro-sync"]);
    let args = SignInArgs {
        portal: replay.base_url(),
        email: Some("vous@exemple.fr".to_string()),
        ..args
    };
    run(
        &args,
        fast_policy(),
        &format!("{CODE_OF_ASTRO_SYNC}\n"),
        true,
    )
    .0
    .unwrap();
    assert_eq!(std::fs::read_to_string(&installation).unwrap(), first);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_json_result_never_holds_the_token() {
    let dir = unique_temp_dir("json");
    let replay = Replay::start(&["request-code-address-with-space", "verify-code-astro-sync"]);
    let args = SignInArgs {
        email: Some("vous@exemple.fr".to_string()),
        json: true,
        ..args_for(&dir, replay.base_url())
    };

    let (result, session) = run(
        &args,
        fast_policy(),
        &format!("{CODE_OF_ASTRO_SYNC}\n"),
        false,
    );

    result.unwrap();
    let result: serde_json::Value =
        serde_json::from_str(&session.stdout()).expect("standard output is one JSON document");
    assert_eq!(result["created"], false);
    assert_eq!(result["space"]["email"], "vous@exemple.fr");
    assert_eq!(result["device"]["name"], "Astro Sync \u{2014} MacBook Pro");
    assert_eq!(result["tokenFile"], args.token.to_string_lossy().as_ref());
    assert!(
        !session.everything_shown().contains(TOKEN_OF_ASTRO_SYNC),
        "the token never appears on an output"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_wrong_code_is_asked_for_again_at_the_keyboard_and_a_failure_anywhere_else() {
    // At a keyboard: the code stays valid, the person has a few tries.
    let dir = unique_temp_dir("wrong-code-keyboard");
    let replay = Replay::start(&[
        "request-code-address-with-space",
        "verify-code-incorrect",
        "verify-code-astro-sync",
    ]);
    let args = args_for(&dir, replay.base_url());
    let (result, session) = run(
        &args,
        fast_policy(),
        &format!("vous@exemple.fr\n481270\n{CODE_OF_ASTRO_SYNC}\n"),
        true,
    );
    result.expect("asked again, then accepted");
    assert!(
        session.stderr().contains("You have 4 tries left."),
        "{}",
        session.stderr()
    );
    replay.assert_played_exactly();
    let _ = std::fs::remove_dir_all(&dir);

    // Anywhere else (a pipe, a script): a failure, and no token written.
    let dir = unique_temp_dir("wrong-code-pipe");
    let replay = Replay::start(&["request-code-address-with-space", "verify-code-incorrect"]);
    let args = SignInArgs {
        email: Some("vous@exemple.fr".to_string()),
        ..args_for(&dir, replay.base_url())
    };
    let (result, _session) = run(&args, fast_policy(), "481270\n", false);
    let error = result.unwrap_err();
    assert!(error.contains("This code does not match"), "{error}");
    assert!(error.contains("You have 4 tries left."), "{error}");
    assert!(!args.token.exists(), "no token without a verified code");
    replay.assert_played_exactly();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Each refusal of the contract has a plain sentence, ends the command with
/// an error, and leaves no token behind.
#[test]
fn every_refusal_of_the_contract_is_said_plainly_and_fails_the_command() {
    for (script, said) in [
        (
            ["request-code-address-with-space", "verify-code-expired"],
            "This code has expired",
        ),
        (
            [
                "request-code-address-with-space",
                "verify-code-already-used",
            ],
            "This code has expired",
        ),
        (
            [
                "request-code-address-with-space",
                "verify-code-fifth-failure-locks",
            ],
            "Too many tries. For your security, codes for this address cannot be entered for now. You can ask for a new one in 15 minutes (from 2026-09-29 21:35 UTC).",
        ),
        (
            ["request-code-address-with-space", "verify-while-locked"],
            "in 10 minutes",
        ),
        (
            [
                "request-code-address-with-space",
                "verify-too-many-failures-from-origin",
            ],
            "Too many codes requested, or too quickly. Try again in 25 minutes (from 2026-09-29 22:05 UTC).",
        ),
        (
            [
                "request-code-address-with-space",
                "verify-on-private-server",
            ],
            "private server",
        ),
    ] {
        let dir = unique_temp_dir("refusal");
        let replay = Replay::start(&script);
        let args = SignInArgs {
            email: Some("vous@exemple.fr".to_string()),
            ..args_for(&dir, replay.base_url())
        };
        let (result, session) = run(&args, fast_policy(), "481207\n", false);
        let error = result.unwrap_err();
        assert!(error.contains(said), "{script:?}: {error}");
        assert!(!args.token.exists(), "{script:?}: no token");
        assert!(
            !session.everything_shown().contains("481207") && !error.contains("481207"),
            "the code is never repeated"
        );
        replay.assert_played_exactly();
        let _ = std::fs::remove_dir_all(&dir);
    }

    // The refusals of the request of a code stop before any code is asked.
    for (script, said) in [
        (
            ["request-code-resend-too-soon"],
            "Too many codes requested, or too quickly. Try again in 42 seconds (from 2026-09-29 21:15 UTC).",
        ),
        (
            ["request-code-while-locked"],
            "You can ask for a new one in 15 minutes (from 2026-09-29 21:35 UTC).",
        ),
        (
            ["request-code-too-many-for-address"],
            "Try again in 24 minutes (from 2026-09-29 22:14 UTC).",
        ),
        (
            ["request-code-delivery-failed"],
            "The message could not be sent.",
        ),
        (
            ["request-code-not-configured"],
            "Signing in by e-mail is not available on this service right now.",
        ),
        (
            ["request-code-invalid-address"],
            "Enter a valid e-mail address.",
        ),
        (["request-code-on-private-server"], "private server"),
    ] {
        let dir = unique_temp_dir("request-refusal");
        let replay = Replay::start(&script);
        let args = SignInArgs {
            email: Some("vous@exemple.fr".to_string()),
            ..args_for(&dir, replay.base_url())
        };
        let (result, _session) = run(&args, fast_policy(), "481207\n", false);
        let error = result.unwrap_err();
        assert!(error.contains(said), "{script:?}: {error}");
        assert!(!args.token.exists(), "{script:?}: no token");
        assert_eq!(
            replay.requests_to("POST", "/api/v1/sign-in/verify").len(),
            0,
            "{script:?}: no code is presented after a refused request"
        );
        replay.assert_played_exactly();
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn what_is_not_an_address_or_a_code_never_leaves_the_computer() {
    let dir = unique_temp_dir("local-checks");
    let replay = Replay::start(&[]);
    let args = SignInArgs {
        email: Some("vous-exemple.fr".to_string()),
        ..args_for(&dir, replay.base_url())
    };
    let error = run(&args, fast_policy(), "\n", false).0.unwrap_err();
    assert!(error.contains("valid e-mail address"), "{error}");

    let args = SignInArgs {
        email: Some("vous@exemple.fr".to_string()),
        verify_only: true,
        ..args_for(&dir, replay.base_url())
    };
    let error = run(&args, fast_policy(), "12345\n", false).0.unwrap_err();
    assert!(error.contains("six digits"), "{error}");
    let posts = replay
        .service()
        .requests()
        .into_iter()
        .filter(|request| request.method == "POST")
        .count();
    assert_eq!(posts, 0, "nothing was sent");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn without_a_screen_the_code_is_sent_then_given_on_the_standard_input() {
    let dir = unique_temp_dir("headless");
    let replay = Replay::start(&[
        "request-code-address-with-space",
        // Later, from another run: the code alone.
        "verify-code-astro-sync",
    ]);
    let send = SignInArgs {
        email: Some("vous@exemple.fr".to_string()),
        send_only: true,
        json: true,
        ..args_for(&dir, replay.base_url())
    };
    let (result, session) = run(&send, fast_policy(), "", false);
    result.expect("the code is sent and the command ends");
    let sent: serde_json::Value = serde_json::from_str(&session.stdout()).unwrap();
    assert_eq!(sent["email"], "vous@exemple.fr");
    assert_eq!(sent["resendAfterSeconds"], 60);
    assert!(!send.token.exists(), "nothing is signed in yet");

    let verify = SignInArgs {
        email: Some("vous@exemple.fr".to_string()),
        verify_only: true,
        ..args_for(&dir, replay.base_url())
    };
    let (result, session) = run(
        &verify,
        fast_policy(),
        &format!("{CODE_OF_ASTRO_SYNC}\n"),
        false,
    );
    result.expect("the code given on the standard input is verified");
    assert_eq!(
        std::fs::read_to_string(&verify.token).unwrap(),
        TOKEN_OF_ASTRO_SYNC
    );
    assert!(
        session.stdout().contains("Signed in to the space"),
        "{}",
        session.stdout()
    );
    replay.assert_played_exactly();
    assert_eq!(
        replay.requests_to("POST", "/api/v1/sign-in/code").len(),
        1,
        "`--verify-only` asks for no second message"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_address_and_the_code_can_both_come_from_the_standard_input() {
    let dir = unique_temp_dir("piped");
    let replay = Replay::start(&["request-code-address-with-space", "verify-code-astro-sync"]);
    let args = args_for(&dir, replay.base_url());
    let (result, _session) = run(
        &args,
        fast_policy(),
        &format!("vous@exemple.fr\n{CODE_OF_ASTRO_SYNC}\n"),
        false,
    );
    result.unwrap();
    replay.assert_played_exactly();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_missing_code_is_an_error_that_says_where_the_code_goes() {
    let dir = unique_temp_dir("no-code");
    let replay = Replay::start(&["request-code-address-with-space"]);
    let args = SignInArgs {
        email: Some("vous@exemple.fr".to_string()),
        ..args_for(&dir, replay.base_url())
    };
    let error = run(&args, fast_policy(), "", false).0.unwrap_err();
    assert!(error.contains("standard input"), "{error}");
    assert!(error.contains("never an argument"), "{error}");
    assert!(!args.token.exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_private_server_is_pointed_to_its_pairing_and_nothing_is_asked() {
    for (announcement, said) in [
        ("private-server-announces-pairing", "private server"),
        ("older-private-server-announces-nothing", "private server"),
        ("service-without-private-api", "accepts no sign-in"),
    ] {
        let dir = unique_temp_dir("private");
        let replay = Replay::start(&[announcement]);
        let args = SignInArgs {
            email: Some("vous@exemple.fr".to_string()),
            ..args_for(&dir, replay.base_url())
        };
        let (result, session) = run(&args, fast_policy(), "", true);
        let error = result.unwrap_err();
        assert!(error.contains(said), "{announcement}: {error}");
        assert!(
            !session.stderr().contains("Adresse e-mail"),
            "an address is not asked of a private server"
        );
        assert_eq!(
            replay.requests_to("POST", "/api/v1/sign-in/code").len(),
            0,
            "{announcement}: no code is asked for"
        );
        assert!(!args.token.exists());
        replay.assert_played_exactly();
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn a_service_that_is_starting_is_waited_for_and_says_so() {
    let dir = unique_temp_dir("starting");
    let replay = Replay::start_with(
        &["request-code-address-with-space", "verify-code-astro-sync"],
        Wake::Held(Duration::from_millis(1_200)),
        "hosted-service-announces-email-code",
    );
    let args = SignInArgs {
        email: Some("vous@exemple.fr".to_string()),
        ..args_for(&dir, replay.base_url())
    };

    let (result, session) = run(
        &args,
        fast_policy(),
        &format!("{CODE_OF_ASTRO_SYNC}\n"),
        false,
    );

    result.unwrap();
    assert_eq!(
        *session.said.lock().unwrap(),
        [
            SERVICE_STARTING.to_string(),
            SIGN_IN_SERVICE_READY.to_string()
        ]
    );
    assert!(
        !session.stdout().contains(SERVICE_STARTING),
        "standard output stays a clean result"
    );
    replay.assert_played_exactly();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_service_that_cannot_be_reached_ends_the_command_at_once() {
    let dir = unique_temp_dir("unreachable");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let args = SignInArgs {
        email: Some("vous@exemple.fr".to_string()),
        ..args_for(&dir, format!("http://127.0.0.1:{port}"))
    };
    let started = std::time::Instant::now();
    let error = run(&args, fast_policy(), "481207\n", false).0.unwrap_err();
    assert!(
        error.contains("Could not reach the service")
            || error.contains("could not reach the service"),
        "{error}"
    );
    assert!(started.elapsed() < Duration::from_millis(1_500));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn upload_stops_and_fails_when_the_service_says_this_computer_was_detached() {
    // The service says so, whatever it is asked. The file is not at fault:
    // it stays queued, and the command fails so a script knows.
    let dir = unique_temp_dir("upload-detached");
    let detached = exchange("call-from-detached-device");
    let service = FakeService::start(Wake::Instant, move |request| {
        if request.path == "/api/v1/service" {
            identity_reply()
        } else {
            super::contract_replay::reply_of(&detached)
        }
    });
    let args = upload_args_for(&dir, service.base_url());
    let said: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink = Arc::clone(&said);

    let error = upload_with(
        &args,
        fast_policy(),
        DriveOptions::command_line(),
        "en",
        Arc::new(move |line: &str| sink.lock().unwrap().push(line.to_string())),
    )
    .unwrap_err()
    .to_string();

    assert!(
        error.contains("detached from your Astro Portal space"),
        "{error}"
    );
    assert!(error.contains("astro-sync sign-in"), "{error}");
    assert!(error.contains("are kept"), "{error}");
    let queue = UploadQueue::open(&args.queue).unwrap();
    assert_eq!(
        queue.list().unwrap()[0].state,
        astro_sync_core::upload_queue::QueueState::Queued,
        "a sign-in that ends loses no pending upload"
    );
    assert_eq!(
        service.app_hits("POST", "/api/v1/uploads"),
        1,
        "and it does not insist"
    );
    let _ = std::fs::remove_dir_all(&dir);

    // A deleted space says the same, in its own words.
    let deleted = describe_sign_in_ended(SignInEnd::SpaceDeleted, "en");
    assert!(deleted.contains("was deleted"), "{deleted}");
}

/// A newer sign-in of the same installation replaced the token: the old one
/// answers `401 device_authentication_required`. The command fails and says to
/// sign in again; the file stays queued.
#[test]
fn upload_fails_and_keeps_the_queue_when_the_token_was_replaced() {
    let dir = unique_temp_dir("upload-replaced");
    let service = FakeService::start(Wake::Instant, |request| {
        if request.path == "/api/v1/service" {
            identity_reply()
        } else {
            super::fake_service::error_reply(
                401,
                "device_authentication_required",
                "Device authentication is required.",
            )
        }
    });
    let args = upload_args_for(&dir, service.base_url());
    let error = upload_with(
        &args,
        fast_policy(),
        DriveOptions::command_line(),
        "en",
        Arc::new(|_: &str| {}),
    )
    .unwrap_err()
    .to_string();

    assert!(error.contains("no longer recognises"), "{error}");
    assert!(error.contains("astro-sync sign-in"), "{error}");
    assert!(error.contains("are kept"), "{error}");
    assert!(!error.contains("device_authentication_required"), "{error}");
    let queue = UploadQueue::open(&args.queue).unwrap();
    assert_eq!(
        queue.list().unwrap()[0].state,
        astro_sync_core::upload_queue::QueueState::Queued
    );
    assert_eq!(service.app_hits("POST", "/api/v1/uploads"), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

/// `403` `space_unavailable`: the code is right, the space is gone. Nothing
/// to retry: even at a keyboard the command does not ask for the code again.
/// A refusal nobody knows yet is the generic sentence, never a raw error.
#[test]
fn a_space_that_is_gone_and_a_refusal_nobody_knows_are_said_plainly_and_not_retried() {
    for (code, said, absent) in [
        ("space_unavailable", "is no longer available", "Try again"),
        (
            "a_refusal_from_the_future",
            "update Astro Sync",
            "a_refusal_from_the_future",
        ),
    ] {
        let dir = unique_temp_dir("space-unavailable");
        let service = FakeService::start(Wake::Instant, move |request| {
            if request.method == "GET" {
                super::contract_replay::reply_of(&exchange("hosted-service-announces-email-code"))
            } else if request.path == "/api/v1/sign-in/code" {
                super::contract_replay::reply_of(&exchange("request-code-address-with-space"))
            } else {
                super::fake_service::error_reply(403, code, "The service's own English text.")
            }
        });
        let args = SignInArgs {
            email: Some("vous@exemple.fr".to_string()),
            ..args_for(&dir, service.base_url())
        };

        // At a keyboard, with two codes ready to be typed.
        let (result, session) = run(&args, fast_policy(), "481207\n999999\n", true);

        let error = result.unwrap_err();
        assert!(error.contains(said), "{code}: {error}");
        assert!(
            !error.contains(absent) && !error.contains("English"),
            "{code}: {error}"
        );
        assert_eq!(
            service.raw_hits("POST", "/api/v1/sign-in/verify"),
            1,
            "{code}: the second code is never presented"
        );
        assert!(!args.token.exists(), "{code}: no token");
        assert!(
            !session.everything_shown().contains("English"),
            "{code}: never the service's own text"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
