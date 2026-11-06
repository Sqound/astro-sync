import assert from "node:assert/strict";
import test from "node:test";

import {
  EVERY_TARGET,
  NO_PROCESSING_ID,
  applyToAll,
  asksTrust,
  defaultRenders,
  loadTargetRenders,
  pipelineDialogView,
  rendersOf,
  saveTargetRenders,
  toggleChoice,
  type DialogTarget,
} from "./pipelineChoice.ts";
import type { Pipeline, ToolRow } from "./bridge";
import { IC, M31, NGC, PIPELINES, tools } from "./homeFixtures.ts";
import { createTranslator } from "../i18n/locale.ts";

const fr = createTranslator("fr");
/** The texts as the window shows them (no-break spaces kept). */
const nb = (text: string | null | undefined) => text ?? "";

function memory(initial: Record<string, string> = {}) {
  const items = new Map(Object.entries(initial));
  return {
    getItem: (key: string) => items.get(key) ?? null,
    setItem: (key: string, value: string) => void items.set(key, value),
    items,
  };
}

test("the pipelines are chosen target by target -- defaults by band, kept by night, a choice made for every target read silently as each one's", () => {
  // Natural render; and the dual-band correction on a dual-band target.
  assert.deepEqual(defaultRenders("broadband", PIPELINES), ["raw"]);
  assert.deepEqual(defaultRenders("dualband", PIPELINES), [
    "raw",
    "dualband-correction",
  ]);
  assert.deepEqual(defaultRenders("dualband", PIPELINES, false), [
    "dualband-correction",
  ]);
  // A target's own choice, else the one made for every target, else its
  // default; what is no longer listed or refused is dropped.
  const chosen = {
    [IC]: ["hoo", "gone", "linear"],
    [EVERY_TARGET]: ["polished"],
  };
  assert.deepEqual(rendersOf(chosen, IC, "broadband", PIPELINES), [
    "hoo",
    "linear",
  ]);
  assert.deepEqual(rendersOf(chosen, M31, "broadband", PIPELINES), [
    "polished",
  ]);
  assert.deepEqual(rendersOf({}, NGC, "dualband", PIPELINES), [
    "raw",
    "dualband-correction",
  ]);
  const refused = PIPELINES.map((p) =>
    p.id === "hoo" ? { ...p, trust: "refused" as const } : p,
  );
  assert.deepEqual(rendersOf({ [IC]: ["hoo"] }, IC, "broadband", refused), [
    "raw",
  ]);
  assert.deepEqual(
    rendersOf({ [IC]: ["hoo", "raw"] }, IC, "broadband", PIPELINES, false),
    ["hoo"],
    "one image per capture: the first",
  );

  // Kept on this computer, by night; the former list of the window
  // (one choice for every target) is read as every target's.
  const storage = memory({
    "astro-sync.pipelines:night-old": JSON.stringify(["raw", "linear"]),
  });
  assert.deepEqual(loadTargetRenders(storage, "night-old"), {
    [EVERY_TARGET]: ["raw", "linear"],
  });
  assert.deepEqual(
    rendersOf(
      loadTargetRenders(storage, "night-old"),
      NGC,
      "dualband",
      PIPELINES,
    ),
    ["raw", "linear"],
  );
  saveTargetRenders(storage, "night-7", { [NGC]: ["hoo"], [IC]: ["raw"] });
  assert.deepEqual(loadTargetRenders(storage, "night-7"), {
    [NGC]: ["hoo"],
    [IC]: ["raw"],
  });
  assert.deepEqual(loadTargetRenders(storage, "night-none"), {});
  assert.deepEqual(
    loadTargetRenders(memory({ "astro-sync.pipelines:x": "{" }), "x"),
    {},
    "a choice that cannot be read: the defaults",
  );
  assert.deepEqual(loadTargetRenders(null, "night-7"), {});
});

function dialog(options: {
  target?: Partial<DialogTarget>;
  selection?: string[];
  several?: boolean;
  engine?: "graxpert-ai" | "siril";
  tools?: ToolRow[];
  progress?: Record<string, number | null>;
  pipelines?: Pipeline[];
}) {
  return pipelineDialogView(fr, "fr", {
    target: {
      targetId: IC,
      designation: "IC 1396",
      frames: 38,
      duration: "1 h 16",
      filter: "UV/IR-cut",
      band: "broadband",
      ...options.target,
    },
    pipelines: options.pipelines ?? PIPELINES,
    tools: options.tools ?? tools({}),
    selection: options.selection ?? ["raw"],
    several: options.several ?? true,
    engine: options.engine ?? "graxpert-ai",
    progress: options.progress ?? {},
  });
}

