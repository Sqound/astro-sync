# The Journal: the list of the sessions

The rules are in [v1.md](v1.md), sections « The Journal: a plan carried out
becomes a session », « Reworking a session » and « A capture and its
session »; the shapes are [`journal.schema.json`](journal.schema.json) and
[`session-rework.schema.json`](session-rework.schema.json). The list is
replayed against one shared fixture, `fixtures/journal-sessions.json`, by the
service, the web and the phone alike; what the service actually answers is
recorded in `fixtures/journal-exchange-<name>.json`.

## The words

A **plan** is a promise (« plan d'une nuit », « plan multi-nuits »; internally
a _project_). A **session** is the plan carried out, enriched with what the
night gave. The **Journal** is the page of all the sessions. The **night** is
the time from evening to morning, the **execution** the action in progress
(« Nuit en cours »). Never « séance », « session en cours », « journal des
nuits ».

## The example

A space with three sessions of the Heart (IC 1805) and a free observation:

- **9 August**, a plan executed, debriefed `executed`, 3 h 20 acquired;
- **10 August**, a session without a plan (an observation made freely): state
  `no_plan`, `origin: "execution"`;
- **12 August**, a plan debriefed `partial`, one of its two targets removed;
- **14 August**, a plan whose night is past, carried out without the
  application: a line with `sessionId: null` and `state: "to_debrief"`;
- the three nights of the Heart belong to a multi-night plan, finished on
  15 August: they form a **group**.

`GET /v1/journal?year=2026` answers the lines newest first (14, 12, 10, 9
August), `groups` with the Heart's plan (`sessionIds` = the three sessions),
`years: [2026]`, `instruments`, `stats` over the four lines that hold a
session, and `toDebriefCount: 1` (the 14 August line). With
`outcome=to_debrief` the answer holds the 14 August line alone, and
`stats.sessions` is 0 (a line without a session counts for nothing).

## Rules of the list

- **The order is total**: `eveningDate` descending, `startedAt` descending (a
  line without one last), then the line's id descending. A cursor is
  exclusive and opaque.
- **The filters add up**; `years` and `instruments` ignore them; `stats`
  follows all of them and covers the whole selection, not the page.
- **`state`** is derived: `to_debrief` for a finished session of a plan with
  no debriefing, and for a plan whose night is past and was not carried out in
  the application (no session: `sessionId` is `null`, `origin` is `null`);
  `executed` / `partial` / `abandoned` from the debriefing; `no_plan` for a
  session with no plan. An open or paused session is not listed. The session of
  a **guided evening** (its plan carries `guidedEvening: true`, the phone's
  internal plan) is `no_plan` too, with `guidedEvening: true` on its line: it is
  shown « Soirée guidée · {planName} », never to debrief; that plan with no
  session is not listed.
- **The figures of a target** come from the debriefing, then from the events,
  then from Astro Sync's analysis when there is one; `null` when nothing says.
  The thumbnail is the target's capture of that session. Its **times**
  (`startedAt`, `finishedAt`) come from the rework, else the debriefing, else
  the events, each one apart; its **exposure** (`exposureSeconds`, with
  `exposureFamilies`) from the analysis, else what the rework or the
  debriefing declared (v1.md, « What the screens of the Journal still read »).
  Its **shooting** (EXPO-G, additive): `planned` (the lines the launch froze
  for its plan item) and `measured` (`astro-sync`, else `declared`), both
  absent when there is nothing to say (v1.md, same section).
- **A target taken out by a rework** is never erased: the plan's, the one met
  in the field and the one added at the debriefing all stay in the session,
  `removed`, out of the figures. Its signals fall to the whole
  night, unless the rework moves them. A signal the person modifies is
  **replaced**, not edited: the original stays in the log, hidden, and the new
  one is `reconstructed` (v1.md, « Replacing a signal, and the targets a rework
  takes out »).
- **A read-only night** (a catalogue night of a private server) has the id
  `catalogue:<date>:<target key>`, `readOnly: true`, `origin: "import"`;
  `GET /v1/sessions/{id}` reads it, nothing writes to it (`409
session_read_only`).
- **`toDebriefCount`** is the one count of the lines in the state `to_debrief`,
  whatever the filters: the source of the badge of the Journal.
- **A deleted session is out of the list**, of `stats`, of `toDebriefCount` and
  of the debrief queue; it is read, with the days left, by
  `GET /v1/journal/deleted` (v1.md, « Deleting a session »).
- **`abandoned`** is a night not carried out; the interfaces write it
  « Abandonné » (it was « Non exécuté »), the value on the wire is unchanged.
- **A group** is a finished multi-night plan; its sessions are also in
  `sessions[]`, and a client draws them as the group.
  A night that serves **two** plans (one target for each: `projectIds` on the
  line, sent only then) belongs to both groups: both name it in `sessionIds`
  and `lineKeys`, each counts it in its `lineCount`, and its `frameCount`
  holds the frames of the plan's own target only. The line stays one line of
  `sessions[]`; a client draws its card in each group that names it.
- **Never held by a right.** The list is a read: free, on a private server and
  in the read-only window alike.
