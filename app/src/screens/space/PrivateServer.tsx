import { useEffect, useState } from "react";

import { Notice } from "../../components/Notice";
import { useI18n } from "../../i18n";
import { devicePair, settingsGet, type SpaceStatus } from "../../lib/bridge";
import { formatError } from "../../lib/format";
import {
  badgeClassName,
  buttonClassName,
  fieldClassName,
  fieldGroupClassName,
  fieldLabelClassName,
  helpTextClassName,
} from "../../theme";

/**
 * The private server's way in, exactly as it was: the address of the server,
 * then the eight-character code its account page shows (`device_pair`) -- no
 * e-mail address. Shown once the server has announced `pairing`.
 */
export function PairingForm({
  portalUrl,
  onPaired,
}: {
  portalUrl: string;
  onPaired: () => void;
}) {
  const { t } = useI18n();
  const [code, setCode] = useState("");
  const [label, setLabel] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // As before: the label already saved for this computer, if any -- nothing
  // is proposed for a computer that never had one.
  useEffect(() => {
    let cancelled = false;
    settingsGet()
      .then((settings) => {
        if (!cancelled) {
          setLabel((typed) => typed || (settings.deviceLabel ?? ""));
        }
      })
      .catch(() => {
        // The field simply stays empty.
      });
    return () => {
      cancelled = true;
    };
  }, []);

  async function handlePair() {
    setError(null);
    setBusy(true);
    try {
      await devicePair(portalUrl.trim(), code.trim(), label.trim());
      setCode("");
      onPaired();
    } catch (caught) {
      setError(formatError(t, caught));
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <h4>{t("settings.pairingTitle")}</h4>
      <p className={helpTextClassName}>{t("settings.pairingDescription")}</p>
      <Notice tone="error">{error}</Notice>
      <div className={fieldGroupClassName}>
        <label className={fieldLabelClassName} htmlFor="space-pairing-code">
          {t("settings.pairingCodeLabel")}
        </label>
        <input
          id="space-pairing-code"
          className={fieldClassName}
          type="text"
          value={code}
          placeholder={t("settings.pairingCodePlaceholder")}
          onChange={(event) => setCode(event.target.value)}
        />
      </div>
      <div className={fieldGroupClassName}>
        <label className={fieldLabelClassName} htmlFor="space-pairing-label">
          {t("settings.deviceLabelLabel")}
        </label>
        <input
          id="space-pairing-label"
          className={fieldClassName}
          type="text"
          value={label}
          placeholder={t("settings.deviceLabelPlaceholder")}
          onChange={(event) => setLabel(event.target.value)}
        />
      </div>
      <p>
        <button
          type="button"
          className={buttonClassName("primary")}
          onClick={() => void handlePair()}
          disabled={busy}
        >
          {t("settings.pairButton")}
        </button>
      </p>
    </>
  );
}

/**
 * A device paired with a private server: the badge and the address. The way
 * out (`device_unpair`), which keeps the upload queue and the local
 * associations, is « Détacher cet ordinateur » in « Réglages »
 * (`DeviceSettings.tsx`).
 */
export function PrivateConnected({ status }: { status: SpaceStatus }) {
  const { t } = useI18n();
  return (
    <>
      <h3>{t("space.private.title")}</h3>
      <p>
        <span className={badgeClassName("success")}>
          {t("settings.pairedBadge")}
        </span>
      </p>
      <p>{t("space.private.connectedTo", { url: status.serviceUrl })}</p>
      <p className={helpTextClassName}>{t("space.private.description")}</p>
    </>
  );
}
