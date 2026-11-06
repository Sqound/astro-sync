//! The generated texts of images already online: an image Astro Sync sent
//! before it wrote its texts in the ten languages carries a title and a
//! description in the language of the application of that day, stored as
//! the owner's (`capture-variant.schema.json`). This module recomputes them
//! from what Astro Sync recorded beside each render it made -- the pipeline,
//! the contrast, the colours and the softened stars of every output of a
//! `<name>_stacking.json` (a night) or a `<name>_plan.json` (a multi-night
//! plan), each output with the SHA-256 of its file -- and sends them as the
//! image's generated texts (`PATCH /v1/captures/{id}/variants/{imageId}`
//! `generatedTitle`, `generatedDescription`). The service then clears the
//! one-language title and description Astro Sync proposed before; a title
//! or a description the owner typed stays, and wins.
//!
//! An image is found by its digest: in a record first, else among the
//! images under the folder of the same size, hashed, whose file name says
//! the render as the stacker writes it (`<stem>_<pipeline>[_strong |
//! _presentation | _contrast-2.5][_colour-40][_stars-85]`; a plan's record
//! keeps only its last pipeline's outputs). An image Astro Sync did not make
//! (a photo, an image retouched elsewhere) is left as it is, and said so.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::gui_bridge::{GuiBridgeError, HttpPortalGuiTransport};
use crate::metadata::{self, GeneratedText, Proposals};
use crate::pipelines::PipelineList;
use crate::service_wait::RequestFactory;
use crate::stacking::{Colour, Contrast, Retouch, SoftStars};

/// How deep under the folder given the records are looked for: a working
/// folder holds `<night>/<target>/stacked/` and `<plan>/stacked/`.
const MAX_DEPTH: usize = 6;

/// What Astro Sync recorded of one file it made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderRecord {
    pub pipeline: String,
    pub contrast: Contrast,
    pub retouch: Retouch,
}

#[derive(Debug, Default, Deserialize)]
struct Record {
    #[serde(default)]
    outputs: Vec<RecordOutput>,
}

#[derive(Debug, Default, Deserialize)]
struct RecordOutput {
    #[serde(default)]
    pipeline: String,
    #[serde(default)]
    sha256: String,
    #[serde(default)]
    contrast: Contrast,
    #[serde(default)]
    colour: Colour,
    #[serde(default, rename = "softStars")]
    soft_stars: SoftStars,
}

/// Whether `name` is a record of the renders: `<name>_stacking.json` beside a
/// night's renders, `<name>_plan.json` beside a plan's.
fn is_record(name: &str) -> bool {
    name.ends_with("_stacking.json") || name.ends_with("_plan.json")
}

/// The extensions of the images a render is written as.
const IMAGE_EXTENSIONS: [&str; 5] = ["tif", "tiff", "png", "jpg", "jpeg"];

/// What Astro Sync made under a folder: the files its records list, by
/// SHA-256 (lower case), and the images beside them with their size, hashed
/// only when an image online of the same size is not in a record.
#[derive(Debug, Default)]
pub struct LocalRenders {
    pub records: BTreeMap<String, RenderRecord>,
    images: Vec<(PathBuf, u64)>,
    hashed: BTreeMap<PathBuf, String>,
}

impl LocalRenders {
    /// The render of the image of digest `sha256` and size `byte_size`, with
    /// the ids of the pipelines its file name may carry.
    pub fn find(
        &mut self,
        sha256: &str,
        byte_size: Option<u64>,
        pipeline_ids: &[String],
    ) -> Option<RenderRecord> {
        let sha256 = sha256.to_ascii_lowercase();
        if let Some(record) = self.records.get(&sha256) {
            return Some(record.clone());
        }
        let size = byte_size?;
        for (path, _) in self.images.iter().filter(|(_, bytes)| *bytes == size) {
            let digest = match self.hashed.get(path) {
                Some(digest) => digest.clone(),
                None => {
                    let Ok(digest) = crate::scanner::file_sha256(path) else {
                        continue;
                    };
                    let digest = digest.to_ascii_lowercase();
                    self.hashed.insert(path.clone(), digest.clone());
                    digest
                }
            };
            if digest == sha256 {
                let stem = path.file_stem()?.to_string_lossy().into_owned();
                return render_of_stem(&stem, pipeline_ids);
            }
        }
        None
    }
}

