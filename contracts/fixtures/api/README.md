# Recorded API exchanges

Unlike the `<module>.json` files one directory up, which are generated from
the specification files under `spec/`, the files under `api/` are written by hand. Each one
holds HTTP exchanges — a request and the answer the service gives to it —
that the mobile application, the web and Astro Sync can replay against a
fake service, and that the service's own tests replay against the real one.

```json
{
  "contract": "sign-in.schema.json",
  "status": "built",
  "exchanges": [
    {
      "id": "request-code-accepted",
      "description": "…",
      "given": { "authMode": "email-code", "now": "2026-09-29T21:14:00.000Z" },
      "request": {
        "method": "POST",
        "path": "/api/v1/sign-in/code",
        "headers": {},
        "body": {}
      },
      "response": {
        "status": 202,
        "headers": {},
        "body": {},
        "schema": "sign-in.schema.json#/$defs/codeRequestAccepted"
      }
    }
  ]
}
```

- `status` is `built` when the service answers these exchanges today, and
  `planned` when the contract describes a later lot: the shapes are fixed,
  the service does not answer them yet. Every file is built today.
- `given` states what the service holds before the request (its sign-in
  mode, the clock, a code already sent, an attempt already failed…).
- `path` is the full path the service answers (`/api/v1/…`); `v1.md`
  shortens it to `/v1/…`.
- Identifiers, tokens and dates are examples: a real service returns other
  values of the same shape. `response.schema` names the definition the body
  must satisfy, relative to `schemas/api/`.
- Error bodies use the v1 envelope; extra fields (`attemptsRemaining`,
  `retryAt`) sit beside `error`, as `serverRevision` does for a plan
  conflict.

| File                               | Status | What it covers                                                                                                 |
| ---------------------------------- | ------ | -------------------------------------------------------------------------------------------------------------- |
| `entitlements-private-server.json` | built  | `GET /v1/entitlements` on a private server: no purchase, its own profile                                       |
| `service-identity.json`            | built  | `GET /v1/service` and its `signIn` field                                                                       |
| `sign-in-code-request.json`        | built  | `POST /v1/sign-in/code`, every answer                                                                          |
| `sign-in-verify.json`              | built  | `POST /v1/sign-in/verify`, by code and by link, refusals                                                       |
| `sign-in-link.json`                | built  | the link of the message and the page it opens                                                                  |
| `space-current.json`               | built  | `GET /v1/space`                                                                                                |
| `space-devices.json`               | built  | devices of a space: list, rename, detach, a phone with no space                                                |
| `space-deletion.json`              | built  | `DELETE /v1/space`, and any call after it                                                                      |
| `plan-trash.json`                  | built  | the trash of the plans: delete, `plan_in_progress`, `deletedPlanIds`, list, restore, erase, a multi-night plan |
