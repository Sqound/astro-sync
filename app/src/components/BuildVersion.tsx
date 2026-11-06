import { useEffect, useState } from "react";

import { useI18n } from "../i18n";
import { appBuildInfo, type BuildInfo } from "../lib/bridge";
import { menuVersionLine, menuVersionTitle } from "../lib/buildInfo";

/** The build this window runs (`app_build_info`), `null` until known. A
 * failure leaves it `null`: the version line is simply not shown. */
export function useBuildInfo(): BuildInfo | null {
  const [info, setInfo] = useState<BuildInfo | null>(null);
  useEffect(() => {
    let cancelled = false;
    appBuildInfo()
      .then((loaded) => !cancelled && setInfo(loaded))
      .catch(() => {
        // No build stamp (a plain browser preview): nothing to show.
      });
    return () => {
      cancelled = true;
    };
  }, []);
  return info;
}

/** « Astro Sync 1.0.0 · 6aa28f767 » at the foot of the menu, the build
 * date on hover. */
export function BuildVersion() {
  const { t, locale } = useI18n();
  const info = useBuildInfo();
  if (info === null) return null;
  return (
    <p
      className="as-sidebar__version"
      title={menuVersionTitle(t, info, locale)}
    >
      {menuVersionLine(t("app.title"), info)}
    </p>
  );
}