/// The render a file name says (its stem, without extension), the stacker's
/// rule: the pipeline's id, then the contrast, the colours and the stars.
/// `None` when no pipeline of `pipeline_ids` ends it (a retouch made
/// elsewhere, named by hand).
#[must_use]
pub fn render_of_stem(stem: &str, pipeline_ids: &[String]) -> Option<RenderRecord> {
    let mut rest = stem;
    let mut retouch = Retouch::default();
    if let Some((head, value)) = rest.rsplit_once("_stars-")
        && let Ok(stars) = value.parse::<SoftStars>()
    {
        retouch.soft_stars = stars;
        rest = head;
    }
    if let Some((head, value)) = rest.rsplit_once("_colour-")
        && let Ok(colour) = value.parse::<Colour>()
    {
        retouch.colour = colour;
        rest = head;
    }
    let mut contrast = Contrast::None;
    for (suffix, value) in [
        ("_strong", Contrast::Strong),
        ("_presentation", Contrast::Presentation),
    ] {
        if let Some(head) = rest.strip_suffix(suffix) {
            contrast = value;
            rest = head;
        }
    }
    if contrast.is_none()
        && let Some((head, value)) = rest.rsplit_once("_contrast-")
        && let Ok(gain) = value.parse::<Contrast>()
    {
        contrast = gain;
        rest = head;
    }
    let mut ids: Vec<&String> = pipeline_ids.iter().collect();
    // The longest first: `dualband-correction` before a shorter id it ends with.
    ids.sort_by_key(|id| std::cmp::Reverse(id.len()));
    ids.into_iter()
        .find(|id| rest.ends_with(&format!("_{id}")))
        .map(|id| RenderRecord {
            pipeline: id.clone(),
            contrast,
            retouch,
        })
}

/// Every file Astro Sync recorded under `root`, and the images beside them.
#[must_use]
pub fn local_renders(root: &Path) -> LocalRenders {
    let mut found = LocalRenders::default();
    collect(root, 0, &mut found);
    found
}

/// Every file Astro Sync recorded under `root`, by its SHA-256 (lower case).
#[must_use]
pub fn recorded_renders(root: &Path) -> BTreeMap<String, RenderRecord> {
    local_renders(root).records
}

fn collect(folder: &Path, depth: usize, found: &mut LocalRenders) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            if depth < MAX_DEPTH {
                collect(&path, depth + 1, found);
            }
            continue;
        }
        let extension = path
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if IMAGE_EXTENSIONS.contains(&extension.as_str()) {
            if let Ok(meta) = entry.metadata() {
                found.images.push((path, meta.len()));
            }
            continue;
        }
        if !is_record(&name) {
            continue;
        }
        let Some(record) = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<Record>(&text).ok())
        else {
            continue;
        };
        for output in record.outputs {
            if output.sha256.is_empty() || output.pipeline.is_empty() {
                continue;
            }
            found
                .records
                .entry(output.sha256.to_ascii_lowercase())
                .or_insert(RenderRecord {
                    pipeline: output.pipeline,
                    contrast: output.contrast,
                    retouch: Retouch {
                        colour: output.colour,
                        soft_stars: output.soft_stars,
                    },
                });
        }
    }
}

/// One image of a capture as the owner's resource says it (`variants`).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnlineImage {
    pub id: String,
    pub sha256: String,
    #[serde(default)]
    pub byte_size: Option<u64>,
    #[serde(default)]
    pub main: bool,
    #[serde(default)]
    pub title: Option<String>,
    /// Where `title` came from: `astro-sync` (cleared by the generated
    /// texts), `owner` or `import` (kept, and shown over them).
    #[serde(default)]
    pub title_source: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
}

/// `GET /v1/captures/{id}` answers `{ capture }`.
#[derive(Debug, Deserialize)]
struct CaptureAnswer {
    capture: CaptureImages,
}

#[derive(Debug, Deserialize)]
struct CaptureImages {
    #[serde(default)]
    variants: Vec<OnlineImage>,
}

