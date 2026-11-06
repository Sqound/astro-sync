import assert from "node:assert/strict";
import test from "node:test";

import {
  EMPTY_DETECT_WINDOW,
  buildRequest,
  isCancelled,
  isEngineError,
  scanPercent,
  windowReady,
  withFolder,
} from "./detectFlow.ts";

test("the window starts empty with the sub-folders ticked", () => {
  assert.equal(EMPTY_DETECT_WINDOW.subfolders, true);
  assert.equal(windowReady(EMPTY_DETECT_WINDOW), false);
});

test("the frames' folder is required, even when the others are chosen", () => {
  let state = withFolder(EMPTY_DETECT_WINDOW, "calibration", "/c");
  state = withFolder(state, "logs", "/l");
  assert.equal(windowReady(state), false);
  state = withFolder(state, "fits", "/f");
  assert.equal(windowReady(state), true);
  assert.equal(windowReady(withFolder(state, "fits", null)), false);
});

test("the request carries one folder per chosen location and the sub-folders choice", () => {
  const state = withFolder(
    withFolder(EMPTY_DETECT_WINDOW, "fits", "/f"),
    "logs",
    "/l",
  );
  assert.deepEqual(buildRequest(state), {
    fits: ["/f"],
    calibration: [],
    logs: ["/l"],
    subfolders: true,
  });
  assert.equal(buildRequest({ ...state, subfolders: false }).subfolders, false);
});

test("the bar follows the files read and stays empty without a total", () => {
  assert.equal(scanPercent(482, 1423), 34);
  assert.equal(scanPercent(0, 0), 0);
  assert.equal(scanPercent(5, 0), 0);
  assert.equal(scanPercent(9, 3), 100);
});

test("the engine's errors are told apart from a cancellation", () => {
  assert.equal(isEngineError("no_fits_folder"), true);
  assert.equal(isEngineError("folder_missing"), true);
  assert.equal(isEngineError("cancelled"), false);
  assert.equal(isCancelled("cancelled"), true);
});

test("the window opens on the last folders; « Lancer » keeps the three, not the sending's", async () => {
  const { windowFromLast, lastFoldersOf, foundLines } =
    await import("./detectFlow.ts");
  const opened = windowFromLast({ fits: "/a", logs: "/l", sessionFits: "/s" });
  assert.deepEqual(
    [opened.fits, opened.calibration, opened.logs, opened.subfolders],
    ["/a", null, "/l", true],
  );
  assert.deepEqual(
    lastFoldersOf({ ...opened, calibration: "/c" }, { sessionFits: "/s" }),
    {
      sessionFits: "/s",
      fits: "/a",
      calibration: "/c",
      logs: "/l",
    },
  );
  const t = (key: string, values?: Record<string, string | number>) =>
    `${key}${values ? JSON.stringify(values) : ""}`;
  const plural = (key: string, count: number) => `${key}:${count}`;
  assert.equal(foundLines(t, plural, null), null);
  const lines = foundLines(t, plural, {
    nights: 5,
    targets: 3,
    places: 1,
    instruments: 1,
    lights: 9,
    calibrationFiles: 214,
    calibrationSets: 9,
    calibrationKinds: ["dark", "bias"],
    logFiles: 0,
    logKinds: [],
    toSolve: 1,
  })!;
  assert.match(lines.found, /detect.nights:5/);
  assert.match(
    lines.files!,
    /detect.scan.files:214.*detect.scan.kind.dark, detect.scan.kind.bias/,
  );
  assert.equal(lines.solving, "detect.scan.solving:1");
});
