import type { ReactNode } from "react";

import { noticeClassName, type BadgeTone } from "../theme";
import { Icon, type IconName } from "./Icon";

/**
 * The one shared success/warning/error/neutral banner every screen uses so
 * these four states stay visually and semantically consistent across the
 * app ("états succès/avertissement/erreur cohérents"). Renders nothing
 * when there is no message, so screens can pass a possibly-null string
 * directly.
 *
 * With an `icon` the banner leads with it (`info`: « ⓘ »): the message that
 * informs, tinted like the others, never a warning.
 */
export function Notice({
  tone,
  icon,
  children,
}: {
  tone: BadgeTone;
  icon?: IconName;
  children: ReactNode;
}) {
  if (!children) {
    return null;
  }
  return (
    <div
      className={[
        noticeClassName(tone),
        icon ? "as-notice--icon" : "",
        // An error or a warning is copied as it is read, to be sent or looked up.
        tone === "error" || tone === "warning" ? "as-selectable" : "",
      ]
        .filter(Boolean)
        .join(" ")}
      role={tone === "error" ? "alert" : "status"}
    >
      {icon && <Icon name={icon} className="as-notice__icon" />}
      {icon ? <div className="as-notice__body">{children}</div> : children}
    </div>
  );
}
