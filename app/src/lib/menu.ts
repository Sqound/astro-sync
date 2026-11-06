// The side menu of the window: « Accueil » and two titled groups, and the entry
// a screen belongs to. Pure TypeScript, so the order and the chosen entry are
// tested under `node --test`.

/** Every screen of the window. */
export type ScreenId =
  | "home"
  | "sessions"
  | "night"
  | "progression"
  | "logs"
  | "annotations"
  | "guide"
  | "settings"
  | "dependencies"
  | "space"
  | "dryRun"
  | "queue"
  | "import";

/** The two titled groups of the menu; « Accueil » stays alone above
 * them, outside any section. */
export type MenuSection = "nights" | "app";

export interface MenuEntry {
  id: ScreenId;
  labelKey: string;
  /** The group the entry sits under; none for « Accueil ». */
  section?: MenuSection;
}

/** The nine entries, in the order of the menu: « Accueil » alone, then
 * « Vos nuits » (Sessions, Progression, Journaux, Annotations) and
 * « Application » (Envois, the one page of the uploads --, Espace Astro
 * Portal, Réglages, Guide). The simulation (`dryRun`) has no entry: it is
 * opened from the summary of a night. */
export const MENU: readonly MenuEntry[] = [
  { id: "home", labelKey: "nav.home" },
  { id: "sessions", labelKey: "nav.sessions", section: "nights" },
  { id: "progression", labelKey: "nav.progression", section: "nights" },
  { id: "logs", labelKey: "nav.logs", section: "nights" },
  { id: "annotations", labelKey: "nav.annotations", section: "nights" },
  { id: "queue", labelKey: "nav.queue", section: "app" },
  { id: "space", labelKey: "nav.space", section: "app" },
  { id: "settings", labelKey: "nav.settings", section: "app" },
  { id: "guide", labelKey: "nav.guide", section: "app" },
];

export interface MenuGroup {
  section: MenuSection;
  titleKey: string;
  entries: readonly MenuEntry[];
}

/** « Accueil », outside any section. */
export const MENU_TOP: readonly MenuEntry[] = MENU.filter(
  (entry) => entry.section === undefined,
);

/** The titled groups, in the menu's order. */
export const MENU_GROUPS: readonly MenuGroup[] = (
  ["nights", "app"] as const
).map((section) => ({
  section,
  titleKey: `nav.section.${section}`,
  entries: MENU.filter((entry) => entry.section === section),
}));

/** « Import en cours »: an entry of « Vos nuits », right under « Sessions »,
 * only while an import is alive (from « Lancer la détection » until
 * « Terminé » is left) -- never a lasting « Import » entry. It is not in
 * `MENU`. */
export const IMPORT_ENTRY: MenuEntry = {
  id: "import",
  labelKey: "nav.import",
  section: "nights",
};

/** The entries of a group as the sidebar draws them: « Import en cours »
 * after « Sessions » while an import is shown. */
export function groupEntries(
  group: MenuGroup,
  importShown: boolean,
): readonly MenuEntry[] {
  if (!importShown || group.section !== "nights") return group.entries;
  return group.entries.flatMap((entry) =>
    entry.id === "sessions" ? [entry, IMPORT_ENTRY] : [entry],
  );
}

/** The entry chosen for a screen: the wizard of a night and the simulation
 * are opened from « Nuits » (the summary's links), which stays
 * chosen; Réglages › « Dépendances » is a tab of Réglages; the
 * import is « Import en cours » once it has its entry, « Sessions » while
 * the person still chooses the folders. */
export function menuEntryOf(screen: ScreenId, importShown = false): ScreenId {
  if (MENU.some((entry) => entry.id === screen)) return screen;
  switch (screen) {
    case "import":
      return importShown ? "import" : "sessions";
    case "night":
    case "dryRun":
      return "sessions";
    case "dependencies":
      return "settings";
    default:
      return "home";
  }
}
