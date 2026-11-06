//! `ASSISTANT-IA.md` and the prompt "Ask an AI assistant" copies.
//!
//! - **`ASSISTANT-IA.md`** is one file, **in English, whatever the
//!   application's language**: the same bytes for
//!   everyone. It carries the folders, the rules **and the
//!   step-by-step procedure** an assistant follows to gather a night's files
//!   (one folder or separate places; raw FITS or only a final image; search
//!   or a full path; lights, darks, flats and dark-flats, offsets; a
//!   processing already done into `final/`; a summary, then back to Astro
//!   Sync). Its source of truth is `assistant_prompt/assistant-file.json`
//!   (schema beside it, [`flow`]); [`ASSISTANT_FILE_TEXT`] is its rendering,
//!   committed and held equal to it by a test. It is written at the root of
//!   the working folder when the root is made, and written again only while
//!   it still holds bytes Astro Sync wrote: the digest kept in the local
//!   settings (`assistant_file_sha256`), or one of the earlier texts
//!   ([`EARLIER_ASSISTANT_FILE_SHA256`]). A file the person changed is left
//!   as it is ([`write_assistant_file`]).
//! - **The multi-night plans**: when they are open for
//!   the space, the file carries one section more, « Multi-night plans »
//!   (one folder per night, the same target names from one night to the
//!   next, where the integrations are, never two nights in one folder):
//!   [`ASSISTANT_FILE_TEXT_MULTI_NIGHT`], written by
//!   [`write_assistant_file_for`]. Each text is Astro Sync's for the other:
//!   the file follows the right as it comes and goes.
//! - **The prompt** ([`build_prompt`]) stays short. It is **in the
//!   application's language** and **opens with a language line**
//!   ("Parle-moi en français.", "日本語で話してください。"…, the ten
//!   languages of the app). It names `ASSISTANT-IA.md` and asks to follow its
//!   procedure one question at a time, gives the night's folder and date
//!   (the evening, then the morning after), the targets (designation, and
//!   common name when the caller knows it) with their exact paths, and the
//!   messages of "Files" when it is about one target; nothing else (no
//!   address, no place beyond the folder's name, no full identifier). It
//!   says "tu" (a text for an AI, by exception to the "vous" of the
//!   app's texts). It goes to the clipboard: **nothing is ever sent to an AI
//!   service** by Astro Sync.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::working_folder::ASSISTANT_FILE_NAME;

pub mod flow;

/// The text of `ASSISTANT-IA.md`: the rendering of `assistant-file.json`
/// ([`flow::Flow::render`]), committed so that its bytes are reviewed.
pub const ASSISTANT_FILE_TEXT: &str =
    include_str!("assistant_prompt/snapshots/ASSISTANT-IA.md.txt");

/// The text of `ASSISTANT-IA.md` when the multi-night plans are open for
/// the space: [`ASSISTANT_FILE_TEXT`] and the section « Multi-night
/// plans » ([`flow::Flow::render_for`]).
pub const ASSISTANT_FILE_TEXT_MULTI_NIGHT: &str =
    include_str!("assistant_prompt/snapshots/ASSISTANT-IA.multi-night.md.txt");

/// The text written for a space whose multi-night plans are open or not.
#[must_use]
pub fn assistant_text(multi_night: bool) -> &'static str {
    if multi_night {
        ASSISTANT_FILE_TEXT_MULTI_NIGHT
    } else {
        ASSISTANT_FILE_TEXT
    }
}

/// The SHA-256 of the texts of `ASSISTANT-IA.md` earlier versions wrote,
/// oldest first: such a file is Astro Sync's, and is written again even when
/// the settings lost its digest.
pub const EARLIER_ASSISTANT_FILE_SHA256: [&str; 5] = [
    "4c89aca009e1fa0fe7089fd7078418d1d68f5911882d60153292574998dd57c4",
    // Before the links rule.
    "e755edf53afc1c4bcdc36102450c359e7f232526a26af914a81ecf1b2a633d78",
    // Before the logs step.
    "79f01cb7a7aa38ea5d920dbc4ea9f766e9df0cb5e3f00776d2610d407691c17c",
    // Before the hidden work folder of a night.
    "144610e34a59fd7bf6c8c07c2569c7864f0ad5ae1920e29232bb8aee88eef878",
    // Before the masters kept in `stacked/`.
    "4a9df5f9ff4d566c94d4b0047f18294166d2f2061247425022c69c984abe0217",
];

