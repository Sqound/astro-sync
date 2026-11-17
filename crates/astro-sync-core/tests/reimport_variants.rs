//! A night imported again after its captures were deleted from the portal:
//! every image of the new import leaves, as this import describes it. A row
//! the queue kept from an older import of the night no longer names the main
//! image of that older import (whose capture is gone), and a refusal an older
//! version could not read is tried again. Invented images, a fake service.

use std::path::PathBuf;

use astro_sync_core::gui_bridge::GuiBridgeError;
use astro_sync_core::precheck::{PrecheckTransport, SpaceStorage, UploadCheck};
use astro_sync_core::upload_queue::{EntryTarget, QueueState, ResultFile, ResultKind, UploadQueue};
use astro_sync_core::uploader::{UploadMetadata, VARIANT_OF_SHA256, requeue_vanished};

/// The natural render of the first import, and the one of today's (made
/// again since: other bytes).
const NATURAL_OLD: &str = "a0a0";
const NATURAL_NEW: &str = "a1a1";
/// A render whose bytes did not change between the two imports.
const HOO: &str = "c0c0";
/// A render new in today's import.
const DUAL_BAND: &str = "b1b1";
/// A render refused on the 16th by a version that did not know the code of a
/// missing capture (`upload_refused`).
const CARE: &str = "d0d0";

/// A space that holds nothing any more: every capture was deleted.
struct EmptySpace;

impl PrecheckTransport for EmptySpace {
    fn space_storage(&self, _: &str, _: &str) -> Result<SpaceStorage, GuiBridgeError> {
        unimplemented!("the check of a sent file never reads the storage")
    }

    fn check_upload(
        &self,
        _: &str,
        _: &str,
        metadata: &UploadMetadata,
    ) -> Result<UploadCheck, GuiBridgeError> {
        if metadata.capture_id.is_some() {
            return Err(GuiBridgeError::Refused {
                code: "capture_not_found".into(),
                message: "Capture not found.".into(),
            });
        }
        Err(GuiBridgeError::Transport(
            "a main image is never asked in these tests".into(),
        ))
    }
}

