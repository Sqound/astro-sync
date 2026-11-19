import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

// What can be selected: the interface does not (like a native application),
// the zones where copying is useful do.

const read = (relative: string) =>
  readFile(new URL(relative, import.meta.url), "utf8");

function rule(source: string, selector: string): string {
  const start = source.indexOf(`${selector} {`);
  assert.ok(start >= 0, `selection.css has no rule for ${selector}`);
  return source.slice(start, source.indexOf("}", start));
}

test("the whole window is not selectable, in one rule", async () => {
  const body = rule(await read("./selection.css"), "html,\nbody");
  assert.match(body, /-webkit-user-select: none/);
  assert.match(body, /\buser-select: none/);
});

test("the zones to copy, and the fields, are selectable by one class or by nature", async () => {
  const source = await read("./selection.css");
  const text = rule(
    source,
    '.as-selectable,\n.as-selectable *,\ncode,\npre,\nkbd,\nsamp,\ninput,\ntextarea,\n[contenteditable]:not([contenteditable="false"])',
  );
  assert.match(text, /-webkit-user-select: text/);
  assert.match(text, /\buser-select: text/);
  // A button inside such a zone stays a button.
  assert.match(
    rule(
      source,
      '.as-selectable button,\n.as-selectable [role="button"],\n.as-selectable summary',
    ),
    /user-select: none/,
  );
});

test("while something is dragged nothing is selectable, in any zone, and it wins over every zone", async () => {
  const dragging = rule(
    await read("./selection.css"),
    ".as-dragging,\n.as-dragging *",
  );
  assert.match(dragging, /-webkit-user-select: none !important/);
  assert.match(dragging, /\buser-select: none !important/);
});

test("the sheet is loaded by the theme, before the others", async () => {
  const index = await read("./index.ts");
  assert.ok(index.includes('import "./selection.css";'));
  assert.ok(
    index.indexOf("selection.css") < index.indexOf("primitives.css"),
    "selection.css comes first",
  );
});

test("the guard is installed by the window and the curtain declares itself a drag surface", async () => {
  const main = await read("../main.tsx");
  assert.match(main, /installDragGuard\(window\)/);
  const adjust = await read("../screens/adjust/AdjustScreen.tsx");
  assert.match(adjust, /className="as-adjust__frame"\s+data-as-drag/);
});

/** The zones of the application where selecting and copying is useful, with
 * what they hold: the file at the left must carry `as-selectable`. */
const ZONES: [string, string][] = [
  ["../components/Notice.tsx", "errors and warnings (every screen's banner)"],
  [
    "../components/Snackbar.tsx",
    "the messages that name a file or an identifier",
  ],
  ["../components/BuildVersion.tsx", "the version of the build, in the menu"],
  ["../screens/SettingsScreen.tsx", "the working folder, the version line"],
  [
    "../screens/detect/FoldersStep.tsx",
    "the folder chosen, the path being read",
  ],
  ["../screens/night/SessionLocations.tsx", "the folders of a night"],
  ["../screens/ProcessingFiles.tsx", "the names of the files"],
  [
    "../screens/space/ConnectedSpace.tsx",
    "the sign-in address, the web address",
  ],
  ["../screens/space/PrivateServer.tsx", "the address of the private service"],
  ["../components/TrustDialog.tsx", "the digest of the file"],
  ["../screens/DryRunScreen.tsx", "the paths and digests of the simulation"],
  [
    "../screens/adjust/SavedAdjustments.tsx",
    "the errors of the saved settings",
  ],
  ["../screens/QueueScreen.tsx", "the file names and the reasons of the queue"],
  ["../components/PoseStatsCard.tsx", "the failure of the measure"],
];

test("every zone worth copying carries the class", async () => {
  for (const [file, what] of ZONES) {
    const source = await read(file);
    assert.ok(source.includes("as-selectable"), `${file}: ${what}`);
  }
});

test("a path, a code or an identifier written as `code` or `pre` is selectable by nature", async () => {
  const source = await read("./selection.css");
  for (const tag of ["code", "pre", "input", "textarea"]) {
    assert.match(source, new RegExp(`^${tag},?$`, "m"), tag);
  }
});
