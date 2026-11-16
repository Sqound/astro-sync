//! A capture deleted from the portal is not « already sent »: before a
//! sending, the queue asks the service (`POST /v1/uploads/check`, which
//! writes nothing) about the files it holds as sent, puts back in the queue
//! the ones the space no longer has -- the capture first, its variants after
//! -- keeps the ones it still has, forgets what it cannot ask, and says the
//! truth about an image whose capture is gone. Invented images, a fake
//! service.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::PathBuf;

use astro_sync_core::gui_bridge::GuiBridgeError;
use astro_sync_core::precheck::{
    CheckStorage, HeldAs, PrecheckTransport, SpaceStorage, UploadCheck,
};
use astro_sync_core::refusal;
use astro_sync_core::upload_queue::{EntryTarget, QueueState, ResultFile, ResultKind, UploadQueue};
use astro_sync_core::uploader::{
    CAPTURE_NOT_IN_SPACE, UploadMetadata, VARIANT_OF_SHA256, requeue_vanished,
};

const NATURAL: &str = "aaaa";
const DUAL_BAND: &str = "bbbb";
const HOO: &str = "cccc";
const CARE: &str = "dddd";

/// What the space holds: the digests of its images, and its captures.
#[derive(Default)]
struct Space {
    images: BTreeSet<String>,
    captures: BTreeSet<String>,
    /// Every question asked, in order.
    asked: RefCell<Vec<UploadMetadata>>,
    /// The service cannot be reached.
    down: bool,
}

impl PrecheckTransport for Space {
    fn space_storage(&self, _: &str, _: &str) -> Result<SpaceStorage, GuiBridgeError> {
        unimplemented!("the check of a sent file never reads the storage")
    }

    fn check_upload(
        &self,
        _: &str,
        _: &str,
        metadata: &UploadMetadata,
    ) -> Result<UploadCheck, GuiBridgeError> {
        self.asked.borrow_mut().push(metadata.clone());
        if self.down {
            return Err(GuiBridgeError::Transport("down".into()));
        }
        let storage = CheckStorage {
            used_bytes: 0,
            quota_bytes: 1000,
            max_file_bytes: 1000,
            capture_count: 0,
            capture_count_limit: 3,
            variant_limit: 12,
            variant_count: None,
        };
        if let Some(capture) = &metadata.capture_id
            && !self.captures.contains(capture)
        {
            return Err(GuiBridgeError::Refused {
                code: "capture_not_found".into(),
                message: "Capture not found.".into(),
            });
        }
        if self.images.contains(&metadata.sha256) {
            return Ok(UploadCheck::Present {
                capture_id: "capture-1".into(),
                held: HeldAs::Image {
                    image_id: "image".into(),
                    main: true,
                },
                storage,
            });
        }
        Ok(UploadCheck::Accepted {
            adds: "capture".into(),
            storage,
        })
    }
}

