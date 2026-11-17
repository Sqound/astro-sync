// « Exporter pour partager »: the drawing of the copy. The image is drawn
// first, at (0, 0), whole; the band is drawn beneath it, from the image's
// last row down: not one pixel of the image is covered. The sizes are shares
// of the band's height, so the same drawing serves a 600 px copy and a
// 4 000 px one. The colours are the tokens of the application and the figure
// is the geometry the whole product shares (`vendor/brand-mark.js`).

import tokens from "../theme/tokens.json" with { type: "json" };
import { BRAND_GEOMETRY as G, brandBranchPath } from "../vendor/brand-mark.js";
import {
  BAND_ADDRESS,
  BAND_BRAND,
  type BandText,
  type CopyLayout,
  type ShareOptions,
} from "./shareBand.ts";

const COLOURS = tokens.colors;

/** The part of the 2D context the drawing uses (a canvas fits). */
export interface DrawContext {
  fillStyle: string | CanvasGradient | CanvasPattern;
  strokeStyle: string | CanvasGradient | CanvasPattern;
  lineWidth: number;
  lineJoin: CanvasLineJoin;
  font: string;
  textAlign: CanvasTextAlign;
  textBaseline: CanvasTextBaseline;
  globalCompositeOperation: GlobalCompositeOperation;
  save(): void;
  restore(): void;
  translate(x: number, y: number): void;
  scale(x: number, y: number): void;
  rotate(angle: number): void;
  fillRect(x: number, y: number, w: number, h: number): void;
  fillText(text: string, x: number, y: number): void;
  measureText(text: string): { width: number };
  beginPath(): void;
  arc(x: number, y: number, r: number, a: number, b: number): void;
  stroke(path?: Path2D): void;
  fill(path?: Path2D): void;
  drawImage(image: CanvasImageSource, ...args: number[]): void;
}

export interface DrawEnvironment {
  /** The family of the application's text (it covers every script). */
  fontFamily: string;
  /** A transparent layer to draw the figure on, then lay on the copy. */
  makeLayer: (size: number) => { image: CanvasImageSource; ctx: DrawContext };
  makePath: (data: string) => Path2D;
}

/** The figure: the ring cut where the long branches cross it, then the
 * tilted eight-branch star. `size` px square, top left at (x, y). */
export function drawBrandMark(
  ctx: DrawContext,
  x: number,
  y: number,
  size: number,
  colour: string,
  env: DrawEnvironment,
): void {
  const side = Math.max(32, Math.ceil(size * 2));
  const layer = env.makeLayer(side);
  const g = layer.ctx;
  g.translate(side / 2, side / 2);
  g.scale(side / G.box, side / G.box);
  g.strokeStyle = colour;
  g.lineWidth = G.ringStroke;
  g.beginPath();
  g.arc(0, 0, G.ringRadius, 0, Math.PI * 2);
  g.stroke();
  const long = env.makePath(brandBranchPath(G.longBranch, G.longHalfWidth));
  const short = env.makePath(brandBranchPath(G.shortBranch, G.shortHalfWidth));
  const tilt = (G.tilt * Math.PI) / 180;
  // The cut: the long branches, widened, wipe the ring.
  g.globalCompositeOperation = "destination-out";
  g.lineWidth = G.cut * 2;
  g.lineJoin = "round";
  for (const k of [0, 1, 2, 3]) {
    g.save();
    g.rotate(tilt + (k * Math.PI) / 2);
    g.fill(long);
    g.stroke(long);
    g.restore();
  }
  g.globalCompositeOperation = "source-over";
  g.fillStyle = colour;
  for (let i = 0; i < 8; i += 1) {
    g.save();
    g.rotate(tilt + (i * Math.PI) / 4);
    g.fill(i % 2 === 0 ? long : short);
    g.restore();
  }
  g.beginPath();
  g.arc(0, 0, G.core, 0, Math.PI * 2);
  g.fill();
  ctx.drawImage(layer.image, x, y, size, size);
}

/** The geometry of the band, from its height `h`. */
export function bandGeometry(h: number) {
  return {
    pad: Math.round(h * 0.55),
    big: h * 0.31,
    small: h * 0.235,
    rule: Math.max(2, Math.round(h * 0.014)),
    line1: h * 0.345,
    line2: h * 0.71,
  };
}

/** The smallest the text may shrink to before it is cut. */
const LEAST_SCALE = 0.55;

function setFont(
  ctx: DrawContext,
  weight: number,
  size: number,
  family: string,
): void {
  ctx.font = `${weight} ${Math.max(1, Math.round(size * 10) / 10)}px ${family}`;
}

/** `text` cut with an ellipsis so that it measures at most `room`. */
function fitText(ctx: DrawContext, text: string, room: number): string {
  if (ctx.measureText(text).width <= room) return text;
  let end = text.length;
  while (end > 1 && ctx.measureText(`${text.slice(0, end)}…`).width > room) {
    end -= 1;
  }
  return `${text.slice(0, end).trimEnd()}…`;
}

