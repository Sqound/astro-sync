//! The prompt "Ask an AI a question" copies: it sends an AI
//! assistant to read the Guide of Astro Sync, written in Markdown at the root of the working folder
//! (`GUIDE-ASTRO-SYNC.<language>.md`, [`crate::folder_files`]), and to
//! answer the person's questions.
//!
//! Like the prompt of the nights ([`crate::assistant_prompt::build_prompt`])
//! it is **in the language the person chose** (the application's by default)
//! and **opens with the same language line** ("Parle-moi en français."…, the
//! ten languages of the app); it says "tu" (a text for an AI, by exception
//! to the "vous" of the app's texts) and goes to the clipboard: **nothing
//! is ever sent to an AI service** by Astro Sync.
//!
//! It has two variants:
//!
//! - **with the guide's path** (the file was written, or kept as the person
//!   changed it): the full path, and the fallback sentence « if you cannot
//!   read this file, ask me to paste the guide »;
//! - **without a path** (the file could not be written): « I will paste the
//!   guide right after this message », and the assistant waits for it.
//!
//! Both give the working folder and say what the assistant may do when the
//! question goes beyond the guide: say so, **then look anyway**
//! (it may read the working folder, its files and its logs), separate what it
//! checked from what it supposes, and change or delete nothing without
//! asking.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::assistant_prompt::{join_display, language_line};
use crate::folder_files::normalise_language;

/// The words of the prompt in one language.
struct Phrases {
    intro: &'static str,
    /// `{root}`
    folder: &'static str,
    /// `{path}`
    read_guide: &'static str,
    /// Beyond the guide: say so, then look anyway.
    beyond: &'static str,
    fallback: &'static str,
    then_ask: &'static str,
    paste_soon: &'static str,
    wait_pasted: &'static str,
}

const FR: Phrases = Phrases {
    intro: "Tu vas répondre à mes questions sur Astro Sync, l’application qui range les fichiers d’une nuit d’astrophotographie et les envoie à Astro Portal.",
    folder: "Mon dossier de travail\u{a0}: «\u{a0}{root}\u{a0}».",
    read_guide: "Lis d’abord le guide «\u{a0}{path}\u{a0}»\u{a0}: il fait foi. Réponds d’après lui.",
    beyond: "Si ma question dépasse ce guide, dis-le, puis cherche quand même\u{a0}: tu peux lire le dossier de travail, ses fichiers et ses journaux. Sépare ce que tu as vérifié de ce que tu supposes, et ne modifie ni ne supprime rien sans me le demander.",
    fallback: "Si tu ne peux pas lire ce fichier, dis-le-moi et demande-moi de coller le guide dans la conversation.",
    then_ask: "Ensuite, demande-moi ce que je veux savoir.",
    paste_soon: "Je vais te coller le guide d’Astro Sync juste après ce message\u{a0}: réponds d’après lui.",
    wait_pasted: "Attends que je l’aie collé, puis demande-moi ce que je veux savoir.",
};

const EN: Phrases = Phrases {
    intro: "You are going to answer my questions about Astro Sync, the application that tidies the files of an astrophotography night and sends them to Astro Portal.",
    folder: "My working folder: “{root}”.",
    read_guide: "First read the guide “{path}”: it is authoritative. Answer from it.",
    beyond: "If my question goes beyond this guide, say so, then look anyway: you may read the working folder, its files and its logs. Separate what you checked from what you suppose, and change or delete nothing without asking me.",
    fallback: "If you cannot read that file, tell me and ask me to paste the guide into the conversation.",
    then_ask: "Then ask me what I want to know.",
    paste_soon: "I will paste the Astro Sync guide right after this message: answer from it.",
    wait_pasted: "Wait until I have pasted it, then ask me what I want to know.",
};

const JA: Phrases = Phrases {
    intro: "Astro Sync についての私の質問に答えてください。Astro Sync は、天体写真の一夜分のファイルを整理し、Astro Portal に送るアプリです。",
    folder: "私の作業フォルダー：「{root}」",
    read_guide: "まず、ガイド「{path}」を読んでください。このファイルを根拠に答えてください。",
    beyond: "私の質問がこのガイドの範囲を超える場合は、そう伝えたうえで、それでも調べてください。作業フォルダー、その中のファイル、ログを読んで構いません。確認したことと推測を分けて伝え、私に確認せずに何かを変更したり削除したりしないでください。",
    fallback: "このファイルを読めない場合は、そう伝えて、ガイドを会話に貼り付けるよう私に頼んでください。",
    then_ask: "そのうえで、知りたいことを私に尋ねてください。",
    paste_soon: "このメッセージの直後に Astro Sync のガイドを貼り付けます。それを根拠に答えてください。",
    wait_pasted: "貼り付け終わるまで待ってから、知りたいことを私に尋ねてください。",
};

