import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { SUPPORTED_LOCALES } from "../i18n/locale.ts";

// With no session yet, the Sessions screen must not leave a new person
// with « Create a session » alone -- the detection of the sessions from
// the capture folders is the normal first gesture.

const source = (relative: string) =>
  readFileSync(new URL(`../${relative}`, import.meta.url), "utf8");

/** The JSX of the empty state: from its opening `{empty && (` to the next
 * block of the screen. */
function emptyBlock(): string {
  const text = source("screens/SessionsScreen.tsx");
  const start = text.search(/\{empty && \(\n\s+<div/);
  assert.ok(start >= 0, "the empty state is drawn");
  const end = text.indexOf("{rows && rows.length > 0", start);
  assert.ok(end > start, "the empty state ends before the list");
  return text.slice(start, end);
}

test("the empty state offers the detection as the primary and the creation as the secondary, in one row", () => {
  const block = emptyBlock();
  const buttons = [...block.matchAll(/<button[\s\S]*?<\/button>/g)].map(
    (match) => match[0],
  );
  assert.equal(buttons.length, 2, "two actions, no more");
  const [first, second] = buttons;
  assert.match(first, /buttonClassName\("primary"\)/);
  assert.match(first, /onClick=\{onImport\}/);
  assert.match(first, /t\("sessions\.detect"\)/);
  assert.match(second, /buttonClassName\("secondary"\)/);
  assert.match(second, /setCreating\(true\)/);
  assert.match(second, /t\("nights\.create"\)/);
  assert.match(block, /as-empty__actions/, "both sit in the same row");
  assert.match(
    block,
    /t\("nights\.emptyDetect"\)/,
    "the detection is explained",
  );
});

test("the explanation of the detection is written in every language", () => {
  for (const locale of SUPPORTED_LOCALES) {
    const dictionary = JSON.parse(source(`i18n/${locale}.json`)) as Record<
      string,
      string
    >;
    assert.ok(
      (dictionary["nights.emptyDetect"] ?? "").trim().length > 0,
      locale,
    );
  }
});
