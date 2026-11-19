import { useEffect, useId, useRef, useState } from "react";

import { useSnackbar } from "../../components/Snackbar";
import { useI18n } from "../../i18n";
import {
  imageAdjustSavedExport,
  imageAdjustSavedImport,
  imageAdjustSavedList,
  imageAdjustSavedRemove,
  imageAdjustSavedRename,
  imageAdjustSavedSave,
  isCommandError,
  type SavedAdjustment,
} from "../../lib/bridge";
import { formatError } from "../../lib/format";
import {
  adjustedList,
  importText,
  type AdjustValues,
} from "../../lib/imageAdjust";
import { buttonClassName, helpTextClassName } from "../../theme";

type Translate = ReturnType<typeof useI18n>["t"];

/**
 * The saved settings in the head of « Réglages » (P1): « Appliquer un réglage
 * enregistré » (tertiary) opens the list -- each setting with its values,
 * « Renommer » and « Supprimer » on its line, « Exporter… » / « Importer… »
 * at the bottom --; « Enregistrer ce réglage » (link) opens the field of its
 * name. Choosing a setting only moves the sliders of the preview (the
 * screen's `onApply`); nothing is written before « Enregistrer l'image
 * réglée ». No setting is ever applied by itself.
 */
export function SavedAdjustments({
  values,
  neutral,
  ready,
  onApply,
}: {
  values: AdjustValues;
  neutral: boolean;
  ready: boolean;
  onApply: (saved: SavedAdjustment) => void;
}) {
  const { t, locale } = useI18n();
  const { notify } = useSnackbar();
  const [list, setList] = useState<SavedAdjustment[] | null>(null);
  const [mode, setMode] = useState<"closed" | "naming" | "list">("closed");
  const [name, setName] = useState("");
  const [nameError, setNameError] = useState<string | null>(null);
  const [renaming, setRenaming] = useState<{
    from: string;
    to: string;
    error: string | null;
  } | null>(null);
  const [removing, setRemoving] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const nameId = useId();
  const listId = useId();
  const nameField = useRef<HTMLInputElement>(null);

  useEffect(() => {
    let live = true;
    void imageAdjustSavedList()
      .then((saved) => {
        if (live) setList(saved);
      })
      .catch(() => {
        if (live) setList([]);
      });
    return () => {
      live = false;
    };
  }, []);

  useEffect(() => {
    if (mode === "naming") nameField.current?.focus();
  }, [mode]);

  const empty = list !== null && list.length === 0;
  const closeList = () => {
    setMode("closed");
    setRenaming(null);
    setRemoving(null);
    setError(null);
  };

  // A name already taken is said under the field; any other refusal too.
  const refusal = (caught: unknown): string =>
    isCommandError(caught) && caught.code === "adjust_name_taken"
      ? t("errors.adjust_name_taken")
      : formatError(t, caught);

  const save = () => {
    const chosen = name.trim();
    if (!chosen) return;
    setBusy(true);
    void imageAdjustSavedSave(chosen, values)
      .then((saved) => {
        setBusy(false);
        setList(saved);
        setMode("closed");
        setName("");
        setNameError(null);
        notify({
          tone: "success",
          text: t("adjust.saved.saved", { name: chosen }),
        });
      })
      .catch((caught) => {
        setBusy(false);
        setNameError(refusal(caught));
      });
  };

  const rename = () => {
    if (!renaming || !renaming.to.trim()) return;
    setBusy(true);
    void imageAdjustSavedRename(renaming.from, renaming.to.trim())
      .then((saved) => {
        setBusy(false);
        setList(saved);
        setRenaming(null);
      })
      .catch((caught) => {
        setBusy(false);
        setRenaming((current) =>
          current ? { ...current, error: refusal(caught) } : current,
        );
      });
  };

  const remove = (target: string) => {
    setBusy(true);
    void imageAdjustSavedRemove(target)
      .then((saved) => {
        setBusy(false);
        setList(saved);
        setRemoving(null);
        if (saved.length === 0) closeList();
      })
      .catch((caught) => {
        setBusy(false);
        setError(formatError(t, caught));
      });
  };

  const exportAll = () => {
    setError(null);
    void imageAdjustSavedExport(t("adjust.saved.fileName"))
      .then((file) => {
        if (file)
          notify({
            tone: "success",
            text: t("adjust.saved.exported", { file }),
          });
      })
      .catch((caught) => setError(formatError(t, caught)));
  };

  const importFile = () => {
    setError(null);
    void imageAdjustSavedImport()
      .then((answer) => {
        if (!answer) return;
        setList(answer.list);
        notify({
          tone: answer.added.length > 0 ? "success" : "info",
          text: importText(t, locale, answer),
        });
      })
      .catch((caught) => setError(formatError(t, caught)));
  };

  return (
    <>
      <div className="as-adjust__saved-actions">
        <button
          type="button"
          className={buttonClassName("tertiary")}
          disabled={!ready || !list || list.length === 0}
          aria-expanded={mode === "list"}
          aria-controls={listId}
          onClick={() => (mode === "list" ? closeList() : setMode("list"))}
        >
          {t("adjust.saved.apply")}
        </button>
        <button
          type="button"
          className={buttonClassName("link")}
          disabled={!ready || neutral}
          aria-expanded={mode === "naming"}
          onClick={() => {
            closeList();
            setNameError(null);
            setMode(mode === "naming" ? "closed" : "naming");
          }}
        >
          {t("adjust.saved.save")}
        </button>
      </div>
      {mode === "list" && list && list.length > 0 && (
        <div
          id={listId}
          className="as-adjust__saved-list"
          role="region"
          aria-label={t("adjust.saved.title")}
        >
          <h3 className="as-adjust__saved-title">{t("adjust.saved.title")}</h3>
          <ul className="as-adjust__saved-items">
            {list.map((item) => (
              <li key={item.name} className="as-adjust__saved-item">
                {renaming?.from === item.name ? (
                  <div className="as-adjust__saved-edit">
                    <div className="as-adjust__saved-row">
                      <input
                        type="text"
                        className="as-field as-adjust__saved-field"
                        value={renaming.to}
                        maxLength={60}
                        aria-label={t("adjust.saved.nameLabel")}
                        aria-invalid={renaming.error ? true : undefined}
                        autoFocus
                        onChange={(event) =>
                          setRenaming({
                            ...renaming,
                            to: event.target.value,
                            error: null,
                          })
                        }
                        onKeyDown={(event) => {
                          if (event.key === "Enter") rename();
                        }}
                      />
                      <button
                        type="button"
                        className={buttonClassName("secondary")}
                        disabled={busy || !renaming.to.trim()}
                        onClick={rename}
                      >
                        {t("adjust.saved.rename")}
                      </button>
                      <button
                        type="button"
                        className={buttonClassName("link")}
                        onClick={() => setRenaming(null)}
                      >
                        {t("common.cancel")}
                      </button>
                    </div>
                    {renaming.error && (
                      <p
                        className={`${helpTextClassName} as-adjust__saved-error as-selectable`}
                      >
                        {renaming.error}
                      </p>
                    )}
                  </div>
                ) : removing === item.name ? (
                  <div className="as-adjust__saved-row">
                    <div className="as-adjust__saved-text">
                      <strong className="as-adjust__saved-ask">
                        {t("adjust.saved.removeAsk", { name: item.name })}
                      </strong>
                      <span className={helpTextClassName}>
                        {t("adjust.saved.removeKeeps")}
                      </span>
                    </div>
                    <button
                      type="button"
                      className={buttonClassName("tertiary", {
                        destructive: true,
                      })}
                      disabled={busy}
                      onClick={() => remove(item.name)}
                    >
                      {t("adjust.saved.remove")}
                    </button>
                    <button
                      type="button"
                      className={buttonClassName("link")}
                      onClick={() => setRemoving(null)}
                    >
                      {t("common.cancel")}
                    </button>
                  </div>
                ) : (
                  <div className="as-adjust__saved-row">
                    <div className="as-adjust__saved-text">
                      <button
                        type="button"
                        className={`${buttonClassName("link")} as-adjust__saved-name`}
                        onClick={() => {
                          closeList();
                          onApply(item);
                        }}
                      >
                        {item.name}
                      </button>
                      <span className={helpTextClassName}>
                        {adjustedList(t, locale, item.values)}
                      </span>
                    </div>
                    <button
                      type="button"
                      className={buttonClassName("link")}
                      onClick={() => {
                        setRemoving(null);
                        setRenaming({
                          from: item.name,
                          to: item.name,
                          error: null,
                        });
                      }}
                    >
                      {t("adjust.saved.rename")}
                    </button>
                    <button
                      type="button"
                      className={buttonClassName("link", { destructive: true })}
                      onClick={() => {
                        setRenaming(null);
                        setRemoving(item.name);
                      }}
                    >
                      {t("adjust.saved.remove")}
                    </button>
                  </div>
                )}
              </li>
            ))}
          </ul>
          <p className={helpTextClassName}>{t("adjust.saved.listHelp")}</p>
          {error && (
            <p
              className={`${helpTextClassName} as-adjust__saved-error as-selectable`}
            >
              {error}
            </p>
          )}
          <div className="as-adjust__saved-row as-adjust__saved-foot">
            <button
              type="button"
              className={buttonClassName("tertiary")}
              onClick={exportAll}
            >
              {t("adjust.saved.export")}
            </button>
            <button
              type="button"
              className={buttonClassName("link")}
              onClick={importFile}
            >
              {t("adjust.saved.import")}
            </button>
          </div>
        </div>
      )}
      <SavedBelow
        t={t}
        mode={mode}
        empty={empty}
        name={name}
        nameId={nameId}
        nameField={nameField}
        nameError={nameError}
        busy={busy}
        error={mode === "list" ? null : error}
        valuesText={adjustedList(t, locale, values)}
        onName={(next) => {
          setName(next);
          setNameError(null);
        }}
        onSave={save}
        onCancel={() => {
          setMode("closed");
          setNameError(null);
        }}
        onImport={importFile}
      />
    </>
  );
}

