import {
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
  type ReactNode,
  type RefObject,
} from "react";

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
  imageAdjustSpots,
  imageAdjustStars,
  type AdjustOpen,
  type AdjustStars,
  type AdjustState,
  type SavedAdjustment,
} from "../../lib/bridge";
import { fitSize, type Size } from "../../lib/adjustLayout";
import {
  CURTAIN_START,
  curtainFromKey,
  curtainFromPointer,
  curtainToggle,
  labelVisible,
} from "../../lib/compareCurtain";
import { formatError } from "../../lib/format";
import {
  spikeLayer,
  starCount,
  type SpikeStars,
} from "../../lib/diffractionSpikes";
import {
  ADJUST_SPECS,
  NEUTRAL,
  adjustPixels,
  canonicalValues,
  isChanged,
  isNeutral,
  reachText,
  spikesGroupText,
  valueText,
  type AdjustName,
  type AdjustSpec,
  type AdjustValues,
} from "../../lib/imageAdjust";
import {
  cleanTurns,
  turnBy,
  turnMatrix,
  turnStateKey,
  turnedSize,
  type Turns,
} from "../../lib/quarterTurns";
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

/** The size of the box an element fills, followed as the window changes. */
function useBoxSize(element: RefObject<HTMLElement | null>): Size {
  const [size, setSize] = useState<Size>({ width: 0, height: 0 });
  useLayoutEffect(() => {
    const node = element.current;
    if (!node) return;
    const measure = () =>
      setSize((now) =>
        now.width === node.clientWidth && now.height === node.clientHeight
          ? now
          : { width: node.clientWidth, height: node.clientHeight },
      );
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const watcher = new ResizeObserver(measure);
    watcher.observe(node);
    return () => watcher.disconnect();
  }, [element]);
  return size;
}

/** `image` drawn on `target` turned by `turns` quarter turns clockwise: the
 * canvas takes the turned size, the pixels are only moved. */
function paintTurned(
  target: HTMLCanvasElement,
  image: ImageData,
  turns: Turns,
): void {
  const scratch = document.createElement("canvas");
  scratch.width = image.width;
  scratch.height = image.height;
  scratch.getContext("2d")!.putImageData(image, 0, 0);
  const size = turnedSize(turns, image.width, image.height);
  target.width = size.width;
  target.height = size.height;
  const context = target.getContext("2d")!;
  context.setTransform(...turnMatrix(turns, image.width, image.height));
  context.drawImage(scratch, 0, 0);
}

/** The wait before the smoothing of the preview is asked (the slider moves). */
const SMOOTH_DEBOUNCE_MS = 250;

function sameValues(a: AdjustValues, b: AdjustValues): boolean {
  return ADJUST_SPECS.every((spec) => a[spec.name] === b[spec.name]);
}

