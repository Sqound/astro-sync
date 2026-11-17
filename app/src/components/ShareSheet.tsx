import { useEffect, useRef, useState } from "react";

import { useI18n } from "../i18n";
import {
  resultFilePreview,
  shareCopyChoose,
  shareCopyWrite,
  targetsSearch,
} from "../lib/bridge";
import type { SummaryImage } from "../lib/detectTreatments";
import {
  SHARE_OPTIONS_KEY,
  bandInputFromRow,
  bandText,
  parseShareOptions,
  serialiseShareOptions,
  shareCloseKey,
  shareFileName,
  type ShareOptions,
} from "../lib/shareBand";
import { drawPreview, layoutOf, renderCopy } from "../lib/shareCopy";
import { windowStorage } from "../lib/lastReport";
import { buttonClassName, helpTextClassName } from "../theme";

/** The widest picture the preview draws, in pixels (shown smaller). */
const PREVIEW_PX = 720;

function remembered(): ShareOptions {
  try {
    return parseShareOptions(
      windowStorage()?.getItem(SHARE_OPTIONS_KEY) ?? null,
    );
  } catch {
    return parseShareOptions(null);
  }
}

function remember(options: ShareOptions): void {
  try {
    windowStorage()?.setItem(SHARE_OPTIONS_KEY, serialiseShareOptions(options));
  } catch {
    // A storage that refuses: the choice lasts until the sheet closes.
  }
}

/**
 * « Exporter pour partager »: the sheet of the copy with a band. Two
 * switches (the night's data, the Astro Sync mark), both on at first and
 * remembered, a preview drawn by the same code as the file, and two
 * buttons: « Annuler » and « Enregistrer l'image… », which asks where to
 * save the PNG. The options are set here and the main button applies them.
 * The band exists only in this copy: no file sent and no file of the render
 * is touched.
 */
export function ShareSheet({
  row,
  shown,
  nightDates,
  onClose,
}: {
  row: SummaryImage;
  /** The image the card shows (the image set when the render was set). */
  shown: string;
  /** The evening dates of the nights of the row's target. */
  nightDates: readonly string[];
  onClose: () => void;
}) {
  const { t, locale } = useI18n();
  const [options, setOptions] = useState<ShareOptions>(remembered);
  const [bitmap, setBitmap] = useState<ImageBitmap | null>(null);
  const [name, setName] = useState<string | null>(null);
  const [status, setStatus] = useState<
    "idle" | "saving" | "done" | "failed" | "missing"
  >("idle");
  const canvas = useRef<HTMLCanvasElement>(null);

  // The picture the card shows, decoded once.
  useEffect(() => {
    let live = true;
    let decoded: ImageBitmap | null = null;
    void (async () => {
      try {
        const blob = await resultFilePreview(shown);
        if (!blob) {
          if (live) setStatus("missing");
          return;
        }
        decoded = await createImageBitmap(blob);
        if (live) setBitmap(decoded);
        else decoded.close();
      } catch {
        if (live) setStatus("missing");
      }
    })();
    return () => {
      live = false;
    };
  }, [shown]);

  // The usual name of the target, when the catalogue has one in the
  // language of the window; without it the designation leads.
  useEffect(() => {
    let live = true;
    targetsSearch(row.designation, locale, 0, 8)
      .then((found) => {
        if (!live) return;
        const same =
          found.targets.find((target) => target.id === row.targetId) ??
          found.targets.find(
            (target) =>
              target.designation.toLowerCase() ===
              row.designation.toLowerCase(),
          );
        setName(same?.name ?? null);
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [row.designation, row.targetId, locale]);

  const text = bandText(t, locale, bandInputFromRow(row, name, nightDates));
  const layout = bitmap ? layoutOf(bitmap, text, options) : null;
  const textKey = JSON.stringify(text);

  useEffect(() => {
    if (bitmap && canvas.current) {
      drawPreview(canvas.current, bitmap, text, options, PREVIEW_PX);
    }
    // `textKey` stands for `text`, which is a new object at each render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [bitmap, textKey, options]);

  useEffect(() => () => bitmap?.close(), [bitmap]);

  const change = (next: ShareOptions) => {
    setOptions(next);
    remember(next);
    setStatus("idle");
  };

  const save = async () => {
    if (!bitmap) return;
    setStatus("saving");
    try {
      const path = await shareCopyChoose(shareFileName(row.designation));
      if (!path) {
        setStatus("idle");
        return;
      }
      const png = await renderCopy(bitmap, text, options);
      await shareCopyWrite(path, row.path, png);
      setStatus("done");
    } catch {
      setStatus("failed");
    }
  };

  const messages: Record<string, string | null> = {
    idle: null,
    saving: t("share.saving"),
    done: t("share.done"),
    failed: t("share.failed"),
    missing: t("detect.send.noPreview"),
  };

  return (
    <div className="as-dialog__scrim" role="presentation">
      <div
        className="as-dialog as-share"
        role="dialog"
        aria-modal="true"
        aria-labelledby="share-title"
      >
        <h2 id="share-title" className="as-dialog__title">
          {t("share.title")}
        </h2>
        <div className="as-share__body">
          <figure className="as-share__preview">
            <canvas
              ref={canvas}
              className="as-share__canvas"
              role="img"
              aria-label={t("share.preview.alt")}
            />
            <figcaption className={helpTextClassName}>
              {t("share.preview")}
            </figcaption>
          </figure>
          <div className="as-share__options">
            <label className="as-share__option">
              <input
                type="checkbox"
                role="switch"
                className="as-switch"
                checked={options.data}
                onChange={(event) =>
                  change({ ...options, data: event.target.checked })
                }
              />
              <span className="as-share__label">
                <strong>{t("share.data.title")}</strong>
                <span className={helpTextClassName}>
                  {t("share.data.line")}
                </span>
              </span>
            </label>
            <label className="as-share__option">
              <input
                type="checkbox"
                role="switch"
                className="as-switch"
                checked={options.mark}
                onChange={(event) =>
                  change({ ...options, mark: event.target.checked })
                }
              />
              <span className="as-share__label">
                <strong>{t("share.mark.title")}</strong>
                <span className={helpTextClassName}>
                  {t("share.mark.line")}
                </span>
              </span>
            </label>
            <div className="as-share__facts">
              {layout && (
                <p className="as-share__format">
                  {t("share.format", {
                    width: layout.width,
                    height: layout.height,
                  })}
                </p>
              )}
              <p className={helpTextClassName}>{t("share.reassure")}</p>
              {messages[status] && (
                <p className="as-share__status" role="status">
                  {messages[status]}
                </p>
              )}
            </div>
          </div>
        </div>
        <div className="as-actions">
          <button
            type="button"
            className={buttonClassName("secondary")}
            onClick={onClose}
          >
            {t(shareCloseKey(status === "done"))}
          </button>
          <button
            type="button"
            className={buttonClassName("primary")}
            disabled={!bitmap || status === "saving"}
            onClick={() => void save()}
          >
            {t("share.save")}
          </button>
        </div>
      </div>
    </div>
  );
}
