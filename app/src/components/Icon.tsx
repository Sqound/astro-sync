import type { SVGProps } from "react";

import { iconClassName } from "../theme";

/** The drawings of the home: 24-unit
 * boxes, a 1.8 stroke with round ends, in the colour of the text around. */
const PATHS = {
  person: (
    <>
      <circle cx="12" cy="9" r="3.5" />
      <path d="M5 20a7 7 0 0 1 14 0" />
    </>
  ),
  check: <path d="M5 12.5l4.5 4.5L19 7.5" />,
  folder: <path d="M3 6h6l2 2h10v11H3z" />,
  upload: <path d="M12 20V9M7 13l5-5 5 5M5 4h14" />,
  chart: <path d="M4 20V4M4 20h16M8 16v-5M12 16V8M16 16v-3" />,
  list: <path d="M8 6h12M8 12h12M8 18h12M4 6h.01M4 12h.01M4 18h.01" />,
  sliders: (
    <>
      <path d="M4 7h10M18 7h2M4 17h2M10 17h10" />
      <circle cx="16" cy="7" r="2" />
      <circle cx="8" cy="17" r="2" />
    </>
  ),
  gear: (
    <>
      <circle cx="12" cy="12" r="3" />
      <path d="M12 3v3M12 18v3M3 12h3M18 12h3M5.6 5.6l2.1 2.1M16.3 16.3l2.1 2.1M18.4 5.6l-2.1 2.1M7.7 16.3l-2.1 2.1" />
    </>
  ),
  chevron: <path d="M9 6l6 6-6 6" />,
  info: (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="M12 11v5" />
      <path d="M12 8h.01" />
    </>
  ),
  link: (
    <>
      <path d="M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1" />
      <path d="M14 10a4 4 0 0 0-5.7 0l-3 3A4 4 0 0 0 10 18.7l1-1" />
    </>
  ),
  lock: (
    <>
      <rect x="5" y="11" width="14" height="9" rx="2" />
      <path d="M8 11V8a4 4 0 0 1 8 0v3" />
    </>
  ),
  clock: (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="M12 7v5l3 2" />
    </>
  ),
  book: <path d="M6 4h11v16H6a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1zM8 4v16" />,
  plus: <path d="M12 5v14M5 12h14" />,
  external: <path d="M14 4h6v6M20 4l-9 9M18 14v6H4V6h6" />,
  copy: (
    <>
      <rect x="8" y="8" width="12" height="12" rx="2" />
      <path d="M16 8V5a1 1 0 0 0-1-1H5a1 1 0 0 0-1 1v10a1 1 0 0 0 1 1h3" />
    </>
  ),
  sparkle: (
    <path d="M12 3l1.8 5.2L19 10l-5.2 1.8L12 17l-1.8-5.2L5 10l5.2-1.8zM19 16l.8 2.2L22 19l-2.2.8L19 22l-.8-2.2L16 19l2.2-.8z" />
  ),
  refresh: <path d="M20 11a8 8 0 1 0-2.3 5.7M20 4v7h-7" />,
  warning: (
    <>
      <path d="M12 4l9 16H3z" />
      <path d="M12 10v4M12 17h.01" />
    </>
  ),
  cross: (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="M9 9l6 6M15 9l-6 6" />
    </>
  ),
  pending: <circle cx="12" cy="12" r="8" strokeDasharray="2.5 2.5" />,
  // « nothing to do », a plain circle crossed by a bar.
  none: (
    <>
      <circle cx="12" cy="12" r="8" />
      <path d="M8.5 12h7" />
    </>
  ),
  phone: (
    <>
      <rect x="7" y="3" width="10" height="18" rx="2" />
      <path d="M11 18h2" />
    </>
  ),
  variant: <path d="M20 12a8 8 0 1 1-2.3-5.7M20 4v5h-5" />,
  back: <path d="M15 6l-6 6 6 6" />,
  // « Cibles »: the catalogue's search, a capture's file.
  search: (
    <>
      <circle cx="11" cy="11" r="6" />
      <path d="M20 20l-4.5-4.5" />
    </>
  ),
  file: <path d="M6 3h8l4 4v14H6zM14 3v4h4" />,
  // The small cross that closes a snackbar or a window.
  close: <path d="M7 7l10 10M17 7L7 17" />,
  pencil: <path d="M4 20h4L19 9l-4-4L4 16zM13.5 6.5l4 4" />,
  // « Poser une question à une IA » (a speech bubble).
  chat: <path d="M5 5h14v10h-8l-4 4v-4H5zM9 9h6M9 12h4" />,
  trash: <path d="M4 7h16M9 7V4.5h6V7M6.5 7l1 13h9l1-13M10 11v6M14 11v6" />,
} as const;

export type IconName = keyof typeof PATHS;

export function Icon({
  name,
  className = iconClassName,
  strokeWidth = 1.8,
  ...rest
}: { name: IconName; strokeWidth?: number } & Omit<
  SVGProps<SVGSVGElement>,
  "children"
>) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={strokeWidth}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      className={className}
      {...rest}
    >
      {PATHS[name]}
    </svg>
  );
}
