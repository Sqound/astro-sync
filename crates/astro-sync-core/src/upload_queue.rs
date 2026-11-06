//! The persistent, resumable upload queue.
//!
//! One row per file a dry-run report named, stored in a SQLite database
//! ([`rusqlite`], bundled) at a path the caller chooses -- never inside the
//! scanned source folder, since this file is Astro Sync's own bookkeeping,
//! not one of the user's captures. The queue survives process restarts: a
//! file's [`QueueState`] and durable server `offset` are read back exactly
//! as they were left, which is what lets [`crate::uploader`] resume a
//! transfer after an interruption instead of re-sending bytes already
//! accepted.
//!
//! Identity is the file's own `sha256` **within the space the upload is
//! addressed to** (`space_id`, unique together), on the same
//! content-addressing precedent the portal's own `captures` table already
//! uses: [`UploadQueue::enqueue_from_dry_run`] is idempotent on it, and
//! [`UploadQueue::next_pending`] never returns a row already `completed`.
//!
//! **The queue belongs to a space, not to the computer.** A queue opened
//! with [`UploadQueue::for_space`] reads and writes only the rows of that
//! space (the one `space.json` names): a file sent to another space -- or
//! before this computer was linked to any (`space_id` NULL, « before the
//! link ») -- never makes it « already known » here, and the history, the
//! resumption and the sky annotations of one space never show in another.
//! Linking another space loses nothing: the rows of the former one stay in
//! the database, out of sight. A queue opened without a space (the command
//! line, a paired computer) works on the rows without one.
//!
//! **Only results enter it**: a final image, a stacked image, a progression image -- a TIFF, a
//! JPEG or a PNG, never under `lights/`, `calibration/` or `logs/`. A FITS,
//! a XISF, a raw camera file, a calibration frame or a raw log is refused
//! ([`result_kind_of`]) and never leaves the computer; rows of those classes
//! left by an older build are dropped when the queue opens. The night's
//! report and its sky solutions are not files: they are sent as they are
//! made ([`crate::night_analysis`]). Each row carries what its upload is
//! attributed to ([`EntryTarget`]): the night, its targets, and the capture
//! it is a variant of.

use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::dry_run::DryRunReport;
use crate::sky_after_upload::{SkyAnnotation, SkyState};
use crate::time_util::now_iso8601_utc;

/// The code a completed row carries when its image replaced the one its
/// capture had (`last_error_code` of a completed entry is otherwise empty:
/// it is a refusal's or a hold's code). The interface looks it up
/// (`queue.replacedPhoto`).
pub const REPLACED_NOTE: &str = "replaced";

/// What a result is: the only files the queue takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResultKind {
    /// The processed image of a target, from its `final/` folder.
    Final,
    /// The image the stacking wrote in `<target>/stacked/`.
    Stacked,
    /// An image of the progression, from `<target>/progression/`.
    Progression,
    /// The image of a multi-night plan, from `<plan>/stacked/`: a capture of
    /// the plan, never of a night (`role: "plan"`).
    Plan,
}

impl ResultKind {
    fn as_sql(self) -> &'static str {
        match self {
            ResultKind::Final => "final",
            ResultKind::Stacked => "stacked",
            ResultKind::Progression => "progression",
            ResultKind::Plan => "plan",
        }
    }

    fn from_sql(value: Option<&str>) -> Self {
        match value {
            Some("stacked") => ResultKind::Stacked,
            Some("progression") => ResultKind::Progression,
            Some("plan") => ResultKind::Plan,
            _ => ResultKind::Final,
        }
    }
}

/// Why a file is not a result: it never enters the queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotAResult {
    /// A FITS or XISF frame, or a raw camera file (light or calibration).
    Frame,
    /// A file under `calibration/`.
    CalibrationFrame,
    /// A raw log (`.log`, `.txt`, `.csv`, or anything under `logs/`).
    RawLog,
    /// A file under `lights/`, whatever its extension.
    InLights,
    /// Not an image the service keeps (TIFF, JPEG, PNG).
    NotAnImage,
}

const RESULT_EXTENSIONS: [&str; 5] = ["tif", "tiff", "jpg", "jpeg", "png"];
const FRAME_EXTENSIONS: [&str; 9] = [
    "fit", "fits", "fts", "xisf", "cr2", "cr3", "nef", "arw", "dng",
];
const LOG_EXTENSIONS: [&str; 3] = ["log", "txt", "csv"];

/// Whether `path` is a result the queue may take, and which. The extension
/// decides first (a FITS is a frame wherever it is), then the folders the
/// path goes through (`lights/`, `calibration/`, `logs/` hold no result);
/// a file under `stacked/` is a stacked image, one under `progression/` a
/// progression image, any other a final one.
pub fn result_kind_of(path: &Path) -> Result<ResultKind, NotAResult> {
    let extension = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if FRAME_EXTENSIONS.contains(&extension.as_str()) {
        return Err(NotAResult::Frame);
    }
    if LOG_EXTENSIONS.contains(&extension.as_str()) {
        return Err(NotAResult::RawLog);
    }
    let folders: Vec<String> = path
        .parent()
        .into_iter()
        .flat_map(Path::components)
        .map(|c| c.as_os_str().to_string_lossy().to_ascii_lowercase())
        .collect();
    let within = |name: &str| folders.iter().any(|f| f == name);
    if within("calibration") {
        return Err(NotAResult::CalibrationFrame);
    }
    if within("logs") {
        return Err(NotAResult::RawLog);
    }
    if within("lights") {
        return Err(NotAResult::InLights);
    }
    if !RESULT_EXTENSIONS.contains(&extension.as_str()) {
        return Err(NotAResult::NotAnImage);
    }
    Ok(if within("stacked") {
        ResultKind::Stacked
    } else if within("progression") {
        ResultKind::Progression
    } else {
        ResultKind::Final
    })
}

/// What an upload is attributed to, kept with its row: the night, its targets,
/// the capture it is a variant of (`captureId`: never the main image by
/// itself), and its source (`astro-sync`, `stacking`). Empty: the run's own
/// [`crate::uploader::UploadTarget`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryTarget {
    pub session_id: Option<String>,
    #[serde(default)]
    pub target_ids: Vec<String>,
    pub variant_of: Option<String>,
    pub source: Option<String>,
    /// The capture metadata Astro Sync proposes with this image
    /// ([`crate::metadata`]), kept with the row so a resumed sending
    /// proposes the same.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<crate::metadata::Proposals>,
    /// The light frames a progression image stacks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_count: Option<u64>,
    /// The multi-night plan an image of the plan belongs to, and what it
    /// combines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<PlanUpload>,
}

/// What the image of a multi-night plan declares (`uploads.md`, « The image
/// of a multi-night plan »): its plan, the nights it combines, its total by
/// filter, its render.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanUpload {
    pub project_id: String,
    pub combined_night_ids: Vec<String>,
    pub totals_by_filter: Vec<PlanFilterTotal>,
    /// `natural`, `dualband`, `polished`, `hoo` or `none`.
    pub plan_render: Option<String>,
}

/// One filter's line of the image of a plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanFilterTotal {
    pub filter: Option<String>,
    pub frame_count: u64,
    pub total_seconds: f64,
}

/// One result to enqueue: its file, digest, size, kind and attribution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultFile {
    pub path: PathBuf,
    pub sha256: String,
    pub size: u64,
    pub kind: ResultKind,
    #[serde(default)]
    pub target: EntryTarget,
}

/// A queued file's lifecycle. Stored in SQLite as its lowercase name.
///
/// `Queued` and `Uploading` are the states a run picks up: they are also
/// where a file waits while Astro Sync tries again later by itself
/// ([`crate::uploader::UploadError::verdict`], [`crate::gui_bridge::drive_queue`]).
/// `Completed`, `Failed` and `Refused` end the file's turn for good; `Held`
/// ends it until the person acts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueState {
    /// Enqueued, no upload session created yet.
    Queued,
    /// A session exists and at least one byte has been durably accepted
    /// (or is in flight); also the state a crashed transfer is left in, so
    /// [`UploadQueue::next_pending`] finds it again on the next run.
    Uploading,
    /// Finalized: a capture exists. Never returned by `next_pending` again.
    Completed,
    /// The transfer failed for good and is not retried on its own: a digest
    /// mismatch on a second attempt, an offset the service keeps disputing,
    /// a source file that cannot be read. `next_pending` skips it, so a run
    /// never loops on it.
    Failed,
    /// The service refuses this file for good (an SVG or HTML document, a
    /// file over the size the space allows, a request over the service's
    /// ceiling, or a refusal this Astro Sync does not know): it leaves the
    /// pending set with the code of its refusal
    /// ([`QueueEntry::last_error_code`]), which the interface turns into a
    /// sentence. Never sent again on its own; the file on the computer is
    /// never touched.
    Refused,
    /// Nothing is wrong with the file, but the person has to act before
    /// anything more can be sent (the space's storage is full, its
    /// subscription has ended). The session and the offset already reached
    /// are kept. `next_pending` skips it until [`UploadQueue::release_held`],
    /// which every run the person starts calls first.
    Held,
}

impl QueueState {
    fn from_sql(value: &str) -> Self {
        match value {
            "uploading" => QueueState::Uploading,
            "completed" => QueueState::Completed,
            "failed" => QueueState::Failed,
            "refused" => QueueState::Refused,
            "held" => QueueState::Held,
            _ => QueueState::Queued,
        }
    }

    /// The name stored in SQLite (and serialized to the interface).
    fn as_sql(self) -> &'static str {
        match self {
            QueueState::Queued => "queued",
            QueueState::Uploading => "uploading",
            QueueState::Completed => "completed",
            QueueState::Failed => "failed",
            QueueState::Refused => "refused",
            QueueState::Held => "held",
        }
    }
}

