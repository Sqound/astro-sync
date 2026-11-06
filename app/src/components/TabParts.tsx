import { useEffect, useState, type ReactNode } from "react";

import { useI18n } from "../i18n";
import { openInBrowser, spaceStatus } from "../lib/bridge";
import type { NightChoice } from "../lib/tabs";
import {
  fieldClassName,
  fieldLabelClassName,
  helpTextClassName,
} from "../theme";
import { Icon } from "./Icon";
import { InfoLine } from "./InfoLine";

/**
 * The message at the head of « Progression » and « Journaux »: a title, two lines and an ⓘ
 * whose panel says the rest.
 */
export function TabHead({
  title,
  line,
  note,
  details,
}: {
  title: string;
  line: string;
  note: string;
  details: ReactNode;
}) {
  const { t } = useI18n();
  const [open, setOpen] = useState(false);
  return (
    <>
      <div className="as-message">
        <Icon name="sparkle" className="as-message__icon" />
        <div className="as-message__body">
          <h2 className="as-message__title">{title}</h2>
          <p>{line}</p>
          <p className={helpTextClassName}>{note}</p>
        </div>
        <button
          type="button"
          className="as-info-button"
          aria-label={t("common.learnMore")}
          aria-expanded={open}
          onClick={() => setOpen((current) => !current)}
        >
          <Icon name="info" />
        </button>
      </div>
      {open && <div className="as-notice as-notice--neutral">{details}</div>}
    </>
  );
}

/** The line of a reserved tab: one sentence, « Voir les offres » a link
 * to the web (a notice has no primary). */
export function ReservedLine({
  text,
  onOffers,
}: {
  text: string;
  onOffers: () => void;
}) {
  const { t } = useI18n();
  return (
    <div className="as-option-note">
      <Icon name="lock" className="as-line__glyph" />
      <span>{text}</span>
      <button
        type="button"
        className="as-button as-button--link"
        onClick={onOffers}
      >
        <Icon name="external" className="as-button__icon" />
        {t("wizard.offers")}
      </button>
    </div>
  );
}

/** The « Nuit » selector (outside the four levels). */
export function NightSelect({
  choices,
  value,
  disabled,
  onChange,
}: {
  choices: readonly NightChoice[];
  value: string | null;
  disabled?: boolean;
  onChange: (nightId: string) => void;
}) {
  const { t } = useI18n();
  return (
    <label>
      <span className={fieldLabelClassName}>{t("tabs.night")}</span>
      <select
        className={fieldClassName}
        value={value ?? ""}
        disabled={disabled}
        onChange={(event) => onChange(event.target.value)}
      >
        {choices.map((choice) => (
          <option key={choice.nightId} value={choice.nightId}>
            {choice.label}
          </option>
        ))}
      </select>
    </label>
  );
}

/** A line followed by its ⓘ, inside a form card. */
export function CardInfo({
  line,
  details,
}: {
  line: ReactNode;
  details: ReactNode;
}) {
  return <InfoLine line={line} details={details} />;
}

/** The space's address on the web, when this computer is signed in, and
 * whether the service is a private server (its words name no offer). */
export function useSpaceWeb(): { web: string | null; privateServer: boolean } {
  const [info, setInfo] = useState<{
    web: string | null;
    privateServer: boolean;
  }>({ web: null, privateServer: false });
  useEffect(() => {
    let cancelled = false;
    spaceStatus()
      .then((status) => {
        if (!cancelled) {
          setInfo({
            web:
              status.space?.webUrl ??
              (status.hosted ? null : status.serviceUrl),
            privateServer: !status.hosted,
          });
        }
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, []);
  return info;
}

/** « Voir les offres »: the offers' page of the space on the web, when the
 * space is known (`<web>/<langue>/offers`). */
export function offersOpener(web: string | null, locale: string): () => void {
  return () => {
    if (web) void openInBrowser(`${web.replace(/\/$/, "")}/${locale}/offers`);
  };
}
