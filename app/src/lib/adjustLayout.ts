// « Régler l'image »: the arithmetic of the layout. The preview is never
// cropped: it is drawn at the largest size that fits the box the window leaves
// it, whatever the shape of the image or of the box. Pure TypeScript, tested
// under `node --test`.

export interface Size {
  width: number;
  height: number;
}

/** The largest size of an image `width` × `height` that fits in a box
 * `boxWidth` × `boxHeight`, its proportions kept (never cropped); a box or an
 * image without size gives no size. */
export function fitSize(
  boxWidth: number,
  boxHeight: number,
  width: number,
  height: number,
): Size {
  if (!(boxWidth > 0 && boxHeight > 0 && width > 0 && height > 0)) {
    return { width: 0, height: 0 };
  }
  const scale = Math.min(boxWidth / width, boxHeight / height);
  return {
    width: Math.floor(width * scale),
    height: Math.floor(height * scale),
  };
}

/** The window width (in rem) under which the panel goes under the image. */
export const NARROW_REM = 48;

/** The width (in rem) of the panel from which its sliders take two columns. */
export const PANEL_TWO_COLUMNS_REM = 30;

/** The share of the height the image keeps in the narrow layout. */
export const NARROW_IMAGE_SHARE = 0.4;
