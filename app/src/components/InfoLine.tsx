import { useId, useState, type ReactNode } from "react";

import { useI18n } from "../i18n";
import {
  helpTextClassName,
  infoButtonClassName,
  infoLineClassName,
  noticeClassName,
  stackClassName,
} from "../theme";
import { Icon } from "./Icon";

/**
 * « Une ligne, puis ⓘ »: the one short line a person needs, and a round
 * button that opens what the line leaves out (U5). The details are moved
 * behind it, never deleted; a safety instruction or a warning never goes in
 * here -- it stays on the surface.
 */
export function InfoLine({
  line,
  details,
}: {
  line: ReactNode;
  details: ReactNode;
}) {
  const { t } = useI18n();
  const [open, setOpen] = useState(false);
  const panelId = useId();
  return (
    <>
      <div className={infoLineClassName}>
        <p className={helpTextClassName}>{line}</p>
        <button
          type="button"
          className={infoButtonClassName}
          aria-label={t("common.learnMore")}
          aria-expanded={open}
          aria-controls={panelId}
          onClick={() => setOpen((current) => !current)}
        >
          <Icon name="info" />
        </button>
      </div>
      {open && (
        <div
          id={panelId}
          className={`${noticeClassName("neutral")} ${stackClassName}`}
        >
          {details}
        </div>
      )}
    </>
  );
}