/// One row of the queue.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueEntry {
    /// Row id; stable for the lifetime of the row, used only for ordering.
    pub id: i64,
    pub source_path: PathBuf,
    pub sha256: String,
    pub size: u64,
    pub upload_id: Option<String>,
    pub offset: u64,
    pub state: QueueState,
    pub capture_id: Option<String>,
    /// The technical text of the failure: for logs and the command line's
    /// table, never for the interface.
    pub last_error: Option<String>,
    /// The stable code of the refusal or the hold that put the file aside
    /// (`capture_content_refused`, `storage_quota_reached`, ...): what the
    /// interface looks up, in the person's language.
    pub last_error_code: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    /// What the file is, and what its upload is attributed to.
    pub kind: ResultKind,
    pub target: EntryTarget,
}

fn row_to_entry(row: &Row<'_>) -> rusqlite::Result<QueueEntry> {
    let size: i64 = row.get("size")?;
    let offset: i64 = row.get("offset")?;
    let state: String = row.get("state")?;
    Ok(QueueEntry {
        id: row.get("id")?,
        source_path: PathBuf::from(row.get::<_, String>("source_path")?),
        sha256: row.get("sha256")?,
        size: size.max(0) as u64,
        upload_id: row.get("upload_id")?,
        offset: offset.max(0) as u64,
        state: QueueState::from_sql(&state),
        capture_id: row.get("capture_id")?,
        last_error: row.get("last_error")?,
        last_error_code: row.get("last_error_code")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        kind: ResultKind::from_sql(row.get::<_, Option<String>>("kind")?.as_deref()),
        target: EntryTarget {
            session_id: row.get("session_id")?,
            target_ids: row
                .get::<_, Option<String>>("target_ids")?
                .and_then(|json| serde_json::from_str(&json).ok())
                .unwrap_or_default(),
            variant_of: row.get("variant_of")?,
            source: row.get("source")?,
            metadata: row
                .get::<_, Option<String>>("metadata")?
                .and_then(|json| serde_json::from_str(&json).ok()),
            frame_count: row
                .get::<_, Option<String>>("frames")?
                .and_then(|text| text.parse().ok()),
            plan: row
                .get::<_, Option<String>>("plan")?
                .and_then(|json| serde_json::from_str(&json).ok()),
        },
    })
}

#[derive(Debug, Error)]
pub enum UploadQueueError {
    #[error("could not open the upload queue database at {path}: {source}")]
    Open {
        path: PathBuf,
        source: rusqlite::Error,
    },
    #[error("upload queue database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("no queue entry with sha256 {0:?}")]
    NotFound(String),
    /// The file is not a result and never enters the queue.
    #[error("{path} is not a result Astro Sync sends ({reason:?})")]
    NotAResult { path: PathBuf, reason: NotAResult },
}

/// Summarizes what [`UploadQueue::enqueue_from_dry_run`] did, so a caller
/// can report "N new, M already known" without a second query.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnqueueOutcome {
    pub inserted: usize,
    /// Files this space's queue already holds, in any state (they are left
    /// as they are).
    pub already_known: usize,
    /// Files of the report left out because they are not results (frames,
    /// calibration, raw logs): they stay on this computer.
    #[serde(default)]
    pub left_out: usize,
    /// Among the files already known, those already **sent to this space**
    /// (`completed`): the interface says so at once (« Déjà envoyé le … »)
    /// instead of waiting for a run that has nothing to send.
    #[serde(default)]
    pub already_sent: Vec<AlreadySent>,
}

/// A file enqueued again that this space already has.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlreadySent {
    pub sha256: String,
    pub capture_id: Option<String>,
    /// When it was sent (the row's `updated_at`, ISO 8601 UTC).
    pub sent_at: String,
}

/// The shape of the queue's table, under `name` (the table itself, or the
/// copy a migration fills before renaming it).
fn queue_table(name: &str) -> String {
    format!(
        "CREATE TABLE IF NOT EXISTS {name} (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source_path TEXT NOT NULL,
    sha256 TEXT NOT NULL,
    size INTEGER NOT NULL,
    upload_id TEXT,
    offset INTEGER NOT NULL DEFAULT 0,
    state TEXT NOT NULL DEFAULT 'queued',
    capture_id TEXT,
    last_error TEXT,
    last_error_code TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    kind TEXT,
    session_id TEXT,
    target_ids TEXT,
    variant_of TEXT,
    source TEXT,
    metadata TEXT,
    frames TEXT,
    plan TEXT,
    space_id TEXT
)"
    )
}

/// The shape of the sky annotations' table, under `name`.
fn sky_table(name: &str) -> String {
    format!(
        "CREATE TABLE IF NOT EXISTS {name} (
    sha256 TEXT NOT NULL,
    space_id TEXT,
    capture_id TEXT,
    state TEXT NOT NULL,
    failure TEXT,
    error_code TEXT,
    message TEXT,
    elapsed_ms INTEGER,
    source TEXT,
    updated_at TEXT NOT NULL
)"
    )
}

const FOLDER_MOVES: &str = "CREATE TABLE IF NOT EXISTS folder_moves (
    from_path TEXT PRIMARY KEY,
    to_path TEXT NOT NULL,
    moved_at TEXT NOT NULL
);";

/// A queue file written before `last_error_code` existed keeps its rows and
/// gains the column: the queue holds uploads still to be sent, which is no
/// place to ask a person to start over.
fn add_missing_columns(connection: &Connection) -> rusqlite::Result<()> {
    let columns = connection
        .prepare("PRAGMA table_info(upload_queue)")?
        .query_map([], |row| row.get::<_, String>("name"))?
        .collect::<Result<Vec<_>, _>>()?;
    for column in [
        "last_error_code",
        "kind",
        "session_id",
        "target_ids",
        "variant_of",
        "source",
        "metadata",
        "frames",
        "plan",
        "space_id",
    ] {
        if !columns.iter().any(|name| name == column) {
            connection.execute(
                &format!("ALTER TABLE upload_queue ADD COLUMN {column} TEXT"),
                [],
            )?;
        }
    }
    let sky_columns = connection
        .prepare("PRAGMA table_info(sky_annotation)")?
        .query_map([], |row| row.get::<_, String>("name"))?
        .collect::<Result<Vec<_>, _>>()?;
    if !sky_columns.iter().any(|name| name == "space_id") {
        connection.execute("ALTER TABLE sky_annotation ADD COLUMN space_id TEXT", [])?;
    }
    Ok(())
}

/// The version of the queue's schema (`PRAGMA user_version`) from which a
/// row belongs to a space: the digest is unique **per space**, no longer in
/// the whole file.
const SPACE_SCOPED_VERSION: i64 = 1;

const QUEUE_COLUMNS: &str = "id, source_path, sha256, size, upload_id, offset, state, capture_id, \
    last_error, last_error_code, created_at, updated_at, kind, session_id, target_ids, \
    variant_of, source, metadata, frames, plan, space_id";

const SKY_COLUMNS: &str = "sha256, space_id, capture_id, state, failure, error_code, message, elapsed_ms, source, updated_at";

/// A queue file written before the rows belonged to a space keeps every row,
/// with `space_id` NULL (« before the link »: sent, or meant, for a space
/// this computer no longer knows), and loses the uniqueness of the digest
/// alone: the tables are rebuilt with the new shape (SQLite cannot drop a
/// column constraint), the rows copied over, then renamed back (`ALTER TABLE
/// … RENAME`), in one transaction. The digest becomes unique per space
/// ([`SPACE_INDEXES`]).
fn scope_rows_by_space(connection: &Connection) -> rusqlite::Result<()> {
    let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if version < SPACE_SCOPED_VERSION {
        // Every column exists by now (`add_missing_columns`): the copies take
        // the shape of today, whatever the shape the file was written with.
        let queue_shape = queue_table("upload_queue_scoped");
        let sky_shape = sky_table("sky_annotation_scoped");
        let transaction = connection.unchecked_transaction()?;
        transaction.execute_batch(&format!(
            "DROP TABLE IF EXISTS upload_queue_scoped;
             DROP TABLE IF EXISTS sky_annotation_scoped;
             {queue_shape};
             INSERT INTO upload_queue_scoped ({QUEUE_COLUMNS})
                 SELECT {QUEUE_COLUMNS} FROM upload_queue;
             DROP TABLE upload_queue;
             ALTER TABLE upload_queue_scoped RENAME TO upload_queue;
             {sky_shape};
             INSERT INTO sky_annotation_scoped ({SKY_COLUMNS})
                 SELECT {SKY_COLUMNS} FROM sky_annotation;
             DROP TABLE sky_annotation;
             ALTER TABLE sky_annotation_scoped RENAME TO sky_annotation;
             PRAGMA user_version = {SPACE_SCOPED_VERSION};"
        ))?;
        transaction.commit()?;
    }
    connection.execute_batch(SPACE_INDEXES)
}

/// One row per digest **and space**; the rows without a space (NULL) are one
/// space of their own (`COALESCE`: SQLite holds two NULLs distinct).
const SPACE_INDEXES: &str = "CREATE UNIQUE INDEX IF NOT EXISTS upload_queue_space_sha256
    ON upload_queue (COALESCE(space_id, ''), sha256);
CREATE UNIQUE INDEX IF NOT EXISTS sky_annotation_space_sha256
    ON sky_annotation (COALESCE(space_id, ''), sha256);";

/// Creates or brings up to date the tables of a queue file.
fn prepare_schema(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(&format!(
        "{};\n{};\n{FOLDER_MOVES}",
        queue_table("upload_queue"),
        sky_table("sky_annotation")
    ))?;
    add_missing_columns(connection)?;
    scope_rows_by_space(connection)
}

