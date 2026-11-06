//! The render pipelines of the stacking (`stacking/pipelines/
//! SCHEMA.md`): one `pipeline.json` per way of rendering the stacked image,
//! shipped with Astro Sync (`raw`, `dualband-correction`) or added by the
//! person in `<Astro Sync data>/pipelines/<id>/`.
//!
//! [`list_pipelines`] answers the "Processing" screen:
//!
//! - **from the stacking script** (`astro_sync_stack.py list-pipelines
//!   --json`) when its Python environment is present: the script's own
//!   validation, its dependencies resolved against the registry, each
//!   script's `scriptSha256`;
//! - **else from the manifests directly**, read here with the rules of
//!   `SCHEMA.md` that decide whether a definition is listed or ignored (the
//!   id is the folder's, `en` texts present, the two `entry` forms, the
//!   dependencies named by the registry and nothing else, a script inside
//!   its folder) -- so the screen lists the pipelines before the
//!   environment exists.
//!
//! An ignored definition is returned with its reason, never hidden.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use crate::external_process;
use crate::pipeline_trust::{TrustState, TrustStore};
use crate::plan_image::Band;
use crate::python_env;
use crate::tools::{self, DependencyKind, Registry, ToolRow, ToolState};

/// The definitions shipped with Astro Sync, embedded at build time (the
/// folder `stacking/pipelines/`; a pipeline added there by a pull
/// request is added to this list).
pub const SHIPPED: [(&str, &str); 4] = [
    (
        "raw",
        include_str!("../../../stacking/pipelines/raw/pipeline.json"),
    ),
    (
        "dualband-correction",
        include_str!("../../../stacking/pipelines/dualband-correction/pipeline.json"),
    ),
    (
        "hoo",
        include_str!("../../../stacking/pipelines/hoo/pipeline.json"),
    ),
    (
        "polished",
        include_str!("../../../stacking/pipelines/polished/pipeline.json"),
    ),
];

/// The preview image of each shipped definition (`preview` of its
/// `pipeline.json`), embedded like the definitions so that the listing
/// carries it even when the app has no `stacking/` folder to read.
const SHIPPED_PREVIEWS: [(&str, &[u8]); 4] = [
    (
        "raw",
        include_bytes!("../../../stacking/pipelines/raw/preview.jpg"),
    ),
    (
        "dualband-correction",
        include_bytes!("../../../stacking/pipelines/dualband-correction/preview.jpg"),
    ),
    (
        "hoo",
        include_bytes!("../../../stacking/pipelines/hoo/preview.jpg"),
    ),
    (
        "polished",
        include_bytes!("../../../stacking/pipelines/polished/preview.jpg"),
    ),
];

/// The largest preview image read (`SCHEMA.md`: 512 KiB; 150 KiB advised).
const MAX_PREVIEW_BYTES: u64 = 512 * 1024;

/// The pipeline used when none is chosen.
pub const DEFAULT_PIPELINE: &str = "raw";

/// The id of "No processing": not a pipeline, the option that runs no
/// render and keeps only the linear stack as a TIFF (`<name>_linear.tif`).
/// It is reserved: no definition may take it (`stacking/pipeline_defs.py`
/// refuses it too).
pub const LINEAR_PIPELINE: &str = "linear";

/// "No processing" (name) and its one-line help (description) in the
/// ten languages of the application: the title and description of the
/// variant the linear image leaves as (a pipeline's come from its
/// definition; this option has none). Same words as the wizard's
/// `wizard.treatments.noProcessing*` texts.
const LINEAR_TEXTS: [(&str, &str, &str); 10] = [
    (
        "fr",
        "Sans traitement",
        "Pour traiter vous-même\u{a0}: l’empilement calibré et aligné, sans étirement.",
    ),
    (
        "en",
        "No processing",
        "To process it yourself: the calibrated, aligned stack, not stretched.",
    ),
    (
        "ja",
        "処理なし",
        "ご自身で処理する方向けです。較正と位置合わせを済ませた積算画像を、引き伸ばさずにお渡しします。",
    ),
    (
        "de",
        "Ohne Bearbeitung",
        "Zum Selbstbearbeiten: der kalibrierte und ausgerichtete Stack, ungestreckt.",
    ),
    (
        "es",
        "Sin procesar",
        "Para procesarla tú mismo: el apilado calibrado y alineado, sin estirar.",
    ),
    (
        "it",
        "Senza elaborazione",
        "Per elaborarla tu stesso: la somma calibrata e allineata, senza stiramento.",
    ),
    (
        "pt-BR",
        "Sem processamento",
        "Para você mesmo processar: o empilhamento calibrado e alinhado, sem esticamento.",
    ),
    (
        "ru",
        "Без обработки",
        "Чтобы обработать самостоятельно: калиброванный и выровненный стек без растяжения.",
    ),
    (
        "zh-Hans",
        "不作处理",
        "供你自行处理：已校准并对齐的叠加结果，未做拉伸。",
    ),
    (
        "zh-Hant",
        "不作處理",
        "供你自行處理：已校正並對齊的疊加結果，未做拉伸。",
    ),
];

/// The name and the help of "No processing" in `language` (then its base
/// language, then English).
#[must_use]
pub fn linear_texts(language: &str) -> (String, String) {
    let base = language.split(['-', '_']).next().unwrap_or(language);
    let row = |code: &str| LINEAR_TEXTS.iter().find(|(known, _, _)| *known == code);
    let (_, name, help) = row(language)
        .or_else(|| row(base))
        .or_else(|| row("en"))
        .copied()
        .unwrap_or(("en", "", ""));
    (name.to_string(), help.to_string())
}

/// One dependency a pipeline names, by the registry's id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineDependency {
    pub id: String,
    pub required: bool,
    #[serde(default)]
    pub reason: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_version: Option<String>,
    /// What the person downloads for it on this system, in bytes
    /// ("Download (130 MB)"), set by [`list_pipelines`] from the
    /// registry; never read from a manifest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
}

