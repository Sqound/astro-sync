import { useCallback, useEffect, useId, useRef, useState } from "react";

import { Icon } from "../../components/Icon";
import { Notice } from "../../components/Notice";
import { PageTop } from "../../components/PageTop";
import { useSnackbar } from "../../components/Snackbar";
import { Waiting } from "../../components/Waiting";
import { useI18n } from "../../i18n";
import {
  imageAdjustLayer,
  imageAdjustOpen,
  imageAdjustSave,
  imageAdjustSmooth,
  imageAdjustStars,
  type AdjustOpen,
  type AdjustStars,
  type AdjustState,
  type SavedAdjustment,
} from "../../lib/bridge";
import { formatError } from "../../lib/format";
import {
  ADJUST_SPECS,
  NEUTRAL,
  adjustPixels,
  isChanged,
  isNeutral,
  valueText,
  type AdjustName,
  type AdjustSpec,
  type AdjustValues,
} from "../../lib/imageAdjust";
import { formatSize } from "../../lib/written";
import { SavedAdjustments } from "./SavedAdjustments";
import {
  badgeClassName,
  buttonClassName,
  helpTextClassName,
  inactiveButtonClassName,
} from "../../theme";

/** The pixels of a picture of the preview. */
async function pixelsOf(blob: Blob): Promise<ImageData> {
  const bitmap = await createImageBitmap(blob);
  const canvas = document.createElement("canvas");
  canvas.width = bitmap.width;
  canvas.height = bitmap.height;
  const context = canvas.getContext("2d", { willReadFrequently: true })!;
  context.drawImage(bitmap, 0, 0);
  bitmap.close();
  return context.getImageData(0, 0, canvas.width, canvas.height);
}

/** The wait before the smoothing of the preview is asked (the slider moves). */
const SMOOTH_DEBOUNCE_MS = 250;

function sameValues(a: AdjustValues, b: AdjustValues): boolean {
  return ADJUST_SPECS.every((spec) => a[spec.name] === b[spec.name]);
}

/**
 * « Régler l'image »: the preview over the whole
 * width -- « Avant » / « Après » --, the five settings in a band under it,
 * three per line, each with its value and its help, the render's value
 * marked on the track; « Réinitialiser » brings every setting back to the
 * render's. The window computes the preview (1080 px) itself, at once; the
 * smoothing alone asks the script (≈ 1 s, the star of the application says
 * so). « Enregistrer l'image réglée » computes the whole image beside the
 * render, never over it.
 */
