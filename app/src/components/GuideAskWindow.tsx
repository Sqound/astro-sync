import { useCallback, useRef, useState } from "react";

import { localeName, useI18n } from "../i18n";
import { createTranslator } from "../i18n/locale";
import {
  folderOpen,
  guidePrompt,
  guideWrite,
  managedFileReplace,
  workingFolderGet,
  workingFolderPrepare,
} from "../lib/bridge";
import { formatError } from "../lib/format";
import {
  GUIDE_LANGUAGES,
  afterGuideCopy,
  guideFileLine,
  guideNotices,
  languageName,
  prepareGuideAsk,
  replaceManagedFile,
  type GuideAsk,
  type GuideNotice,
} from "../lib/guideAsk";
import { guideMarkdown } from "../lib/guideMarkdown";
import { displayPath, homeOf } from "../lib/home";
import { afterCopy } from "../lib/promptWindow";
import { baseName } from "../lib/wizardSteps";
import { buttonClassName, helpTextClassName } from "../theme";
import { copyText } from "../screens/night/parts";
import { Icon } from "./Icon";
import { LanguageList } from "./LanguageList";
import { Notice } from "./Notice";
import { PromptWindow } from "./PromptWindow";
import { useSnackbar } from "./Snackbar";
import { useAssistantFileNotice } from "./useAssistantFileNotice";

/** What the window shows: what one click (or one choice of language) made
 * ready, and whether the prompt changed since it was copied. */
interface Shown {
  ask: GuideAsk;
  /** The language was changed in the window: the prompt is not copied yet. */
  changed: boolean;
  /** The clipboard refused: the prompt is selected for a copy by hand. */
  selectAtOpen: boolean;
}

/**
 * « Poser une question à une IA »: the same
 * mechanism as « Demander à un assistant IA » (`usePromptWindow`) for the
 * Guide. A click makes the files ready (`ASSISTANT-IA.md`, `README.md`, the
 * guide in Markdown), builds the prompt in the language of the application,
 * copies it and says so with « Voir le prompt »; the window shows the
 * prompt with the language to choose, the guide's file and « Copier le guide
 * entier ». A file the person changed is kept, said, and can be replaced
 * (its copy is kept first); the prompt works all the same.
 */
export function useGuideAsk(): {
  ask: () => Promise<void>;
  busy: boolean;
  window: React.ReactNode;
} {
  const { t, locale } = useI18n();
  const { notify } = useSnackbar();
  const noticeAssistantFile = useAssistantFileNotice();
  const [shown, setShown] = useState<Shown | null>(null);
  const [busy, setBusy] = useState(false);
  // `run` is stable across renders (an effect or a handler can hold it); the
  // snackbars it raises are made by the latest `tell`.
  const latest = useRef<(notice: GuideNotice, result: GuideAsk) => void>(
    () => undefined,
  );

  const say = useCallback((error: unknown) => formatError(t, error), [t]);

  /** The files made ready and the prompt built, in `language`; the notices
   * of what became of the files are said (and say nothing when all is well). */
  const run = useCallback(
    async (language: string): Promise<GuideAsk> => {
      const result = await prepareGuideAsk({
        language,
        render: (lang, root) =>
          guideMarkdown(
            createTranslator(lang),
            root ? displayPath(root, homeOf(root)) : "~/Documents/Astro Sync",
            root ? baseName(root) : "Astro Sync",
          ),
        prepare: workingFolderPrepare,
        write: guideWrite,
        build: guidePrompt,
        rootFallback: () => workingFolderGet().then((state) => state.root),
      });
      noticeAssistantFile(result.assistantFile);
      for (const notice of guideNotices(result, t, say, guideFileName)) {
        latest.current(notice, result);
      }
      return result;
    },
    [noticeAssistantFile, t, say],
  );

  /** One snackbar of the guide, with « Remplacer par la version d'Astro
   * Sync » when the person's file was kept. */
  latest.current = (notice, result) => {
    notify({
      key: notice.key,
      tone: notice.tone,
      text: notice.text,
      mode: notice.mode,
      action: notice.replace
        ? {
            label: t("assistantFile.replace"),
            run: () => void replace(notice.replace!, result),
          }
        : undefined,
    });
  };

  /** « Remplacer par la version d'Astro Sync »: the copy is kept first, then
   * the files are made ready again (the window, if open, shows the new
   * state; the prompt is not copied). */
  async function replace(kind: "guide" | "readme", result: GuideAsk) {
    const name =
      kind === "guide"
        ? result.guide.kind === "failed"
          ? guideFileName(result.language)
          : result.guide.file.name
        : (result.readme?.name ?? "README.md");
    const done = await replaceManagedFile(
      () =>
        managedFileReplace(
          kind,
          result.language,
          kind === "guide" ? result.text : undefined,
        ),
      name,
      say,
      t,
    );
    notify({
      key: kind === "guide" ? "guide-file" : "readme-file",
      ...done,
    });
    if (done.tone === "success") {
      const again = await prepareGuideAsk({
        language: result.language,
        render: () => result.text,
        prepare: workingFolderPrepare,
        write: guideWrite,
        build: guidePrompt,
        rootFallback: () => Promise.resolve(result.root),
      });
      setShown((current) => (current ? { ...current, ask: again } : current));
    }
  }

  const ask = useCallback(async () => {
    setBusy(true);
    try {
      const result = await run(locale);
      if (!result.prompt) return;
      const copied = await copyText(result.prompt);
      const said = afterCopy(copied);
      const open = (selectAtOpen: boolean) =>
        setShown({ ask: result, changed: false, selectAtOpen });
      notify({
        key: "assistant-prompt",
        tone: said.tone,
        text: t(said.textKey),
        action: {
          label: t("wizard.folder.viewPrompt"),
          run: () => open(false),
        },
      });
      if (said.openAtOnce) open(true);
    } finally {
      setBusy(false);
    }
  }, [locale, notify, run, t]);

  /** The language changed in the window: the guide is written in it and the
   * prompt made again, **without copying** (the clipboard changes only on
   * « Copier »). */
  async function changeLanguage(language: string) {
    const result = await run(language);
    setShown({ ask: result, changed: true, selectAtOpen: false });
  }

  const close = useCallback(() => setShown(null), []);

  async function copyGuide(current: GuideAsk) {
    const copied = await copyText(current.text);
    const said = afterGuideCopy(
      copied,
      Array.from(current.text).length,
      t,
      locale,
    );
    const path =
      current.guide.kind === "failed" ? null : current.guide.file.path;
    notify({
      key: "guide-copied",
      tone: said.tone,
      text: said.text,
      action:
        said.offersOpen && path
          ? { label: t("guide.ask.openGuide"), run: () => void openGuide(path) }
          : undefined,
    });
  }

  async function openGuide(path: string) {
    try {
      await folderOpen(path);
    } catch (error) {
      notify({ tone: "error", text: say(error) });
    }
  }

  let content: React.ReactNode = null;
  if (shown?.ask.prompt) {
    const current = shown.ask;
    const line = guideFileLine(current.guide, t, locale);
    const name = languageName(current.language, locale);
    content = (
      <PromptWindow
        prompt={current.prompt ?? ""}
        onClose={close}
        title={t("guide.ask.title")}
        hideContains
        selectAtOpen={shown.selectAtOpen}
        promptLabel={t("guide.ask.promptIn", { language: name })}
        onCopied={(copied) => {
          if (copied)
            setShown((state) => state && { ...state, changed: false });
        }}
        beforePrompt={
          <GuideAskHead
            shown={shown}
            line={line}
            filePath={
              current.guide.kind === "failed" ? null : current.guide.file.path
            }
            name={name}
            onLanguage={(language) => void changeLanguage(language)}
            onOpen={(path) => void openGuide(path)}
            onReplace={(kind) => void replace(kind, current)}
          />
        }
        extraActions={
          <button
            type="button"
            className={buttonClassName("secondary")}
            onClick={() => void copyGuide(current)}
          >
            <Icon name="copy" className="as-button__icon" />
            {t("guide.ask.copyGuide")}
          </button>
        }
        footer={<p className={helpTextClassName}>{t("guide.ask.fallback")}</p>}
      />
    );
  }
  return { ask, busy, window: content };
}

