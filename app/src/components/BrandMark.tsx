import { useId } from "react";

import { BRAND_GEOMETRY as G, brandBranchPath } from "../vendor/brand-mark.js";

/**
 * The figure of the brand -- the ring crossed by the tilted eight-branch star
 * -- in the colour of the text around it. Drawn from the one geometry the
 * portal, the mobile applications and the icons of this application share
 * (`src/vendor/brand-mark.js`); nothing is redrawn here.
 */
export function BrandMark({ className }: { className?: string }) {
  const cutId = `as-brand-cut-${useId().replace(/:/g, "")}`;
  const long = brandBranchPath(G.longBranch, G.longHalfWidth);
  const short = brandBranchPath(G.shortBranch, G.shortHalfWidth);
  return (
    <svg
      viewBox="-80 -80 160 160"
      className={className}
      aria-hidden="true"
      focusable="false"
    >
      <mask
        id={cutId}
        maskUnits="userSpaceOnUse"
        x="-80"
        y="-80"
        width="160"
        height="160"
      >
        <rect x="-80" y="-80" width="160" height="160" fill="#fff" />
        <g
          transform={`rotate(${G.tilt})`}
          fill="#000"
          stroke="#000"
          strokeWidth={G.cut * 2}
          strokeLinejoin="round"
        >
          {[0, 90, 180, 270].map((angle) => (
            <path key={angle} d={long} transform={`rotate(${angle})`} />
          ))}
        </g>
      </mask>
      <circle
        r={G.ringRadius}
        fill="none"
        stroke="currentColor"
        strokeWidth={G.ringStroke}
        mask={`url(#${cutId})`}
      />
      <g transform={`rotate(${G.tilt})`} fill="currentColor">
        {Array.from({ length: 8 }, (_, index) => (
          <path
            key={index}
            d={index % 2 === 0 ? long : short}
            transform={`rotate(${index * 45})`}
          />
        ))}
        <circle r={G.core} />
      </g>
    </svg>
  );
}
