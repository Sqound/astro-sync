//! The person's trust in the scripts of the pipelines they added.
//!
//! A pipeline shipped with Astro Sync (reviewed by a pull request) runs
//! without a question. A pipeline the person added that brings its own
//! script asks, at its first use: "You added this pipeline yourself;
//! Astro Sync has not verified it. Do you trust it?" (in French). The answer
//! is recorded **for that script's SHA-256**, in Astro Sync's data
//! (`pipeline-trust.json`), never in the pipeline's folder:
//!
//! - yes: the digest is passed to the script at each run
//!   (`--trusted <id>=<sha256>`);
//! - no: the pipeline stays listed, disabled, never run;
//! - another digest later (the script changed): the question is asked again.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::pipelines::Pipeline;

/// The file in Astro Sync's data.
pub const TRUST_FILE: &str = "pipeline-trust.json";

/// One answer, for one digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrustAnswer {
    pub sha256: String,
    pub trusted: bool,
}

/// Every answer, by pipeline id.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustStore {
    #[serde(default)]
    pub pipelines: BTreeMap<String, TrustAnswer>,
}

/// Where a pipeline stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustState {
    /// Shipped with Astro Sync, or a definition that runs no script of its
    /// own: no question.
    NotNeeded,
    /// Never answered: asked at the first use.
    Ask,
    /// Trusted for this very script.
    Trusted,
    /// Refused for this very script: listed, disabled, never run.
    Refused,
    /// Answered for another script than the one there now: asked again.
    Changed,
}

/// Reads the store (missing or unreadable: no answer yet).
#[must_use]
pub fn load(data_dir: &Path) -> TrustStore {
    fs::read_to_string(data_dir.join(TRUST_FILE))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Records the person's answer for `id` and `sha256`.
pub fn record(data_dir: &Path, id: &str, sha256: &str, trusted: bool) -> std::io::Result<()> {
    let mut store = load(data_dir);
    store.pipelines.insert(
        id.to_string(),
        TrustAnswer {
            sha256: sha256.to_ascii_lowercase(),
            trusted,
        },
    );
    fs::create_dir_all(data_dir)?;
    fs::write(
        data_dir.join(TRUST_FILE),
        serde_json::to_string_pretty(&store).expect("the store serializes"),
    )
}

impl TrustStore {
    /// See [`TrustState`].
    #[must_use]
    pub fn state(&self, pipeline: &Pipeline) -> TrustState {
        if !pipeline.needs_trust() {
            return TrustState::NotNeeded;
        }
        let Some(sha256) = pipeline.script_sha256.as_deref() else {
            return TrustState::Ask;
        };
        match self.pipelines.get(&pipeline.id) {
            None => TrustState::Ask,
            Some(answer) if !answer.sha256.eq_ignore_ascii_case(sha256) => TrustState::Changed,
            Some(answer) if answer.trusted => TrustState::Trusted,
            Some(_) => TrustState::Refused,
        }
    }

    /// The digest to pass with `--trusted`, only when the person trusted
    /// this very script.
    #[must_use]
    pub fn trusted_sha256(&self, pipeline: &Pipeline) -> Option<String> {
        (self.state(pipeline) == TrustState::Trusted)
            .then(|| pipeline.script_sha256.clone())
            .flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipelines::read_manifests;
    use crate::test_support::TempDir;

    fn added(dir: &Path, script: &[u8]) -> Pipeline {
        let folder = dir.join("pipelines/mono-ha");
        fs::create_dir_all(&folder).unwrap();
        fs::write(
            folder.join("pipeline.json"),
            r#"{"id": "mono-ha", "version": "1", "name": {"en": "Warm Hα"}, "description": {"en": "Warm."},
               "filter": "h-alpha", "author": "Camille", "licence": "MIT", "entry": "script:render.py",
               "requires": ["siril"], "outputs": ["tif"]}"#,
        )
        .unwrap();
        fs::write(folder.join("render.py"), script).unwrap();
        read_manifests(None, Some(&dir.join("pipelines")), "fr")
            .get("mono-ha")
            .cloned()
            .unwrap()
    }

    #[test]
    fn a_script_added_by_the_person_is_asked_trusted_refused_and_asked_again_when_it_changes() {
        let dir = TempDir::new("pipeline-trust");
        let data = dir.path().join("data");
        let shipped = read_manifests(None, None, "fr")
            .get("raw")
            .cloned()
            .unwrap();
        let mine = added(
            dir.path(),
            b"def render(nebula, stars):\n    return nebula\n",
        );
        let digest = mine.script_sha256.clone().unwrap();

        let store = load(&data);
        assert_eq!(store.state(&shipped), TrustState::NotNeeded);
        assert_eq!(store.trusted_sha256(&shipped), None);
        assert_eq!(store.state(&mine), TrustState::Ask);
        assert_eq!(store.trusted_sha256(&mine), None);

        record(&data, "mono-ha", &digest.to_ascii_uppercase(), true).unwrap();
        let store = load(&data);
        assert_eq!(store.state(&mine), TrustState::Trusted);
        assert_eq!(store.trusted_sha256(&mine), Some(digest.clone()));
        // Recorded in Astro Sync's data, never in the pipeline's folder.
        assert!(data.join(TRUST_FILE).is_file());
        assert_eq!(
            fs::read_dir(dir.path().join("pipelines/mono-ha"))
                .unwrap()
                .count(),
            2
        );

        // The script changed: asked again, its digest no longer passed.
        let edited = added(
            dir.path(),
            b"def render(nebula, stars):\n    return stars\n",
        );
        assert_eq!(store.state(&edited), TrustState::Changed);
        assert_eq!(store.trusted_sha256(&edited), None);

        // Refused: disabled, never passed.
        record(
            &data,
            "mono-ha",
            edited.script_sha256.as_deref().unwrap(),
            false,
        )
        .unwrap();
        let store = load(&data);
        assert_eq!(store.state(&edited), TrustState::Refused);
        assert_eq!(store.trusted_sha256(&edited), None);
    }
}
