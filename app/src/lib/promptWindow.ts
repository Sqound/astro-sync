// « Demander à un assistant IA »: the prompt is copied and said in a
// snackbar with « Voir le prompt », which opens it in a window of the app --
// never laid out under the step. Pure TypeScript, tested under
// `node --test`; the prompt itself is the core's (`assistant_prompt`), shown
// as it comes.

import type { NightOnline } from "./bridge";

/** What the window says after « Demander à un assistant IA »: copied (a
 * success, its link to see the prompt), or refused by the clipboard (a
 * warning, and the window opens at once so the prompt can be copied by
 * hand). */
export function afterCopy(copied: boolean): {
  tone: "success" | "warning";
  textKey: string;
  openAtOnce: boolean;
} {
  return copied
    ? {
        tone: "success",
        textKey: "wizard.folder.promptCopied",
        openAtOnce: false,
      }
    : {
        tone: "warning",
        textKey: "wizard.folder.copyRefused",
        openAtOnce: true,
      };
}

/** The common names the night knows online, by target folder (`M31` →
 * `Galaxie d'Andromède`), for the prompt's `names`: only a name that says
 * more than the folder's own. */
export function promptNames(
  targets: readonly { targetId: string; folderName: string }[],
  online: Pick<NightOnline, "targets"> | null,
): Record<string, string> {
  const names: Record<string, string> = {};
  for (const target of targets) {
    const name = online?.targets
      .find((item) => item.targetId === target.targetId)
      ?.name?.trim();
    if (name && name !== target.folderName) names[target.folderName] = name;
  }
  return names;
}

/** The keys that close the window. */
export function closesWindow(key: string): boolean {
  return key === "Escape";
}

/** The file `ASSISTANT-IA.md` the prompt refers to. */
export type AssistantFileState =
  { kind: "ready" } | { kind: "kept" } | { kind: "failed"; error: unknown };

/** Makes the file ready through `prepare` (the core's `working_folder_prepare`):
 * never throws, the failure is a state. */
export async function ensureAssistantFile(
  prepare: () => Promise<{ assistantFile: { outcome: string } }>,
): Promise<AssistantFileState> {
  try {
    const prepared = await prepare();
    return prepared.assistantFile.outcome === "kept_as_changed"
      ? { kind: "kept" }
      : { kind: "ready" };
  } catch (error) {
    return { kind: "failed", error };
  }
}

/** What to say about the file: nothing when it is as it should be. */
export function assistantFileSnack(
  state: AssistantFileState,
  formatError: (error: unknown) => string,
  say: (key: string, params?: Record<string, string>) => string,
): {
  tone: "warning" | "error";
  text: string;
  mode: "persistent";
  /** Offers « Remplacer par la version d'Astro Sync ». */
  replaceable: boolean;
} | null {
  if (state.kind === "kept") {
    return {
      tone: "warning",
      text: say("assistantFile.kept"),
      mode: "persistent",
      replaceable: true,
    };
  }
  if (state.kind === "failed") {
    return {
      tone: "error",
      text: say("assistantFile.failed", { reason: formatError(state.error) }),
      mode: "persistent",
      replaceable: false,
    };
  }
  return null;
}

/** « Demander à un assistant IA »: the file is made ready first, so that the
 * file the prompt cites is the one on disk; whatever happens to it, the
 * prompt is built and copied. */
export async function askAssistant(deps: {
  prepare: () => Promise<{ assistantFile: { outcome: string } }>;
  build: () => Promise<string | null>;
  copy: (text: string) => Promise<boolean>;
  onFile: (state: AssistantFileState) => void;
  onCopied: (prompt: string, copied: boolean) => void;
}): Promise<void> {
  deps.onFile(await ensureAssistantFile(deps.prepare));
  const prompt = await deps.build().catch(() => null);
  if (!prompt) return;
  deps.onCopied(prompt, await deps.copy(prompt));
}

/** At startup: prepare when the folder is known (it exists or was chosen), or
 * once the person is connected (the first connected launch creates it). */
export function shouldPrepareAtStartup(
  folder: { exists: boolean; chosen: boolean },
  paired: boolean,
): boolean {
  return folder.exists || folder.chosen || paired;
}

/** What the snackbar says once « Remplacer par la version d'Astro Sync » is
 * done (the copy's name) or failed. */
export async function replaceAssistantFile(
  replace: () => Promise<{ keptCopy: string | null }>,
  formatError: (error: unknown) => string,
  say: (key: string, params?: Record<string, string>) => string,
): Promise<{ tone: "success" | "error"; text: string }> {
  try {
    const { keptCopy } = await replace();
    const name = keptCopy ? (keptCopy.split(/[\\/]/).pop() ?? keptCopy) : "";
    return {
      tone: "success",
      text: say("assistantFile.replaced", { name }),
    };
  } catch (error) {
    return {
      tone: "error",
      text: say("assistantFile.replaceFailed", { reason: formatError(error) }),
    };
  }
}
