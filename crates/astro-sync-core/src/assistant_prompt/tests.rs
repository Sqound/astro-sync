use super::flow::{Action, FLOW_JSON, FLOW_SCHEMA, Flow, LINKS_RULE, PLACEMENT_RULE};
use super::*;
use crate::test_support::TempDir;
use serde_json::Value;

/* ------------------------------ Snapshots -------------------------------- */

/// Compares `actual` with `snapshots/<name>`; `UPDATE_SNAPSHOTS=1` writes
/// it instead (then read the diff before committing).
fn snapshot(name: &str, actual: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/assistant_prompt/snapshots")
        .join(name);
    if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
        fs::write(&path, actual).unwrap();
        return;
    }
    let expected = fs::read_to_string(&path).unwrap_or_default();
    assert!(
        expected == actual,
        "snapshot {name} differs (UPDATE_SNAPSHOTS=1 to rewrite it):\n{actual}"
    );
}

/* ---------------------------- ASSISTANT-IA.md ---------------------------- */

/// The text first written to the file, word for word.
const EARLIER_TEXT: &str = concat!(
    "**Astro Sync — how this folder is organised.** This is the working folder of Astro Sync, an application that sends astrophotography nights to an Astro Portal space. It holds one folder per night, named \"date · plan or place · short identifier\". Inside each night: `logs/` for the logs (NINA, PHD2, ASIAIR); `calibration/darks/`, `calibration/flats/`, `calibration/dark-flats/` and `calibration/biases/` for the calibration frames; one folder per target (\"M31\", \"NGC 7000\"…) with `lights/` for that target's raw FITS frames and `final/` for the processed final image (TIFF, JPEG or PNG). Folder names stay in English. A frame's type is read from its FITS header (`IMAGETYP`: LIGHT, DARK, FLAT, DARKFLAT, BIAS), its target from `OBJECT`, its filter from `FILTER`. Astro Sync never sends the FITS frames or the raw logs: they stay on this computer. **Rules for you, the assistant**: move files, do not copy them; never delete, rename or modify a FITS file; do not touch files whose name starts with a dot (`.astro-sync-…`); propose the list of moves before doing them and wait for the person's agreement; a file whose type is doubtful stays where it is. The person writes to you in their own language: answer in that language; this file stays in English.",
    "\n"
);

#[test]
fn the_file_is_the_rendering_of_the_json() {
    let rendered = Flow::committed().render();
    snapshot("ASSISTANT-IA.md.txt", &rendered);
    assert_eq!(ASSISTANT_FILE_TEXT, rendered);
    assert!(ASSISTANT_FILE_TEXT.starts_with("# Astro Sync — how this folder is organised\n"));
}

#[test]
fn the_multi_night_file_adds_its_section_after_the_rules() {
    let rendered = Flow::committed().render_for(true);
    snapshot("ASSISTANT-IA.multi-night.md.txt", &rendered);
    assert_eq!(ASSISTANT_FILE_TEXT_MULTI_NIGHT, rendered);
    assert!(rendered.contains("\n## Multi-night plans\n"));
    assert!(!ASSISTANT_FILE_TEXT.contains("## Multi-night plans"));
    for words in [
        "one folder per night",
        "the same target folder name",
        "_integration.fit",
        "never put the frames of two nights",
    ] {
        assert!(rendered.contains(words), "{words}");
    }
    // The base text is the multi-night one without its section.
    let rules = rendered.find("\n## Multi-night plans").unwrap();
    let end = rules + 1 + rendered[rules + 1..].find("\n## ").unwrap();
    assert_eq!(
        format!("{}{}", &rendered[..rules], &rendered[end..]),
        ASSISTANT_FILE_TEXT
    );
}

