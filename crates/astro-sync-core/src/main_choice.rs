//! Which image is the main image of a capture the space holds already, decided
//! **when the image leaves**: the screen of « Images à envoyer » shows the
//! choice and writes what it will do, but it reads the space at one moment and
//! the queue may send much later (offline, then back online).
//!
//! The service's own rule is that an image sent without a capture, from Astro
//! Sync, into the capture of its session, target and series becomes that
//! capture's main image when the capture is private (the former main image
//! becomes a variant), and arrives as a variant when it is public. An image
//! sent **with** a capture is always a variant. So the head image of a series
//! carries a wish (`EntryTarget::main_wish`) and the space is asked, now:
//!
//! - `keep` (nothing was chosen, the screen could not read the space): the
//!   capture holds a main image, the image leaves as a variant of that
//!   capture; no capture, it creates one;
//! - `over` (the person chose it as the main image): the capture is private
//!   and does not hold these bytes, the image leaves as the service's rule
//!   makes it the main image; the capture is public, or holds the bytes
//!   already (a variant: only the web turns a variant into the main image),
//!   it leaves as a variant.
//!
//! The space cannot say: the image waits and is tried again later, never sent
//! as the main image on a state that is not confirmed. The wait is bounded
//! ([`READS_BEFORE_VARIANT`]): if the reading keeps failing while the sends
//! pass, the image leaves as a **new variant** (`placement: add`), which can
//! never take a main image's place, and the row says so
//! ([`crate::upload_queue::VARIANT_UNCONFIRMED_NOTE`]). A main image is never
//! removed here: the former one becomes a variant, as on the web.

use crate::night_online::NightOnline;
use crate::series::Series;

/// Nothing was chosen: keep the main image the space holds.
pub const WISH_KEEP: &str = "keep";
/// The image was chosen as the main image of the capture.
pub const WISH_OVER: &str = "over";

/// How many readings of the space may fail, one after the other and each
/// after the queue's own growing wait, before the image leaves as a new
/// variant instead of waiting for ever.
pub const READS_BEFORE_VARIANT: u32 = 5;

/// What the image does after `failed_reads` readings of the space that
/// failed (this one included): it waits while the count is below
/// [`READS_BEFORE_VARIANT`], then it leaves as a new variant.
#[must_use]
pub fn when_unread(failed_reads: u32) -> MainAtSend {
    if failed_reads >= READS_BEFORE_VARIANT {
        MainAtSend::NewVariant
    } else {
        MainAtSend::Wait
    }
}

/// Where the head image of a series goes, given what the space holds now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MainAtSend {
    /// As queued: no capture named, the service's rule applies.
    AsQueued,
    /// A variant of this capture of the space; its main image is not touched.
    VariantOf(String),
    /// The space could not say: not sent now.
    Wait,
    /// The space could not say for long enough: a new variant
    /// (`placement: add`), never the main image.
    NewVariant,
}

