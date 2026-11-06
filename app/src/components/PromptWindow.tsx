import {
  useCallback,
  useEffect,
  useId,
  useRef,
  useState,
  type ReactNode,
} from "react";

import { useI18n } from "../i18n";
import { workingFolderPrepare } from "../lib/bridge";
import { afterCopy, askAssistant, closesWindow } from "../lib/promptWindow";
import { buttonClassName, helpTextClassName } from "../theme";
import { copyText } from "../screens/night/parts";
import { Icon } from "./Icon";
import { useSnackbar } from "./Snackbar";
import { useAssistantFileNotice } from "./useAssistantFileNotice";

/**
 * The prompt for an AI assistant in a window of the app: its title, what
 * it contains, the prompt in a read-only field that can be selected,
 * « Copier le prompt » (the action of the window), and its cross
 * « Fermer » (or Échap). The prompt is shown as the core wrote it.
 */
export function PromptWindow({
  prompt,
  onClose,
  title,
  hideContains = false,
  beforePrompt,
  promptLabel,
  extraActions,
  footer,
  selectAtOpen = false,
  onCopied,
}: {
  prompt: string;
  onClose: () => void;
  /** The window's title (« Poser une question à une IA »); the prompt's own
   * by default. */
  title?: string;
  /** Leaves out « Ce que le prompt contient » (a sentence about a night). */
  hideContains?: boolean;
  /** What sits between the sentence and the prompt (the language and the
   * guide's file). */
  beforePrompt?: ReactNode;
  /** The label above the prompt (« Le prompt (en français) »). */
  promptLabel?: string;
  /** Buttons after « Copier le prompt » (« Copier le guide entier »). */
  extraActions?: ReactNode;
  /** What sits under the buttons. */
  footer?: ReactNode;
  /** The prompt selected when the window opens (the clipboard refused). */
  selectAtOpen?: boolean;
  /** « Copier le prompt » was pressed: whether the clipboard accepted. */
  onCopied?: (copied: boolean) => void;
}) {
  const { t } = useI18n();
  const { notify } = useSnackbar();
  const titleId = `as-prompt-title-${useId().replace(/[^A-Za-z0-9_-]/g, "")}`;
  const area = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    area.current?.focus();
    if (selectAtOpen) area.current?.select();
    const onKey = (event: KeyboardEvent) => {
      if (closesWindow(event.key)) {
        event.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // `selectAtOpen` is read once, when the window opens.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [onClose]);

  async function copy() {
    const copied = await copyText(prompt);
    notify(
      copied
        ? { tone: "success", text: t("wizard.folder.promptCopied") }
        : { tone: "warning", text: t("wizard.folder.copyRefused") },
    );
    if (!copied) area.current?.select();
    onCopied?.(copied);
  }

  return (
    <div className="as-dialog__scrim" role="presentation" onClick={onClose}>
      <div
        className="as-dialog as-prompt-window"
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        onClick={(event) => event.stopPropagation()}
      >
        <div className="as-prompt-window__head">
          <h2 id={titleId} className="as-dialog__title">
            {title ?? t("wizard.folder.promptTitle")}
          </h2>
          <button
            type="button"
            className="as-snackbar__close"
            aria-label={t("common.close")}
            title={t("common.close")}
            onClick={onClose}
          >
            <Icon name="close" className="as-snackbar__close-icon" />
          </button>
        </div>
        <p className={helpTextClassName}>{t("wizard.folder.promptGiveTo")}</p>
        {!hideContains && (
          <p className={helpTextClassName}>
            {t("wizard.folder.promptContains")}
          </p>
        )}
        {beforePrompt}
        {promptLabel && (
          <label
            className="as-prompt-window__label"
            htmlFor={`${titleId}-text`}
          >
            {promptLabel}
          </label>
        )}
        <textarea
          id={`${titleId}-text`}
          ref={area}
          className="as-field as-prompt-window__text"
          readOnly
          value={prompt}
          aria-label={promptLabel ?? title ?? t("wizard.folder.promptTitle")}
          spellCheck={false}
        />
        <div className="as-actions">
          <button
            type="button"
            className={buttonClassName("primary")}
            onClick={() => void copy()}
          >
            <Icon name="copy" className="as-button__icon" />
            {t("wizard.folder.copyPrompt")}
          </button>
          {extraActions}
        </div>
        {footer}
      </div>
    </div>
  );
}

/** « Demander à un assistant IA »: asks the core for the prompt, copies it,
 * says so in a snackbar with « Voir le prompt » -- or, when the clipboard
 * refuses, opens the window at once. `window` is the element to render. */
export function usePromptWindow(): {
  ask: (build: () => Promise<string | null>) => Promise<void>;
  window: React.ReactNode;
} {
  const { t, locale } = useI18n();
  const { notify } = useSnackbar();
  const noticeAssistantFile = useAssistantFileNotice();
  const [shown, setShown] = useState<string | null>(null);
  const close = useCallback(() => setShown(null), []);
  const ask = useCallback(
    async (build: () => Promise<string | null>) => {
      await askAssistant({
        // The folder's `README.md` is in the application's language.
        prepare: () => workingFolderPrepare(locale),
        build,
        copy: copyText,
        onFile: noticeAssistantFile,
        onCopied: (prompt, copied) => {
          const said = afterCopy(copied);
          notify({
            key: "assistant-prompt",
            tone: said.tone,
            text: t(said.textKey),
            action: {
              label: t("wizard.folder.viewPrompt"),
              run: () => setShown(prompt),
            },
          });
          if (said.openAtOnce) setShown(prompt);
        },
      });
    },
    [notify, t, locale, noticeAssistantFile],
  );
  return {
    ask,
    window: shown ? <PromptWindow prompt={shown} onClose={close} /> : null,
  };
}