#[test]
fn the_file_follows_the_multi_night_plans_as_they_come_and_go() {
    let root = TempDir::new("assistant-file-multi-night");
    let path = root.path().join(ASSISTANT_FILE_NAME);
    let plain = write_assistant_file_for(root.path(), None, false).unwrap();
    assert_eq!(plain.outcome, AssistantFileOutcome::Created);
    let opened = write_assistant_file_for(root.path(), plain.sha256.as_deref(), true).unwrap();
    assert_eq!(opened.outcome, AssistantFileOutcome::Rewritten);
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        ASSISTANT_FILE_TEXT_MULTI_NIGHT
    );
    // The right ends, the settings lost the digest: the other text is still
    // Astro Sync's, written over.
    let closed = write_assistant_file_for(root.path(), None, false).unwrap();
    assert_eq!(closed.outcome, AssistantFileOutcome::Rewritten);
    assert_eq!(fs::read_to_string(&path).unwrap(), ASSISTANT_FILE_TEXT);
    // A file the person changed stays as it is.
    fs::write(&path, "My notes.\n").unwrap();
    let kept = write_assistant_file_for(root.path(), closed.sha256.as_deref(), true).unwrap();
    assert_eq!(kept.outcome, AssistantFileOutcome::KeptAsChanged);
    // "Replace with Astro Sync’s version": the multi-night text.
    let (copy, replaced) = replace_assistant_file_for(root.path(), "2026-10-12", true).unwrap();
    assert!(copy.is_some());
    assert_eq!(replaced.outcome, AssistantFileOutcome::Created);
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        ASSISTANT_FILE_TEXT_MULTI_NIGHT
    );
}

#[test]
fn the_earlier_text_is_known_and_is_not_the_current_one() {
    let earlier = format!("{:x}", Sha256::digest(EARLIER_TEXT.as_bytes()));
    assert_eq!(EARLIER_ASSISTANT_FILE_SHA256[0], earlier.as_str());
    assert!(!EARLIER_ASSISTANT_FILE_SHA256.contains(&assistant_file_sha256().as_str()));
}

#[test]
fn the_file_is_written_once_and_rewritten_only_while_it_is_astro_syncs() {
    let root = TempDir::new("assistant-file");
    let created = write_assistant_file(root.path(), None).unwrap();
    assert_eq!(created.outcome, AssistantFileOutcome::Created);
    assert_eq!(
        created.sha256.as_deref(),
        Some(assistant_file_sha256().as_str())
    );
    assert_eq!(
        fs::read_to_string(&created.path).unwrap(),
        ASSISTANT_FILE_TEXT
    );

    let again = write_assistant_file(root.path(), created.sha256.as_deref()).unwrap();
    assert_eq!(again.outcome, AssistantFileOutcome::Unchanged);

    // An earlier text of Astro Sync, unmodified: written again.
    let older = "An older text Astro Sync wrote.\n";
    fs::write(&created.path, older).unwrap();
    let older_sha = format!("{:x}", Sha256::digest(older.as_bytes()));
    let rewritten = write_assistant_file(root.path(), Some(&older_sha)).unwrap();
    assert_eq!(rewritten.outcome, AssistantFileOutcome::Rewritten);
    assert_eq!(
        fs::read_to_string(&created.path).unwrap(),
        ASSISTANT_FILE_TEXT
    );

    // The person changed it: left as it is.
    fs::write(&created.path, "My own notes.\n").unwrap();
    let kept = write_assistant_file(root.path(), rewritten.sha256.as_deref()).unwrap();
    assert_eq!(kept.outcome, AssistantFileOutcome::KeptAsChanged);
    assert_eq!(
        fs::read_to_string(&created.path).unwrap(),
        "My own notes.\n"
    );
    assert_eq!(kept.sha256, rewritten.sha256);
}

#[test]
fn the_first_earlier_file_is_updated_even_without_its_recorded_digest() {
    // The settings recorded the digest of the first text: the ordinary path.
    let root = TempDir::new("assistant-file-earlier");
    let path = root.path().join(ASSISTANT_FILE_NAME);
    fs::write(&path, EARLIER_TEXT).unwrap();
    let file = write_assistant_file(root.path(), Some(EARLIER_ASSISTANT_FILE_SHA256[0])).unwrap();
    assert_eq!(file.outcome, AssistantFileOutcome::Rewritten);
    assert_eq!(fs::read_to_string(&path).unwrap(), ASSISTANT_FILE_TEXT);
    assert_eq!(
        file.sha256.as_deref(),
        Some(assistant_file_sha256().as_str())
    );

    // The settings lost it (or never had it): known all the same.
    fs::write(&path, EARLIER_TEXT).unwrap();
    let file = write_assistant_file(root.path(), None).unwrap();
    assert_eq!(file.outcome, AssistantFileOutcome::Rewritten);
    assert_eq!(fs::read_to_string(&path).unwrap(), ASSISTANT_FILE_TEXT);

    // The first text with one word changed by the person: kept.
    let changed = EARLIER_TEXT.replace("NGC 7000", "NGC 7023");
    fs::write(&path, &changed).unwrap();
    let file = write_assistant_file(root.path(), Some(EARLIER_ASSISTANT_FILE_SHA256[0])).unwrap();
    assert_eq!(file.outcome, AssistantFileOutcome::KeptAsChanged);
    assert_eq!(fs::read_to_string(&path).unwrap(), changed);
}