/// Rows an older build enqueued that are not results (a FITS, a XISF, a
/// raw log, a calibration frame...) are dropped, whatever their state:
/// none of them is ever sent. Returns how many.
fn drop_rows_that_are_not_results(connection: &Connection) -> rusqlite::Result<usize> {
    let rows: Vec<(i64, String)> = connection
        .prepare("SELECT id, source_path FROM upload_queue")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<_, _>>()?;
    let mut dropped = 0;
    for (id, path) in rows {
        if result_kind_of(Path::new(&path)).is_err() {
            dropped += connection.execute("DELETE FROM upload_queue WHERE id = ?1", params![id])?;
        }
    }
    Ok(dropped)
}

/// A persistent, SQLite-backed upload queue at a caller-chosen file path,
/// seen from one space ([`UploadQueue::for_space`]).
pub struct UploadQueue {
    connection: Connection,
    /// The space this queue reads and writes the rows of; `None`: the rows
    /// without a space.
    space_id: Option<String>,
}

impl UploadQueue {
    /// Opens (creating if needed) the queue database at `path`, creating
    /// its schema on first use. Safe to call repeatedly on the same path
    /// across process restarts -- that is the entire point of persistence.
    pub fn open(path: &Path) -> Result<Self, UploadQueueError> {
        let connection = Connection::open(path).map_err(|source| UploadQueueError::Open {
            path: path.to_path_buf(),
            source,
        })?;
        prepare_schema(&connection)?;
        drop_rows_that_are_not_results(&connection)?;
        let queue = Self {
            connection,
            space_id: None,
        };
        queue.follow_moved_sources()?;
        Ok(queue)
    }

    /// The same queue, seen from the space `space_id` (`space.json` ›
    /// `link.space.id`): every read, every write and every « already known »
    /// is that space's. `None`: the rows without a space (the command line,
    /// a computer paired without an e-mail sign-in).
    #[must_use]
    pub fn for_space(mut self, space_id: Option<String>) -> Self {
        self.space_id = space_id.filter(|id| !id.is_empty());
        self
    }

    /// The space this queue is seen from.
    pub fn space_id(&self) -> Option<&str> {
        self.space_id.as_deref()
    }

    /// Opens an in-memory queue; only ever used by this module's own tests.
    #[cfg(test)]
    fn open_in_memory() -> Result<Self, UploadQueueError> {
        let connection = Connection::open_in_memory()?;
        prepare_schema(&connection)?;
        Ok(Self {
            connection,
            space_id: None,
        })
    }

    /// The rows finished for good (`completed`): digest → capture, what the
    /// resume of a night reads as « sent from this computer ».
    pub fn completed(&self) -> Result<Vec<QueueEntry>, UploadQueueError> {
        Ok(self
            .list()?
            .into_iter()
            .filter(|entry| entry.state == QueueState::Completed)
            .collect())
    }

