//! The `.md` files Astro Sync writes at the root of the working folder
//! besides `ASSISTANT-IA.md`:
//!
//! - **`README.md`**: a short explanation of the folder (its folders and
//!   their role, every `.md` file, what Astro Sync rewrites and never
//!   touches, that nothing is sent to an AI), **in the application's
//!   language** (one text per language of the app, `folder_files/readme/`).
//! - **`GUIDE-ASTRO-SYNC.<language>.md`**: the text of the Guide page in
//!   Markdown, written when the person asks an AI a question about Astro
//!   Sync. The window renders it from its dictionaries and hands the text
//!   over: this module only writes it, one file per language asked for.
//!
//! Both follow **the rule of `ASSISTANT-IA.md`** ([`crate::assistant_prompt`]):
//! a file is written again only while it still holds bytes Astro Sync wrote
//! (its SHA-256 is the digest kept in the local settings, or, for the
//! README, the text of one of the ten languages: a change of the app's
//! language rewrites it); a file the person changed is **never
//! overwritten** ([`ManagedOutcome::KeptAsChanged`]), and
//! [`replace_managed_file`] renames it `<name>.modifie-<date>.md` before
//! writing Astro Sync's version. Nothing is ever deleted.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::assistant_prompt::LANGUAGES;

/// The README, at the root of the working folder.
pub const README_FILE_NAME: &str = "README.md";
/// What every guide's name starts with: `GUIDE-ASTRO-SYNC.<language>.md`.
pub const GUIDE_FILE_STEM: &str = "GUIDE-ASTRO-SYNC";

/// The README in each of the ten languages of the app, in [`LANGUAGES`]'
/// order.
const READMES: [&str; 10] = [
    include_str!("folder_files/readme/fr.md"),
    include_str!("folder_files/readme/en.md"),
    include_str!("folder_files/readme/ja.md"),
    include_str!("folder_files/readme/de.md"),
    include_str!("folder_files/readme/es.md"),
    include_str!("folder_files/readme/it.md"),
    include_str!("folder_files/readme/pt-BR.md"),
    include_str!("folder_files/readme/ru.md"),
    include_str!("folder_files/readme/zh-Hans.md"),
    include_str!("folder_files/readme/zh-Hant.md"),
];

/// The tag of the language of the app among [`LANGUAGES`] (`pt` reads
/// `pt-BR`, `zh-TW` reads `zh-Hant`…); any other language reads English,
/// like the prompt.
#[must_use]
pub fn normalise_language(language: &str) -> &'static str {
    let tag = match language {
        "pt" => "pt-BR",
        "zh" | "zh-CN" => "zh-Hans",
        "zh-TW" | "zh-HK" => "zh-Hant",
        other => other,
    };
    LANGUAGES
        .iter()
        .copied()
        .find(|candidate| *candidate == tag)
        .unwrap_or("en")
}

/// The file name of the guide in `language`:
/// `GUIDE-ASTRO-SYNC.<language>.md`.
#[must_use]
pub fn guide_file_name(language: &str) -> String {
    format!("{GUIDE_FILE_STEM}.{}.md", normalise_language(language))
}

/// The text of the README in `language`.
#[must_use]
pub fn readme_text(language: &str) -> &'static str {
    let tag = normalise_language(language);
    let index = LANGUAGES.iter().position(|item| *item == tag).unwrap_or(1);
    READMES[index]
}

/// The SHA-256 of a text, lower-case hexadecimal.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// The digests of the README in the ten languages: any of them found in the
/// file is Astro Sync's text.
fn readme_digests() -> Vec<String> {
    READMES
        .iter()
        .map(|text| sha256_hex(text.as_bytes()))
        .collect()
}

/// What became of a file Astro Sync writes under the rule of the module.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedOutcome {
    /// It was not there: written.
    Created,
    /// It held an earlier text Astro Sync wrote: written again.
    Rewritten,
    /// It holds today's text.
    Unchanged,
    /// The person changed it: left as it is.
    KeptAsChanged,
}

/// One file of the module, after it was made ready.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedFile {
    /// The file's name at the root (`GUIDE-ASTRO-SYNC.fr.md`).
    pub name: String,
    pub path: PathBuf,
    pub outcome: ManagedOutcome,
    /// The digest to keep in the settings: of the text Astro Sync wrote, or
    /// of the one it kept before when the person changed the file.
    pub sha256: Option<String>,
    /// The size of the file on disk, in bytes.
    pub bytes: u64,
    /// The number of characters of Astro Sync's text.
    pub chars: usize,
}

