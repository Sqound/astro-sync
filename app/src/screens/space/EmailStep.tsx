import { useState, type FormEvent } from "react";

import { Icon } from "../../components/Icon";
import { InfoLine } from "../../components/InfoLine";
import { Notice } from "../../components/Notice";
import { useI18n } from "../../i18n";
import {
  openInBrowser,
  spaceRequestCode,
  type CodeRequested,
  type SignInAnnouncement,
  type SignInEndedNotice,
} from "../../lib/bridge";
import {
  endedMessageKey,
  isClosedPortal,
  legalAddress,
  legalTemplateKey,
  looksLikeEmail,
  messageLanguage,
  signInErrorText,
  splitTemplate,
} from "../../lib/space";
import {
  buttonClassName,
  fieldClassName,
  fieldGroupClassName,
  fieldLabelClassName,
  helpTextClassName,
  linkClassName,
  cardHeadingClassName,
  cardTextClassName,
} from "../../theme";

type EmailCodeAnnouncement = Extract<
  SignInAnnouncement,
  { method: "email-code" }
>;

/** « En continuant, vous acceptez les conditions d'utilisation et la
 * politique de confidentialité d'Astro Portal. » -- only the links the service
 * has configured, each opening the system browser. */
function LegalLine({ announcement }: { announcement: EmailCodeAnnouncement }) {
  const { t, locale } = useI18n();
  const key = legalTemplateKey(announcement);
  if (key === null) {
    return null;
  }
  const targets: Record<string, { label: string; url: string | null }> = {
    terms: {
      label: t("space.legal.terms"),
      url: legalAddress(announcement, "terms", locale),
    },
    privacy: {
      label: t("space.legal.privacy"),
      url: legalAddress(announcement, "privacy", locale),
    },
  };
  return (
    <p className={helpTextClassName}>
      {splitTemplate(t(key)).map((part, index) => {
        if ("text" in part) {
          return <span key={index}>{part.text}</span>;
        }
        const target = targets[part.placeholder];
        if (!target) {
          return <span key={index}>{`{${part.placeholder}}`}</span>;
        }
        const { label, url } = target;
        return url === null ? (
          <span key={index}>{label}</span>
        ) : (
          <button
            key={index}
            type="button"
            className={linkClassName}
            onClick={() => void openInBrowser(url)}
          >
            {label}
          </button>
        );
      })}
    </p>
  );
}

/**
 * « Créer votre espace ou vous connecter »: an address, then « Recevoir mon
 * code de connexion ». There is one entry for creating a space and for signing
 * in: an address without a space gets one at its first successful
 * verification. No password.
 *
 * On a private server signing in by e-mail code (`closed`) the same form says
 * « Vous connecter à votre portail »: no creation of a space, no legal line,
 * and the owner's address is the only one that receives a code.
 */
export function EmailStep({
  announcement,
  portalUrl,
  initialEmail,
  ended,
  onCodeSent,
  onOwnServer,
}: {
  announcement: EmailCodeAnnouncement;
  /** A private server's address; `null` for the hosted service. */
  portalUrl: string | null;
  initialEmail: string;
  /** The service ended the last sign-in: what the sentence says. */
  ended: SignInEndedNotice | null;
  onCodeSent: (email: string, requested: CodeRequested) => void;
  /** Offered on the hosted service only. */
  onOwnServer?: () => void;
}) {
  const { t, locale } = useI18n();
  const closed = isClosedPortal(announcement);
  const [email, setEmail] = useState(initialEmail);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handleSubmit(event: FormEvent) {
    event.preventDefault();
    if (busy) {
      return;
    }
    // The button is never greyed out before the address is typed (a gold
    // button that looks disabled was finding A1): an address that is not one
    // is said when it is sent.
    if (!looksLikeEmail(email)) {
      setError(t("errors.email_invalid"));
      return;
    }
    setError(null);
    setBusy(true);
    try {
      const requested = await spaceRequestCode(
        portalUrl,
        email,
        messageLanguage(locale),
      );
      onCodeSent(requested.email, requested);
    } catch (caught) {
      setError(signInErrorText(t, caught, Date.now(), locale));
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      {ended && (
        <Notice tone="warning">
          {t(endedMessageKey(ended.reason, closed))}
        </Notice>
      )}
      {closed ? (
        <h4>{t("space.portal.formTitle")}</h4>
      ) : (
        <>
          <h2 className={cardHeadingClassName}>
            {t("space.intro.createOrSignIn")}
          </h2>
          <p className={cardTextClassName}>{t("space.intro.sendsAndFinds")}</p>
          <p className={cardTextClassName}>{t("space.intro.names")}</p>
          <InfoLine
            line={t("space.intro.withoutSpaceShort")}
            details={
              <>
                <span>{t("space.intro.withoutSpace")}</span>
                <span>{t("space.email.helpMore")}</span>
              </>
            }
          />
        </>
      )}
      <form onSubmit={(event) => void handleSubmit(event)}>
        <div className={fieldGroupClassName}>
          <label className={fieldLabelClassName} htmlFor="space-email">
            {t("space.email.label")}
          </label>
          <input
            id="space-email"
            className={fieldClassName}
            type="email"
            inputMode="email"
            autoComplete="email"
            autoCapitalize="none"
            spellCheck={false}
            value={email}
            placeholder={t("space.email.placeholder")}
            onChange={(event) => setEmail(event.target.value)}
          />
          <p className={helpTextClassName}>
            {closed ? t("space.portal.emailHelp") : t("space.email.helpShort")}
          </p>
        </div>
        <Notice tone="error">{error}</Notice>
        <p>
          <button
            type="submit"
            className={buttonClassName("primary", { block: true })}
            disabled={busy}
          >
            <Icon name="link" className="as-button__icon" />
            {busy ? t("space.email.sending") : t("space.email.send")}
          </button>
        </p>
      </form>
      {!closed && <LegalLine announcement={announcement} />}
      {onOwnServer && (
        <p>
          <button
            type="button"
            className={buttonClassName("link")}
            onClick={onOwnServer}
          >
            {t("space.ownServer")}
          </button>
        </p>
      )}
    </>
  );
}
