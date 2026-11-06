// « Poser une question à une IA »: the button
// at the top of the Guide. Same mechanism as « Demander à un assistant IA »
// (`promptWindow.ts`): the files are made ready first, the prompt is built by
// the core in the language chosen and copied, a snackbar says so with « Voir
// le prompt », the window shows it. What is new is the guide: the Guide page
// in Markdown, written at the root of the working folder as
// `GUIDE-ASTRO-SYNC.<language>.md` under the rule of `ASSISTANT-IA.md`, and the
// README of the folder, compared to their fingerprints on every click:
// a file the person changed is kept, said, and can be replaced by
// Astro Sync's version (the changed copy is kept first); the prompt still
// works without restoring. Pure TypeScript, tested under `node --test`.

import { SUPPORTED_LOCALES, localeName } from "../i18n/locale.ts";
import type { Translate } from "../i18n/locale.ts";
import type { GuidePromptInput, ManagedFile } from "./bridge.ts";
import { displayPath, homeOf } from "./home.ts";
import {
  ensureAssistantFile,
  type AssistantFileState,
} from "./promptWindow.ts";
import { formatSize } from "./written.ts";

/** The languages of the list: the registry's locales, French then English
 * first (the core's `assistant_prompt::LANGUAGES` order), the
 * others in the registry's order; a test holds them equal to the
 * application's languages. */
const GUIDE_FIRST = (tag: string): number =>
  tag === "fr" ? 0 : tag === "en" ? 1 : 2;
export const GUIDE_LANGUAGES: readonly string[] = [...SUPPORTED_LOCALES].sort(
  (a, b) => GUIDE_FIRST(a) - GUIDE_FIRST(b),
);

/** What became of a file of the guide. */
export type GuideFileState =
  | { kind: "ready"; file: ManagedFile }
  | { kind: "kept"; file: ManagedFile }
  | { kind: "failed"; error: unknown };

export function guideFileState(file: ManagedFile): GuideFileState {
  return file.outcome === "kept_as_changed"
    ? { kind: "kept", file }
    : { kind: "ready", file };
}

/** The language's name in the language of the window (« japonais »,
 * « Japanese »), as the sentences of the window say it; the language's own
 * name when the engine does not know it. */
export function languageName(language: string, inLocale: string): string {
  try {
    return (
      new Intl.DisplayNames([inLocale], { type: "language" }).of(language) ??
      localeName(language)
    );
  } catch {
    return localeName(language);
  }
}

/** Everything one click (or one change of language) made ready. */
export interface GuideAsk {
  language: string;
  /** The Guide page in Markdown: the file's text, and « Copier le guide
   * entier ». */
  text: string;
  root: string | null;
  /** `ASSISTANT-IA.md`, as `ensureAssistantFile` says it. */
  assistantFile: AssistantFileState;
  /** `README.md`; none when it could not be made ready. */
  readme: ManagedFile | null;
  guide: GuideFileState;
  /** The prompt; none when it could not be built (no working folder). */
  prompt: string | null;
}

export interface GuideAskDeps {
  language: string;
  /** The Guide page in Markdown, in `language`; `root` is the working folder
   * when it is known. */
  render: (language: string, root: string | null) => string;
  /** `workingFolderPrepare(language)`. */
  prepare: (language: string) => Promise<{
    root: string;
    assistantFile: { outcome: string };
    readme?: ManagedFile | null;
  }>;
  /** `guideWrite`. */
  write: (
    language: string,
    text: string,
  ) => Promise<{ root: string; guide: ManagedFile }>;
  /** `guidePrompt`. */
  build: (language: string, input: GuidePromptInput) => Promise<string>;
  /** The working folder when the folder could not be prepared. */
  rootFallback: () => Promise<string | null>;
}

/** The files of the guide made ready, then the prompt built: never throws,
 * every failure is a state. The guide's failure gives the prompt without a
 * path (it asks the assistant to wait for the guide pasted). */