fn fresh_queue() -> UploadQueue {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let folder = std::env::temp_dir().join(format!(
        "astro-sync-resend-vanished-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&folder).unwrap();
    UploadQueue::open(&folder.join("queue.sqlite")).unwrap()
}

fn file(name: &str, sha: &str, variant_of: Option<&str>) -> ResultFile {
    ResultFile {
        path: PathBuf::from(format!("/night/M45/stacked/{name}.tif")),
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

/// The night of the 13th, sent: the natural render creates the capture, the
/// three others are its variants.
fn night() -> Vec<ResultFile> {
    vec![
        file("m45-natural", NATURAL, None),
        file("m45-dual-band", DUAL_BAND, Some(NATURAL)),
        file("m45-hoo", HOO, Some(NATURAL)),
    ]
}

fn sent_queue(files: &[ResultFile]) -> UploadQueue {
    let queue = fresh_queue();
    queue.enqueue_results(files).unwrap();
    for entry in queue.list().unwrap() {
        queue.mark_completed(&entry.sha256, "capture-1").unwrap();
    }
    queue
}

fn state(queue: &UploadQueue, sha: &str) -> QueueState {
    queue.find_by_sha256(sha).unwrap().unwrap().state
}

#[test]
fn a_capture_deleted_from_the_portal_is_sent_again_with_its_variants() {
    let files = night();
    let queue = sent_queue(&files);
    let sky = astro_sync_core::sky_after_upload::SkyAnnotation {
        sha256: NATURAL.into(),
        capture_id: Some("capture-1".into()),
        state: astro_sync_core::sky_after_upload::SkyState::Already,
        failure: None,
        error_code: None,
        message: None,
        elapsed_ms: None,
        source: None,
        updated_at: "2026-11-13T12:02:00Z".into(),
    };
    queue.set_sky(&sky).unwrap();
    // The space holds nothing any more.
    let space = Space::default();

    let found = requeue_vanished(&space, "https://portal", "token", &queue, &files).unwrap();

    assert_eq!(found.requeued, [NATURAL, DUAL_BAND, HOO]);
    for sha in [NATURAL, DUAL_BAND, HOO] {
        assert_eq!(state(&queue, sha), QueueState::Queued, "{sha}");
    }
    // What was said of its sky annotations was said of a capture that is gone.
    assert!(queue.sky_of(NATURAL).unwrap().is_none());
    // The capture goes first: it is the oldest row waiting.
    assert_eq!(queue.next_pending().unwrap().unwrap().sha256, NATURAL);
    // One question: the variants follow their capture without being asked.
    assert_eq!(space.asked.borrow().len(), 1);
}

#[test]
fn a_capture_still_in_the_space_is_not_sent_again() {
    let files = night();
    let queue = sent_queue(&files);
    let mut space = Space::default();
    space.captures.insert("capture-1".into());
    space.images = [NATURAL, DUAL_BAND, HOO].map(String::from).into();

    let found = requeue_vanished(&space, "https://portal", "token", &queue, &files).unwrap();

    assert!(found.requeued.is_empty() && found.unchecked.is_empty());
    for sha in [NATURAL, DUAL_BAND, HOO] {
        assert_eq!(state(&queue, sha), QueueState::Completed, "{sha}");
    }
}

#[test]
fn one_image_deleted_from_a_capture_that_stays_is_the_only_one_sent_again() {
    let files = night();
    let queue = sent_queue(&files);
    let mut space = Space::default();
    space.captures.insert("capture-1".into());
    space.images = [NATURAL, HOO].map(String::from).into();

    let found = requeue_vanished(&space, "https://portal", "token", &queue, &files).unwrap();

    assert_eq!(found.requeued, [DUAL_BAND]);
    assert_eq!(state(&queue, NATURAL), QueueState::Completed);
    assert_eq!(state(&queue, DUAL_BAND), QueueState::Queued);
    assert_eq!(state(&queue, HOO), QueueState::Completed);
}

#[test]
fn a_service_that_cannot_be_asked_changes_nothing() {
    let files = night();
    let queue = sent_queue(&files);
    let space = Space {
        down: true,
        ..Space::default()
    };

    let found = requeue_vanished(&space, "https://portal", "token", &queue, &files).unwrap();

    assert!(found.requeued.is_empty());
    assert_eq!(found.unchecked.len(), 3);
    for sha in [NATURAL, DUAL_BAND, HOO] {
        assert_eq!(state(&queue, sha), QueueState::Completed, "{sha}");
    }
}

/// A night as it was reported: three renders already sent, the capture deleted since, and a
/// fourth render (other bytes) that went as a variant of the capture the
/// queue believed in and was refused there.
#[test]
fn an_image_refused_for_a_capture_that_was_gone_follows_its_capture_on_the_next_sending() {
    let mut files = night();
    files.push(file("m45-care", CARE, Some(NATURAL)));
    let queue = sent_queue(&files[..3]);
    queue.enqueue_results(&files[3..]).unwrap();
    queue
        .mark_refused(CARE, CAPTURE_NOT_IN_SPACE, "Capture not found.")
        .unwrap();
    let space = Space::default();

    let found = requeue_vanished(&space, "https://portal", "token", &queue, &files).unwrap();

    assert_eq!(found.requeued, [NATURAL, DUAL_BAND, HOO, CARE]);
    assert_eq!(state(&queue, CARE), QueueState::Queued);
    let order: Vec<String> = queue
        .list()
        .unwrap()
        .into_iter()
        .map(|entry| entry.sha256)
        .collect();
    assert_eq!(order, [NATURAL, DUAL_BAND, HOO, CARE]);
}

#[test]
fn an_image_refused_for_another_reason_is_left_as_it_is() {
    let files = night();
    let queue = fresh_queue();
    queue.enqueue_results(&files).unwrap();
    queue
        .mark_refused(NATURAL, "file_too_large", "too large")
        .unwrap();

    let found =
        requeue_vanished(&Space::default(), "https://portal", "token", &queue, &files).unwrap();

    assert!(found.requeued.is_empty());
    assert_eq!(state(&queue, NATURAL), QueueState::Refused);
}

#[test]
fn an_image_whose_capture_is_gone_says_so_and_how_to_send_it_again() {
    assert_eq!(
        refusal::code_for_answer(404, "capture_not_found"),
        Some("capture_not_found")
    );
    let sentence = refusal::plain_sentence("capture_not_found").unwrap();
    assert!(sentence.contains("no longer in your space"));
    assert!(sentence.contains("Import the night again"));
    assert!(!sentence.to_lowercase().contains("update"));
}

/// The sentence is said in each of the ten languages of the window: its own
/// words, no raw code, no semicolon nor long dash to join two ideas.
#[test]
fn the_missing_capture_is_said_in_the_ten_languages() {
    let mut seen = BTreeSet::new();
    for locale in [
        "fr", "en", "ja", "de", "es", "it", "pt-BR", "zh-Hans", "zh-Hant", "ru",
    ] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../app/src/i18n/{locale}.json"));
        let words: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let sentence = words["errors.capture_not_found"]
            .as_str()
            .unwrap_or_else(|| panic!("{locale}.json has no errors.capture_not_found"));
        assert!(!sentence.contains("capture_not_found"), "{locale}");
        assert!(!sentence.contains([';', '—', '–', '{', '}']), "{locale}");
        assert!(!sentence.to_lowercase().contains("update"), "{locale}");
        seen.insert(sentence.to_string());
    }
    assert_eq!(seen.len(), 10);
    assert_eq!(
        seen.iter()
            .find(|s| s.starts_with("This image"))
            .map(String::as_str),
        refusal::plain_sentence("capture_not_found")
    );
}