export function AdjustScreen({
  path,
  target,
  render,
  eyebrow,
  back,
  onSaved,
}: {
  /** The PNG render. */
  path: string;
  /** The target's designation and the render's name, for the line. */
  target: string;
  render: string;
  eyebrow: string;
  /** The way back (« Retour aux images à envoyer »), also « Annuler ». */
  back: { label: string; onClick: () => void };
  onSaved: (state: AdjustState) => void;
}) {
  const { t, locale } = useI18n();
  const { notify } = useSnackbar();
  const [open, setOpen] = useState<AdjustOpen | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [values, setValues] = useState<AdjustValues>({ ...NEUTRAL });
  const [saved, setSaved] = useState<AdjustValues | null>(null);
  const [view, setView] = useState<"before" | "after">("after");
  const [stars, setStars] = useState<AdjustStars | "running" | "failed">(
    "todo",
  );
  const [smoothing, setSmoothing] = useState(false);
  const [saving, setSaving] = useState(false);
  // The saved setting last chosen: its name stays on the head while the
  // sliders hold its values.
  const [applied, setApplied] = useState<SavedAdjustment | null>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const layers = useRef<{
    base: ImageData | null;
    mask: ImageData | null;
    stars: ImageData | null;
    smoothed: Map<number, ImageData>;
  }>({ base: null, mask: null, stars: null, smoothed: new Map() });
  const [loaded, setLoaded] = useState(0);
  const lastMs = useRef<number | null>(null);

  // The preview prepared, the layers read, the image set of before reopened.
  useEffect(() => {
    let live = true;
    void (async () => {
      try {
        const answer = await imageAdjustOpen(path);
        if (!live) return;
        const [base, mask] = await Promise.all([
          imageAdjustLayer(path, "base").then(pixelsOf),
          imageAdjustLayer(path, "mask").then(pixelsOf),
        ]);
        if (!live) return;
        layers.current.base = base;
        layers.current.mask = mask;
        if (answer.stars === "ready") {
          layers.current.stars = await imageAdjustLayer(path, "stars").then(
            pixelsOf,
          );
        }
        if (!live) return;
        setOpen(answer);
        setStars(answer.stars);
        if (answer.state) {
          setValues({ ...answer.state.values });
          setSaved({ ...answer.state.values });
        }
        setLoaded((n) => n + 1);
      } catch (caught) {
        if (live) setError(formatError(t, caught));
      }
    })();
    return () => {
      live = false;
    };
    // `t` changes with the language only; the screen opens once per image.
  }, [path]);

  // The star layer, separated once per render while the screen is open. The
  // effect follows the opening only: marking the separation « running » must
  // not run it again, or its cleanup would throw the answer away and the
  // line « Séparation des étoiles… » would stay once the layer is written.
  useEffect(() => {
    if (!open || open.stars !== "todo") return;
    let live = true;
    setStars("running");
    void imageAdjustStars(path)
      .then(async (answer) => {
        if (answer.stars === "ready") {
          layers.current.stars = await imageAdjustLayer(path, "stars").then(
            pixelsOf,
          );
        }
        if (live) {
          setStars(answer.stars);
          setLoaded((n) => n + 1);
        }
      })
      .catch(() => {
        if (live) setStars("failed");
      });
    return () => {
      live = false;
    };
  }, [open, path]);

  // The smoothed preview for the amount chosen, asked once the slider rests.
  useEffect(() => {
    const amount = values.smoothing;
    if (amount === 0 || layers.current.smoothed.has(amount) || !open) {
      setSmoothing(false);
      return;
    }
    let live = true;
    setSmoothing(true);
    const timer = window.setTimeout(() => {
      void imageAdjustSmooth(path, amount)
        .then(pixelsOf)
        .then((pixels) => {
          layers.current.smoothed.set(amount, pixels);
          if (live) {
            setSmoothing(false);
            setLoaded((n) => n + 1);
          }
        })
        .catch((caught) => {
          if (live) {
            setSmoothing(false);
            setError(formatError(t, caught));
          }
        });
    }, SMOOTH_DEBOUNCE_MS);
    return () => {
      live = false;
      window.clearTimeout(timer);
    };
  }, [values.smoothing, open, path]);

  // The preview drawn: the render (« Avant »), or the settings on it.
  useEffect(() => {
    const target = canvas.current;
    const { base, mask } = layers.current;
    if (!target || !base || !mask) return;
    const started = performance.now();
    const source =
      values.smoothing > 0
        ? (layers.current.smoothed.get(values.smoothing) ??
          // Still smoothing: the last picture stays under the veil.
          null)
        : base;
    if (!source) return;
    target.width = base.width;
    target.height = base.height;
    const context = target.getContext("2d")!;
    if (view === "before") {
      context.putImageData(base, 0, 0);
      return;
    }
    const out = context.createImageData(base.width, base.height);
    adjustPixels(
      source.data,
      layers.current.stars?.data ?? null,
      mask.data,
      values,
      out.data,
    );
    context.putImageData(out, 0, 0);
    lastMs.current = Math.round(performance.now() - started);
    target.dataset.previewMs = String(lastMs.current);
  }, [values, view, loaded]);

  const change = useCallback((name: AdjustName, value: number) => {
    setValues((current) => ({ ...current, [name]: value }));
    setView("after");
  }, []);

  // A saved setting only moves the sliders of the preview; nothing is
  // written before « Enregistrer l'image réglée ».
  const apply = (saved: SavedAdjustment) => {
    const before = values;
    const appliedBefore = applied;
    setValues({ ...saved.values });
    setApplied(saved);
    setView("after");
    notify({
      tone: "success",
      text: t("adjust.saved.applied", { name: saved.name }),
      action: {
        label: t("adjust.undo"),
        run: () => {
          setValues(before);
          setApplied(appliedBefore);
        },
      },
    });
  };

  const reset = () => {
    const before = values;
    setValues({ ...NEUTRAL });
    notify({
      tone: "success",
      text: t("adjust.reset.done"),
      action: { label: t("adjust.undo"), run: () => setValues(before) },
    });
  };

  const save = () => {
    setSaving(true);
    setError(null);
    void imageAdjustSave(path, values)
      .then((state) => {
        setSaving(false);
        onSaved(state);
      })
      .catch((caught) => {
        setSaving(false);
        setError(formatError(t, caught));
      });
  };

  const neutral = isNeutral(values);
  const unchanged = neutral || (saved !== null && sameValues(values, saved));
  const canSave = !!open && !unchanged && !smoothing && !saving;
  const badge =
    view === "before"
      ? t("adjust.badge.before")
      : neutral
        ? t("adjust.badge.unchanged")
        : t("adjust.badge.after");

  return (
    <div className="as-page as-adjust">
      <PageTop>
        <button
          type="button"
          className="as-button as-button--link as-back"
          onClick={back.onClick}
        >
          <Icon name="back" className="as-button__icon" />
          {back.label}
        </button>
        <p className="as-page__eyebrow">{eyebrow}</p>
        <h1 className="as-page__title">{t("adjust.title")}</h1>
        <p className={helpTextClassName}>
          {t("adjust.line", {
            target,
            render,
            format: "PNG",
            size: open ? formatSize(t, open.size, locale) : "…",
          })}
        </p>
      </PageTop>
      <section className="as-card as-adjust__zone">
        <div className="as-adjust__bar">
          <div className="as-segments" role="group">
            {(["before", "after"] as const).map((item) => (
              <button
                key={item}
                type="button"
                className={
                  view === item ? "as-segment is-chosen" : "as-segment"
                }
                aria-pressed={view === item}
                onClick={() => setView(item)}
              >
                {t(item === "before" ? "adjust.before" : "adjust.after")}
              </button>
            ))}
          </div>
          <span className={helpTextClassName}>{t("adjust.previewNote")}</span>
        </div>
        <div className="as-adjust__preview">
          {open ? (
            <>
              <canvas
                ref={canvas}
                className="as-adjust__canvas"
                aria-label={badge}
              />
              <span className="as-adjust__pill">{badge}</span>
              {smoothing && view === "after" && (
                <div className="as-adjust__veil">
                  <Waiting
                    size="inline"
                    label={t("adjust.smoothing.computing")}
                  />
                </div>
              )}
            </>
          ) : (
            !error && <Waiting label={t("adjust.wait.open")} />
          )}
        </div>
        <div className="as-adjust__panel">
          <div className="as-adjust__panel-head">
            <h2 className="as-adjust__panel-title">{t("adjust.panel")}</h2>
            <span className={badgeClassName(neutral ? "outline" : "neutral")}>
              {neutral ? t("adjust.panel.asRender") : t("adjust.panel.changed")}
            </span>
            {applied && sameValues(values, applied.values) && (
              <span className="as-adjust__applied">
                {t("adjust.saved.onPreview", { name: applied.name })}
              </span>
            )}
            <span className="as-wizard-foot__gap" />
            <SavedAdjustments
              values={values}
              neutral={neutral}
              ready={!!open}
              onApply={apply}
            />
            <button
              type="button"
              className={buttonClassName("link")}
              disabled={neutral || !open}
              onClick={reset}
            >
              {t("adjust.reset")}
            </button>
          </div>
          <div className="as-adjust__sliders">
            {ADJUST_SPECS.map((spec) => (
              <AdjustSlider
                key={spec.name}
                spec={spec}
                value={values[spec.name]}
                disabled={
                  !open ||
                  (spec.name === "stars" &&
                    stars !== "ready" &&
                    values.stars === 0)
                }
                note={spec.name === "stars" ? starsNote(stars, t) : null}
                onChange={(value) => change(spec.name, value)}
                text={valueText(t, locale, spec.name, values[spec.name])}
              />
            ))}
          </div>
        </div>
      </section>
      <Notice tone="error">{error}</Notice>
      <div className="as-step-foot as-wizard-foot">
        <div className="as-wizard-foot__message">
          {saving ? (
            <Waiting size="inline" label={t("adjust.saving")} />
          ) : (
            <p className={helpTextClassName} role="status">
              {neutral ? t("adjust.foot.unchanged") : t("adjust.foot.kept")}
            </p>
          )}
        </div>
        <div className="as-wizard-foot__row">
          <button
            type="button"
            className={`${buttonClassName("link")} as-wizard-foot__previous`}
            onClick={back.onClick}
          >
            {t("common.cancel")}
          </button>
          <span className="as-wizard-foot__gap" />
          <button
            type="button"
            className={`${canSave ? buttonClassName("primary") : inactiveButtonClassName} as-adjust__save`}
            disabled={!canSave}
            onClick={save}
          >
            {t("adjust.save")}
          </button>
        </div>
      </div>
    </div>
  );
}

