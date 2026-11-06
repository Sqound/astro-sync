//! `astro-sync sign-in`, the real binary: the sign-in of the hosted service by
//! e-mail address and code, with its real standard input, its real exit
//! codes and its real token file, against a fake service that replays the
//! exchanges the contract records
//! (`astro-sync-core/tests/support/contract_replay.rs`).
//!
//! What a person or a script depends on: the code is read from the keyboard
//! or the standard input and **never from an argument**; the command can be
//! used with no screen (the code is sent, then given on the standard input);
//! the token lands in a file only its owner can read, the very file
//! `upload --token` reads, and **never on an output**; every refusal of the
//! contract has a plain sentence and a non-zero exit code.

#[path = "../../astro-sync-core/tests/support/contract_replay.rs"]
mod contract_replay;
#[path = "../../astro-sync-core/tests/support/fake_service.rs"]
mod fake_service;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use contract_replay::Replay;
use fake_service::{FakeService, UploadModel, Wake, error_reply};

const TOKEN: &str = "QJR3D-0HcwGBdSXJ8IpOIuINTTIPAl2OHFyn1hIeyUQ";
const CODE: &str = "730418";

fn unique_temp_dir(label: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "astro-sync-cli-signin-bin-{label}-{}-{nanos}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Runs the binary with `stdin` on its standard input (which is never a
/// terminal here).
fn astro_sync(args: &[&str], stdin: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_astro-sync"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run the astro-sync binary");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Everything the command showed, on both outputs.
fn shown(output: &Output) -> String {
    format!("{}\n{}", text(&output.stdout), text(&output.stderr))
}

fn sign_in_args<'a>(portal: &'a str, token: &'a str) -> Vec<&'a str> {
    vec![
        "sign-in",
        "--portal",
        portal,
        "--token",
        token,
        "--email",
        "vous@exemple.fr",
        "--language",
        "fr",
        "--device-name",
        "Astro Sync \u{2014} MacBook Pro",
    ]
}

#[test]
fn without_a_screen_the_code_is_sent_then_given_on_the_standard_input_and_the_token_is_usable() {
    let dir = unique_temp_dir("headless");
    let token_file = dir.join("astro/token.txt");
    let token_arg = token_file.to_string_lossy().into_owned();
    let replay = Replay::start(&["request-code-address-with-space", "verify-code-astro-sync"]);
    let portal = replay.base_url();

    // 1. The code is sent; the command ends.
    let mut args = sign_in_args(&portal, &token_arg);
    args.push("--send-only");
    let sent = astro_sync(&args, "");
    assert!(sent.status.success(), "{}", shown(&sent));
    assert!(text(&sent.stdout).contains("Code sent to vous@exemple.fr"));
    assert!(!token_file.exists(), "nothing is signed in yet");

    // 2. The code, on the standard input -- never in an argument.
    let mut args = sign_in_args(&portal, &token_arg);
    args.push("--verify-only");
    let done = astro_sync(&args, &format!("{CODE}\n"));
    assert!(done.status.success(), "{}", shown(&done));
    let out = text(&done.stdout);
    assert!(
        out.contains("Signed in to the space vous@exemple.fr"),
        "{out}"
    );
    assert!(out.contains("Astro Sync \u{2014} MacBook Pro"), "{out}");
    replay.assert_played_exactly();

    // The token is in the file, and only there.
    assert_eq!(std::fs::read_to_string(&token_file).unwrap(), TOKEN);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&token_file).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "only its owner can read the token file");
    }
    for output in [&sent, &done] {
        let all = shown(output);
        assert!(!all.contains(TOKEN), "the token is on no output: {all}");
        assert!(!all.contains(CODE), "the code is never repeated: {all}");
    }
    // The installation identifier sits next to the token.
    assert!(dir.join("astro/installation-id").exists());
    // The requests carried the application `sync` (checked by the replay) and
    // this identifier.
    let sent_identifier = replay.requests_to("POST", "/api/v1/sign-in/verify")[0].json()["device"]
        ["installationId"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        std::fs::read_to_string(dir.join("astro/installation-id")).unwrap(),
        sent_identifier
    );

    // 3. The token is used exactly like the token of a pairing: `upload
    // --token` reads that very file.
    let model = Arc::new(UploadModel::default());
    let app_model = Arc::clone(&model);
    let service = FakeService::start(Wake::Instant, move |request| {
        app_model
            .reply(request, 100)
            .unwrap_or_else(|| error_reply(404, "not_found", "no such route"))
    });
    let folder = dir.join("night");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("frame.tif"), vec![5_u8; 250]).unwrap();
    let queue = dir.join("queue.sqlite");
    let uploaded = astro_sync(
        &[
            "upload",
            "--queue",
            &queue.to_string_lossy(),
            "--portal",
            &service.base_url(),
            "--token",
            &token_arg,
            "--folder",
            &folder.to_string_lossy(),
            "--json",
        ],
        "",
    );
    assert!(uploaded.status.success(), "{}", shown(&uploaded));
    assert_eq!(model.captures_created(), 1);
    let authorized = service
        .requests()
        .iter()
        .filter(|request| request.path.starts_with("/api/v1/uploads"))
        .all(|request| request.header("authorization") == Some(&format!("Bearer {TOKEN}")));
    assert!(authorized, "every call carries the token as a bearer");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_address_and_the_code_can_both_be_piped() {
    let dir = unique_temp_dir("piped");
    let token_file = dir.join("token.txt");
    let replay = Replay::start(&["request-code-address-with-space", "verify-code-astro-sync"]);
    let output = astro_sync(
        &[
            "sign-in",
            "--portal",
            &replay.base_url(),
            "--token",
            &token_file.to_string_lossy(),
            "--language",
            "fr",
            "--device-name",
            "Bureau",
            "--json",
        ],
        &format!("vous@exemple.fr\n{CODE}\n"),
    );
    assert!(output.status.success(), "{}", shown(&output));
    let result: serde_json::Value = serde_json::from_slice(&output.stdout)
        .expect("standard output is one JSON document, whatever is said on standard error");
    assert_eq!(result["space"]["email"], "vous@exemple.fr");
    assert!(!shown(&output).contains(TOKEN));
    assert_eq!(std::fs::read_to_string(&token_file).unwrap(), TOKEN);
    replay.assert_played_exactly();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Every refusal of the contract ends the command with a non-zero exit code
/// and a sentence a person can act on, and leaves no token behind.
#[test]
fn every_refusal_of_the_contract_fails_the_command_with_a_plain_sentence() {
    for (script, said) in [
        // The code.
        (
            vec!["verify-code-incorrect"],
            "This code does not match. Check the last message you received",
        ),
        (
            vec!["verify-code-expired"],
            "This code has expired. A code is good for 10 minutes",
        ),
        (
            vec!["verify-code-fifth-failure-locks"],
            "Too many tries. For your security, codes for this address cannot be entered for now",
        ),
        (
            vec!["verify-too-many-failures-from-origin"],
            "Try again in 25 minutes (from 2026-09-29 22:05 UTC).",
        ),
        // Compared five times: even the right code is void.
        (
            vec!["verify-code-compared-five-times"],
            "This code has expired. A code is good for 10 minutes",
        ),
        (vec!["verify-on-private-server"], "private server"),
        // The request of a code.
        (
            vec!["request-code-resend-too-soon"],
            "Try again in 42 seconds (from 2026-09-29 21:15 UTC).",
        ),
        (
            vec!["request-code-while-locked"],
            "You can ask for a new one in 15 minutes (from 2026-09-29 21:35 UTC).",
        ),
        (
            vec!["request-code-delivery-failed"],
            "The message could not be sent.",
        ),
        (
            vec!["request-code-not-configured"],
            "is not available on this service",
        ),
        // The e-mail budget of the whole service is spent: the sending of
        // codes is suspended, and the sentence says until when.
        (
            vec!["request-code-service-budget-spent"],
            "Sending codes is paused for the moment. You can try again in 15 minutes (from 2026-09-29 21:45 UTC).",
        ),
        // What the service announces.
        (vec!["private-server-announces-pairing"], "private server"),
        (vec!["service-without-private-api"], "accepts no sign-in"),
    ] {
        let dir = unique_temp_dir("refusal");
        let token_file = dir.join("token.txt");
        // A command that asks for a code first (the default) plays the
        // request, then the refusal; the announcements and the refused
        // requests end before.
        let ends_before_the_code = script[0].starts_with("request-code")
            || script[0].contains("announces")
            || script[0].starts_with("service-without");
        let mut played: Vec<&str> = Vec::new();
        if !ends_before_the_code {
            played.push("request-code-address-with-space");
        }
        played.extend(&script);
        let replay = Replay::start(&played);
        let output = astro_sync(
            &sign_in_args(&replay.base_url(), &token_file.to_string_lossy()),
            "481207\n",
        );
        assert!(
            !output.status.success(),
            "{script:?}: a refusal must be a non-zero exit code\n{}",
            shown(&output)
        );
        assert_eq!(output.status.code(), Some(1), "{script:?}");
        let stderr = text(&output.stderr);
        assert!(
            stderr.starts_with("A six-digit code") || stderr.contains(said),
            "{script:?}: {stderr}"
        );
        assert!(
            shown(&output).contains(said),
            "{script:?}: {}",
            shown(&output)
        );
        assert!(!token_file.exists(), "{script:?}: no token");
        assert!(
            !shown(&output).contains("481207"),
            "{script:?}: the code is not repeated"
        );
        replay.assert_played_exactly();
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// A zero-width space pasted along with an address (or any control or
/// invisible character): refused on the computer, before anything is sent,
/// with the sentence of an invalid address.
#[test]
fn an_address_with_an_invisible_character_fails_before_anything_is_sent() {
    let dir = unique_temp_dir("invisible");
    let replay = Replay::start(&[]);
    let token_file = dir.join("token.txt");
    let token_arg = token_file.to_string_lossy().into_owned();
    let portal = replay.base_url();
    let mut args = sign_in_args(&portal, &token_arg);
    let position = args
        .iter()
        .position(|arg| *arg == "vous@exemple.fr")
        .unwrap();
    args[position] = "vous\u{200B}@exemple.fr";
    let output = astro_sync(&args, "481207\n");
    assert_eq!(output.status.code(), Some(1), "{}", shown(&output));
    let stderr = text(&output.stderr);
    assert!(stderr.contains("enter a valid e-mail address"), "{stderr}");
    assert!(!token_file.exists());
    assert!(
        replay
            .requests_to("POST", "/api/v1/sign-in/code")
            .is_empty(),
        "no code was asked for"
    );
    replay.assert_played_exactly();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_code_cannot_be_given_as_an_argument() {
    let dir = unique_temp_dir("argument");
    let output = astro_sync(
        &[
            "sign-in",
            "--token",
            &dir.join("token.txt").to_string_lossy(),
            "--code",
            "481207",
        ],
        "",
    );
    assert!(!output.status.success());
    assert!(
        text(&output.stderr).contains("--code"),
        "{}",
        shown(&output)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn without_a_code_the_command_fails_and_says_where_it_goes() {
    let dir = unique_temp_dir("no-code");
    let replay = Replay::start(&["request-code-address-with-space"]);
    let output = astro_sync(
        &sign_in_args(&replay.base_url(), &dir.join("token.txt").to_string_lossy()),
        "",
    );
    assert!(!output.status.success());
    let stderr = text(&output.stderr);
    assert!(stderr.contains("standard input"), "{stderr}");
    assert!(stderr.contains("never an argument"), "{stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn upload_fails_and_keeps_the_queue_when_the_service_says_this_computer_was_detached() {
    let dir = unique_temp_dir("detached");
    let detached = contract_replay::exchange("call-from-detached-device");
    let service = FakeService::start(Wake::Instant, move |request| {
        if request.path == "/api/v1/service" {
            fake_service::identity_reply()
        } else {
            contract_replay::reply_of(&detached)
        }
    });
    let folder = dir.join("night");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("frame.tif"), vec![5_u8; 250]).unwrap();
    let token = dir.join("token.txt");
    std::fs::write(&token, TOKEN).unwrap();
    let queue = dir.join("queue.sqlite");

    let output = astro_sync(
        &[
            "upload",
            "--queue",
            &queue.to_string_lossy(),
            "--portal",
            &service.base_url(),
            "--token",
            &token.to_string_lossy(),
            "--folder",
            &folder.to_string_lossy(),
            "--json",
        ],
        "",
    );

    assert!(!output.status.success(), "{}", shown(&output));
    let stderr = text(&output.stderr);
    assert!(
        stderr.contains("detached from your Astro Portal space"),
        "{stderr}"
    );
    assert!(stderr.contains("astro-sync sign-in"), "{stderr}");
    let status = astro_sync(
        &[
            "queue",
            "status",
            "--queue",
            &queue.to_string_lossy(),
            "--json",
        ],
        "",
    );
    let entries: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(
        entries[0]["state"], "queued",
        "the file waits for the next sign-in: {entries}"
    );
    assert!(!shown(&output).contains(TOKEN));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = Path::new("");
}

/// A private server that signs in with a password shows a code on its account
/// page: `pair` exchanges it for the token file `upload --token` reads. The code
/// comes from the standard input, never from an argument; the token lands in a
/// file only its owner can read and on no output.
#[test]
fn a_private_server_code_is_exchanged_for_a_token_file_that_only_its_owner_can_read() {
    let dir = unique_temp_dir("pair");
    let claimed = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let seen = Arc::clone(&claimed);
    let service = FakeService::start(Wake::Instant, move |request| {
        if request.method == "POST" && request.path == "/api/v1/devices/claim" {
            seen.lock()
                .unwrap()
                .push(String::from_utf8_lossy(&request.body).into_owned());
            return fake_service::Reply::Json(200, format!(r#"{{"deviceToken":"{TOKEN}"}}"#));
        }
        fake_service::identity_reply()
    });
    let token = dir.join("device-token.txt");
    let output = astro_sync(
        &[
            "pair",
            "--portal",
            &service.base_url(),
            "--token",
            token.to_str().unwrap(),
            "--device-name",
            "Bureau",
        ],
        "abcd2345\n",
    );

    assert!(output.status.success(), "{}", shown(&output));
    assert_eq!(std::fs::read_to_string(&token).unwrap().trim(), TOKEN);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&token).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let said = shown(&output);
    assert!(!said.contains(TOKEN), "the token is on no output: {said}");
    assert!(said.contains("Bureau"), "{said}");
    let bodies = claimed.lock().unwrap().clone();
    assert_eq!(bodies.len(), 1, "the code is sent exactly once");
    assert!(
        bodies[0].contains("ABCD2345") && bodies[0].contains("Bureau"),
        "{}",
        bodies[0]
    );

    // A code that cannot be one fails before anything is sent.
    let bad = astro_sync(
        &[
            "pair",
            "--portal",
            &service.base_url(),
            "--token",
            dir.join("other.txt").to_str().unwrap(),
        ],
        "123\n",
    );
    assert!(!bad.status.success());
    assert!(!dir.join("other.txt").exists());
    assert_eq!(claimed.lock().unwrap().len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The code is asked for and never taken from an argument.
#[test]
fn a_pairing_code_cannot_be_given_as_an_argument() {
    let dir = unique_temp_dir("pair-argument");
    let output = astro_sync(
        &[
            "pair",
            "--portal",
            "http://127.0.0.1:9",
            "--token",
            dir.join("t.txt").to_str().unwrap(),
            "--code",
            "ABCD2345",
        ],
        "",
    );
    assert!(!output.status.success());
    assert!(!dir.join("t.txt").exists());
    let _ = std::fs::remove_dir_all(&dir);
}