/// The SHA-256 of [`ASSISTANT_FILE_TEXT`].
#[must_use]
pub fn assistant_file_sha256() -> String {
    assistant_file_sha256_for(false)
}

/// The SHA-256 of [`assistant_text`].
#[must_use]
pub fn assistant_file_sha256_for(multi_night: bool) -> String {
    format!(
        "{:x}",
        Sha256::digest(assistant_text(multi_night).as_bytes())
    )
}

/// What became of `ASSISTANT-IA.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantFileOutcome {
    /// It was not there: written.
    Created,
    /// It held an earlier text Astro Sync wrote (the digest recorded, or
    /// one of [`EARLIER_ASSISTANT_FILE_SHA256`]): written again.
    Rewritten,
    /// It holds today's text.
    Unchanged,
    /// The person changed it: left as it is.
    KeptAsChanged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantFile {
    pub path: PathBuf,
    pub outcome: AssistantFileOutcome,
    /// The digest to keep in the settings (`assistant_file_sha256`): the
    /// text Astro Sync wrote, or the one it kept before when the person
    /// changed the file.
    pub sha256: Option<String>,
}

/// Writes `ASSISTANT-IA.md` at `root` under the rule of the module
/// documentation; `recorded` is the digest of the text Astro Sync last
/// wrote there.
pub fn write_assistant_file(root: &Path, recorded: Option<&str>) -> std::io::Result<AssistantFile> {
    write_assistant_file_for(root, recorded, false)
}

/// [`write_assistant_file`] for a space whose multi-night plans are open or
/// not: the other text is Astro Sync's too, and is written over.
pub fn write_assistant_file_for(
    root: &Path,
    recorded: Option<&str>,
    multi_night: bool,
) -> std::io::Result<AssistantFile> {
    let text = assistant_text(multi_night);
    let path = root.join(ASSISTANT_FILE_NAME);
    let ours = assistant_file_sha256_for(multi_night);
    let other = assistant_file_sha256_for(!multi_night);
    let outcome = match fs::read(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?;
            file.write_all(text.as_bytes())?;
            AssistantFileOutcome::Created
        }
        Err(error) => return Err(error),
        Ok(bytes) => {
            let theirs = format!("{:x}", Sha256::digest(&bytes));
            if theirs == ours {
                AssistantFileOutcome::Unchanged
            } else if recorded.is_some_and(|r| r.eq_ignore_ascii_case(&theirs))
                || EARLIER_ASSISTANT_FILE_SHA256.contains(&theirs.as_str())
                || theirs == other
            {
                fs::write(&path, text)?;
                AssistantFileOutcome::Rewritten
            } else {
                AssistantFileOutcome::KeptAsChanged
            }
        }
    };
    let sha256 = match outcome {
        AssistantFileOutcome::KeptAsChanged => recorded.map(str::to_string),
        _ => Some(ours),
    };
    Ok(AssistantFile {
        path,
        outcome,
        sha256,
    })
}

/// Today's date in UTC, `YYYY-MM-DD` (the name of the copy
/// [`replace_assistant_file`] keeps).
pub fn today_utc() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let (year, month, day) = crate::time_util::civil_from_days((seconds / 86_400) as i64);
    format!("{year:04}-{month:02}-{day:02}")
}

/// "Replace with Astro Sync’s version": the file
/// the person changed is first **renamed** `ASSISTANT-IA.modifie-<date>.md`
/// in the same folder (`-2`, `-3`… when that name is taken), then today's
/// text is written. Nothing is ever deleted. Returns the copy's path (none
/// when there was nothing to keep: no file, or already today's text).
pub fn replace_assistant_file(
    root: &Path,
    date: &str,
) -> std::io::Result<(Option<PathBuf>, AssistantFile)> {
    replace_assistant_file_for(root, date, false)
}

