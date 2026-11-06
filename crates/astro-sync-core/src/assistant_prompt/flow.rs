//! The source of `ASSISTANT-IA.md`:
//! `assistant-file.json`, checked against `assistant-file.schema.json`, is
//! the folders, the rules and the step-by-step procedure an AI assistant
//! follows to gather a night's files. [`render`] writes it out as the
//! Markdown of `ASSISTANT-IA.md`, in English, the same bytes for everyone;
//! `README.md` beside the JSON says how to add a step.
//!
//! The JSON is read with unknown fields refused; the cross-references the
//! schema cannot express (a step's `next`, a kind, a question, an answer, a
//! placeholder) are checked by [`Flow::check`] and the tests.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::Deserialize;

/// The JSON, as committed.
pub const FLOW_JSON: &str = include_str!("assistant-file.json");
/// Its schema (JSON Schema 2020-12).
pub const FLOW_SCHEMA: &str = include_str!("assistant-file.schema.json");

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Flow {
    #[serde(rename = "$schema", default)]
    pub schema: Option<String>,
    pub version: u32,
    pub title: String,
    pub intro: Vec<String>,
    pub context: Vec<ContextField>,
    pub folders: Vec<Folder>,
    pub folders_note: String,
    pub rules: Vec<Rule>,
    /// Written only when the multi-night plans are open for the space
    /// ([`Flow::render_for`]).
    #[serde(default)]
    pub multi_night: Option<Section>,
    pub kinds: Vec<Kind>,
    pub actions: Actions,
    pub phrases: Phrases,
    pub questions: BTreeMap<String, String>,
    pub answers: BTreeMap<String, String>,
    pub steps: Vec<Step>,
}

/// A section of its own: a heading and its paragraphs.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Section {
    pub heading: String,
    pub paragraphs: Vec<String>,
}

