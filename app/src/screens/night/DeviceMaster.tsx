import { useEffect, useState } from "react";

import { Icon } from "../../components/Icon";
import { useI18n } from "../../i18n";
import { masterMeasureCurrent, type MasterOutcome } from "../../lib/bridge";
import type {
  DeviceBox,
  DeviceChoiceView,
  DeviceFamilyChoice,
  DeviceSide,
  DeviceSource,
} from "../../lib/deviceMaster";
import {
  masterQualityText,
  type MasterEntryState,
} from "../../lib/masterMeasure";
import {
  badgeClassName,
  buttonClassName,
  helpTextClassName,
} from "../../theme";
import type { MeasureChoices } from "./MasterMeasure";
import { Lines, Segs } from "./parts";

/**
 * « Image empilée par l'appareil » under « Poses FITS »: what the device left in the
 * target's `device/`, no button -- the
 * choice is made at « Traitements ».
 */
export function DeviceBoxView({ box }: { box: DeviceBox }) {
  const { t } = useI18n();
  return (
    <div className="as-file-box as-device-box">
      <div className="as-file-box__head">
        <h4>{t("deviceMaster.title")}</h4>
        <span className={badgeClassName(box.state.tone)}>
          <Icon name={box.state.tone === "success" ? "check" : "warning"} />
          {box.state.text}
        </span>
      </div>
      {box.groups.map((group, index) => (
        <div key={index} className="as-device-family">
          {group.heading && (
            <p className="as-device-family__head">
              <strong>{group.heading}</strong>
              {group.status && (
                <span className={badgeClassName(group.status.tone)}>
                  {group.status.text}
                </span>
              )}
            </p>
          )}
          <Lines lines={group.lines} />
          {group.note && (
            <p className={`${helpTextClassName} as-device-family__note`}>
              {group.note}
            </p>
          )}
        </div>
      ))}
      {box.notes.length > 0 && <Lines lines={box.notes} />}
    </div>
  );
}

/** One side of a family's choice: a radio, its title, « Par défaut » or
 * « Indisponible », and its line (or why it is not offered). */
function Side({
  name,
  title,
  side,
  chosen,
  onChoose,
}: {
  name: string;
  title: string;
  side: DeviceSide;
  chosen: boolean;
  onChoose: () => void;
}) {
  const { t } = useI18n();
  const className = [
    "as-device-side",
    chosen ? "is-chosen" : "",
    side.available ? "" : "is-unavailable",
  ]
    .filter(Boolean)
    .join(" ");
  return (
    <label className={className}>
      <input
        type="radio"
        name={name}
        checked={chosen && side.available}
        disabled={!side.available}
        onChange={onChoose}
      />
      <span className="as-device-side__body">
        <span className="as-device-side__title">
          <strong>{title}</strong>
          {side.isDefault && (
            <span className="as-tag as-tag--good">
              <Icon name="check" />
              {t("deviceMaster.default")}
            </span>
          )}
          {!side.available && (
            <span className="as-tag">{t("deviceMaster.unavailable")}</span>
          )}
        </span>
        {side.why ? (
          <span className={`${helpTextClassName} as-device-side__line`}>
            {side.why}
          </span>
        ) : (
          <span className="as-device-side__line">
            <Segs segs={side.segs} />
          </span>
        )}
      </span>
    </label>
  );
}

function FamilyChoice({
  targetId,
  row,
  onChoose,
}: {
  targetId: string;
  row: DeviceFamilyChoice;
  onChoose: (label: string, source: DeviceSource) => void;
}) {
  const { t } = useI18n();
  const name = `device-${targetId}-${row.label}`;
  return (
    <fieldset className="as-device-family">
      {row.heading && (
        <legend className="as-device-family__head">
          <strong>{row.heading}</strong>
        </legend>
      )}
      <div className="as-device-choice">
        <Side
          name={name}
          title={t("deviceMaster.use")}
          side={row.use}
          chosen={row.source === "device"}
          onChoose={() => onChoose(row.label, "device")}
        />
        <Side
          name={name}
          title={t("deviceMaster.restack")}
          side={row.restack}
          chosen={row.source === "restack"}
          onChoose={() => onChoose(row.label, "restack")}
        />
      </div>
    </fieldset>
  );
}

/**
 * « Mesurer cette image »: the measure of the image of
 * the device, by the code of « Mesurer un master… »: the image is known,
 * so there is no file to choose. It keeps its right (`analysis.logs`);
 * the **use** of the image is not reserved, only the figures that leave
 * with the render. A case, unticked, **records** the measure and
 * « Continuer » makes it; the measure kept
 * is said, with the case « Retirer la mesure »; the case is greyed with
 * the right's name without it.
 */