#[test]
fn the_root_is_made_with_its_assistant_file() {
    let parent = TempDir::new("assistant-root");
    let root = parent.path().join("Astro Sync");
    let (created, file) = prepare_root(&root, None).unwrap();
    assert!(created);
    assert_eq!(file.outcome, AssistantFileOutcome::Created);
    let (created, file) = prepare_root(&root, file.sha256.as_deref()).unwrap();
    assert!(!created);
    assert_eq!(file.outcome, AssistantFileOutcome::Unchanged);
    let names: Vec<_> = fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["ASSISTANT-IA.md"]);
}

/* ------------------------- The JSON and its schema ----------------------- */

/// The subset of JSON Schema 2020-12 `assistant-file.schema.json` uses
/// (no new dependency): `$ref` to `#/$defs/…`, `type`, `required`,
/// `properties`, `additionalProperties`, `items`, `enum`, `minLength`,
/// `minItems`, `minProperties`, `minimum`. A keyword outside it fails, so
/// the schema cannot silently grow beyond what is checked.
fn validate(root: &Value, schema: &Value, value: &Value, at: &str, problems: &mut Vec<String>) {
    let Value::Object(rules) = schema else {
        problems.push(format!("{at}: schema is not an object"));
        return;
    };
    if let Some(Value::String(reference)) = rules.get("$ref") {
        let name = reference.strip_prefix("#/$defs/").unwrap();
        validate(root, &root["$defs"][name], value, at, problems);
    }
    for (keyword, rule) in rules {
        match keyword.as_str() {
            "$ref" | "$schema" | "$id" | "$defs" | "title" | "description" => {}
            "type" => {
                let ok = match rule.as_str().unwrap() {
                    "object" => value.is_object(),
                    "array" => value.is_array(),
                    "string" => value.is_string(),
                    "integer" => value.is_u64() || value.is_i64(),
                    "boolean" => value.is_boolean(),
                    other => panic!("type {other}"),
                };
                if !ok {
                    problems.push(format!("{at}: not a {rule}"));
                }
            }
            "enum" => {
                if !rule.as_array().unwrap().contains(value) {
                    problems.push(format!("{at}: {value} not in {rule}"));
                }
            }
            "minLength" => {
                if value
                    .as_str()
                    .is_some_and(|s| (s.chars().count() as u64) < rule.as_u64().unwrap())
                {
                    problems.push(format!("{at}: too short"));
                }
            }
            "minItems" => {
                if value
                    .as_array()
                    .is_some_and(|a| (a.len() as u64) < rule.as_u64().unwrap())
                {
                    problems.push(format!("{at}: too few items"));
                }
            }
            "minProperties" => {
                if value
                    .as_object()
                    .is_some_and(|o| (o.len() as u64) < rule.as_u64().unwrap())
                {
                    problems.push(format!("{at}: too few properties"));
                }
            }
            "minimum" => {
                if value.as_f64().is_some_and(|n| n < rule.as_f64().unwrap()) {
                    problems.push(format!("{at}: below the minimum"));
                }
            }
            "required" => {
                if let Some(object) = value.as_object() {
                    for name in rule.as_array().unwrap() {
                        if !object.contains_key(name.as_str().unwrap()) {
                            problems.push(format!("{at}: `{name}` is missing"));
                        }
                    }
                }
            }
            "properties" | "additionalProperties" => {
                let Some(object) = value.as_object() else {
                    continue;
                };
                if keyword == "additionalProperties" {
                    let known = rules.get("properties").and_then(Value::as_object);
                    for (name, item) in object {
                        if known.is_some_and(|k| k.contains_key(name)) {
                            continue;
                        }
                        match rule {
                            Value::Bool(false) => {
                                problems.push(format!("{at}: unknown property `{name}`"));
                            }
                            Value::Bool(true) => {}
                            _ => validate(root, rule, item, &format!("{at}.{name}"), problems),
                        }
                    }
                } else {
                    for (name, item_schema) in rule.as_object().unwrap() {
                        if let Some(item) = object.get(name) {
                            validate(root, item_schema, item, &format!("{at}.{name}"), problems);
                        }
                    }
                }
            }
            "items" => {
                if let Some(array) = value.as_array() {
                    for (index, item) in array.iter().enumerate() {
                        validate(root, rule, item, &format!("{at}[{index}]"), problems);
                    }
                }
            }
            other => problems.push(format!("{at}: keyword `{other}` is not checked")),
        }
    }
}

