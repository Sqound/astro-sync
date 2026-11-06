import assert from "node:assert/strict";
import { readFileSync, readdirSync, statSync } from "node:fs";
import test from "node:test";

import { DICTIONARIES, SUPPORTED_LOCALES } from "./locale.ts";

// Every error code the core or the Tauri bridge can hand to the window has a
// sentence of its own in every language: the window looks `errors.<code>` up
// (`formatError`), and a code without a sentence would show its bare key.
//
// The codes are read from the Rust sources: the literals of
// `CommandError::new("<code>", …)` in the bridge, and the codes returned by
// the `code()` of the core's error types the bridge converts. (The codes of
// `analysis/photometry.rs`, `sky_solution.rs` and `precheck.rs` are data
// reasons, said under `sky.*` and `annotations.failure.*`, not errors.)

const root = new URL("../../../", import.meta.url);
const coreSource = new URL("crates/astro-sync-core/src/", root);
const bridgeSource = new URL("app/src-tauri/src/", root);
const errorTypes = [
  "gui_bridge.rs",
  "local_settings.rs",
  "service_wait.rs",
  "session_detection.rs",
  "session_detection/provisional.rs",
  "session_detection/treat.rs",
  "space/mod.rs",
  "uploader.rs",
  "working_folder.rs",
  "detection.rs",
];

function rustFiles(directory: URL): URL[] {
  return readdirSync(directory).flatMap((name) => {
    const url = new URL(name, directory);
    if (statSync(url).isDirectory())
      return rustFiles(new URL(`${name}/`, directory));
    return name.endsWith(".rs") ? [url] : [];
  });
}

/** The body of every `pub fn code(…) { … }` (or `static_code`) of a source. */
function codeBodies(source: string): string[] {
  const bodies: string[] = [];
  for (const match of source.matchAll(
    /pub fn (?:static_)?code\([^)]*\)[^{]*\{/g,
  )) {
    let depth = 1;
    let index = match.index + match[0].length;
    while (depth > 0 && index < source.length) {
      if (source[index] === "{") depth += 1;
      else if (source[index] === "}") depth -= 1;
      index += 1;
    }
    bodies.push(source.slice(match.index, index));
  }
  return bodies;
}

function errorCodes(): Map<string, string> {
  const codes = new Map<string, string>();
  for (const file of rustFiles(bridgeSource)) {
    if (/_tests\.rs$/.test(file.pathname)) continue;
    const source = readFileSync(file, "utf8");
    for (const match of source.matchAll(
      /CommandError::new\(\s*"([a-z0-9_]+)"/g,
    ))
      codes.set(match[1], file.pathname);
  }
  for (const name of errorTypes) {
    const source = readFileSync(new URL(name, coreSource), "utf8");
    for (const body of codeBodies(source))
      for (const match of body.matchAll(
        /=>\s*"([a-z0-9_]+)"|^\s*"([a-z0-9_]+)"\s*[,|]/gm,
      ))
        codes.set(match[1] ?? match[2], name);
  }
  // The refusals of the service the core knows (`refusal.rs`, `KNOWN`).
  const refusals = readFileSync(new URL("refusal.rs", coreSource), "utf8");
  const known = refusals.slice(refusals.indexOf("const KNOWN"));
  for (const match of known
    .slice(0, known.indexOf("];"))
    .matchAll(/\(\s*"([a-z0-9_]+)",\s*"/g))
    codes.set(match[1], "refusal.rs");
  return codes;
}

test("the sources are read: the known codes are found", () => {
  const codes = errorCodes();
  for (const known of [
    "network_error",
    "not_paired",
    "night_folder_missing",
    "project_not_found",
    "invalid_night",
    "sign_in_code_incorrect",
    "capture_limit_reached",
  ])
    assert.ok(codes.has(known), `${known} was not found in the sources`);
  assert.ok(codes.size > 50, `only ${codes.size} codes found`);
});

test("every error code of the core and of the bridge has a sentence in every language", () => {
  const missing: string[] = [];
  for (const [code, where] of errorCodes())
    for (const locale of SUPPORTED_LOCALES)
      if (!DICTIONARIES[locale]?.[`errors.${code}`])
        missing.push(`${locale}: errors.${code} (${where})`);
  assert.deepEqual(missing, []);
});
