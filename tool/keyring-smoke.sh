#!/usr/bin/env bash
# Runs the smoke test of the real credential store of this system
# (`crates/astro-sync-core/tests/keyring_smoke.rs`: store, read, replace and
# delete a made-up token, then check the fallback file was never used).
#
#   bash tool/keyring-smoke.sh
#
# macOS (Keychain) and Windows (Credential Manager, from Git Bash) run it as
# is. Linux needs a Secret Service on a session bus: when there is none (a
# container, a CI runner), this script starts a private session bus and a
# throw-away GNOME Keyring with a made-up password, so nothing of the person's
# own keyring is ever touched.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$here/.."

# On a fresh private bus the keyring needs a moment before it answers: the first
# access (the creation of its default collection, the D-Bus activation of the
# service next to the daemon just started) can fail with "Message recipient
# disconnected from message bus without replying", which is the daemon being
# replaced under the caller, not a defect of the store (seen on a cold run, on
# arm64 and on emulated x64). Wait until the Secret Service answers twice in a
# row, a second apart, right before the tests start. The tests themselves are
# never retried: a failure after this point is real.
wait_for_secret_service() {
  command -v secret-tool >/dev/null 2>&1 || return 0
  local ok=0 attempt
  for attempt in $(seq 1 30); do
    if printf '%s' 'probe-not-a-secret' |
        secret-tool store --label='astro-sync smoke probe' astro-sync-smoke probe >/dev/null 2>&1 &&
      [ "$(secret-tool lookup astro-sync-smoke probe 2>/dev/null)" = 'probe-not-a-secret' ] &&
      secret-tool clear astro-sync-smoke probe >/dev/null 2>&1; then
      ok=$((ok + 1))
      [ "$ok" -ge 2 ] && return 0
    else
      ok=0
    fi
    sleep 1
  done
  echo "keyring-smoke: the Secret Service did not answer within 30 s" >&2
  return 1
}

run_smoke() {
  cargo test -p astro-sync-core --test keyring_smoke -- --ignored --test-threads=1 "$@"
}

if [ "$(uname -s)" = "Linux" ] && [ -z "${ASTRO_SYNC_SMOKE_SESSION:-}" ]; then
  if [ -n "${DBUS_SESSION_BUS_ADDRESS:-}" ] && [ -n "${ASTRO_SYNC_SMOKE_USE_CURRENT_SESSION:-}" ]; then
    : # the caller asked for its own session (a real desktop): use it as is
  else
    export ASTRO_SYNC_SMOKE_SESSION=1
    exec dbus-run-session -- bash "$here/$(basename "${BASH_SOURCE[0]}")" "$@"
  fi
fi

if [ "$(uname -s)" = "Linux" ] && [ -n "${ASTRO_SYNC_SMOKE_SESSION:-}" ]; then
  # A private, throw-away keyring on the private bus: the password is made up
  # and protects nothing.
  # Compile the tests first: a cold build takes minutes (an emulated x64 one,
  # three), and a keyring daemon left idle that long is not the one found again
  # when the tests finally start.
  cargo test -p astro-sync-core --test keyring_smoke --no-run
  export XDG_DATA_HOME="$(mktemp -d)"
  printf '%s' 'astro-sync-smoke-not-a-secret' | gnome-keyring-daemon --unlock --components=secrets >/dev/null
  gnome-keyring-daemon --start --components=secrets >/dev/null
  wait_for_secret_service
fi

run_smoke "$@"