/// [`replace_assistant_file`] with the text of a space whose multi-night
/// plans are open or not.
pub fn replace_assistant_file_for(
    root: &Path,
    date: &str,
    multi_night: bool,
) -> std::io::Result<(Option<PathBuf>, AssistantFile)> {
    let path = root.join(ASSISTANT_FILE_NAME);
    let mut copy = None;
    if let Ok(bytes) = fs::read(&path)
        && format!("{:x}", Sha256::digest(&bytes)) != assistant_file_sha256_for(multi_night)
    {
        let stem = ASSISTANT_FILE_NAME.trim_end_matches(".md");
        let mut candidate = root.join(format!("{stem}.modifie-{date}.md"));
        let mut n = 2;
        while candidate.exists() {
            candidate = root.join(format!("{stem}.modifie-{date}-{n}.md"));
            n += 1;
        }
        fs::rename(&path, &candidate)?;
        copy = Some(candidate);
    }
    let file = write_assistant_file_for(root, None, multi_night)?;
    Ok((copy, file))
}

/// The working folder made ready at the first signed-in launch, or after
/// "Change folder": the root created when absent, then
/// `ASSISTANT-IA.md` under its rule. Returns whether the root was created.
pub fn prepare_root(
    root: &Path,
    recorded: Option<&str>,
) -> Result<(bool, AssistantFile), crate::working_folder::WorkingFolderError> {
    prepare_root_for(root, recorded, false)
}

/// [`prepare_root`] for a space whose multi-night plans are open or not.
pub fn prepare_root_for(
    root: &Path,
    recorded: Option<&str>,
    multi_night: bool,
) -> Result<(bool, AssistantFile), crate::working_folder::WorkingFolderError> {
    let created = crate::working_folder::ensure_root(root)?;
    let file = write_assistant_file_for(root, recorded, multi_night).map_err(|source| {
        crate::working_folder::WorkingFolderError::Io {
            path: root.join(ASSISTANT_FILE_NAME),
            source,
        }
    })?;
    Ok((created, file))
}

/* -------------------------------- The prompt ----------------------------- */

/// The words of the prompt in one language.
struct Phrases {
    language_line: &'static str,
    intro: &'static str,
    /// `{file}`
    read_first: &'static str,
    /// `{night}`
    night: &'static str,
    /// `{evening}`, `{morning}` (ISO dates)
    date: &'static str,
    /// `{targets}`
    targets: &'static str,
    /// `{target}`, `{path}`
    target: &'static str,
    files: &'static str,
    propose: &'static str,
    wait: &'static str,
    list_separator: &'static str,
}

const FR: Phrases = Phrases {
    language_line: "Parle-moi en français.",
    intro: "Tu m’aides à ranger les fichiers d’une nuit d’astrophotographie pour Astro Sync.",
    read_first: "Lis d’abord «\u{a0}{file}\u{a0}» (il est en anglais) et suis sa procédure, une question à la fois.",
    night: "Nuit\u{a0}: «\u{a0}{night}\u{a0}».",
    date: "Date de la nuit\u{a0}: du soir du {evening} au matin du {morning}.",
    targets: "Cibles\u{a0}: {targets} (un dossier chacune dans la nuit).",
    target: "Cible\u{a0}: {target}, dossier «\u{a0}{path}\u{a0}».",
    files: "Fichiers à ranger\u{a0}: ceux que je t’indique, ou ceux qui sont hors de la structure. Demande-moi aussi les journaux de la nuit (NINA, PHD2, ASIAIR)\u{a0}: ils donnent les étapes et la qualité du guidage.",
    propose: "Avant de placer un fichier, propose-moi les choix de la règle «\u{a0}Placing a file\u{a0}» d’ASSISTANT-IA.md,",
    wait: "sans rien supprimer, puis attends ma réponse.",
    list_separator: ", ",
};

const EN: Phrases = Phrases {
    language_line: "Speak to me in English.",
    intro: "You are helping me tidy the files of an astrophotography night for Astro Sync.",
    read_first: "First read “{file}” (it is in English) and follow its procedure, one question at a time.",
    night: "Night: “{night}”.",
    date: "Night’s date: from the evening of {evening} to the morning of {morning}.",
    targets: "Targets: {targets} (one folder each in the night).",
    target: "Target: {target}, folder “{path}”.",
    files: "Files to tidy: the ones I point out to you, or the ones outside the structure. Also ask me for the night’s logs (NINA, PHD2, ASIAIR): they give the steps and the guiding quality.",
    propose: "Before placing a file, offer me the choices of the rule “Placing a file” in ASSISTANT-IA.md,",
    wait: "without deleting anything, then wait for my answer.",
    list_separator: ", ",
};