/**
 * « Régler l'image »: two columns -- the preview at the largest size the
 * window leaves it, never cropped, and the panel of the settings that
 * scrolls by itself, its foot (« Annuler », « Enregistrer l'image réglée »)
 * always in sight --, each setting with its value and its help, the render's
 * value marked on the track; « Réinitialiser » brings every setting back to
 * the render's. In a narrow window the panel goes under the image. The window computes the preview (1080 px) itself, at once; the
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
  // The quarter turns, clockwise, of the image to write, and those of the
  // image set already saved.
  const [turns, setTurns] = useState<Turns>(0);
  const [savedTurns, setSavedTurns] = useState<Turns>(0);
  // The share of the image, from the left, that shows « Avant ».
  const [curtain, setCurtain] = useState(CURTAIN_START);
  const [stars, setStars] = useState<AdjustStars | "running" | "failed">(
    "todo",
  );
  const [smoothing, setSmoothing] = useState(false);
  const [saving, setSaving] = useState(false);
  // The saved setting last chosen: its name stays on the head while the
  // sliders hold its values.
  const [applied, setApplied] = useState<SavedAdjustment | null>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const beforeCanvas = useRef<HTMLCanvasElement>(null);
  const turnsRef = useRef<Turns>(0);
  const stageRef = useRef<HTMLDivElement>(null);
  const stage = useBoxSize(stageRef);
  const layers = useRef<{
    base: ImageData | null;
    mask: ImageData | null;
    stars: ImageData | null;
    smoothed: Map<number, ImageData>;
    // The settings on the preview, before the turn: a turn alone redraws it.
    after: ImageData | null;
    // The stars the diffraction spikes are drawn from (whole-image pixels),
    // the long side of the whole image, and the last light drawn (kept while
    // the two spike sliders and « Étoiles » stay where they are).
    spots: SpikeStars | null;
    fullLong: number;
    light: { key: string; layer: Float32Array | null } | null;
  }>({
    base: null,
    mask: null,
    stars: null,
    smoothed: new Map(),
    after: null,
    spots: null,
    fullLong: 0,
    light: null,
  });
  // The group « Aigrettes »: folded, unless the image reopened has some.
  const [spikesOpen, setSpikesOpen] = useState(false);
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
        layers.current.fullLong = Math.max(answer.width, answer.height);
        if (answer.stars === "ready") {
          layers.current.stars = await imageAdjustLayer(path, "stars").then(
            pixelsOf,
          );
          layers.current.spots = await spotsOf(path, setError, t);
        }
        if (!live) return;
        setOpen(answer);
        setStars(answer.stars);
        if (answer.state) {
          setValues({ ...answer.state.values });
          setSaved({ ...answer.state.values });
          // An image reopened with spikes shows its group open.
          if (answer.state.values.spikes > 0) setSpikesOpen(true);
          const reopened = cleanTurns(answer.state.quarterTurns);
          setTurns(reopened);
          setSavedTurns(reopened);
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
          layers.current.spots = await spotsOf(path, setError, t);
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

  // The preview drawn: the render (« Avant ») under, the settings on it
  // (« Après ») over, the curtain showing one or the other.
  useEffect(() => {
    const before = beforeCanvas.current;
    const after = canvas.current;
    const { base, mask } = layers.current;
    if (!before || !after || !base || !mask) return;
    const started = performance.now();
    const source =
      values.smoothing > 0
        ? (layers.current.smoothed.get(values.smoothing) ??
          // Still smoothing: the last picture stays under the veil.
          null)
        : base;
    if (!source) return;
    const out = new ImageData(base.width, base.height);
    adjustPixels(
      source.data,
      layers.current.stars?.data ?? null,
      mask.data,
      values,
      out.data,
      spikeLight(layers.current, base.width, base.height, values),
    );
    layers.current.after = out;
    paintTurned(before, base, turnsRef.current);
    paintTurned(after, out, turnsRef.current);
    lastMs.current = Math.round(performance.now() - started);
    after.dataset.previewMs = String(lastMs.current);
  }, [values, loaded]);

  // A turn alone: the two pictures drawn again, turned, nothing recomputed.
  useEffect(() => {
    turnsRef.current = turns;
    const before = beforeCanvas.current;
    const after = canvas.current;
    const { base } = layers.current;
    const out = layers.current.after;
    if (!before || !after || !base || !out) return;
    paintTurned(before, base, turns);
    paintTurned(after, out, turns);
  }, [turns]);

  const change = useCallback((name: AdjustName, value: number) => {
    setValues((current) => canonicalValues({ ...current, [name]: value }));
    setCurtain(CURTAIN_START);
  }, []);

  // A saved setting only moves the sliders of the preview; nothing is
  // written before « Enregistrer l'image réglée ».
  const apply = (saved: SavedAdjustment) => {
    const before = values;
    const appliedBefore = applied;
    setValues({ ...saved.values });
    if (saved.values.spikes > 0) setSpikesOpen(true);
    setApplied(saved);
    setCurtain(CURTAIN_START);
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
    void imageAdjustSave(path, canonicalValues(values), turns)
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
  // Nothing to write: every setting is the render's and the image is not
  // turned, or the image set already saved is exactly this one.
  const unchanged =
    (neutral && turns === 0) ||
    (saved !== null && sameValues(values, saved) && turns === savedTurns);
  const canSave = !!open && !unchanged && !smoothing && !saving;

  const shape = turnedSize(
    turns,
    open?.previewWidth ?? 0,
    open?.previewHeight ?? 0,
  );
  const fit = fitSize(stage.width, stage.height, shape.width, shape.height);
  const stateKey = turnStateKey(turns);

  return (
    <div className="as-page as-adjust">
      <div className="as-adjust__main">
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
        <section className="as-adjust__zone">
          <div className="as-adjust__tools">
            {stateKey && (
              <p className={`${helpTextClassName} as-adjust__turned`}>
                {t(stateKey)}
                <button
                  type="button"
                  className={buttonClassName("link")}
                  onClick={() => setTurns(0)}
                >
                  {t("adjust.rotate.reset")}
                </button>
              </p>
            )}
            <span className="as-wizard-foot__gap" />
            {(["left", "right"] as const).map((direction) => (
              <button
                key={direction}
                type="button"
                className={buttonClassName("tertiary")}
                disabled={!open}
                onClick={() => setTurns((now) => turnBy(now, direction))}
              >
                <Icon
                  name={direction === "left" ? "rotateLeft" : "rotateRight"}
                  className="as-button__icon"
                />
                {t(
                  direction === "left"
                    ? "adjust.rotate.left"
                    : "adjust.rotate.right",
                )}
              </button>
            ))}
          </div>
          <div className="as-adjust__preview" ref={stageRef}>
            {open ? (
              <CompareFrame
                fit={fit}
                position={curtain}
                onPosition={setCurtain}
                before={beforeCanvas}
                after={canvas}
                veil={
                  smoothing && curtain < 100 ? (
                    <div className="as-adjust__veil">
                      <Waiting
                        size="inline"
                        label={t("adjust.smoothing.computing")}
                      />
                    </div>
                  ) : null
                }
              />
            ) : (
              !error && <Waiting label={t("adjust.wait.open")} />
            )}
          </div>
          <span className={helpTextClassName}>{t("adjust.previewNote")}</span>
        </section>
      </div>
      <aside className="as-adjust__panel">
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
        <div className="as-adjust__scroll">
          <div className="as-adjust__sliders">
            {ADJUST_SPECS.filter((spec) => !spec.group).map((spec) => (
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
            <SpikesGroup
              opened={spikesOpen}
              onToggle={() => setSpikesOpen((now) => !now)}
              values={values}
              ready={
                !!open && stars === "ready" && layers.current.spots !== null
              }
              note={starsNote(stars, t)}
              count={
                layers.current.spots
                  ? starCount(layers.current.spots.stars, values.spikesReach)
                  : 0
              }
              onChange={change}
            />
          </div>
        </div>
        <Notice tone="error">{error}</Notice>
        <div className="as-adjust__foot as-wizard-foot">
          <div className="as-wizard-foot__message">
            {saving ? (
              <Waiting size="inline" label={t("adjust.saving")} />
            ) : (
              <p className={helpTextClassName} role="status">
                {neutral && turns === 0
                  ? t("adjust.foot.unchanged")
                  : t("adjust.foot.kept")}
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
      </aside>
    </div>
  );
}

/** The radius of the handle (2.25 rem at 16 px, border included). */
const HANDLE_RADIUS_PX = 18;

