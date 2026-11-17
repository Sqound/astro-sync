import { useEffect, useState } from "react";

import { Icon } from "../../components/Icon";
import { ShareSheet } from "../../components/ShareSheet";
import { TiffWarning } from "../../components/TiffWarning";
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
  pngRowOf,
  seriesKeyOfRow,
  seriesOfRow,
  tiffWaitsForPng,
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
 * « Images à envoyer »: one card per image -- each target's capture and, after
 * it, its variants, every image the treatments made. A card holds the picture
 * (never a button: « Régler l'image » is the one way into the editor), the
 * title, what the image set says once it was set, « À envoyer » with a box for
 * the PNG and a box for its TIFF in full resolution (unticked unless chosen,
 * with a warning when ticked, and only once the PNG is), the two actions of
 * the editor, and the file's place on one grey line with « Afficher dans le
 * Finder ». The linear file of « Sans traitement » is a lighter line under the
 * cards of its target. A card the space cannot take says why and its boxes are
 * disabled.
 */
export function SummaryImages({
  rows,
  ticked,
  room,
  pipelineName,
  onToggle,
  onAdjust,
  nightDatesOf,
  adjustKey = 0,
}: {
  rows: readonly SummaryImage[];
  ticked: readonly string[];
  room: { remaining: number } | null;
  pipelineName: (id: string) => string;
  onToggle: (key: string) => void;
  /** « Régler l'image » on a row (absent: no button). */
  onAdjust?: (row: SummaryImage, render: string) => void;
  /** The evening dates of a target's nights, for the band of « Exporter
   * pour partager ». */
  nightDatesOf?: (targetId: string) => readonly string[];
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
  // The row whose « Exporter pour partager » sheet is open, with the image
  // the card shows.
  const [sharing, setSharing] = useState<{
    row: SummaryImage;
    shown: string;
  } | null>(null);
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
      // The series' box ticks and unticks the PNG of its images; the TIFF
      // of each is its own box, and goes with its PNG.
      const members = rows.filter(
        (other) =>
          other.targetId === row.targetId &&
          other.role !== "file" &&
          !other.fullResolution &&
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
    <>
      <ul className="as-treat__images">
        {rows.flatMap((row) => {
          // The TIFF of a render is the second box of its PNG's card.
          if (row.fullResolution && pngRowOf(row, rows)) return [];
          const head = seriesHead(row) ?? [];
          if (row.role === "file") {
            return [
              ...head,
              <LinearLine
                key={row.key}
                row={row}
                rows={rows}
                ticked={ticked}
                room={room}
                pipelineName={pipelineName}
                onToggle={onToggle}
              />,
            ];
          }
          // The render's card and the card of its TIFF in full resolution
          // both say the file of the image set that leaves in their place.
          const set = onAdjust ? rowAdjustment(row, adjusted) : null;
          const tiffRow =
            rows.find(
              (other) =>
                other.fullResolution &&
                other.targetId === row.targetId &&
                other.twinOf === row.path,
            ) ?? null;
          const tiffSet =
            onAdjust && tiffRow ? rowAdjustment(tiffRow, adjusted) : null;
          const view = summaryImageView(
            t,
            locale,
            set ? { ...row, size: set.size } : row,
            pipelineName,
          );
          const tiffView = tiffRow
            ? summaryImageView(
                t,
                locale,
                tiffSet ? { ...tiffRow, size: tiffSet.size } : tiffRow,
                pipelineName,
              )
            : null;
          const shown = set ? set.path : row.path;
          const on = ticked.includes(row.key);
          const open = canTickImage(row, rows, ticked, room);
          const tiffOn = !!tiffRow && ticked.includes(tiffRow.key);
          const tiffOpen =
            !!tiffRow && canTickImage(tiffRow, rows, ticked, room);
          const waitsForPng =
            !!tiffRow && !tiffOn && tiffWaitsForPng(tiffRow, rows, ticked);
          const name = row.path.split(/[\\/]/).pop() ?? row.path;
          const classes = [
            "as-treat__image",
            row.role === "variant" ? "is-variant" : "",
            open ? "" : "is-full",
          ]
            .filter(Boolean)
            .join(" ");
          return [
            ...head,
            <li key={row.key} className={classes} data-role={row.role}>
              <Thumb
                path={shown}
                name={name}
                cacheKey={set ? `${shown}#${set.sha256}` : undefined}
              />
              <div className="as-treat__links">
                <p className="as-send__title">
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
                </p>
                {set && (
                  <span className="as-treat__meta as-treat__adjusted">
                    {adjustedLine(t, locale, set.values, set.quarterTurns)}
                  </span>
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
                <fieldset className="as-send__formats">
                  <legend className="as-treat__meta">
                    {t("detect.send.toSend")}
                  </legend>
                  <label>
                    <input
                      type="checkbox"
                      checked={on}
                      disabled={!open}
                      onChange={() => onToggle(row.key)}
                    />
                    <span>{view.meta}</span>
                  </label>
                  {tiffRow && tiffView && (
                    <>
                      <label>
                        <input
                          type="checkbox"
                          checked={tiffOn}
                          disabled={!tiffOpen}
                          onChange={() => onToggle(tiffRow.key)}
                        />
                        <span>{tiffView.meta}</span>
                        {tiffView.blocked && (
                          <span className="as-treat__meta as-treat__why">
                            {tiffView.blocked}
                          </span>
                        )}
                        {waitsForPng && !tiffView.blocked && (
                          <span className="as-treat__meta">
                            {t("detect.send.tiffNeedsPng")}
                          </span>
                        )}
                      </label>
                      {tiffOn && <TiffWarning />}
                    </>
                  )}
                </fieldset>
                {/* « Régler l'image »: never on « Sans traitement » nor on a
                  TIFF. Once set: « Régler à nouveau » and « Revenir au
                  rendu d'origine ». */}
                {onAdjust && isAdjustable(row) && (
                  <div className="as-treat__adjust-actions">
                    <button
                      type="button"
                      className={`${buttonClassName("tertiary")} as-treat__adjust`}
                      onClick={() => onAdjust(row, view.render)}
                    >
                      <Icon name="sliders" className="as-button__icon" />
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
                <FilePlace path={shown} />
                <div className="as-treat__share">
                  <button
                    type="button"
                    className={buttonClassName("tertiary")}
                    onClick={() => setSharing({ row, shown })}
                  >
                    <Icon name="external" className="as-button__icon" />
                    {t("share.open")}
                  </button>
                  <span className={helpTextClassName}>
                    {t("share.openNote")}
                  </span>
                </div>
              </div>
            </li>,
          ];
        })}
      </ul>
      {sharing && (
        <ShareSheet
          row={sharing.row}
          shown={sharing.shown}
          nightDates={nightDatesOf?.(sharing.row.targetId) ?? []}
          onClose={() => setSharing(null)}
        />
      )}
    </>
  );
}

/** The place of a file on one grey line, cut short (the whole path is its
 * tooltip), that opens it; « Afficher dans le Finder » beside it. */
function FilePlace({ path }: { path: string }) {
  const { t } = useI18n();
  return (
    <div className="as-send__where">
      <button
        type="button"
        className="as-send__path"
        title={path}
        aria-label={`${t("detect.send.openTitle")} : ${path}`}
        onClick={() => void resultFileOpen(path).catch(() => {})}
      >
        {path}
      </button>
      <button
        type="button"
        className={buttonClassName("link")}
        onClick={() => void resultFileReveal(path).catch(() => {})}
      >
        {t(revealKey())}
      </button>
    </div>
  );
}

/** « Sans traitement »: the linear file, a light line under the cards of its
 * target -- a TIFF to download, never an image of the capture. */
function LinearLine({
  row,
  rows,
  ticked,
  room,
  pipelineName,
  onToggle,
}: {
  row: SummaryImage;
  rows: readonly SummaryImage[];
  ticked: readonly string[];
  room: { remaining: number } | null;
  pipelineName: (id: string) => string;
  onToggle: (key: string) => void;
}) {
  const { t, locale } = useI18n();
  const view = summaryImageView(t, locale, row, pipelineName);
  const on = ticked.includes(row.key);
  const open = canTickImage(row, rows, ticked, room);
  return (
    <li
      className={
        open ? "as-treat__image is-file" : "as-treat__image is-file is-full"
      }
      data-role="file"
    >
      <label>
        <input
          type="checkbox"
          checked={on}
          disabled={!open}
          onChange={() => onToggle(row.key)}
        />
        <span>
          <strong>{view.title}</strong>
          {t("detect.join")}
          {view.render}
          <span className="as-treat__meta">{view.meta}</span>
          {view.sentence && (
            <span className="as-treat__meta">{view.sentence}</span>
          )}
          {view.blocked && (
            <span className="as-treat__meta as-treat__why">{view.blocked}</span>
          )}
          {on && <TiffWarning as="span" />}
        </span>
      </label>
    </li>
  );
}
