// Every stacking script the app ships must ship the local modules it imports:
// a missing one makes the installed app fail at the first stack or plan image
// (it happened: registration_framing.py, level_match.py and
// plan_framing.py were imported but not bundled).
import { test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { basename, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const stacking = join(here, "../../../stacking");
const conf = JSON.parse(
  readFileSync(join(here, "../../src-tauri/tauri.conf.json"), "utf8"),
);

test("every local module imported by a bundled stacking script is bundled too", () => {
  const bundled = new Set(
    Object.keys(conf.bundle.resources)
      .filter((path) => path.includes("/stacking/"))
      .map((path) => basename(path)),
  );
  const local = new Set(
    readdirSync(stacking).filter(
      (name) => name.endsWith(".py") && !name.startsWith("test_"),
    ),
  );
  const missing = [];
  for (const script of bundled) {
    if (!script.endsWith(".py")) continue;
    const source = readFileSync(join(stacking, script), "utf8");
    for (const [, module] of source.matchAll(
      /^\s*(?:import|from)\s+([a-z_]+)/gm,
    )) {
      const file = `${module}.py`;
      if (local.has(file) && !bundled.has(file))
        missing.push(`${script} → ${file}`);
    }
  }
  assert.deepEqual(missing, []);
});

// The shipped render pipelines: the window « Pipelines pour <cible> » lists
// what is in the bundle's `stacking/pipelines/`, read directly (no copy to
// the data folder). A pipeline of the repository that is not bundled is
// announced by the site and absent from the installed app (it happened:
// `polished` and `hoo` were left out).
test("every shipped pipeline is bundled with all its files", () => {
  const pipelinesDir = join(stacking, "pipelines");
  const resources: Record<string, string> = conf.bundle.resources;
  const bundledTargets = new Set(Object.values(resources));
  const missing: string[] = [];
  const ids = readdirSync(pipelinesDir, { withFileTypes: true })
    .filter((entry) => entry.isDirectory() && !entry.name.startsWith("."))
    .map((entry) => entry.name);
  assert.ok(ids.length >= 4, `pipelines found: ${ids.join(", ")}`);
  for (const id of ids) {
    const files = readdirSync(join(pipelinesDir, id), { recursive: true });
    assert.ok(files.includes("pipeline.json"), `${id} has no pipeline.json`);
    for (const file of files as string[]) {
      if (!statSync(join(pipelinesDir, id, file)).isFile()) continue;
      const target = `stacking/pipelines/${id}/${file}`.replaceAll("\\", "/");
      const source = `../../${target}`;
      if (resources[source] !== target || !bundledTargets.has(target))
        missing.push(target);
    }
  }
  assert.deepEqual(missing, []);
});

test("every bundled pipeline file exists in the repository", () => {
  const stale = Object.keys(conf.bundle.resources)
    .filter((path) => path.startsWith("../../stacking/pipelines/"))
    .filter((path) => !existsSync(join(here, "../../src-tauri", path)));
  assert.deepEqual(stale, []);
});

test("the render functions of the shipped pipelines are in the bundled renders.py", () => {
  const pipelinesDir = join(stacking, "pipelines");
  const renders = readFileSync(join(stacking, "renders.py"), "utf8");
  const missing: string[] = [];
  for (const id of readdirSync(pipelinesDir)) {
    const file = join(pipelinesDir, id, "pipeline.json");
    if (!existsSync(file)) continue;
    const entry: string = JSON.parse(readFileSync(file, "utf8")).entry;
    if (entry.startsWith("renders:")) {
      const fn = entry.slice("renders:".length);
      if (!new RegExp(`^def ${fn}\\(`, "m").test(renders)) missing.push(id);
    } else if (entry.startsWith("script:")) {
      const script = `stacking/pipelines/${id}/${entry.slice(7)}`;
      if (conf.bundle.resources[`../../${script}`] !== script)
        missing.push(`${id} → ${script}`);
    }
  }
  assert.deepEqual(missing, []);
});

// Every shipped pipeline shows a preview of its render in its card
// (`preview` of its `pipeline.json`): the file exists, is a JPEG or a PNG of
// the size the schema advises (at most 150 KiB), and is bundled, like the
// definition, so the installed app can show it.
test("every shipped pipeline declares a preview image that is bundled", () => {
  const pipelinesDir = join(stacking, "pipelines");
  const problems: string[] = [];
  let found = 0;
  for (const id of readdirSync(pipelinesDir)) {
    const file = join(pipelinesDir, id, "pipeline.json");
    if (!existsSync(file)) continue;
    found += 1;
    const { preview } = JSON.parse(readFileSync(file, "utf8")) as {
      preview?: string;
    };
    if (!preview) {
      problems.push(`${id}: no preview`);
      continue;
    }
    if (/^\/|\.\.|\\/.test(preview)) {
      problems.push(`${id}: ${preview} is not a relative path`);
      continue;
    }
    const image = join(pipelinesDir, id, preview);
    if (!existsSync(image)) {
      problems.push(`${id}: ${preview} is missing`);
      continue;
    }
    const bytes = readFileSync(image);
    const isJpeg = bytes[0] === 0xff && bytes[1] === 0xd8 && bytes[2] === 0xff;
    const isPng = bytes.subarray(1, 4).toString("latin1") === "PNG";
    if (!isJpeg && !isPng) problems.push(`${id}: ${preview} is not an image`);
    if (bytes.length > 150 * 1024)
      problems.push(`${id}: ${preview} is ${bytes.length} bytes (max 153600)`);
    const target = `stacking/pipelines/${id}/${preview}`;
    if (conf.bundle.resources[`../../${target}`] !== target)
      problems.push(`${id}: ${target} is not bundled`);
  }
  assert.ok(found >= 4, `pipelines found: ${found}`);
  assert.deepEqual(problems, []);
});