/** The image with its curtain: « Avant » under, « Après » over it, clipped
 * to the right of the bar; the bar and its handle (a slider, reached by Tab)
 * are in the theme's amber. Drag with the mouse or a finger anywhere on the
 * image, arrows 2 % (Shift 10 %), Home / End the two edges, a double click on
 * the bar all « Avant » / all « Après ». */
function CompareFrame({
  fit,
  position,
  onPosition,
  before,
  after,
  veil,
}: {
  fit: Size;
  position: number;
  onPosition: (position: number) => void;
  before: RefObject<HTMLCanvasElement | null>;
  after: RefObject<HTMLCanvasElement | null>;
  veil: ReactNode;
}) {
  const { t } = useI18n();
  const frame = useRef<HTMLDivElement>(null);
  const dragging = useRef(false);
  // The handle never leaves the image: at an edge it leans in by its radius.
  const bar = (position / 100) * fit.width;
  const lean =
    Math.max(0, HANDLE_RADIUS_PX - bar) -
    Math.max(0, bar - (fit.width - HANDLE_RADIUS_PX));
  const pointer = (event: ReactPointerEvent<HTMLDivElement>) => {
    const box = frame.current?.getBoundingClientRect();
    if (box) onPosition(curtainFromPointer(event.clientX, box.left, box.width));
  };
  return (
    <div
      ref={frame}
      className="as-adjust__frame"
      style={{ width: fit.width, height: fit.height }}
      onPointerDown={(event) => {
        if (event.button !== 0) return;
        dragging.current = true;
        event.currentTarget.setPointerCapture(event.pointerId);
        pointer(event);
      }}
      onPointerMove={(event) => {
        if (dragging.current) pointer(event);
      }}
      onPointerUp={() => {
        dragging.current = false;
      }}
      onPointerCancel={() => {
        dragging.current = false;
      }}
    >
      <canvas
        ref={before}
        className="as-adjust__canvas"
        aria-label={t("adjust.before")}
      />
      <canvas
        ref={after}
        className="as-adjust__canvas as-adjust__canvas--over"
        style={{ clipPath: `inset(0 0 0 ${position}%)` }}
        aria-label={t("adjust.after")}
      />
      {labelVisible("before", position, fit.width) && (
        <span className="as-adjust__side as-adjust__side--before">
          {t("adjust.before")}
        </span>
      )}
      {labelVisible("after", position, fit.width) && (
        <span className="as-adjust__side as-adjust__side--after">
          {t("adjust.after")}
        </span>
      )}
      {veil}
      <div
        className="as-adjust__rule"
        style={{ left: `${position}%` }}
        onDoubleClick={() => onPosition(curtainToggle(position))}
      >
        <div
          className="as-adjust__handle"
          role="slider"
          tabIndex={0}
          style={{ marginLeft: lean }}
          aria-label={t("adjust.compare")}
          aria-orientation="horizontal"
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={Math.round(position)}
          aria-valuetext={t("adjust.compare.value", {
            percent: Math.round(position),
          })}
          onKeyDown={(event) => {
            const next = curtainFromKey(position, event.key, event.shiftKey);
            if (next === null) return;
            event.preventDefault();
            onPosition(next);
          }}
        >
          <Icon name="chevron" className="as-adjust__chevron is-left" />
          <Icon name="chevron" className="as-adjust__chevron" />
        </div>
      </div>
    </div>
  );
}