fn schema_problems(json: &Value) -> Vec<String> {
    let schema: Value = serde_json::from_str(FLOW_SCHEMA).unwrap();
    let mut problems = Vec::new();
    validate(&schema, &schema, json, "$", &mut problems);
    problems
}

#[test]
fn the_json_follows_its_schema() {
    let json: Value = serde_json::from_str(FLOW_JSON).unwrap();
    assert_eq!(schema_problems(&json), Vec::<String>::new());
    assert_eq!(json["$schema"], "./assistant-file.schema.json");
}

#[test]
fn the_schema_refuses_what_the_json_must_not_hold() {
    let json: Value = serde_json::from_str(FLOW_JSON).unwrap();
    let mut unknown_action = json.clone();
    unknown_action["steps"][1]["action"] = "delete".into();
    assert!(!schema_problems(&unknown_action).is_empty());
    let mut unknown_field = json.clone();
    unknown_field["steps"][0]["when"] = "always".into();
    assert!(!schema_problems(&unknown_field).is_empty());
    let mut no_version = json.clone();
    no_version.as_object_mut().unwrap().remove("version");
    assert!(!schema_problems(&no_version).is_empty());
    let mut empty_text = json;
    empty_text["rules"][0]["text"] = "".into();
    assert!(!schema_problems(&empty_text).is_empty());
}

#[test]
fn the_json_is_read_with_its_cross_references_checked() {
    let flow = Flow::committed();
    assert_eq!(flow.version, 2);
    assert_eq!(flow.check(), Ok(()));

    let mut broken: Flow = serde_json::from_str(FLOW_JSON).unwrap();
    broken.steps[0].options[0].next = "nowhere".into();
    broken.steps[1].files = vec!["comets".into()];
    broken.steps[2].options[0].answer = "maybe".into();
    broken.actions.search = "Search for {things}.".into();
    let problems = broken.check().unwrap_err();
    assert!(
        problems
            .iter()
            .any(|p| p.contains("unknown step `nowhere`")),
        "{problems:?}"
    );
    assert!(
        problems.iter().any(|p| p.contains("unknown kind `comets`")),
        "{problems:?}"
    );
    assert!(
        problems
            .iter()
            .any(|p| p.contains("unknown answer `maybe`")),
        "{problems:?}"
    );
    assert!(
        problems.iter().any(|p| p.contains("{things}")),
        "{problems:?}"
    );
    assert!(
        problems.iter().any(|p| p.contains("`{files}` is missing")),
        "{problems:?}"
    );

    let mut refused: Value = serde_json::from_str(FLOW_JSON).unwrap();
    refused["steps"][0]["when"] = "always".into();
    assert!(serde_json::from_value::<Flow>(refused).is_err());
}