export async function prepareGuideAsk(deps: GuideAskDeps): Promise<GuideAsk> {
  const { language } = deps;
  let readme: ManagedFile | null = null;
  let root: string | null = null;
  const assistantFile = await ensureAssistantFile(async () => {
    const prepared = await deps.prepare(language);
    readme = prepared.readme ?? null;
    root = prepared.root;
    return prepared;
  });
  if (root === null) root = await deps.rootFallback().catch(() => null);
  const text = deps.render(language, root);
  let guide: GuideFileState;
  try {
    const written = await deps.write(language, text);
    root = written.root;
    guide = guideFileState(written.guide);
  } catch (error) {
    guide = { kind: "failed", error };
  }
  const prompt =
    root === null
      ? null
      : await deps
          .build(language, {
            root,
            guideFile: guide.kind === "failed" ? null : guide.file.name,
          })
          .catch(() => null);
  return { language, text, root, assistantFile, readme, guide, prompt };
}

/** One notice of the guide: a snackbar, with « Remplacer par la version
 * d'Astro Sync » when the person's file was kept. */
export interface GuideNotice {
  key: "guide-file" | "readme-file";
  tone: "warning" | "error";
  text: string;
  mode: "persistent";
  /** The file to replace, when the notice offers it. */
  replace: "guide" | "readme" | null;
}

/** What to say about the guide and the README: nothing when they are as they
 * should be. The guide that could not be written is said with its reason; a
 * file the person changed is kept and said. */
export function guideNotices(
  result: Pick<GuideAsk, "guide" | "readme" | "language">,
  say: Translate,
  formatError: (error: unknown) => string,
  fileName: (language: string) => string,
): GuideNotice[] {
  const notices: GuideNotice[] = [];
  if (result.guide.kind === "kept") {
    notices.push({
      key: "guide-file",
      tone: "warning",
      text: say("guide.ask.kept", { name: result.guide.file.name }),
      mode: "persistent",
      replace: "guide",
    });
  } else if (result.guide.kind === "failed") {
    notices.push({
      key: "guide-file",
      tone: "error",
      text: say("guide.ask.failed", {
        file: fileName(result.language),
        reason: formatError(result.guide.error),
      }),
      mode: "persistent",
      replace: null,
    });
  }
  if (result.readme?.outcome === "kept_as_changed") {
    notices.push({
      key: "readme-file",
      tone: "warning",
      text: say("guide.ask.kept", { name: result.readme.name }),
      mode: "persistent",
      replace: "readme",
    });
  }
  return notices;
}

/** The line of the window under the language: the guide's path, its size and
 * what became of it; none when the guide could not be written (the window
 * says so in words). */
export function guideFileLine(
  guide: GuideFileState,
  say: Translate,
  locale: string,
): string | null {
  if (guide.kind === "failed") return null;
  const { file } = guide;
  const state =
    guide.kind === "kept"
      ? "guide.ask.stateKept"
      : file.outcome === "unchanged"
        ? "guide.ask.stateCurrent"
        : "guide.ask.stateWritten";
  return say("guide.ask.file", {
    path: displayPath(file.path, homeOf(file.path)),
    size: formatSize(say, file.bytes, locale),
    state: say(state),
  });
}

/** What « Copier le guide entier » says: copied with the number of
 * characters (so the paste can be checked), or refused by the clipboard with
 * the way out (« Ouvrir le guide »). */
export function afterGuideCopy(
  copied: boolean,
  chars: number,
  say: Translate,
  locale: string,
): { tone: "success" | "warning"; text: string; offersOpen: boolean } {
  return copied
    ? {
        tone: "success",
        text: say("guide.ask.guideCopied", {
          count: new Intl.NumberFormat(locale).format(chars),
        }),
        offersOpen: false,
      }
    : {
        tone: "warning",
        text: say("guide.ask.guideRefused"),
        offersOpen: true,
      };
}

/** What the snackbar says once « Remplacer par la version d'Astro Sync » is
 * done for the guide or the README, or failed. */
export async function replaceManagedFile(
  replace: () => Promise<{ keptCopy: string | null; file: ManagedFile }>,
  name: string,
  formatError: (error: unknown) => string,
  say: Translate,
): Promise<{ tone: "success" | "error"; text: string }> {
  try {
    const { keptCopy } = await replace();
    const copy = keptCopy ? (keptCopy.split(/[\\/]/).pop() ?? keptCopy) : "";
    return {
      tone: "success",
      text: say("guide.ask.replaced", { name, copy }),
    };
  } catch (error) {
    return {
      tone: "error",
      text: say("guide.ask.replaceFailed", {
        name,
        reason: formatError(error),
      }),
    };
  }
}
