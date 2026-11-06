import { useCallback, useEffect, useRef, useState } from "react";

import { Notice } from "../components/Notice";
import { useSnackbar } from "../components/Snackbar";
import { Waiting } from "../components/Waiting";
import { useI18n } from "../i18n";
import {
  serviceAnnouncement,
  spaceRefresh,
  spaceStatus,
  type CapabilitiesState,
  type CodeRequested,
  type SignInAnnouncement,
  type SignInOutcome,
  type SpaceStatus,
} from "../lib/bridge";
import { analysisSendingNotice } from "../lib/capabilities";
import { formatError } from "../lib/format";
import {
  emailToPropose,
  flowOf,
  isClosedPortal,
  ownServerDraft,
  signInErrorText,
} from "../lib/space";
import {
  buttonClassName,
  cardClassName,
  fieldClassName,
  fieldGroupClassName,
  fieldLabelClassName,
  helpTextClassName,
  inactiveButtonClassName,
  panelClassName,
} from "../theme";
import { CodeStep } from "./space/CodeStep";
import { ConnectedSpace } from "./space/ConnectedSpace";
import { EmailStep } from "./space/EmailStep";
import { PairingForm, PrivateConnected } from "./space/PrivateServer";

/** What the service in front of us said, or why it could not be asked. */
type Probe =
  | { state: "idle" }
  | { state: "checking" }
  | { state: "ready"; announcement: SignInAnnouncement }
  | { state: "failed"; message: string };

/**
 * « Espace Astro Portal »: how this computer is connected to the service.
 *
 * On the hosted service Astro Sync signs in with an
 * e-mail address, then the code of six digits received by e-mail, no password,
 * and announces itself as the application `sync`: a helper that sends
 * captures and logs, never refused for having another space. Signed out,
 * it is the card of « Accueil » (`variant="card"`). Once connected, « Accueil »
 * draws the connection itself and this screen, opened from its card, shows the
 * space and the address of the space on the web; the name of this computer
 * and « Détacher cet ordinateur » are in « Réglages ».
 *
 * What the service **announces** (`GET /v1/service`, `signIn`) decides the
 * screen: an address for the hosted service, the code its account page shows
 * for a private server that pairs (as before), the same address-then-code
 * with the words of « votre portail » for a private server signing in by
 * e-mail code (`spaceCreation: "closed"`: only its owner's address opens it),
 * and an absent announcement reads as a private server that pairs.
 *
 * A service that ends the sign-in (this computer detached, the space deleted)
 * brings the person back here with a sentence that says so; the folders
 * associated with sessions and the pending uploads are kept.
 */
