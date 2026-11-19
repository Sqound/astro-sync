import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import test from "node:test";

import {
  openFailureText,
  setOpenFailureReporter,
  type OpenFailure,
} from "./openFailure.ts";
import { SUPPORTED_LOCALES, createTranslator } from "../i18n/locale.ts";

// The window's bridge, with the Tauri side replaced: `invoke` answers what the
// test says, and every call is kept.
type Call = { command: string; args: Record<string, unknown> };
const calls: Call[] = [];
let answer: (call: Call) => unknown = () => undefined;
Object.assign(globalThis, {
  window: {
    __TAURI_INTERNALS__: {
      invoke: async (command: string, args: Record<string, unknown> = {}) => {
        const call = { command, args };
        calls.push(call);
        const result = answer(call);
        if (
          result instanceof Error ||
          (result && typeof result === "object" && "code" in result)
        )
          throw result;
        return result;
      },
      transformCallback: () => 0,
    },
  },
});
const bridge = await import("./openSystem.ts");

function watch(): { failures: OpenFailure[]; stop: () => void } {
  const failures: OpenFailure[] = [];
  const stop = setOpenFailureReporter((failure) => failures.push(failure));
  return { failures, stop };
}

test("« Ouvrir le dossier de travail » opens the folder and says nothing when it works", async () => {
  calls.length = 0;
  answer = () => undefined;
  const { failures, stop } = watch();
  assert.equal(await bridge.folderOpen("/w/night/M 31", "/w"), true);
  assert.deepEqual(calls, [
    { command: "folder_open", args: { folder: "/w/night/M 31" } },
  ]);
  assert.deepEqual(failures, []);
  stop();
});

test("a folder that is not there falls back to the next one, then the refusal is said with the path", async () => {
  calls.length = 0;
  const missing = (folder: unknown) => ({
    code: "folder_missing",
    message: String(folder),
  });
  answer = ({ args }) =>
    args.folder === "/w" ? undefined : missing(args.folder);
  const { failures, stop } = watch();
  assert.equal(await bridge.folderOpen("/w/night/M 31", "/w"), true);
  assert.deepEqual(
    calls.map((call) => call.args.folder),
    ["/w/night/M 31", "/w"],
  );
  assert.deepEqual(failures, []);

  calls.length = 0;
  answer = ({ args }) => missing(args.folder);
  assert.equal(await bridge.folderOpen("/w/night/M 31", "/w"), false);
  assert.equal(failures.length, 1);
  assert.equal(failures[0].kind, "folder");
  assert.equal(failures[0].target, "/w");
  stop();
});

test("a refusal of the system is said, never swallowed: the folder, the file, the link, the pipelines", async () => {
  const { failures, stop } = watch();
  answer = ({ command }) =>
    command === "pipelines_folder_open"
      ? {
          code: "open_failed",
          message: "No application knows this",
          details: { path: "/data/pipelines" },
        }
      : { code: "open_failed", message: "refused" };
  assert.equal(await bridge.folderOpen("/w"), false);
  assert.equal(await bridge.resultFileOpen("/w/a.png"), false);
  assert.equal(await bridge.resultFileReveal("/w/a.png"), false);
  assert.equal(await bridge.pipelinesFolderOpen(), false);
  assert.equal(await bridge.openInBrowser("https://example.test/x"), false);
  assert.deepEqual(
    failures.map((failure) => [failure.kind, failure.target]),
    [
      ["folder", "/w"],
      ["file", "/w/a.png"],
      ["file", "/w/a.png"],
      ["folder", ""],
      ["link", "https://example.test/x"],
    ],
  );
  stop();
});

test("each failure is a sentence of the ten languages with the path in it", () => {
  const cases: OpenFailure[] = [
    {
      kind: "folder",
      target: "/w/M 31",
      caught: { code: "folder_missing", message: "/w/M 31" },
    },
    {
      kind: "folder",
      target: "/w/M 31",
      caught: { code: "open_failed", message: "denied" },
    },
    { kind: "file", target: "/w/M 31/a.png", caught: new Error("denied") },
    { kind: "link", target: "https://example.test/x", caught: "denied" },
    {
      kind: "folder",
      target: "",
      caught: {
        code: "open_failed",
        message: "x",
        details: { path: "/data/pipelines" },
      },
    },
  ];
  assert.equal(SUPPORTED_LOCALES.length, 10);
  for (const locale of SUPPORTED_LOCALES) {
    const t = createTranslator(locale);
    for (const failure of cases) {
      const { text, detail } = openFailureText(t, failure);
      const path = failure.target || "/data/pipelines";
      assert.ok(text.includes(path), `${locale}: ${text}`);
      assert.ok(
        !text.includes("{") && !text.startsWith("open."),
        `${locale}: ${text}`,
      );
      if (
        failure.kind === "folder" &&
        failure.target === "/w/M 31" &&
        failure.caught &&
        (failure.caught as { code: string }).code === "folder_missing"
      )
        assert.equal(detail, undefined);
    }
    assert.equal(openFailureText(t, cases[1]).detail, "denied");
  }
});

test("the window says every failed opening, and no screen swallows one", async () => {
  const app = await readFile(new URL("../App.tsx", import.meta.url), "utf8");
  assert.match(app, /<SnackbarProvider>\s*<OpenFailureReporter \/>/);
  // No opening is answered by an empty `catch`, whatever the screen.
  const roots = ["../screens", "../components"].map(
    (p) => new URL(p, import.meta.url),
  );
  const files: URL[] = [];
  const walk = async (dir: URL) => {
    for (const entry of await readdir(dir, { withFileTypes: true })) {
      const child = new URL(entry.name + (entry.isDirectory() ? "/" : ""), dir);
      if (entry.isDirectory()) await walk(child);
      else if (entry.name.endsWith(".tsx")) files.push(child);
    }
  };
  for (const root of roots) await walk(new URL(root.href + "/"));
  for (const file of files) {
    const source = await readFile(file, "utf8");
    assert.doesNotMatch(
      source,
      /(folderOpen|resultFileOpen|resultFileReveal|openInBrowser|pipelinesFolderOpen)\([^)]*\)\)?\.catch\(\s*\(\)\s*=>\s*(\{\s*\}|undefined)/,
      file.pathname,
    );
  }
});

test("the plugin's own commands the window calls are in the capability file", async () => {
  const capability = JSON.parse(
    await readFile(
      new URL("../../src-tauri/capabilities/default.json", import.meta.url),
      "utf8",
    ),
  ) as { permissions: string[] };
  // The window calls `openUrl` of the plugin (a scoped command: `opener:default`
  // allows http(s), mailto and tel). Opening a path goes through the app's own
  // commands (`folder_open`, `result_file_open`), which no scope refuses.
  const openerSource = await readFile(
    new URL("./openSystem.ts", import.meta.url),
    "utf8",
  );
  const used = [
    ...openerSource.matchAll(
      /import \{([^}]*)\} from "@tauri-apps\/plugin-opener"/g,
    ),
  ].flatMap((m) =>
    m[1]
      .split(",")
      .map((name) => name.trim())
      .filter(Boolean),
  );
  assert.deepEqual(used, ["openUrl"]);
  assert.ok(capability.permissions.includes("opener:default"));
  for (const path of ["screens", "components"]) {
    const dir = new URL(`../${path}/`, import.meta.url);
    for (const name of await readdir(dir)) {
      if (!name.endsWith(".tsx")) continue;
      const source = await readFile(new URL(name, dir), "utf8");
      assert.doesNotMatch(source, /@tauri-apps\/plugin-opener/, name);
    }
  }
});
