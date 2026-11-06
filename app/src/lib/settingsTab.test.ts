import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import { createTranslator } from "../i18n/locale.ts";
import {
  OFFSET_FIELDS,
  dependenciesHead,
  dependencyRows,
  offsetDraft,
  offsetsSavable,
  pipelineSectionViews,
  readOffset,
  sectionsToInstall,
} from "./settingsTab.ts";
import type { PipelineSections, SectionDependency } from "./bridge";
import { dependencyTools } from "./homeFixtures.ts";

const fr = createTranslator("fr");
const plain = (text: string) => text.replaceAll(" ", " ").replaceAll("'", "’");
const same = (actual: string, expected: string) =>
  assert.equal(plain(actual), plain(expected));

test("Réglages: two tabs, « Général » opens on the working folder, today's settings follow", async () => {
  same(fr("settings.tab.general"), "Général");
  same(fr("settings.tab.dependencies"), "Dépendances");
  same(fr("settings.folder.title"), "Dossier de travail");
  same(fr("settings.folder.change"), "Changer de dossier");
  same(
    fr("settings.folder.line"),
    "Astro Sync y crée un dossier par nuit ; il ne touche jamais à vos propres fichiers. Ses fichiers de traitement se nettoient plus bas.",
  );
  same(fr("wizard.folder.openFinder"), "Ouvrir dans le Finder");
  // The screen draws them in their fixed order: the working folder first,
  // then the language, the computer's name, « Détacher cet ordinateur »
  // last, in its own zone.
  const screen = await readFile(
    new URL("../screens/SettingsScreen.tsx", import.meta.url),
    "utf8",
  );
  const order = [
    't("settings.folder.title")',
    't("settings.languageLabel")',
    "<DeviceName",
    't("settings.utc.title")',
    "<DetachDevice",
  ].map((needle) => screen.indexOf(needle));
  assert.ok(order.every((index) => index > 0));
  assert.deepEqual(
    [...order].sort((a, b) => a - b),
    order,
  );
});

test("Réglages: « Changer de dossier » confirms with a plain primary, « Annuler » a link, nothing moved", async () => {
  same(fr("settings.folder.confirmTitle"), "Utiliser ce dossier de travail ?");
  same(
    fr("settings.folder.confirmBody", { old: "~/Documents/Astro Sync" }),
    "Astro Sync cherchera vos nuits dans ce dossier et y créera les prochaines. Rien n’est déplacé : les nuits déjà créées restent dans ~/Documents/Astro Sync ; déplacez-les vous-même si vous le voulez.",
  );
  same(fr("settings.folder.use"), "Utiliser ce dossier");
  const screen = await readFile(
    new URL("../screens/SettingsScreen.tsx", import.meta.url),
    "utf8",
  );
  const dialog = screen.slice(screen.indexOf("working-folder-title"));
  // Rule 4: a confirmation that keeps everything is a plain primary.
  assert.ok(dialog.includes('buttonClassName("primary")'));
  assert.ok(!dialog.includes("destructive"));
  assert.ok(dialog.includes('buttonClassName("link")'));
});

test("Général: the three clocks of the logs (NINA, PHD2, ASIAIR), empty is UTC, a bad offset is never saved", () => {
  assert.deepEqual(
    OFFSET_FIELDS.map((field) => plain(fr(field.labelKey))),
    ["Journaux NINA", "Journaux PHD2", "Journaux ASIAIR"],
  );
  assert.deepEqual(readOffset(""), { ok: true, value: null });
  assert.deepEqual(readOffset("UTC"), { ok: true, value: null });
  assert.deepEqual(readOffset("+2"), { ok: true, value: "+02:00" });
  assert.deepEqual(readOffset("-05:30"), { ok: true, value: "-05:30" });
  assert.deepEqual(readOffset("+0930"), { ok: true, value: "+09:30" });
  assert.deepEqual(readOffset("2 h"), { ok: false });
  assert.deepEqual(readOffset("+15:00"), { ok: false });
  const saved = {
    workingFolder: null,
    ninaUtcOffset: "+02:00",
    phd2UtcOffset: null,
    asiairUtcOffset: null,
  };
  const draft = offsetDraft(saved);
  assert.equal(offsetsSavable(saved, draft), false, "nothing changed");
  assert.equal(offsetsSavable(saved, { ...draft, phd2UtcOffset: "+2" }), true);
  assert.equal(offsetsSavable(saved, { ...draft, phd2UtcOffset: "x" }), false);
  assert.equal(
    offsetsSavable(saved, { ...draft, ninaUtcOffset: "+2" }),
    false,
    "+2 is +02:00",
  );
});

