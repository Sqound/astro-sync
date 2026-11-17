// The two files of a render, as the window says them (`render_files` of the
// core): a render written as PNG and TIFF leaves as its PNG; its TIFF -- the
// full resolution, only used to download the image -- leaves only when the
// person ticks « Joindre aussi les TIFF ». A render that exists only as a TIFF
// (« Sans traitement ») leaves as it is. Pure TypeScript, tested under
// `node --test`.

import type { TranslateParams } from "../i18n/locale.ts";
import { formatSpaceSize } from "./written.ts";

type Translate = (key: string, params?: TranslateParams) => string;

/** A TIFF, by its name. */
export function isTiffPath(path: string): boolean {
  return /\.tiff?$/i.test(path);
}

/** The TIFF of a render that also leaves as a PNG (the core names its twin). */
export function isFullResolutionTwin(image: {
  path: string;
  twinSha256?: string | null;
}): boolean {
  return !!image.twinSha256 && isTiffPath(image.path);
}

/** « PNG », « TIFF pleine résolution », « TIFF seul »: the format a row of a
 * render says, strong in the line. */
export function renderFormatText(
  t: Translate,
  image: { path: string; twinSha256?: string | null },
): string {
  if (!isTiffPath(image.path)) return "PNG";
  return t(
    image.twinSha256
      ? "uploadFormat.tiffFullResolution"
      : "uploadFormat.tiffOnly",
  );
}

/** The label of the box. One TIFF known: its own weight, « Joindre aussi le
 * TIFF (pleine résolution, 47 Mo) ». Several: only their total is known, so
 * the figure is an average and says so, « ≈ 47 Mo en moyenne » -- never
 * « chacun », which would claim a weight for each file. None known (the
 * renders are made at the sending): no weight. */
export function joinTiffLabel(
  t: Translate,
  locale: string,
  fullResolution: { count: number; bytes: number } | null,
): string {
  if (!fullResolution || fullResolution.count <= 0) {
    return t("uploadFormat.joinTiff");
  }
  const size = formatSpaceSize(
    t,
    Math.round(fullResolution.bytes / fullResolution.count),
    locale,
  );
  return t(
    fullResolution.count === 1
      ? "uploadFormat.joinTiffSizedOne"
      : "uploadFormat.joinTiffSized",
    { size },
  );
}

/** « Sans les TIFF : 479 Mo, cela tient. » under a refusal for room, when the
 * sending without the TIFF of its renders would fit; `null` otherwise. */
export function withoutTiffText(
  t: Translate,
  locale: string,
  quota: { toSendBytes: number; fullResolutionBytes?: number },
  remainingBytes: number,
): string | null {
  const tiff = quota.fullResolutionBytes ?? 0;
  const without = quota.toSendBytes - tiff;
  if (tiff <= 0 || without > remainingBytes) return null;
  return t("uploadFormat.withoutTiff", {
    size: formatSpaceSize(t, without, locale),
  });
}