export function SpaceScreen({
  capabilitiesState,
  onConnectionChanged,
  variant = "screen",
  onSignedIn,
}: {
  /** Owned by `App.tsx`, shared with the sidebar's own tab gating. */
  capabilitiesState: CapabilitiesState | null;
  /** The connection changed (signed in, detached, paired): the capabilities
   * of the account are to be read again. */
  onConnectionChanged: () => void;
  /** `card`: the sign-in as the card of « Accueil » (the home draws the
   * connected state itself); `screen`: « Espace Astro Portal » on its own. */
  variant?: "screen" | "card";
  /** A sign-in succeeded (`created`: it made the space). */
  onSignedIn?: (created: boolean) => void;
}) {
  const { t, locale } = useI18n();

  const [status, setStatus] = useState<SpaceStatus | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  // The server being typed outlives the screen: going to another page and
  // coming back finds the address again (`ownServerDraft`).
  const [ownServer, setOwnServer] = useState(
    () => ownServerDraft.recall() !== null,
  );
  const [serverUrl, setServerUrl] = useState(
    () => ownServerDraft.recall()?.serverUrl ?? "",
  );
  const [probe, setProbe] = useState<Probe>({ state: "idle" });
  const [codeSent, setCodeSent] = useState<{
    email: string;
    requested: CodeRequested;
  } | null>(null);
  // The address last typed, kept for « Changer d'adresse ».
  const [lastEmail, setLastEmail] = useState("");
  const [created, setCreated] = useState(false);
  // What passes is said by a snackbar.
  const { notify } = useSnackbar();
  const saySuccess = (text: string) => notify({ tone: "success", text });
  // A probe that is no longer the latest is ignored when it answers.
  const probeSerial = useRef(0);
  // « Vérifier le serveur » keeps the keyboard focus while the card redraws.
  const verifyButton = useRef<HTMLButtonElement | null>(null);
  const verifyHadFocus = useRef(false);

  const runProbe = useCallback(
    async (url: string | null) => {
      const serial = ++probeSerial.current;
      setProbe({ state: "checking" });
      try {
        const announcement = await serviceAnnouncement(url);
        if (serial === probeSerial.current) {
          setProbe({ state: "ready", announcement });
        }
      } catch (caught) {
        if (serial === probeSerial.current) {
          setProbe({
            state: "failed",
            message: signInErrorText(t, caught, Date.now(), locale),
          });
        }
      }
    },
    [t, locale],
  );

  const load = useCallback(async () => {
    setLoadError(null);
    try {
      const loaded = await spaceStatus();
      setStatus(loaded);
      const draft = ownServerDraft.recall();
      if (!loaded.hosted) {
        // A private server was saved: start from it, or from the address
        // being typed when the person left this screen.
        setOwnServer(true);
        setServerUrl(draft?.serverUrl ?? loaded.serviceUrl);
      }
      if (loaded.hosted && !loaded.paired && draft) {
        // The address typed before leaving: asked again only if it had been
        // checked, never contacted unasked.
        if (draft.verified && draft.serverUrl.trim() !== "") {
          void runProbe(draft.serverUrl.trim());
        }
      } else if (loaded.paired && loaded.space) {
        // What is on disk is shown at once; the service is asked quietly.
        void spaceRefresh()
          .then(setStatus)
          .catch(() => {
            // The screen keeps what it has.
          });
      } else if (!loaded.paired) {
        void runProbe(loaded.hosted ? null : loaded.serviceUrl);
      }
    } catch (caught) {
      setLoadError(formatError(t, caught));
    }
    // `t` changes with the language: reloading then would throw away a code
    // the person is typing.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  // What is typed for a private server is kept while the card is on screen
  // and the computer is not connected.
  useEffect(() => {
    if (status && !status.paired && ownServer) {
      ownServerDraft.remember({
        serverUrl,
        verified: probe.state === "ready",
      });
    } else if (status) {
      ownServerDraft.forget();
    }
  }, [status, ownServer, serverUrl, probe.state]);

  // The card redraws while the server is checked: the focus stays on the
  // button that was used, or comes back to it.
  useEffect(() => {
    if (
      probe.state !== "checking" &&
      verifyHadFocus.current &&
      (document.activeElement === null ||
        document.activeElement === document.body)
    ) {
      verifyButton.current?.focus();
    }
    if (probe.state !== "checking") {
      verifyHadFocus.current = false;
    }
  }, [probe.state]);

  function handleOwnServer() {
    probeSerial.current += 1;
    setOwnServer(true);
    setProbe({ state: "idle" });
    setCodeSent(null);
  }

  function handleBackToHosted() {
    ownServerDraft.forget();
    setOwnServer(false);
    setCodeSent(null);
    void runProbe(null);
  }

  function handleSignedIn(outcome: SignInOutcome) {
    setStatus(outcome.status);
    setCreated(outcome.created);
    setCodeSent(null);
    setOwnServer(!outcome.status.hosted);
    onSignedIn?.(outcome.created);
    onConnectionChanged();
  }

  async function handlePaired() {
    saySuccess(t("settings.pairingSuccessNotice"));
    try {
      setStatus(await spaceStatus());
    } catch (caught) {
      setLoadError(formatError(t, caught));
    }
    onConnectionChanged();
  }

  // The analysis is local and free; only the hosted service has a display
  // of it, and says so here. A private server has none to name.
  const analysisNotice = analysisSendingNotice(
    capabilitiesState,
    status?.hosted ?? true,
  );

  // A private server signing in by e-mail code says « votre portail ».
  const closedPortal =
    probe.state === "ready" && isClosedPortal(probe.announcement);

  function signedOutBody(current: SpaceStatus) {
    const portalUrl = ownServer ? serverUrl.trim() : null;
    if (codeSent) {
      return (
        <CodeStep
          email={codeSent.email}
          portalUrl={portalUrl}
          closed={closedPortal}
          requested={codeSent.requested}
          onSignedIn={handleSignedIn}
          onChangeAddress={() => setCodeSent(null)}
        />
      );
    }
    return (
      <>
        {ownServer && (
          <>
            {!closedPortal && <h3>{t("space.private.title")}</h3>}
            {/* « Aucune adresse e-mail n'est demandée » only once the server
                announced that it pairs: before its address is checked, it
                may be a portal that asks one. */}
            {probe.state === "ready" &&
              probe.announcement.method === "pairing" && (
                <p className={helpTextClassName}>
                  {t("space.private.description")}
                </p>
              )}
            <div className={fieldGroupClassName}>
              <label className={fieldLabelClassName} htmlFor="space-server-url">
                {t("settings.portalUrlLabel")}
              </label>
              <input
                id="space-server-url"
                className={fieldClassName}
                type="text"
                value={serverUrl}
                placeholder={t("settings.portalUrlPlaceholder")}
                onChange={(event) => {
                  setServerUrl(event.target.value);
                  probeSerial.current += 1;
                  setProbe({ state: "idle" });
                }}
              />
            </div>
            <p>
              {/* Never `disabled` while it checks: a disabled button loses
                  the keyboard focus and Tab starts again from the menu. */}
              <button
                ref={verifyButton}
                type="button"
                className={
                  probe.state === "checking"
                    ? inactiveButtonClassName
                    : buttonClassName("tertiary")
                }
                disabled={serverUrl.trim() === ""}
                aria-disabled={probe.state === "checking" ? true : undefined}
                onClick={() => {
                  if (probe.state === "checking") return;
                  verifyHadFocus.current = true;
                  void runProbe(serverUrl.trim());
                }}
              >
                {t("space.private.check")}
              </button>{" "}
              <button
                type="button"
                className={buttonClassName("link")}
                onClick={handleBackToHosted}
              >
                {t("space.backToHosted")}
              </button>
            </p>
          </>
        )}
        {probe.state === "checking" && (
          <Notice tone="neutral">{t("space.checkingService")}</Notice>
        )}
        {probe.state === "failed" && (
          <>
            <Notice tone="error">{probe.message}</Notice>
            <p>
              <button
                type="button"
                className={buttonClassName("tertiary")}
                onClick={() =>
                  void runProbe(ownServer ? serverUrl.trim() : null)
                }
              >
                {t("space.retry")}
              </button>{" "}
              {!ownServer && (
                <button
                  type="button"
                  className={buttonClassName("link")}
                  onClick={handleOwnServer}
                >
                  {t("space.ownServer")}
                </button>
              )}
            </p>
          </>
        )}
        {probe.state === "ready" &&
          announcementBody(probe.announcement, current)}
      </>
    );
  }

  function announcementBody(
    announcement: SignInAnnouncement,
    current: SpaceStatus,
  ) {
    switch (flowOf(announcement)) {
      case "email":
        return announcement.method === "email-code" ? (
          <>
            {ownServer && isClosedPortal(announcement) && (
              <Notice tone="success">{t("space.private.operational")}</Notice>
            )}
            <EmailStep
              announcement={announcement}
              portalUrl={ownServer ? serverUrl.trim() : null}
              initialEmail={emailToPropose(lastEmail, current.ended)}
              ended={current.ended}
              onCodeSent={(email, requested) => {
                setLastEmail(email);
                setCodeSent({ email, requested });
              }}
              onOwnServer={ownServer ? undefined : handleOwnServer}
            />
          </>
        ) : null;
      case "pairing":
        return (
          <>
            {ownServer && (
              <Notice tone="success">{t("space.private.operational")}</Notice>
            )}
            <PairingForm
              portalUrl={ownServer ? serverUrl.trim() : current.serviceUrl}
              onPaired={() => void handlePaired()}
            />
            {!ownServer && (
              <p>
                <button
                  type="button"
                  className={buttonClassName("link")}
                  onClick={handleOwnServer}
                >
                  {t("space.ownServer")}
                </button>
              </p>
            )}
          </>
        );
      case "none":
        return <Notice tone="warning">{t("space.noSignIn")}</Notice>;
      default:
        return <Notice tone="warning">{t("space.unsupported")}</Notice>;
    }
  }

  // The title of the screen: a portal of one person is « Votre portail », the
  // hosted service's space « Espace Astro Portal ».
  const portalScreen =
    closedPortal || (!!status?.paired && !!status.space && !status.hosted);

  if (variant === "card") {
    // « Accueil » draws the connected state; this is only the way in.
    return (
      <section className={cardClassName()} aria-label={t("space.title")}>
        <Notice tone="error">{loadError}</Notice>
        {!status && !loadError && <Waiting label={t("common.wait.space")} />}
        {!status && loadError && (
          <div>
            <button
              type="button"
              className={buttonClassName("tertiary")}
              onClick={() => void load()}
            >
              {t("space.retry")}
            </button>
          </div>
        )}
        {status && !status.paired && signedOutBody(status)}
      </section>
    );
  }

  return (
    <div className={panelClassName}>
      <h2>{portalScreen ? t("space.portal.title") : t("space.title")}</h2>
      <Notice tone="error">{loadError}</Notice>
      {!status && !loadError && <Waiting label={t("common.wait.space")} />}
      {!status && loadError && (
        <p>
          <button
            type="button"
            className={buttonClassName("tertiary")}
            onClick={() => void load()}
          >
            {t("space.retry")}
          </button>
        </p>
      )}
      {status && status.paired && status.space && (
        <ConnectedSpace
          status={status}
          created={created}
          analysisNotice={analysisNotice}
          onAcknowledge={() => setCreated(false)}
        />
      )}
      {status && status.paired && !status.space && (
        <PrivateConnected status={status} />
      )}
      {status && !status.paired && signedOutBody(status)}
    </div>
  );
}
