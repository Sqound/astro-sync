import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

// What the pure tests cannot see: the screens are wired to them.

const source = (relative: string) =>
  readFileSync(new URL(`../${relative}`, import.meta.url), "utf8");

test("the sign-in card keeps the server being typed, and its keyboard focus while the server is checked", () => {
  const card = source("screens/SpaceScreen.tsx");
  assert.match(card, /ownServerDraft\.remember\(/);
  assert.match(card, /ownServerDraft\.recall\(\)/);
  assert.match(card, /ownServerDraft\.forget\(\)/);
  // « Vérifier le serveur » is never `disabled` because it is checking:
  // a disabled button drops the focus and Tab starts again from the menu.
  assert.doesNotMatch(
    card,
    /(?<!aria-)disabled=\{[^}]*probe\.state === "checking"/,
  );
  assert.match(card, /aria-disabled=\{probe\.state === "checking"/);
  assert.match(card, /verifyButton\.current\?\.focus\(\)/);
});

test("Dependencies shows the place of the pipelines, with a wait, before they are read", () => {
  const tab = source("screens/DependenciesTab.tsx");
  assert.match(tab, /sections === null && \(/);
  assert.match(tab, /common\.wait\.pipelines/);
});
