// The two files of a render, as the window says them (`render_files` of the
// core): a render written as PNG and TIFF leaves as its PNG; its TIFF -- the
// full resolution, only used to download the image -- leaves only when the
// person ticks « Joindre aussi les TIFF ». A render that exists only as a TIFF
// (« Sans traitement ») leaves as it is. Pure TypeScript, tested under
// `node --test`.

import type { TranslateParams } from "../i18n/locale.ts";
import { formatSize } from "./written.ts";

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

/** The label of the box: « Joindre aussi les TIFF (pleine résolution, ≈ 47 Mo
 * chacun) » when the TIFF are known, else without their weight (the renders
 * are made at the sending). */
export function joinTiffLabel(
  t: Translate,
  locale: string,
  fullResolution: { count: number; bytes: number } | null,
): string {
  return fullResolution && fullResolution.count > 0
    ? t("uploadFormat.joinTiffSized", {
        size: formatSize(
          t,
          Math.round(fullResolution.bytes / fullResolution.count),
          locale,
        ),
      })
    : t("uploadFormat.joinTiff");
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
    size: formatSize(t, without, locale),
  });
}
