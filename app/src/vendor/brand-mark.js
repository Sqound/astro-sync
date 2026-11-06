// The figure of the brand: the gold ring of the icon, crossed by the
// eight-branch star, tilted 22 degrees clockwise. One figure for the icon of
// the portal (favicon and header) and for its loading indicator
// (`components/BrandLoader.js`).
//
// A 160-unit box centred on the origin. Only the ring is cut where a long
// branch crosses it (the branch's outline widened by `cut` units); the star
// itself has no outline.

export const BRAND_GEOMETRY = Object.freeze({
  box: 160,
  ringRadius: 36,
  ringStroke: 10.3,
  longBranch: 78,
  shortBranch: 21,
  longHalfWidth: 8,
  shortHalfWidth: 5.3,
  core: 7.9,
  cut: 3,
  tilt: 22,
  // The loader: the star rests inside the closed ring, then, on each step,
  // turns a quarter turn and swells past the ring before coming back.
  restScale: 0.33,
  peakScale: 1,
  stepDegrees: 90,
  stepSeconds: 1.3,
  pauseSeconds: 0.25,
});

export const BRAND_GOLD = "#d7a24e";
export const BRAND_NIGHT_RED = "#8e2f24";
export const BRAND_TILE = "#070a08";

const G = BRAND_GEOMETRY;

/** One branch of length `length` and half-width `half`, tip up. */
export const brandBranchPath = (length, half) =>
  `M${-half} 0Q${-half * 0.12} ${-length * 0.2} 0 ${-length}Q${half * 0.12} ${-length * 0.2} ${half} 0Z`;

/** Accelerates, then slows down: the cubic ease-in-out. */
export function brandEaseInOutCubic(x) {
  return x < 0.5 ? 4 * x * x * x : 1 - Math.pow(-2 * x + 2, 3) / 2;
}

/**
 * The loader at `seconds` from its start: the star's angle in degrees
 * (clockwise, the tilt included) and its scale. During a step the angle
 * goes from the tilt to the tilt plus a quarter turn (cubic ease-in-out) and
 * the scale is a half sine from the rest to the peak and back; during the
 * pause it rests. The star has the symmetry of a quarter turn, so every
 * period starts again from the tilt without a visible seam.
 */
export function brandLoaderPose(seconds) {
  const period = G.stepSeconds + G.pauseSeconds;
  const t = ((seconds % period) + period) % period;
  if (t >= G.stepSeconds)
    return { rotation: G.tilt + G.stepDegrees, scale: G.restScale };
  const x = t / G.stepSeconds;
  return {
    rotation: G.tilt + G.stepDegrees * brandEaseInOutCubic(x),
    scale: G.restScale + (G.peakScale - G.restScale) * Math.sin(Math.PI * x),
  };
}

/** The still figure of reduced motion: at rest, tilted. */
export const brandLoaderStillPose = Object.freeze({
  rotation: G.tilt,
  scale: G.restScale,
});

const star = () =>
  Array.from({ length: 8 }, (_, i) => {
    const long = i % 2 === 0;
    return `<path d="${brandBranchPath(long ? G.longBranch : G.shortBranch, long ? G.longHalfWidth : G.shortHalfWidth)}" transform="rotate(${i * 45})"/>`;
  }).join("") + `<circle r="${G.core}"/>`;

const cuts = () =>
  Array.from(
    { length: 4 },
    (_, i) =>
      `<path d="${brandBranchPath(G.longBranch, G.longHalfWidth)}" transform="rotate(${i * 90})"/>`,
  ).join("");

/**
 * The still figure as an SVG file: `tile` "rounded" (the icon), "square"
 * (a full-bleed square, for a platform that rounds the corners itself) or
 * none; `viewBox` widens the canvas around the 160-unit box (a launcher's
 * safe zone).
 */
export function brandMarkSvg({
  id,
  color = BRAND_GOLD,
  tile = null,
  viewBox = "-80 -80 160 160",
}) {
  const tilt = ` transform="rotate(${G.tilt})"`;
  const wrap = (inner, attrs) => `<g${tilt}${attrs}>${inner}</g>`;
  const ground =
    tile === "rounded"
      ? `<rect x="-80" y="-80" width="160" height="160" rx="35" fill="${BRAND_TILE}"/>`
      : tile === "square"
        ? `<rect x="-80" y="-80" width="160" height="160" fill="${BRAND_TILE}"/>`
        : "";
  return (
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="${viewBox}" role="img" aria-label="PlanAstro">` +
    ground +
    `<mask id="planastro-cut-${id}" maskUnits="userSpaceOnUse" x="-80" y="-80" width="160" height="160"><rect x="-80" y="-80" width="160" height="160" fill="#fff"/>` +
    wrap(
      cuts(),
      ` fill="#000" stroke="#000" stroke-width="${G.cut * 2}" stroke-linejoin="round"`,
    ) +
    `</mask>` +
    `<circle r="${G.ringRadius}" fill="none" stroke="${color}" stroke-width="${G.ringStroke}" mask="url(#planastro-cut-${id})"/>` +
    wrap(star(), ` fill="${color}"`) +
    `</svg>\n`
  );
}

const round = (value, digits) => Number(value.toFixed(digits));

/**
 * The CSS of the loader: one keyframe every 50 ms of the step, sampled from
 * `brandLoaderPose` (linear in between), the rest until the end of the
 * period, and the still figure when the person asks for reduced motion.
 * `selector` is the class of the two groups that move together (the star and
 * the cut of the ring).
 */
export function brandLoaderCss(selector = "brand-loader-pose") {
  const period = G.stepSeconds + G.pauseSeconds;
  const samples = Math.round(G.stepSeconds / 0.05);
  const pose = ({ rotation, scale }) =>
    `transform:rotate(${round(rotation, 3)}deg) scale(${round(scale, 4)})`;
  const stops = [];
  for (let index = 0; index <= samples; index += 1) {
    const seconds = (G.stepSeconds * index) / samples;
    const at =
      index === samples
        ? { rotation: G.tilt + G.stepDegrees, scale: G.restScale }
        : brandLoaderPose(seconds);
    stops.push(`${round((seconds / period) * 100, 3)}%{${pose(at)}}`);
  }
  stops.push(
    `100%{${pose({ rotation: G.tilt + G.stepDegrees, scale: G.restScale })}}`,
  );
  return (
    `.${selector}{${pose(brandLoaderStillPose)};animation:${selector} ${period}s linear infinite}` +
    `@keyframes ${selector}{${stops.join("")}}` +
    `@media (prefers-reduced-motion:reduce){.${selector}{animation:none}}`
  );
}

/** The loader's ring, star and cut, for a component to lay out. */
export const brandLoaderParts = Object.freeze({
  long: brandBranchPath(G.longBranch, G.longHalfWidth),
  short: brandBranchPath(G.shortBranch, G.shortHalfWidth),
});
