// « Exporter pour partager »: the copy made in the window. The image (the
// picture the card shows) is drawn on a canvas, the band beneath it
// (`shareBandDraw.ts`), and the result is a PNG written where the person
// chose, through the core's guarded writer. The preview of the sheet is the
// same drawing, scaled: what is shown is what is written.

import {
  copyLayout,
  type BandText,
  type CopyLayout,
  type ShareOptions,
} from "./shareBand.ts";
import {
  drawShareCopy,
  type DrawContext,
  type DrawEnvironment,
} from "./shareBandDraw.ts";
import { platformFontFaces, themeFontFamily } from "../theme/tokens.ts";

export function drawingEnvironment(): DrawEnvironment {
  // The family the window set for its text (it follows the language, so
  // Chinese reads in its own script's faces); else the stack of the theme.
  const set =
    typeof document !== "undefined"
      ? getComputedStyle(document.documentElement)
          .getPropertyValue("--as-font-family")
          .trim()
      : "";
  const family =
    set ||
    platformFontFaces(
      typeof navigator !== "undefined" ? navigator.userAgent : "",
    ) + themeFontFamily();
  return {
    fontFamily: family,
    makeLayer: (size) => {
      const canvas = document.createElement("canvas");
      canvas.width = size;
      canvas.height = size;
      return {
        image: canvas,
        ctx: canvas.getContext("2d") as unknown as DrawContext,
      };
    },
    makePath: (data) => new Path2D(data),
  };
}

/** The layout of the copy of `image`. */
export function layoutOf(
  image: { width: number; height: number },
  text: BandText,
  options: ShareOptions,
): CopyLayout {
  return copyLayout(image.width, image.height, text, options);
}

/** The copy drawn on `canvas`, scaled to `shownWidth` px of width (the
 * sheet's preview); the file is the same drawing at full size. */
export function drawPreview(
  canvas: HTMLCanvasElement,
  image: ImageBitmap,
  text: BandText,
  options: ShareOptions,
  shownWidth: number,
): CopyLayout {
  const layout = layoutOf(image, text, options);
  const factor = Math.min(1, shownWidth / layout.width);
  canvas.width = Math.max(1, Math.round(layout.width * factor));
  canvas.height = Math.max(1, Math.round(layout.height * factor));
  const ctx = canvas.getContext("2d");
  if (!ctx) return layout;
  ctx.imageSmoothingQuality = "high";
  ctx.scale(canvas.width / layout.width, canvas.height / layout.height);
  drawShareCopy(
    ctx as unknown as DrawContext,
    image,
    layout,
    text,
    options,
    drawingEnvironment(),
  );
  return layout;
}

/** The copy at full size, as PNG bytes (a canvas writes no metadata; the
 * core removes any other anyway). */
export async function renderCopy(
  image: ImageBitmap,
  text: BandText,
  options: ShareOptions,
): Promise<ArrayBuffer> {
  const layout = layoutOf(image, text, options);
  const canvas = document.createElement("canvas");
  canvas.width = layout.width;
  canvas.height = layout.height;
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("no 2D context");
  drawShareCopy(
    ctx as unknown as DrawContext,
    image,
    layout,
    text,
    options,
    drawingEnvironment(),
  );
  const blob = await new Promise<Blob | null>((resolve) =>
    canvas.toBlob(resolve, "image/png"),
  );
  canvas.width = 0;
  if (!blob) throw new Error("the copy could not be encoded");
  return blob.arrayBuffer();
}