type Translate = ReturnType<typeof useI18n>["t"];

/** The list of the stars of the spikes; a failure is said, the spikes stay
 * out of reach. */
async function spotsOf(
  path: string,
  fail: (message: string) => void,
  t: Translate,
): Promise<SpikeStars | null> {
  try {
    return await imageAdjustSpots(path);
  } catch (caught) {
    fail(formatError(t, caught));
    return null;
  }
}

/** The light of the spikes for these values: drawn again only when the two
 * sliders of the group or « Étoiles » (which takes the spikes of the stars it
 * takes away) move. `null` while there is nothing to draw. */
function spikeLight(
  layers: {
    spots: SpikeStars | null;
    fullLong: number;
    light: { key: string; layer: Float32Array | null } | null;
  },
  width: number,
  height: number,
  values: AdjustValues,
): Float32Array | null {
  if (values.spikes === 0 || !layers.spots || layers.fullLong === 0) {
    return null;
  }
  const key = [
    width,
    height,
    values.spikes,
    values.spikesReach,
    values.stars,
  ].join("|");
  if (layers.light?.key !== key) {
    layers.light = {
      key,
      layer: spikeLayer(
        width,
        height,
        layers.spots.stars,
        values.spikes,
        values.spikesReach,
        Math.max(width, height) / layers.fullLong,
        1 + values.stars / 100,
      ),
    };
  }
  return layers.light.layer;
}