#[test]
fn every_step_is_reachable_and_the_procedure_has_no_loop() {
    // Depth-first from the first step: a step met again on the current
    // path is a loop; every path ends at a `summarize` step.
    fn walk<'a>(flow: &'a Flow, id: &'a str, path: &mut Vec<&'a str>, seen: &mut Vec<&'a str>) {
        assert!(!path.contains(&id), "loop: {path:?} -> {id}");
        if !seen.contains(&id) {
            seen.push(id);
        }
        let (_, step) = flow.step(id).unwrap();
        let next = Flow::successors(step);
        if next.is_empty() {
            assert_eq!(
                step.action,
                Some(Action::Summarize),
                "{id} ends the procedure"
            );
        }
        path.push(id);
        for next in next {
            walk(flow, next, path, seen);
        }
        path.pop();
    }
    let flow = Flow::committed();
    let mut seen = Vec::new();
    walk(&flow, &flow.steps[0].id, &mut Vec::new(), &mut seen);
    let unreachable: Vec<&str> = flow
        .steps
        .iter()
        .map(|s| s.id.as_str())
        .filter(|id| !seen.contains(id))
        .collect();
    assert_eq!(unreachable, Vec::<&str>::new());
}

#[test]
fn the_placement_rule_is_one_entry_with_the_chosen_options() {
    let flow = Flow::committed();
    let rules: Vec<_> = flow
        .rules
        .iter()
        .filter(|r| r.id == PLACEMENT_RULE)
        .collect();
    assert_eq!(rules.len(), 1);
    let text = &rules[0].text;
    let link = text.find("(1) a symbolic link").unwrap();
    let copy = text.find("(2) a copy").unwrap();
    let mv = text.find("(3) a move").unwrap();
    assert!(link < copy && copy < mv, "{text}");
    assert!(text.contains("macOS and Linux only"), "{text}");
    assert!(
        text.contains("On Windows, offer only (2) a copy and (3) a move"),
        "{text}"
    );
    assert!(text.contains("the same choice for everything"), "{text}");
    // No other text says how to place a file.
    let links = flow.rules.iter().find(|r| r.id == LINKS_RULE).unwrap();
    let others = ASSISTANT_FILE_TEXT
        .replace(text.as_str(), "")
        .replace(links.text.as_str(), "");
    assert!(!others.contains("symbolic"), "{others}");
    assert!(!others.contains("do not copy"), "{others}");
}

#[test]
fn the_links_rule_is_one_entry_and_every_search_step_cites_it() {
    let flow = Flow::committed();
    let rules: Vec<_> = flow.rules.iter().filter(|r| r.id == LINKS_RULE).collect();
    assert_eq!(rules.len(), 1);
    let rule = rules[0];
    for needle in [
        "follow symbolic links",
        "`find -L`",
        "Finder aliases",
        "`.lnk` shortcuts and junctions",
        "ask whether to follow them",
        "mark those that come through a link, with the link's target",
        "point to the real file, never to a link",
        "Do not loop",
        "Never modify or delete a link",
    ] {
        assert!(rule.text.contains(needle), "{needle}");
    }
    let cite = format!("by the rule \"{}\"", rule.title);
    let mut searches = 0;
    for step in flow
        .steps
        .iter()
        .filter(|s| s.action == Some(Action::Search))
    {
        searches += 1;
        assert!(flow.render_step(step).contains(&cite), "{}", step.id);
    }
    assert!(searches >= 5);
    // The rule's text is written once.
    assert_eq!(ASSISTANT_FILE_TEXT.matches(rule.text.as_str()).count(), 1);
}

#[test]
fn the_third_earlier_text_is_known() {
    assert_eq!(
        EARLIER_ASSISTANT_FILE_SHA256[2],
        "79f01cb7a7aa38ea5d920dbc4ea9f766e9df0cb5e3f00776d2610d407691c17c"
    );
}

#[test]
fn the_fourth_earlier_text_is_known_and_the_file_names_the_hidden_work_folder() {
    assert_eq!(
        EARLIER_ASSISTANT_FILE_SHA256[3],
        "144610e34a59fd7bf6c8c07c2569c7864f0ad5ae1920e29232bb8aee88eef878"
    );
    assert!(
        ASSISTANT_FILE_TEXT
            .contains("- `.astro-sync-work/`: a hidden folder, at the root of a night's folder")
    );
    assert!(ASSISTANT_FILE_TEXT.contains("Put nothing in it and do not touch it"));
}

#[test]
fn the_text_before_the_links_rule_is_known() {
    // The text before the links rule: its digest is the second earlier digest.
    assert_eq!(
        EARLIER_ASSISTANT_FILE_SHA256[1],
        "e755edf53afc1c4bcdc36102450c359e7f232526a26af914a81ecf1b2a633d78"
    );
}

