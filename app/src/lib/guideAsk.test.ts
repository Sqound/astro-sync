import assert from "node:assert/strict";
import test from "node:test";

import { SUPPORTED_LOCALES, createTranslator } from "../i18n/locale.ts";
import type { ManagedFile } from "./bridge.ts";
import {
  GUIDE_LANGUAGES,
  afterGuideCopy,
  guideFileLine,
  guideFileState,
  guideNotices,
  languageName,
  prepareGuideAsk,
  replaceManagedFile,
  type GuideAskDeps,
} from "./guideAsk.ts";

// « Poser une question à une IA ».

const fr = createTranslator("fr");
const ROOT = "/home/marie/Documents/Astro Sync";

function file(name: string, outcome: ManagedFile["outcome"]): ManagedFile {
  return {
    name,
    path: `${ROOT}/${name}`,
    outcome,
    sha256: null,
    bytes: 5812,
    chars: 5812,
  };
}

/** The bridge, as the stand-in answers it: each call is noted. */
function deps(
  options: {
    language?: string;
    guide?: ManagedFile["outcome"] | "failed";
    readme?: ManagedFile["outcome"] | null;
    prepareFails?: boolean;
    buildFails?: boolean;
  } = {},
): { deps: GuideAskDeps; calls: string[]; built: unknown[] } {
  const language = options.language ?? "fr";
  const calls: string[] = [];
  const built: unknown[] = [];
  return {
    calls,
    built,
    deps: {
      language,
      render: (lang, root) => {
        calls.push(`render:${lang}:${root}`);
        return `# Guide ${lang}\n`;
      },
      prepare: async (lang) => {
        calls.push(`prepare:${lang}`);
        if (options.prepareFails) throw { code: "io_error", message: "no" };
        return {
          root: ROOT,
          assistantFile: { outcome: "unchanged" },
          readme:
            options.readme === null
              ? null
              : file("README.md", options.readme ?? "unchanged"),
        };
      },
      write: async (lang, text) => {
        calls.push(`write:${lang}:${text.length}`);
        if (options.guide === "failed") {
          throw { code: "io_error", message: "disk full" };
        }
        return {
          root: ROOT,
          guide: file(
            `GUIDE-ASTRO-SYNC.${lang}.md`,
            options.guide ?? "created",
          ),
        };
      },
      build: async (lang, input) => {
        calls.push(`build:${lang}`);
        built.push(input);
        if (options.buildFails) throw new Error("no");
        return `prompt ${lang} ${input.guideFile ?? "-"}`;
      },
      rootFallback: async () => ROOT,
    },
  };
}

test("the ten languages of the list are the ten of the application, in the same order", () => {
  assert.deepEqual([...GUIDE_LANGUAGES].sort(), [...SUPPORTED_LOCALES].sort());
  assert.equal(GUIDE_LANGUAGES.length, 10);
  assert.deepEqual(GUIDE_LANGUAGES.slice(0, 3), ["fr", "en", "ja"]);
});

test("a click: the folder is made ready, the guide written in the language, the prompt built with its file", async () => {
  const run = deps();
  const result = await prepareGuideAsk(run.deps);
  assert.deepEqual(run.calls, [
    "prepare:fr",
    `render:fr:${ROOT}`,
    "write:fr:" + "# Guide fr\n".length,
    "build:fr",
  ]);
  assert.deepEqual(run.built, [
    { root: ROOT, guideFile: "GUIDE-ASTRO-SYNC.fr.md" },
  ]);
  assert.equal(result.prompt, "prompt fr GUIDE-ASTRO-SYNC.fr.md");
  assert.equal(result.guide.kind, "ready");
  assert.equal(result.readme?.outcome, "unchanged");
  assert.deepEqual(result.assistantFile, { kind: "ready" });
  assert.equal(result.text, "# Guide fr\n");
});

test("another language writes its own guide and builds its own prompt", async () => {
  const run = deps({ language: "ja" });
  const result = await prepareGuideAsk(run.deps);
  assert.equal(result.language, "ja");
  assert.equal(result.prompt, "prompt ja GUIDE-ASTRO-SYNC.ja.md");
  assert.ok(run.calls.includes("write:ja:" + "# Guide ja\n".length));
});