const DE: Phrases = Phrases {
    intro: "Du beantwortest meine Fragen zu Astro Sync, der Anwendung, die die Dateien einer Astrofotografie-Nacht ordnet und an Astro Portal sendet.",
    folder: "Mein Arbeitsordner: „{root}“.",
    read_guide: "Lies zuerst die Anleitung „{path}“: Sie ist maßgeblich. Antworte anhand dieser Anleitung.",
    beyond: "Wenn meine Frage über diese Anleitung hinausgeht, sag es und recherchiere dann trotzdem: Du darfst den Arbeitsordner, seine Dateien und seine Logs lesen. Trenne, was du geprüft hast, von dem, was du vermutest, und ändere oder lösche nichts, ohne mich zu fragen.",
    fallback: "Wenn du diese Datei nicht lesen kannst, sag es mir und bitte mich, die Anleitung in die Unterhaltung einzufügen.",
    then_ask: "Frag mich danach, was ich wissen möchte.",
    paste_soon: "Ich füge dir die Anleitung von Astro Sync direkt nach dieser Nachricht ein: Antworte anhand dieser Anleitung.",
    wait_pasted: "Warte, bis ich sie eingefügt habe, und frag mich dann, was ich wissen möchte.",
};

const ES: Phrases = Phrases {
    intro: "Vas a responder a mis preguntas sobre Astro Sync, la aplicación que ordena los archivos de una noche de astrofotografía y los envía a Astro Portal.",
    folder: "Mi carpeta de trabajo: «{root}».",
    read_guide: "Lee primero la guía «{path}»: es la referencia. Responde según ella.",
    beyond: "Si mi pregunta va más allá de esta guía, dilo y busca de todos modos: puedes leer la carpeta de trabajo, sus archivos y sus registros. Separa lo que has comprobado de lo que supones, y no modifiques ni elimines nada sin preguntármelo.",
    fallback: "Si no puedes leer ese archivo, dímelo y pídeme que pegue la guía en la conversación.",
    then_ask: "Después, pregúntame lo que quiero saber.",
    paste_soon: "Voy a pegarte la guía de Astro Sync justo después de este mensaje: responde según ella.",
    wait_pasted: "Espera a que la haya pegado y luego pregúntame lo que quiero saber.",
};

const IT: Phrases = Phrases {
    intro: "Risponderai alle mie domande su Astro Sync, l’applicazione che sistema i file di una notte di astrofotografia e li invia ad Astro Portal.",
    folder: "La mia cartella di lavoro: «{root}».",
    read_guide: "Leggi prima la guida «{path}»: fa fede. Rispondi in base a essa.",
    beyond: "Se la mia domanda va oltre questa guida, dillo, poi cerca comunque: puoi leggere la cartella di lavoro, i suoi file e i suoi log. Distingui ciò che hai verificato da ciò che supponi, e non modificare né cancellare nulla senza chiedermelo.",
    fallback: "Se non riesci a leggere quel file, dimmelo e chiedimi di incollare la guida nella conversazione.",
    then_ask: "Poi chiedimi che cosa voglio sapere.",
    paste_soon: "Ti incollerò la guida di Astro Sync subito dopo questo messaggio: rispondi in base a essa.",
    wait_pasted: "Aspetta che l’abbia incollata, poi chiedimi che cosa voglio sapere.",
};

const PT_BR: Phrases = Phrases {
    intro: "Você vai responder às minhas perguntas sobre o Astro Sync, o aplicativo que organiza os arquivos de uma noite de astrofotografia e os envia ao Astro Portal.",
    folder: "Minha pasta de trabalho: “{root}”.",
    read_guide: "Leia primeiro o guia “{path}”: ele é a referência. Responda com base nele.",
    beyond: "Se a minha pergunta for além deste guia, diga isso e procure mesmo assim: você pode ler a pasta de trabalho, os arquivos dela e os registros. Separe o que você verificou do que você supõe, e não modifique nem apague nada sem me perguntar.",
    fallback: "Se você não conseguir ler esse arquivo, diga-me e peça-me que cole o guia na conversa.",
    then_ask: "Depois, pergunte-me o que eu quero saber.",
    paste_soon: "Vou colar o guia do Astro Sync logo depois desta mensagem: responda com base nele.",
    wait_pasted: "Espere eu colar e depois pergunte-me o que eu quero saber.",
};

