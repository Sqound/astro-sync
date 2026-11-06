import { useId } from "react";

import {
  BRAND_GEOMETRY as G,
  brandLoaderCss,
  brandLoaderParts,
} from "../vendor/brand-mark.js";

// The loading indicator of the application: the ring of the icon with the star inside it. At each step the
// star turns a quarter turn and swells past the ring, which opens where a
// long branch crosses it, then comes back to rest. Still, at rest and tilted,
// when the person asks for « Réduire les animations » (the media query of the
// CSS). Drawn from the one geometry and the one animation of the brand mark
// (`src/vendor/brand-mark.js`, as `BrandMark.tsx` is); nothing is redrawn
// here. It takes the colour of the text around it.

const css = brandLoaderCss();
const LONG = [0, 90, 180, 270];

export function BrandLoader({
  className,
  size = 22,
}: {
  className?: string;
  size?: number;
}) {
  const mask = `as-loader-cut-${useId().replace(/[^A-Za-z0-9_-]/g, "")}`;
  return (
    <svg
      className={className}
      width={size}
      height={size}
      viewBox="-80 -80 160 160"
      fill="currentColor"
      aria-hidden="true"
      focusable="false"
      data-brand-loader=""
    >
      <style>{css}</style>
      <mask
        id={mask}
        maskUnits="userSpaceOnUse"
        x="-80"
        y="-80"
        width="160"
        height="160"
      >
        <rect x="-80" y="-80" width="160" height="160" fill="#fff" />
        <g
          className="brand-loader-pose"
          fill="#000"
          stroke="#000"
          strokeWidth={G.cut * 2}
          strokeLinejoin="round"
        >
          {LONG.map((angle) => (
            <path
              key={angle}
              d={brandLoaderParts.long}
              transform={`rotate(${angle})`}
            />
          ))}
        </g>
      </mask>
      <circle
        r={G.ringRadius}
        fill="none"
        stroke="currentColor"
        strokeWidth={G.ringStroke}
        mask={`url(#${mask})`}
      />
      <g className="brand-loader-pose">
        {Array.from({ length: 8 }, (_, index) => (
          <path
            key={index}
            d={index % 2 === 0 ? brandLoaderParts.long : brandLoaderParts.short}
            transform={`rotate(${index * 45})`}
          />
        ))}
        <circle r={G.core} />
      </g>
    </svg>
  );
}