test("the guide the person changed is kept, said, and the prompt cites the file as it is", async () => {
  const result = await prepareGuideAsk(deps({ guide: "kept_as_changed" }).deps);
  assert.equal(result.guide.kind, "kept");
  assert.equal(result.prompt, "prompt fr GUIDE-ASTRO-SYNC.fr.md");
  const notices = guideNotices(
    result,
    fr,
    () => "?",
    (l) => `GUIDE-ASTRO-SYNC.${l}.md`,
  );
  assert.equal(notices.length, 1);
  assert.equal(notices[0].key, "guide-file");
  assert.equal(notices[0].tone, "warning");
  assert.equal(notices[0].mode, "persistent");
  assert.equal(notices[0].replace, "guide");
  assert.equal(
    notices[0].text,
    "GUIDE-ASTRO-SYNC.fr.md a été modifié : il ne suit pas la version d’Astro Sync et n’a pas été remplacé.",
  );
});

test("the README the person changed is kept and said as well; the prompt does not depend on it", async () => {
  const result = await prepareGuideAsk(
    deps({ readme: "kept_as_changed" }).deps,
  );
  assert.equal(result.guide.kind, "ready");
  assert.ok(result.prompt);
  const notices = guideNotices(
    result,
    fr,
    () => "?",
    (l) => l,
  );
  assert.deepEqual(
    notices.map((n) => [n.key, n.replace]),
    [["readme-file", "readme"]],
  );
  assert.match(notices[0].text, /^README\.md a été modifié/u);
});

test("both changed: two notices, the guide first", async () => {
  const result = await prepareGuideAsk(
    deps({ guide: "kept_as_changed", readme: "kept_as_changed" }).deps,
  );
  assert.deepEqual(
    guideNotices(
      result,
      fr,
      () => "?",
      (l) => l,
    ).map((n) => n.replace),
    ["guide", "readme"],
  );
});

test("a guide as it should be says nothing", async () => {
  for (const outcome of ["created", "rewritten", "unchanged"] as const) {
    const result = await prepareGuideAsk(deps({ guide: outcome }).deps);
    assert.deepEqual(
      guideNotices(
        result,
        fr,
        () => "?",
        (l) => l,
      ),
      [],
    );
  }
  const none = await prepareGuideAsk(deps({ readme: null }).deps);
  assert.equal(none.readme, null);
  assert.deepEqual(
    guideNotices(
      none,
      fr,
      () => "?",
      (l) => l,
    ),
    [],
  );
});

test("the guide that cannot be written is a state: the prompt has no path, the guide's text is still there", async () => {
  const run = deps({ guide: "failed" });
  const result = await prepareGuideAsk(run.deps);
  assert.equal(result.guide.kind, "failed");
  assert.deepEqual(run.built, [{ root: ROOT, guideFile: null }]);
  assert.equal(result.prompt, "prompt fr -");
  assert.equal(result.text, "# Guide fr\n");
  const notices = guideNotices(
    result,
    fr,
    (error) => (error as { message: string }).message,
    (l) => `GUIDE-ASTRO-SYNC.${l}.md`,
  );
  assert.equal(notices.length, 1);
  assert.equal(notices[0].tone, "error");
  assert.equal(notices[0].replace, null);
  assert.equal(
    notices[0].text,
    "GUIDE-ASTRO-SYNC.fr.md n’a pas pu être écrit dans votre dossier de travail (disk full). Le prompt est copié sans chemin : collez ensuite le guide, avec « Copier le guide entier ».",
  );
  assert.equal(guideFileLine(result.guide, fr, "fr"), null);
});

test("a folder that cannot be made ready is a state too: the root comes from the settings, the guide is tried all the same", async () => {
  const run = deps({ prepareFails: true });
  const result = await prepareGuideAsk(run.deps);
  assert.equal(result.assistantFile.kind, "failed");
  assert.equal(result.root, ROOT);
  assert.equal(result.readme, null);
  assert.ok(result.prompt);
});

test("a prompt that cannot be built is none, not an exception", async () => {
  const result = await prepareGuideAsk(deps({ buildFails: true }).deps);
  assert.equal(result.prompt, null);
});

test("the guide's file line: its path with the home as ~, its size, what became of it", () => {
  const written = guideFileState(file("GUIDE-ASTRO-SYNC.fr.md", "created"));
  assert.equal(
    guideFileLine(written, fr, "fr")!,
    "Le guide : ~/Documents/Astro Sync/GUIDE-ASTRO-SYNC.fr.md · 5,8 ko · écrit à l’instant",
  );
  const current = guideFileState(file("GUIDE-ASTRO-SYNC.fr.md", "unchanged"));
  assert.match(guideFileLine(current, fr, "fr")!, /à jour$/u);
  const kept = guideFileState(
    file("GUIDE-ASTRO-SYNC.fr.md", "kept_as_changed"),
  );
  assert.match(
    guideFileLine(kept, fr, "fr")!,
    /votre version, gardée telle quelle$/u,
  );
});

