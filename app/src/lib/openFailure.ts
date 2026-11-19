import type { Translate } from "../i18n/locale.ts";

/** What the person asked the system to open. */
export type OpenKind = "folder" | "file" | "link";

/** An opening the system (or Astro Sync) refused. */
export interface OpenFailure {
  kind: OpenKind;
  /** The folder, the file or the web address. */
  target: string;
  /** What the command (or the opener plugin) rejected with. */
  caught: unknown;
}

type Reporter = (failure: OpenFailure) => void;

let reporter: Reporter | null = null;

/** The window registers how a failed opening is said (a snackbar); the
 * returned function unregisters it. */
export function setOpenFailureReporter(next: Reporter): () => void {
  reporter = next;
  return () => {
    if (reporter === next) reporter = null;
  };
}

/** A failed opening is never silent: it goes to the window's reporter. */
export function reportOpenFailure(failure: OpenFailure): void {
  if (reporter) reporter(failure);
  else
    console.error(
      "could not open",
      failure.kind,
      failure.target,
      failure.caught,
    );
}

function codeOf(caught: unknown): string | null {
  return typeof caught === "object" &&
    caught !== null &&
    typeof (caught as { code?: unknown }).code === "string"
    ? (caught as { code: string }).code
    : null;
}

/** The path a command said it could not open (`details.path`). */
function pathOf(caught: unknown): string | null {
  const details =
    typeof caught === "object" && caught !== null
      ? (caught as { details?: unknown }).details
      : null;
  const path =
    typeof details === "object" && details !== null
      ? (details as { path?: unknown }).path
      : null;
  return typeof path === "string" && path ? path : null;
}

function reasonOf(caught: unknown): string | null {
  if (typeof caught === "string") return caught || null;
  if (caught instanceof Error) return caught.message || null;
  const message =
    typeof caught === "object" && caught !== null
      ? (caught as { message?: unknown }).message
      : null;
  return typeof message === "string" && message ? message : null;
}

/** The sentence for a failed opening, with the path or the address in it and
 * what to do, and the system's own reason as a second line. */
export function openFailureText(
  t: Translate,
  failure: OpenFailure,
): { text: string; detail?: string } {
  const { kind, caught } = failure;
  const target = pathOf(caught) ?? failure.target;
  if (kind === "folder" && codeOf(caught) === "folder_missing")
    return { text: t("open.missingFolder", { path: target }) };
  const key =
    kind === "folder"
      ? "open.failedFolder"
      : kind === "file"
        ? "open.failedFile"
        : "open.failedLink";
  const reason = reasonOf(caught);
  return {
    text: t(key, { path: target }),
    ...(reason && reason !== target ? { detail: reason } : {}),
  };
}
