// The words of the foot of the import: `lib/importFlow.ts` says which
// action each step offers, this file what it is called.

import type { useI18n } from "../../i18n";
import type { FootPrimary, FootSecondary } from "../../lib/importFlow";

type Translate = ReturnType<typeof useI18n>["t"];

/** The words of a secondary action of the foot. */
export function secondaryLabel(t: Translate, kind: FootSecondary): string {
  switch (kind) {
    case "stopScan":
      return t("detect.stop");
    case "skipTreatments":
      return t("import.treat.skip");
    case "keepImages":
      return t("import.treat.keep");
    case "stopTreatments":
      return t("detect.treat.stop");
    case "finishWithoutSending":
      return t("detect.send.skip");
    case "createWithoutSending":
      return t("import.create.withoutSending");
  }
}

/** The words of a primary that do not count anything (the others are
 * written by their step). */
export function plainPrimaryLabel(
  t: Translate,
  kind: Exclude<FootPrimary, "create" | "treat" | "send">,
): string {
  switch (kind) {
    case "startScan":
      return t("detect.start");
    case "continue":
      return t("import.continue");
    case "createSend":
      return t("import.create.send");
    case "createOnly":
      return t("import.create.only");
    case "finish":
      return t("organise.done");
    case "backToSessions":
      return t("import.back");
  }
}
