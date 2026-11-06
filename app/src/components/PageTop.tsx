import type { ReactNode } from "react";

/**
 * The head of a screen: the way back and the title, with what belongs to
 * them (the description, the tabs, the screen's buttons). It stays at the
 * top of the window while the content scrolls under it.
 */
export function PageTop({ children }: { children: ReactNode }) {
  return <header className="as-page__top">{children}</header>;
}
