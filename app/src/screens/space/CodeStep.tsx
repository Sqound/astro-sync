import { useEffect, useState, type FormEvent } from "react";

import { Notice } from "../../components/Notice";
import { useSnackbar } from "../../components/Snackbar";
import { useServiceWaiting } from "../../hooks/useServiceWaiting";
import { useI18n } from "../../i18n";
import {
  spaceRequestCode,
  spaceVerifyCode,
  type CodeRequested,
  type SignInOutcome,
} from "../../lib/bridge";
import {
  CODE_VALID_MINUTES,
  codeProblemOf,
  formatCountdown,
  messageLanguage,
  retryAfterSeconds,
  secondsLeft,
  signInErrorText,
  untilText,
  type CodeProblem,
} from "../../lib/space";
import {
  buttonClassName,
  fieldClassName,
  fieldGroupClassName,
  fieldLabelClassName,
  helpTextClassName,
} from "../../theme";

/**
 * « Saisissez votre code »: the six digits received by e-mail, with its states
 * -- a wrong code and the tries left, an expired code, too many
 * tries and when the wait ends, no connection, the service starting -- and the
 * resend, which unlocks after the delay the service announced.
 *
 * The link of the message does not open Astro Sync: on a computer it shows the
 * service's page with the code. The screen says so.
 */
