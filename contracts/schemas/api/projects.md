# The nights of a multi-night plan

The resource of a multi-night plan, its routes and its integrations are in
[v1.md](v1.md), section « Projects »; the shapes here are in
[`project.schema.json`](project.schema.json) (`nightsWrite`, `night`, `nights`,
`create`). « Project » stays an internal name: an observer reads « plan
multi-nuits ».

## The example

The Heart (IC 1805), three nights run before any multi-night plan existed:
**9 August** with its plan of one night « Nuit du 9 août »; **10 August** run
without a plan (« Nuit sans plan », its target written on the night or met in
its steps); **12 August** with its plan. A plan of one night of the Heart for
**20 August** is not run yet. The person creates « IC 1805 » « à partir de
plans déjà existants » and ticks the three nights and the plan of the 20th.

## One truth: where the link lives

- A **plan of one night** whose items include the target of the multi-night
  plan carries the link on that item: `planItem.projectId`
  ([`plan.schema.json`](plan.schema.json)), the field the planner, the phone,
  Astro Sync and the progress already read. Attaching writes it on the item
  of the target (every other item is untouched), as a new revision of the
  plan — the replaced payload is kept as `history`, a device holding the
  older revision meets the usual `409 plan_conflict` and its copy is set
  aside, never lost. A plan already run (`closedAs`, frozen) is written all
  the same: the link is not a change of the night. 9 and 12 August, and the
  plan of the 20th, are attached this way (`link: "plan"`). **Through
  `POST` / `DELETE …/nights`, not through `PUT /v1/plans/{planId}`**: a plan
  the service froze (`executed`, `partial`, `abandoned`, `archived`, set by
  a debriefing) refuses every `PUT`, a link included (`409 plan_frozen`), so
  the phone carries the links of such a night by these two routes
  and reads the plan again.
