// The note under the pipelines of « Traitements »: they are a base
// to start from, the person can write their own. Two links: the documentation
// of the format, and the folder where their definitions go. Pure TypeScript,
// tested under `node --test`.

/** The format of a pipeline definition, in the public repository. */
export const PIPELINES_DOC_URL =
  "https://github.com/sqound/astro-sync/blob/main/stacking/pipelines/SCHEMA.md";

export interface PipelinesNoteLink {
  /** The dictionary key of the link's words. */
  labelKey: string;
  /** What the link opens: a web page in the browser, or a folder of the data. */
  kind: "web" | "folder";
  /** The address of a `web` link. */
  target: string | null;
  /** Opens it. */
  open: () => void | Promise<void>;
}

export interface PipelinesNote {
  /** The dictionary key of the sentence. */
  textKey: string;
  links: PipelinesNoteLink[];
}

/** The note and its two links; `openUrl` and `openFolder` are the window's
 * (`openInBrowser`, `pipelinesFolderOpen`). */
export function pipelinesNote(actions: {
  openUrl: (url: string) => void | Promise<void>;
  openFolder: () => void | Promise<void>;
}): PipelinesNote {
  return {
    textKey: "wizard.treatments.pipelinesNote.text",
    links: [
      {
        labelKey: "wizard.treatments.pipelinesNote.doc",
        kind: "web",
        target: PIPELINES_DOC_URL,
        open: () => actions.openUrl(PIPELINES_DOC_URL),
      },
      {
        labelKey: "wizard.treatments.pipelinesNote.folder",
        kind: "folder",
        target: null,
        open: () => actions.openFolder(),
      },
    ],
  };
}