/// The steps one conversation goes through: the person's answers, in order.
fn trace(answers: &[&str]) -> String {
    let flow = Flow::committed();
    let mut id = flow.steps[0].id.as_str();
    let mut answers = answers.iter();
    let mut out = String::new();
    loop {
        let (index, step) = flow.step(id).unwrap();
        out.push_str(&format!("Step {}. {}\n", index + 1, flow.render_step(step)));
        if !step.options.is_empty() {
            let answer = answers.next().expect("an answer for this question");
            let choice = step
                .options
                .iter()
                .find(|c| c.answer == *answer)
                .unwrap_or_else(|| panic!("`{answer}` at `{id}`"));
            out.push_str(&format!("> {}\n", flow.answers[&choice.answer]));
            id = &choice.next;
        } else if let Some(next) = &step.next {
            id = next;
        } else {
            break;
        }
    }
    assert!(answers.next().is_none(), "answers left over");
    out
}

#[test]
fn the_fits_and_a_processing_go_through_every_kind() {
    let path = trace(&[
        "separate", "fits", "search", "path", "search", "none", "path", "yes", "path",
    ]);
    snapshot("path-fits-and-processed.txt", &path);
    for kind in [
        "light frames",
        "darks",
        "flats",
        "dark-flats",
        "logs of the night",
        "final image",
    ] {
        assert!(path.contains(kind), "{kind}");
    }
}

#[test]
fn a_final_image_alone_is_asked_for_directly() {
    let path = trace(&["separate", "final_only", "none", "search"]);
    snapshot("path-final-only.txt", &path);
    assert!(!path.contains("Search for the darks"));
    assert!(!path.contains("Search for the light frames"));
    assert!(path.contains("`<target>/final/`"));
}

#[test]
fn everything_in_one_folder_is_searched_there_once() {
    let path = trace(&["one_folder", "fits"]);
    snapshot("path-one-folder.txt", &path);
    assert!(path.contains("full path of the folder that holds this night's files"));
    assert_eq!(path.matches("Search for").count(), 1);
    let final_only = trace(&["one_folder", "final_only"]);
    assert!(final_only.contains("`<target>/final/`"));
    assert!(path.contains("`logs/`") && final_only.contains("`logs/`"));
}

/* ------------------------------- The prompt ------------------------------ */

fn night() -> PromptInput {
    PromptInput {
        root: PathBuf::from("~/<dossier-de-travail>"),
        night_folder: "2026-10-07 · Col des Étoiles · 7f3a2c".to_string(),
        targets: vec![
            "NGC 7000".to_string(),
            "IC 1396".to_string(),
            "M31".to_string(),
        ],
        focus: None,
        messages: Vec::new(),
        names: BTreeMap::from([
            ("M31".to_string(), "Galaxie d’Andromède".to_string()),
            (
                "NGC 7000".to_string(),
                "Nébuleuse North America".to_string(),
            ),
        ]),
    }
}

#[test]
fn the_french_prompt_on_macos_and_linux() {
    let prompt = build_prompt("fr", &night());
    snapshot("prompt-fr-unix.txt", &prompt);
    assert!(
        prompt.contains("Date de la nuit\u{a0}: du soir du 2026-10-07 au matin du 2026-10-08.\n")
    );
    assert!(prompt.contains("M31 (Galaxie d’Andromède)"));
    assert!(
        prompt.contains("~/<dossier-de-travail>/2026-10-07 · Col des Étoiles · 7f3a2c/M31\u{a0}»")
    );
    assert!(prompt.contains("et suis sa procédure, une question à la fois."));
}

#[test]
fn the_french_prompt_on_windows_keeps_backslashes() {
    let mut input = night();
    input.root = PathBuf::from(r"X:\<dossier-de-travail>");
    let prompt = build_prompt("fr", &input);
    snapshot("prompt-fr-windows.txt", &prompt);
    assert!(prompt.contains(r"X:\<dossier-de-travail>\ASSISTANT-IA.md"));
    assert!(prompt.contains(r"X:\<dossier-de-travail>\2026-10-07 · Col des Étoiles · 7f3a2c\M31"));
    assert!(!prompt.contains('/'));
}

