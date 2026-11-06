import { useState } from "react";

import { Notice } from "../../components/Notice";
import { useSnackbar } from "../../components/Snackbar";
import { useI18n } from "../../i18n";
import {
  deviceUnpair,
  spaceDetach,
  spaceRename,
  type DetachOutcome,
  type SpaceStatus,
} from "../../lib/bridge";
import { formatError } from "../../lib/format";
import {
  buttonClassName,
  confirmClassName,
  fieldClassName,
  fieldGroupClassName,
  fieldLabelClassName,
  helpTextClassName,
} from "../../theme";

/**
 * The name this computer wears in the space (editable, by default the name of
 * the machine). It moved here from the « Espace Astro Portal » screen: the
 * name of a computer is a setting of the computer, not of the space.
 */
export function DeviceName({
  status,
  onStatus,
}: {
  status: SpaceStatus;
  onStatus: (status: SpaceStatus) => void;
}) {
  const { t } = useI18n();
  // A private server signing in by e-mail code: « votre portail ».
  const portal = !status.hosted;
  const savedName = status.device?.name ?? status.defaultDeviceName;

  const [name, setName] = useState(savedName);
  const [busy, setBusy] = useState(false);
  // What passes is said by a snackbar.
  const { notify } = useSnackbar();
  const saySuccess = (text: string) => notify({ tone: "success", text });
  const [error, setError] = useState<string | null>(null);

  async function handleRename() {
    setError(null);
    setBusy(true);
    try {
      const outcome = await spaceRename(name);
      onStatus(outcome.status);
      setName(outcome.status.device?.name ?? name.trim());
      saySuccess(
        outcome.serviceUpdated
          ? t("space.connected.nameSaved")
          : t("space.connected.nameSavedHere"),
      );
    } catch (caught) {
      setError(formatError(t, caught));
    } finally {
      setBusy(false);
    }
  }

  return (
    <section>
      <Notice tone="error">{error}</Notice>
      <div className={fieldGroupClassName}>
        <label className={fieldLabelClassName} htmlFor="settings-device-name">
          {t("space.connected.deviceName")}
        </label>
        <input
          id="settings-device-name"
          className={fieldClassName}
          type="text"
          value={name}
          maxLength={120}
          onChange={(event) => setName(event.target.value)}
        />
        <p className={helpTextClassName}>
          {portal
            ? t("space.portal.deviceNameHelp")
            : t("space.connected.deviceNameHelp")}
        </p>
      </div>
      <p>
        <button
          type="button"
          className={buttonClassName("primary")}
          disabled={busy || name.trim() === "" || name.trim() === savedName}
          onClick={() => void handleRename()}
        >
          {t("space.connected.saveName")}
        </button>
      </p>
    </section>
  );
}

/**
 * « Détacher cet ordinateur »: a tertiary in the destructive tint (it keeps
 * the level its role gives it and is never promoted to draw attention), and
 * its confirmation, the one solid red block
 * of the window: the destructive primary. Detaching keeps the folders
 * associated with nights and the files waiting in the upload queue: they wait
 * for the next sign-in.
 *
 * Signed in to a space by e-mail code it ends the sign-in (`space_detach`);
 * paired with a private server by a code (`status.space` is `null`) it
 * unpairs (`device_unpair`), with the sentences of that flow.
 */
export function DetachDevice({
  status,
  onDetached,
  onUnpaired,
}: {
  status: SpaceStatus;
  onDetached: (outcome: DetachOutcome) => void;
  onUnpaired: () => void;
}) {
  const { t } = useI18n();
  const paired = status.paired;
  const bySpace = !!status.space;
  const portal = !status.hosted;
  const savedName = status.device?.name ?? status.defaultDeviceName;

  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  if (!paired) {
    return null;
  }

  async function handleDetach() {
    setConfirming(false);
    setError(null);
    setBusy(true);
    try {
      if (bySpace) {
        onDetached(await spaceDetach());
      } else {
        await deviceUnpair();
        onUnpaired();
      }
    } catch (caught) {
      setError(formatError(t, caught));
    } finally {
      setBusy(false);
    }
  }

  const buttonKey = bySpace ? "space.detach.button" : "settings.unpairButton";

  return (
    <section>
      <Notice tone="error">{error}</Notice>
      <p>
        <button
          type="button"
          className={buttonClassName("tertiary", { destructive: true })}
          disabled={busy}
          onClick={() => setConfirming(true)}
        >
          {t(buttonKey)}
        </button>
      </p>
      {confirming && (
        <div
          className={confirmClassName(true)}
          role="alertdialog"
          aria-modal="true"
        >
          {bySpace ? (
            <>
              <h4>{t("space.detach.title")}</h4>
              <p>
                <strong>{savedName}</strong>
              </p>
              <p>
                {portal
                  ? t("space.portal.detach.stops")
                  : t("space.detach.stops")}
              </p>
              <p>
                {portal
                  ? t("space.portal.detach.kept")
                  : t("space.detach.kept")}
              </p>
              <p>{t("space.detach.local")}</p>
            </>
          ) : (
            <>
              <h4>{t("settings.unpairConfirmTitle")}</h4>
              <p>{t("settings.unpairConfirmBody")}</p>
            </>
          )}
          <p>
            <button
              type="button"
              className={buttonClassName("primary", { destructive: true })}
              onClick={() => void handleDetach()}
            >
              {t(buttonKey)}
            </button>{" "}
            <button
              type="button"
              className={buttonClassName("link")}
              onClick={() => setConfirming(false)}
            >
              {t("common.cancel")}
            </button>
          </p>
        </div>
      )}
    </section>
  );
}
