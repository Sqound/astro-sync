//! Smoke test of the **real** credential store of the system Astro Sync runs
//! on -- the macOS Keychain, the Windows Credential Manager, the Linux Secret
//! Service -- through the same public functions the application calls to keep
//! the device token (`save_device_token`, `load_device_token`,
//! `clear_device_token`) and through the `keyring` crate itself.
//!
//! Every test is `#[ignore]`d: `cargo test` never touches a real store (a
//! locked Keychain would show a prompt, a bare Linux session has no Secret
//! Service), and `README.md` says so. They are run on purpose, on
//! each system, by `tool/keyring-smoke.sh` (the release workflow
//! runs it on its three runners):
//!
//! ```sh
//! cargo test -p astro-sync-core --test keyring_smoke -- --ignored --test-threads=1
//! ```
//!
//! What a green run proves: the store accepts a write, returns the same bytes,
//! keeps two portals apart, forgets on delete, and **the token did not end up
//! in the fallback file** (a store that refuses silently makes the application
//! fall back to a file, which would hide the failure; the tests look at the
//! file). What it cannot prove: that a *signed* build of the application keeps
//! its access to an entry across builds (the macOS Keychain ties an entry to
//! the code signature), or that a person's own session shows no prompt.
//!
//! The tests use a made-up account per run (`smoke-test.invalid-<pid>-<n>`,
//! never a real portal URL) under the application's real service name, and
//! delete what they create, even when an assertion fails.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use astro_sync_core::gui_bridge::{
    DEVICE_TOKEN_KEYRING_SERVICE, clear_device_token, load_device_token, save_device_token,
};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A made-up portal URL, unique to this run and this test.
fn account(label: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    format!(
        "https://smoke-test.invalid/{label}-{}-{nanos}-{n}",
        std::process::id()
    )
}

/// A folder of its own for the fallback file, removed at the end.
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "astro-sync-keyring-smoke-{label}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&dir).unwrap();
        Self { dir }
    }

    fn token_file(&self) -> PathBuf {
        self.dir.join("device-token")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// Deletes the credential of an account when it goes out of scope.
struct Cleanup(String);

impl Drop for Cleanup {
    fn drop(&mut self) {
        if let Ok(entry) = keyring::Entry::new(DEVICE_TOKEN_KEYRING_SERVICE, &self.0) {
            let _ = entry.delete_credential();
        }
    }
}

#[test]
#[ignore = "touches the real credential store of this system; run by tool/keyring-smoke.sh"]
fn the_keyring_crate_stores_reads_and_deletes_a_token_in_the_real_store() {
    let account = account("crate");
    let _cleanup = Cleanup(account.clone());
    let entry = keyring::Entry::new(DEVICE_TOKEN_KEYRING_SERVICE, &account).unwrap();

    assert!(
        matches!(entry.get_password(), Err(keyring::Error::NoEntry)),
        "a fresh account must read as « no entry »"
    );
    entry.set_password("smoke-token-1").unwrap();
    assert_eq!(entry.get_password().unwrap(), "smoke-token-1");

    // A second `Entry` for the same account sees it: the store persists, it is
    // not the in-memory mock.
    let again = keyring::Entry::new(DEVICE_TOKEN_KEYRING_SERVICE, &account).unwrap();
    assert_eq!(again.get_password().unwrap(), "smoke-token-1");

    again.delete_credential().unwrap();
    assert!(matches!(entry.get_password(), Err(keyring::Error::NoEntry)));
}

#[test]
#[ignore = "touches the real credential store of this system; run by tool/keyring-smoke.sh"]
fn save_load_and_clear_use_the_store_and_never_the_fallback_file() {
    let account = account("bridge");
    let _cleanup = Cleanup(account.clone());
    let scratch = Scratch::new("bridge");
    let file = scratch.token_file();

    assert_eq!(load_device_token(&file, &account).unwrap(), None);

    save_device_token(&file, &account, "smoke-token-2").unwrap();
    assert!(
        !file.exists(),
        "the token went to the fallback file: the real store refused the write"
    );
    assert_eq!(
        load_device_token(&file, &account).unwrap().as_deref(),
        Some("smoke-token-2")
    );
    // The store holds it, not only the application's functions.
    let entry = keyring::Entry::new(DEVICE_TOKEN_KEYRING_SERVICE, &account).unwrap();
    assert_eq!(entry.get_password().unwrap(), "smoke-token-2");

    // Replacing a token replaces it (a re-attached computer).
    save_device_token(&file, &account, "smoke-token-3").unwrap();
    assert_eq!(
        load_device_token(&file, &account).unwrap().as_deref(),
        Some("smoke-token-3")
    );

    clear_device_token(&file, Some(&account)).unwrap();
    assert_eq!(load_device_token(&file, &account).unwrap(), None);
    assert!(matches!(entry.get_password(), Err(keyring::Error::NoEntry)));
    // Clearing what is not there is not an error.
    clear_device_token(&file, Some(&account)).unwrap();
}

#[test]
#[ignore = "touches the real credential store of this system; run by tool/keyring-smoke.sh"]
fn two_portals_keep_two_separate_credentials() {
    let first = account("first");
    let second = account("second");
    let _cleanup_first = Cleanup(first.clone());
    let _cleanup_second = Cleanup(second.clone());
    let scratch = Scratch::new("two");
    let file = scratch.token_file();

    save_device_token(&file, &first, "token-of-the-first").unwrap();
    save_device_token(&file, &second, "token-of-the-second").unwrap();
    assert!(!file.exists());
    assert_eq!(
        load_device_token(&file, &first).unwrap().as_deref(),
        Some("token-of-the-first")
    );
    assert_eq!(
        load_device_token(&file, &second).unwrap().as_deref(),
        Some("token-of-the-second")
    );

    clear_device_token(&file, Some(&first)).unwrap();
    assert_eq!(load_device_token(&file, &first).unwrap(), None);
    assert_eq!(
        load_device_token(&file, &second).unwrap().as_deref(),
        Some("token-of-the-second"),
        "detaching from one portal must leave the other one attached"
    );
}

#[test]
#[ignore = "touches the real credential store of this system; run by tool/keyring-smoke.sh"]
fn a_token_left_in_the_legacy_file_moves_into_the_store_on_the_first_read() {
    let account = account("migrate");
    let _cleanup = Cleanup(account.clone());
    let scratch = Scratch::new("migrate");
    let file = scratch.token_file();
    fs::write(&file, "smoke-legacy-token\n").unwrap();

    assert_eq!(
        load_device_token(&file, &account).unwrap().as_deref(),
        Some("smoke-legacy-token")
    );
    assert!(
        !file.exists(),
        "the legacy file must be removed once the token is in the store"
    );
    let entry = keyring::Entry::new(DEVICE_TOKEN_KEYRING_SERVICE, &account).unwrap();
    assert_eq!(entry.get_password().unwrap(), "smoke-legacy-token");
}
