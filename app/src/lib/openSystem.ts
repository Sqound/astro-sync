// Everything the window asks the system to open: a folder, a file, a web
// address. None of these ever rejects: a refusal is said by the window
// (`openFailure`, the path in it) and the answer is whether it opened. Kept
// apart from `bridge.ts` so a test can load it with a stand-in for Tauri.
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";

import { reportOpenFailure } from "./openFailure.ts";

/** « Ouvrir le dossier des pipelines »: the folder of the person's
 * own definitions, created when missing, opened by the system. Never
 * rejects: a refusal is said by the window (`openFailure`), and the answer
 * is whether it opened. */
export async function pipelinesFolderOpen(): Promise<boolean> {
  try {
    await invoke<void>("pipelines_folder_open");
    return true;
  } catch (caught) {
    reportOpenFailure({ kind: "folder", target: "", caught });
    return false;
  }
}

/** Opens a web address in the system browser (only `http(s)` addresses the
 * service or the space announced). Never rejects: a refusal is said by the
 * window, with the address to copy. */
export async function openInBrowser(url: string): Promise<boolean> {
  try {
    await openUrl(url);
    return true;
  } catch (caught) {
    reportOpenFailure({ kind: "link", target: url, caught });
    return false;
  }
}

/** Opens an existing folder in the Finder, the Explorer or the file manager.
 * When it is not there, the next of `instead` is tried (the folder of a
 * target, then the working folder). Never rejects: what no folder could be
 * opened is said by the window (`openFailure`, the path in it), and the answer
 * is whether one opened. */
export async function folderOpen(
  folder: string,
  ...instead: string[]
): Promise<boolean> {
  const candidates = [folder, ...instead];
  for (const [index, candidate] of candidates.entries()) {
    try {
      await invoke<void>("folder_open", { folder: candidate });
      return true;
    } catch (caught) {
      const missing =
        typeof caught === "object" &&
        caught !== null &&
        (caught as { code?: unknown }).code === "folder_missing";
      if (missing && index < candidates.length - 1) continue;
      reportOpenFailure({ kind: "folder", target: candidate, caught });
      return false;
    }
  }
  return false;
}

/** « Ouvrir »: an image of « Images à envoyer » opened by the system's usual
 * application. */
export async function resultFileOpen(path: string): Promise<boolean> {
  try {
    await invoke<void>("result_file_open", { path });
    return true;
  } catch (caught) {
    reportOpenFailure({ kind: "file", target: path, caught });
    return false;
  }
}

/** « Afficher dans le Finder »: the image selected in its folder. */
export async function resultFileReveal(path: string): Promise<boolean> {
  try {
    await invoke<void>("result_file_reveal", { path });
    return true;
  } catch (caught) {
    reportOpenFailure({ kind: "file", target: path, caught });
    return false;
  }
}
