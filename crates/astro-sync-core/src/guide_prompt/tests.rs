use super::*;
use crate::assistant_prompt::LANGUAGES;
use crate::folder_files::guide_file_name;

fn input(language: &str, with_file: bool) -> GuidePromptInput {
    GuidePromptInput {
        root: PathBuf::from("/home/marie/Documents/Astro Sync"),
        guide_file: with_file.then(|| guide_file_name(language)),
    }
}

#[test]
fn the_french_prompt_with_the_path_is_the_boards_change() {
    let prompt = build_guide_prompt("fr", &input("fr", true));
    assert_eq!(
        prompt,
        [
            "Parle-moi en français.",
            "Tu vas répondre à mes questions sur Astro Sync, l’application qui range les fichiers d’une nuit d’astrophotographie et les envoie à Astro Portal.",
            "Mon dossier de travail\u{a0}: «\u{a0}/home/marie/Documents/Astro Sync\u{a0}».",
            "Lis d’abord le guide «\u{a0}/home/marie/Documents/Astro Sync/GUIDE-ASTRO-SYNC.fr.md\u{a0}»\u{a0}: il fait foi. Réponds d’après lui.",
            "Si ma question dépasse ce guide, dis-le, puis cherche quand même\u{a0}: tu peux lire le dossier de travail, ses fichiers et ses journaux. Sépare ce que tu as vérifié de ce que tu supposes, et ne modifie ni ne supprime rien sans me le demander.",
            "Si tu ne peux pas lire ce fichier, dis-le-moi et demande-moi de coller le guide dans la conversation.",
            "Ensuite, demande-moi ce que je veux savoir.",
            "",
        ]
        .join("\n")
    );
}

#[test]
fn the_french_prompt_without_a_path_waits_for_the_pasted_guide() {
    let prompt = build_guide_prompt("fr", &input("fr", false));
    assert!(prompt.starts_with("Parle-moi en français.\n"));
    assert!(prompt.contains("Je vais te coller le guide d’Astro Sync juste après ce message"));
    assert!(prompt.contains("Attends que je l’aie collé"));
    // No path, no fallback sentence (there is no file to fail to read).
    assert!(!prompt.contains("GUIDE-ASTRO-SYNC"));
    assert!(!prompt.contains("Si tu ne peux pas lire ce fichier"));
    // The working folder and the rule beyond the guide stay.
    assert!(prompt.contains("/home/marie/Documents/Astro Sync"));
    assert!(prompt.contains("puis cherche quand même"));
}

#[test]
fn the_prompt_no_longer_says_to_say_so_rather_than_invent() {
    for language in LANGUAGES {
        for with_file in [true, false] {
            let prompt = build_guide_prompt(language, &input(language, with_file));
            for old in [
                "plutôt que d’inventer",
                "rather than invent",
                "推測せずに",
                "statt zu erfinden",
                "en lugar de inventar",
                "invece di inventare",
                "em vez de inventar",
                "вместо того чтобы выдумывать",
                "而不是编造",
                "而不是編造",
            ] {
                assert!(!prompt.contains(old), "{language}: {old}");
            }
        }
    }
}

#[test]
fn the_ten_languages_open_with_their_language_line_and_differ() {
    let mut seen = std::collections::BTreeSet::new();
    for language in LANGUAGES {
        for with_file in [true, false] {
            let prompt = build_guide_prompt(language, &input(language, with_file));
            let first = prompt.lines().next().unwrap();
            assert_eq!(first, language_line(language), "{language}");
            assert!(prompt.ends_with('\n'));
            // The full path (or none), whichever the language.
            assert_eq!(
                prompt.contains(&format!(
                    "/home/marie/Documents/Astro Sync/{}",
                    guide_file_name(language)
                )),
                with_file,
                "{language} {with_file}"
            );
            // The working folder is always given.
            assert!(
                prompt.contains("/home/marie/Documents/Astro Sync"),
                "{language}"
            );
            // Every sentence is translated: no English left in another
            // language's prompt (the product names aside).
            if language != "en" {
                for english in ["First read", "If my question", "Then ask me"] {
                    assert!(!prompt.contains(english), "{language}: {english}");
                }
            }
            if with_file {
                assert!(seen.insert(prompt), "{language} repeats another prompt");
            }
        }
    }
}

#[test]
fn every_variant_keeps_the_three_promises_to_the_assistant() {
    // « you may read the working folder, its files and its logs », « separate
    // what you checked from what you suppose », « change or delete nothing
    // without asking me »: three sentences in the same field of every
    // language, found by their distinct beginnings.
    for language in LANGUAGES {
        let words = phrases(language);
        assert!(!words.beyond.contains("{root}"));
        assert!(
            words.beyond.chars().count() > 80,
            "{language}: the sentence is too short to hold the three promises"
        );
    }
    let english = build_guide_prompt("en", &input("en", true));
    assert!(english.contains("say so, then look anyway"));
    assert!(english.contains("you may read the working folder, its files and its logs"));
    assert!(english.contains("Separate what you checked from what you suppose"));
    assert!(english.contains("change or delete nothing without asking me"));
}

#[test]
fn a_windows_root_keeps_its_backslashes() {
    let prompt = build_guide_prompt(
        "en",
        &GuidePromptInput {
            root: PathBuf::from("D:\\Marie\\Documents\\Astro Sync"),
            guide_file: Some("GUIDE-ASTRO-SYNC.en.md".to_string()),
        },
    );
    assert!(prompt.contains("“D:\\Marie\\Documents\\Astro Sync\\GUIDE-ASTRO-SYNC.en.md”"));
    assert!(prompt.contains("My working folder: “D:\\Marie\\Documents\\Astro Sync”."));
}

#[test]
fn an_unknown_language_reads_english() {
    assert_eq!(
        build_guide_prompt("xx", &input("en", true)),
        build_guide_prompt("en", &input("en", true))
    );
}

/// `prompts.snapshot.json` holds the prompts of a sample folder, in the ten
/// languages, with and without a path. It is the core's own output
/// (`UPDATE_SNAPSHOTS=1 cargo test -p astro-sync-core --lib guide_prompt`
/// writes it).
#[test]
fn the_snapshot_holds_the_cores_prompts() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/guide_prompt/prompts.snapshot.json");
    let mut prompts = serde_json::Map::new();
    for language in LANGUAGES {
        let root = PathBuf::from("/home/marie/Documents/Astro Sync");
        let with = build_guide_prompt(
            language,
            &GuidePromptInput {
                root: root.clone(),
                guide_file: Some(guide_file_name(language)),
            },
        );
        let without = build_guide_prompt(
            language,
            &GuidePromptInput {
                root,
                guide_file: None,
            },
        );
        prompts.insert(
            (*language).to_string(),
            serde_json::json!({ "path": with, "nopath": without }),
        );
    }
    let actual = serde_json::to_string_pretty(&serde_json::Value::Object(prompts)).unwrap() + "\n";
    if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
        std::fs::write(&path, &actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        expected == actual,
        "prompts.snapshot.json differs (UPDATE_SNAPSHOTS=1 to rewrite it)"
    );
}