test("Dépendances: every tool present, no primary; Tycho-2, and the stacking pipeline with its Python environment ready", () => {
  const tools = dependencyTools();
  const rows = dependencyRows(fr, "fr", tools, {});
  assert.deepEqual(
    rows.map((row) => plain(row.name)),
    [
      "Siril",
      "ASTAP et ses index",
      "astrometry.net",
      "Liste d’étoiles Tycho-2",
      "Pipeline d’empilement",
    ],
    "StarNet2 and the Python environment are the section « Pipelines de traitement »",
  );
  const [siril, astap, astrometry, tycho, stacking] = rows;
  same(
    siril.role,
    "Les images de progression (empilement intégré, réglages fixes).",
  );
  same(siril.lines[0], "/Applications/Siril.app — compatible");
  same(siril.badge.text, "Présent · 1.4.0");
  assert.equal(siril.action, null);
  same(astap.role, "Les solutions de ciel des captures (recommandé).");
  same(astap.badge.text, "Présent · 2026.09");
  assert.equal(astrometry.optional, true);
  // No file of the registry for it: « À installer » and its page.
  same(astrometry.badge.text, "À installer");
  assert.equal(astrometry.action, null);
  same(
    astrometry.role,
    "Résolution de secours des solutions de ciel, facultatif.",
  );
  assert.equal(tycho.optional, true);
  assert.equal(tycho.action, "install");
  same(stacking.badge.text, "Présent · 3.13.7");
  same(
    stacking.lines[0],
    "Environnement Python 3.13.7 et Siril prêts, dans les données d’Astro Sync.",
  );
  assert.equal(stacking.action, null);
  assert.equal(stacking.optional, false);
  assert.equal(stacking.page, null, "nothing to fetch by hand");
  const head = dependenciesHead(fr, "fr", rows, tools);
  same(head.line, "Tous les outils nécessaires sont présents.");
  assert.equal(head.installAll, false, "nothing to install, no primary");
});

test("Dépendances: a required tool missing, « Tout installer » the page's primary, « Installer » per row", () => {
  const tools = dependencyTools({ astap: "missing" });
  const rows = dependencyRows(fr, "fr", tools, {});
  const astap = rows.find((row) => row.id === "astap")!;
  same(astap.lines[0], "Programme et base d’étoiles : à télécharger, 410 Mo.");
  same(astap.badge.text, "Absent");
  assert.equal(astap.action, "install");
  const head = dependenciesHead(fr, "fr", rows, tools);
  same(head.line, "1 outil nécessaire manque.");
  assert.equal(head.installAll, true);
  same(fr("deps.installAll"), "Tout installer");
  same(fr("deps.page"), "Page de téléchargement");
});