/// What the copied prompt gives the assistant.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextField {
    pub id: ContextId,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextId {
    NightFolder,
    NightDate,
    Targets,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Folder {
    pub path: String,
    pub text: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub id: String,
    pub title: String,
    pub text: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Kind {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub destination: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Actions {
    pub search: String,
    pub ask_path: String,
    pub place: String,
    pub summarize: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Phrases {
    pub procedure_intro: String,
    pub procedure_follow: String,
    pub step: String,
    pub ask: String,
    pub branch: String,
    pub then: String,
    pub rules_heading: String,
    pub folders_heading: String,
    pub procedure_heading: String,
    pub list_and: String,
    pub list_separator: String,
    pub destination_item: String,
    pub destination_separator: String,
}

/// The closed set of actions a step may take.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// Search the files (in the path given, else the usual places).
    Search,
    /// Ask the full path of a folder.
    AskPath,
    /// Propose placing the files, by the rule `placement`.
    Place,
    /// Sum up and send the person back to Astro Sync; ends the procedure.
    Summarize,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub id: String,
    #[serde(default)]
    pub say: Option<String>,
    /// A key of `questions`.
    #[serde(default)]
    pub ask: Option<String>,
    #[serde(default)]
    pub action: Option<Action>,
    /// Ids of `kinds`.
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub options: Vec<Choice>,
    #[serde(default)]
    pub next: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Choice {
    /// A key of `answers`.
    pub answer: String,
    pub next: String,
}

/// The rule [`Action::Place`] names.
pub const PLACEMENT_RULE: &str = "placement";

/// The rule [`Action::Search`] names (links, aliases, shortcuts).
pub const LINKS_RULE: &str = "links";

impl Flow {
    /// The committed flow. It is checked by the tests: a failure here is a
    /// bug of the JSON shipped with the build.
    #[must_use]
    pub fn committed() -> Self {
        let flow: Self = serde_json::from_str(FLOW_JSON).expect("assistant-file.json is valid");
        if let Err(problems) = flow.check() {
            panic!("assistant-file.json: {}", problems.join("; "));
        }
        flow
    }

    #[must_use]
    pub fn step(&self, id: &str) -> Option<(usize, &Step)> {
        self.steps.iter().enumerate().find(|(_, s)| s.id == id)
    }

    fn kind(&self, id: &str) -> Option<&Kind> {
        self.kinds.iter().find(|k| k.id == id)
    }

    /// The steps that follow `step`, in the order of its branches.
    #[must_use]
    pub fn successors(step: &Step) -> Vec<&str> {
        let mut out: Vec<&str> = step.options.iter().map(|c| c.next.as_str()).collect();
        if let Some(next) = &step.next {
            out.push(next);
        }
        out
    }

    /// Every cross-reference the schema cannot express: unique ids, a
    /// known `next`, kind, question and answer, one way out per step (its
    /// `options`, its `next`, or the end for `summarize` only), the
    /// placeholders each text may use, a destination for every kind
    /// placed, the placement rule present.
    pub fn check(&self) -> Result<(), Vec<String>> {
        let mut problems = Vec::new();
        let mut seen = Vec::new();
        for step in &self.steps {
            if seen.contains(&step.id.as_str()) {
                problems.push(format!("step `{}` twice", step.id));
            }
            seen.push(step.id.as_str());
        }
        let mut kinds = Vec::new();
        for kind in &self.kinds {
            if kinds.contains(&kind.id.as_str()) {
                problems.push(format!("kind `{}` twice", kind.id));
            }
            kinds.push(kind.id.as_str());
        }
        for needed in [PLACEMENT_RULE, LINKS_RULE] {
            if !self.rules.iter().any(|r| r.id == needed) {
                problems.push(format!("no rule `{needed}`"));
            }
        }
        for step in &self.steps {
            let at = &step.id;
            for next in Self::successors(step) {
                if self.step(next).is_none() {
                    problems.push(format!("`{at}`: unknown step `{next}`"));
                }
            }
            if !step.options.is_empty() && step.next.is_some() {
                problems.push(format!("`{at}`: both `options` and `next`"));
            }
            let ends = step.options.is_empty() && step.next.is_none();
            if ends != (step.action == Some(Action::Summarize)) {
                problems.push(format!("`{at}`: only `summarize` ends the procedure"));
            }
            if !step.options.is_empty() && step.ask.is_none() {
                problems.push(format!("`{at}`: branches without a question"));
            }
            if step.ask.is_none() && step.action.is_none() && step.say.is_none() {
                problems.push(format!("`{at}`: says nothing"));
            }
            if let Some(ask) = &step.ask {
                match self.questions.get(ask) {
                    None => problems.push(format!("`{at}`: unknown question `{ask}`")),
                    Some(text) if text.contains("{files}") && step.files.is_empty() => {
                        problems.push(format!("`{at}`: the question needs `files`"));
                    }
                    Some(_) => {}
                }
            }
            for choice in &step.options {
                if !self.answers.contains_key(&choice.answer) {
                    problems.push(format!("`{at}`: unknown answer `{}`", choice.answer));
                }
            }
            for file in &step.files {
                match self.kind(file) {
                    None => problems.push(format!("`{at}`: unknown kind `{file}`")),
                    Some(kind)
                        if step.action == Some(Action::Place) && kind.destination.is_none() =>
                    {
                        problems.push(format!("`{at}`: kind `{file}` has no destination"));
                    }
                    Some(_) => {}
                }
            }
            if matches!(
                step.action,
                Some(Action::Search | Action::AskPath | Action::Place)
            ) && step.files.is_empty()
            {
                problems.push(format!("`{at}`: the action needs `files`"));
            }
        }
        let allowed: [(&str, &str, &[&str]); 9] = [
            ("actions.search", &self.actions.search, &["files", "links"]),
            ("actions.ask_path", &self.actions.ask_path, &["files"]),
            (
                "actions.place",
                &self.actions.place,
                &["destinations", "rule"],
            ),
            ("actions.summarize", &self.actions.summarize, &[]),
            ("phrases.step", &self.phrases.step, &["n"]),
            ("phrases.ask", &self.phrases.ask, &["question"]),
            ("phrases.branch", &self.phrases.branch, &["answer", "step"]),
            ("phrases.then", &self.phrases.then, &["step"]),
            (
                "phrases.destination_item",
                &self.phrases.destination_item,
                &["label", "destination"],
            ),
        ];
        for (name, text, names) in allowed {
            for found in placeholders(text) {
                if !names.contains(&found.as_str()) {
                    problems.push(format!("{name}: unknown placeholder `{{{found}}}`"));
                }
            }
            for wanted in names {
                if !text.contains(&format!("{{{wanted}}}")) {
                    problems.push(format!("{name}: `{{{wanted}}}` is missing"));
                }
            }
        }
        for (key, text) in &self.questions {
            for found in placeholders(text) {
                if found != "files" {
                    problems.push(format!(
                        "questions.{key}: unknown placeholder `{{{found}}}`"
                    ));
                }
            }
        }
        let mut free: Vec<(String, &str)> = vec![
            ("title".into(), &self.title),
            ("folders_note".into(), &self.folders_note),
            (
                "phrases.procedure_intro".into(),
                &self.phrases.procedure_intro,
            ),
            (
                "phrases.procedure_follow".into(),
                &self.phrases.procedure_follow,
            ),
        ];
        free.extend(self.intro.iter().map(|t| ("intro".into(), t.as_str())));
        free.extend(
            self.rules
                .iter()
                .map(|r| (format!("rules.{}", r.id), r.text.as_str())),
        );
        free.extend(
            self.answers
                .iter()
                .map(|(k, t)| (format!("answers.{k}"), t.as_str())),
        );
        free.extend(
            self.steps
                .iter()
                .filter_map(|s| s.say.as_deref().map(|t| (format!("steps.{}.say", s.id), t))),
        );
        for (name, text) in free {
            if let Some(found) = placeholders(text).first() {
                problems.push(format!("{name}: unknown placeholder `{{{found}}}`"));
            }
        }
        if problems.is_empty() {
            Ok(())
        } else {
            Err(problems)
        }
    }

    /// The labels of `files`, joined (« a, b and c »).
    fn labels(&self, files: &[String]) -> String {
        let labels: Vec<&str> = files
            .iter()
            .filter_map(|f| self.kind(f).map(|k| k.label.as_str()))
            .collect();
        join_list(
            &labels,
            &self.phrases.list_separator,
            &self.phrases.list_and,
        )
    }

    fn destinations(&self, files: &[String]) -> String {
        files
            .iter()
            .filter_map(|f| self.kind(f))
            .filter_map(|k| {
                k.destination.as_deref().map(|d| {
                    self.phrases
                        .destination_item
                        .replace("{label}", &k.label)
                        .replace("{destination}", d)
                })
            })
            .collect::<Vec<_>>()
            .join(&self.phrases.destination_separator)
    }

    /// One step, as the procedure writes it (no step number).
    #[must_use]
    pub fn render_step(&self, step: &Step) -> String {
        let number = |id: &str| self.step(id).map_or(0, |(i, _)| i + 1).to_string();
        let files = self.labels(&step.files);
        let mut sentences = Vec::new();
        if let Some(say) = &step.say {
            sentences.push(say.clone());
        }
        match step.action {
            Some(Action::Search) => {
                let rule = self
                    .rules
                    .iter()
                    .find(|r| r.id == LINKS_RULE)
                    .map_or("", |r| r.title.as_str());
                sentences.push(
                    self.actions
                        .search
                        .replace("{files}", &files)
                        .replace("{links}", rule),
                );
            }
            Some(Action::AskPath) => {
                sentences.push(self.actions.ask_path.replace("{files}", &files));
            }
            Some(Action::Place) => {
                let rule = self
                    .rules
                    .iter()
                    .find(|r| r.id == PLACEMENT_RULE)
                    .map_or("", |r| r.title.as_str());
                sentences.push(
                    self.actions
                        .place
                        .replace("{destinations}", &self.destinations(&step.files))
                        .replace("{rule}", rule),
                );
            }
            Some(Action::Summarize) => sentences.push(self.actions.summarize.clone()),
            None => {}
        }
        if let Some(question) = step.ask.as_ref().and_then(|q| self.questions.get(q)) {
            sentences.push(
                self.phrases
                    .ask
                    .replace("{question}", &question.replace("{files}", &files)),
            );
        }
        if let Some(next) = &step.next {
            sentences.push(self.phrases.then.replace("{step}", &number(next)));
        }
        let mut text = sentences.join(" ");
        for choice in &step.options {
            let answer = self.answers.get(&choice.answer).map_or("", String::as_str);
            let _ = write!(
                text,
                "\n  - {}",
                self.phrases
                    .branch
                    .replace("{answer}", answer)
                    .replace("{step}", &number(&choice.next))
            );
        }
        text
    }

    /// `ASSISTANT-IA.md`.
    #[must_use]
    pub fn render(&self) -> String {
        self.render_for(false)
    }

    /// `ASSISTANT-IA.md`, with the section of the multi-night plans after
    /// the rules when they are open for the space.
    #[must_use]
    pub fn render_for(&self, multi_night: bool) -> String {
        let mut out = format!("# {}\n", self.title);
        for paragraph in &self.intro {
            let _ = write!(out, "\n{paragraph}\n");
        }
        let _ = write!(out, "\n## {}\n\n", self.phrases.folders_heading);
        for folder in &self.folders {
            let _ = writeln!(out, "- `{}`: {}", folder.path, folder.text);
        }
        let _ = write!(out, "\n{}\n", self.folders_note);
        let _ = write!(out, "\n## {}\n\n", self.phrases.rules_heading);
        for rule in &self.rules {
            let _ = writeln!(out, "- **{}**: {}", rule.title, rule.text);
        }
        if let Some(section) = self.multi_night.as_ref().filter(|_| multi_night) {
            let _ = write!(out, "\n## {}\n", section.heading);
            for paragraph in &section.paragraphs {
                let _ = write!(out, "\n{paragraph}\n");
            }
        }
        let _ = write!(
            out,
            "\n## {}\n\n{}\n\n",
            self.phrases.procedure_heading, self.phrases.procedure_intro
        );
        for field in &self.context {
            let _ = writeln!(out, "- {}", field.text);
        }
        let _ = write!(out, "\n{}\n", self.phrases.procedure_follow);
        for (index, step) in self.steps.iter().enumerate() {
            let label = self.phrases.step.replace("{n}", &(index + 1).to_string());
            let _ = write!(out, "\n**{label}** {}\n", self.render_step(step));
        }
        out
    }
}

/// The `{name}`s of a text.
#[must_use]
pub fn placeholders(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else { break };
        let name = &after[..close];
        if !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
            out.push(name.to_string());
        }
        rest = &after[close + 1..];
    }
    out
}

fn join_list(items: &[&str], separator: &str, and: &str) -> String {
    match items {
        [] => String::new(),
        [one] => (*one).to_string(),
        [head @ .., last] => format!("{}{and}{last}", head.join(separator)),
    }
}