/** The mark's line, right-aligned at `right`, centred on `centre`: the
 * figure, the name and the address. */
function drawMark(
  ctx: DrawContext,
  right: number,
  centre: number,
  size: number,
  env: DrawEnvironment,
): void {
  const gap = size * 0.4;
  ctx.textAlign = "left";
  ctx.textBaseline = "middle";
  setFont(ctx, 400, size, env.fontFamily);
  const addressWidth = ctx.measureText(BAND_ADDRESS).width;
  setFont(ctx, 700, size, env.fontFamily);
  const brandWidth = ctx.measureText(BAND_BRAND).width;
  const figure = size * 1.9;
  const total = figure + gap + brandWidth + gap + addressWidth;
  let x = right - total;
  drawBrandMark(ctx, x, centre - figure / 2, figure, COLOURS.goldAccent, env);
  x += figure + gap;
  ctx.fillStyle = COLOURS.primaryText;
  ctx.fillText(BAND_BRAND, x, centre);
  x += brandWidth + gap;
  setFont(ctx, 400, size, env.fontFamily);
  ctx.fillStyle = COLOURS.mutedText;
  ctx.fillText(BAND_ADDRESS, x, centre);
}

/** The width the mark's line takes at font size `size`. */
function markWidth(ctx: DrawContext, size: number, family: string): number {
  setFont(ctx, 400, size, family);
  const address = ctx.measureText(BAND_ADDRESS).width;
  setFont(ctx, 700, size, family);
  return size * 1.9 + size * 0.8 + ctx.measureText(BAND_BRAND).width + address;
}

/** Draws the copy on a context of `layout.width` × `layout.height`: the
 * image whole at the top, then the band or the strip beneath it. */
export function drawShareCopy(
  ctx: DrawContext,
  image: CanvasImageSource,
  layout: CopyLayout,
  text: BandText,
  options: ShareOptions,
  env: DrawEnvironment,
): void {
  ctx.drawImage(image, 0, 0, layout.width, layout.imageHeight);
  if (layout.kind === "none") return;
  const top = layout.imageHeight;
  const h = layout.bandHeight;
  const width = layout.width;
  ctx.fillStyle = COLOURS.pageBackground;
  ctx.fillRect(0, top, width, h);

  if (layout.kind === "strip") {
    const rule = Math.max(2, Math.round(h * 0.03));
    ctx.fillStyle = COLOURS.border;
    ctx.fillRect(0, top, width, rule);
    const size = h * 0.46;
    drawMark(ctx, width - h * 0.6, top + rule + (h - rule) / 2, size, env);
    return;
  }

  const geo = bandGeometry(h);
  ctx.fillStyle = COLOURS.border;
  ctx.fillRect(0, top, width, geo.rule);
  const room = width - geo.pad * 2;
  ctx.textBaseline = "middle";

  // The right column: the integration, then the mark.
  const rightWidth = (scale: number) => {
    let widest = 0;
    if (text.integration) {
      setFont(ctx, 700, geo.big * scale, env.fontFamily);
      widest = ctx.measureText(text.integration).width;
    }
    if (options.mark) {
      widest = Math.max(
        widest,
        markWidth(ctx, geo.small * scale, env.fontFamily),
      );
    }
    return widest;
  };
  const leftWidth = (scale: number) => {
    let widest = 0;
    if (text.top) {
      setFont(ctx, 700, geo.big * scale, env.fontFamily);
      widest = ctx.measureText(text.top).width;
    }
    if (text.bottom) {
      setFont(ctx, 400, geo.small * scale, env.fontFamily);
      widest = Math.max(widest, ctx.measureText(text.bottom).width);
    }
    return widest;
  };
  let scale = 1;
  while (
    scale > LEAST_SCALE &&
    leftWidth(scale) + rightWidth(scale) + geo.pad > room
  ) {
    scale = Math.round((scale - 0.05) * 100) / 100;
  }
  const rightRoom = rightWidth(scale);
  const leftRoom = Math.max(0, room - rightRoom - geo.pad);
  const big = geo.big * scale;
  const small = geo.small * scale;

  ctx.textAlign = "left";
  if (text.top) {
    setFont(ctx, 700, big, env.fontFamily);
    ctx.fillStyle = COLOURS.primaryText;
    ctx.fillText(fitText(ctx, text.top, leftRoom), geo.pad, top + geo.line1);
  }
  if (text.bottom) {
    setFont(ctx, 400, small, env.fontFamily);
    ctx.fillStyle = COLOURS.mutedText;
    ctx.fillText(fitText(ctx, text.bottom, leftRoom), geo.pad, top + geo.line2);
  }
  ctx.textAlign = "right";
  if (text.integration) {
    setFont(ctx, 700, big, env.fontFamily);
    ctx.fillStyle = COLOURS.goldAccent;
    ctx.fillText(text.integration, width - geo.pad, top + geo.line1);
  }
  if (options.mark) {
    drawMark(
      ctx,
      width - geo.pad,
      top + (text.integration || text.top ? geo.line2 : h / 2),
      small,
      env,
    );
  }
}