test("Dépendances: no file for this system is « À installer » with the page, never « Échec »; a real failure stays « Échec »", () => {
  const tools = dependencyTools({ astap: "missing", noDownload: true });
  const row = (downloads = {}) =>
    dependencyRows(fr, "fr", tools, downloads).find(
      (item) => item.id === "astap",
    )!;
  const manual = row();
  same(manual.badge.text, "À installer");
  assert.equal(manual.badge.tone, "neutral");
  assert.equal(manual.action, null, "nothing Astro Sync could fetch");
  assert.equal(manual.page, "https://www.hnsky.org/astap.htm");
  same(
    manual.lines[0],
    "Pas de téléchargement automatique pour ce système : passez par la page de téléchargement.",
  );
  // The answer « no automatic download » never turns the badge red.
  const answered = row({
    astap: { state: "failed", failure: { code: "no_automatic_download" } },
  });
  same(answered.badge.text, "À installer");
  // The real failures keep their red badge:
  for (const failure of [
    { code: "digest_mismatch" },
    { code: "extraction", detail: "bad zip" },
    { code: "io", detail: "disk full" },
    { code: "interrupted", keptBytes: 1, total: 2, detail: "reset" },
  ] as const) {
    const failed = dependencyRows(
      fr,
      "fr",
      dependencyTools({ astap: "missing" }),
      { astap: { state: "failed", failure } },
    ).find((item) => item.id === "astap")!;
    same(failed.badge.text, "Échec");
    assert.equal(failed.badge.tone, "error");
  }
  const unpack = dependencyRows(
    fr,
    "fr",
    dependencyTools({ astap: "missing" }),
    {
      astap: {
        state: "failed",
        failure: { code: "extraction", detail: "bad zip" },
      },
    },
  ).find((item) => item.id === "astap")!;
  assert.equal(unpack.action, "install");
  // Fetchable and missing: « Absent », « Installer », both sizes added up.
  const fetchable = dependencyRows(
    fr,
    "fr",
    dependencyTools({ astap: "missing" }),
    {},
  ).find((item) => item.id === "astap")!;
  same(fetchable.badge.text, "Absent");
  assert.equal(fetchable.action, "install");
  same(
    fetchable.lines[0],
    "Programme et base d’étoiles : à télécharger, 410 Mo.",
  );
  // Every other tool without a file (astrometry.net here) is « À installer » too.
  const other = dependencyRows(fr, "fr", tools, {}).find(
    (item) => item.id === "astrometry-net",
  )!;
  same(other.badge.text, "À installer");
  assert.equal(other.action, null);
});

test("Dépendances: a download running, its progress in words and « Annuler le téléchargement »", () => {
  const tools = dependencyTools({ astap: "missing" });
  const rows = dependencyRows(fr, "fr", tools, {
    astap: { state: "downloading", bytes: 184_000_000, total: 410_000_000 },
  });
  const astap = rows.find((row) => row.id === "astap")!;
  same(
    astap.lines[0],
    "184 Mo sur 410 Mo · vérification de l’empreinte à la fin",
  );
  same(astap.badge.text, "Téléchargement");
  assert.equal(astap.action, "cancel");
  const head = dependenciesHead(fr, "fr", rows, tools);
  same(head.line, "Téléchargement de ASTAP en cours.");
  assert.equal(head.downloading, true);
});

test("Dépendances: a failure in words, what is kept, « Reprendre le téléchargement »", () => {
  const tools = dependencyTools({ astap: "missing" });
  const rows = dependencyRows(fr, "fr", tools, {
    astap: {
      state: "failed",
      failure: {
        code: "interrupted",
        keptBytes: 184_000_000,
        total: 410_000_000,
        detail: "reset",
      },
    },
  });
  const astap = rows.find((row) => row.id === "astap")!;
  assert.deepEqual(astap.lines.map(plain), [
    "La connexion s’est interrompue à 184 Mo sur 410 Mo.",
    "Ce qui est déjà téléchargé est gardé.",
  ]);
  same(astap.badge.text, "Échec");
  assert.equal(astap.action, "resume");
  // A part kept from an earlier run says « Reprendre » too.
  const partial = dependencyRows(
    fr,
    "fr",
    dependencyTools({ astap: "missing", partial: 1 }),
    {},
  );
  assert.equal(partial.find((row) => row.id === "astap")!.action, "resume");
  same(
    dependenciesHead(fr, "fr", rows, tools).line,
    "1 outil nécessaire manque.",
  );
});

