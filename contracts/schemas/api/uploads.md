# Astro Portal resumable uploads

This is the resumable replacement transport `api/v1.md`'s "Uploads and captures"
section already promises: the original, non-resumable `multipart/form-data`
`POST /v1/uploads` remains — a thin, single-request form that
suits a small file from the mobile app — but a large transfer from Astro
Sync or the CLI now has a protocol that survives a
dropped connection without re-sending bytes already durably received.

The **capture** resource this protocol eventually produces is unchanged —
[`capture.schema.json`](capture.schema.json)'s `capture` def, content-addressed
by `sha256`, private to its tenant, shown with `targetIds`. Everything below
is about how the bytes get there.

```text
POST /v1/uploads            {sha256, byteSize, originalName, …}   → session, offset 0
PUT  /v1/uploads/{id}        raw bytes, Upload-Offset: 0           → session, offset N
PUT  /v1/uploads/{id}        raw bytes, Upload-Offset: N           → session, offset N+M
  …
POST /v1/uploads/{id}/finalize                                    → capture
```

## One route, two request shapes

`POST /v1/uploads` answers two different bodies on the exact same URL,
told apart by `Content-Type` — never a query parameter or a second path:

- `multipart/form-data` — the original form: a `metadata` part (
  [`capture.schema.json`](capture.schema.json)'s `uploadMetadata`) then a
  `file` part, streamed, hashed and written incrementally in one request.
- `application/json` — this protocol's create step, below. The body is the
  _same_ `uploadMetadata` shape, with no file bytes anywhere in the request:
  a create request **is** an upload's declared metadata, just delivered as
  JSON instead of a multipart part.

Both share every capability/entitlement gate (quota, per-file size,
`entitlement_read_only`) and both eventually write through
[`capture-repository.js`](https://github.com/sqound/astro-portal/blob/main/packages/portal/src/persistence/capture-repository.js)'s
same idempotent `(tenant, sha256)` insert. Nothing about the legacy form
changes; this document only adds the second shape.

## Checking before sending: `POST /v1/uploads/check`

Every image leaves with its
SHA-256: before it sends anything, a client asks what an upload of these
bytes would meet. The body is the create step's own (below): `sha256`,
`byteSize`, `originalName`, and the optional `captureId`, `targetId` /
`targetIds`, `sessionId`, `planId`, `source`. A device token or the web
session (with its CSRF token). Nothing is written; the answer
([`upload-check.schema.json`](upload-check.schema.json)) is one of three:

```json
{ "result": "present", "captureId": "…", "imageId": "…", "main": true, "storage": { … } }
{ "result": "accepted", "adds": "variant", "storage": { … } }
{ "result": "refused", "code": "capture_variant_limit_reached", "message": "…", "storage": { … } }
```

- `present`: the space already holds these bytes, as any image of any of
  its captures (the main image or a variant); an upload would store
  nothing (« déjà en ligne, identique »). Never refused for a limit.
- `accepted`: the upload would be received — a new capture (`adds:
capture`) or an image added to `captureId` (`adds: variant`).
- `refused`: the upload would be refused, with the code and the numbers the
  upload itself answers, in the same order: `entitlement_read_only`, then
  `capture_limit_reached` (a new capture past `capture.count_limit`) or
  `capture_variant_limit_reached` (an image added to `captureId` past
  `capture.variant_limit`, the images beyond the main one), then
  `storage_quota_reached`, then `file_too_large`. `storage` carries the
  deciding numbers (`usedBytes`, `quotaBytes`, `maxFileBytes`,
  `captureCount`, `captureCountLimit`, `variantLimit`, and `variantCount`
  for an added image).

A malformed body is `400`; an unknown or another space's capture is `404`
`capture_not_found`; an unknown plan, session or target is `404` as for the
upload. The service checks again at reception: a refusal there (another
upload took the place meanwhile) keeps the upload resumable.

**An image for an existing capture**: `captureId` with bytes the space
does not hold adds a **variant** to that capture, not main, private, never
its main image by itself — its owner chooses the main one from the web
(`api/v1.md`, « Images of a capture »). Its `source` (`phone`, `astro-sync`,
`stacking`; never `web` from a device) is recorded with it.

**A progression image of a capture**: `captureId` with `role: "progression"` (and, optionally,
`progressionFrames`, the light frames the milestone stacks) adds a milestone
of the night Astro Sync stacked to that capture as its **history**
(`capture_variants.role`, migration 74). It is never the capture's main
image (not even of a capture without one), never a variant: it is not
counted against `capture.variant_limit` (`0` still accepts it) nor
`capture.count_limit`, only against the storage and the per-file size; it
stays private, never appears in the list of the captures, the galleries or
« what is online for a night », and the capture's page lists it under
`progression` (`manage-captures.schema.json`), from the first milestone to
the last. `role` without `captureId`, an unknown role, or `progressionFrames`
without the role is `400 invalid_request`. The check answers `accepted` with
`adds: "progression"`. A resumable upload keeps the role and the frames it
declared at its create step until it completes.

**The image of a multi-night plan**: `role: "plan"` with `projectId` declares the image Astro Sync
made by combining the integrations of several nights of a multi-night plan
(`integration.schema.json`). It is **a new capture of the space attached to
that plan** (`captures.project_id`, migration 76), never an image of a night:
`captureId`, `sessionId` and `planId` are absent (`400 invalid_request`
otherwise), it is listed in no night's captures and is no variant of one; it
counts like any capture against `capture.count_limit` and the storage. It
carries `combinedNightIds` (the nights it combines), `totalsByFilter`
(`[{ filter, frameCount?, totalSeconds }]`, its total by filter) and
`planRender` (`natural`, `dualband`, `polished`, `hoo`, `none`), all optional and only with the
role. The project answers it as `planCapture` (the newest one) on
`GET /v1/projects/{projectId}`, and the capture resource carries `projectId`
and `plan`. It needs the multi-night plans of Astro Sync:
`403 multi_night_not_active` (with `capability: "analysis.progression"`)
without « Synchronisation avancée+ », in a subscription's read-only window,
and **always on a private server**; `404 project_not_found` for a plan the
space does not hold. The check answers the same refusals as errors (never as
`refused`), and `accepted` with `adds: "capture"`. A resumable upload keeps
the role and the plan's fields it declared until it completes.

## `POST /v1/uploads` — create

```json
{
  "sessionId": null,
  "planId": null,
  "targetIds": ["m31"],
  "originalName": "m31-session-14.tif",
  "sha256": "3b1f0d6c9a8e4725d0c1b2a39f8e7d6c5b4a39281706f5e4d3c2b1a09f8e7d6c",
  "byteSize": 734003200,
  "contentType": "image/tiff"
}
```

Answers [`capture.schema.json`](capture.schema.json)'s `uploadSession`:

```json
{
  "uploadId": "8a25c9d2-7f0b-4b3a-9c9e-2f7e6f0b1a3d",
  "offset": 0,
  "byteSize": 734003200,
  "chunkSize": 8388608,
  "completed": false,
  "expiresAt": "2026-09-23T20:00:00.000Z"
}
```

`chunkSize` is server-announced, not negotiated: the client reads it here (or
from any later `GET`) and never sends a `PUT` body larger than it. It comes
from `ASTRO_PORTAL_UPLOAD_CHUNK_BYTES` (default 8 MiB, `capture-storage.js`'s
`defaultUploadChunkBytes`) — a deployment-wide operational knob, not a
per-account capability; see the design choices below.

**Capability refusals happen here, before any byte of the file is
transferred** — the same two capabilities, in the same order, `POST
/v1/uploads`'s multipart form already refuses by (`api/v1.md`, "Uploads and
captures"):

0. With `captureId`: `capture.variant_limit` — `409`
   `capture_variant_limit_reached` when the capture already holds as many
   images beyond its main one as the profile allows (`0`: the main image
   only); `404` `capture_not_found` for an unknown or another space's
   capture. Otherwise `capture.count_limit` — `409` `capture_limit_reached` when the space's
   captures, plus its other uploads in progress, plus this one, would exceed
   it (a file the space already holds is never counted twice); checked
   first, at creation and again at finalize.
1. `storage.quota_bytes` — `409` `storage_quota_reached` when it is `0`, or
   `usedBytes + byteSize` would exceed it. `usedBytes` counts the stored
   captures **and the bytes the space's other unfinished uploads already
   hold** (`api/v1.md`, "Isolation between spaces"); every `PUT` checks the
   same sum before writing a byte of its chunk, and finalize again.
2. `storage.max_file_bytes` — `413` `file_too_large` when it is `0`, or
   `byteSize` exceeds it.

The soft-downgrade write gate also applies at creation: an account whose
subscription has lapsed (`entitlement_read_only`) is refused `403` before an
upload session is even allocated — existing captures stay readable and
exportable, but no new transfer starts.

### Idempotency — two levels

The pair `(tenant, sha256)` is unique for an upload session, on the exact
precedent `captures` itself already sets:

- **Bytes the space already holds** (through _either_ transport — this
  resumable one, or a single-request upload — as any image of any capture)
  — creating an upload for that `sha256` answers the **completed** record
  immediately, **before any chunk**: `completed: true`, the existing
  `captureId`, `present: true` and the `imageId` of the image that holds
  them, no file transfer needed at all, never refused for a
  limit. Newly-declared `targetIds` are unioned onto the existing capture,
  exactly like a legacy replay (`capture.schema.json`'s `uploadResponse`
  doctrine) — sending the same bytes with one more target still adds it.
  Never a second capture row.
- **Bytes already mid-transfer** — creating an upload for a `sha256` that
  already has an `in_progress` session answers that **same** session, at its
  current offset, rather than starting a second, competing transfer of the
  same content. A declared `byteSize` that disagrees with the one already on
  record is `400` `invalid_request` (a real inconsistency, not a resumed
  transfer); a differing `originalName`/`contentType`/`sessionId`/`planId` is
  silently ignored — first declaration wins — while any new `targetIds` are
  still unioned onto the in-progress session so they are not lost by the
  time it finalizes. One exception: a create that names **another capture**
  (`captureId`, or none where the session had one) than the session aims at
  takes the session to that capture, with this create's `role`,
  `progressionFrames`, proposals, `sessionId`, `planId` and `source`; the
  bytes already received and the offset are kept. The same bytes are then
  sent elsewhere — typically the capture the first create named was removed
  (its space emptied) and the multi-night plan sent again made a new one —
  and the latest create says where the image goes.

A creation that finds neither answers `201` with a fresh `uploadId`, offset
`0`. Everything else above answers `200`.

## `PUT /v1/uploads/{id}` — one chunk

```http
PUT /v1/uploads/8a25c9d2-7f0b-4b3a-9c9e-2f7e6f0b1a3d HTTP/1.1
Upload-Offset: 0
Content-Length: 8388608
Content-Type: application/octet-stream

<raw bytes>
```

Two headers, both required, both plain non-negative integers:

- `Upload-Offset` — must equal the server's own current, durable offset for
  this session **exactly**. A mismatch never partially applies the chunk; it
  answers `409` `upload_offset_mismatch` with the server's real offset:

  ```json
  {
    "error": { "code": "upload_offset_mismatch", "message": "…" },
    "offset": 4194304
  }
  ```

  A client resumes from that number without a second `GET`.

- `Content-Length` — the exact byte length of this request's body. It must
  not exceed the session's own `chunkSize` (the last chunk is typically
  smaller, not larger), and `offset + Content-Length` must not exceed the
  session's declared `byteSize`; either violation is `400` `invalid_request`.
  A body whose actual byte count disagrees with its own declared
  `Content-Length` is also `400` `invalid_request`.

A successful `PUT` answers the same `uploadSession` envelope with the new
`offset` — **never** `completed: true` on its own, even when the new offset
reaches `byteSize`: finalizing is always the separate, explicit call below.
`PUT` on an already-`completed` session is an idempotent no-op, `200`,
ignoring the body.

**Durability.** A chunk is buffered whole in memory (bounded by `chunkSize`,
never the file's total size) before a single append to the session's part
file. A connection dropped mid-chunk therefore leaves the previous,
already-durable offset untouched — nothing is torn — and the client simply
retries the same chunk from that offset; see "Resuming after an
interruption" below.

## `GET /v1/uploads/{id}` — status

Answers the same `uploadSession` envelope: `offset`, `byteSize`,
`chunkSize`, `completed`, `expiresAt`. Reconciles against the part file's
actual size on disk before answering, so a status check after an
interruption always reports the true, durable offset — never a stale
database cache, and never a partially-written chunk's byte count. An id
whose 72-hour window has passed, or that was never created, answers `404`
`upload_not_found`.

## `POST /v1/uploads/{id}/finalize`

No body. Three steps, in order:

1. **Digest verification.** The assembled part file's own SHA-256 is
   computed and compared to the `sha256` declared at creation. A mismatch
   answers `422` `upload_hash_mismatch` and **creates nothing** — the
   session resets to offset `0` (`"offset": 0` in the error body) so the
   client re-uploads from scratch, since any chunk could be the corrupt one.
   Calling finalize before every declared byte has arrived (`offset !==
byteSize`) is `409` `upload_incomplete` instead, also with the current
   `offset`.
   An upload that adds an image to a capture (`captureId` at its create)
   whose capture has left the space since (removed, its space emptied)
   answers `409` `upload_capture_gone` and creates nothing: not a refusal of
   the image. The session and its bytes are kept; a new create naming the
   capture the image goes to now resumes it at its offset (see
   Idempotency), and a capture the space does not hold is refused there,
   `404` `capture_not_found`.
2. **The exact same gates the legacy single-request upload always has** —
   `entitlement_read_only`, then `storage.quota_bytes`, then
   `storage.max_file_bytes`, then the optional `planId`/`targetIds`
   existence checks — run again here, against the now-confirmed byte size.
   A long-lived transfer can therefore still be refused at the finish line
   if the account's standing changed mid-transfer; see the design choices
   below. Refusing here never deletes the part file: a client can retry
   finalize later (once upgraded, or once the missing target exists) without
   re-uploading.
3. **Capture creation.** Only once every gate passes does the file leave the
   resumable area and become a capture, through the identical
   `insertCaptureWithTargets` the legacy upload calls.

Success answers [`capture.schema.json`](capture.schema.json)'s
`uploadFinalizeResponse` — the `uploadSession` envelope
(`completed: true`, a `captureId`, `expiresAt: null`) plus `capture` and
`deduplicated`, exactly like the legacy form's `uploadResponse`:

```json
{
  "uploadId": "8a25c9d2-7f0b-4b3a-9c9e-2f7e6f0b1a3d",
  "offset": 734003200,
  "byteSize": 734003200,
  "chunkSize": 8388608,
  "completed": true,
  "expiresAt": null,
  "captureId": "2e7d9b31-4c06-4a8f-91b3-5f8c0d2a6e47",
  "capture": { "…": "capture.schema.json's capture" },
  "deduplicated": false
}
```

**Idempotent.** Calling finalize again on an already-completed session
replays this exact result — the same `captureId`, no second capture, no
re-verification of a part file that no longer exists.

## Resuming after an interruption

```text
create  → offset 0
PUT     → offset 8 MiB
PUT     → connection drops mid-chunk        (offset still 8 MiB, durable)
GET     → offset 8 MiB                      (client learns where to resume)
PUT     → the SAME chunk, sent whole        → offset 16 MiB
  …
finalize
```

A client that lost its connection never needs to guess: `GET` (or a fresh
`create` for the same `sha256`, which answers the same thing) always reports
the last durably-written offset, never a partial one.

## Expiry and purge

An `in_progress` session that has not finalized within **72 hours** of its
creation is treated as gone: any `GET`/`PUT`/`finalize`/create-replay that
would touch it instead purges it first (deletes the row and its part file)
and answers as if it had never existed (`404` `upload_not_found` for
`GET`/`PUT`/`finalize`; a fresh `201` session for a create). A **completed**
session is never subject to this expiry — it is a small, permanent
idempotency record once a capture exists, exactly like `captures.sha256`
itself.

Besides this lazy check on access, the service sweeps _every_ expired
session (`upload-resources.js`'s `purgeExpiredUploads(asOf)`) once when it
starts, then along the upload requests at most once per
`ASTRO_PORTAL_UPLOAD_PURGE_INTERVAL_MS` (default one hour) — no timer, so an
idle service can sleep and a stopping one exits at once.

## Error codes this protocol adds

| Code                            | Status | Body also carries       | Meaning                                                                                                                      |
| ------------------------------- | :----: | ----------------------- | ---------------------------------------------------------------------------------------------------------------------------- |
| `upload_offset_mismatch`        |  409   | `offset` (server's own) | `Upload-Offset` disagreed with the session's actual current offset.                                                          |
| `upload_incomplete`             |  409   | `offset` (server's own) | Finalize was called before all declared bytes arrived.                                                                       |
| `upload_hash_mismatch`          |  422   | `offset: 0`             | The assembled digest disagreed; nothing created, session reset.                                                              |
| `upload_not_found`              |  404   | —                       | Unknown id, or its 72-hour window already passed.                                                                            |
| `upload_capture_gone`           |  409   | —                       | Finalize: the capture the upload adds an image to left the space after its create; create again (bytes kept).                |
| `upload_sessions_limit_reached` |  409   | —                       | The space already keeps `ASTRO_PORTAL_UPLOAD_MAX_OPEN_SESSIONS` (32) unfinished uploads open; resuming one is never refused. |
| `capture_content_refused`       |  415   | —                       | The bytes are an SVG or HTML document (checked at the first chunk and at finalize); the upload is discarded.                 |

Every capability/entitlement refusal (`capture_limit_reached`,
`capture_variant_limit_reached`, `storage_quota_reached`, `file_too_large`,
`entitlement_read_only`) and every relational one
(`plan_not_found`, `target_not_found`) reuses exactly the codes and bodies
`api/v1.md`'s "Uploads and captures" section already defines.

## Design choices

The following product-visible choices are surfaced here rather than assumed
silently:

1. **One route, two Content-Types**, rather than a second path for the
   create step — see "One route, two request shapes" above.
2. **`chunkSize` is a deployment-wide operational knob**
   (`ASTRO_PORTAL_UPLOAD_CHUNK_BYTES`, default 8 MiB), not a capability that
   could differ by offer/account.
3. **A single upload session per `(tenant, sha256)`** — re-creating for bytes
   already mid-transfer resumes the SAME session rather than starting a
   competing one. The Rust client must never assume a fresh, empty session
   from a repeated create call; it must read the returned `offset`.
4. **Finalize re-checks quota/entitlement**, not only creation — a
   long-running transfer can still be refused at the finish line if the
   account's standing changed. The part file is kept (not deleted) on that
   refusal, so a later retry does not re-upload.
5. **Purge is lazy-on-access, at startup, and along the requests** (at most
   hourly), not a timer — no background runner exists in this app, and a
   timer would keep an idle service awake.
