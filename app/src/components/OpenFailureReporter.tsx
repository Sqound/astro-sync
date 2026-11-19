import { useEffect } from "react";

import { useI18n } from "../i18n";
import { openFailureText, setOpenFailureReporter } from "../lib/openFailure";
import { useSnackbar } from "./Snackbar";

/** Says, in a snackbar, every opening of a folder, a file or a link that the
 * system refused (« Ouvrir le dossier de travail » and its kin): a click is
 * never answered by nothing. Renders nothing. */
export function OpenFailureReporter() {
  const { t } = useI18n();
  const { notify } = useSnackbar();
  useEffect(
    () =>
      setOpenFailureReporter((failure) => {
        const { text, detail } = openFailureText(t, failure);
        notify({ tone: "error", key: "open-failure", text, detail });
      }),
    [t, notify],
  );
  return null;
}
