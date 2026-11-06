import { useCallback, useEffect, useState } from "react";

import {
  onPythonEnv,
  onToolDownload,
  openInBrowser,
  pythonEnvInstall,
  toolsDetect,
  toolsDownload,
  toolsInstallAll,
  type ToolRow,
} from "../../lib/bridge";
import type { Progress, ToolPill } from "../../lib/pipelineChoice";

/**
 * The downloads of « Traitements »: a missing tool is always
 * offered where it is needed -- the card of a pipeline, the box
 * « Empilement » -- with the engine of Réglages › Dépendances (the
 * registry's download, the Python environment made here, the download page
 * when the registry has no file for this system). The progress of each
 * download, by tool id, follows the core's events; the window may close,
 * the download goes on. When one ends, the tools are read again.
 */
export function useToolInstall(onTools: (rows: ToolRow[]) => void) {
  const [progress, setProgress] = useState<Progress>({});

  useEffect(() => {
    const stops: (() => void)[] = [];
    let stopped = false;
    const keep = (unlisten: () => void) => {
      if (stopped) unlisten();
      else stops.push(unlisten);
    };
    onToolDownload((id, step) =>
      setProgress((current) =>
        id in current
          ? {
              ...current,
              [id]: step.total
                ? Math.min(99, Math.floor((step.bytes / step.total) * 100))
                : null,
            }
          : current,
      ),
    )
      .then(keep)
      .catch(() => undefined);
    onPythonEnv((step) =>
      setProgress((current) =>
        "python-env" in current
          ? {
              ...current,
              "python-env": step.packagesTotal
                ? Math.min(
                    99,
                    Math.floor((step.packagesDone / step.packagesTotal) * 100),
                  )
                : null,
            }
          : current,
      ),
    )
      .then(keep)
      .catch(() => undefined);
    return () => {
      stopped = true;
      for (const stop of stops) stop();
    };
  }, []);

  const settle = useCallback(
    async (ids: readonly string[]) => {
      setProgress((current) => {
        const next = { ...current };
        for (const id of ids) delete next[id];
        return next;
      });
      try {
        onTools(await toolsDetect());
      } catch {
        // The rows stay as they were: the next opening reads them again.
      }
    },
    [onTools],
  );

  /** The button of one pill: its download, its installation, or its page. */
  const install = useCallback(
    async (pill: ToolPill, row: ToolRow | undefined) => {
      if (pill.action?.kind === "page") {
        if (row?.page) await openInBrowser(row.page);
        return;
      }
      setProgress((current) => ({ ...current, [pill.id]: null }));
      try {
        if (pill.action?.kind === "install") await pythonEnvInstall();
        else await toolsDownload(pill.id);
      } catch {
        // The pill comes back with its button: the person can try again.
      }
      await settle([pill.id]);
    },
    [settle],
  );

  /** « Tout installer »: the required tools missing, together. */
  const installAll = useCallback(
    async (ids: readonly string[]) => {
      setProgress((current) => ({
        ...current,
        ...Object.fromEntries(ids.map((id) => [id, null])),
      }));
      try {
        await toolsInstallAll();
      } catch {
        // Each pill says what is still missing.
      }
      await settle(ids);
    },
    [settle],
  );

  return { progress, install, installAll };
}