type Translate = ReturnType<typeof useI18n>["t"];

/** What « Étoiles » says when its layer is not there. */
function starsNote(
  stars: AdjustStars | "running" | "failed",
  t: Translate,
): { waiting: boolean; text: string } | null {
  switch (stars) {
    case "running":
    case "todo":
      return { waiting: true, text: t("adjust.stars.waiting") };
    case "missing":
      return { waiting: false, text: t("errors.stacking.starnet_missing") };
    case "failed":
      return { waiting: false, text: t("errors.stacking.starnet_failed") };
    case "small":
      return { waiting: false, text: t("adjust.stars.small") };
    case "ready":
      return null;
  }
}

/** One setting: its title (and « Modifié »), the track with the render's
 * value marked, the value, the help. */
function AdjustSlider({
  spec,
  value,
  text,
  disabled,
  note,
  onChange,
}: {
  spec: AdjustSpec;
  value: number;
  text: string;
  disabled: boolean;
  note: { waiting: boolean; text: string } | null;
  onChange: (value: number) => void;
}) {
  const { t } = useI18n();
  const id = useId();
  const changed = isChanged({ ...NEUTRAL, [spec.name]: value }, spec.name);
  const mark = ((NEUTRAL[spec.name] - spec.min) / (spec.max - spec.min)) * 100;
  return (
    <div className={`as-adjust__slider${changed ? " is-changed" : ""}`}>
      <div className="as-adjust__slider-head">
        <label htmlFor={id} className="as-adjust__slider-title">
          {t(spec.titleKey)}
        </label>
        {changed && (
          <span className={badgeClassName("neutral")}>
            {t("adjust.slider.changed")}
          </span>
        )}
      </div>
      <div className="as-adjust__track">
        <input
          id={id}
          type="range"
          className="as-contrast__range as-adjust__range"
          min={spec.min}
          max={spec.max}
          step={spec.step}
          value={value}
          disabled={disabled}
          aria-valuetext={text}
          aria-describedby={`${id}-help`}
          onChange={(event) => onChange(Number(event.target.value))}
        />
        <span
          className="as-adjust__mark"
          style={{ left: `${mark}%` }}
          aria-hidden="true"
        />
      </div>
      <span className="as-adjust__value">{text}</span>
      <p id={`${id}-help`} className={`${helpTextClassName} as-adjust__help`}>
        {t(spec.helpKey)}
      </p>
      {note &&
        (note.waiting ? (
          <Waiting size="inline" label={note.text} />
        ) : (
          <p className={`${helpTextClassName} as-adjust__note`}>{note.text}</p>
        ))}
    </div>
  );
}
