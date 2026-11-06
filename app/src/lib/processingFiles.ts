// « Fichiers de traitement » of Réglages › Général: what the section says of
// the scan the core made (`processing_files_scan`) and of the boxes of the
// confirmation. Pure TypeScript, tested under `node --test`.
//
// The rule behind every line: what the person copied,
// moved or linked into the working folder is never cleaned and never
// counted as a thing to clean -- it is only said, as « Vos fichiers ». What
// is cleanable is what the pipelines produced.

import type { TranslateParams } from "../i18n/locale.ts";
import type {
  ProcessingCategory,
  ProcessingCleanKind,
  ProcessingKind,
  ProcessingScan,
} from "./bridge";
import { conjunction, formatSize, oneDecimal, pluralText } from "./written.ts";

type Translate = (key: string, params?: TranslateParams) => string;

/** A size as this section writes it: whole MB, one decimal for GB (« 168 Mo
 * », « 1,0 Go »); under a megabyte the smaller unit, so a small file is
 * never said « 0 Mo ». `null`: « — » (nothing was measured). */
export function processingSize(
  t: Translate,
  bytes: number | null,
  locale: string,
): string {
  if (bytes === null) return "—";
  if (bytes >= 1_000_000_000) {
    return t("common.gigabytes", {
      value: oneDecimal(bytes / 1_000_000_000, locale),
    });
  }
  if (bytes >= 500_000) {
    return t("common.megabytes", {
      value: new Intl.NumberFormat(locale, {
        maximumFractionDigits: 0,
      }).format(Math.round(bytes / 1_000_000)),
    });
  }
  if (bytes === 0) return t("common.megabytes", { value: "0" });
  return formatSize(t, bytes, locale);
}

/** « 18:52 »: the hour of the calculation, on 24 hours in every language. */
export function stampHour(computedAt: string, locale: string): string {
  const date = new Date(computedAt);
  if (Number.isNaN(date.getTime())) return "—";
  return new Intl.DateTimeFormat(locale, {
    hour: "2-digit",
    minute: "2-digit",
    hourCycle: "h23",
  }).format(date);
}

export interface FileRowView {
  kind: ProcessingKind;
  name: string;
  /** The grey line under the name; none when the folder is unreachable. */
  note: string | null;
  /** `null` while nothing was measured (« — »). */
  bytes: number | null;
}

export interface FilesView {
  reachable: boolean;
  rows: FileRowView[];
  /** The produced files together, and the weight of the whole folder. */
  total: { bytes: number | null; note: string | null };
  /** « Vos fichiers (copiés ou liés) »: never touched. */
  user: { bytes: number | null };
  python: { bytes: number };
  stamp: string | null;
  /** Something the pipelines produced is there: « Nettoyer… » is active. */
  cleanable: boolean;
}

const KIND_ORDER: ProcessingKind[] = [
  "masters",
  "nightResults",
  "planRenders",
  "temp",
];

function categoryOf(scan: ProcessingScan, kind: ProcessingKind) {
  return scan.categories.find((category) => category.kind === kind);
}

/** The cleanable total: what the four categories weigh. */
export function producedBytes(scan: ProcessingScan): number {
  return scan.categories.reduce((sum, category) => sum + category.bytes, 0);
}

function rowNote(
  t: Translate,
  locale: string,
  category: ProcessingCategory,
): string {
  if (category.bytes === 0 && category.count === 0) {
    return t(
      category.kind === "temp"
        ? "settings.processing.temp.noneNote"
        : "settings.processing.noneNote",
    );
  }
  switch (category.kind) {
    case "masters":
    case "nightResults":
      return pluralText(
        t,
        locale,
        "settings.processing.inNights",
        category.count,
      );
    case "planRenders":
      return pluralText(
        t,
        locale,
        "settings.processing.inPlans",
        category.count,
      );
    case "temp":
      return t("settings.processing.temp.note");
  }
}

/** What the section shows for `scan`. */
export function filesView(
  t: Translate,
  locale: string,
  scan: ProcessingScan,
): FilesView {
  const rows = KIND_ORDER.map((kind): FileRowView => {
    const category = categoryOf(scan, kind);
    const name = t(`settings.processing.${kind}.name`);
    if (!category || !scan.reachable) {
      // The temporaries of an older build live in Astro Sync's data, and
      // stay measured; every other size is « — » while the folder is out.
      if (kind === "temp" && category && category.bytes > 0) {
        return {
          kind,
          name,
          note: rowNote(t, locale, category),
          bytes: category.bytes,
        };
      }
      return { kind, name, note: null, bytes: null };
    }
    return {
      kind,
      name,
      note: rowNote(t, locale, category),
      bytes: category.bytes,
    };
  });
  const produced = producedBytes(scan);
  return {
    reachable: scan.reachable,
    rows,
    total: scan.reachable
      ? {
          bytes: produced,
          note: t("settings.processing.total.note", {
            size: processingSize(t, scan.folderBytes, locale),
          }),
        }
      : { bytes: null, note: null },
    user: { bytes: scan.reachable ? scan.userBytes : null },
    python: { bytes: scan.pythonEnvBytes },
    stamp: scan.reachable
      ? t("settings.processing.stamp", {
          time: stampHour(scan.computedAt, locale),
        })
      : null,
    cleanable: produced > 0,
  };
}