/** Under the head: the field of the name (P1-b), or, with nothing saved yet,
 * the sentence that says what a saved setting is for (P1-a). */
function SavedBelow({
  t,
  mode,
  empty,
  name,
  nameId,
  nameField,
  nameError,
  busy,
  error,
  valuesText,
  onName,
  onSave,
  onCancel,
  onImport,
}: {
  t: Translate;
  mode: "closed" | "naming" | "list";
  empty: boolean;
  name: string;
  nameId: string;
  nameField: React.RefObject<HTMLInputElement | null>;
  nameError: string | null;
  busy: boolean;
  error: string | null;
  valuesText: string;
  onName: (name: string) => void;
  onSave: () => void;
  onCancel: () => void;
  onImport: () => void;
}) {
  if (mode === "naming") {
    return (
      <div className="as-adjust__saved-box as-adjust__below">
        <label htmlFor={nameId} className="as-adjust__saved-label">
          {t("adjust.saved.nameLabel")}
        </label>
        <div className="as-adjust__saved-row">
          <input
            id={nameId}
            ref={nameField}
            type="text"
            className="as-field as-adjust__saved-field"
            value={name}
            maxLength={60}
            aria-invalid={nameError ? true : undefined}
            aria-describedby={`${nameId}-help`}
            onChange={(event) => onName(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") onSave();
            }}
          />
          <button
            type="button"
            className={buttonClassName("secondary")}
            disabled={busy || !name.trim()}
            onClick={onSave}
          >
            {t("adjust.saved.confirmSave")}
          </button>
          <button
            type="button"
            className={buttonClassName("link")}
            onClick={onCancel}
          >
            {t("common.cancel")}
          </button>
        </div>
        <p
          id={`${nameId}-help`}
          className={`${helpTextClassName}${nameError ? " as-adjust__saved-error as-selectable" : ""}`}
        >
          {nameError ?? t("adjust.saved.values", { list: valuesText })}
        </p>
      </div>
    );
  }
  if (empty && mode === "closed") {
    // With nothing saved, « Importer… » stays at hand (a new computer).
    return (
      <div className="as-adjust__saved-empty as-adjust__below">
        <p className={helpTextClassName}>{t("adjust.saved.empty")}</p>
        <button
          type="button"
          className={buttonClassName("link")}
          onClick={onImport}
        >
          {t("adjust.saved.import")}
        </button>
        {error && (
          <p
            className={`${helpTextClassName} as-adjust__saved-error as-selectable`}
          >
            {error}
          </p>
        )}
      </div>
    );
  }
  return null;
}