/// Writes `name` at `root` with `text`, under the rule of the module.
/// `recorded` is the digest of the text Astro Sync last wrote there;
/// `also_ours` lists other digests that are Astro Sync's (the README of
/// another language).
pub fn write_managed_file(
    root: &Path,
    name: &str,
    text: &str,
    recorded: Option<&str>,
    also_ours: &[String],
) -> std::io::Result<ManagedFile> {
    let path = root.join(name);
    let ours = sha256_hex(text.as_bytes());
    let outcome = match fs::read(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?;
            file.write_all(text.as_bytes())?;
            ManagedOutcome::Created
        }
        Err(error) => return Err(error),
        Ok(bytes) => {
            let theirs = sha256_hex(&bytes);
            if theirs == ours {
                ManagedOutcome::Unchanged
            } else if recorded.is_some_and(|r| r.eq_ignore_ascii_case(&theirs))
                || also_ours.contains(&theirs)
            {
                fs::write(&path, text)?;
                ManagedOutcome::Rewritten
            } else {
                ManagedOutcome::KeptAsChanged
            }
        }
    };
    let sha256 = match outcome {
        ManagedOutcome::KeptAsChanged => recorded.map(str::to_string),
        _ => Some(ours),
    };
    let bytes = fs::metadata(&path).map_or(0, |meta| meta.len());
    Ok(ManagedFile {
        name: name.to_string(),
        path,
        outcome,
        sha256,
        bytes,
        chars: text.chars().count(),
    })
}

/// The README in `language`, under the rule of the module. A README of
/// another language that Astro Sync wrote is Astro Sync's too.
pub fn write_readme(
    root: &Path,
    language: &str,
    recorded: Option<&str>,
) -> std::io::Result<ManagedFile> {
    write_managed_file(
        root,
        README_FILE_NAME,
        readme_text(language),
        recorded,
        &readme_digests(),
    )
}

/// The guide in `language` (`text` is the Guide page in Markdown, rendered
/// by the window), under the rule of the module.
pub fn write_guide(
    root: &Path,
    language: &str,
    text: &str,
    recorded: Option<&str>,
) -> std::io::Result<ManagedFile> {
    write_managed_file(root, &guide_file_name(language), text, recorded, &[])
}

/// "Replace with Astro Sync’s version": the file the person changed
/// is first **renamed** `<stem>.modifie-<date>.md` in the same folder (`-2`,
/// `-3`… when that name is taken), then `text` is written. Nothing is
/// deleted. Returns the copy's path (none when there was nothing to keep: no
/// file, or already `text`).
pub fn replace_managed_file(
    root: &Path,
    name: &str,
    text: &str,
    date: &str,
) -> std::io::Result<(Option<PathBuf>, ManagedFile)> {
    let path = root.join(name);
    let mut copy = None;
    if let Ok(bytes) = fs::read(&path)
        && sha256_hex(&bytes) != sha256_hex(text.as_bytes())
    {
        let stem = name.trim_end_matches(".md");
        let mut candidate = root.join(format!("{stem}.modifie-{date}.md"));
        let mut n = 2;
        while candidate.exists() {
            candidate = root.join(format!("{stem}.modifie-{date}-{n}.md"));
            n += 1;
        }
        fs::rename(&path, &candidate)?;
        copy = Some(candidate);
    }
    let file = write_managed_file(root, name, text, None, &[])?;
    Ok((copy, file))
}

/// [`replace_managed_file`] for the README in `language`.
pub fn replace_readme(
    root: &Path,
    language: &str,
    date: &str,
) -> std::io::Result<(Option<PathBuf>, ManagedFile)> {
    replace_managed_file(root, README_FILE_NAME, readme_text(language), date)
}

/// [`replace_managed_file`] for the guide in `language`.
pub fn replace_guide(
    root: &Path,
    language: &str,
    text: &str,
    date: &str,
) -> std::io::Result<(Option<PathBuf>, ManagedFile)> {
    replace_managed_file(root, &guide_file_name(language), text, date)
}

#[cfg(test)]
mod tests;