/// The decision, from `online` (what the night holds, `None` when it could not
/// be read), the target and series of the image and its digest.
#[must_use]
pub fn main_at_send(
    wish: &str,
    online: Option<&NightOnline>,
    target_id: &str,
    series: Option<&Series>,
    sha256: &str,
) -> MainAtSend {
    let Some(online) = online else {
        return MainAtSend::Wait;
    };
    let Some(capture) = online.capture_for(target_id, series) else {
        return MainAtSend::AsQueued;
    };
    if capture.main_image.is_none() {
        return MainAtSend::AsQueued;
    }
    let variant = || MainAtSend::VariantOf(capture.id.clone());
    if capture.visibility == "public" {
        return variant();
    }
    // The bytes are online already: the image is the main image or a variant
    // of that capture, and stays what it is.
    if capture
        .images
        .iter()
        .any(|image| image.sha256.eq_ignore_ascii_case(sha256))
    {
        return variant();
    }
    if wish == WISH_OVER {
        MainAtSend::AsQueued
    } else {
        variant()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::upload_queue::{EntryTarget, QueueEntry, ResultFile, ResultKind, UploadQueue};
    use crate::uploader::{
        FinalizeOutcome, PutOutcome, UploadError, UploadMetadata, UploadSession, UploadTarget,
        UploadTransport,
    };
    use std::cell::RefCell;

    /// A night with one capture of `visibility` holding `images`
    /// (`(sha256, main)`), of the series IRCUT 30 s, gain 200.
    fn night(visibility: &str, images: &[(&str, bool)]) -> NightOnline {
        let images: Vec<serde_json::Value> = images
            .iter()
            .enumerate()
            .map(|(index, (sha, main))| {
                serde_json::json!({
                    "id": format!("img-{index}"), "main": main, "sha256": sha,
                    "byteSize": 35, "originalName": format!("{index}.png"),
                    "source": "stacking", "createdAt": "2026-11-12T20:00:00Z",
                    "skySolution": "missing"
                })
            })
            .collect();
        let main = images.iter().find(|image| image["main"] == true).cloned();
        serde_json::from_value(serde_json::json!({
            "sessionId": "night-1",
            "targets": [{
                "targetId": "m45", "name": "M 45", "planned": true,
                "captures": [{
                    "id": "cap-1", "targetIds": ["m45"], "source": "stacking",
                    "stage": "draft", "visibility": visibility, "uploaded": true,
                    "createdAt": "2026-11-12T20:00:00Z",
                    "series": { "filter": "IRCUT", "exposureSeconds": 30.0,
                                "gain": 200, "binning": 1 },
                    "mainImage": main,
                    "imageCount": images.len(),
                    "images": images,
                }]
            }],
            "analysis": { "present": false, "stepCount": 0, "editedStepCount": 0 }
        }))
        .unwrap()
    }

    fn series() -> Series {
        Series {
            filter: Some("IRCUT".into()),
            exposure_seconds: 30.0,
            gain: Some(200),
            binning: 1,
            frames: None,
            total_seconds: None,
        }
    }

    fn decide(wish: &str, online: Option<&NightOnline>, sha: &str) -> MainAtSend {
        main_at_send(wish, online, "m45", Some(&series()), sha)
    }

    #[test]
    fn keeping_sends_the_image_as_a_variant_of_the_capture_the_space_holds() {
        let online = night("private", &[("natural", true)]);
        assert_eq!(
            decide(WISH_KEEP, Some(&online), "polished"),
            MainAtSend::VariantOf("cap-1".into())
        );
    }

    #[test]
    fn a_chosen_image_takes_the_main_place_only_on_a_private_capture() {
        let private = night("private", &[("natural", true)]);
        assert_eq!(
            decide(WISH_OVER, Some(&private), "polished"),
            MainAtSend::AsQueued
        );
        let public = night("public", &[("natural", true)]);
        assert_eq!(
            decide(WISH_OVER, Some(&public), "polished"),
            MainAtSend::VariantOf("cap-1".into())
        );
    }

    #[test]
    fn a_choice_whose_bytes_are_online_already_stays_what_it_is() {
        // Online as a variant: only the web makes it the main image.
        let online = night("private", &[("natural", true), ("polished", false)]);
        assert_eq!(
            decide(WISH_OVER, Some(&online), "polished"),
            MainAtSend::VariantOf("cap-1".into())
        );
        // Online as the main image: nothing to take.
        assert_eq!(
            decide(WISH_OVER, Some(&online), "natural"),
            MainAtSend::VariantOf("cap-1".into())
        );
    }

    #[test]
    fn no_capture_of_this_series_online_leaves_as_queued() {
        let online = night("private", &[("natural", true)]);
        let other = Series {
            exposure_seconds: 10.0,
            ..series()
        };
        for wish in [WISH_KEEP, WISH_OVER] {
            assert_eq!(
                main_at_send(wish, Some(&online), "m45", Some(&other), "polished"),
                MainAtSend::AsQueued
            );
            assert_eq!(
                main_at_send(wish, Some(&online), "m31", Some(&series()), "polished"),
                MainAtSend::AsQueued
            );
        }
    }

    #[test]
    fn a_space_that_cannot_be_read_never_decides() {
        for wish in [WISH_KEEP, WISH_OVER] {
            assert_eq!(decide(wish, None, "polished"), MainAtSend::Wait);
        }
    }

    /// A transport that answers what the night holds and records the create
    /// step; the create is then refused, which ends the test.
    struct Recording {
        online: Option<NightOnline>,
        created: RefCell<Vec<UploadMetadata>>,
        asked: RefCell<u32>,
    }

    impl UploadTransport for Recording {
        fn create(
            &self,
            _token: &str,
            metadata: &UploadMetadata,
        ) -> Result<UploadSession, UploadError> {
            self.created.borrow_mut().push(metadata.clone());
            Err(UploadError::refusal(
                400,
                "stop_here",
                "The test ends here.",
            ))
        }
        fn status(&self, _t: &str, _u: &str) -> Result<UploadSession, UploadError> {
            unreachable!()
        }
        fn put_chunk(
            &self,
            _t: &str,
            _u: &str,
            _o: u64,
            _c: &[u8],
        ) -> Result<PutOutcome, UploadError> {
            unreachable!()
        }
        fn finalize(&self, _t: &str, _u: &str) -> Result<FinalizeOutcome, UploadError> {
            unreachable!()
        }
        fn night_online_now(&self, _token: &str, _session_id: &str) -> Option<NightOnline> {
            *self.asked.borrow_mut() += 1;
            self.online.clone()
        }
    }

    /// The head image of a series, queued with `wish`, then sent: what the
    /// create step carried (`None`: it never got there) and the error.
    fn sent(
        online: Option<NightOnline>,
        wish: Option<&str>,
        variant_of: Option<&str>,
    ) -> (Option<UploadMetadata>, Option<String>, u32) {
        sent_again(online, wish, variant_of, 1)
    }

    /// The same, the image tried `times` times one after the other on the same
    /// queue: what the last try did.
    fn sent_again(
        online: Option<NightOnline>,
        wish: Option<&str>,
        variant_of: Option<&str>,
        times: u32,
    ) -> (Option<UploadMetadata>, Option<String>, u32) {
        let dir = crate::test_support::TempDir::new("main-choice");
        let stacked = dir.path().join("night").join("M 45").join("stacked");
        std::fs::create_dir_all(&stacked).unwrap();
        let render = stacked.join("M 45_polished.png");
        std::fs::write(&render, b"the polished render").unwrap();
        let queue = UploadQueue::open(&dir.path().join("queue.sqlite")).unwrap();
        let proposals = crate::metadata::Proposals {
            series: Some(series()),
            ..Default::default()
        };
        queue
            .enqueue_results(&[ResultFile {
                path: render.clone(),
                sha256: "polished".into(),
                size: 19,
                kind: ResultKind::Stacked,
                target: EntryTarget {
                    session_id: Some("night-1".into()),
                    target_ids: vec!["m45".into()],
                    variant_of: variant_of.map(str::to_string),
                    source: Some("stacking".into()),
                    metadata: Some(proposals),
                    main_wish: wish.map(str::to_string),
                    ..EntryTarget::default()
                },
            }])
            .unwrap();
        let entry: QueueEntry = queue.find_by_sha256("polished").unwrap().unwrap();
        let transport = Recording {
            online,
            created: RefCell::default(),
            asked: RefCell::default(),
        };
        let mut error = None;
        for _ in 0..times {
            let outcome = crate::uploader::upload_entry(
                &transport,
                &queue,
                "token",
                &entry,
                &UploadTarget::default(),
                |_, _| {},
            );
            error = outcome.err().map(|e| e.to_string());
        }
        let created = transport.created.borrow().last().cloned();
        let asked = *transport.asked.borrow();
        (created, error, asked)
    }

    #[test]
    fn the_default_keeps_the_main_image_of_a_capture_that_is_online_when_it_leaves() {
        let online = night("private", &[("natural", true)]);
        let (created, _, _) = sent(Some(online), Some(WISH_KEEP), None);
        let created = created.expect("the image was sent");
        assert_eq!(created.capture_id.as_deref(), Some("cap-1"));
        assert_eq!(created.placement, None);
    }

    #[test]
    fn a_chosen_image_leaves_without_a_capture_when_the_space_confirms() {
        let online = night("private", &[("natural", true)]);
        let (created, _, _) = sent(Some(online), Some(WISH_OVER), None);
        assert_eq!(created.expect("sent").capture_id, None);
    }

    #[test]
    fn a_chosen_image_is_a_variant_of_a_public_capture() {
        let online = night("public", &[("natural", true)]);
        let (created, _, _) = sent(Some(online), Some(WISH_OVER), None);
        assert_eq!(created.expect("sent").capture_id.as_deref(), Some("cap-1"));
    }

    #[test]
    fn offline_the_image_waits_and_nothing_is_created() {
        for wish in [WISH_KEEP, WISH_OVER] {
            let (created, error, asked) = sent(None, Some(wish), None);
            assert!(created.is_none(), "{wish}: nothing may be created");
            assert_eq!(asked, 1);
            assert!(error.unwrap().contains("trying again later"));
        }
    }

    #[test]
    fn a_space_that_stays_unreadable_ends_the_wait_with_a_new_variant() {
        for wish in [WISH_KEEP, WISH_OVER] {
            // The first readings fail: the image waits, nothing is created.
            let (created, error, asked) =
                sent_again(None, Some(wish), None, READS_BEFORE_VARIANT - 1);
            assert!(created.is_none(), "{wish}: still waiting");
            assert_eq!(asked, READS_BEFORE_VARIANT - 1);
            assert!(error.unwrap().contains("trying again later"));
            // The last one: a new variant, never the main image's place.
            let (created, _, asked) = sent_again(None, Some(wish), None, READS_BEFORE_VARIANT);
            let created = created.expect("the wait has an end");
            assert_eq!(asked, READS_BEFORE_VARIANT);
            assert_eq!(created.placement.as_deref(), Some("add"), "{wish}");
            assert_eq!(created.capture_id, None);
        }
    }

    #[test]
    fn the_count_of_failed_readings_is_kept_by_the_queue_and_only_for_its_image() {
        let dir = crate::test_support::TempDir::new("main-choice-count");
        let path = dir.path().join("queue.sqlite");
        let file = |sha: &str| ResultFile {
            path: format!("/n/M45/stacked/{sha}.png").into(),
            sha256: sha.into(),
            size: 3,
            kind: ResultKind::Stacked,
            target: EntryTarget::default(),
        };
        {
            let queue = UploadQueue::open(&path).unwrap();
            queue.enqueue_results(&[file("a"), file("b")]).unwrap();
            assert_eq!(queue.count_unread_main("a").unwrap(), 1);
            assert_eq!(queue.count_unread_main("a").unwrap(), 2);
        }
        let queue = UploadQueue::open(&path).unwrap();
        assert_eq!(queue.count_unread_main("a").unwrap(), 3);
        assert_eq!(queue.count_unread_main("b").unwrap(), 1);
    }

    #[test]
    fn the_waiting_ends_at_the_bound_and_nowhere_else() {
        for reads in 1..READS_BEFORE_VARIANT {
            assert_eq!(when_unread(reads), MainAtSend::Wait);
        }
        assert_eq!(when_unread(READS_BEFORE_VARIANT), MainAtSend::NewVariant);
        assert_eq!(
            when_unread(READS_BEFORE_VARIANT + 3),
            MainAtSend::NewVariant
        );
    }

    #[test]
    fn without_a_wish_the_space_is_not_even_asked() {
        let online = night("private", &[("natural", true)]);
        let (created, _, asked) = sent(Some(online), None, None);
        assert_eq!(created.expect("sent").capture_id, None);
        assert_eq!(asked, 0);
    }

    #[test]
    fn an_image_that_names_its_capture_is_never_decided_again() {
        let online = night("private", &[("natural", true)]);
        let (created, _, asked) = sent(Some(online), Some(WISH_OVER), Some("cap-9"));
        assert_eq!(created.expect("sent").capture_id.as_deref(), Some("cap-9"));
        assert_eq!(asked, 0);
    }

    #[test]
    fn a_queue_written_before_the_wish_existed_is_read_as_before() {
        let target: EntryTarget = serde_json::from_str(
            r#"{"sessionId":"s","targetIds":["m45"],"variantOf":null,"source":"stacking"}"#,
        )
        .unwrap();
        assert_eq!(target.main_wish, None);
        let round = serde_json::to_string(&target).unwrap();
        assert!(!round.contains("mainWish"));
    }

    #[test]
    fn the_wish_survives_the_queue_and_an_older_queue_gains_its_column() {
        let dir = crate::test_support::TempDir::new("main-choice-column");
        let path = dir.path().join("queue.sqlite");
        {
            // A queue file written before the wish existed: the same table
            // without its column, with a row waiting.
            let old = rusqlite::Connection::open(&path).unwrap();
            old.execute_batch(
                "CREATE TABLE upload_queue (
                    id INTEGER PRIMARY KEY AUTOINCREMENT, source_path TEXT NOT NULL,
                    sha256 TEXT NOT NULL, size INTEGER NOT NULL, upload_id TEXT,
                    offset INTEGER NOT NULL DEFAULT 0, state TEXT NOT NULL,
                    capture_id TEXT, last_error TEXT, last_error_code TEXT,
                    created_at TEXT NOT NULL, updated_at TEXT NOT NULL, kind TEXT,
                    session_id TEXT, target_ids TEXT, variant_of TEXT, source TEXT,
                    metadata TEXT, frames TEXT, plan TEXT, space_id TEXT,
                    placement TEXT, replaces_sha256 TEXT);
                 INSERT INTO upload_queue (source_path, sha256, size, state, created_at, updated_at)
                 VALUES ('/n/M45/stacked/old.png', 'old', 3, 'queued', 'now', 'now');",
            )
            .unwrap();
        }
        let queue = UploadQueue::open(&path).unwrap();
        let old = queue.find_by_sha256("old").unwrap().unwrap();
        assert_eq!(old.target.main_wish, None);
        queue
            .enqueue_results(&[ResultFile {
                path: "/n/M45/stacked/new.png".into(),
                sha256: "new".into(),
                size: 3,
                kind: ResultKind::Stacked,
                target: EntryTarget {
                    main_wish: Some(WISH_OVER.into()),
                    ..EntryTarget::default()
                },
            }])
            .unwrap();
        let new = queue.find_by_sha256("new").unwrap().unwrap();
        assert_eq!(new.target.main_wish.as_deref(), Some(WISH_OVER));
    }
}
