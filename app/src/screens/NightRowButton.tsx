import { Icon } from "../components/Icon";
import { BrandLoader } from "../components/BrandLoader";
import { useI18n } from "../i18n";
import type { NightRow } from "../lib/nights";
import { badgeClassName } from "../theme";

/**
 * One row of « Nuits »: a button of six grid tracks (night, targets, on this
 * computer, online, state, chevron). Every cell is a flow box of its own
 * track -- none is positioned out of it -- so a long value or chip wraps
 * inside its column and the row grows in height.
 */
export function NightRowButton({
  row,
  serviceDown,
  onOpen,
}: {
  row: NightRow;
  serviceDown: boolean;
  onOpen: () => void;
}) {
  const { t } = useI18n();
  return (
    <button
      type="button"
      role="row"
      aria-busy={row.reading}
      // A session still to debrief (or abandoned) cannot be
      // selected -- no click, no focus, no treatment -- until the
      // service gives it a debriefing; its message says what to do.
      disabled={Boolean(row.held)}
      aria-disabled={row.held ? true : undefined}
      className={row.held ? "as-nights__row is-held" : "as-nights__row"}
      onClick={onOpen}
    >
      <span className="as-nights__cell" role="cell">
        <strong>{row.day}</strong>
        <span className="as-nights__sub">{row.hours}</span>
      </span>
      <span className="as-nights__cell" role="cell">
        <span>{row.placeAndTargets}</span>
        {row.planTag && (
          <span
            className={`${badgeClassName("neutral")} as-nights__plan-tag`}
            title={row.planTag}
          >
            {row.planTag}
          </span>
        )}
        {row.open && (
          <span className={badgeClassName("neutral")}>{t("status.open")}</span>
        )}
      </span>
      <span className="as-nights__cell" role="cell">
        {row.reading ? (
          <span className="as-nights__reading">
            <BrandLoader size={16} className="as-waiting__star" />
            {t("wizard.wait.night")}
          </span>
        ) : (
          (row.local ?? "…")
        )}
      </span>
      <span
        className={
          serviceDown ? "as-nights__cell is-greyed" : "as-nights__cell"
        }
        role="cell"
      >
        {row.reading ? "" : (row.online ?? "…")}
      </span>
      <span className="as-nights__cell" role="cell">
        {row.state && <StatePill state={row.state} />}
        {row.missing && (
          <span className="as-nights__missing">{row.missing}</span>
        )}
        {row.held && row.heldMessage && (
          <span className="as-nights__missing as-nights__held">
            {row.heldMessage}
          </span>
        )}
      </span>
      {row.held ? (
        <span aria-hidden="true" />
      ) : (
        <Icon name="chevron" className="as-nights__chevron" />
      )}
    </button>
  );
}

function StatePill({ state }: { state: NonNullable<NightRow["state"]> }) {
  const { t } = useI18n();
  if (state === "complete") {
    return (
      <span className={badgeClassName("success")}>
        <Icon name="check" strokeWidth={2.4} />
        {t("nights.state.complete")}
      </span>
    );
  }
  if (state === "to_complete") {
    return (
      <span className={badgeClassName("neutral")}>
        <Icon name="clock" />
        {t("nights.state.toComplete")}
      </span>
    );
  }
  return (
    <span className="as-badge as-badge--dotted">
      <Icon name="pending" />
      {t("nights.state.notStarted")}
    </span>
  );
}
