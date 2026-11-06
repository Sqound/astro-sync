import { PaymentFailedNotice } from "../../components/PaymentFailedNotice";
import { SendingHeldNotice } from "../../components/SendingHeldNotice";
import { useI18n } from "../../i18n";
import { openInBrowser, type SpaceStatus } from "../../lib/bridge";
import { type AnalysisSendingNotice } from "../../lib/capabilities";
import { hostOf } from "../../lib/space";
import {
  badgeClassName,
  buttonClassName,
  fieldGroupClassName,
  fieldLabelClassName,
  helpTextClassName,
} from "../../theme";

/**
 * Signed in to a space by e-mail code: the address it signed in with, the
 * address of the space on the web, and what the space receives from Astro
 * Sync. The name of this computer and « Détacher cet ordinateur » are in
 * « Réglages » (`DeviceSettings.tsx`).
 *
 * Right after a sign-in that created the space it opens as « Votre espace est
 * prêt ».
 *
 * Signed in to a private server (`status.hosted` is false: its e-mail-code
 * mode), the same screen says « votre portail », creates nothing, and names
 * no offer.
 */
export function ConnectedSpace({
  status,
  created,
  analysisNotice,
  onAcknowledge,
}: {
  status: SpaceStatus;
  /** This sign-in created the space. */
  created: boolean;
  /** The line that says the analysis is here and free, while the service does
   * not keep its reports (`null`: nothing to say). */
  analysisNotice: AnalysisSendingNotice | null;
  onAcknowledge: () => void;
}) {
  const { t } = useI18n();
  const space = status.space;
  // A private server signing in by e-mail code: « votre portail ».
  const portal = !status.hosted;

  if (!space) {
    return null;
  }

  return (
    <>
      <p>
        <span className={badgeClassName("success")}>
          {t("space.connected.badge")}
        </span>
      </p>
      {portal && (
        <p className={helpTextClassName}>{t("space.portal.connectedLine")}</p>
      )}
      {/* A payment that failed: the space is read-only for 90 days (a private
          server sells nothing and has no such state). */}
      {!portal && <PaymentFailedNotice variant="state" />}
      {created && !portal && <h3>{t("space.connected.ready")}</h3>}
      <div className={fieldGroupClassName}>
        <span className={fieldLabelClassName}>
          {t("space.connected.signInAddress")}
        </span>
        <strong>{space.email}</strong>
      </div>

      <h4>{portal ? t("space.portal.web") : t("space.connected.web")}</h4>
      <p>
        <strong>{hostOf(space.webUrl)}</strong>
      </p>
      <p className={helpTextClassName}>{t("space.connected.webHelp")}</p>
      <p>
        <button
          type="button"
          className={buttonClassName("tertiary")}
          onClick={() => void openInBrowser(space.webUrl)}
        >
          {t("space.connected.openWeb")}
        </button>
      </p>

      <h4>
        {portal ? t("space.portal.receives") : t("space.connected.receives")}
      </h4>
      <p>{t("space.connected.receivesCaptures")}</p>
      <p className={helpTextClassName}>{t("space.connected.receivesFind")}</p>
      <p className={helpTextClassName}>{t("space.connected.method")}</p>
      {analysisNotice && <SendingHeldNotice notice={analysisNotice} />}
      {created && !portal && (
        <p>
          <button
            type="button"
            className={buttonClassName("primary")}
            onClick={onAcknowledge}
          >
            {t("space.connected.start")}
          </button>
        </p>
      )}
    </>
  );
}