/// One step of a pipeline's `details`: what it does and why, per
/// language.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipelineDetail {
    pub title: BTreeMap<String, String>,
    pub text: BTreeMap<String, String>,
}

/// One step of `details` in the person's language.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayDetail {
    pub title: String,
    pub text: String,
}

/// The steps of `details` in `language` (then its base language, then English).
#[must_use]
pub fn display_details(details: &[PipelineDetail], language: &str) -> Vec<DisplayDetail> {
    details
        .iter()
        .map(|step| DisplayDetail {
            title: tools::localized(&step.title, language),
            text: tools::localized(&step.text, language),
        })
        .collect()
}

/// A pipeline, listed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pipeline {
    pub id: String,
    pub version: String,
    pub name: BTreeMap<String, String>,
    pub description: BTreeMap<String, String>,
    pub filter: String,
    pub author: String,
    pub licence: String,
    pub entry: String,
    pub requires: Vec<String>,
    pub outputs: Vec<String>,
    #[serde(default)]
    pub dependencies: Vec<PipelineDependency>,
    /// The ordered steps, what each does and why (optional).
    #[serde(default)]
    pub details: Vec<PipelineDetail>,
    /// The relative path (to the pipeline's folder) of its preview image:
    /// checked (PNG or JPEG, inside the folder, 512 KiB at most), else `None`.
    #[serde(default)]
    pub preview: Option<String>,
    /// Why a `preview` that was written is not shown (the pipeline stays
    /// listed, without a preview).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview_issue: Option<String>,
    /// The preview image as a data URL, read when the list is made (set by
    /// [`list_pipelines`]; never read from a manifest).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview_data: Option<String>,
    /// `shipped` or `user`.
    #[serde(default)]
    pub source: String,
    /// The SHA-256 of a `script:` entry's file, `None` for `renders:`.
    #[serde(default)]
    pub script_sha256: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub display_description: Option<String>,
    /// `details` in the language of the listing.
    #[serde(default)]
    pub display_details: Option<Vec<DisplayDetail>>,
    /// The person's trust in its script, set by
    /// [`with_trust`].
    #[serde(default)]
    pub trust: Option<TrustState>,
    /// The bands of target it is recommended for ("Recommended"),
    /// from its `filter`: set by [`list_pipelines`].
    #[serde(default)]
    pub recommended_for: Vec<Band>,
    /// The bands of target it is discouraged for ("Not advised for this
    /// filter"): proposed all the same, never refused.
    #[serde(default)]
    pub discouraged_for: Vec<Band>,
}

/// The bands of target a pipeline made for `filter` is recommended and
/// discouraged for (recommended or discouraged, never
/// refused): a `dual-band` render is
/// recommended on a dual-band target and discouraged on a broadband one; a
/// render for every filter (`none`) is recommended on a broadband target
/// only; any other filter says nothing.
#[must_use]
pub fn recommendations(filter: &str) -> (Vec<Band>, Vec<Band>) {
    match filter {
        "dual-band" => (vec![Band::Dualband], vec![Band::Broadband]),
        "none" => (vec![Band::Broadband], Vec::new()),
        _ => (Vec::new(), Vec::new()),
    }
}

/// Sets what [`list_pipelines`] adds to each pipeline: its recommendations
/// and the download size of each of its dependencies on this system.
fn with_listing_facts(mut pipeline: Pipeline, registry: &Registry) -> Pipeline {
    let (recommended, discouraged) = recommendations(&pipeline.filter);
    pipeline.recommended_for = recommended;
    pipeline.discouraged_for = discouraged;
    for dependency in &mut pipeline.dependencies {
        dependency.bytes = registry
            .entry(&dependency.id)
            .and_then(tools::download_bytes);
    }
    pipeline
}

impl Pipeline {
    /// Whether it runs a script the person added (trusted by them, one
    /// digest at a time: `trustedSha256`).
    #[must_use]
    pub fn needs_trust(&self) -> bool {
        self.source == "user" && self.entry.starts_with("script:")
    }
}

/// A definition left out, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IgnoredPipeline {
    pub folder: String,
    pub source: String,
    pub reason: String,
}

/// Where the list came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListedBy {
    /// `astro_sync_stack.py list-pipelines --json`.
    Script,
    /// The manifests, read here.
    Manifests,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineList {
    pub pipelines: Vec<Pipeline>,
    pub ignored: Vec<IgnoredPipeline>,
    pub listed_by: ListedBy,
    /// The folder where the person's own definitions go
    /// (`<data>/pipelines`), when the listing was given one
    /// ("Open the pipelines folder").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_folder: Option<String>,
}

impl PipelineList {
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Pipeline> {
        self.pipelines.iter().find(|p| p.id == id)
    }
}

/// The stacking's Python environment, when present.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackingEnvironment {
    pub python: PathBuf,
    /// `astro_sync_stack.py`.
    pub script: PathBuf,
}

impl StackingEnvironment {
    /// The environment in Astro Sync's data (`stacking-env/`) and the script
    /// beside the app, when both exist.
    #[must_use]
    pub fn find(data_dir: &Path, script: &Path) -> Option<Self> {
        let python = python_env::env_python(data_dir);
        (python.is_file() && script.is_file()).then(|| Self {
            python,
            script: script.to_path_buf(),
        })
    }
}

/* ------------------------------- Manifests ------------------------------- */

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 40
        && !id.starts_with('-')
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn script_path(folder: &Path, relative: &str) -> Result<PathBuf, String> {
    let relative_path = Path::new(relative);
    let inside = relative.ends_with(".py")
        && !relative.contains('\\')
        && relative_path
            .components()
            .all(|c| matches!(c, Component::Normal(_)));
    if !inside {
        return Err(format!(
            "script path {relative:?} must stay inside the pipeline's folder"
        ));
    }
    let path = folder.join(relative_path);
    let canonical_folder = fs::canonicalize(folder).map_err(|e| e.to_string())?;
    let canonical =
        fs::canonicalize(&path).map_err(|_| format!("script {relative:?} not found"))?;
    if !canonical.starts_with(&canonical_folder) {
        return Err(format!(
            "script {relative:?} leads out of the pipeline's folder"
        ));
    }
    Ok(canonical)
}