export function CodeStep({
  email,
  portalUrl,
  closed = false,
  requested,
  onSignedIn,
  onChangeAddress,
}: {
  email: string;
  /** A private server's address; `null` for the hosted service. */
  portalUrl: string | null;
  /** The service is a private server signing in by e-mail code: it says «
   * votre portail », not « votre espace ». */
  closed?: boolean;
  requested: CodeRequested;
  onSignedIn: (outcome: SignInOutcome) => void;
  onChangeAddress: () => void;
}) {
  const { t, locale } = useI18n();
  const serviceWaiting = useServiceWaiting();

  const [code, setCode] = useState("");
  const [busy, setBusy] = useState<"verify" | "resend" | null>(null);
  const [problem, setProblem] = useState<CodeProblem | null>(null);
  // What passes is said by a snackbar.
  const { notify } = useSnackbar();
  const saySuccess = (text: string) => notify({ tone: "success", text });
  const [resendError, setResendError] = useState<string | null>(null);
  // When « Renvoyer le code » unlocks: what the service announced, counted
  // from now (a count, so the two clocks need not agree).
  const [unlocksAt, setUnlocksAt] = useState(
    () => Date.now() + requested.resendAfterSeconds * 1000,
  );
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, []);

  const untilResend = secondsLeft(unlocksAt, now);

  async function handleVerify(event?: FormEvent) {
    event?.preventDefault();
    if (busy) {
      return;
    }
    setProblem(null);
    setResendError(null);
    setBusy("verify");
    try {
      onSignedIn(await spaceVerifyCode(portalUrl, email, code));
    } catch (caught) {
      const found = codeProblemOf(caught, Date.now());
      setProblem(found);
      // Too many wrong codes: every code already sent is void and the
      // address waits (15 minutes, 1 hour or 24 hours, as the service
      // says). A new one can be asked for when the wait ends.
      if (found.kind === "locked" && found.seconds !== null) {
        setUnlocksAt(Date.now() + found.seconds * 1000);
      }
    } finally {
      setBusy(null);
    }
  }

  async function handleResend() {
    if (busy) {
      return;
    }
    setProblem(null);
    setResendError(null);
    setBusy("resend");
    try {
      const again = await spaceRequestCode(
        portalUrl,
        email,
        messageLanguage(locale),
      );
      setCode("");
      setUnlocksAt(Date.now() + again.resendAfterSeconds * 1000);
      saySuccess(t("space.code.resent", { email: again.email }));
    } catch (caught) {
      const wait = retryAfterSeconds(caught, Date.now());
      if (wait !== null) {
        setUnlocksAt(Date.now() + wait * 1000);
      }
      setResendError(signInErrorText(t, caught, Date.now(), locale));
    } finally {
      setBusy(null);
    }
  }

  /** The new-code button of the states where the code is of no more use:
   * it counts down when the service asked for a wait. */
  // A wait of an hour or a day is not counted down to the second: the
  // sentence above says when it ends.
  const newCodeLabel =
    untilResend > 0 && untilResend <= 3600
      ? t("space.code.newCodeIn", { time: formatCountdown(untilResend) })
      : t("space.code.newCode");
  const verifying = busy === "verify";
  const noMoreUse =
    problem?.kind === "expired" ||
    problem?.kind === "locked" ||
    problem?.kind === "deviceLimit" ||
    problem?.kind === "unconfirmed";
  const unavailable = problem?.kind === "unavailable";

  function problemBlock() {
    if (!problem) {
      return null;
    }
    switch (problem.kind) {
      case "wrong":
        return (
          <Notice tone="error">
            {t("space.code.wrong.title")}{" "}
            {problem.attemptsRemaining === null
              ? t("space.code.wrong.unknown")
              : problem.attemptsRemaining === 1
                ? t("space.code.wrong.one")
                : t("space.code.wrong.many", {
                    count: problem.attemptsRemaining,
                  })}
          </Notice>
        );
      case "expired":
        return (
          <Notice tone="error">
            {t("space.code.expired.title")} {t("space.code.expired.body")}
          </Notice>
        );
      case "locked":
        return (
          <Notice tone="error">
            {t("space.code.locked.title")}{" "}
            {t("space.code.locked.body", {
              when: untilText(t, locale, problem.until, now),
            })}
          </Notice>
        );
      case "deviceLimit":
        // The address is proven and the code used: detach a device from
        // « Mes appareils », then ask for a new code.
        return (
          <Notice tone="error">
            {closed
              ? t("space.portal.deviceLimit")
              : t("errors.device_limit_reached")}
          </Notice>
        );
      case "offline":
        return (
          <Notice tone="warning">
            {t("space.code.offline.title")} {t("space.code.offline.body")}
          </Notice>
        );
      case "startTimeout":
        return <Notice tone="warning">{t("space.code.startTimeout")}</Notice>;
      case "unconfirmed":
        return <Notice tone="warning">{t("space.code.unconfirmed")}</Notice>;
      case "malformed":
        return <Notice tone="error">{t("space.code.malformed")}</Notice>;
      case "rateLimited":
        return (
          <Notice tone="error">
            {t("space.code.rateLimited", {
              when: untilText(t, locale, problem.until, now),
            })}
          </Notice>
        );
      case "cancelled":
        return <Notice tone="neutral">{t("space.code.cancelled")}</Notice>;
      case "unavailable":
        return (
          <Notice tone="error">
            {closed ? (
              t("space.portal.code.unavailable")
            ) : (
              <>
                {t("space.code.unavailable.title")}{" "}
                {t("space.code.unavailable.body")}
              </>
            )}
          </Notice>
        );
      default:
        return (
          <Notice tone="error">
            {signInErrorText(t, problem.error, now, locale)}
          </Notice>
        );
    }
  }

  return (
    <>
      <h3>{t("space.code.heading")}</h3>
      <p>{t("space.code.sentTo", { email })}</p>

      {problemBlock()}

      {!unavailable && (
        <form onSubmit={(event) => void handleVerify(event)}>
          <div className={fieldGroupClassName}>
            <label className={fieldLabelClassName} htmlFor="space-code">
              {t("space.code.label")}
            </label>
            <input
              id="space-code"
              className={fieldClassName}
              type="text"
              inputMode="numeric"
              autoComplete="one-time-code"
              spellCheck={false}
              maxLength={14}
              autoFocus
              value={code}
              disabled={verifying}
              onChange={(event) => setCode(event.target.value)}
            />
            <p className={helpTextClassName}>
              {t("space.code.validFor", { minutes: CODE_VALID_MINUTES })}
            </p>
          </div>
          <p>
            {noMoreUse ? (
              <button
                type="button"
                className={buttonClassName("primary")}
                disabled={busy !== null || untilResend > 0}
                onClick={() => void handleResend()}
              >
                {newCodeLabel}
              </button>
            ) : (
              <button
                type="submit"
                className={buttonClassName("primary")}
                disabled={busy !== null || code.trim() === ""}
              >
                {verifying
                  ? t("space.code.verifying")
                  : t("space.code.validate")}
              </button>
            )}
          </p>
          {verifying && serviceWaiting.waiting && (
            <Notice tone="warning">{t("space.code.willVerify")}</Notice>
          )}
        </form>
      )}

      {!unavailable && (
        <>
          <p className={helpTextClassName}>{t("space.code.linkNote")}</p>
          <p className={helpTextClassName}>{t("space.code.nothingReceived")}</p>
        </>
      )}
      <Notice tone="error">{resendError}</Notice>
      <p>
        {!unavailable && !noMoreUse && (
          <>
            <button
              type="button"
              className={buttonClassName("link")}
              disabled={busy !== null || untilResend > 0}
              onClick={() => void handleResend()}
            >
              {untilResend > 0
                ? t("space.code.resendIn", {
                    time: formatCountdown(untilResend),
                  })
                : t("space.code.resend")}
            </button>{" "}
          </>
        )}
        <button
          type="button"
          className={buttonClassName("link")}
          disabled={busy !== null}
          onClick={onChangeAddress}
        >
          {t("space.code.changeAddress")}
        </button>
      </p>
    </>
  );
}
