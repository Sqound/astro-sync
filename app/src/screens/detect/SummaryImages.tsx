import { useEffect, useState } from "react";

import { useI18n } from "../../i18n";
import {
  imageAdjustRevert,
  imageAdjustState,
  resultFileOpen,
  resultFilePreview,
  resultFileReveal,
  type AdjustState,
} from "../../lib/bridge";
import {
  adjustedLine,
  isAdjustable,
  rowAdjustment,
} from "../../lib/imageAdjust";
import {
  canTickImage,
  seriesKeyOfRow,
  seriesOfRow,
  type SummaryImage,
} from "../../lib/detectTreatments";
import {
  seriesBanner,
  seriesRowText,
  targetsWithSeries,
} from "../../lib/seriesOfPoses";
import { summaryImageView } from "../../lib/summaryImages";
import {
  badgeClassName,
  buttonClassName,
  helpTextClassName,
} from "../../theme";

/** The longest side of the thumbnail kept, in pixels (shown at a quarter). */
const THUMB_SIDE = 360;

/** One picture shrinks after the other: a PNG of ten megabytes is decoded
 * once at a time, never four together. */
let shrinking: Promise<unknown> = Promise.resolve();
const shrunk = new Map<string, string>();

async function shrink(blob: Blob): Promise<string> {
  const bitmap = await createImageBitmap(blob);
  const scale = Math.min(1, THUMB_SIDE / Math.max(bitmap.width, bitmap.height));
  const canvas = document.createElement("canvas");
  canvas.width = Math.max(1, Math.round(bitmap.width * scale));
  canvas.height = Math.max(1, Math.round(bitmap.height * scale));
  canvas.getContext("2d")?.drawImage(bitmap, 0, 0, canvas.width, canvas.height);
  bitmap.close();
  return canvas.toDataURL("image/jpeg", 0.8);
}

/** The picture of an image to look at, shrunk here: the PNG written beside the
 * TIFF (the one with the curve of presentation when there is one). */
function useThumb(
  path: string,
  cacheKey: string = path,
): string | null | undefined {
  const [thumb, setThumb] = useState<string | null | undefined>(
    shrunk.get(cacheKey),
  );
  useEffect(() => {
    if (shrunk.has(cacheKey)) {
      setThumb(shrunk.get(cacheKey));
      return;
    }
    let live = true;
    shrinking = shrinking.then(async () => {
      if (!live) return;
      try {
        const blob = await resultFilePreview(path);
        const url = blob ? await shrink(blob) : null;
        if (url) shrunk.set(cacheKey, url);
        if (live) setThumb(url);
      } catch {
        if (live) setThumb(null);
      }
    });
    return () => {
      live = false;
    };
  }, [path, cacheKey]);
  return thumb;
}

function Thumb({
  path,
  name,
  cacheKey,
}: {
  path: string;
  name: string;
  /** The image set's digest: a new one shows its new picture. */
  cacheKey?: string;
}) {
  const { t } = useI18n();
  const thumb = useThumb(path, cacheKey);
  return (
    <div className="as-treat__thumb">
      {thumb ? (
        <img src={thumb} alt={t("detect.send.previewAlt", { file: name })} />
      ) : (
        <span className={helpTextClassName}>
          {thumb === null ? t("detect.send.noPreview") : ""}
        </span>
      )}
    </div>
  );
}

/** The name of the Finder, the Explorer or the file manager, in a label. */
function revealKey(): string {
  return navigator.userAgent.includes("Windows")
    ? "detect.send.revealExplorer"
    : navigator.userAgent.includes("Mac")
      ? "detect.send.revealFinder"
      : "detect.send.revealFolder";
}

/**
 * The rows of « Images à envoyer »: each target's capture and, beneath it, its
 * variants -- every image the treatments made --, each with its box, its
 * thumbnail, its path (a link that opens the file) and « Afficher dans le
 * Finder ». A row the space cannot take says why and its box is disabled.
 */