test("« Pipeline d’empilement »: to install, no Python, unfinished, installing, failed and resumed, waiting for Siril", () => {
  const row = (
    environment: "missing" | "interrupted" | "noPython",
    downloads = {},
  ) =>
    dependencyRows(fr, "fr", dependencyTools({ environment }), downloads).find(
      (item) => item.id === "stacking-pipeline",
    )!;

  const missing = row("missing");
  same(missing.badge.text, "Absent");
  assert.equal(missing.action, "install");
  same(
    missing.lines[0],
    "Environnement Python à créer dans les données d’Astro Sync : environ 64 Mo à télécharger, chaque paquet vérifié par son empreinte.",
  );
  assert.equal(missing.page, null);

  const noPython = row("noPython");
  assert.equal(noPython.action, null, "nothing Astro Sync can install");
  same(
    noPython.lines[0],
    "Aucun Python 3.12 à 3.14 sur cet ordinateur : installez-le depuis sa page, puis « Vérifier à nouveau ».",
  );
  assert.equal(noPython.page, "https://www.python.org/downloads/");

  const unfinished = row("interrupted");
  assert.equal(unfinished.action, "resumeInstall");
  same(fr("deps.resumeInstall"), "Reprendre l’installation");
  assert.deepEqual(unfinished.lines.map(plain), [
    "L’installation s’est arrêtée avant la fin.",
    "Ce qui est déjà installé est gardé.",
  ]);

  const tools = dependencyTools({ environment: "missing" });
  const installing = dependencyRows(fr, "fr", tools, {
    "stacking-pipeline": {
      state: "installing",
      progress: {
        step: "packages",
        index: 3,
        total: 4,
        packagesDone: 5,
        packagesTotal: 14,
      },
    },
  });
  const running = installing.find((item) => item.id === "stacking-pipeline")!;
  same(running.lines[0], "Paquets : 5 sur 14 · vérifiés par leur empreinte");
  same(running.badge.text, "Installation");
  assert.equal(running.action, "cancelInstall");
  assert.deepEqual(running.progress, { bytes: 33, total: 56 });
  const head = dependenciesHead(fr, "fr", installing, tools, ["python-env"]);
  same(head.line, "Installation de Pipeline d’empilement en cours.");
  assert.equal(head.downloading, true);

  const failed = row("missing", {
    "stacking-pipeline": {
      state: "envFailed",
      failure: { code: "packages", detail: "reset" },
    },
  });
  same(failed.badge.text, "Échec");
  assert.equal(failed.action, "resumeInstall");
  assert.deepEqual(failed.lines.map(plain), [
    "Le téléchargement des paquets s’est interrompu.",
    "Ce qui est déjà installé est gardé.",
  ]);

  // The environment made, Siril missing: the row waits for Siril.
  const noSiril = dependencyTools().map((item) =>
    item.id === "stacking-pipeline"
      ? { ...item, state: "missing" as const, blockedBy: ["siril"] }
      : item,
  );
  const waiting = dependencyRows(fr, "fr", noSiril, {}).find(
    (item) => item.id === "stacking-pipeline",
  )!;
  same(
    waiting.lines[0],
    "Environnement Python prêt ; Siril doit encore être installé.",
  );
  assert.equal(waiting.action, null);

  // « Tout installer » also makes what the pipelines need.
  const rows = dependencyRows(fr, "fr", tools, {});
  assert.equal(dependenciesHead(fr, "fr", rows, tools).installAll, false);
  const withPipelines = dependenciesHead(fr, "fr", rows, tools, ["python-env"]);
  assert.equal(withPipelines.installAll, true);
  same(
    withPipelines.line,
    "L’environnement Python des pipelines est à installer.",
  );
});

function dependency(
  id: string,
  extra: Partial<SectionDependency> = {},
): SectionDependency {
  return {
    id,
    name: id === "python-env" ? "Environnement Python" : id,
    reason: `Pourquoi ${id}.`,
    required: true,
    state: "present",
    version: null,
    minVersion: null,
    licence: "MIT",
    page: "https://exemple.test",
    bytes: null,
    installable: false,
    interrupted: false,
    blockedBy: [],
    ...extra,
  };
}