const JA: Phrases = Phrases {
    language_line: "日本語で話してください。",
    intro: "Astro Sync のために、天体写真の一夜分のファイルを整理するのを手伝ってください。",
    read_first: "まず「{file}」を読んでください（英語で書かれています）。その手順に従い、質問は一度に一つずつしてください。",
    night: "夜：「{night}」",
    date: "夜の日付：{evening} の夜から {morning} の朝まで",
    targets: "対象天体：{targets}（夜のフォルダーの中に、それぞれ1つずつフォルダーがあります）",
    target: "対象天体：{target}、フォルダー「{path}」",
    files: "整理するファイル：私が指示するもの、または決まった構成の外にあるもの。夜のログ（NINA、PHD2、ASIAIR）についても私に尋ねてください。作業の段階とガイディングの質がわかります。",
    propose: "ファイルを置く前に、ASSISTANT-IA.md の「Placing a file」のルールにある選択肢を示してください。",
    wait: "何も削除せずに、私の返事を待ってください。",
    list_separator: "、",
};

const DE: Phrases = Phrases {
    language_line: "Sprich mit mir auf Deutsch.",
    intro: "Du hilfst mir, die Dateien einer Astrofotografie-Nacht für Astro Sync zu ordnen.",
    read_first: "Lies zuerst „{file}“ (die Datei ist auf Englisch) und folge ihrem Ablauf, eine Frage nach der anderen.",
    night: "Nacht: „{night}“.",
    date: "Datum der Nacht: vom Abend des {evening} bis zum Morgen des {morning}.",
    targets: "Ziele: {targets} (je ein Ordner in der Nacht).",
    target: "Ziel: {target}, Ordner „{path}“.",
    files: "Zu ordnende Dateien: die, die ich dir nenne, oder die außerhalb der Struktur. Frag mich auch nach den Logs der Nacht (NINA, PHD2, ASIAIR): Sie liefern die Schritte und die Qualität der Nachführung.",
    propose: "Bevor du eine Datei ablegst, biete mir die Möglichkeiten der Regel „Placing a file“ aus ASSISTANT-IA.md an,",
    wait: "ohne etwas zu löschen, und warte dann auf meine Antwort.",
    list_separator: ", ",
};

const ES: Phrases = Phrases {
    language_line: "Háblame en español.",
    intro: "Me ayudas a ordenar los archivos de una noche de astrofotografía para Astro Sync.",
    read_first: "Lee primero «{file}» (está en inglés) y sigue su procedimiento, una pregunta cada vez.",
    night: "Noche: «{night}».",
    date: "Fecha de la noche: desde la noche del {evening} hasta la mañana del {morning}.",
    targets: "Objetivos: {targets} (una carpeta para cada uno en la noche).",
    target: "Objetivo: {target}, carpeta «{path}».",
    files: "Archivos que ordenar: los que te indique, o los que están fuera de la estructura. Pídeme también los registros de la noche (NINA, PHD2, ASIAIR): dan las etapas y la calidad del guiado.",
    propose: "Antes de colocar un archivo, ofréceme las opciones de la regla «Placing a file» de ASSISTANT-IA.md,",
    wait: "sin borrar nada, y luego espera mi respuesta.",
    list_separator: ", ",
};

const IT: Phrases = Phrases {
    language_line: "Parlami in italiano.",
    intro: "Mi aiuti a sistemare i file di una notte di astrofotografia per Astro Sync.",
    read_first: "Leggi prima «{file}» (è in inglese) e segui la sua procedura, una domanda alla volta.",
    night: "Notte: «{night}».",
    date: "Data della notte: dalla sera del {evening} alla mattina del {morning}.",
    targets: "Oggetti: {targets} (una cartella ciascuno nella notte).",
    target: "Oggetto: {target}, cartella «{path}».",
    files: "File da sistemare: quelli che ti indico, o quelli fuori dalla struttura. Chiedimi anche i log della notte (NINA, PHD2, ASIAIR): danno le fasi e la qualità del guiding.",
    propose: "Prima di sistemare un file, proponimi le scelte della regola «Placing a file» di ASSISTANT-IA.md,",
    wait: "senza cancellare nulla, poi aspetta la mia risposta.",
    list_separator: ", ",
};