export function SummaryImages({
  rows,
  ticked,
  room,
  pipelineName,
  onToggle,
  onAdjust,
  adjustKey = 0,
}: {
  rows: readonly SummaryImage[];
  ticked: readonly string[];
  room: { remaining: number } | null;
  pipelineName: (id: string) => string;
  onToggle: (key: string) => void;
  /** « Régler l'image » on a row (absent: no button). */
  onAdjust?: (row: SummaryImage, render: string) => void;
  /** Changes when an image was set or brought back: the rows read again. */
  adjustKey?: number;
}) {
  const { t, locale } = useI18n();
  // The image set of each PNG render, read on the disk: « Réglée », its
  // values, and its file in place of the render's.
  const [adjusted, setAdjusted] = useState<Record<string, AdjustState | null>>(
    {},
  );
  const [reverted, setReverted] = useState(0);
  const adjustable = onAdjust
    ? rows.filter((row) => row.role !== "file" && isAdjustable(row))
    : [];
  const adjustablePaths = adjustable.map((row) => row.path).join("\u0000");
  useEffect(() => {
    if (!adjustablePaths) return;
    let live = true;
    const paths = adjustablePaths.split("\u0000");
    void Promise.all(
      paths.map((path) =>
        imageAdjustState(path)
          .then((state) => [path, state] as const)
          .catch(() => [path, null] as const),
      ),
    ).then((pairs) => {
      if (live) setAdjusted(Object.fromEntries(pairs));
    });
    return () => {
      live = false;
    };
  }, [adjustablePaths, adjustKey, reverted]);
  // A target with several series of poses: a banner says why they are not
  // stacked together and which one gives the main image, and each series has
  // its line and one box that ticks or unticks its images at once (the box
  // of each image stays).
  const multi = targetsWithSeries(
    rows
      .filter((row) => row.role !== "file")
      .map((row) => ({ targetId: row.targetId, series: seriesOfRow(row) })),
  );
  const bannered = new Set<string>();
  const headed = new Set<string>();
  const seriesHead = (row: SummaryImage) => {
    const list = multi.get(row.targetId);
    const key = seriesKeyOfRow(row);
    if (!list || row.role === "file") return null;
    const items = [];
    if (!bannered.has(row.targetId)) {
      bannered.add(row.targetId);
      const banner = seriesBanner(t, locale, row.designation, list);
      items.push(
        <li key={`banner:${row.targetId}`} className="as-treat__series-banner">
          <strong>{banner.title}</strong>
          <span className="as-treat__meta">{banner.body}</span>
          <span className="as-treat__meta">{banner.main}</span>
        </li>,
      );
    }
    const entry = list.find((item) => item.key === key);
    if (entry && !headed.has(`${row.targetId}\u0000${entry.key}`)) {
      headed.add(`${row.targetId}\u0000${entry.key}`);
      const members = rows.filter(
        (other) =>
          other.targetId === row.targetId &&
          other.role !== "file" &&
          seriesKeyOfRow(other) === entry.key,
      );
      const all = members.every((other) => ticked.includes(other.key));
      items.push(
        <li
          key={`series:${row.targetId}:${entry.key}`}
          className="as-treat__series"
        >
          <label>
            <input
              type="checkbox"
              checked={all}
              onChange={() => {
                for (const other of members) {
                  const on = ticked.includes(other.key);
                  if (all ? on : !on && canTickImage(other, rows, ticked, room))
                    onToggle(other.key);
                }
              }}
            />
            <span>
              <strong>{seriesRowText(t, locale, entry.series)}</strong>
              {entry.main && (
                <span className="as-treat__meta">{t("series.mainBadge")}</span>
              )}
              {entry.minor && !all && (
                <span className="as-treat__meta">
                  {t("series.minorImport")}
                </span>
              )}
            </span>
          </label>
        </li>,
      );
    }
    return items;
  };
  return (
    <ul className="as-treat__images">
      {rows.flatMap((row) => [
        ...(seriesHead(row) ?? []),
        (() => {
          // The render's row and the row of its TIFF in full resolution
          // both say the file of the image set that leaves in their place.
          const set = onAdjust ? rowAdjustment(row, adjusted) : null;
          const view = summaryImageView(
            t,
            locale,
            set ? { ...row, size: set.size } : row,
            pipelineName,
          );
          const shown = set ? set.path : row.path;
          const on = ticked.includes(row.key);
          const open = canTickImage(row, rows, ticked, room);
          const name = row.path.split(/[\\/]/).pop() ?? row.path;
          const classes = [
            "as-treat__image",
            row.role === "variant" ? "is-variant" : "",
            row.role === "file" ? "is-file" : "",
            open ? "" : "is-full",
          ]
            .filter(Boolean)
            .join(" ");
          return (
            <li key={row.key} className={classes} data-role={row.role}>
              <label>
                <input
                  type="checkbox"
                  checked={on}
                  disabled={!open}
                  onChange={() => onToggle(row.key)}
                />
                <span>
                  <strong>{view.title}</strong>
                  {set && (
                    <>
                      {" "}
                      <span className={badgeClassName("outline-gold")}>
                        {t("adjust.badge.adjusted")}
                      </span>
                    </>
                  )}
                  {t("detect.join")}
                  {view.render}
                  <span className="as-treat__meta">{view.meta}</span>
                  {set && (
                    <span className="as-treat__meta as-treat__adjusted">
                      {adjustedLine(t, locale, set.values)}
                    </span>
                  )}
                  {view.sentence && (
                    <span className="as-treat__meta">{view.sentence}</span>
                  )}
                  {view.blocked ? (
                    <span className="as-treat__meta as-treat__why">
                      {view.blocked}
                    </span>
                  ) : (
                    !open && (
                      <span className="as-treat__meta">
                        {t("detect.send.full")}
                      </span>
                    )
                  )}
                </span>
              </label>
              <div className="as-treat__file">
                {row.role !== "file" && (
                  <Thumb
                    path={shown}
                    name={name}
                    cacheKey={set ? `${shown}#${set.sha256}` : undefined}
                  />
                )}
                <div className="as-treat__links">
                  <button
                    type="button"
                    className={`${buttonClassName("link")} as-treat__path`}
                    title={t("detect.send.openTitle")}
                    onClick={() => void resultFileOpen(shown).catch(() => {})}
                  >
                    {shown}
                  </button>
                  <button
                    type="button"
                    className={buttonClassName("link")}
                    onClick={() => void resultFileReveal(shown).catch(() => {})}
                  >
                    {t(revealKey())}
                  </button>
                  {/* « Régler l'image »: never on « Sans traitement » nor
                      on a TIFF. Once set: « Régler à nouveau » and «
                      Revenir au rendu d'origine ». */}
                  {onAdjust && isAdjustable(row) && row.role !== "file" && (
                    <div className="as-treat__adjust-actions">
                      <button
                        type="button"
                        className={`${buttonClassName("tertiary")} as-treat__adjust`}
                        onClick={() => onAdjust(row, view.render)}
                      >
                        {t(set ? "adjust.again" : "adjust.open")}
                      </button>
                      {set && (
                        <button
                          type="button"
                          className={`${buttonClassName("link")} as-treat__revert`}
                          onClick={() =>
                            void imageAdjustRevert(row.path)
                              .catch(() => {})
                              .then(() => setReverted((n) => n + 1))
                          }
                        >
                          {t("adjust.revert")}
                        </button>
                      )}
                    </div>
                  )}
                </div>
              </div>
            </li>
          );
        })(),
      ])}
    </ul>
  );
}
