import { useCallback, useRef } from "react";

import { useI18n } from "../i18n";
import { workingFolderReplaceAssistantFile } from "../lib/bridge";
import { formatError } from "../lib/format";
import {
  assistantFileSnack,
  replaceAssistantFile,
  type AssistantFileState,
} from "../lib/promptWindow";
import { useSnackbar } from "./Snackbar";

/** Says what became of `ASSISTANT-IA.md`: nothing when it is as it should be;
 * a persistent warning with « Remplacer par la version d'Astro Sync » when the
 * person changed it; an error in words when it could not be written. The
 * returned function is stable (the words are read at call time), so an effect
 * can use it without running again on a language change. */
export function useAssistantFileNotice(): (state: AssistantFileState) => void {
  const { t } = useI18n();
  const { notify } = useSnackbar();
  const latest = useRef({ t, notify });
  latest.current = { t, notify };
  return useCallback((state: AssistantFileState) => {
    const { t: say, notify: tell } = latest.current;
    const snack = assistantFileSnack(
      state,
      (error) => formatError(say, error),
      say,
    );
    if (!snack) return;
    tell({
      key: "assistant-file",
      tone: snack.tone,
      text: snack.text,
      mode: snack.mode,
      action: snack.replaceable
        ? {
            label: say("assistantFile.replace"),
            run: () => {
              void replaceAssistantFile(
                workingFolderReplaceAssistantFile,
                (error) => formatError(say, error),
                say,
              ).then((done) =>
                latest.current.notify({ key: "assistant-file", ...done }),
              );
            },
          }
        : undefined,
    });
  }, []);
}