/** « Aigrettes · effet ajouté »: a group folded by default, its line saying
 * « Sans » or « Ajoutées, 60 % »; open, a sentence that says the effect is
 * added, then « Force » and « Étoiles concernées » (with the number of stars
 * it reaches), the second one waiting for the first. */
function SpikesGroup({
  opened,
  onToggle,
  values,
  ready,
  note,
  count,
  onChange,
}: {
  opened: boolean;
  onToggle: () => void;
  values: AdjustValues;
  ready: boolean;
  note: { waiting: boolean; text: string } | null;
  count: number;
  onChange: (name: AdjustName, value: number) => void;
}) {
  const { t, locale } = useI18n();
  const id = useId();
  const [strength, reach] = ADJUST_SPECS.filter(
    (spec) => spec.group === "spikes",
  );
  const none = values.spikes === 0;
  return (
    <section className="as-adjust__group">
      <button
        type="button"
        className="as-adjust__group-head"
        aria-expanded={opened}
        aria-controls={`${id}-body`}
        onClick={onToggle}
      >
        <Icon
          name="chevron"
          className={`as-adjust__group-chevron${opened ? " is-open" : ""}`}
        />
        <span className="as-adjust__slider-title">
          {t("adjust.spikes.title")}
        </span>
        <span className={`${helpTextClassName} as-adjust__group-note`}>
          {t("adjust.spikes.note")}
        </span>
        {opened && !none && (
          <span className={badgeClassName("neutral")}>
            {t("adjust.slider.changed")}
          </span>
        )}
        {!opened && (
          <span className="as-adjust__value as-adjust__group-value">
            {spikesGroupText(t, locale, values)}
          </span>
        )}
      </button>
      {opened && (
        <div id={`${id}-body`} className="as-adjust__group-body">
          <p className={`${helpTextClassName} as-adjust__help`}>
            {t("adjust.spikes.intro")}
          </p>
          <div className="as-adjust__sliders">
            <AdjustSlider
              spec={strength}
              value={values.spikes}
              disabled={!ready && none}
              note={ready ? null : note}
              onChange={(value) => onChange("spikes", value)}
              text={valueText(t, locale, "spikes", values.spikes)}
            />
            <AdjustSlider
              spec={reach}
              value={values.spikesReach}
              disabled={none || !ready}
              note={
                none && ready
                  ? {
                      waiting: false,
                      quiet: true,
                      text: t("adjust.spikes.reach.first"),
                    }
                  : null
              }
              onChange={(value) => onChange("spikesReach", value)}
              text={
                none
                  ? valueText(t, locale, "spikesReach", values.spikesReach)
                  : reachText(t, locale, values.spikesReach, count)
              }
            />
          </div>
        </div>
      )}
    </section>
  );
}

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
  note: { waiting: boolean; text: string; quiet?: boolean } | null;
  onChange: (value: number) => void;
}) {
  const { t } = useI18n();
  const id = useId();
  const changed = isChanged({ ...NEUTRAL, [spec.name]: value }, spec.name);
  const mark = ((NEUTRAL[spec.name] - spec.min) / (spec.max - spec.min)) * 100;
  return (
    <div
      className={`as-adjust__slider${changed ? " is-changed" : ""}${disabled ? " is-disabled" : ""}`}
    >
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
          <p
            className={`${helpTextClassName} as-adjust__note${note.quiet ? " is-quiet" : ""}`}
          >
            {note.text}
          </p>
        ))}
    </div>
  );
}
