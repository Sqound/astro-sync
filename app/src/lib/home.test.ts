import assert from "node:assert/strict";
import test from "node:test";

import {
  displayPath,
  homeOf,
  homeState,
  HOW_STEPS,
  RESUME_LIMIT,
} from "./home.ts";
import { createTranslator } from "../i18n/locale.ts";

const plain = (text: string) => text.replaceAll("\u00a0", " ");
const same = (actual: string, expected: string) =>
  assert.equal(plain(actual), plain(expected));
const row = (n: number) => ({
  sessionId: `night-${n}`,
  title: `Nuit ${n}`,
  line: "À compléter : …",
  step: "fichiers" as const,
});

test("signed in: « Commencer » is the one primary and opens « Sessions », how it works is unfolded", () => {
  const state = homeState({ connected: true, toResume: [], sentOnce: false });
  assert.equal(state.startEnabled, true);
  assert.equal(state.startReasonKey, null);
  assert.equal(state.howFolded, false);
  assert.deepEqual(state.primaries, { signIn: 0, how: 1 });
});

test("signed out: « Commencer » greyed with its reason, the sign-in card has its own primary, no night to resume", () => {
  const state = homeState({
    connected: false,
    toResume: [row(5)],
    sentOnce: true,
  });
  assert.equal(state.startEnabled, false);
  same(
    createTranslator("fr")(state.startReasonKey!),
    "Connectez-vous d’abord : les sessions viennent de votre espace.",
  );
  assert.deepEqual(state.resume, []);
  assert.equal(state.howFolded, false);
  assert.deepEqual(state.primaries, { signIn: 1, how: 1 });
});

test("a night to resume: its row leads, « Comment ça marche » is folded after a first sending", () => {
  const rows = [1, 2, 3, 4].map(row);
  const state = homeState({ connected: true, toResume: rows, sentOnce: true });
  assert.equal(state.resume.length, RESUME_LIMIT);
  assert.equal(state.resume[0].step, "fichiers");
  assert.equal(state.howFolded, true);
  assert.equal(state.startEnabled, true);
});

test("how it works says its four steps of one sentence, word for word", () => {
  const fr = createTranslator("fr");
  assert.deepEqual(
    HOW_STEPS.map((step) => fr(step.titleKey)),
    [
      "Choisissez une session",
      "Rangez vos fichiers",
      "Vérifiez et choisissez",
      "Envoyez",
    ],
  );
  same(
    fr("home.how.step4.line"),
    "Seuls des résultats partent, après la vérification de leur taille ; les FITS et les journaux restent ici.",
  );
  same(
    fr("home.how.folder", {
      path: displayPath(
        "/home/camille/Documents/Astro Sync",
        homeOf("/home/camille/Documents/Astro Sync/"),
      ),
    }),
    "Tout se passe dans votre dossier de travail : ~/Documents/Astro Sync.",
  );
  assert.equal(
    displayPath("D:\\Astro Sync", homeOf("D:\\Astro Sync")),
    "D:\\Astro Sync",
  );
});
