import assert from "node:assert/strict";
import test from "node:test";

import { createTranslator } from "../i18n/locale.ts";
import { engineNoticeText } from "./engineNotice.ts";

test("a fallback of the engine says its cause and what did the work", () => {
  const t = createTranslator("fr");
  const text = engineNoticeText(t, {
    used: "siril",
    causes: ["graxpert_no_library"],
  });
  assert.match(text, /onnxruntime/u);
  assert.match(text, /Siril/u);
  assert.doesNotMatch(text, /\{/u);
});