#[test]
fn the_japanese_prompt_opens_with_its_language_line_and_one_target_carries_its_messages() {
    let mut input = night();
    input.focus = Some("M31".to_string());
    input.messages = vec![
        "M31/lights/ に 3 枚のフラットがあります。".to_string(),
        "  ".to_string(),
    ];
    let prompt = build_prompt("ja", &input);
    let lines: Vec<&str> = prompt.lines().collect();
    assert_eq!(lines[0], "日本語で話してください。");
    assert!(lines[2].contains("ASSISTANT-IA.md"));
    assert_eq!(
        lines[4],
        "夜の日付：2026-10-07 の夜から 2026-10-08 の朝まで"
    );
    assert!(lines[5].starts_with("対象天体：NGC 7000 (Nébuleuse North America)、IC 1396、M31"));
    let target_lines: Vec<&&str> = lines
        .iter()
        .filter(|l| l.contains("、フォルダー「"))
        .collect();
    assert_eq!(target_lines.len(), 1);
    assert!(target_lines[0].contains("M31 (Galaxie d’Andromède)"));
    assert!(prompt.contains("M31/lights/ に 3 枚のフラットがあります。\n"));
    assert_eq!(lines.len(), 11, "{prompt}");
}

#[test]
fn every_language_of_the_app_has_its_language_line_the_date_and_the_procedure() {
    let mut seen = Vec::new();
    for language in LANGUAGES {
        let prompt = build_prompt(language, &night());
        let first = prompt.lines().next().unwrap().to_string();
        assert_eq!(first, language_line(language));
        assert!(!seen.contains(&first), "{language}");
        seen.push(first);
        assert!(prompt.contains("ASSISTANT-IA.md"), "{language}");
        assert!(prompt.contains("Placing a file"), "{language}");
        assert!(
            prompt.contains("2026-10-07") && prompt.contains("2026-10-08"),
            "{language}"
        );
        assert!(prompt.contains("M31 (Galaxie d’Andromède)"), "{language}");
        assert!(!prompt.contains("Move-Item"), "{language}");
        assert!(!prompt.contains('{'), "{language}: {prompt}");
    }
    assert_eq!(language_line("fr"), "Parle-moi en français.");
    assert_eq!(language_line("it"), "Parlami in italiano.");
    assert_eq!(language_line("de"), "Sprich mit mir auf Deutsch.");
    assert_eq!(language_line("ko"), "Speak to me in English.");
}

#[test]
fn the_prompt_gives_what_the_file_says_it_gives() {
    let flow = Flow::committed();
    let input = night();
    let prompt = build_prompt("en", &input);
    for field in &flow.context {
        let given = match field.id {
            flow::ContextId::NightFolder => {
                prompt.contains(&format!("~/<dossier-de-travail>/{}", input.night_folder))
            }
            flow::ContextId::NightDate => {
                prompt.contains("from the evening of 2026-10-07 to the morning of 2026-10-08")
            }
            flow::ContextId::Targets => input.targets.iter().all(|t| prompt.contains(t.as_str())),
        };
        assert!(given, "{:?}: {prompt}", field.id);
    }
}

#[test]
fn a_folder_without_a_date_gets_no_date_line_and_no_name_repeats_its_designation() {
    let mut input = night();
    input.night_folder = "<dossier-de-la-nuit>".to_string();
    input
        .names
        .insert("IC 1396".to_string(), " IC 1396 ".to_string());
    let prompt = build_prompt("fr", &input);
    assert!(!prompt.contains("Date de la nuit"));
    assert!(prompt.contains("IC 1396, M31"));
    assert!(!prompt.contains("IC 1396 ("));
}

#[test]
fn the_morning_after_crosses_months_years_and_leap_days() {
    let dates = |name: &str| night_dates(name).map(|(e, m)| format!("{e}>{m}"));
    assert_eq!(
        dates("2026-10-07 · x").as_deref(),
        Some("2026-10-07>2026-10-08")
    );
    assert_eq!(
        dates("2026-10-31 · x").as_deref(),
        Some("2026-10-31>2026-11-01")
    );
    assert_eq!(
        dates("2026-12-31").as_deref(),
        Some("2026-12-31>2027-01-01")
    );
    assert_eq!(
        dates("2028-02-28").as_deref(),
        Some("2028-02-28>2028-02-29")
    );
    assert_eq!(
        dates("2027-02-28").as_deref(),
        Some("2027-02-28>2027-03-01")
    );
    assert_eq!(
        dates("2100-02-28").as_deref(),
        Some("2100-02-28>2100-03-01")
    );
    assert_eq!(dates("2026-13-01"), None);
    assert_eq!(dates("2026-09-31"), None);
    assert_eq!(dates("2026/10/07"), None);
    assert_eq!(dates("Nuit"), None);
    assert_eq!(dates("éé2026-10-07"), None);
}