test("window « Pipelines pour {cible} »: recommended or discouraged by the target's filter, never refused; tools with their download and size; « Sans traitement » last and combinable; « Valider » waits for one pipeline", () => {
  const broad = dialog({ selection: ["raw", "hoo"] });
  assert.equal(broad.title, "Pipelines pour IC 1396");
  assert.equal(
    broad.subtitle,
    "38 poses · 1 h 16 · filtre UV/IR-cut · une cible large bande",
  );
  assert.deepEqual(
    broad.cards.map((c) => [c.name, c.filterLabel, c.recommended, c.ticked]),
    [
      ["Rendu naturel", "Tous filtres", true, true],
      ["Correction bibande", "Bibande", false, false],
      ["Palette HOO", "Bibande", false, true],
      ["Rendu soigné", "Tous filtres", true, false],
      ["Sans traitement", "Tous filtres", false, false],
    ],
  );
  assert.equal(
    nb(broad.cards[1].discouraged),
    "Déconseillé pour ce filtre : elle ramène les teintes d’un filtre bibande, et les poses de IC 1396 sont en UV/IR-cut.",
  );
  assert.equal(
    nb(broad.cards[2].discouraged),
    "Déconseillé pour ce filtre : la palette HOO sépare Hα et OIII, que seul un filtre bibande laisse passer ; avec UV/IR-cut, les couleurs seraient fausses.",
  );
  assert.ok(
    broad.cards.every((c) => !c.disabled),
    "never refused",
  );
  assert.equal(broad.count, "2 pipelines choisis");
  assert.ok(broad.valid);

  // On a dual-band target only the dual-band pipelines are recommended.
  const dual = dialog({
    target: { designation: "NGC 7000", filter: "L-eXtreme", band: "dualband" },
  });
  assert.deepEqual(
    dual.cards.map((c) => [c.id, c.recommended, c.discouraged]),
    [
      ["raw", false, null],
      ["dualband-correction", true, null],
      ["hoo", true, null],
      ["polished", false, null],
      [NO_PROCESSING_ID, false, null],
    ],
  );
  assert.match(dual.subtitle, /une cible bibande$/u);

  // « Sans traitement »: the last card, the linear master, combinable.
  const last = broad.cards[broad.cards.length - 1];
  assert.equal(
    nb(last.description),
    "Le master linéaire seul, TIFF : l’empilement calibré et aligné, sans étirement. Pour traiter vous-même.",
  );
  assert.equal(nb(last.produces), "Produit : TIFF 16 bits linéaire");
  assert.deepEqual(toggleChoice(["raw"], NO_PROCESSING_ID, PIPELINES, true), [
    "raw",
    NO_PROCESSING_ID,
  ]);
  // Several: a tick each, down to none (« Valider » then says why); one
  // image per capture: a round button each.
  assert.deepEqual(toggleChoice(["raw"], "raw", PIPELINES, true), []);
  assert.deepEqual(toggleChoice(["raw"], "hoo", PIPELINES, false), ["hoo"]);
  const none = dialog({ selection: [] });
  assert.equal(none.valid, false);
  assert.equal(
    nb(none.count),
    "Cochez au moins un pipeline : le pipeline fait l’empilement, et le service n’accepte pas de FITS.",
  );
  assert.match(
    nb(dialog({ several: false }).subtitle),
    /votre espace ne garde qu’une image par capture : choisissez un seul pipeline\.$/u,
  );

  // The tools: present, optional, or missing with their download and size.
  const polished = broad.cards[3];
  assert.deepEqual(
    polished.tools.map((p) => [
      p.name,
      p.state,
      p.stateLabel,
      p.action?.label ?? null,
    ]),
    [
      ["Siril", "present", null, null],
      ["StarNet2", "missing", "Non installé", "Télécharger (127,9 Mo)"],
      ["GraXpert", "missing", "Non installé", "Télécharger (130,1 Mo)"],
      ["Python", "missing", "Absent", "Installer (81,8 Mo)"],
    ],
  );
  assert.equal(nb(polished.produces), "Produit\u00a0: TIFF · PNG");
  // The engine of the « Rendu soigné » and its cost; GraXpert missing: the
  // render is made all the same, with Siril, and the card says it.
  assert.equal(
    nb(polished.engine),
    "Moteur : GraXpert (IA) · environ 7 min de plus par master",
  );
  assert.match(
    nb(polished.fallback),
    /^GraXpert n’est pas installé : le rendu se fait quand même, avec Siril/u,
  );
  const siril = dialog({ engine: "siril" }).cards[3];
  assert.equal(nb(siril.engine), "Moteur : Siril (plus rapide, sans modèle)");
  assert.equal(siril.fallback, null);
  const downloading = dialog({ progress: { graxpert: 48 } }).cards[3];
  assert.deepEqual(
    [
      downloading.tools[2].state,
      downloading.tools[2].percent,
      downloading.tools[2].action,
    ],
    ["downloading", 48, null],
  );
  assert.match(nb(downloading.fallback), /^GraXpert se télécharge/u);
  assert.equal(broad.cards[0].engine, null);

  // « Appliquer à toutes les cibles »: the selection on every ticked
  // target, a discouraged one included (it stays said on its card).
  assert.deepEqual(
    applyToAll({ [M31]: ["polished"] }, [IC, NGC], ["hoo", "linear"]),
    { [M31]: ["polished"], [IC]: ["hoo", "linear"], [NGC]: ["hoo", "linear"] },
  );

  // A script the person added asks first; a refused one is listed, never
  // ticked.
  const mine = (trust: Pipeline["trust"]): Pipeline => ({
    id: "mono-ha",
    name: { fr: "Hα en tons chauds" },
    description: {},
    displayName: "Hα en tons chauds",
    displayDescription: null,
    scriptSha256: "ab".repeat(32),
    source: "user",
    trust,
    filter: "h-alpha",
  });
  assert.equal(asksTrust([...PIPELINES, mine("ask")], "mono-ha", []), true);
  assert.equal(
    asksTrust([...PIPELINES, mine("trusted")], "mono-ha", []),
    false,
  );
  assert.equal(asksTrust(PIPELINES, NO_PROCESSING_ID, []), false);
  const refused = dialog({
    pipelines: [...PIPELINES, mine("refused")],
    selection: ["mono-ha", "raw"],
  }).cards[4];
  assert.deepEqual(
    [refused.added, refused.filterLabel, refused.disabled, refused.ticked],
    [true, "h-alpha", true, false],
  );
  assert.deepEqual(
    toggleChoice(["raw"], "mono-ha", [...PIPELINES, mine("refused")], true),
    ["raw"],
  );
});