    /// The rows finished for good in **every** space: what the working
    /// folder -- one for every space -- knows as sent from this computer.
    pub fn completed_everywhere(&self) -> Result<Vec<QueueEntry>, UploadQueueError> {
        let mut statement = self
            .connection
            .prepare("SELECT * FROM upload_queue WHERE state = 'completed' ORDER BY id ASC")?;
        let rows = statement
            .query_map([], row_to_entry)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Enqueues the results among the entries of `report` (a final image, a
    /// stacked image: [`result_kind_of`]) that are not already known by
    /// their `sha256`; every other file is left out and counted
    /// ([`EnqueueOutcome::left_out`]). A file already present (in any state,
    /// including `completed` or `failed`) is left untouched: re-running a
    /// dry-run and enqueueing it again must never reset progress or forget a
    /// refusal.
    pub fn enqueue_from_dry_run(
        &self,
        report: &DryRunReport,
    ) -> Result<EnqueueOutcome, UploadQueueError> {
        let mut outcome = EnqueueOutcome::default();
        for entry in &report.entries {
            let source_path = report.source_root.join(&entry.source_relative_path);
            let Ok(kind) = result_kind_of(Path::new(&entry.source_relative_path)) else {
                outcome.left_out += 1;
                continue;
            };
            let file = ResultFile {
                path: source_path,
                sha256: entry.sha256.clone(),
                size: entry.bytes,
                kind,
                target: EntryTarget::default(),
            };
            if self.insert(&file)? {
                outcome.inserted += 1;
            } else {
                self.count_known(&file.sha256, &mut outcome)?;
            }
        }
        Ok(outcome)
    }

    /// A file this space's queue already holds: counted, and named when it
    /// was already sent.
    fn count_known(
        &self,
        sha256: &str,
        outcome: &mut EnqueueOutcome,
    ) -> Result<(), UploadQueueError> {
        outcome.already_known += 1;
        if let Some(entry) = self.find_by_sha256(sha256)?
            && entry.state == QueueState::Completed
        {
            outcome.already_sent.push(AlreadySent {
                sha256: entry.sha256,
                capture_id: entry.capture_id,
                sent_at: entry.updated_at,
            });
        }
        Ok(())
    }

    /// Enqueues `files`, results each with what its upload is attributed
    /// to. Refuses the whole call, inserting nothing, when one of them is
    /// not a result ([`UploadQueueError::NotAResult`]). The kind is the
    /// caller's (a progression image lives in Astro Sync's own data).
    pub fn enqueue_results(
        &self,
        files: &[ResultFile],
    ) -> Result<EnqueueOutcome, UploadQueueError> {
        for file in files {
            if let Err(reason) = result_kind_of(&file.path) {
                return Err(UploadQueueError::NotAResult {
                    path: file.path.clone(),
                    reason,
                });
            }
        }
        let mut outcome = EnqueueOutcome::default();
        for file in files {
            // A caller may still name a folder that was renamed since the
            // image was made (the provisional folder of a plan): the row is
            // written with the file's place now.
            let inserted = match self.moved_source(&file.path)? {
                Some(path) => self.insert(&ResultFile {
                    path,
                    ..file.clone()
                })?,
                None => self.insert(file)?,
            };
            if inserted {
                outcome.inserted += 1;
            } else {
                self.count_known(&file.sha256, &mut outcome)?;
            }
        }
        Ok(outcome)
    }

    fn insert(&self, file: &ResultFile) -> Result<bool, UploadQueueError> {
        let now = now_iso8601_utc();
        let target_ids = (!file.target.target_ids.is_empty())
            .then(|| serde_json::to_string(&file.target.target_ids).unwrap_or_default());
        let changed = self.connection.execute(
            "INSERT INTO upload_queue
                (source_path, sha256, size, offset, state, created_at, updated_at,
                 kind, session_id, target_ids, variant_of, source, metadata, frames, plan,
                 space_id)
             VALUES (?1, ?2, ?3, 0, 'queued', ?4, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT DO NOTHING",
            params![
                file.path.to_string_lossy(),
                file.sha256,
                file.size as i64,
                now,
                file.kind.as_sql(),
                file.target.session_id,
                target_ids,
                file.target.variant_of,
                file.target.source,
                file.target
                    .metadata
                    .as_ref()
                    .and_then(|m| serde_json::to_string(m).ok()),
                file.target.frame_count.map(|n| n.to_string()),
                file.target
                    .plan
                    .as_ref()
                    .and_then(|p| serde_json::to_string(p).ok()),
                self.space_id,
            ],
        )?;
        Ok(changed == 1)
    }

    /// The oldest entry still worth acting on: `queued` or `uploading`
    /// (resumable), ordered by insertion order. Never `completed` (already
    /// done), `failed` or `refused` (terminal; retrying is a deliberate,
    /// separate action outside this crate's scope) nor `held` (waiting for the
    /// person: [`UploadQueue::release_held`]).
    pub fn next_pending(&self) -> Result<Option<QueueEntry>, UploadQueueError> {
        self.connection
            .query_row(
                "SELECT * FROM upload_queue
                 WHERE state IN ('queued', 'uploading') AND space_id IS ?1
                 ORDER BY id ASC LIMIT 1",
                params![self.space_id],
                row_to_entry,
            )
            .optional()
            .map_err(UploadQueueError::from)
    }

    /// Records that a session exists and bytes have advanced to `offset`.
    /// Moves the entry to `uploading` if it was not already. Called after
    /// every successful create/status/`PUT` response so a crash leaves the
    /// last known-durable offset on disk, never a stale one.
    pub fn mark_progress(
        &self,
        sha256: &str,
        upload_id: &str,
        offset: u64,
    ) -> Result<(), UploadQueueError> {
        let changed = self.connection.execute(
            "UPDATE upload_queue
             SET upload_id = ?2, offset = ?3, state = 'uploading',
                 last_error = NULL, last_error_code = NULL, updated_at = ?4
             WHERE sha256 = ?1 AND space_id IS ?5",
            params![
                sha256,
                upload_id,
                offset as i64,
                now_iso8601_utc(),
                self.space_id
            ],
        )?;
        if changed == 0 {
            return Err(UploadQueueError::NotFound(sha256.to_string()));
        }
        Ok(())
    }

    /// Marks an entry finalized: a capture exists, and it will never be
    /// returned by `next_pending` again.
    pub fn mark_completed(&self, sha256: &str, capture_id: &str) -> Result<(), UploadQueueError> {
        let changed = self.connection.execute(
            "UPDATE upload_queue
             SET state = 'completed', capture_id = ?2, last_error = NULL,
                 last_error_code = NULL, updated_at = ?3
             WHERE sha256 = ?1 AND space_id IS ?4",
            params![sha256, capture_id, now_iso8601_utc(), self.space_id],
        )?;
        if changed == 0 {
            return Err(UploadQueueError::NotFound(sha256.to_string()));
        }
        Ok(())
    }

    /// Notes on a completed entry that its image **replaced** the one the
    /// capture had (the phone's photo of the session, the Journal's
    /// `replaced`): the interface says it on the row, in the person's
    /// language ([`REPLACED_NOTE`]). Touches nothing but the code, and only
    /// on a completed entry.
    pub fn mark_replaced(&self, sha256: &str) -> Result<(), UploadQueueError> {
        self.connection.execute(
            "UPDATE upload_queue SET last_error_code = ?2
             WHERE sha256 = ?1 AND state = 'completed' AND space_id IS ?3",
            params![sha256, REPLACED_NOTE, self.space_id],
        )?;
        Ok(())
    }

    /// Marks an entry as a terminal failure with a human-readable `reason`.
    /// `next_pending` will not return it again.
    pub fn mark_failed(&self, sha256: &str, reason: &str) -> Result<(), UploadQueueError> {
        self.set_aside(sha256, QueueState::Failed, None, reason)
    }

    /// Like [`UploadQueue::mark_failed`], with the stable `code` of the
    /// failure (`source_file_missing`...), which the interface looks up in the
    /// person's language and uses to offer « Réessayer ».
    pub fn mark_failed_coded(
        &self,
        sha256: &str,
        code: &str,
        reason: &str,
    ) -> Result<(), UploadQueueError> {
        self.set_aside(sha256, QueueState::Failed, Some(code), reason)
    }

    /// The service refuses this file for good: the entry leaves the pending
    /// set as [`QueueState::Refused`], with the stable `code` of the refusal
    /// (what the interface says, in the person's language) and the technical
    /// `reason`. The file on the computer is not touched.
    pub fn mark_refused(
        &self,
        sha256: &str,
        code: &str,
        reason: &str,
    ) -> Result<(), UploadQueueError> {
        self.set_aside(sha256, QueueState::Refused, Some(code), reason)
    }

    /// The person has to act before this file can go on (storage full,
    /// subscription ended): the entry becomes [`QueueState::Held`] with its
    /// session and its offset kept, and waits for [`UploadQueue::release_held`].
    pub fn hold(&self, sha256: &str, code: &str, reason: &str) -> Result<(), UploadQueueError> {
        self.set_aside(sha256, QueueState::Held, Some(code), reason)
    }

    fn set_aside(
        &self,
        sha256: &str,
        state: QueueState,
        code: Option<&str>,
        reason: &str,
    ) -> Result<(), UploadQueueError> {
        let changed = self.connection.execute(
            "UPDATE upload_queue
             SET state = ?2, last_error = ?3, last_error_code = ?4, updated_at = ?5
             WHERE sha256 = ?1 AND space_id IS ?6",
            params![
                sha256,
                state.as_sql(),
                reason,
                code,
                now_iso8601_utc(),
                self.space_id
            ],
        )?;
        if changed == 0 {
            return Err(UploadQueueError::NotFound(sha256.to_string()));
        }
        Ok(())
    }

    /// Puts every `held` entry back in the queue -- `uploading` when a
    /// session was already open (it resumes from its offset), `queued`
    /// otherwise -- and returns how many. Called by every run the person
    /// starts: acting (freeing space, renewing) and starting again is the
    /// only thing that ends a hold, and a hold that still stands is simply
    /// declared again by the service on the first call.
    pub fn release_held(&self) -> Result<usize, UploadQueueError> {
        Ok(self.connection.execute(
            "UPDATE upload_queue
             SET state = CASE WHEN upload_id IS NULL THEN 'queued' ELSE 'uploading' END,
                 updated_at = ?1
             WHERE state = 'held' AND space_id IS ?2",
            params![now_iso8601_utc(), self.space_id],
        )?)
    }

    /// Resets an entry back to `offset` 0, `queued`, and clears its session
    /// id -- the recovery step after `422 upload_hash_mismatch`, whose
    /// contract requires starting the transfer over from scratch since any
    /// chunk could have been the corrupt one.
    pub fn reset_for_restart(&self, sha256: &str) -> Result<(), UploadQueueError> {
        let changed = self.connection.execute(
            "UPDATE upload_queue
             SET state = 'queued', upload_id = NULL, offset = 0,
                 last_error = NULL, last_error_code = NULL, updated_at = ?2
             WHERE sha256 = ?1 AND space_id IS ?3",
            params![sha256, now_iso8601_utc(), self.space_id],
        )?;
        if changed == 0 {
            return Err(UploadQueueError::NotFound(sha256.to_string()));
        }
        Ok(())
    }

    /// Drops the rows not finished (`completed` stays: it is the record of
    /// what was sent) whose file was deleted by "Clean", which would only
    /// fail at their turn. Returns how many.
    pub fn forget_unfinished_files(&self, paths: &[PathBuf]) -> Result<usize, UploadQueueError> {
        let mut dropped = 0;
        for path in paths {
            dropped += self.connection.execute(
                "DELETE FROM upload_queue WHERE source_path = ?1 AND state != 'completed'",
                params![path.to_string_lossy()],
            )?;
        }
        Ok(dropped)
    }

    /// A provisional night (or plan) folder was renamed once the space
    /// created its session: the rows whose file lies in `from` now name it in
    /// `to`, and the provisional identifiers they carry (the session, the
    /// plan and its nights) the ones the space gave. Returns how many rows
    /// changed.
    pub fn rebase_provisional(
        &self,
        from: &Path,
        to: &Path,
        ids: &[(String, String)],
    ) -> Result<usize, UploadQueueError> {
        let rows: Vec<(i64, String, Option<String>, Option<String>)> = self
            .connection
            .prepare("SELECT id, source_path, session_id, plan FROM upload_queue")?
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?
            .collect::<Result<_, _>>()?;
        let swap = |text: &str| {
            ids.iter()
                .fold(text.to_string(), |text, (old, new)| text.replace(old, new))
        };
        if from != to {
            self.record_move(from, to)?;
        }
        let mut changed = 0;
        for (id, path, session, plan) in rows {
            let moved = Path::new(&path)
                .strip_prefix(from)
                .ok()
                .map(|rest| to.join(rest).to_string_lossy().into_owned())
                .unwrap_or_else(|| path.clone());
            let session_now = session.as_deref().map(swap);
            let plan_now = plan.as_deref().map(swap);
            if moved == path && session_now == session && plan_now == plan {
                continue;
            }
            changed += self.connection.execute(
                "UPDATE upload_queue SET source_path = ?1, session_id = ?2, plan = ?3 WHERE id = ?4",
                params![moved, session_now, plan_now, id],
            )?;
        }
        Ok(changed)
    }

    /// Keeps that the folder `from` is now `to`, so a row (or an image still
    /// held by the interface) that names the old place finds its file in the
    /// new one ([`UploadQueue::moved_source`]).
    fn record_move(&self, from: &Path, to: &Path) -> Result<(), UploadQueueError> {
        self.connection.execute(
            "INSERT OR REPLACE INTO folder_moves (from_path, to_path, moved_at)
             VALUES (?1, ?2, ?3)",
            params![
                from.to_string_lossy(),
                to.to_string_lossy(),
                now_iso8601_utc()
            ],
        )?;
        Ok(())
    }

    /// Where the file `path` is now when it is not where it was named: the
    /// folders renamed since ([`UploadQueue::rebase_provisional`]) are
    /// followed, one after the other. `None` when the file is there already
    /// or when no renamed folder leads to a file that exists.
    pub fn moved_source(&self, path: &Path) -> Result<Option<PathBuf>, UploadQueueError> {
        if path.exists() {
            return Ok(None);
        }
        let moves: Vec<(PathBuf, PathBuf)> = self
            .connection
            .prepare(
                "SELECT from_path, to_path FROM folder_moves ORDER BY moved_at ASC, rowid ASC",
            )?
            .query_map([], |row| {
                Ok((
                    PathBuf::from(row.get::<_, String>(0)?),
                    PathBuf::from(row.get::<_, String>(1)?),
                ))
            })?
            .collect::<Result<_, _>>()?;
        let mut current = path.to_path_buf();
        // A move may be followed by another (a night folder, then the folder
        // of its plan); the bound keeps a cycle from looping.
        for _ in 0..=moves.len() {
            let Some(next) = moves
                .iter()
                .filter_map(|(from, to)| {
                    current
                        .strip_prefix(from)
                        .ok()
                        .map(|rest| (from.as_os_str().len(), to.join(rest)))
                })
                .max_by_key(|(length, _)| *length)
                .map(|(_, next)| next)
            else {
                break;
            };
            if next == current {
                break;
            }
            current = next;
            if current.exists() {
                return Ok(Some(current));
            }
        }
        Ok(None)
    }

    /// The row `sha256` now names `path` for its file.
    pub fn relocate(&self, sha256: &str, path: &Path) -> Result<(), UploadQueueError> {
        let changed = self.connection.execute(
            "UPDATE upload_queue SET source_path = ?2, updated_at = ?3
             WHERE sha256 = ?1 AND space_id IS ?4",
            params![
                sha256,
                path.to_string_lossy(),
                now_iso8601_utc(),
                self.space_id
            ],
        )?;
        if changed == 0 {
            return Err(UploadQueueError::NotFound(sha256.to_string()));
        }
        Ok(())
    }

    /// Every row not finished whose file is not where it was named but is in
    /// a folder renamed since now names it there; a row set aside as
    /// `failed` for that file (its file could not be read) goes back to
    /// `queued`, since what failed it is mended. Returns how many rows
    /// changed. Run each time the queue is opened.
    pub fn follow_moved_sources(&self) -> Result<usize, UploadQueueError> {
        // A renamed folder is the computer's, not a space's: the rows of every
        // space follow it.
        let rows: Vec<(i64, String, String)> = self
            .connection
            .prepare(
                "SELECT id, source_path, state FROM upload_queue
                 WHERE state IN ('queued', 'uploading', 'held', 'failed')",
            )?
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<Result<_, _>>()?;
        let mut changed = 0;
        for (id, path, state) in rows {
            let Some(found) = self.moved_source(Path::new(&path))? else {
                continue;
            };
            let now = now_iso8601_utc();
            self.connection.execute(
                "UPDATE upload_queue SET source_path = ?2, updated_at = ?3 WHERE id = ?1",
                params![id, found.to_string_lossy(), now],
            )?;
            if state == "failed" {
                self.connection.execute(
                    "UPDATE upload_queue
                     SET state = 'queued', upload_id = NULL, offset = 0,
                         last_error = NULL, last_error_code = NULL, updated_at = ?2
                     WHERE id = ?1",
                    params![id, now],
                )?;
            }
            changed += 1;
        }
        Ok(changed)
    }

    /// « Réessayer » on a file set aside as `failed` because its file could
    /// not be found: the row goes back to `queued` (from the start, since
    /// whatever session it had is not worth trusting), naming the file where
    /// it is now when a renamed folder leads to it.
    pub fn requeue_failed(&self, sha256: &str) -> Result<(), UploadQueueError> {
        let entry = self
            .find_by_sha256(sha256)?
            .ok_or_else(|| UploadQueueError::NotFound(sha256.to_string()))?;
        if entry.state != QueueState::Failed {
            return Ok(());
        }
        if let Some(found) = self.moved_source(&entry.source_path)? {
            self.relocate(sha256, &found)?;
        }
        self.reset_for_restart(sha256)
    }

    /// « Renvoyer » on a file already sent to this space (its capture may
    /// have been deleted on the web since): the row goes back to `queued`
    /// from the start. The service answers with the capture it still holds
    /// when it has the bytes (`deduplicated`), or makes a new one. Nothing
    /// for a row that is not `completed`; returns whether it was requeued.
    pub fn requeue_sent(&self, sha256: &str) -> Result<bool, UploadQueueError> {
        match self.find_by_sha256(sha256)? {
            Some(entry) if entry.state == QueueState::Completed => {
                self.reset_for_restart(sha256)?;
                Ok(true)
            }
            Some(_) => Ok(false),
            None => Err(UploadQueueError::NotFound(sha256.to_string())),
        }
    }

    /// Every row of this space, oldest first -- the full picture `queue
    /// status` prints.
    pub fn list(&self) -> Result<Vec<QueueEntry>, UploadQueueError> {
        let mut statement = self
            .connection
            .prepare("SELECT * FROM upload_queue WHERE space_id IS ?1 ORDER BY id ASC")?;
        let rows = statement
            .query_map(params![self.space_id], row_to_entry)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// The rows written before the queue knew its spaces (`space_id` NULL),
    /// oldest first, as a queue seen from a space shows them apart (« Avant
    /// la liaison »). Empty for a queue seen from no space: they are its own
    /// rows ([`UploadQueue::list`]).
    pub fn list_before_link(&self) -> Result<Vec<QueueEntry>, UploadQueueError> {
        if self.space_id.is_none() {
            return Ok(Vec::new());
        }
        let mut statement = self
            .connection
            .prepare("SELECT * FROM upload_queue WHERE space_id IS NULL ORDER BY id ASC")?;
        let rows = statement
            .query_map([], row_to_entry)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// « Effacer l'historique », for the rows before the link: they are
    /// deleted (every state: none of them is ever sent from a queue seen from
    /// a space), with their sky annotations. Nothing for a queue seen from no
    /// space. Returns how many rows went.
    pub fn forget_before_link(&self) -> Result<usize, UploadQueueError> {
        if self.space_id.is_none() {
            return Ok(0);
        }
        let transaction = self.connection.unchecked_transaction()?;
        let removed = transaction.execute("DELETE FROM upload_queue WHERE space_id IS NULL", [])?;
        transaction.execute("DELETE FROM sky_annotation WHERE space_id IS NULL", [])?;
        transaction.commit()?;
        Ok(removed)
    }

    /// Looks up a single entry by its content hash; used by the uploader to
    /// re-read the freshest row after a mutation.
    pub fn find_by_sha256(&self, sha256: &str) -> Result<Option<QueueEntry>, UploadQueueError> {
        self.connection
            .query_row(
                "SELECT * FROM upload_queue WHERE sha256 = ?1 AND space_id IS ?2",
                params![sha256, self.space_id],
                row_to_entry,
            )
            .optional()
            .map_err(UploadQueueError::from)
    }

    /// Keeps what became of the sky annotations of an uploaded image, beside
    /// its row (the sibling table `sky_annotation`, one row per digest; a
    /// newer state replaces the older).
    pub fn set_sky(&self, annotation: &SkyAnnotation) -> Result<(), UploadQueueError> {
        let failure = annotation
            .failure
            .as_ref()
            .and_then(|failure| serde_json::to_string(failure).ok());
        let source = annotation
            .source
            .and_then(|source| serde_json::to_value(source).ok())
            .and_then(|value| value.as_str().map(str::to_string));
        self.connection.execute(
            "INSERT OR REPLACE INTO sky_annotation
                (sha256, capture_id, state, failure, error_code, message, elapsed_ms, source,
                 updated_at, space_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                annotation.sha256,
                annotation.capture_id,
                annotation.state.as_sql(),
                failure,
                annotation.error_code,
                annotation.message,
                annotation
                    .elapsed_ms
                    .map(|ms| i64::try_from(ms).unwrap_or(i64::MAX)),
                source,
                annotation.updated_at,
                self.space_id,
            ],
        )?;
        Ok(())
    }

    /// The annotations of every image of this space that has any, oldest
    /// first.
    pub fn sky_annotations(&self) -> Result<Vec<SkyAnnotation>, UploadQueueError> {
        let mut statement = self.connection.prepare(
            "SELECT * FROM sky_annotation WHERE space_id IS ?1
             ORDER BY updated_at ASC, rowid ASC",
        )?;
        let rows = statement
            .query_map(params![self.space_id], |row| {
                let state: String = row.get("state")?;
                let elapsed: Option<i64> = row.get("elapsed_ms")?;
                Ok(SkyAnnotation {
                    sha256: row.get("sha256")?,
                    capture_id: row.get("capture_id")?,
                    state: SkyState::from_sql(&state),
                    failure: row
                        .get::<_, Option<String>>("failure")?
                        .and_then(|json| serde_json::from_str(&json).ok()),
                    error_code: row.get("error_code")?,
                    message: row.get("message")?,
                    elapsed_ms: elapsed.map(|ms| ms.max(0) as u64),
                    source: row.get::<_, Option<String>>("source")?.and_then(|text| {
                        serde_json::from_value(serde_json::Value::String(text)).ok()
                    }),
                    updated_at: row.get("updated_at")?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// The annotations of one uploaded image.
    pub fn sky_of(&self, sha256: &str) -> Result<Option<SkyAnnotation>, UploadQueueError> {
        Ok(self
            .sky_annotations()?
            .into_iter()
            .find(|annotation| annotation.sha256 == sha256))
    }

    /// Ends the resolutions an application that stopped left `running`: they
    /// are failures the person can try again ([`crate::sky_after_upload::INTERRUPTED`]).
    /// Called once when the application starts, never while it works.
    pub fn end_interrupted_sky(&self) -> Result<usize, UploadQueueError> {
        Ok(self.connection.execute(
            "UPDATE sky_annotation
             SET state = 'failed', error_code = ?1, updated_at = ?2
             WHERE state = 'running'",
            params![crate::sky_after_upload::INTERRUPTED, now_iso8601_utc()],
        )?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dry_run::{DryRunEntry, DryRunReport};
    use crate::package::AssetKind;
    use crate::scanner::FileClass;
    use std::path::PathBuf;

    fn report_with(entries: &[(&str, &str, u64)]) -> DryRunReport {
        DryRunReport {
            source_root: PathBuf::from("/observations/night-1"),
            entries: entries
                .iter()
                .map(|(relative, sha256, bytes)| DryRunEntry {
                    source_relative_path: (*relative).to_string(),
                    class: FileClass::Image,
                    asset_kind: AssetKind::Image,
                    package_relative_path: format!("images/{relative}"),
                    bytes: *bytes,
                    sha256: (*sha256).to_string(),
                })
                .collect(),
            total_bytes: entries.iter().map(|(_, _, bytes)| bytes).sum(),
        }
    }

    #[test]
    fn enqueue_is_idempotent_on_sha256() {
        let queue = UploadQueue::open_in_memory().unwrap();
        let report = report_with(&[("M31/final/frame-1.tif", "aaa", 100)]);

        let first = queue.enqueue_from_dry_run(&report).unwrap();
        assert_eq!(
            first,
            EnqueueOutcome {
                inserted: 1,
                already_known: 0,
                left_out: 0,
                already_sent: Vec::new(),
            }
        );

        let second = queue.enqueue_from_dry_run(&report).unwrap();
        assert_eq!(
            second,
            EnqueueOutcome {
                inserted: 0,
                already_known: 1,
                left_out: 0,
                already_sent: Vec::new(),
            }
        );

        assert_eq!(queue.list().unwrap().len(), 1);
    }

    #[test]
    fn a_completed_entry_is_never_returned_by_next_pending_even_after_re_enqueue() {
        let queue = UploadQueue::open_in_memory().unwrap();
        let report = report_with(&[("M31/final/frame-1.tif", "aaa", 100)]);
        queue.enqueue_from_dry_run(&report).unwrap();
        queue.mark_progress("aaa", "upload-1", 100).unwrap();
        queue.mark_completed("aaa", "capture-1").unwrap();

        // Re-running the dry-run and enqueueing again (e.g. a second scan
        // of the same folder) must not resurrect the completed row.
        queue.enqueue_from_dry_run(&report).unwrap();

        assert_eq!(queue.next_pending().unwrap(), None);
        let entry = queue.find_by_sha256("aaa").unwrap().unwrap();
        assert_eq!(entry.state, QueueState::Completed);
        assert_eq!(entry.capture_id.as_deref(), Some("capture-1"));
    }

    #[test]
    fn a_failed_entry_is_never_returned_by_next_pending_within_the_same_run() {
        let queue = UploadQueue::open_in_memory().unwrap();
        let report = report_with(&[("M31/final/too-big.tif", "bbb", 10)]);
        queue.enqueue_from_dry_run(&report).unwrap();
        queue.mark_failed("bbb", "413 file_too_large").unwrap();

        assert_eq!(queue.next_pending().unwrap(), None);
    }

    #[test]
    fn next_pending_returns_queued_and_uploading_oldest_first() {
        let queue = UploadQueue::open_in_memory().unwrap();
        let report = report_with(&[
            ("M31/final/a.tif", "aaa", 10),
            ("M31/final/b.tif", "bbb", 20),
            ("M31/final/c.tif", "ccc", 30),
        ]);
        queue.enqueue_from_dry_run(&report).unwrap();
        queue.mark_completed("aaa", "capture-a").unwrap();
        queue.mark_progress("bbb", "upload-b", 5).unwrap();

        let next = queue.next_pending().unwrap().unwrap();
        assert_eq!(next.sha256, "bbb");
        assert_eq!(next.state, QueueState::Uploading);
        assert_eq!(next.offset, 5);
    }

    #[test]
    fn state_and_offset_survive_reopening_the_same_database_file() {
        let directory = crate::test_support::TempDir::new("upload-queue-persistence");
        let db_path = directory.path().join("queue.sqlite");

        {
            let queue = UploadQueue::open(&db_path).unwrap();
            let report = report_with(&[("M31/final/frame-1.tif", "aaa", 1000)]);
            queue.enqueue_from_dry_run(&report).unwrap();
            queue.mark_progress("aaa", "upload-1", 400).unwrap();
        }

        let reopened = UploadQueue::open(&db_path).unwrap();
        let entry = reopened.find_by_sha256("aaa").unwrap().unwrap();
        assert_eq!(entry.offset, 400);
        assert_eq!(entry.upload_id.as_deref(), Some("upload-1"));
        assert_eq!(entry.state, QueueState::Uploading);
    }

    #[test]
    fn reset_for_restart_clears_offset_and_session_and_requeues() {
        let queue = UploadQueue::open_in_memory().unwrap();
        let report = report_with(&[("M31/final/frame-1.tif", "aaa", 1000)]);
        queue.enqueue_from_dry_run(&report).unwrap();
        queue.mark_progress("aaa", "upload-1", 900).unwrap();

        queue.reset_for_restart("aaa").unwrap();

        let entry = queue.find_by_sha256("aaa").unwrap().unwrap();
        assert_eq!(entry.offset, 0);
        assert_eq!(entry.upload_id, None);
        assert_eq!(entry.state, QueueState::Queued);
    }

    #[test]
    fn a_refused_entry_leaves_the_pending_set_with_the_code_of_its_refusal() {
        let queue = UploadQueue::open_in_memory().unwrap();
        let report = report_with(&[
            ("M31/final/drawing.png", "sss", 10),
            ("M31/final/frame.tif", "fff", 20),
        ]);
        queue.enqueue_from_dry_run(&report).unwrap();
        queue
            .mark_refused("sss", "capture_content_refused", "415: an SVG document")
            .unwrap();

        let refused = queue.find_by_sha256("sss").unwrap().unwrap();
        assert_eq!(refused.state, QueueState::Refused);
        assert_eq!(
            refused.last_error_code.as_deref(),
            Some("capture_content_refused")
        );
        assert_eq!(refused.last_error.as_deref(), Some("415: an SVG document"));
        // The other file is next, and the refused one never comes back: not
        // even after `release_held`, which is for a hold only.
        assert_eq!(queue.next_pending().unwrap().unwrap().sha256, "fff");
        assert_eq!(queue.release_held().unwrap(), 0);
        queue.mark_completed("fff", "capture-f").unwrap();
        assert_eq!(queue.next_pending().unwrap(), None);
    }

    #[test]
    fn a_held_entry_keeps_its_session_and_waits_for_the_person() {
        let queue = UploadQueue::open_in_memory().unwrap();
        let report = report_with(&[
            ("M31/final/half.tif", "hhh", 1000),
            ("M31/final/untouched.tif", "uuu", 2000),
        ]);
        queue.enqueue_from_dry_run(&report).unwrap();
        queue.mark_progress("hhh", "upload-h", 400).unwrap();
        queue
            .hold("hhh", "storage_quota_reached", "409 storage_quota_reached")
            .unwrap();
        queue
            .hold("uuu", "storage_quota_reached", "409 storage_quota_reached")
            .unwrap();

        let held = queue.find_by_sha256("hhh").unwrap().unwrap();
        assert_eq!(held.state, QueueState::Held);
        assert_eq!(held.upload_id.as_deref(), Some("upload-h"));
        assert_eq!(held.offset, 400, "the bytes already accepted are kept");
        assert_eq!(
            queue.next_pending().unwrap(),
            None,
            "a hold ends the file's turn until the person acts"
        );

        // The person acted and started again: the file with a session goes
        // on from its offset, the other one starts.
        assert_eq!(queue.release_held().unwrap(), 2);
        let resumed = queue.find_by_sha256("hhh").unwrap().unwrap();
        assert_eq!(resumed.state, QueueState::Uploading);
        assert_eq!(resumed.offset, 400);
        assert_eq!(
            queue.find_by_sha256("uuu").unwrap().unwrap().state,
            QueueState::Queued
        );
        assert_eq!(queue.next_pending().unwrap().unwrap().sha256, "hhh");
    }

    #[test]
    fn progress_forgets_an_earlier_refusal_or_hold() {
        let queue = UploadQueue::open_in_memory().unwrap();
        queue
            .enqueue_from_dry_run(&report_with(&[("M31/final/frame.tif", "aaa", 100)]))
            .unwrap();
        queue.hold("aaa", "storage_quota_reached", "409").unwrap();
        queue.release_held().unwrap();
        queue.mark_progress("aaa", "upload-a", 50).unwrap();
        let entry = queue.find_by_sha256("aaa").unwrap().unwrap();
        assert_eq!(entry.last_error, None);
        assert_eq!(entry.last_error_code, None);
    }

    #[test]
    fn a_queue_file_written_before_the_error_code_gets_the_column_and_keeps_its_rows() {
        let directory = crate::test_support::TempDir::new("upload-queue-old-schema");
        let db_path = directory.path().join("queue.sqlite");
        {
            let old = Connection::open(&db_path).unwrap();
            old.execute_batch(
                "CREATE TABLE upload_queue (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    source_path TEXT NOT NULL,
                    sha256 TEXT NOT NULL UNIQUE,
                    size INTEGER NOT NULL,
                    upload_id TEXT,
                    offset INTEGER NOT NULL DEFAULT 0,
                    state TEXT NOT NULL DEFAULT 'queued',
                    capture_id TEXT,
                    last_error TEXT,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                INSERT INTO upload_queue
                    (source_path, sha256, size, upload_id, offset, state, created_at, updated_at)
                VALUES ('/night/M31/final/frame.tif', 'old', 900, 'upload-old', 300, 'uploading', 't', 't');",
            )
            .unwrap();
        }

        let queue = UploadQueue::open(&db_path).unwrap();
        let entry = queue.find_by_sha256("old").unwrap().unwrap();
        assert_eq!(entry.state, QueueState::Uploading);
        assert_eq!(entry.offset, 300);
        assert_eq!(entry.last_error_code, None);
        queue.mark_refused("old", "file_too_large", "413").unwrap();
        assert_eq!(
            queue
                .find_by_sha256("old")
                .unwrap()
                .unwrap()
                .last_error_code
                .as_deref(),
            Some("file_too_large")
        );
        // Opening it again does not try to add the column twice.
        drop(queue);
        UploadQueue::open(&db_path).unwrap();
    }

    #[test]
    fn the_new_states_serialize_as_the_interface_reads_them() {
        assert_eq!(
            serde_json::to_string(&QueueState::Refused).unwrap(),
            "\"refused\""
        );
        assert_eq!(
            serde_json::to_string(&QueueState::Held).unwrap(),
            "\"held\""
        );
    }

    #[test]
    fn mutating_an_unknown_sha256_reports_not_found() {
        let queue = UploadQueue::open_in_memory().unwrap();
        let error = queue.mark_failed("missing", "boom").unwrap_err();
        assert!(matches!(error, UploadQueueError::NotFound(sha) if sha == "missing"));
    }

    #[test]
    fn list_reports_every_row_regardless_of_state() {
        let queue = UploadQueue::open_in_memory().unwrap();
        let report = report_with(&[("M31/final/a.tif", "aaa", 1), ("M31/final/b.tif", "bbb", 2)]);
        queue.enqueue_from_dry_run(&report).unwrap();
        queue.mark_failed("bbb", "boom").unwrap();

        let rows = queue.list().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].sha256, "aaa");
        assert_eq!(rows[1].sha256, "bbb");
        assert_eq!(rows[1].state, QueueState::Failed);
    }

    fn result(path: &str, sha256: &str, kind: ResultKind, target: EntryTarget) -> ResultFile {
        ResultFile {
            path: PathBuf::from(path),
            sha256: sha256.into(),
            size: 10,
            kind,
            target,
        }
    }

    #[test]
    fn only_results_are_results() {
        use std::path::Path;
        let kind = |p: &str| result_kind_of(Path::new(p));
        assert_eq!(kind("M31/final/m31.tif"), Ok(ResultKind::Final));
        assert_eq!(kind("M31/final/M31.JPEG"), Ok(ResultKind::Final));
        assert_eq!(kind("M31/stacked/m31-raw.tif"), Ok(ResultKind::Stacked));
        assert_eq!(kind("M31/final/m31.fits"), Err(NotAResult::Frame));
        assert_eq!(kind("M31/lights/l-0001.xisf"), Err(NotAResult::Frame));
        assert_eq!(kind("M31/lights/raw.CR2"), Err(NotAResult::Frame));
        assert_eq!(kind("M31/lights/preview.jpg"), Err(NotAResult::InLights));
        assert_eq!(
            kind("calibration/flats/Ha/flat.png"),
            Err(NotAResult::CalibrationFrame)
        );
        assert_eq!(
            kind("logs/PHD2_GuideLog_2026-10-07.txt"),
            Err(NotAResult::RawLog)
        );
        assert_eq!(kind("logs/nina-export.png"), Err(NotAResult::RawLog));
        assert_eq!(kind("M31/final/notes.md"), Err(NotAResult::NotAnImage));
    }

    #[test]
    fn a_folder_enqueues_its_results_and_leaves_frames_calibration_and_logs_here() {
        let queue = UploadQueue::open_in_memory().unwrap();
        let report = report_with(&[
            ("M31/final/m31.tif", "aaa", 100),
            ("M31/lights/l-0001.fits", "bbb", 100),
            ("M31/lights/l-0002.xisf", "ccc", 100),
            ("calibration/darks/d-0001.fits", "ddd", 100),
            ("logs/PHD2_GuideLog.txt", "eee", 100),
            ("logs/NINA.log", "fff", 100),
            ("M31/stacked/m31-raw.tif", "ggg", 100),
        ]);
        let outcome = queue.enqueue_from_dry_run(&report).unwrap();
        assert_eq!(outcome.inserted, 2);
        assert_eq!(outcome.left_out, 5);
        let kinds: Vec<ResultKind> = queue.list().unwrap().iter().map(|e| e.kind).collect();
        assert_eq!(kinds, [ResultKind::Final, ResultKind::Stacked]);
    }

    #[test]
    fn enqueuing_results_refuses_a_frame_and_inserts_nothing() {
        let queue = UploadQueue::open_in_memory().unwrap();
        let error = queue
            .enqueue_results(&[
                result(
                    "/n/M31/final/m31.tif",
                    "aaa",
                    ResultKind::Final,
                    EntryTarget::default(),
                ),
                result(
                    "/n/M31/lights/l.fits",
                    "bbb",
                    ResultKind::Final,
                    EntryTarget::default(),
                ),
            ])
            .unwrap_err();
        assert!(matches!(
            error,
            UploadQueueError::NotAResult {
                reason: NotAResult::Frame,
                ..
            }
        ));
        assert!(queue.list().unwrap().is_empty(), "all or nothing");
    }

    #[test]
    fn a_result_keeps_what_its_upload_is_attributed_to() {
        let queue = UploadQueue::open_in_memory().unwrap();
        let target = EntryTarget {
            session_id: Some("night-1".into()),
            target_ids: vec!["m31".into()],
            variant_of: Some("capture-1".into()),
            source: Some("stacking".into()),
            metadata: None,
            frame_count: None,
            plan: None,
        };
        queue
            .enqueue_results(&[
                result(
                    "/n/M31/stacked/m31.tif",
                    "aaa",
                    ResultKind::Stacked,
                    target.clone(),
                ),
                result(
                    "/data/progression/m31-1.jpg",
                    "bbb",
                    ResultKind::Progression,
                    EntryTarget::default(),
                ),
            ])
            .unwrap();
        let rows = queue.list().unwrap();
        assert_eq!(rows[0].target, target);
        assert_eq!(rows[0].kind, ResultKind::Stacked);
        assert_eq!(rows[1].kind, ResultKind::Progression);
        assert_eq!(rows[1].target, EntryTarget::default());
        queue.mark_completed("aaa", "capture-1").unwrap();
        assert_eq!(queue.completed().unwrap().len(), 1);
    }

    #[test]
    fn rows_an_older_build_enqueued_that_are_not_results_are_dropped_on_opening() {
        let directory = crate::test_support::TempDir::new("upload-queue-old-rows");
        let db_path = directory.path().join("queue.sqlite");
        {
            let old = Connection::open(&db_path).unwrap();
            old.execute_batch(
                "CREATE TABLE upload_queue (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    source_path TEXT NOT NULL,
                    sha256 TEXT NOT NULL UNIQUE,
                    size INTEGER NOT NULL,
                    upload_id TEXT,
                    offset INTEGER NOT NULL DEFAULT 0,
                    state TEXT NOT NULL DEFAULT 'queued',
                    capture_id TEXT,
                    last_error TEXT,
                    last_error_code TEXT,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                INSERT INTO upload_queue (source_path, sha256, size, state, created_at, updated_at) VALUES
                    ('/night/M31/lights/l-0001.fits', 'f1', 9, 'queued', 't', 't'),
                    ('/night/logs/PHD2_GuideLog.txt', 'l1', 9, 'completed', 't', 't'),
                    ('/night/calibration/flats/f.xisf', 'c1', 9, 'uploading', 't', 't'),
                    ('/night/M31/final/m31.tif', 'r1', 9, 'queued', 't', 't');",
            )
            .unwrap();
        }
        let queue = UploadQueue::open(&db_path).unwrap();
        let rows = queue.list().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].sha256, "r1");
        assert_eq!(rows[0].kind, ResultKind::Final);
        assert_eq!(queue.next_pending().unwrap().unwrap().sha256, "r1");
    }
    /// A plan folder named `<plan> · local-x/progression/<name>.jpg`, with the
    /// JPEG written, and the row that names it.
    fn waiting_plan_image(
        queue: &UploadQueue,
        root: &Path,
        sha256: &str,
        name: &str,
    ) -> (PathBuf, PathBuf) {
        let folder = root.join("Sh2-103 · local-fea0a6");
        std::fs::create_dir_all(folder.join("progression")).unwrap();
        let file = folder.join("progression").join(name);
        std::fs::write(&file, name).unwrap();
        queue
            .enqueue_results(&[ResultFile {
                path: file.clone(),
                sha256: sha256.to_string(),
                size: name.len() as u64,
                kind: ResultKind::Progression,
                target: EntryTarget::default(),
            }])
            .unwrap();
        (folder, file)
    }

    #[test]
    fn rows_waiting_while_their_folder_is_renamed_follow_it() {
        let directory = crate::test_support::TempDir::new("upload-queue-rename");
        let queue = UploadQueue::open(&directory.path().join("queue.sqlite")).unwrap();
        let (folder, _) = waiting_plan_image(&queue, directory.path(), "p1", "night-01.jpg");
        waiting_plan_image(&queue, directory.path(), "p2", "night-02.jpg");
        let renamed = directory.path().join("Sh2-103 · 80340a");
        std::fs::rename(&folder, &renamed).unwrap();

        assert_eq!(queue.rebase_provisional(&folder, &renamed, &[]).unwrap(), 2);

        for (sha, name) in [("p1", "night-01.jpg"), ("p2", "night-02.jpg")] {
            let row = queue.find_by_sha256(sha).unwrap().unwrap();
            assert_eq!(row.source_path, renamed.join("progression").join(name));
            assert!(row.source_path.is_file());
        }
    }

    #[test]
    fn an_image_queued_with_the_old_folder_is_queued_where_the_file_is() {
        let directory = crate::test_support::TempDir::new("upload-queue-stale-enqueue");
        let queue = UploadQueue::open(&directory.path().join("queue.sqlite")).unwrap();
        let (folder, stale) = waiting_plan_image(&queue, directory.path(), "p1", "night-01.jpg");
        let renamed = directory.path().join("Sh2-103 · 80340a");
        std::fs::rename(&folder, &renamed).unwrap();
        queue.rebase_provisional(&folder, &renamed, &[]).unwrap();

        // The interface still holds the path of before the rename.
        let outcome = queue
            .enqueue_results(&[ResultFile {
                path: folder.join("progression/night-02.jpg"),
                sha256: "p2".into(),
                size: 12,
                kind: ResultKind::Progression,
                target: EntryTarget::default(),
            }])
            .unwrap();
        // Not on disk anywhere: kept as given, nothing is invented.
        assert_eq!(outcome.inserted, 1);
        assert_eq!(
            queue.find_by_sha256("p2").unwrap().unwrap().source_path,
            folder.join("progression/night-02.jpg")
        );
        std::fs::write(renamed.join("progression/night-03.jpg"), "03").unwrap();
        queue
            .enqueue_results(&[ResultFile {
                path: folder.join("progression/night-03.jpg"),
                sha256: "p3".into(),
                size: 2,
                kind: ResultKind::Progression,
                target: EntryTarget::default(),
            }])
            .unwrap();
        let row = queue.find_by_sha256("p3").unwrap().unwrap();
        assert_eq!(row.source_path, renamed.join("progression/night-03.jpg"));
        assert!(!stale.exists());
    }

    #[test]
    fn a_file_set_aside_as_missing_is_queued_again_once_its_folder_is_found() {
        let directory = crate::test_support::TempDir::new("upload-queue-heal");
        let db = directory.path().join("queue.sqlite");
        let queue = UploadQueue::open(&db).unwrap();
        let (folder, file) = waiting_plan_image(&queue, directory.path(), "p1", "night-01.jpg");
        let renamed = directory.path().join("Sh2-103 · 80340a");
        std::fs::rename(&folder, &renamed).unwrap();
        // A build that did not follow the rename failed the row on its turn.
        queue
            .mark_failed_coded("p1", "source_file_missing", "not found")
            .unwrap();
        queue.record_move(&folder, &renamed).unwrap();
        assert!(queue.next_pending().unwrap().is_none());

        let queue = UploadQueue::open(&db).unwrap();
        let row = queue.find_by_sha256("p1").unwrap().unwrap();
        assert_eq!(row.state, QueueState::Queued);
        assert_eq!(row.last_error_code, None);
        assert_eq!(
            row.source_path,
            renamed.join("progression/night-01.jpg"),
            "was {}",
            file.display()
        );
        assert_eq!(queue.next_pending().unwrap().unwrap().sha256, "p1");
    }

    #[test]
    fn moves_are_followed_one_after_the_other() {
        let directory = crate::test_support::TempDir::new("upload-queue-chain");
        let queue = UploadQueue::open(&directory.path().join("queue.sqlite")).unwrap();
        let (a, b, c) = (
            directory.path().join("a"),
            directory.path().join("b"),
            directory.path().join("c"),
        );
        std::fs::create_dir_all(&c).unwrap();
        std::fs::write(c.join("x.jpg"), "x").unwrap();
        queue.record_move(&a, &b).unwrap();
        queue.record_move(&b, &c).unwrap();
        assert_eq!(
            queue.moved_source(&a.join("x.jpg")).unwrap(),
            Some(c.join("x.jpg"))
        );
        // A file that is where it was named is left alone.
        assert_eq!(queue.moved_source(&c.join("x.jpg")).unwrap(), None);
    }

    #[test]
    fn requeue_failed_puts_a_missing_file_back_in_the_queue() {
        let queue = UploadQueue::open_in_memory().unwrap();
        queue
            .enqueue_results(&[ResultFile {
                path: PathBuf::from("/nowhere/progression/a.jpg"),
                sha256: "p1".into(),
                size: 1,
                kind: ResultKind::Progression,
                target: EntryTarget::default(),
            }])
            .unwrap();
        queue
            .mark_failed_coded("p1", "source_file_missing", "not found")
            .unwrap();
        assert!(queue.next_pending().unwrap().is_none());
        queue.requeue_failed("p1").unwrap();
        let row = queue.next_pending().unwrap().unwrap();
        assert_eq!(row.state, QueueState::Queued);
        assert_eq!(row.last_error, None);
        assert_eq!(row.last_error_code, None);
        // A file that is not failed is left as it is.
        queue.mark_progress("p1", "up-1", 1).unwrap();
        queue.requeue_failed("p1").unwrap();
        assert_eq!(
            queue.find_by_sha256("p1").unwrap().unwrap().state,
            QueueState::Uploading
        );
    }

    fn sky(sha256: &str, state: SkyState) -> SkyAnnotation {
        SkyAnnotation {
            sha256: sha256.into(),
            capture_id: Some("cap".into()),
            state,
            failure: None,
            error_code: None,
            message: None,
            elapsed_ms: None,
            source: None,
            updated_at: "2026-11-05T18:55:00Z".into(),
        }
    }

    #[test]
    fn a_queue_written_before_the_spaces_keeps_its_rows_before_the_link() {
        let directory = crate::test_support::TempDir::new("upload-queue-before-spaces");
        let db_path = directory.path().join("queue.sqlite");
        {
            let old = Connection::open(&db_path).unwrap();
            old.execute_batch(
                "CREATE TABLE upload_queue (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    source_path TEXT NOT NULL,
                    sha256 TEXT NOT NULL UNIQUE,
                    size INTEGER NOT NULL,
                    upload_id TEXT,
                    offset INTEGER NOT NULL DEFAULT 0,
                    state TEXT NOT NULL DEFAULT 'queued',
                    capture_id TEXT,
                    last_error TEXT,
                    last_error_code TEXT,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    kind TEXT, session_id TEXT, target_ids TEXT, variant_of TEXT,
                    source TEXT, metadata TEXT, frames TEXT, plan TEXT
                );
                CREATE TABLE sky_annotation (
                    sha256 TEXT PRIMARY KEY,
                    capture_id TEXT,
                    state TEXT NOT NULL,
                    failure TEXT,
                    error_code TEXT,
                    message TEXT,
                    elapsed_ms INTEGER,
                    source TEXT,
                    updated_at TEXT NOT NULL
                );
                INSERT INTO upload_queue
                    (source_path, sha256, size, state, capture_id, created_at, updated_at, kind)
                VALUES ('/w/Sh2-103/final/sh2.tif', 'sh2', 900, 'completed', 'old-capture',
                        '2026-11-05T18:53:00Z', '2026-11-05T18:53:00Z', 'final');
                INSERT INTO sky_annotation (sha256, capture_id, state, updated_at)
                VALUES ('sh2', 'old-capture', 'sent', '2026-11-05T18:55:00Z');",
            )
            .unwrap();
        }

        // Seen from no space, the rows are as they were.
        let queue = UploadQueue::open(&db_path).unwrap();
        assert_eq!(queue.list().unwrap().len(), 1);
        assert_eq!(queue.sky_annotations().unwrap().len(), 1);
        drop(queue);

        // Seen from the space linked since, they are « before the link »:
        // they neither block the file nor show as this space's.
        let queue = UploadQueue::open(&db_path)
            .unwrap()
            .for_space(Some("space-sample".into()));
        assert!(queue.list().unwrap().is_empty());
        assert!(queue.sky_annotations().unwrap().is_empty());
        assert!(queue.sky_of("sh2").unwrap().is_none());
        let before = queue.list_before_link().unwrap();
        assert_eq!(before.len(), 1);
        assert_eq!(before[0].capture_id.as_deref(), Some("old-capture"));
        let outcome = queue
            .enqueue_results(&[result(
                "/w/Sh2-103/final/sh2.tif",
                "sh2",
                ResultKind::Final,
                EntryTarget::default(),
            )])
            .unwrap();
        assert_eq!(outcome.inserted, 1);
        assert_eq!(queue.next_pending().unwrap().unwrap().sha256, "sh2");

        // Opening it again migrates nothing twice.
        drop(queue);
        let queue = UploadQueue::open(&db_path)
            .unwrap()
            .for_space(Some("space-sample".into()));
        assert_eq!(queue.list().unwrap().len(), 1);
        assert_eq!(queue.list_before_link().unwrap().len(), 1);

        // « Effacer l'historique » purges what came before the link, and only
        // that.
        assert_eq!(queue.forget_before_link().unwrap(), 1);
        assert!(queue.list_before_link().unwrap().is_empty());
        assert_eq!(queue.list().unwrap().len(), 1);
        let unscoped = UploadQueue::open(&db_path).unwrap();
        assert!(unscoped.list().unwrap().is_empty());
        assert!(unscoped.sky_annotations().unwrap().is_empty());
    }

    #[test]
    fn a_digest_is_unique_per_space_and_known_only_in_its_own() {
        let directory = crate::test_support::TempDir::new("upload-queue-per-space");
        let db_path = directory.path().join("queue.sqlite");
        let file = result(
            "/w/M31/final/m31.tif",
            "m31",
            ResultKind::Final,
            EntryTarget::default(),
        );
        let first = UploadQueue::open(&db_path)
            .unwrap()
            .for_space(Some("space-a".into()));
        assert_eq!(
            first
                .enqueue_results(std::slice::from_ref(&file))
                .unwrap()
                .inserted,
            1
        );
        first.mark_progress("m31", "up-a", 900).unwrap();
        first.mark_completed("m31", "capture-a").unwrap();
        first.set_sky(&sky("m31", SkyState::Sent)).unwrap();

        // The same space: already known, and already sent, with its capture.
        let again = first.enqueue_results(std::slice::from_ref(&file)).unwrap();
        assert_eq!(again.inserted, 0);
        assert_eq!(again.already_known, 1);
        assert_eq!(again.already_sent.len(), 1);
        assert_eq!(again.already_sent[0].sha256, "m31");
        assert_eq!(
            again.already_sent[0].capture_id.as_deref(),
            Some("capture-a")
        );
        assert!(first.next_pending().unwrap().is_none());

        // Another space: a file of its own, to send.
        let second = UploadQueue::open(&db_path)
            .unwrap()
            .for_space(Some("space-b".into()));
        let outcome = second.enqueue_results(std::slice::from_ref(&file)).unwrap();
        assert_eq!(outcome.inserted, 1);
        assert!(outcome.already_sent.is_empty());
        assert_eq!(
            second.next_pending().unwrap().unwrap().state,
            QueueState::Queued
        );
        assert!(second.sky_of("m31").unwrap().is_none());
        assert!(second.completed().unwrap().is_empty());
        // Its progress is its own: the first space's row is untouched.
        second.mark_progress("m31", "up-b", 10).unwrap();
        second.hold("m31", "storage_quota_reached", "409").unwrap();
        assert_eq!(
            first.find_by_sha256("m31").unwrap().unwrap().state,
            QueueState::Completed
        );
        // The resumption of one space never picks the other's.
        assert!(first.next_pending().unwrap().is_none());
        assert_eq!(first.release_held().unwrap(), 0);
        assert_eq!(second.release_held().unwrap(), 1);
        let resumed = second.next_pending().unwrap().unwrap();
        assert_eq!(resumed.state, QueueState::Uploading);
        assert_eq!(resumed.offset, 10);
        // Linking back to the first space: its history is there, whole.
        let back = UploadQueue::open(&db_path)
            .unwrap()
            .for_space(Some("space-a".into()));
        assert_eq!(back.completed().unwrap().len(), 1);
        assert_eq!(back.sky_of("m31").unwrap().unwrap().state, SkyState::Sent);
        // None of these rows is « before the link ».
        assert!(back.list_before_link().unwrap().is_empty());
    }

    #[test]
    fn a_file_already_sent_can_be_sent_again() {
        let queue = UploadQueue::open_in_memory()
            .unwrap()
            .for_space(Some("space-a".into()));
        let file = result(
            "/w/M31/final/m31.tif",
            "m31",
            ResultKind::Final,
            EntryTarget::default(),
        );
        queue.enqueue_results(&[file]).unwrap();
        // Not sent yet: nothing to send again.
        assert!(!queue.requeue_sent("m31").unwrap());
        queue.mark_progress("m31", "up-1", 900).unwrap();
        queue.mark_completed("m31", "capture-1").unwrap();
        assert!(queue.requeue_sent("m31").unwrap());
        let row = queue.next_pending().unwrap().unwrap();
        assert_eq!(row.state, QueueState::Queued);
        assert_eq!(row.offset, 0);
        assert_eq!(row.upload_id, None);
        assert!(matches!(
            queue.requeue_sent("unknown"),
            Err(UploadQueueError::NotFound(_))
        ));
    }
}
