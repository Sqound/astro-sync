import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

// In the step « Cibles », the zone that adds a target (the
// search, the FITS, the form) is under the night's targets, never above them
// -- whether the list is empty, holds one target or several. The screen is
// one JSX tree whose two zones do not depend on the list's content, so their
// order in the source is their order on screen; the states themselves are
// captured in a real window.

const SOURCE = new URL("../screens/night/TargetsStep.tsx", import.meta.url);

test("the night's targets come before the zone that adds targets", async () => {
  const source = await readFile(SOURCE, "utf8");
  const list = source.indexOf('aria-labelledby="targets-list-title"');
  const add = source.indexOf('aria-labelledby="targets-add-title"');
  assert.ok(list > 0 && add > 0, "both zones are in the step");
  assert.ok(list < add, "the list of the night's targets comes first");
  // Both states of the list are inside the first zone, before the second.
  const empty = source.indexOf("wizard.targets.list.emptyTitle");
  const rows = source.indexOf('className="as-choices"');
  assert.ok(list < empty && empty < add);
  assert.ok(list < rows && rows < add);
});

test("the zone that adds is not rendered twice, nor behind a condition on the list", async () => {
  const source = await readFile(SOURCE, "utf8");
  assert.equal(source.match(/targets-add-title/g)?.length, 2); // id + aria
  const before = source.slice(
    source.lastIndexOf("<section", source.indexOf('id="targets-add-title"')),
    source.indexOf('id="targets-add-title"'),
  );
  assert.ok(!/choices\.length/.test(before));
});