#[test]
fn a_launch_after_an_update_rewrites_the_first_earlier_file_the_settings_recorded() {
    // A .dmg update: the first text at the root, its digest recorded in the
    // settings, equal to the earlier digest. Every launch applies the rule
    // (`prepare_root`), so the first one after the update rewrites it.
    let root = TempDir::new("assistant-file-launch-after-update");
    let path = root.path().join(ASSISTANT_FILE_NAME);
    fs::write(&path, EARLIER_TEXT).unwrap();
    assert_eq!(EARLIER_TEXT.len(), 1260, "the .dmg's file");
    let (created, file) =
        prepare_root(root.path(), Some(EARLIER_ASSISTANT_FILE_SHA256[0])).unwrap();
    assert!(!created);
    assert_eq!(file.outcome, AssistantFileOutcome::Rewritten);
    assert_eq!(fs::read_to_string(&path).unwrap(), ASSISTANT_FILE_TEXT);
    let (_, again) = prepare_root(root.path(), file.sha256.as_deref()).unwrap();
    assert_eq!(again.outcome, AssistantFileOutcome::Unchanged);
}

#[test]
fn replacing_a_changed_file_keeps_the_persons_copy_beside_it() {
    let root = TempDir::new("assistant-file-replace");
    let path = root.path().join(ASSISTANT_FILE_NAME);
    fs::write(&path, "mine").unwrap();
    let (copy, file) = replace_assistant_file(root.path(), "2026-10-10").unwrap();
    let copy = copy.expect("the person's file is kept");
    assert_eq!(copy, root.path().join("ASSISTANT-IA.modifie-2026-10-10.md"));
    assert_eq!(fs::read_to_string(&copy).unwrap(), "mine");
    assert_eq!(fs::read_to_string(&path).unwrap(), ASSISTANT_FILE_TEXT);
    assert_eq!(
        file.sha256.as_deref(),
        Some(assistant_file_sha256().as_str())
    );

    // Changed again the same day: the first copy is not overwritten.
    fs::write(&path, "mine again").unwrap();
    let (second, _) = replace_assistant_file(root.path(), "2026-10-10").unwrap();
    assert_eq!(
        second.unwrap(),
        root.path().join("ASSISTANT-IA.modifie-2026-10-10-2.md")
    );
    assert_eq!(fs::read_to_string(&copy).unwrap(), "mine");

    // Nothing to keep: today's text stays, no copy.
    let (none, _) = replace_assistant_file(root.path(), "2026-10-10").unwrap();
    assert!(none.is_none());
    assert_eq!(today_utc().len(), 10);
}

#[test]
fn the_logs_are_asked_in_every_path_once_and_may_be_skipped() {
    // The question is reachable on each way, asked once, and the answer
    // "none" leads on without a loop.
    for answers in [
        vec![
            "separate", "fits", "search", "none", "none", "none", "none", "no",
        ],
        vec![
            "separate", "fits", "search", "none", "none", "none", "search", "no",
        ],
        vec![
            "separate", "fits", "search", "none", "none", "none", "path", "no",
        ],
        vec!["separate", "final_only", "none", "search"],
        vec!["separate", "final_only", "path", "search"],
        vec!["separate", "final_only", "search", "search"],
    ] {
        let path = trace(&answers);
        assert_eq!(
            path.matches("Do you have the logs of this night").count(),
            1,
            "{answers:?}"
        );
        assert!(path.contains("NINA writes its logs"), "{answers:?}");
    }
    let skipped = trace(&[
        "separate", "fits", "search", "none", "none", "none", "none", "no",
    ]);
    assert!(!skipped.contains("Search for the logs"));
    assert!(skipped.contains("Sum up"));
}