test("« Copier le guide entier »: copied with the count of characters; refused with the way out", () => {
  const copied = afterGuideCopy(true, 5812, fr, "fr");
  assert.equal(copied.tone, "success");
  assert.equal(copied.offersOpen, false);
  assert.match(
    copied.text.replace(/[  ]/gu, " "),
    /^Guide copié \(5 812 caractères\)\. Collez-le dans la conversation si l’assistant ne peut pas lire le fichier\.$/u,
  );
  const refused = afterGuideCopy(false, 5812, fr, "fr");
  assert.equal(refused.tone, "warning");
  assert.equal(refused.offersOpen, true);
  assert.equal(
    refused.text,
    "Le presse-papiers a refusé la copie du guide : ouvrez le fichier et copiez-le vous-même.",
  );
  assert.equal(fr("guide.ask.openGuide"), "Ouvrir le guide");
});

test("« Remplacer par la version d'Astro Sync »: says the copy's name, or why it failed", async () => {
  const done = await replaceManagedFile(
    async () => ({
      keptCopy: `${ROOT}/GUIDE-ASTRO-SYNC.fr.modifie-2026-10-17.md`,
      file: file("GUIDE-ASTRO-SYNC.fr.md", "created"),
    }),
    "GUIDE-ASTRO-SYNC.fr.md",
    () => "?",
    fr,
  );
  assert.equal(done.tone, "success");
  assert.equal(
    done.text,
    "GUIDE-ASTRO-SYNC.fr.md est remplacé. Votre version est gardée sous le nom GUIDE-ASTRO-SYNC.fr.modifie-2026-10-17.md.",
  );
  const failed = await replaceManagedFile(
    async () => {
      throw new Error("x");
    },
    "README.md",
    () => "disque plein",
    fr,
  );
  assert.equal(failed.tone, "error");
  assert.equal(
    failed.text,
    "README.md n’a pas pu être remplacé (disque plein). Votre version n’a pas été touchée.",
  );
});

test("the language is named in the language of the window", () => {
  assert.equal(languageName("ja", "fr"), "japonais");
  assert.equal(languageName("fr", "fr"), "français");
  assert.equal(languageName("ja", "en"), "Japanese");
  assert.equal(languageName("ja", "ja"), "日本語");
});

test("every sentence of the window exists in the ten languages, with the same placeholders", () => {
  const keys = [
    "guide.ask.button",
    "guide.ask.lead",
    "guide.ask.title",
    "guide.ask.language",
    "guide.ask.languageDefault",
    "guide.ask.file",
    "guide.ask.stateWritten",
    "guide.ask.stateCurrent",
    "guide.ask.stateKept",
    "guide.ask.openGuide",
    "guide.ask.promptIn",
    "guide.ask.copyGuide",
    "guide.ask.fallback",
    "guide.ask.changed",
    "guide.ask.guideCopied",
    "guide.ask.guideRefused",
    "guide.ask.failed",
    "guide.ask.windowFailed",
    "guide.ask.kept",
    "guide.ask.replaced",
    "guide.ask.replaceFailed",
  ];
  const placeholders = (text: string) =>
    [...text.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();
  const en = createTranslator("en");
  for (const locale of SUPPORTED_LOCALES) {
    const t = createTranslator(locale);
    for (const key of keys) {
      const text = t(key);
      assert.notEqual(text, key, `${locale}: ${key} missing`);
      assert.deepEqual(
        placeholders(text),
        placeholders(en(key)),
        `${locale}: ${key}`,
      );
      if (locale !== "en") {
        assert.notEqual(text, en(key), `${locale}: ${key} is still English`);
      }
    }
  }
});

test("the French words of the guide's question", () => {
  assert.equal(fr("guide.ask.button"), "Poser une question à une IA");
  assert.equal(fr("guide.ask.copyGuide"), "Copier le guide entier");
  assert.equal(
    fr("guide.ask.promptIn", { language: "français" }),
    "Le prompt (en français)",
  );
  assert.equal(
    fr("guide.ask.fallback"),
    "Si votre assistant ne peut pas lire le fichier, il le dit et vous demande le guide : « Copier le guide entier », puis collez-le dans la conversation.",
  );
  assert.equal(
    fr("guide.ask.changed", { language: "japonais" }),
    "Le prompt a changé : copiez-le de nouveau. Le guide en japonais vient d’être écrit.",
  );
  assert.equal(
    fr("assistantFile.replace"),
    "Remplacer par la version d’Astro Sync",
  );
});
