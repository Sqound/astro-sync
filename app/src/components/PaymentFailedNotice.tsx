import { useI18n } from "../i18n";
import {
  beforeSendingText,
  paymentFailedLine,
  paymentFailedNotice,
} from "../lib/standing";
import { badgeClassName, helpTextClassName } from "../theme";
import { usePaymentFailed } from "./CapabilitiesProvider";
import { Notice } from "./Notice";

/**
 * A payment that failed, said where Astro Sync says the state of the space: the
 * space is **read-only** for a while -- it keeps everything, nothing new is
 * accepted -- and what Astro Sync sends is held, not refused. Nothing is drawn
 * while the payment did not fail.
 *
 * - `state`: « Paiement en échec · lecture seule jusqu'au … » and the
 *   sentence of the space (the « Espace Astro Portal » page, the card of
 *   « Envois »).
 * - `beforeSending`: « Votre espace est en lecture seule jusqu'au … : rien ne
 *   sera envoyé. » -- before the last step of a night or of an import.
 */
export function PaymentFailedNotice({
  variant,
}: {
  variant: "state" | "beforeSending";
}) {
  const { t, locale } = useI18n();
  const window = usePaymentFailed();
  if (!window) {
    return null;
  }
  if (variant === "beforeSending") {
    return (
      <Notice tone="warning">{beforeSendingText(t, locale, window)}</Notice>
    );
  }
  return (
    <div data-standing="paymentFailed">
      <p>
        <span className={badgeClassName("warning")}>
          {paymentFailedLine(t, locale, window)}
        </span>
      </p>
      <p className={helpTextClassName}>
        {paymentFailedNotice(t, locale, window)}
      </p>
    </div>
  );
}
