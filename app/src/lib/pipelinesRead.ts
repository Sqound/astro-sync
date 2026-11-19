// What the page « Traitements » of the import may do while the pipelines of
// this computer are being read (the script of the stacking answers after a
// while, the tools are asked for their version): the cards are there at once,
// what each one runs is not known yet. Pure TypeScript, tested under
// `node --test`.

/** The reading of the pipelines: under way, done, or failed. */
export type PipelinesStatus = "loading" | "ready" | "failed";

/** What the page offers for a reading at this point. */
export interface PipelinesActions {
  /** The window « Pipelines pour {cible} » may open: the pipelines are read
   * and there is something to choose from. The tools may still be read: the
   * window says so itself, and holds its « Valider » until they are. */
  edit: boolean;
  /** The treatments may start: each card knows what it runs. */
  launch: boolean;
  /** What the zone of the pipelines says. */
  zone: "waiting" | "failed" | "none" | "listed";
}

export function pipelinesActions(
  status: PipelinesStatus,
  pipelineCount: number,
): PipelinesActions {
  if (status === "loading") {
    return { edit: false, launch: false, zone: "waiting" };
  }
  if (status === "failed") {
    return { edit: false, launch: false, zone: "failed" };
  }
  if (pipelineCount === 0) {
    return { edit: false, launch: false, zone: "none" };
  }
  return { edit: true, launch: true, zone: "listed" };
}