/// Whether `relative` is a path `preview` may name: relative to the
/// pipeline's folder, `/` separators, no `..`, ending in `.png`, `.jpg` or
/// `.jpeg` (`pipeline_defs.PREVIEW_PATH_PATTERN`).
fn preview_path_ok(relative: &str) -> bool {
    let lower = relative.to_ascii_lowercase();
    let extension = [".png", ".jpg", ".jpeg"]
        .iter()
        .any(|extension| lower.ends_with(extension));
    extension
        && !relative.contains('\\')
        && Path::new(relative)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
        && relative.split('/').all(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .is_some_and(|first| first.is_ascii_alphanumeric() || first == '_')
                && part
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
        })
}

/// The media type of an image by its first bytes (PNG or JPEG), never by
/// its extension.
fn image_media_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else {
        None
    }
}

/// The preview image of a definition: `relative` inside `folder`, a PNG or
/// a JPEG of 512 KiB at most; the reason otherwise. For a shipped
/// definition with no folder (embedded), the image embedded with it.
fn read_preview(id: &str, folder: Option<&Path>, relative: &str) -> Result<Vec<u8>, String> {
    if !preview_path_ok(relative) {
        return Err(format!(
            "preview: the path {relative:?} must be relative to the pipeline's folder, with '/' separators, no '..', ending in .png, .jpg or .jpeg"
        ));
    }
    let bytes = match folder {
        Some(folder) => {
            let path = fs::canonicalize(folder.join(relative)).map_err(|_| {
                format!("preview: no such file {relative:?} in the pipeline's folder")
            })?;
            let canonical_folder = fs::canonicalize(folder).map_err(|e| e.to_string())?;
            if !path.starts_with(&canonical_folder) {
                return Err("preview: the image must stay inside the pipeline's folder".to_string());
            }
            let metadata = fs::metadata(&path).map_err(|e| format!("preview: unreadable: {e}"))?;
            if !metadata.is_file() {
                return Err(format!("preview: {relative:?} is not a file"));
            }
            if metadata.len() > MAX_PREVIEW_BYTES {
                return Err(format!(
                    "preview: the image is larger than {} KiB",
                    MAX_PREVIEW_BYTES / 1024
                ));
            }
            fs::read(&path).map_err(|e| format!("preview: unreadable: {e}"))?
        }
        None => SHIPPED_PREVIEWS
            .iter()
            .find(|(shipped, _)| *shipped == id)
            .filter(|_| relative == "preview.jpg")
            .map(|(_, bytes)| bytes.to_vec())
            .ok_or_else(|| {
                format!("preview: no such file {relative:?} in the pipeline's folder")
            })?,
    };
    if image_media_type(&bytes).is_none() {
        return Err("preview: the file is neither a PNG nor a JPEG image".to_string());
    }
    Ok(bytes)
}

const DEPENDENCY_FIELDS: [&str; 4] = ["id", "required", "reason", "minVersion"];

/// The most steps `details` may have (`pipeline_defs.MAX_DETAILS`).
const MAX_DETAILS: usize = 12;

/// Reads one definition with the rules of `SCHEMA.md`; `folder` is `None`
/// for an embedded one (no script possible).
fn read_manifest(
    text: &str,
    folder_name: &str,
    folder: Option<&Path>,
    source: &str,
    registry: &Registry,
) -> Result<Pipeline, String> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("not valid JSON: {e}"))?;
    if let Some(dependencies) = value.get("dependencies").and_then(|d| d.as_array()) {
        for dependency in dependencies {
            for key in dependency.as_object().into_iter().flat_map(|o| o.keys()) {
                if !DEPENDENCY_FIELDS.contains(&key.as_str()) {
                    return Err(format!(
                        "a dependency carries {key:?}: a pipeline names a dependency of the registry, nothing else"
                    ));
                }
            }
        }
    }
    // `preview` is optional and never makes the definition invalid: a bad
    // one is dropped here, and its reason kept (`preview_issue`).
    let mut value = value;
    let written_preview = value
        .as_object_mut()
        .and_then(|object| {
            object.remove("previewData");
            object.remove("previewIssue");
            object.remove("preview")
        })
        .filter(|preview| !preview.is_null());
    let mut pipeline: Pipeline =
        serde_json::from_value(value).map_err(|e| format!("invalid definition: {e}"))?;
    if !valid_id(&pipeline.id) || pipeline.id != folder_name {
        return Err(format!(
            "id {:?} must be the folder's name ({folder_name:?})",
            pipeline.id
        ));
    }
    if pipeline.id == LINEAR_PIPELINE {
        return Err(format!(
            "id {LINEAR_PIPELINE:?} is reserved: it names the option « No processing », which runs no pipeline"
        ));
    }
    if pipeline.version != "1" {
        return Err(format!("unknown version {:?}", pipeline.version));
    }
    if !pipeline.name.contains_key("en") || !pipeline.description.contains_key("en") {
        return Err("name and description need an English text".to_string());
    }
    if pipeline.details.len() > MAX_DETAILS
        || pipeline
            .details
            .iter()
            .any(|step| !step.title.contains_key("en") || !step.text.contains_key("en"))
    {
        return Err(format!(
            "details must be at most {MAX_DETAILS} steps, each with an English title and text"
        ));
    }
    if !pipeline.requires.iter().any(|r| r == "siril") {
        return Err("requires must list siril".to_string());
    }
    if !pipeline.outputs.iter().any(|o| o == "tif") {
        return Err("outputs must list tif".to_string());
    }
    pipeline.script_sha256 = None;
    if let Some(function) = pipeline.entry.strip_prefix("renders:") {
        if function.is_empty() {
            return Err("entry renders: needs a function".to_string());
        }
    } else if let Some(relative) = pipeline.entry.strip_prefix("script:") {
        let folder = folder.ok_or("a shipped definition cannot bring a script")?;
        let path = script_path(folder, relative)?;
        let bytes = fs::read(&path).map_err(|e| e.to_string())?;
        if bytes.len() > 256 * 1024 {
            return Err("the script is over 256 KiB".to_string());
        }
        pipeline.script_sha256 = Some(format!("{:x}", Sha256::digest(&bytes)));
    } else {
        return Err(format!(
            "entry {:?} must be renders:<function> or script:<path>",
            pipeline.entry
        ));
    }
    let mut seen = Vec::new();
    for dependency in &pipeline.dependencies {
        let Some(entry) = registry.entry(&dependency.id) else {
            return Err(format!(
                "dependency {:?} is not allowed (not in the dependency registry); propose it with a pull request on the repository",
                dependency.id
            ));
        };
        if seen.contains(&dependency.id) {
            return Err(format!("dependency {:?} is listed twice", dependency.id));
        }
        seen.push(dependency.id.clone());
        if !dependency.reason.contains_key("en") {
            return Err(format!(
                "dependency {:?} needs an English reason",
                dependency.id
            ));
        }
        if let Some(minimum) = &dependency.min_version {
            let lower = entry.kind == DependencyKind::Tool
                && entry
                    .min_version
                    .as_deref()
                    .is_some_and(|registry_min| !tools::at_least(minimum, registry_min));
            if lower || entry.kind != DependencyKind::Tool {
                return Err(format!(
                    "dependency {:?}: minVersion {minimum:?} is not allowed",
                    dependency.id
                ));
            }
        }
    }
    pipeline.preview = None;
    pipeline.preview_issue = None;
    pipeline.preview_data = None;
    if let Some(written) = written_preview {
        let checked = match written.as_str() {
            Some(relative) => {
                read_preview(&pipeline.id, folder, relative).map(|_| relative.to_string())
            }
            None => Err("preview: must be a text (a relative path)".to_string()),
        };
        match checked {
            Ok(relative) => pipeline.preview = Some(relative),
            Err(reason) => pipeline.preview_issue = Some(reason),
        }
    }
    pipeline.source = source.to_string();
    Ok(pipeline)
}