- A **night with no plan item of the target** — run without a plan, or a
  target met in the field that its plan did not hold — carries the link
  itself (the night's own multi-night plan; `link: "night"`). 10 August is
  attached this way. Its plan, if it has one, is not written.
- A night belongs to **one multi-night plan at most**: attaching it
  elsewhere moves it (the item names the new plan, the night's own link is
  cleared). A night of a plan's item reads its plan's link first.

## Which nights may be attached: the same target

A multi-night plan has one target, `target.targetId` or `target.framingId`
(a framing answers to its own id and to the catalogue target it was made
on). A plan of one night is of that target when one of its items is (its
`targetId`, `target.id`, `framingId` or `id`). A night of the journal is of
that target when its plan is, or when the night met it: its own targets
(`night.targetIds`), its acquisition steps, its journal (`target_started`,
`target_finished`), or the integrations Astro Sync sent for it. Anything else
is refused `409 target_mismatch`, naming the `sessionId` or `planId`, and no
capability lifts it.

## The routes

`GET /v1/projects/{projectId}/nights` (either credential) answers
`200 { projectId, nights, candidates }`. **Reading never needs a
capability**, on any service.

- `nights`: the nights of the plan — planned from it or attached since —
  the earliest first. Each line (`night`): `sessionId` (`null` for a plan of
  one night not run yet), `planId` (`null` for a night run without a plan),
  `planName`, `eveningDate` (its plan's date, else the evening of its start
  at its longitude, local noon to local noon, never a time zone),
  `sessionStatus` (`open`, `paused`, `finished`, or `null`), `link` (`plan`
  or `night`), and `integrationCount` / `integrationSeconds`, the
  integrations of that night counted for this plan.
- `candidates`: the nights of the journal and the plans of one night of the
  same target that are not among them, the earliest first, with `link: null`
  and `projectId`, the other multi-night plan each already belongs to (or
  `null`): what « Ajouter une nuit du journal » lists, cochable. A plan of
  one night already run appears once, as its night (with its `planId`).

`GET /v1/projects/candidates?targetId=…` (or `?framingId=…`; either
credential, never gated) answers `200 { candidates }` (`candidates`) for a
target that **has no multi-night plan yet**: the same lines as the
`candidates` of the route above (`link: null`, `projectId` the multi-night
plan a night already belongs to, or `null`), with the integrations Astro Sync
sent for each night counted whichever plan they follow. It is what « À partir
de plans déjà existants » lists once the person has chosen the target, before
`POST /v1/projects` creates the plan from the ticked nights.
`400 invalid_request` when neither id is given.

`POST /v1/projects/{projectId}/nights` (either credential, CSRF for the web)
takes `{ sessionIds?, planIds? }` (`nightsWrite`: at least one id, fifty per
list at most) and attaches **all of them, or none**: every night and plan is
checked before one is written. Send a candidate by its `sessionId` when it
has one, else by its `planId`. Attaching a night already attached to the
plan changes nothing. Answers `200` with the body of `GET`.

`DELETE /v1/projects/{projectId}/nights/{nightId}` detaches: `nightId` is a
night of the journal, or a plan of one night not run yet (its own id). Its
item no longer names the plan, the night's own link is cleared, and its
integrations leave the plan. **Idempotent**: a night that is not attached
changes nothing. Answers `200` with the body of `GET`;
`404 night_not_found` when the space holds neither a night nor a plan of
that id.

`POST /v1/projects` creates a multi-night plan without `If-Match` (`create`):
the body of a `PUT` creation, an optional `id` (generated when absent), and
« à partir de plans déjà existants » `fromSessionIds` / `fromPlanIds`. The
nights and plans are **checked before the plan is created** — nothing is
created unless all can be attached — then attached in the same request.
Answers `201` with the project resource and its `nights`. « Un plan
multi-nuits vierge » is the same call without them (or `PUT` with
`If-Match: "0"`, unchanged).

`GET /v1/projects/{projectId}` also lists `nights` (never the list).
`GET /v1/projects?sessionId=…` or `?planId=…` keeps only the multi-night
plans that night, or that plan of one night, belongs to: what « Rattacher à
un plan multi-nuits » and « Détacher » need on the page of a night or of a
plan.

## Never on a private server

Attaching, detaching, and creating a plan from existing nights are gestures
on the plan itself: their right is `planner.mosaic` (« Planification
avancée »). Without it, and on **every private server**, they answer `403 multi_night_not_active`
with `capability: "planner.mosaic"`, and nothing is written. A blank creation
has the same right: `planner.mosaic`, else `403 capability_required`.
« Synchronisation avancée+ » alone does **not** attach a night. In the
read-only window of an ended subscription the right holds only when it comes
from a purchase that is still held (see below).

**The plan's items carry the same rule.** The link is also written by a plan
write (`PUT /v1/plans/{planId}`, a restore, the web planner's `PUT /api/plans`)
that sets, changes or removes `planItem.projectId`: that item is then checked
as `POST …/nights` checks a plan (right, `404 project_not_found`,
`409 target_mismatch` for an item that is not of the target of the plan), the
night moves to the new multi-night plan with its integrations, and taking the
link away detaches. A plan write that leaves every `projectId` untouched is
never subject to these checks; a stale copy is still `409 plan_conflict`.

**A purchase still held keeps its writes in the read-only window.** The window
of an ended subscription refuses a write unless the capability that
authorizes it comes from a purchase that is still held: a space that still
holds « Planification avancée » after its subscription ended still
attaches, detaches, creates and edits a multi-night plan (and writes the links
on the items of its plans); the other writes (ordinary plans, media, places)
and what belongs to a subscription stay refused.

What stays under `analysis.progression` (« Synchronisation avancée+ ») is
what Astro Sync sends to a plan: the integrations attached to it
(`PUT /v1/projects/{projectId}/integrations`) and the image of the plan,
which answer `403 multi_night_not_active` with `capability:
"analysis.progression"` (see integration.schema.json).

## The integrations follow the night

An integration names its night (`nightId`). When a night is attached, its
integrations of the plan's target count for the plan (they move from any
other plan); when it is detached, they leave it and count for none until the
night is attached again. The progress of the plan stays computed from the
debriefings of its nights.

## Every refusal

| Status | Code                     | When                                                                                    |
| ------ | ------------------------ | --------------------------------------------------------------------------------------- |
| `400`  | `invalid_request`        | A body outside `nightsWrite` / `create`.                                                |
| `403`  | `multi_night_not_active` | Without « Planification avancée » (`planner.mosaic`), read-only window, private server. |
| `403`  | `capability_required`    | A blank creation without `planner.mosaic`.                                              |
| `404`  | `project_not_found`      | The space holds no such multi-night plan.                                               |
| `404`  | `session_not_found`      | A `sessionIds` / `fromSessionIds` entry (with `sessionId`).                             |
| `404`  | `plan_not_found`         | A `planIds` / `fromPlanIds` entry (with `planId`).                                      |
| `404`  | `night_not_found`        | `DELETE`: neither a night nor a plan of that id.                                        |
| `409`  | `target_mismatch`        | A night or plan of another target (with `sessionId` or `planId`).                       |

Every attachment and detachment emits the change-signal domains `projects`,
and `plans` or `sessions` for what it wrote, and records an audit event
(`project.nights_attached`, `project.night_detached`).