const PT_BR: Phrases = Phrases {
    language_line: "Fale comigo em português.",
    intro: "Você vai me ajudar a organizar os arquivos de uma noite de astrofotografia para o Astro Sync.",
    read_first: "Leia primeiro “{file}” (está em inglês) e siga o procedimento dele, uma pergunta de cada vez.",
    night: "Noite: “{night}”.",
    date: "Data da noite: da noite de {evening} até a manhã de {morning}.",
    targets: "Alvos: {targets} (uma pasta para cada um na noite).",
    target: "Alvo: {target}, pasta “{path}”.",
    files: "Arquivos a organizar: os que eu indicar, ou os que estão fora da estrutura. Peça-me também os registros da noite (NINA, PHD2, ASIAIR): eles dão as etapas e a qualidade do guiamento.",
    propose: "Antes de colocar um arquivo, ofereça-me as opções da regra “Placing a file” do ASSISTANT-IA.md,",
    wait: "sem apagar nada, e depois espere a minha resposta.",
    list_separator: ", ",
};

const RU: Phrases = Phrases {
    language_line: "Говори со мной по-русски.",
    intro: "Ты помогаешь мне разложить файлы одной ночи астросъёмки для Astro Sync.",
    read_first: "Сначала прочитай «{file}» (он на английском) и следуй его порядку действий, задавая по одному вопросу за раз.",
    night: "Ночь: «{night}».",
    date: "Дата ночи: с вечера {evening} до утра {morning}.",
    targets: "Объекты: {targets} (по одной папке на каждый в папке ночи).",
    target: "Объект: {target}, папка «{path}».",
    files: "Файлы, которые нужно разложить: те, что я укажу, или те, что лежат вне структуры. Спроси меня также о журналах ночи (NINA, PHD2, ASIAIR): по ним видны этапы и качество гидирования.",
    propose: "Прежде чем разместить файл, предложи мне варианты из правила «Placing a file» в ASSISTANT-IA.md,",
    wait: "ничего не удаляя, а затем дождись моего ответа.",
    list_separator: ", ",
};

const ZH_HANS: Phrases = Phrases {
    language_line: "请用简体中文和我交流。",
    intro: "你要帮我为 Astro Sync 整理一个天文摄影之夜的文件。",
    read_first: "请先阅读“{file}”（它是英文的），并按照其中的步骤进行，一次只问一个问题。",
    night: "夜晚：“{night}”。",
    date: "夜晚日期：从 {evening} 晚上到 {morning} 早上。",
    targets: "目标：{targets}（每个目标在这个夜晚的文件夹里各有一个文件夹）。",
    target: "目标：{target}，文件夹“{path}”。",
    files: "要整理的文件：我指给你的那些，或者在结构之外的那些。请同时向我索取当晚的日志（NINA、PHD2、ASIAIR）：它们记录了各个步骤和导星的质量。",
    propose: "放置文件之前，请给我 ASSISTANT-IA.md 中“Placing a file”规则的选项，",
    wait: "不要删除任何东西，然后等待我的回答。",
    list_separator: "、",
};

const ZH_HANT: Phrases = Phrases {
    language_line: "請用繁體中文和我交流。",
    intro: "你要幫我為 Astro Sync 整理一個天文攝影之夜的檔案。",
    read_first: "請先閱讀「{file}」（它是英文的），並依照其中的步驟進行，一次只問一個問題。",
    night: "夜晚：「{night}」。",
    date: "夜晚日期：從 {evening} 晚上到 {morning} 早上。",
    targets: "目標：{targets}（每個目標在這個夜晚的資料夾裡各有一個資料夾）。",
    target: "目標：{target}，資料夾「{path}」。",
    files: "要整理的檔案：我指給你的那些，或是在結構之外的那些。請同時向我索取當晚的日誌（NINA、PHD2、ASIAIR）：它們記錄了各個步驟和導星的品質。",
    propose: "放置檔案之前，請給我 ASSISTANT-IA.md 中「Placing a file」規則的選項，",
    wait: "不要刪除任何東西，然後等我回覆。",
    list_separator: "、",
};

/// The ten languages of the application.
pub const LANGUAGES: [&str; 10] = [
    "fr", "en", "ja", "de", "es", "it", "pt-BR", "ru", "zh-Hans", "zh-Hant",
];

