import { useEffect, useState } from "react";

import { useI18n } from "../../i18n";
import {
  resultFileOpen,
  resultFilePreview,
  resultFileReveal,
} from "../../lib/bridge";
import { canTickImage, type SummaryImage } from "../../lib/detectTreatments";
import { summaryImageView } from "../../lib/summaryImages";
import { buttonClassName, helpTextClassName } from "../../theme";

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
function useThumb(path: string): string | null | undefined {
  const [thumb, setThumb] = useState<string | null | undefined>(
    shrunk.get(path),
  );
  useEffect(() => {
    if (shrunk.has(path)) return;
    let live = true;
    shrinking = shrinking.then(async () => {
      if (!live) return;
      try {
        const blob = await resultFilePreview(path);
        const url = blob ? await shrink(blob) : null;
        if (url) shrunk.set(path, url);
        if (live) setThumb(url);
      } catch {
        if (live) setThumb(null);
      }
    });
    return () => {
      live = false;
    };
  }, [path]);
  return thumb;
}

function Thumb({ path, name }: { path: string; name: string }) {
  const { t } = useI18n();
  const thumb = useThumb(path);
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
}: {
  rows: readonly SummaryImage[];
  ticked: readonly string[];
  room: { remaining: number } | null;
  pipelineName: (id: string) => string;
  onToggle: (key: string) => void;
}) {
  const { t, locale } = useI18n();
  return (
    <ul className="as-treat__images">
      {rows.map((row) => {
        const view = summaryImageView(t, locale, row, pipelineName);
        const on = ticked.includes(row.key);
        const open = canTickImage(row, rows, ticked, room);
        const name = row.path.split(/[\\/]/).pop() ?? row.path;
        const classes = [
          "as-treat__image",
          row.role === "variant" ? "is-variant" : "",
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
                {t("detect.join")}
                {view.render}
                <span className="as-treat__meta">{view.meta}</span>
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
              <Thumb path={row.path} name={name} />
              <div className="as-treat__links">
                <button
                  type="button"
                  className={`${buttonClassName("link")} as-treat__path`}
                  title={t("detect.send.openTitle")}
                  onClick={() => void resultFileOpen(row.path).catch(() => {})}
                >
                  {row.path}
                </button>
                <button
                  type="button"
                  className={buttonClassName("link")}
                  onClick={() =>
                    void resultFileReveal(row.path).catch(() => {})
                  }
                >
                  {t(revealKey())}
                </button>
              </div>
            </div>
          </li>
        );
      })}
    </ul>
  );
}