const RU: Phrases = Phrases {
    intro: "Ты будешь отвечать на мои вопросы об Astro Sync — приложении, которое раскладывает файлы одной ночи астросъёмки и отправляет их в Astro Portal.",
    folder: "Моя рабочая папка: «{root}».",
    read_guide: "Сначала прочитай руководство «{path}»: оно главное. Отвечай по нему.",
    beyond: "Если мой вопрос выходит за рамки этого руководства, скажи об этом, а потом всё равно поищи: ты можешь читать рабочую папку, её файлы и журналы. Отделяй то, что ты проверил, от того, что предполагаешь, и ничего не изменяй и не удаляй, не спросив меня.",
    fallback: "Если ты не можешь прочитать этот файл, скажи мне об этом и попроси меня вставить руководство в беседу.",
    then_ask: "Затем спроси меня, что я хочу узнать.",
    paste_soon: "Сразу после этого сообщения я вставлю руководство Astro Sync: отвечай по нему.",
    wait_pasted: "Подожди, пока я его вставлю, а потом спроси меня, что я хочу узнать.",
};

const ZH_HANS: Phrases = Phrases {
    intro: "请回答我关于 Astro Sync 的问题。Astro Sync 是一款整理一夜天文摄影文件并将其发送到 Astro Portal 的应用。",
    folder: "我的工作文件夹：“{root}”。",
    read_guide: "请先阅读指南“{path}”：它是依据。请根据它回答。",
    beyond: "如果我的问题超出了这份指南，请直接说明，然后仍然去查：你可以读取工作文件夹、其中的文件和日志。请区分你已验证的内容和你推测的内容，未经我同意，不要修改或删除任何东西。",
    fallback: "如果你无法读取这个文件，请告诉我，并请我把指南粘贴到对话中。",
    then_ask: "然后，请问我想了解什么。",
    paste_soon: "我会在这条消息之后立刻粘贴 Astro Sync 的指南：请根据它回答。",
    wait_pasted: "请等我粘贴完，再问我想了解什么。",
};

const ZH_HANT: Phrases = Phrases {
    intro: "請回答我關於 Astro Sync 的問題。Astro Sync 是一款整理一夜天文攝影檔案並將其傳送到 Astro Portal 的應用程式。",
    folder: "我的工作資料夾：「{root}」。",
    read_guide: "請先閱讀指南「{path}」：它是依據。請根據它回答。",
    beyond: "如果我的問題超出了這份指南，請直接說明，然後仍然去查：你可以讀取工作資料夾、其中的檔案和日誌。請區分你已驗證的內容和你推測的內容，未經我同意，不要修改或刪除任何東西。",
    fallback: "如果你無法讀取這個檔案，請告訴我，並請我把指南貼到對話中。",
    then_ask: "然後，請問我想了解什麼。",
    paste_soon: "我會在這則訊息之後立刻貼上 Astro Sync 的指南：請根據它回答。",
    wait_pasted: "請等我貼完，再問我想了解什麼。",
};

fn phrases(language: &str) -> &'static Phrases {
    match normalise_language(language) {
        "fr" => &FR,
        "ja" => &JA,
        "de" => &DE,
        "es" => &ES,
        "it" => &IT,
        "pt-BR" => &PT_BR,
        "ru" => &RU,
        "zh-Hans" => &ZH_HANS,
        "zh-Hant" => &ZH_HANT,
        _ => &EN,
    }
}

/// What the prompt is about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuidePromptInput {
    /// The working folder.
    pub root: PathBuf,
    /// The guide's file name at the root (`GUIDE-ASTRO-SYNC.fr.md`); none
    /// when it could not be written: the prompt then says the guide will be
    /// pasted.
    #[serde(default)]
    pub guide_file: Option<String>,
}

/// The prompt, in `language` (one of [`crate::assistant_prompt::LANGUAGES`];
/// any other is English).
#[must_use]
pub fn build_guide_prompt(language: &str, input: &GuidePromptInput) -> String {
    let words = phrases(language);
    let mut lines = vec![
        language_line(language).to_string(),
        words.intro.to_string(),
        words
            .folder
            .replace("{root}", &join_display(&input.root, &[])),
    ];
    match &input.guide_file {
        Some(file) => {
            lines.push(
                words
                    .read_guide
                    .replace("{path}", &join_display(&input.root, &[file])),
            );
            lines.push(words.beyond.to_string());
            lines.push(words.fallback.to_string());
            lines.push(words.then_ask.to_string());
        }
        None => {
            lines.push(words.paste_soon.to_string());
            lines.push(words.beyond.to_string());
            lines.push(words.wait_pasted.to_string());
        }
    }
    lines.join("\n") + "\n"
}

#[cfg(test)]
mod tests;