export function DeviceMeasure({
  sessionId,
  targetId,
  target,
  folderName,
  image,
  state,
  measure,
}: {
  sessionId: string;
  targetId: string;
  target: string;
  folderName: string;
  image: { fileName: string; path: string };
  state: MasterEntryState;
  measure: MeasureChoices;
}) {
  const { t, locale } = useI18n();
  const [kept, setKept] = useState<Extract<
    MasterOutcome,
    { result: "measured" }
  > | null>(null);
  // What the target keeps, read again for each image.
  useEffect(() => {
    let cancelled = false;
    setKept(null);
    masterMeasureCurrent(sessionId, folderName)
      .then((outcome) => {
        if (!cancelled && outcome?.result === "measured") setKept(outcome);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [sessionId, folderName, image.path]);
  const measuredHere = kept !== null && kept.master === image.fileName;
  const pick = measure.recap.measures[targetId];
  const measuring =
    pick !== undefined && pick.source === "device" && pick.ticked;
  const forgetPick = { targetId, folderName, designation: target };
  const forgetting = measure.recap.forget.some(
    (item) => item.targetId === targetId,
  );
  const closed = state !== "available";
  return (
    <div className="as-device-measure">
      <p className="as-device-measure__head">
        <strong>{t("deviceMaster.measureTitle", { target })}</strong>
        {measuredHere && (
          <span className={badgeClassName("success")}>
            <Icon name="check" />
            {t("deviceMaster.measured")}
          </span>
        )}
        {state === "reserved" && (
          <span className={badgeClassName("neutral")}>
            <Icon name="lock" />
            {t("wizard.treatments.reservedBadge")}
          </span>
        )}
        {state === "toolMissing" && (
          <span className={badgeClassName("warning")}>
            <Icon name="warning" />
            {t("wizard.treatments.toolMissing")}
          </span>
        )}
      </p>
      {measuredHere && kept && (
        <>
          <p>
            <strong>
              {masterQualityText(t, locale, kept.quality, "device")}
            </strong>
          </p>
          <p className={helpTextClassName}>
            {t("deviceMaster.measuredOn", {
              file: image.fileName,
              target,
            })}
          </p>
          <label className="as-device-measure__tick">
            <input
              type="checkbox"
              checked={forgetting}
              disabled={measure.working}
              onChange={() =>
                measure.onRecap({
                  type: "tickForget",
                  pick: forgetPick,
                  on: !forgetting,
                })
              }
            />
            {t("master.forget")}
          </label>
        </>
      )}
      {!measuredHere && (
        <label className="as-device-measure__tick">
          <input
            type="checkbox"
            checked={measuring}
            disabled={closed || measure.working}
            onChange={() =>
              measuring
                ? measure.onRecap({ type: "tickMeasure", targetId, on: false })
                : measure.onRecap({
                    type: "pickMeasure",
                    pick: {
                      targetId,
                      folderName,
                      designation: target,
                      source: "device",
                      file: image.path,
                      fileName: image.fileName,
                    },
                  })
            }
          />
          {t("deviceMaster.measure")}
        </label>
      )}
      {measuring && (
        <p className={helpTextClassName}>{t("deviceMaster.pending")}</p>
      )}
    </div>
  );
}

/**
 * The device's part of a target's card at « Traitements »: for each family, the
 * image of the device -- by default
 * when it covers the family -- or a new stacking of the poses; « Mesurer cette
 * image »; « Pourquoi empiler à nouveau ? ».
 */
export function DeviceChoice({
  targetId,
  designation,
  folderName,
  sessionId,
  view,
  measureState,
  measure,
  onChoose,
}: {
  targetId: string;
  designation: string;
  folderName: string | null;
  sessionId: string;
  view: DeviceChoiceView;
  measureState: MasterEntryState;
  measure: MeasureChoices;
  onChoose: (label: string, source: DeviceSource) => void;
}) {
  const { t } = useI18n();
  const [why, setWhy] = useState(false);
  const whyId = `device-why-${targetId}`;
  return (
    <div className="as-device-master">
      {view.families.map((row) => (
        <FamilyChoice
          key={row.label}
          targetId={targetId}
          row={row}
          onChoose={onChoose}
        />
      ))}
      {view.pipelinesNote && (
        <p className={`${helpTextClassName} as-device-note`}>
          {view.pipelinesNote}
        </p>
      )}
      {view.measure && folderName && (
        <DeviceMeasure
          sessionId={sessionId}
          targetId={targetId}
          target={designation}
          folderName={folderName}
          image={view.measure}
          state={measureState}
          measure={measure}
        />
      )}
      <div className="as-device-actions">
        <button
          type="button"
          className={buttonClassName("link")}
          aria-expanded={why}
          aria-controls={whyId}
          onClick={() => setWhy(!why)}
        >
          {t("deviceMaster.whyLink")}
        </button>
      </div>
      {why && (
        <div id={whyId} className="as-device-why">
          <ol>
            {view.why.cases.map((item) => (
              <li key={item.title}>
                <strong>{item.title}</strong> {item.text}
              </li>
            ))}
          </ol>
          <table>
            <thead>
              <tr>
                <th scope="col" />
                <th scope="col">{view.why.table.head[0]}</th>
                <th scope="col">{view.why.table.head[1]}</th>
              </tr>
            </thead>
            <tbody>
              {view.why.table.rows.map(([label, device, here]) => (
                <tr key={label}>
                  <th scope="row">{label}</th>
                  <td colSpan={here === null ? 2 : 1}>{device}</td>
                  {here !== null && <td>{here}</td>}
                </tr>
              ))}
            </tbody>
          </table>
          <p className={helpTextClassName}>{view.why.caption}</p>
          <p className={helpTextClassName}>{view.why.footnote}</p>
        </div>
      )}
      {view.unused && (
        <p className={`${helpTextClassName} as-device-note`}>{view.unused}</p>
      )}
    </div>
  );
}