fn phrases(language: &str) -> &'static Phrases {
    match language {
        "fr" => &FR,
        "ja" => &JA,
        "de" => &DE,
        "es" => &ES,
        "it" => &IT,
        "pt-BR" | "pt" => &PT_BR,
        "ru" => &RU,
        "zh-Hans" | "zh" | "zh-CN" => &ZH_HANS,
        "zh-Hant" | "zh-TW" | "zh-HK" => &ZH_HANT,
        _ => &EN,
    }
}

/// The language line a prompt opens with.
#[must_use]
pub fn language_line(language: &str) -> &'static str {
    phrases(language).language_line
}

/// What the prompt is about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptInput {
    /// The working folder.
    pub root: PathBuf,
    /// The night's folder name (`2026-10-07 · Col des Étoiles · 7f3a2c`);
    /// its leading date is the evening's.
    pub night_folder: String,
    /// The folder names of the ticked targets (their designations).
    pub targets: Vec<String>,
    /// "Files", one target: only its line, with its messages.
    #[serde(default)]
    pub focus: Option<String>,
    /// The messages of "Files" for it, one per line (already in the
    /// application's language).
    #[serde(default)]
    pub messages: Vec<String>,
    /// The common names the caller knows, by target folder
    /// (`M31` → `Galaxie d'Andromède`), in the application's language.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub names: BTreeMap<String, String>,
}

/// The evening and the morning after of a night folder named
/// `YYYY-MM-DD · …` (ISO dates); `None` when the name does not start with a
/// valid date.
#[must_use]
pub fn night_dates(night_folder: &str) -> Option<(String, String)> {
    let date = night_folder.get(..10)?;
    let bytes = date.as_bytes();
    if !date.is_ascii() {
        return None;
    }
    if bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    let year: u32 = date[..4].parse().ok()?;
    let month: u32 = date[5..7].parse().ok()?;
    let day: u32 = date[8..10].parse().ok()?;
    let leap = (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    };
    if day == 0 || day > days {
        return None;
    }
    let (y, m, d) = if day < days {
        (year, month, day + 1)
    } else if month < 12 {
        (year, month + 1, 1)
    } else {
        (year + 1, 1, 1)
    };
    Some((date.to_string(), format!("{y:04}-{m:02}-{d:02}")))
}

/// `base` followed by `parts`, with the separator `base` already uses (a
/// Windows path shown by the window keeps its backslashes on any machine).
pub(crate) fn join_display(base: &Path, parts: &[&str]) -> String {
    let base = base.display().to_string();
    let separator = if base.contains('\\') && !base.contains('/') {
        '\\'
    } else {
        '/'
    };
    let mut out = base.trim_end_matches(['/', '\\']).to_string();
    for part in parts {
        out.push(separator);
        out.push_str(part);
    }
    out
}

/// The prompt, in `language` (one of [`LANGUAGES`]; any other is English).
#[must_use]
pub fn build_prompt(language: &str, input: &PromptInput) -> String {
    let words = phrases(language);
    let night = join_display(&input.root, &[&input.night_folder]);
    let file = join_display(&input.root, &[ASSISTANT_FILE_NAME]);
    let named = |target: &str| match input.names.get(target).map(|n| n.trim()) {
        Some(name) if !name.is_empty() && name != target => format!("{target} ({name})"),
        _ => target.to_string(),
    };
    let mut lines = vec![
        words.language_line.to_string(),
        words.intro.to_string(),
        words.read_first.replace("{file}", &file),
        words.night.replace("{night}", &night),
    ];
    if let Some((evening, morning)) = night_dates(&input.night_folder) {
        lines.push(
            words
                .date
                .replace("{evening}", &evening)
                .replace("{morning}", &morning),
        );
    }
    let all: Vec<String> = input.targets.iter().map(|t| named(t)).collect();
    lines.push(
        words
            .targets
            .replace("{targets}", &all.join(words.list_separator)),
    );
    let shown: Vec<&String> = match &input.focus {
        Some(focus) => input.targets.iter().filter(|t| *t == focus).collect(),
        None => input.targets.iter().collect(),
    };
    for target in shown {
        lines.push(words.target.replace("{target}", &named(target)).replace(
            "{path}",
            &join_display(&input.root, &[&input.night_folder, target]),
        ));
    }
    lines.push(words.files.to_string());
    lines.extend(
        input
            .messages
            .iter()
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty()),
    );
    lines.push(words.propose.to_string());
    lines.push(words.wait.to_string());
    lines.join("\n") + "\n"
}

#[cfg(test)]
mod tests;
