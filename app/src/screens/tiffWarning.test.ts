import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import test, { before } from "node:test";
import { fileURLToPath } from "node:url";

import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { rolldown } from "rolldown";

// The night's « Récapitulatif » (box « Joindre aussi les TIFF ») and « Envoi
// des rendus » of Réglages carry the same quota warning as « Images à
// envoyer », only while a TIFF is chosen. The components are bundled on the
// fly and drawn to a string.

type Harness = typeof import("./tiffWarningHarness.tsx");
let harness: Harness;
let warning: string;

before(async () => {
  const stubCss = {
    name: "stub-css",
    load(id: string) {
      return id.endsWith(".css")
        ? { code: "export default '';", moduleType: "js" as const }
        : null;
    },
  };
  const bundle = await rolldown({
    input: fileURLToPath(new URL("./tiffWarningHarness.tsx", import.meta.url)),
    platform: "node",
    plugins: [stubCss],
    external: ["react", "react/jsx-runtime", "react-dom"],
    logLevel: "silent",
  });
  const { output } = await bundle.generate({ format: "esm" });
  const folder = new URL(
    "../../node_modules/.cache/astro-sync-tests/",
    import.meta.url,
  );
  await mkdir(folder, { recursive: true });
  const file = new URL("tiff-warning.mjs", folder);
  await writeFile(file, output[0].code);
  harness = await import(file.href);
  const fr = JSON.parse(
    await readFile(new URL("../i18n/fr.json", import.meta.url), "utf8"),
  ) as Record<string, string>;
  warning = fr["detect.send.tiffWarning"].replaceAll("’", "&#x27;");
});

const inFrench = (child: unknown) =>
  renderToStaticMarkup(
    createElement(
      harness.I18nProvider,
      { locale: "fr", onLocaleChange: () => {} },
      child as never,
    ),
  );

const box = (
  checked: boolean,
  fullResolution: { count: number; bytes: number } | null,
) =>
  inFrench(
    createElement(harness.JoinTiffBox, {
      checked,
      fullResolution,
      onChange: () => {},
    }),
  );

const field = (value: "png" | "both") =>
  inFrench(
    createElement(harness.RendersUploadField, { value, onChange: () => {} }),
  );

const KNOWN = { count: 1, bytes: 45_700_000 };

test("the Récapitulatif box shows no warning unticked, the warning ticked", () => {
  assert.ok(!box(false, KNOWN).includes("pèse sur votre quota"));
  const ticked = box(true, KNOWN);
  assert.ok(ticked.includes('role="note"'));
  assert.ok(ticked.includes("Le TIFF pèse sur votre quota"), ticked);
  assert.ok(ticked.indexOf("<input") < ticked.indexOf("pèse sur votre quota"));
});

test("the Récapitulatif box says the weight of the TIFF only when it is known", () => {
  // One TIFF: its own weight. Several: an average, said as such.
  assert.match(box(true, KNOWN), /le TIFF \(pleine résolution, 43,5\sMo\)/);
  assert.match(
    box(true, { count: 4, bytes: 4 * 45_700_000 }),
    /les TIFF \(pleine résolution, ≈\s43,5\sMo en moyenne\)/,
  );
  assert.doesNotMatch(box(true, KNOWN), /chacun|≈/);
  assert.doesNotMatch(box(true, null), /≈/);
});

test("« Envoi des rendus » warns only when PNG + TIFF is the starting value", () => {
  const png = field("png");
  assert.ok(!png.includes("pèse sur votre quota"));
  assert.ok(!png.includes('role="note"'));
  const both = field("both");
  assert.ok(both.includes("Le TIFF pèse sur votre quota"), both);
  assert.ok(both.includes('role="note"'));
  // What the setting does since the import's global box is gone: it starts
  // the Récapitulatif's box only; in « Images à envoyer » the TIFF is a
  // gesture of each card, unticked at first.
  assert.ok(both.includes("Détermine si la case"), both);
  assert.ok(both.includes("Récapitulatif d’une nuit"), both);
  assert.match(both, /Images\sà\senvoyer/);
  assert.ok(both.includes("image par image"), both);
  assert.ok(!both.includes("Valeur de départ"), both);
});

test("the warning is one component, used by the three places and never copied", () => {
  for (const file of [
    "components/JoinTiffBox.tsx",
    "components/RendersUploadField.tsx",
    "screens/detect/SummaryImages.tsx",
  ]) {
    const source = readFileSync(new URL(`../${file}`, import.meta.url), "utf8");
    assert.match(source, /<TiffWarning/, file);
    assert.doesNotMatch(source, /detect\.send\.tiffWarning/, file);
  }
});