/// The preview image of `pipeline` as a data URL, read from its folder
/// (`shipped_dir/<id>` for a shipped definition, `user_dir/<id>` for the
/// person's; the embedded image when a shipped one has no folder). A preview
/// that cannot be read any more is dropped with its reason.
fn with_preview_data(
    mut pipeline: Pipeline,
    shipped_dir: Option<&Path>,
    user_dir: Option<&Path>,
) -> Pipeline {
    pipeline.preview_data = None;
    let Some(relative) = pipeline.preview.clone() else {
        return pipeline;
    };
    let folder = if pipeline.source == "user" {
        user_dir.map(|dir| dir.join(&pipeline.id))
    } else {
        shipped_dir.map(|dir| dir.join(&pipeline.id))
    };
    if pipeline.source == "user" && folder.is_none() {
        pipeline.preview = None;
        return pipeline;
    }
    match read_preview(&pipeline.id, folder.as_deref(), &relative) {
        Ok(bytes) => {
            let media = image_media_type(&bytes).unwrap_or("image/jpeg");
            pipeline.preview_data = Some(format!(
                "data:{media};base64,{}",
                crate::plan_image::base64(&bytes)
            ));
        }
        Err(reason) => {
            pipeline.preview = None;
            pipeline.preview_issue = Some(reason);
        }
    }
    pipeline
}

fn with_display(mut pipeline: Pipeline, language: &str) -> Pipeline {
    pipeline.display_name = Some(tools::localized(&pipeline.name, language));
    pipeline.display_description = Some(tools::localized(&pipeline.description, language));
    pipeline.display_details = Some(display_details(&pipeline.details, language));
    pipeline
}

/// The pipelines read from the manifests: the shipped ones (from
/// `shipped_dir` when given, else those embedded), then the person's in
/// `user_dir`. The first definition of an id wins; the default comes first.
#[must_use]
pub fn read_manifests(
    shipped_dir: Option<&Path>,
    user_dir: Option<&Path>,
    language: &str,
) -> PipelineList {
    let registry = tools::registry();
    let mut pipelines: Vec<Pipeline> = Vec::new();
    let mut ignored = Vec::new();
    let mut consider = |result: Result<Pipeline, String>, folder: &str, source: &str| match result {
        Ok(pipeline) if pipelines.iter().any(|p| p.id == pipeline.id) => {
            ignored.push(IgnoredPipeline {
                folder: folder.to_string(),
                source: source.to_string(),
                reason: format!("id {:?} is already taken", pipeline.id),
            });
        }
        Ok(pipeline) => pipelines.push(with_display(pipeline, language)),
        Err(reason) => ignored.push(IgnoredPipeline {
            folder: folder.to_string(),
            source: source.to_string(),
            reason,
        }),
    };
    match shipped_dir {
        Some(dir) => {
            for (folder, path) in definition_folders(dir) {
                let result = fs::read_to_string(path.join("pipeline.json"))
                    .map_err(|e| format!("no pipeline.json ({e})"))
                    .and_then(|text| {
                        read_manifest(&text, &folder, Some(&path), "shipped", &registry)
                    });
                consider(result, &folder, "shipped");
            }
        }
        None => {
            for (folder, text) in SHIPPED {
                consider(
                    read_manifest(text, folder, None, "shipped", &registry),
                    folder,
                    "shipped",
                );
            }
        }
    }
    if let Some(dir) = user_dir {
        for (folder, path) in definition_folders(dir) {
            let result = fs::read_to_string(path.join("pipeline.json"))
                .map_err(|e| format!("no pipeline.json ({e})"))
                .and_then(|text| read_manifest(&text, &folder, Some(&path), "user", &registry));
            consider(result, &folder, "user");
        }
    }
    pipelines.sort_by_key(|p| p.id != DEFAULT_PIPELINE);
    PipelineList {
        pipelines,
        ignored,
        listed_by: ListedBy::Manifests,
        user_folder: None,
    }
}

