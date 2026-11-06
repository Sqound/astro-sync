import { useEffect } from "react";

import { useI18n } from "../i18n";
import { useServiceWaiting } from "../hooks/useServiceWaiting";
import { serviceCancel } from "../lib/bridge";
import { useSnackbar } from "./Snackbar";

/** The key of the one snackbar of the service's wait. */
export const SERVICE_WAIT_SNACK = "service-wait";

/**
 * Says, in plain words, that the service is starting -- a server that has
 * been idle can take a long time to answer the first request -- and
 * offers to cancel the wait. A snackbar at the bottom of the window (never a
 * banner that pushes the screen down), whatever the screen, because any
 * screen's request can be the one that wakes the service; it goes when the
 * service answers. Renders nothing itself.
 */
export function ServiceWaitBanner() {
  const { t } = useI18n();
  const { waiting, seconds } = useServiceWaiting();
  const { notify, dismissKey } = useSnackbar();

  useEffect(() => {
    if (!waiting) {
      dismissKey(SERVICE_WAIT_SNACK);
      return;
    }
    notify({
      key: SERVICE_WAIT_SNACK,
      tone: "info",
      mode: "persistent",
      text: `${t("service.starting")} ${t("service.startingElapsed", { seconds })}`,
      action: { label: t("common.cancel"), run: () => void serviceCancel() },
    });
  }, [waiting, seconds, t, notify, dismissKey]);

  return null;
}