test("« Pipelines de traitement »: one block per pipeline, its dependencies, the trust in a script the person added", () => {
  const answer: PipelineSections = {
    sections: [
      {
        id: "raw",
        name: "Rendu naturel",
        description: "Les couleurs du capteur, sans retouche de teinte.",
        source: "shipped",
        author: "Astro Sync",
        licence: "MIT",
        trust: "not_needed",
        scriptSha256: null,
        dependencies: [
          dependency("siril", { version: "1.4.4", minVersion: "1.4" }),
          dependency("starnet2", {
            required: false,
            state: "missing",
            licence: "Freeware",
          }),
          dependency("python-env", {
            state: "missing",
            installable: true,
            interrupted: true,
          }),
        ],
      },
      {
        id: "mono-ha",
        name: "Hα en tons chauds",
        description: "Tons chauds.",
        source: "user",
        author: "Camille",
        licence: "MIT",
        trust: "ask",
        scriptSha256: "ab".repeat(32),
        dependencies: [
          dependency("python-env", { state: "missing", installable: true }),
        ],
      },
    ],
    ignored: [{ folder: "with-url", source: "user", reason: "a URL" }],
  };
  const tools = dependencyTools({ environment: "interrupted" });
  const views = pipelineSectionViews(fr, "fr", answer, tools, {});
  const [raw, mine] = views;
  same(raw.origin, "Livré avec Astro Sync");
  assert.equal(raw.trust, null, "a shipped pipeline asks nothing");
  const [siril, starnet, python] = raw.dependencies;
  same(siril.reason, "Pourquoi siril.");
  same(siril.badge.text, "Présent · 1.4.4");
  assert.deepEqual(siril.lines.map(plain), [
    "Version 1.4.4 · 1.4 au moins.",
    "Licence : MIT",
  ]);
  assert.equal(starnet.optional, true);
  same(starnet.badge.text, "Non installé");
  assert.equal(starnet.action, null, "no download in the registry: its page");
  assert.equal(python.action, "resumeInstall");
  assert.equal(python.page, null, "made here, not fetched by hand");
  same(python.lines[0], "L’installation s’est arrêtée avant la fin.");

  same(mine.origin, "Ajouté par vous · Camille · MIT");
  same(mine.trust!.badge.text, "À approuver");
  same(mine.trust!.line, "Empreinte SHA-256 : abab…abab");
  assert.equal(mine.trust!.action, "decide");
  same(fr("deps.trust.decide"), "Décider pour ce script");
  // Answered on this screen: trusted, then refused.
  const trusted = pipelineSectionViews(
    fr,
    "fr",
    answer,
    tools,
    {},
    {
      "mono-ha": "trusted",
    },
  )[1];
  same(trusted.trust!.badge.text, "Approuvé");
  assert.equal(trusted.trust!.action, "revoke");
  const refused = pipelineSectionViews(
    fr,
    "fr",
    answer,
    tools,
    {},
    {
      "mono-ha": "refused",
    },
  )[1];
  same(refused.trust!.badge.text, "Refusé · désactivé");
  const changed = pipelineSectionViews(
    fr,
    "fr",
    answer,
    tools,
    {},
    {
      "mono-ha": "changed",
    },
  )[1];
  same(changed.trust!.badge.text, "Modifié depuis votre accord");

  // « Tout installer »: the environment once, the person's pipeline
  // included; StarNet2 (optional) and Siril (present) are not in it.
  assert.deepEqual(sectionsToInstall(answer), ["python-env"]);
  same(fr("deps.pipelines.title"), "Pipelines de traitement");
  same(
    fr("deps.pipelines.ignored", { folder: "with-url", reason: "a URL" }),
    "Ignoré : with-url — a URL",
  );
});

test("the trust dialog: the question word for word, one primary, the refusal recorded, « Plus tard » a link", async () => {
  same(
    fr("trust.question"),
    "Vous avez ajouté ce pipeline vous-même ; Astro Sync ne l’a pas vérifié. Lui faites-vous confiance ?",
  );
  same(fr("trust.accept"), "Lui faire confiance");
  same(fr("trust.refuse"), "Ne pas lui faire confiance");
  const source = await readFile(
    new URL("../components/TrustDialog.tsx", import.meta.url),
    "utf8",
  );
  const levels = [...source.matchAll(/buttonClassName\("(\w+)"\)/gu)].map(
    (match) => match[1],
  );
  assert.deepEqual(levels, ["link", "tertiary", "primary"], "rules 1, 2, 6, 9");
  assert.match(source, /role="dialog"/u);
  // Both screens ask it: « Dépendances » and the step « Traitements » (in its window
  // « Pipelines pour {cible} »).
  for (const screen of [
    "../screens/DependenciesTab.tsx",
    "../screens/night/PipelineDialog.tsx",
  ]) {
    const text = await readFile(new URL(screen, import.meta.url), "utf8");
    assert.match(text, /<TrustDialog/u, screen);
    assert.match(text, /pipelineTrustSet\(/u, screen);
  }
});