/** The file name of the guide in a language, as the core writes it. */
function guideFileName(language: string): string {
  return `GUIDE-ASTRO-SYNC.${language}.md`;
}

/** The language, the guide's file and what the window has to say about the
 * files. */
function GuideAskHead({
  shown,
  line,
  filePath,
  name,
  onLanguage,
  onOpen,
  onReplace,
}: {
  shown: Shown;
  line: string | null;
  filePath: string | null;
  name: string;
  onLanguage: (language: string) => void;
  onOpen: (path: string) => void;
  onReplace: (kind: "guide" | "readme") => void;
}) {
  const { t, locale } = useI18n();
  const { ask } = shown;
  const options = GUIDE_LANGUAGES.map((tag) => ({
    value: tag,
    name: localeName(tag),
    tag,
  }));
  return (
    <>
      <div className="as-guide-ask-window__language">
        <span className={helpTextClassName}>
          {t("guide.ask.language")}
          {ask.language === locale && ` · ${t("guide.ask.languageDefault")}`}
        </span>
        <LanguageList
          value={ask.language}
          options={options}
          label={t("guide.ask.language")}
          onChange={onLanguage}
        />
      </div>
      {line && filePath && (
        <div className="as-guide-ask-window__file">
          <Icon
            name="file"
            className="as-icon as-guide-ask-window__file-icon"
          />
          <span className="as-guide-ask-window__file-line">{line}</span>
          <button
            type="button"
            className={buttonClassName("link")}
            onClick={() => onOpen(filePath)}
          >
            <Icon name="external" className="as-button__icon" />
            {t("guide.ask.openGuide")}
          </button>
        </div>
      )}
      {ask.guide.kind === "failed" && (
        <Notice tone="warning">
          {t("guide.ask.windowFailed", {
            reason: formatError(t, ask.guide.error),
          })}
        </Notice>
      )}
      {ask.guide.kind === "kept" && (
        <Notice tone="warning">
          <KeptLine
            text={t("guide.ask.kept", { name: ask.guide.file.name })}
            onReplace={() => onReplace("guide")}
          />
        </Notice>
      )}
      {ask.readme?.outcome === "kept_as_changed" && (
        <Notice tone="warning">
          <KeptLine
            text={t("guide.ask.kept", { name: ask.readme.name })}
            onReplace={() => onReplace("readme")}
          />
        </Notice>
      )}
      {shown.changed && (
        <Notice tone="neutral">
          {t("guide.ask.changed", { language: name })}
        </Notice>
      )}
    </>
  );
}

function KeptLine({
  text,
  onReplace,
}: {
  text: string;
  onReplace: () => void;
}) {
  const { t } = useI18n();
  return (
    <span className="as-guide-ask-window__kept">
      <span>{text}</span>
      <button
        type="button"
        className={buttonClassName("link")}
        onClick={onReplace}
      >
        {t("assistantFile.replace")}
      </button>
    </span>
  );
}
