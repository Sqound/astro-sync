import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import fr from "../../../i18n/fr.json" with { type: "json" };
import en from "../../../i18n/en.json" with { type: "json" };
import { pluralText } from "../../../lib/written.ts";

const dir = new URL("./", import.meta.url);
const foot = readFileSync(new URL("SummaryFoot.tsx", dir), "utf8");
const step = readFileSync(new URL("../ReviewStep.tsx", dir), "utf8");
const nights = readFileSync(new URL("NightsCard.tsx", dir), "utf8");
const css = readFileSync(
  new URL("../../../theme/detect-review.css", dir),
  "utf8",
);

const say =
  (dictionary: Record<string, string>) =>
  (key: string, params: Record<string, string | number> = {}) =>
    dictionary[key].replace(/\{(\w+)\}/g, (_, name) => String(params[name]));

test("the reason names the night, or the count and the first", () => {
  const t = say(fr);
  assert.equal(
    pluralText(t, "fr", "detect.create.why.filter", 1, { date: "14 août" }),
    "Précisez la bande de la nuit du 14 août pour créer.",
  );
  assert.equal(
    pluralText(t, "fr", "detect.create.why.filter", 3, { date: "14 août" }),
    "Précisez la bande de 3 nuits, la première étant celle du 14 août, pour créer.",
  );
  assert.match(
    pluralText(say(en), "en", "detect.create.why.filter", 2, {
      date: "Aug 14",
    }),
    /2 nights.*Aug 14/,
  );
});

test("the reason shows only beside the greyed primary and its link leads to the choice", () => {
  assert.match(foot, /label\.kind === "none" && first/);
  assert.match(foot, /id="as-review-why"/);
  // The primary of the foot is described by it while it waits.
  assert.match(
    step,
    /describedBy:\s+!readOnly &&\s+\(unnamed\.length > 0 \|\|\s+\(label\.kind === "none" && waiting\.length > 0\)\)/,
  );
  assert.match(foot, /detect\.create\.why\.go/);
  assert.match(foot, /scrollIntoView/);
  assert.match(foot, /\.focus\(/);
  assert.match(foot, /classList\.add\("is-called"\)/);
});

test("the band choice stays marked until answered, and is highlighted when called", () => {
  assert.match(nights, /answer \? " is-answered" : " is-unanswered"/);
  assert.match(css, /\.as-review__ask\.is-unanswered\s*\{[^}]*outline/);
  assert.match(
    css,
    /\.as-review__ask\.is-unanswered\.is-called\s*\{[^}]*gold-accent/,
  );
});