fn definition_folders(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut folders: Vec<(String, PathBuf)> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .map(|entry| {
            (
                entry.file_name().to_string_lossy().into_owned(),
                entry.path(),
            )
        })
        .filter(|(name, _)| !name.starts_with('.') && !name.starts_with("__"))
        .collect();
    folders.sort();
    folders
}

/* ---------------------------------- Script ------------------------------- */

#[derive(Deserialize)]
struct ScriptAnswer {
    pipelines: Vec<Pipeline>,
    #[serde(default)]
    ignored: Vec<IgnoredPipeline>,
}

/// `list-pipelines --json` of the stacking script.
pub fn list_by_script(
    environment: &StackingEnvironment,
    user_dir: Option<&Path>,
    language: &str,
) -> Result<PipelineList, String> {
    let mut command = python_env::script_command(&environment.python, &environment.script);
    command.args(["list-pipelines", "--json", "--lang", language]);
    if let Some(dir) = user_dir {
        command.arg("--pipelines-dir").arg(dir);
    }
    let end =
        external_process::run(command, Duration::from_secs(60), None).map_err(|e| e.to_string())?;
    if !end.succeeded() {
        return Err(end.stderr.trim().to_string());
    }
    let answer: ScriptAnswer = serde_json::from_str(&end.stdout).map_err(|e| e.to_string())?;
    Ok(PipelineList {
        pipelines: answer.pipelines,
        ignored: answer.ignored,
        listed_by: ListedBy::Script,
        user_folder: None,
    })
}

/// The pipelines for "Processing": by the script when its environment is
/// present (and answers), else from the manifests.
#[must_use]
pub fn list_pipelines(
    environment: Option<&StackingEnvironment>,
    shipped_dir: Option<&Path>,
    user_dir: Option<&Path>,
    language: &str,
) -> PipelineList {
    let mut list = match environment.map(|e| list_by_script(e, user_dir, language)) {
        Some(Ok(list)) => list,
        _ => read_manifests(shipped_dir, user_dir, language),
    };
    list.user_folder = user_dir.map(|dir| dir.display().to_string());
    let registry = tools::registry();
    list.pipelines = list
        .pipelines
        .into_iter()
        .map(|pipeline| with_listing_facts(pipeline, &registry))
        .map(|pipeline| with_preview_data(pipeline, shipped_dir, user_dir))
        .collect();
    list
}

/// Each pipeline with the person's trust in its script.
#[must_use]
pub fn with_trust(mut list: PipelineList, store: &TrustStore) -> PipelineList {
    for pipeline in &mut list.pipelines {
        pipeline.trust = Some(store.state(pipeline));
    }
    list
}

/* ----------------------- "Processing pipelines" ---------------------- */

/// One dependency of a pipeline, resolved by the registry and its row of
/// "Dependencies".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SectionDependency {
    pub id: String,
    pub name: String,
    /// Why this pipeline needs it, in the person's language.
    pub reason: String,
    pub required: bool,
    pub state: ToolState,
    pub version: Option<String>,
    /// The minimum asked (the pipeline's when it asks more, else the
    /// registry's).
    pub min_version: Option<String>,
    pub licence: String,
    pub page: String,
    /// The size of what is downloaded for this system, when known: the
    /// registry's automatic download, else its `sizes`.
    pub bytes: Option<u64>,
    /// "Install": a download of the registry, or the Python environment
    /// made here.
    pub installable: bool,
    pub interrupted: bool,
    pub blocked_by: Vec<String>,
}

/// One block of the section: a pipeline and what it needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineSection {
    pub id: String,
    pub name: String,
    pub description: String,
    /// `shipped` or `user`.
    pub source: String,
    pub author: String,
    pub licence: String,
    pub trust: TrustState,
    pub script_sha256: Option<String>,
    pub dependencies: Vec<SectionDependency>,
    /// The steps of `details`, in the person's language ("Details").
    pub details: Vec<DisplayDetail>,
}

/// The section "Processing pipelines" of "Dependencies": one block
/// per pipeline found (shipped and added), one line per dependency it
/// declares, its state read from the rows of [`tools::detect_tools`] -- a
/// tool shared by several pipelines is detected once.
#[must_use]
pub fn sections(
    list: &PipelineList,
    rows: &[ToolRow],
    store: &TrustStore,
    language: &str,
) -> Vec<PipelineSection> {
    let registry = tools::registry();
    list.pipelines
        .iter()
        .map(|pipeline| PipelineSection {
            id: pipeline.id.clone(),
            name: pipeline
                .display_name
                .clone()
                .unwrap_or_else(|| tools::localized(&pipeline.name, language)),
            description: pipeline
                .display_description
                .clone()
                .unwrap_or_else(|| tools::localized(&pipeline.description, language)),
            source: pipeline.source.clone(),
            author: pipeline.author.clone(),
            licence: pipeline.licence.clone(),
            trust: store.state(pipeline),
            script_sha256: pipeline.script_sha256.clone(),
            details: pipeline
                .display_details
                .clone()
                .unwrap_or_else(|| display_details(&pipeline.details, language)),
            dependencies: pipeline
                .dependencies
                .iter()
                .filter_map(|dependency| {
                    let entry = registry.entry(&dependency.id)?;
                    let row = rows.iter().find(|r| r.id == dependency.id);
                    let min_version = dependency
                        .min_version
                        .clone()
                        .or_else(|| entry.min_version.clone());
                    let mut state = row.map_or(ToolState::Missing, |r| r.state);
                    if let (ToolState::Present, Some(minimum), Some(version)) = (
                        state,
                        dependency.min_version.as_deref(),
                        row.and_then(|r| r.version.as_deref()),
                    ) && !tools::at_least(version, minimum)
                    {
                        state = ToolState::Outdated;
                    }
                    let download = row.and_then(|r| r.download.clone());
                    Some(SectionDependency {
                        id: dependency.id.clone(),
                        name: tools::localized(&entry.name, language),
                        reason: tools::localized(&dependency.reason, language),
                        required: dependency.required,
                        state,
                        version: row.and_then(|r| r.version.clone()),
                        min_version,
                        licence: entry.licence.clone(),
                        page: entry.page.clone(),
                        // The automatic download's size, else the registry's
                        // `sizes` ("Download (90 MB)").
                        bytes: download
                            .as_ref()
                            .and_then(|d| d.bytes)
                            .or_else(|| tools::download_bytes(entry)),
                        installable: download.is_some() || row.is_some_and(|r| r.installable),
                        interrupted: row.is_some_and(|r| r.interrupted),
                        blocked_by: row.map(|r| r.blocked_by.clone()).unwrap_or_default(),
                    })
                })
                .collect(),
        })
        .collect()
}

