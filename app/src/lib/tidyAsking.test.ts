import assert from "node:assert/strict";
import { test } from "node:test";

import { oncePerSubject } from "./tidy.ts";

test("the overview is asked once per subject, however often the screen draws", () => {
  const asked = oncePerSubject();
  const proposal = { sessions: [] };
  const made = { outcome: {} };
  let questions = 0;
  // Five hundred changes of the import's state (a frame stacked, an image
  // prepared, a reading of the space come back) with the same subject.
  for (let change = 0; change < 500; change += 1) {
    if (asked.begin([proposal, made, "symlink", null])) questions += 1;
  }
  assert.equal(questions, 1);
});

test("a new mode, destination, proposal or outcome asks again, and the late answer of the old one is dropped", () => {
  const asked = oncePerSubject();
  const proposal = { sessions: [] };
  const made = { outcome: {} };
  const first = [proposal, made, "symlink", null];
  assert.equal(asked.begin(first), true);
  assert.equal(asked.holds(first), true);
  const second = [proposal, made, "copy", null];
  assert.equal(asked.begin(second), true);
  assert.equal(asked.holds(first), false);
  assert.equal(asked.holds(second), true);
  assert.equal(asked.begin([proposal, made, "copy", "/d"]), true);
  // Same content, another object: the import made it again.
  assert.equal(asked.begin([{ sessions: [] }, made, "copy", "/d"]), true);
  assert.equal(asked.begin([proposal, { outcome: {} }, "copy", "/d"]), true);
});

test("once the screen left, no answer is taken and coming back asks again", () => {
  const asked = oncePerSubject();
  const key = [1, 2, "move", "/d"];
  assert.equal(asked.begin(key), true);
  asked.end();
  assert.equal(asked.holds(key), false);
  assert.equal(asked.begin(key), true);
});