/* ---------------------------- The confirmation ---------------------------- */

export type BoxId = "masters" | "sent" | "unsent" | "temp";

export interface CleanBox {
  id: BoxId;
  /** What the core is asked to delete when it is ticked. */
  kind: ProcessingCleanKind;
  title: string;
  note: string;
  /** The amber line: what the box does that nothing undoes. */
  warning: string | null;
  bytes: number;
  /** Ticked when the sheet opens: the results not sent never are. */
  checkedByDefault: boolean;
}

/** « 5 nuits et 1 plan »: the nights and the plans a box holds, in words. */
export function holdsText(
  t: Translate,
  locale: string,
  nights: number,
  plans: number,
): string {
  const parts: string[] = [];
  if (nights > 0) {
    parts.push(pluralText(t, locale, "settings.processing.nights", nights));
  }
  if (plans > 0) {
    parts.push(pluralText(t, locale, "settings.processing.plans", plans));
  }
  return conjunction(locale, parts);
}

/** The boxes of the sheet, in the order of the screen; a box with nothing in
 * it is left out. */
export function cleanBoxes(
  t: Translate,
  locale: string,
  scan: ProcessingScan,
): CleanBox[] {
  const nights = categoryOf(scan, "nightResults");
  const plans = categoryOf(scan, "planRenders");
  const masters = categoryOf(scan, "masters");
  const temp = categoryOf(scan, "temp");
  const sentBytes = (nights?.sentBytes ?? 0) + (plans?.sentBytes ?? 0);
  const unsentBytes = (nights?.unsentBytes ?? 0) + (plans?.unsentBytes ?? 0);
  const boxes: CleanBox[] = [];
  if (masters && masters.bytes > 0) {
    boxes.push({
      id: "masters",
      kind: "masters",
      title: t("settings.processing.masters.name"),
      note: t("settings.processing.sheet.masters.note"),
      warning: null,
      bytes: masters.bytes,
      checkedByDefault: true,
    });
  }
  if (sentBytes > 0) {
    boxes.push({
      id: "sent",
      kind: "sentResults",
      title: t("settings.processing.sheet.sent.title"),
      note: t("settings.processing.sheet.sent.note", {
        what: holdsText(
          t,
          locale,
          nights?.sentCount ?? 0,
          plans?.sentCount ?? 0,
        ),
      }),
      warning: null,
      bytes: sentBytes,
      checkedByDefault: true,
    });
  }
  if (unsentBytes > 0) {
    boxes.push({
      id: "unsent",
      kind: "unsentResults",
      title: t("settings.processing.sheet.unsent.title"),
      note: t("settings.processing.sheet.unsent.note", {
        what: holdsText(
          t,
          locale,
          nights?.unsentCount ?? 0,
          plans?.unsentCount ?? 0,
        ),
      }),
      warning: t("settings.processing.sheet.unsent.warning"),
      bytes: unsentBytes,
      checkedByDefault: false,
    });
  }
  if (temp && temp.bytes > 0) {
    boxes.push({
      id: "temp",
      kind: "temp",
      title: t("settings.processing.temp.name"),
      note: t("settings.processing.sheet.temp.note"),
      warning: null,
      bytes: temp.bytes,
      checkedByDefault: true,
    });
  }
  return boxes;
}

/** The boxes ticked when the sheet opens. */
export function defaultTicks(boxes: readonly CleanBox[]): BoxId[] {
  return boxes.filter((box) => box.checkedByDefault).map((box) => box.id);
}

/** What the primary button says it deletes: the sum of the ticked boxes. */
export function tickedBytes(
  boxes: readonly CleanBox[],
  ticked: readonly BoxId[],
): number {
  return boxes
    .filter((box) => ticked.includes(box.id))
    .reduce((sum, box) => sum + box.bytes, 0);
}

/** What the core is asked to delete for the ticked boxes. */
export function kindsOf(
  boxes: readonly CleanBox[],
  ticked: readonly BoxId[],
): ProcessingCleanKind[] {
  return boxes.filter((box) => ticked.includes(box.id)).map((box) => box.kind);
}
