import { useEffect, useId, useState, type ReactNode } from "react";

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
import { mainImageName, summaryImageView } from "../../lib/summaryImages";
import {
  KEEP,
  onlineMainLabel,
  sinceDate,
  type MainContext,
  type MainGroup,
} from "../../lib/mainImage";
import { pluralText } from "../../lib/written";
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
  groups,
  mainContext,
  onChoose,
  choiceOpen = true,
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
  /** The groups of the list (a target and a series of poses: one capture),
   * each with its main image resolved. */
  groups: readonly MainGroup[];
  mainContext: MainContext;
  /** The image chosen as the main one of a group, or `KEEP`. */
  onChoose: (group: string, value: string) => void;
  /** The choice may change (not while the images are being sent). */
  choiceOpen?: boolean;
}) {
  const { t, locale } = useI18n();
  const groupsId = useId();
  const groupOf = new Map<string, MainGroup>();
  for (const group of groups)
    for (const card of group.cards) groupOf.set(card.key, group);
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
  /** One card of « Images à envoyer »: the picture, the title, the boxes, the
   * choice of the main image, the actions. */
  const renderCard = (row: SummaryImage) => {
    const group = groupOf.get(row.key);
    // The titles follow the choice: the main image is « M45 », every
    // other image of its group a variant of that capture.
    const shownRole: SummaryImage["role"] =
      group && group.cards.length > 1 && group.selected !== null
        ? group.selected === row.key
          ? "capture"
          : "variant"
        : group && (group.locked || group.selected === KEEP)
          ? "variant"
          : row.role;
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
      { ...row, ...(set ? { size: set.size } : {}), role: shownRole },
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
    const tiffOpen = !!tiffRow && canTickImage(tiffRow, rows, ticked, room);
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
    return (
      <li key={row.key} className={classes} data-role={shownRole}>
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
            <span className="as-treat__meta as-treat__why">{view.blocked}</span>
          ) : (
            !open && (
              <span className="as-treat__meta">{t("detect.send.full")}</span>
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
          {group && (
            <MainChoice
              row={row}
              group={group}
              name={`${groupsId}:${group.key}`}
              open={choiceOpen}
              onChoose={onChoose}
            />
          )}
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
            <span className={helpTextClassName}>{t("share.openNote")}</span>
          </div>
        </div>
      </li>
    );
  };
  // The items of the list, in order: the banner and the line of a series, then
  // the group of its images (a target and a series: one capture) with the
  // choice of its main image, then the linear file.
  const items: ReactNode[] = [];
  const grouped = new Set<string>();
  for (const row of rows) {
    // The TIFF of a render is the second box of its PNG's card.
    if (row.fullResolution && pngRowOf(row, rows)) continue;
    const head = seriesHead(row) ?? [];
    if (row.role === "file") {
      items.push(
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
      );
      continue;
    }
    const group = groupOf.get(row.key);
    if (!group) {
      items.push(...head, renderCard(row));
      continue;
    }
    if (grouped.has(group.key)) continue;
    grouped.add(group.key);
    items.push(
      ...head,
      <MainGroupBlock
        key={`group:${group.key}`}
        id={`${groupsId}:${group.key}`}
        group={group}
        context={mainContext}
        open={choiceOpen}
        onChoose={onChoose}
        renderOf={(card) =>
          mainImageName(t, locale, card, group.cards, pipelineName)
        }
      >
        {group.cards.map((card) => renderCard(card))}
      </MainGroupBlock>,
    );
  }
  return (
    <>
      <ul className="as-treat__images">{items}</ul>
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
        onClick={() => void resultFileOpen(path)}
      >
        {path}
      </button>
      <button
        type="button"
        className={buttonClassName("link")}
        onClick={() => void resultFileReveal(path)}
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

/** The head of a target and series of poses and its images beneath it: « M45 ·
 * une capture, 4 images », the main image in one line, and, when the space
 * holds the capture already, « Garder l'image principale de votre espace ».
 * The main image is chosen on the cards, with one radio each, all of one
 * group named by that title (a screen reader reads « Image principale, groupe
 * de boutons radio »). One image alone: a sentence, no choice. */
function MainGroupBlock({
  id,
  group,
  context,
  open,
  onChoose,
  renderOf,
  children,
}: {
  id: string;
  group: MainGroup;
  context: MainContext;
  open: boolean;
  onChoose: (group: string, value: string) => void;
  renderOf: (card: SummaryImage) => string;
  children: ReactNode;
}) {
  const { t, locale } = useI18n();
  const online = group.online;
  const capture = online.kind === "capture" ? online : null;
  const first = group.cards[0];
  const count = group.cards.length;
  // The render named in the line of the main image: the card's, or, for the
  // image the space holds, the card of the same bytes, else its file.
  const heldName = capture?.main
    ? onlineMainLabel(group, context, capture.main, renderOf)
    : "";
  const selectedCard =
    group.selected && group.selected !== KEEP
      ? group.cards.find((card) => card.key === group.selected)
      : undefined;
  const single = count === 1 && !group.keepOffered && !group.locked;
  const current = single
    ? t("detect.send.main.only")
    : group.locked
      ? t("detect.send.main.public")
      : selectedCard
        ? t("detect.send.main.current", { render: renderOf(selectedCard) })
        : group.selected === KEEP && capture?.main
          ? t("detect.send.main.current", { render: heldName })
          : null;
  // The deliberate gesture, written: what becomes of the image the space holds.
  const swap =
    group.keepOffered && selectedCard && capture?.main
      ? t("detect.send.main.swap", {
          old: heldName,
          new: renderOf(selectedCard),
        })
      : null;
  const choice = !single && !group.locked;
  const titleId = `${id}:title`;
  return (
    <li className="as-treat__group" data-target={group.targetId}>
      <div
        className="as-treat__group-body"
        role={choice ? "radiogroup" : undefined}
        aria-labelledby={choice ? titleId : undefined}
      >
        <div className="as-treat__group-head">
          <p className="as-treat__group-title" id={titleId}>
            <strong>
              {pluralText(t, locale, "detect.send.main.groupTitle", count, {
                target: first.designation,
              })}
            </strong>
          </p>
          {current && <span className="as-treat__meta">{current}</span>}
          {online.kind === "unread" && (
            <span className="as-treat__meta">
              {t("detect.send.main.unread")}
            </span>
          )}
        </div>
        {group.keepOffered && capture?.main && (
          <div className="as-treat__keep">
            <label>
              <input
                type="radio"
                name={id}
                checked={group.selected === KEEP}
                disabled={!open}
                onChange={() => onChoose(group.key, KEEP)}
              />
              <span>
                <strong>{t("detect.send.main.keep")}</strong>
                <span className="as-treat__meta">
                  {t("detect.send.main.keepSince", {
                    render: heldName,
                    date: sinceDate(locale, capture.main.since),
                  })}
                </span>
              </span>
            </label>
            {swap && <span className="as-treat__meta">{swap}</span>}
          </div>
        )}
        <ul className="as-treat__images as-treat__group-images">{children}</ul>
      </div>
    </li>
  );
}

/** « Image principale »: the radio of a card, one per group. Closed, and
 * why, for an image that is not ticked, one whose bytes the space holds
 * already, or on a public capture. */
function MainChoice({
  row,
  group,
  name,
  open,
  onChoose,
}: {
  row: SummaryImage;
  group: MainGroup;
  name: string;
  open: boolean;
  onChoose: (group: string, value: string) => void;
}) {
  const { t } = useI18n();
  if (group.cards.length === 1 && !group.keepOffered && !group.locked)
    return null;
  const block = group.blocks[row.key];
  const why = group.locked || !block ? null : t(`detect.send.main.${block}`);
  return (
    <div className="as-treat__main-choice">
      <label>
        <input
          type="radio"
          name={name}
          checked={group.selected === row.key}
          disabled={!open || group.locked || block !== undefined}
          onChange={() => onChoose(group.key, row.key)}
        />
        <span>{t("detect.send.main.label")}</span>
      </label>
      {why && <span className="as-treat__meta">{why}</span>}
    </div>
  );
}
