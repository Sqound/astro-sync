# Open session package

The public, versioned contract for an `astro-session-package/` directory. Astro
Sync is only one producer: a personal script or another astronomy application
may write this exact layout by hand and have the portal import it directly.
That is the point of the contract -- it, not any particular tool, is the
interface.

## Layout

```text
astro-session-package/
  manifest.json           required -- see manifest.schema.json
  checksums.sha256        required -- sha256sum format, one line per asset
  images/                 optional
  metrics/quality.csv     optional
  metrics/guiding.csv     optional
  events/session.jsonl    optional
  progressions/           optional
```

**The minimal package is a versioned `manifest.json` plus one asset it
lists.** `checksums.sha256` is still required at that size (one line, for
the one asset); every other directory above is optional and only present
when the producer actually generated that kind of data.

## `manifest.json`

Validated by [`manifest.schema.json`](./manifest.schema.json). Top-level
shape:

- `packageVersion` (required) -- the manifest contract's own version. This
  schema accepts exactly `1`.
- `generatedAt` (required) -- ISO 8601 UTC timestamp of when the package was
  produced.
- `assets` (required, at least one) -- every file the package carries,
  besides `manifest.json` and `checksums.sha256` themselves. Each entry
  names a package-root-relative `path`, a `kind`
  (`image` / `metric-log` / `event-log` / `progression` / `other`), the
  exact `bytes` size, an `addedAt` timestamp, and an optional nullable
  `capturedAt` timestamp and `mimeType`.
- `session` (optional) -- a **descriptive-only** label and start/finish
  timestamps for the observation night. It deliberately has no `id` field:
  see "Never trust a client-supplied tenant id" below.
- `producer` (optional) -- free-form `{ name, version }` identifying the
  tool that wrote the package, for troubleshooting only.

## `checksums.sha256`

Plain [`sha256sum`](https://www.gnu.org/software/coreutils/manual/html_node/sha2-utilities.html)
output format, UTF-8, LF line endings:

```text
<hex digest>  <relative path>
<hex digest>  <relative path>
```

Every path in `manifest.json`'s `assets[]` must have exactly one matching
line here, and the two spaces between the digest and the path are
significant (binary-mode marker). This file, not the manifest, is the single
source of truth for asset hashes -- the manifest never repeats a checksum
field, so there is exactly one place a hash can drift from the bytes on
disk.

## `metrics/` and `events/`

`metrics/quality.csv`, `metrics/guiding.csv`, and `events/session.jsonl`
are produced by local log/FITS analysis. The exact column specification, units, timestamp precision, and
JSON Lines event vocabulary are in [`metrics.md`](./metrics.md) --
including the evidence rule every value in either file must satisfy: it
traces to a value a producer actually parsed, never a synthesized or
composite rating.

## Hard rules for importers

An importer (the portal's `POST /v1/sessions/{id}/imports/preview` and
`.../imports/confirm`; or any other tool reading this contract)
**must** enforce all of the following. The schema's own `relativePath`
pattern is a first line of defense, not a substitute for the importer's own
checks -- a schema-valid manifest can still be handed to code that resolves
paths carelessly.

1. **Relative paths only.** Every `assets[].path` is relative to the package
   root. `manifest.schema.json`'s `relativePath` pattern rejects a leading
   `/`, a drive letter (`C:`), and any `.` or `..` path segment at the JSON
   Schema level. The importer independently resolves each path against the
   extraction root and asserts the result is still inside it before opening
   any file -- string-pattern rejection and path-resolution rejection are
   two separate checks, and both are mandatory.
2. **SHA-256 verification is mandatory.** After extraction, the importer
   recomputes the SHA-256 of every file listed in `checksums.sha256` and
   rejects the whole package on any mismatch, any asset missing its file, or
   any asset missing its checksum line. A package is imported atomically:
   verify everything first, then persist, never partially.
3. **Idempotency is keyed on the manifest.** The importer computes a
   fingerprint of the package (e.g. the SHA-256 of `manifest.json`'s exact
   bytes) and treats a repeat import carrying the same fingerprint as a
   no-op returning the already-imported result, never a duplicate. Retrying
   the same package and checksums after a network failure must be safe.
4. **Never trust a client-supplied tenant id.** `manifest.json` carries no
   session id, account id, or tenant/organization id, and the schema has no
   room for one. The session (and therefore the organization) an import
   affects is always the one named by the authenticated import request
   itself, resolved from the caller's membership -- never a value read out
   of the package. `session.label`/`.startedAt`/`.finishedAt` are display
   metadata only.
5. **Preview before confirm, and preview persists nothing.** `imports/preview`
   validates schema, checksums, and path safety, and returns exactly what
   would be imported (recognized data, warnings, visibility, transfer size)
   without writing anything. Only `imports/confirm` persists.

## Examples

[`examples/manifest.minimal.json`](./examples/manifest.minimal.json) is a
schema-valid minimal manifest (one image asset).
[`examples/manifest.invalid-path.json`](./examples/manifest.invalid-path.json)
is the same manifest with a `../../etc/passwd`-style traversal path, rejected
by the `relativePath` pattern.
[`examples/manifest.cli-produced.json`](./examples/manifest.cli-produced.json)
is a real `manifest.json` recorded from `astro-sync package`
(`astro-sync/`) against a neutral fixture folder -- proof that the
Rust producer's actual output, not only hand-written examples, satisfies
this schema. All three are exercised by
`packages/astro-contracts/tests/session-package.test.mjs`.