/// What became of one image.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageTextsLine {
    pub capture_id: String,
    pub image_id: String,
    pub main: bool,
    /// The title the image had (the owner's field): `None` without one.
    pub title_before: Option<String>,
    /// Whether that title stays over the generated one: typed by the owner
    /// or proposed by the import (`titleSource` other than `astro-sync`).
    pub title_kept: bool,
    /// `sent`, `ready` (a dry run), `not_recorded` (no record of Astro Sync
    /// holds its digest) or `no_texts` (its pipeline is not known here).
    pub result: String,
    /// The generated title, in English and in French, when there is one.
    pub generated_en: Option<String>,
    pub generated_fr: Option<String>,
}

/// The two routes, behind a trait so tests never touch the network.
pub trait ImageTextsTransport {
    /// `GET /v1/captures/{id}`: the images of the capture.
    fn capture_images(
        &self,
        base_url: &str,
        token: &str,
        capture_id: &str,
    ) -> Result<Vec<OnlineImage>, GuiBridgeError>;

    /// `PATCH /v1/captures/{id}/variants/{imageId}` with the generated texts
    /// under Astro Sync's name.
    fn send_texts(
        &self,
        base_url: &str,
        token: &str,
        capture_id: &str,
        image_id: &str,
        body: &serde_json::Value,
    ) -> Result<(), GuiBridgeError>;
}

impl ImageTextsTransport for HttpPortalGuiTransport {
    fn capture_images(
        &self,
        base_url: &str,
        token: &str,
        capture_id: &str,
    ) -> Result<Vec<OnlineImage>, GuiBridgeError> {
        let url = format!(
            "{}/api/v1/captures/{capture_id}",
            base_url.trim_end_matches('/')
        );
        let token = token.to_owned();
        let build: RequestFactory =
            Arc::new(move |client| client.get(url.clone()).bearer_auth(&token));
        let answer: CaptureAnswer = self.read_json(&build)?;
        Ok(answer.capture.variants)
    }

    /// The same body twice changes nothing (`api/v1.md`): repeated freely
    /// while the service wakes.
    fn send_texts(
        &self,
        base_url: &str,
        token: &str,
        capture_id: &str,
        image_id: &str,
        body: &serde_json::Value,
    ) -> Result<(), GuiBridgeError> {
        let url = format!(
            "{}/api/v1/captures/{capture_id}/variants/{image_id}",
            base_url.trim_end_matches('/')
        );
        let token = token.to_owned();
        let body = body.clone();
        let build: RequestFactory =
            Arc::new(move |client| client.patch(url.clone()).bearer_auth(&token).json(&body));
        let _: serde_json::Value = self.read_json(&build)?;
        Ok(())
    }
}

/// The generated title and description of the file `record` describes.
#[must_use]
pub fn texts_of(
    record: &RenderRecord,
    pipelines: &PipelineList,
) -> (Option<GeneratedText>, Option<GeneratedText>) {
    metadata::generated_render_texts(
        Some(pipelines),
        &record.pipeline,
        record.contrast,
        record.retouch,
    )
}

