/** `stacking::EngineNotice`: the engine of the « Rendu soigné » was not the
 * one asked for. */
export interface EngineNotice {
  family?: string | null;
  /** What did the work in the end: `siril` or `polynomial`. */
  used: string;
  causes: EngineCause[];
}

export type EngineCause =
  | "graxpert_missing"
  | "graxpert_no_library"
  | "graxpert_failed"
  | "siril_missing"
  | "siril_failed";

type Translate = (
  key: string,
  values?: Record<string, string | number>,
) => string;

const CAUSE_KEYS: Record<EngineCause, string> = {
  graxpert_missing: "graxpertMissing",
  graxpert_no_library: "graxpertNoLibrary",
  graxpert_failed: "graxpertFailed",
  siril_missing: "sirilMissing",
  siril_failed: "sirilFailed",
};

/** What a render made by another engine says: the cause, then what did the
 * work instead. */
export function engineNoticeText(t: Translate, notice: EngineNotice): string {
  const cause = notice.causes
    .map((item) => t(`cli.stack.engine.${CAUSE_KEYS[item]}`))
    .join(" ");
  const used = t(
    notice.used === "siril"
      ? "cli.stack.engine.usedSiril"
      : "cli.stack.engine.usedPolynomial",
  );
  return t("cli.stack.engineNotice", { cause, used });
}
