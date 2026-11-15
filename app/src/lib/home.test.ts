import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  connectionCard,
  displayPath,
  pathParts,
  homeOf,
  homeState,
  HOW_STEPS,
  RESUME_LIMIT,
  rememberServiceAnswered,
  serviceAnsweredThisLaunch,
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

test("« Connecté » only once the service answered since the launch, never on the faith of the token kept here (the smoke's deleted space said « Connected » for 50 s)", async () => {
  assert.equal(connectionCard({ paired: false, answered: null }), "signedOut");
  assert.equal(connectionCard({ paired: true, answered: null }), "checking");
  assert.equal(connectionCard({ paired: true, answered: true }), "connected");
  assert.equal(connectionCard({ paired: true, answered: false }), "unchecked");
  // Once answered, a home mounted again says it at once.
  assert.equal(serviceAnsweredThisLaunch(), false);
  rememberServiceAnswered(false);
  assert.equal(serviceAnsweredThisLaunch(), false);
  rememberServiceAnswered(true);
  assert.equal(serviceAnsweredThisLaunch(), true);
  // The screen draws the card from the check, and « Connecté » from it only.
  const { readFile } = await import("node:fs/promises");
  const screen = await readFile(
    new URL("../screens/HomeScreen.tsx", import.meta.url),
    "utf8",
  );
  assert.match(screen, /spaceCheck\(\)/);
  assert.match(screen, /connectionCard\(\{ paired, answered \}\)/);
  assert.match(
    screen,
    /card === "connected" \? \(\s*<>\s*<Icon name="check"[^]*?"home\.connection\.connected"/,
  );
  const fr = createTranslator("fr");
  assert.equal(
    fr("home.connection.checking"),
    "Vérification de la connexion avec le service…",
  );
});

test("a long path breaks only after a separator, never inside a folder's name (« Astro Sync » stays whole)", () => {
  const parts = pathParts("/mnt/disk/archive/Documents/Astro Sync");
  assert.deepEqual(parts, [
    "/",
    "mnt/",
    "disk/",
    "archive/",
    "Documents/",
    "Astro Sync",
  ]);
  assert.equal(parts.join(""), "/mnt/disk/archive/Documents/Astro Sync");
  assert.deepEqual(pathParts("~/Documents/Astro Sync"), [
    "~/",
    "Documents/",
    "Astro Sync",
  ]);
  assert.deepEqual(pathParts("C:\\Users\\a b\\Astro Sync"), [
    "C:\\",
    "Users\\",
    "a b\\",
    "Astro Sync",
  ]);
  assert.deepEqual(pathParts(""), []);
  // The working folder of « Réglages » is drawn that way, each part unbroken.
  const screen = readFileSync(
    new URL("../screens/SettingsScreen.tsx", import.meta.url),
    "utf8",
  );
  assert.match(screen, /pathParts\(shown\)\.map/);
  const css = readFileSync(
    new URL("../theme/primitives.css", import.meta.url),
    "utf8",
  );
  assert.match(css, /\.as-folder-row__part \{[^}]*display: inline-block;/);
});
