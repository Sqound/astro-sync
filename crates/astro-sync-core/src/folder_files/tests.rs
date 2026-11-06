use super::*;
use crate::test_support::TempDir;

const GUIDE: &str = "# Guide\n\nThe structure.\n";

#[test]
fn the_ten_languages_have_a_readme_and_a_guide_name() {
    assert_eq!(LANGUAGES.len(), READMES.len());
    let mut seen = std::collections::BTreeSet::new();
    for language in LANGUAGES {
        let text = readme_text(language);
        assert!(text.starts_with("# Astro Sync"), "{language}");
        assert!(seen.insert(text), "{language} repeats another text");
        assert_eq!(
            guide_file_name(language),
            format!("GUIDE-ASTRO-SYNC.{language}.md")
        );
        // Every README names the files of the root and the rule's copy.
        for needle in [
            "README.md",
            "ASSISTANT-IA.md",
            "GUIDE-ASTRO-SYNC.",
            ".modifie-",
            "stacked/",
            "progression/",
            ".astro-sync-work/",
            "logs/",
            "lights/",
            "final/",
        ] {
            assert!(text.contains(needle), "{language} lacks {needle}");
        }
    }
    // Another language reads English, a regional tag its language.
    assert_eq!(readme_text("xx"), readme_text("en"));
    assert_eq!(readme_text("pt"), readme_text("pt-BR"));
    assert_eq!(readme_text("zh-TW"), readme_text("zh-Hant"));
    assert_eq!(guide_file_name("zh"), "GUIDE-ASTRO-SYNC.zh-Hans.md");
    assert_eq!(guide_file_name("xx"), "GUIDE-ASTRO-SYNC.en.md");
}

#[test]
fn the_readme_says_what_the_folder_contains() {
    // The English text: folders and their role, every .md file, what is
    // rewritten and never touched, nothing sent to an AI.
    let text = readme_text("en");
    for needle in [
        "## What is in it",
        "## The .md files at the root",
        "## What Astro Sync rewrites, and what it never touches",
        "## Nothing is sent to an AI",
        "`<name>.modifie-<date>.md`",
        "only when you ask",
        "does not contact any AI service",
    ] {
        assert!(text.contains(needle), "{needle}");
    }
}

#[test]
fn a_file_is_created_then_left_alone_when_it_is_today_s_text() {
    let root = TempDir::new("managed-created");
    let first = write_guide(root.path(), "fr", GUIDE, None).unwrap();
    assert_eq!(first.outcome, ManagedOutcome::Created);
    assert_eq!(first.name, "GUIDE-ASTRO-SYNC.fr.md");
    assert_eq!(first.bytes, GUIDE.len() as u64);
    assert_eq!(first.chars, GUIDE.chars().count());
    assert_eq!(
        fs::read_to_string(root.path().join("GUIDE-ASTRO-SYNC.fr.md")).unwrap(),
        GUIDE
    );
    let second = write_guide(root.path(), "fr", GUIDE, first.sha256.as_deref()).unwrap();
    assert_eq!(second.outcome, ManagedOutcome::Unchanged);
    assert_eq!(second.sha256, first.sha256);
}

#[test]
fn a_guide_the_application_has_since_changed_is_written_again() {
    let root = TempDir::new("managed-rewritten");
    let first = write_guide(root.path(), "en", "old text\n", None).unwrap();
    let next = write_guide(root.path(), "en", GUIDE, first.sha256.as_deref()).unwrap();
    assert_eq!(next.outcome, ManagedOutcome::Rewritten);
    assert_eq!(
        fs::read_to_string(root.path().join("GUIDE-ASTRO-SYNC.en.md")).unwrap(),
        GUIDE
    );
    assert_eq!(next.sha256, Some(sha256_hex(GUIDE.as_bytes())));
}