/// Sends the generated texts of every image of `capture_id` Astro Sync made
/// under the folder (`local`), or only says what it would send (`dry_run`). The
/// progression images are the history of the capture: never named.
pub fn regenerate_capture<T: ImageTextsTransport>(
    transport: &T,
    base_url: &str,
    token: &str,
    capture_id: &str,
    local: &mut LocalRenders,
    pipelines: &PipelineList,
    dry_run: bool,
) -> Result<Vec<ImageTextsLine>, GuiBridgeError> {
    let mut pipeline_ids: Vec<String> = pipelines.pipelines.iter().map(|p| p.id.clone()).collect();
    pipeline_ids.push(crate::pipelines::LINEAR_PIPELINE.to_string());
    let mut lines = Vec::new();
    for image in transport.capture_images(base_url, token, capture_id)? {
        if image.role.is_some() {
            continue;
        }
        let mut line = ImageTextsLine {
            capture_id: capture_id.to_string(),
            image_id: image.id.clone(),
            main: image.main,
            title_before: image.title.clone(),
            title_kept: image.title.is_some()
                && image.title_source.as_deref() != Some(metadata::SOURCE),
            result: "not_recorded".to_string(),
            generated_en: None,
            generated_fr: None,
        };
        if let Some(record) = local.find(&image.sha256, image.byte_size, &pipeline_ids) {
            let (title, description) = texts_of(&record, pipelines);
            if title.is_none() && description.is_none() {
                line.result = "no_texts".to_string();
            } else {
                line.generated_en = title.as_ref().and_then(|t| t.get("en").cloned());
                line.generated_fr = title.as_ref().and_then(|t| t.get("fr").cloned());
                let proposals = Proposals {
                    variant_generated_title: title,
                    variant_generated_description: description,
                    ..Proposals::default()
                };
                if dry_run {
                    line.result = "ready".to_string();
                } else {
                    transport.send_texts(
                        base_url,
                        token,
                        capture_id,
                        &image.id,
                        &crate::uploader::image_proposed_body(&proposals),
                    )?;
                    line.result = "sent".to_string();
                }
            }
        }
        lines.push(line);
    }
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipelines;
    use crate::test_support::TempDir;
    use std::cell::RefCell;

    #[derive(Default)]
    struct FakeTransport {
        images: Vec<OnlineImage>,
        sent: RefCell<Vec<(String, serde_json::Value)>>,
    }

    impl ImageTextsTransport for FakeTransport {
        fn capture_images(
            &self,
            _: &str,
            _: &str,
            _: &str,
        ) -> Result<Vec<OnlineImage>, GuiBridgeError> {
            Ok(self.images.clone())
        }

        fn send_texts(
            &self,
            _: &str,
            _: &str,
            _: &str,
            image_id: &str,
            body: &serde_json::Value,
        ) -> Result<(), GuiBridgeError> {
            self.sent
                .borrow_mut()
                .push((image_id.to_string(), body.clone()));
            Ok(())
        }
    }

    fn image(id: &str, sha: &str, title: &str) -> OnlineImage {
        OnlineImage {
            id: id.into(),
            sha256: sha.into(),
            byte_size: Some(1),
            main: id == "main",
            title: Some(title.into()),
            title_source: Some("astro-sync".into()),
            role: None,
        }
    }

    #[test]
    fn the_images_astro_sync_made_get_their_texts_in_the_ten_languages() {
        let root = TempDir::new("image-texts");
        let plan = root.path().join("Sh2-103 · 1ceaff/stacked");
        let night = root.path().join("2026-08-17 · fe9242/NGC 7000/stacked");
        std::fs::create_dir_all(&plan).unwrap();
        std::fs::create_dir_all(&night).unwrap();
        std::fs::write(
            plan.join("Sh2-103_plan-3_plan.json"),
            serde_json::json!({ "outputs": [
                { "pipeline": "dualband-correction", "file": "a.tif", "sha256": "AA" },
                { "pipeline": "dualband-correction", "file": "b.tif", "sha256": "bb",
                  "contrast": "strong", "colour": "40", "softStars": "85" }
            ]})
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            night.join("NGC_7000_2026-08-17_stacking.json"),
            serde_json::json!({ "outputs": [
                { "pipeline": "raw", "file": "c.tif", "sha256": "cc",
                  "contrast": "strong", "colour": 20, "softStars": 50 }
            ]})
            .to_string(),
        )
        .unwrap();
        std::fs::write(night.join("notes.json"), "{}").unwrap();
        // A render no record lists (a plan's record keeps its last pipeline
        // only): found by its digest among the images of its size, named by
        // its file; a retouch named by hand is not.
        let finish_path = plan.join("Sh2-103_plan-3_hoo_strong_colour-40_stars-85.tif");
        let retouch_path = plan.join("Sh2-103_plan_hoo_contraste.tif");
        std::fs::write(&finish_path, b"finish-bytes").unwrap();
        std::fs::write(&retouch_path, b"retouched!!!").unwrap();
        let finish_sha = crate::scanner::file_sha256(&finish_path).unwrap();
        let retouch_sha = crate::scanner::file_sha256(&retouch_path).unwrap();
        let records = recorded_renders(root.path());
        assert_eq!(records.len(), 3);
        assert_eq!(records["aa"].pipeline, "dualband-correction");
        assert!(records["aa"].contrast.is_none());

        let transport = FakeTransport {
            images: vec![
                image(
                    "main",
                    "bb",
                    "Correction bibande · Contraste 3,6 · Couleurs +40\u{a0}% · Étoiles adoucies 85\u{a0}%",
                ),
                image("v2", "aa", "Correction bibande"),
                image(
                    "v3",
                    "cc",
                    "Rendu naturel · Contraste 3,6 · Couleurs +20\u{a0}% · Étoiles adoucies 50\u{a0}%",
                ),
                image("v4", "dd", "Retouche"),
                OnlineImage {
                    byte_size: Some(12),
                    ..image("v5", &finish_sha, "Palette HOO · Contraste 3,6")
                },
                OnlineImage {
                    byte_size: Some(12),
                    ..image("v6", &retouch_sha, "Palette HOO · contraste")
                },
                OnlineImage {
                    role: Some("progression".into()),
                    ..image("p1", "aa", "")
                },
            ],
            ..FakeTransport::default()
        };
        let list = pipelines::read_manifests(None, None, "fr");
        let mut local = local_renders(root.path());
        let dry = regenerate_capture(&transport, "https://s", "t", "c1", &mut local, &list, true)
            .unwrap();
        assert!(
            transport.sent.borrow().is_empty(),
            "a dry run sends nothing"
        );
        assert_eq!(
            dry.iter().map(|l| l.result.as_str()).collect::<Vec<_>>(),
            [
                "ready",
                "ready",
                "ready",
                "not_recorded",
                "ready",
                "not_recorded"
            ]
        );
        assert_eq!(
            dry[4].generated_en.as_deref(),
            Some("HOO palette · Contrast 3.6 · Colours +40% · Softened stars 85%")
        );
        let lines =
            regenerate_capture(&transport, "https://s", "t", "c1", &mut local, &list, false)
                .unwrap();
        // The title the product gave them, now in the ten languages.
        assert_eq!(
            lines[0].generated_fr.as_deref(),
            lines[0].title_before.as_deref()
        );
        assert_eq!(
            lines[2].generated_fr.as_deref(),
            lines[2].title_before.as_deref()
        );
        assert_eq!(
            lines[0].generated_en.as_deref(),
            Some("Dual-band correction · Contrast 3.6 · Colours +40% · Softened stars 85%")
        );
        let sent = transport.sent.borrow();
        assert_eq!(sent.len(), 4);
        let (id, body) = &sent[0];
        assert_eq!(id, "main");
        assert_eq!(body["source"], "astro-sync");
        assert_eq!(
            body["generatedTitle"].as_object().unwrap().len(),
            metadata::TEXT_LANGUAGES.len()
        );
        assert!(body["generatedDescription"]["en"].is_string());
        assert!(body.get("title").is_none() && body.get("description").is_none());
    }

    #[test]
    fn a_file_name_says_its_render_as_the_stacker_writes_it() {
        let ids: Vec<String> = ["raw", "hoo", "dualband-correction", "linear"]
            .iter()
            .map(|id| (*id).to_string())
            .collect();
        let of = |stem: &str| render_of_stem(stem, &ids);
        let plain = of("M31_2026-10-07_dualband-correction").unwrap();
        assert_eq!(plain.pipeline, "dualband-correction");
        assert!(plain.contrast.is_none() && plain.retouch.is_none());
        let finish = of("Sh2-103_plan-3_raw_strong_colour-40_stars-85").unwrap();
        assert_eq!(finish.pipeline, "raw");
        assert_eq!(finish.contrast, Contrast::Strong);
        assert_eq!(finish.retouch.colour.percent(), 40);
        assert_eq!(finish.retouch.soft_stars.percent(), Some(85));
        let gain = of("NGC_7000_hoo_contrast-2.5_stars-50").unwrap();
        assert_eq!(gain.contrast.gain(), Some(2.5));
        assert!(gain.retouch.colour.is_none());
        assert_eq!(
            of("M31_raw_presentation").unwrap().contrast,
            Contrast::Presentation
        );
        assert_eq!(
            of("Sh2-103_plan_dualband-correction_contraste-etoiles"),
            None
        );
        assert_eq!(of("IMG_2041"), None);
    }
}
