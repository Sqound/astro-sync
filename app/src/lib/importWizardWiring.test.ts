import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import test from "node:test";

// What the pure tests of `importFlow.ts` cannot see: that both wizards draw
// the same frame, that the import is owned above the screens, and that the
// menu and « Sessions » are wired to it. The captures (`detect-wizard-*`) show
// the same in a real window.

const source = (relative: string) =>
  readFileSync(new URL(`../${relative}`, import.meta.url), "utf8");

test("the night's wizard and the import render the same frame component, and neither draws its own", () => {
  for (const file of [
    "screens/night/NightWizard.tsx",
    "screens/detect/ImportScreen.tsx",
  ]) {
    const text = source(file);
    assert.match(
      text,
      /import \{ WizardFrame \} from "\.\.\/\.\.\/components\/WizardFrame";/,
      file,
    );
    assert.match(text, /<WizardFrame\b/, file);
    assert.doesNotMatch(text, /<PageTop>/, file);
    assert.doesNotMatch(text, /as-wizard__frame/, file);
    // The thread's done steps open again (`go`), the page goes to its top.
    assert.match(text, /go:/, file);
    assert.match(text, /stepKey=\{/, file);
  }
  const frame = source("components/WizardFrame.tsx");
  assert.match(frame, /<PageTop>/);
  assert.match(frame, /className="as-wizard__frame"/);
  assert.match(frame, /scroller\.scrollTop = 0/);
  assert.match(frame, /className="as-thread__go"/);
  // The steps of the night wizard keep their foot, now the shared one.
  const parts = source("screens/night/parts.tsx");
  assert.match(parts, /<WizardFoot\b/);
  assert.doesNotMatch(parts, /export function Thread\b/);
  assert.doesNotMatch(parts, /as-actions as-step-foot/);
});

test("the import is a page: no window, no scrim, no second thread, no « Choisir les traitements… »", () => {
  const dir = new URL("../screens/detect/", import.meta.url);
  const files = readdirSync(dir).filter((name) => name.endsWith(".tsx"));
  for (const gone of [
    "DetectFlow.tsx",
    "DetectWindow.tsx",
    "ReviewPage.tsx",
    "ResultPage.tsx",
    "TreatmentsFlow.tsx",
  ])
    assert.ok(!files.includes(gone), `${gone} is still there`);
  for (const file of [
    "ImportScreen.tsx",
    "FoldersStep.tsx",
    "ReviewStep.tsx",
    "ImportTreatments.tsx",
    "ImportSummary.tsx",
    "ImportDone.tsx",
  ]) {
    const text = source(`screens/detect/${file}`);
    assert.doesNotMatch(text, /as-dialog__scrim/, file);
    assert.doesNotMatch(text, /<Thread\b/, file);
    assert.doesNotMatch(text, /detect\.treat\.choose/, file);
    // Échap no longer stops anything on a page: the button does.
    assert.doesNotMatch(text, /"Escape"/, file);
  }
  assert.match(
    source("screens/detect/FoldersStep.tsx"),
    /onClick: imp\.stopScan/,
  );
  assert.match(
    source("screens/detect/ImportTreatments.tsx"),
    /imp\.stopTreatments/,
  );
  // « Retour aux sessions » at the head.
  assert.match(
    source("screens/detect/ImportScreen.tsx"),
    /t\("import\.back"\)/,
  );
});

test("the import and its jobs live above the screens, as the uploads do", () => {
  const app = source("App.tsx");
  const uploads = app.indexOf("<UploadsProvider>");
  const imports = app.indexOf("<ImportProvider>");
  const shell = app.indexOf("<AppShell />");
  assert.ok(uploads > 0 && imports > uploads && shell > imports);
  assert.match(app, /screen === "import" && \(/);
  const provider = source("components/ImportProvider.tsx");
  for (const job of [
    "sessionsDetect(",
    "sessionsProvisional(",
    "detectTreatmentsPrepare(",
    "stackingRun(",
    "planImageRunLocal(",
    "detectImportCreate(",
    "detectImportPlans(",
    "detectImagesSend(",
    "detectImportDiscard(",
    "localWorkCancel(",
  ])
    assert.ok(provider.includes(job), `the provider runs ${job}`);
  // The pages only show the import: no job of their own.
  for (const file of [
    "ImportScreen.tsx",
    "FoldersStep.tsx",
    "ReviewStep.tsx",
    "ImportTreatments.tsx",
    "ImportDone.tsx",
  ]) {
    const text = source(`screens/detect/${file}`);
    for (const job of [
      "sessionsDetect(",
      "sessionsCreateBulk(",
      "detectImportCreate(",
      "stackingRun(",
      "detectImagesSend(",
    ])
      assert.ok(!text.includes(job), `${file} calls ${job}`);
  }
  // A job dropped by « Abandonner l'import » or « Arrêter » loses its result.
  assert.match(provider, /mine !== generation\.current/);
});

test("the menu shows « Import en cours » under « Sessions » with its badge, and « Sessions » the card with « Reprendre »", () => {
  const app = source("App.tsx");
  assert.match(app, /groupEntries\(group, presence\.shown\)/);
  assert.match(app, /menuEntryOf\(screen, presence\.shown\)/);
  assert.match(app, /tab\.id === "import" && !presence\.done/);
  assert.match(app, /t\("import\.badge\.waiting"\)/);
  assert.match(app, /data-menu=\{tab\.id\}/);
  const sessions = source("screens/SessionsScreen.tsx");
  assert.match(sessions, /<ImportCard onResume=\{onImport\} \/>/);
  assert.doesNotMatch(sessions, /DetectFlow|detectPage|setDetecting/);
  const card = source("screens/detect/ImportCard.tsx");
  assert.match(
    card,
    /if \(!presence\.shown \|\| presence\.done\) return null;/,
  );
  assert.match(card, /importCardLine\(presence/);
  assert.match(card, /t\("import\.card\.resume"\)/);
  assert.match(card, /buttonClassName\("tertiary"\)/);
  // « Abandonner l'import » only while nothing runs.
  assert.match(card, /presence\.waiting && \(/);
  assert.doesNotMatch(card, /buttonClassName\("primary"\)/);
});