#[test]
fn a_guide_the_person_changed_is_never_overwritten() {
    let root = TempDir::new("managed-kept");
    let first = write_guide(root.path(), "fr", GUIDE, None).unwrap();
    let path = root.path().join("GUIDE-ASTRO-SYNC.fr.md");
    fs::write(&path, "my own notes\n").unwrap();
    // Even when the application's text moved on.
    let kept = write_guide(root.path(), "fr", "a newer text\n", first.sha256.as_deref()).unwrap();
    assert_eq!(kept.outcome, ManagedOutcome::KeptAsChanged);
    assert_eq!(fs::read_to_string(&path).unwrap(), "my own notes\n");
    // The digest to keep is still the one of the text Astro Sync wrote.
    assert_eq!(kept.sha256, first.sha256);
    // A file never recorded and different is the person's too.
    let other = write_guide(root.path(), "fr", GUIDE, None).unwrap();
    assert_eq!(other.outcome, ManagedOutcome::KeptAsChanged);
    assert_eq!(other.sha256, None);
    assert_eq!(fs::read_to_string(&path).unwrap(), "my own notes\n");
}

#[test]
fn replacing_keeps_the_changed_copy_first_and_deletes_nothing() {
    let root = TempDir::new("managed-replace");
    let path = root.path().join("GUIDE-ASTRO-SYNC.fr.md");
    fs::write(&path, "my own notes\n").unwrap();
    let (copy, file) = replace_guide(root.path(), "fr", GUIDE, "2026-10-17").unwrap();
    let copy = copy.expect("the changed file is kept");
    assert_eq!(
        copy.file_name().unwrap().to_string_lossy(),
        "GUIDE-ASTRO-SYNC.fr.modifie-2026-10-17.md"
    );
    assert_eq!(fs::read_to_string(&copy).unwrap(), "my own notes\n");
    assert_eq!(fs::read_to_string(&path).unwrap(), GUIDE);
    assert_eq!(file.outcome, ManagedOutcome::Created);
    // A second replacement the same day does not overwrite the first copy.
    fs::write(&path, "more notes\n").unwrap();
    let (again, _) = replace_guide(root.path(), "fr", GUIDE, "2026-10-17").unwrap();
    assert_eq!(
        again.unwrap().file_name().unwrap().to_string_lossy(),
        "GUIDE-ASTRO-SYNC.fr.modifie-2026-10-17-2.md"
    );
    assert_eq!(fs::read_to_string(&copy).unwrap(), "my own notes\n");
    // Nothing to keep when the file already holds today's text.
    let (none, file) = replace_guide(root.path(), "fr", GUIDE, "2026-10-17").unwrap();
    assert!(none.is_none());
    assert_eq!(file.outcome, ManagedOutcome::Unchanged);
}

#[test]
fn the_readme_follows_the_language_of_the_application() {
    let root = TempDir::new("readme-language");
    let french = write_readme(root.path(), "fr", None).unwrap();
    assert_eq!(french.outcome, ManagedOutcome::Created);
    // The language changed and the settings lost the digest: the French text
    // is still Astro Sync's, written over.
    let japanese = write_readme(root.path(), "ja", None).unwrap();
    assert_eq!(japanese.outcome, ManagedOutcome::Rewritten);
    assert_eq!(
        fs::read_to_string(root.path().join(README_FILE_NAME)).unwrap(),
        readme_text("ja")
    );
    let same = write_readme(root.path(), "ja", japanese.sha256.as_deref()).unwrap();
    assert_eq!(same.outcome, ManagedOutcome::Unchanged);
}

#[test]
fn a_readme_the_person_wrote_or_changed_is_kept_and_replaceable() {
    let root = TempDir::new("readme-kept");
    let path = root.path().join(README_FILE_NAME);
    fs::write(&path, "# My astro folder\n").unwrap();
    let kept = write_readme(root.path(), "en", None).unwrap();
    assert_eq!(kept.outcome, ManagedOutcome::KeptAsChanged);
    assert_eq!(fs::read_to_string(&path).unwrap(), "# My astro folder\n");
    let (copy, file) = replace_readme(root.path(), "en", "2026-10-17").unwrap();
    assert_eq!(
        copy.unwrap().file_name().unwrap().to_string_lossy(),
        "README.modifie-2026-10-17.md"
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), readme_text("en"));
    assert_eq!(file.sha256, Some(sha256_hex(readme_text("en").as_bytes())));
}

#[test]
fn a_write_that_cannot_happen_is_an_error_not_a_panic() {
    // The root is a file: nothing can be written under it.
    let root = TempDir::new("managed-failure");
    let blocker = root.path().join("not-a-folder");
    fs::write(&blocker, "x").unwrap();
    assert!(write_guide(&blocker, "fr", GUIDE, None).is_err());
}