fn fresh_queue() -> UploadQueue {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let folder = std::env::temp_dir().join(format!(
        "astro-sync-reimport-variants-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&folder).unwrap();
    UploadQueue::open(&folder.join("queue.sqlite")).unwrap()
}

fn file(name: &str, sha: &str, variant_of: Option<&str>) -> ResultFile {
    ResultFile {
        path: PathBuf::from(format!("/night/M45/stacked/{name}.png")),
        sha256: sha.into(),
        size: 64,
        kind: ResultKind::Stacked,
        target: EntryTarget {
            session_id: Some("session-1".into()),
            target_ids: vec!["m45".into()],
            variant_of: variant_of.map(|sha| format!("{VARIANT_OF_SHA256}{sha}")),
            ..EntryTarget::default()
        },
    }
}

/// The first import, sent: the natural render made `capture-old`, the HOO
/// palette is its variant.
fn first_import(queue: &UploadQueue) {
    queue
        .enqueue_results(&[
            file("m45-natural", NATURAL_OLD, None),
            file("m45-hoo", HOO, Some(NATURAL_OLD)),
        ])
        .unwrap();
    queue.mark_completed(NATURAL_OLD, "capture-old").unwrap();
    queue.mark_completed(HOO, "capture-old").unwrap();
}

/// Today's import: a natural render made again, the same HOO palette, a new
/// dual-band render, each variant naming today's natural render.
fn todays_import() -> Vec<ResultFile> {
    vec![
        file("m45-natural", NATURAL_NEW, None),
        file("m45-dual-band", DUAL_BAND, Some(NATURAL_NEW)),
        file("m45-hoo", HOO, Some(NATURAL_NEW)),
    ]
}

/// What « Envoyer » does with an import: the files the space no longer holds
/// back in the queue, then the import queued.
fn send(queue: &UploadQueue, files: &[ResultFile]) {
    requeue_vanished(&EmptySpace, "https://portal", "token", queue, files).unwrap();
    queue.enqueue_results(files).unwrap();
}

fn row(queue: &UploadQueue, sha: &str) -> astro_sync_core::upload_queue::QueueEntry {
    queue.find_by_sha256(sha).unwrap().unwrap()
}

/// The refusal of 17/11, « la capture à laquelle elle se rattache n'est plus
/// dans votre espace », at every import: the HOO row kept the attribution of
/// the first import, so its upload named the capture of the first natural
/// render, deleted since. It now names today's natural render.
#[test]
fn a_variant_queued_again_names_the_main_image_of_this_import() {
    let queue = fresh_queue();
    first_import(&queue);

    send(&queue, &todays_import());

    let hoo = row(&queue, HOO);
    assert_eq!(hoo.state, QueueState::Queued);
    assert_eq!(
        hoo.target.variant_of.as_deref(),
        Some(format!("{VARIANT_OF_SHA256}{NATURAL_NEW}").as_str()),
        "the variant follows the main image of today's import, never the one of an older import"
    );
}

/// A variant waiting before the image that creates its capture is held and
/// stops the sending: the row queued again takes its place after it.
#[test]
fn a_variant_queued_again_leaves_after_the_main_image_of_this_import() {
    let queue = fresh_queue();
    first_import(&queue);

    send(&queue, &todays_import());

    assert_eq!(queue.next_pending().unwrap().unwrap().sha256, NATURAL_NEW);
    let waiting: Vec<String> = queue
        .list()
        .unwrap()
        .into_iter()
        .filter(|entry| entry.state == QueueState::Queued)
        .map(|entry| entry.sha256)
        .collect();
    assert_eq!(waiting, [NATURAL_NEW, DUAL_BAND, HOO]);
}

/// The refusal of 16/11 (« mettez Astro Sync à jour, puis ajoutez le fichier
/// de nouveau »): adding the same file again did nothing, the queue kept it
/// refused, and the window said « En file d'envoi » for ever. A new import of
/// the night tries it again, once.
#[test]
fn an_image_refused_with_a_reason_an_older_version_could_not_read_is_tried_again_by_an_import() {
    let queue = fresh_queue();
    first_import(&queue);
    queue
        .enqueue_results(&[file("m45-care", CARE, Some(NATURAL_OLD))])
        .unwrap();
    queue
        .mark_refused(
            CARE,
            "upload_refused",
            "refused by the service (404 capture_not_found): The capture was not found.",
        )
        .unwrap();
    let mut files = todays_import();
    files.push(file("m45-care", CARE, Some(NATURAL_NEW)));

    send(&queue, &files);

    let care = row(&queue, CARE);
    assert_eq!(care.state, QueueState::Queued);
    assert_eq!(
        care.target.variant_of.as_deref(),
        Some(format!("{VARIANT_OF_SHA256}{NATURAL_NEW}").as_str())
    );
}

/// A file refused for a reason that is its own (too large) stays refused: an
/// import does not send it again.
#[test]
fn an_image_refused_for_its_own_reason_stays_refused() {
    let queue = fresh_queue();
    queue
        .enqueue_results(&[file("m45-natural", NATURAL_NEW, None)])
        .unwrap();
    queue
        .mark_refused(NATURAL_NEW, "file_too_large", "too large")
        .unwrap();

    send(&queue, &[file("m45-natural", NATURAL_NEW, None)]);

    assert_eq!(row(&queue, NATURAL_NEW).state, QueueState::Refused);
}

/// A file already on its way (bytes sent, a session open) keeps its row: an
/// import never restarts it.
#[test]
fn a_file_already_on_its_way_keeps_its_row() {
    let queue = fresh_queue();
    queue
        .enqueue_results(&[file("m45-natural", NATURAL_NEW, None)])
        .unwrap();
    queue.mark_progress(NATURAL_NEW, "up-1", 32).unwrap();
    let before = row(&queue, NATURAL_NEW);

    queue
        .enqueue_results(&[file("m45-natural", NATURAL_NEW, None)])
        .unwrap();

    let after = row(&queue, NATURAL_NEW);
    assert_eq!(after.id, before.id);
    assert_eq!(after.offset, 32);
    assert_eq!(after.upload_id.as_deref(), Some("up-1"));
}
