import { useI18n } from "../../../i18n";
import type { EquipmentProposal } from "../../../lib/bridge";
import { pluralText } from "../../../lib/written";
import { badgeClassName, helpTextClassName } from "../../../theme";
import { Tick, useJoin, type ReviewContext } from "./parts";

/** « Matériel »: the equipment named in the headers, matched to « Mon
 * matériel ». */
export function EquipmentCard({ proposal }: ReviewContext) {
  const { t } = useI18n();
  if (proposal.equipment.length === 0) return null;
  return (
    <section className="as-review__card">
      <h2 className="as-review__title">{t("detect.review.equipment")}</h2>
      {proposal.equipment.map((item) => (
        <EquipmentRow key={item.id} item={item} />
      ))}
    </section>
  );
}

function EquipmentRow({ item }: { item: EquipmentProposal }) {
  const { t, locale } = useI18n();
  const join = useJoin();
  // Recognised by its name in « Mon matériel »: the camera, the mount, the
  // telescope. A telescope that only has the focal length in common with
  // one of « Mon matériel » (`by: "focal-length"`) is not recognised: it is
  // said close, to confirm -- never shown under the name of another telescope.
  const near = item.telescope?.by === "focal-length" ? item.telescope : null;
  const known = [item.telescope, item.camera, item.mount]
    .filter(
      (match): match is NonNullable<typeof match> =>
        match !== null && match.by !== "focal-length",
    )
    .map((match) => match.name);
  const headers = [
    item.telescop ? t("detect.equipment.telescop") : "",
    item.instrume ? t("detect.equipment.instrume") : "",
    item.focalLengthMm
      ? t("detect.equipment.focal", { mm: Math.round(item.focalLengthMm) })
      : "",
  ].filter(Boolean);
  // The title is what the file says (TELESCOP · INSTRUME), never the name of
  // an equipment of the space; only a file that names nothing falls back to
  // the equipment matched.
  const fromFile = [item.telescop, item.instrume].filter(Boolean);
  const title =
    fromFile.length > 0
      ? fromFile.join(t("detect.join"))
      : [...known, near?.name].filter(Boolean).join(t("detect.join"));
  return (
    <div className="as-review__row">
      <div className="as-review__row-text">
        <strong>{title}</strong>
        <div className="as-review__sub">
          {join([
            pluralText(t, locale, "detect.nights", item.sessions.length),
            headers.join(t("detect.comma")),
          ])}
        </div>
      </div>
      {near ? (
        <span className={`${badgeClassName("warning")} as-review__pill`}>
          {t("detect.equipment.nearFocal", { name: near.name })}
        </span>
      ) : known.length > 0 ? (
        <span className={`${badgeClassName("success")} as-review__pill`}>
          <Tick />
          {t("detect.equipment.known")}
        </span>
      ) : (
        <span className={`${badgeClassName("warning")} as-review__pill`}>
          {t("detect.equipment.unknown")}
        </span>
      )}
      {near && (
        <p className={`${helpTextClassName} as-review__near`}>
          {t(
            item.camera?.by === "name"
              ? "detect.equipment.nearFocalWhy"
              : "detect.equipment.nearFocalOnlyWhy",
            { name: near.name, mm: Math.round(item.focalLengthMm ?? 0) },
          )}
        </p>
      )}
    </div>
  );
}
