import assert from "node:assert/strict";
import test from "node:test";

import { NOTHING_KEPT, loadPlanKept, savePlanKept } from "./planKept.ts";

function memory(initial: Record<string, string> = {}) {
  const items = new Map(Object.entries(initial));
  return {
    items,
    getItem: (key: string) => items.get(key) ?? null,
    setItem: (key: string, value: string) => void items.set(key, value),
  };
}

test("the frame and the sources kept for a plan come back for that plan only", () => {
  const storage = memory();
  const kept = {
    frame: { top: 958, left: 6, height: 1994, width: 2154 },
    sources: { "n-09|IRCUT_30s_g200": "device" as const },
  };
  savePlanKept(storage, "p-m45", kept);
  assert.deepEqual(loadPlanKept(storage, "p-m45"), kept);
  assert.deepEqual(loadPlanKept(storage, "p-other"), NOTHING_KEPT);
  assert.equal([...storage.items.keys()].join(), "astro-sync.planImage:p-m45");
});

test("anything that cannot be read is nothing kept, and a bad part does not spoil the good", () => {
  assert.deepEqual(loadPlanKept(null, "p"), NOTHING_KEPT);
  assert.deepEqual(
    loadPlanKept(memory({ "astro-sync.planImage:p": "{" }), "p"),
    NOTHING_KEPT,
  );
  assert.deepEqual(
    loadPlanKept(memory({ "astro-sync.planImage:p": "[1]" }), "p"),
    NOTHING_KEPT,
  );
  const partly = memory({
    "astro-sync.planImage:p": JSON.stringify({
      frame: { top: 1, left: 2, height: 0, width: 5 },
      sources: { a: "device", b: "elsewhere", c: "master" },
    }),
  });
  assert.deepEqual(loadPlanKept(partly, "p"), {
    frame: null,
    sources: { a: "device", c: "master" },
  });
  const fractional = memory({
    "astro-sync.planImage:p": JSON.stringify({
      frame: { top: 1.5, left: 2, height: 3, width: 5 },
    }),
  });
  assert.equal(loadPlanKept(fractional, "p").frame, null);
});

test("a storage that refuses to write loses the choice, never the screen", () => {
  const refusing = {
    getItem: () => null,
    setItem: () => {
      throw new Error("quota");
    },
  };
  assert.doesNotThrow(() => savePlanKept(refusing, "p", NOTHING_KEPT));
});