/// "Install all" of the section: the ids of the required
/// dependencies missing (or too old) of every pipeline listed that Astro
/// Sync can install here, each once, the person's pipelines included.
#[must_use]
pub fn to_install(sections: &[PipelineSection]) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    for dependency in sections.iter().flat_map(|s| &s.dependencies) {
        if dependency.required
            && dependency.installable
            && matches!(dependency.state, ToolState::Missing | ToolState::Outdated)
            && !ids.contains(&dependency.id)
        {
            ids.push(dependency.id.clone());
        }
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    fn user_pipeline(dir: &Path, id: &str, extra: &str, entry: &str) -> PathBuf {
        let folder = dir.join(id);
        fs::create_dir_all(&folder).unwrap();
        fs::write(
            folder.join("pipeline.json"),
            format!(
                r#"{{"id": "{id}", "version": "1", "name": {{"en": "Warm Hα", "fr": "Hα chaud"}},
                "description": {{"en": "Warm tones."}}, "filter": "h-alpha", "author": "Camille",
                "licence": "MIT", "entry": "{entry}", "requires": ["siril"], "outputs": ["tif", "png"]
                {extra}}}"#
            ),
        )
        .unwrap();
        folder
    }

    #[test]
    fn the_shipped_pipelines_are_listed_from_their_manifests_raw_first() {
        let list = read_manifests(None, None, "fr");
        let ids: Vec<&str> = list.pipelines.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, vec!["raw", "dualband-correction", "hoo", "polished"]);
        assert_eq!(
            list.pipelines[0].display_name.as_deref(),
            Some("Rendu naturel")
        );
        // Every shipped pipeline details its steps, resolved in the
        // language of the listing.
        for pipeline in &list.pipelines {
            let shown = pipeline.display_details.as_ref().expect("resolved");
            assert!(shown.len() >= 5, "{}", pipeline.id);
            assert_eq!(shown.len(), pipeline.details.len());
            assert!(
                pipeline
                    .details
                    .iter()
                    .all(|step| step.title.len() == 10 && step.text.len() == 10)
            );
        }
        let polished = list.get("polished").unwrap();
        assert_eq!(polished.display_name.as_deref(), Some("Rendu soigné"));
        assert_eq!(
            polished.display_details.as_ref().unwrap()[0].title,
            "Fond de ciel"
        );
        assert!(polished.requires.iter().any(|r| r == "graxpert?"));
        assert_eq!(list.get("hoo").unwrap().filter, "dual-band");
        assert_eq!(list.pipelines[0].source, "shipped");
        assert!(list.ignored.is_empty(), "{:?}", list.ignored);
        assert_eq!(list.listed_by, ListedBy::Manifests);
        assert!(!list.pipelines[0].needs_trust());
        // The same from the folder of the repository.
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stacking/pipelines");
        let from_dir = read_manifests(Some(&dir), None, "fr");
        assert_eq!(from_dir.pipelines, list.pipelines);
    }

    #[test]
    fn a_pipeline_added_by_the_person_is_read_and_a_bad_one_is_said() {
        let dir = TempDir::new("pipelines-user");
        let warm = user_pipeline(
            dir.path(),
            "mono-ha",
            r#", "dependencies": [{"id": "siril", "required": true, "reason": {"en": "Siril stacks."}}]"#,
            "script:render.py",
        );
        fs::write(
            warm.join("render.py"),
            b"def render(nebula, stars):\n    return nebula\n",
        )
        .unwrap();
        user_pipeline(
            dir.path(),
            "with-url",
            r#", "dependencies": [{"id": "siril", "required": true, "reason": {"en": "x"}, "url": "https://x.invalid"}]"#,
            "renders:natural_deep",
        );
        user_pipeline(
            dir.path(),
            "unknown-tool",
            r#", "dependencies": [{"id": "pixinsight", "required": true, "reason": {"en": "x"}}]"#,
            "renders:natural_deep",
        );
        user_pipeline(dir.path(), "raw", "", "renders:natural_deep");
        user_pipeline(dir.path(), "outside", "", "script:../escape.py");
        user_pipeline(dir.path(), LINEAR_PIPELINE, "", "renders:natural_deep");
        let named = user_pipeline(dir.path(), "named", "", "renders:natural_deep");
        fs::rename(&named, dir.path().join("other-folder")).unwrap();

        let list = read_manifests(None, Some(dir.path()), "fr");
        let mono = list.get("mono-ha").unwrap();
        assert_eq!(mono.source, "user");
        assert!(mono.needs_trust());
        assert_eq!(
            mono.script_sha256.as_deref(),
            Some(
                format!(
                    "{:x}",
                    Sha256::digest(b"def render(nebula, stars):\n    return nebula\n")
                )
                .as_str()
            )
        );
        assert_eq!(mono.display_name.as_deref(), Some("Hα chaud"));
        let reason = |folder: &str| {
            list.ignored
                .iter()
                .find(|i| i.folder == folder)
                .map(|i| i.reason.clone())
                .unwrap_or_default()
        };
        assert!(reason("with-url").contains("\"url\""));
        assert!(reason("unknown-tool").contains("propose it with a pull request"));
        assert!(reason("raw").contains("already taken"));
        assert!(reason("outside").contains("inside"));
        assert!(reason(LINEAR_PIPELINE).contains("reserved"));
        assert!(reason("other-folder").contains("folder's name"));
        assert_eq!(list.pipelines.len(), 5);
    }

    #[test]
    fn without_processing_has_its_name_and_help_in_the_ten_languages() {
        let expected = [
            ("fr", "Sans traitement"),
            ("en", "No processing"),
            ("ja", "処理なし"),
            ("de", "Ohne Bearbeitung"),
            ("es", "Sin procesar"),
            ("it", "Senza elaborazione"),
            ("pt-BR", "Sem processamento"),
            ("ru", "Без обработки"),
            ("zh-Hans", "不作处理"),
            ("zh-Hant", "不作處理"),
        ];
        for (language, name) in expected {
            let (title, help) = linear_texts(language);
            assert_eq!(title, name, "{language}");
            assert!(!help.is_empty(), "{language}");
        }
        assert_eq!(
            linear_texts("fr").1,
            "Pour traiter vous-même\u{a0}: l’empilement calibré et aligné, sans étirement."
        );
        // A regional code falls back to its language, an unknown one to English.
        assert_eq!(linear_texts("de-AT").0, "Ohne Bearbeitung");
        assert_eq!(linear_texts("ko").0, "No processing");
        // It is not a shipped pipeline: the id is free of any definition.
        assert!(SHIPPED.iter().all(|(id, _)| *id != LINEAR_PIPELINE));
    }

    fn tool_row(id: &str, state: ToolState, version: Option<&str>) -> ToolRow {
        ToolRow {
            id: id.to_string(),
            name: BTreeMap::new(),
            kind: "tool".to_string(),
            required: false,
            state,
            version: version.map(ToString::to_string),
            min_version: None,
            path: None,
            page: None,
            licence: None,
            download: None,
            download_bytes: None,
            partial_bytes: None,
            progression_compatible: None,
            interrupted: false,
            installable: false,
            blocked_by: Vec::new(),
        }
    }

    #[test]
    fn the_section_resolves_each_dependency_by_the_registry_and_its_row() {
        let dir = TempDir::new("pipelines-section");
        let warm = user_pipeline(
            dir.path(),
            "mono-ha",
            r#", "dependencies": [{"id": "siril", "required": true, "reason": {"en": "Siril stacks.", "fr": "Siril additionne."}, "minVersion": "1.5"},
                {"id": "python-env", "required": true, "reason": {"en": "Python runs render.py."}}]"#,
            "script:render.py",
        );
        fs::write(warm.join("render.py"), b"def render(n, s):\n    return n\n").unwrap();
        let list = read_manifests(None, Some(dir.path()), "fr");
        let mut python = tool_row("python-env", ToolState::Missing, None);
        python.installable = true;
        python.interrupted = true;
        let rows = vec![
            tool_row("siril", ToolState::Present, Some("1.4.4")),
            tool_row("starnet2", ToolState::Missing, None),
            python,
        ];
        let store = TrustStore::default();
        let sections = sections(&list, &rows, &store, "fr");
        let ids: Vec<&str> = sections.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["raw", "dualband-correction", "hoo", "polished", "mono-ha"]
        );

        let raw = &sections[0];
        // "Details": the steps, in the person's language.
        assert_eq!(raw.details[0].title, "Fond de ciel");
        assert!(raw.details.iter().all(|step| !step.text.is_empty()));
        // The "polished render" names GraXpert, optional, with its reason.
        let polished = &sections[3];
        let graxpert = polished
            .dependencies
            .iter()
            .find(|d| d.id == "graxpert")
            .expect("GraXpert is declared");
        assert!(!graxpert.required);
        assert_eq!(graxpert.min_version.as_deref(), Some("3.0"));
        assert!(graxpert.reason.starts_with("GraXpert retire le fond"));
        assert!(graxpert.licence.contains("CC-BY-NC-SA-4.0"));
        assert_eq!(raw.name, "Rendu naturel");
        assert_eq!(raw.trust, TrustState::NotNeeded);
        let deps: Vec<(&str, bool, ToolState)> = raw
            .dependencies
            .iter()
            .map(|d| (d.id.as_str(), d.required, d.state))
            .collect();
        assert_eq!(
            deps,
            vec![
                ("siril", true, ToolState::Present),
                ("starnet2", false, ToolState::Missing),
                ("python-env", true, ToolState::Missing),
            ]
        );
        assert_eq!(raw.dependencies[0].min_version.as_deref(), Some("1.4"));
        assert!(raw.dependencies[2].interrupted);

        let mine = &sections[4];
        assert_eq!(mine.source, "user");
        assert_eq!(mine.trust, TrustState::Ask);
        assert_eq!(mine.dependencies[0].reason, "Siril additionne.");
        // The pipeline asks more than the registry: 1.4.4 is too old for it.
        assert_eq!(mine.dependencies[0].state, ToolState::Outdated);
        assert_eq!(mine.dependencies[1].reason, "Python runs render.py.");

        // "Install all": what Astro Sync can install, each once, the
        // person's pipeline included; Siril (no download) and the optional
        // StarNet2 are not in it.
        assert_eq!(to_install(&sections), vec!["python-env"]);

        let annotated = with_trust(list, &store);
        assert_eq!(
            annotated.get("mono-ha").unwrap().trust,
            Some(TrustState::Ask)
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_script_lists_them_when_its_environment_is_present() {
        use crate::external_process::fake_tool;
        let dir = TempDir::new("pipelines-script");
        let data = dir.path().join("data");
        fs::create_dir_all(data.join("stacking-env/bin")).unwrap();
        let answer = dir.path().join("answer.json");
        fs::write(
            &answer,
            r#"{"format": 1, "registry": [], "ignored": [{"folder": "x", "source": "user", "reason": "bad"}],
               "pipelines": [{"id": "raw", "version": "1", "name": {"en": "Natural render"}, "description": {"en": "d"},
               "filter": "none", "author": "Astro Sync", "licence": "MIT", "entry": "renders:natural_deep",
               "requires": ["siril"], "outputs": ["tif"], "source": "shipped", "scriptSha256": null,
               "displayName": "Rendu naturel", "dependencies": [{"id": "siril", "required": true,
               "reason": {"en": "x"}, "kind": "tool", "page": "https://siril.org/download/"}]}]}"#,
        )
        .unwrap();
        let args = dir.path().join("args.txt");
        fake_tool(
            &data.join("stacking-env/bin"),
            "python3",
            &format!(
                "echo \"$@\" > '{}'\ncat '{}'",
                args.display(),
                answer.display()
            ),
        );
        let script = dir.path().join("astro_sync_stack.py");
        fs::write(&script, b"").unwrap();
        let environment = StackingEnvironment::find(&data, &script).unwrap();
        let list = list_pipelines(
            Some(&environment),
            None,
            Some(&data.join("pipelines")),
            "fr",
        );
        assert_eq!(list.listed_by, ListedBy::Script);
        assert_eq!(
            list.pipelines[0].display_name.as_deref(),
            Some("Rendu naturel")
        );
        assert_eq!(list.ignored.len(), 1);
        let args = fs::read_to_string(args).unwrap();
        assert!(
            args.contains("list-pipelines --json --lang fr --pipelines-dir"),
            "{args}"
        );

        // No environment: the manifests.
        assert!(StackingEnvironment::find(&dir.path().join("none"), &script).is_none());
        assert_eq!(
            list_pipelines(None, None, None, "fr").listed_by,
            ListedBy::Manifests
        );
    }

    const PNG_HEAD: &[u8] = b"\x89PNG\r\n\x1a\n0000";
    const JPEG_HEAD: &[u8] = b"\xff\xd8\xff\xe00000";

    #[test]
    fn every_shipped_pipeline_has_its_preview_and_the_list_carries_it_as_a_data_url() {
        let list = list_pipelines(None, None, None, "fr");
        assert_eq!(list.pipelines.len(), 4);
        for pipeline in &list.pipelines {
            assert_eq!(
                pipeline.preview.as_deref(),
                Some("preview.jpg"),
                "{}",
                pipeline.id
            );
            assert_eq!(pipeline.preview_issue, None, "{}", pipeline.id);
            let data = pipeline.preview_data.as_deref().unwrap();
            assert!(
                data.starts_with("data:image/jpeg;base64,/9j/"),
                "{}",
                pipeline.id
            );
            // Advised 150 KiB; base64 is a third bigger.
            assert!(
                data.len() < 150 * 1024 * 4 / 3 + 64,
                "{} is {} bytes",
                pipeline.id,
                data.len()
            );
        }
        // The same listing from the folder of the repository.
        let stacking = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stacking/pipelines");
        let from_dir = list_pipelines(None, Some(&stacking), None, "fr");
        assert_eq!(
            from_dir.pipelines[0].preview_data,
            list.pipelines[0].preview_data
        );
    }

    #[test]
    fn a_preview_of_the_person_is_read_and_a_broken_one_is_ignored_not_the_pipeline() {
        let dir = TempDir::new("pipelines-preview");
        let write = |id: &str, preview: &str, content: Option<&[u8]>| {
            let folder = user_pipeline(
                dir.path(),
                id,
                &format!(r#", "preview": {preview}"#),
                "renders:natural_deep",
            );
            if let Some(content) = content {
                let name = preview.trim_matches('"');
                if !name.contains("..") && !name.starts_with('/') {
                    let path = folder.join(name);
                    fs::create_dir_all(path.parent().unwrap()).unwrap();
                    fs::write(path, content).unwrap();
                }
            }
        };
        write("with-jpeg", r#""look/preview.jpg""#, Some(JPEG_HEAD));
        write("with-png", r#""preview.png""#, Some(PNG_HEAD));
        write("lying", r#""preview.jpg""#, Some(b"GIF89a0000"));
        let big = vec![0xff; 600 * 1024];
        write("too-big", r#""preview.png""#, Some(&big));
        write("missing", r#""nothing.png""#, None);
        write("dotdot", r#""../escape.png""#, None);
        write("absolute", r#""/etc/hosts""#, None);
        write("not-text", "42", None);
        write("plain", "null", None);
        let outside = dir.path().join("elsewhere.png");
        fs::write(&outside, PNG_HEAD).unwrap();
        write("linked", r#""out.png""#, None);
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, dir.path().join("linked/out.png")).unwrap();

        let list = list_pipelines(None, None, Some(dir.path()), "fr");
        assert_eq!(
            list.ignored,
            vec![],
            "a bad preview never ignores the pipeline"
        );
        let by = |id: &str| list.get(id).unwrap();
        assert_eq!(by("with-jpeg").preview.as_deref(), Some("look/preview.jpg"));
        assert!(
            by("with-jpeg")
                .preview_data
                .as_deref()
                .unwrap()
                .starts_with("data:image/jpeg;base64,")
        );
        assert!(
            by("with-png")
                .preview_data
                .as_deref()
                .unwrap()
                .starts_with("data:image/png;base64,")
        );
        assert_eq!(by("plain").preview, None);
        assert_eq!(by("plain").preview_issue, None);
        for id in [
            "lying", "too-big", "missing", "dotdot", "absolute", "not-text",
        ] {
            assert_eq!(by(id).preview, None, "{id}");
            assert_eq!(by(id).preview_data, None, "{id}");
            assert!(
                by(id)
                    .preview_issue
                    .as_deref()
                    .unwrap()
                    .starts_with("preview:"),
                "{id}"
            );
        }
        #[cfg(unix)]
        assert!(
            by("linked")
                .preview_issue
                .as_deref()
                .unwrap()
                .contains("inside")
        );
    }
}
