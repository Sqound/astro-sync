// « Guide »: the page that says in full what the ⓘ
// of the other screens summarise -- its anchors, the structure table, the
// Finder drawn grey and annotated, the four tips with their command and
// pattern. Pure TypeScript, tested under `node --test`;
// `screens/GuideScreen.tsx` draws it.

/** The anchors at the top of the page, in their display order. */
export const GUIDE_ANCHORS = [
  "structure",
  "finder",
  "move",
  "links",
  "nina",
  "ai",
] as const;

export type GuideAnchor = (typeof GUIDE_ANCHORS)[number];

export const ANCHOR_LABEL_KEY: Record<GuideAnchor, string> = {
  structure: "guide.anchor.structure",
  finder: "guide.anchor.finder",
  move: "guide.anchor.move",
  links: "guide.anchor.links",
  nina: "guide.anchor.nina",
  ai: "guide.anchor.ai",
};

/** The section an anchor scrolls to: the four tips share « Astuces »,
 * each with its own block. */
export function sectionOf(anchor: GuideAnchor): string {
  return `guide-${anchor}`;
}

/** One row of the structure table: the folder (in the code face), what goes
 * in, what Astro Sync does with it. */
export interface StructureRow {
  /** The folder as written in every language, or a key when it holds a
   * word of the language (« <cible>/lights/ »). */
  path: string | { key: string };
  contentsKey: string;
  useKey: string;
}

export const STRUCTURE_ROWS: readonly StructureRow[] = [
  {
    path: "logs/",
    contentsKey: "guide.row.logs.contents",
    useKey: "guide.row.logs.use",
  },
  {
    path: "calibration/darks/ · flats/ · dark-flats/ · biases/",
    contentsKey: "guide.row.calibration.contents",
    useKey: "guide.row.calibration.use",
  },
  {
    path: { key: "guide.row.lights.path" },
    contentsKey: "guide.row.lights.contents",
    useKey: "guide.row.lights.use",
  },
  {
    path: { key: "guide.row.final.path" },
    contentsKey: "guide.row.final.contents",
    useKey: "guide.row.final.use",
  },
];

/** The notes under the table (the last one: the band read from names). */
export const STRUCTURE_NOTES = [
  "guide.note.name",
  "guide.note.minimum",
  "guide.note.results",
  "guide.note.never",
  "guide.note.band",
] as const;

/** The example night: its folder's name and its three targets, NGC 7000
 * unfolded. */
export const EXAMPLE_NIGHT = "2026-10-07 · Col des Étoiles · 7f3a2c";
export const EXAMPLE_TARGETS = ["NGC 7000", "IC 1396", "M31"] as const;

/** The columns of the Finder drawn grey (« dessiné, non capturé »): the
 * working folder beside the user's folders, the nights, one night, one
 * target. `open` marks the folder each column opens. */
export interface FinderColumn {
  items: { name: string; key?: string; folder: boolean; open?: boolean }[];
}

export function finderColumns(rootName: string): FinderColumn[] {
  return [
    {
      items: [
        { name: rootName, folder: true, open: true },
        { name: "", key: "guide.finder.desktop", folder: true },
        { name: "", key: "guide.finder.downloads", folder: true },
      ],
    },
    {
      items: [
        { name: "ASSISTANT-IA.md", folder: false },
        { name: "2026-10-02 · Col des Étoiles · 1d07aa", folder: true },
        { name: "2026-10-05 · Col des Étoiles · 3b91e0", folder: true },
        { name: EXAMPLE_NIGHT, folder: true, open: true },
      ],
    },
    {
      items: [
        { name: "calibration", folder: true },
        { name: "IC 1396", folder: true },
        { name: "logs", folder: true },
        { name: "M31", folder: true, open: true },
        { name: "NGC 7000", folder: true },
      ],
    },
    {
      items: [
        { name: "final", folder: true },
        { name: "lights", folder: true, open: true },
        { name: "Light_M31_120s_0001.fits", folder: false },
        { name: "Light_M31_120s_0002.fits", folder: false },
        { name: "Light_M31_120s_0003.fits", folder: false },
        { name: "…", folder: false },
      ],
    },
  ];
}

/** The system the window runs on, read from the webview's user agent (no
 * dependency: WKWebView says « Macintosh », WebView2 « Windows », WebKitGTK
 * « Linux »). */
export type GuidePlatform = "macos" | "windows" | "linux";

export function guidePlatform(userAgent: string): GuidePlatform {
  if (/Windows/iu.test(userAgent)) return "windows";
  if (/Macintosh|Mac OS X/iu.test(userAgent)) return "macos";
  return "linux";
}

/** The real capture of the Finder lives in `src/assets/guide/`. It replaces
 * the grey drawing on macOS and Linux (the folders are the same and the
 * window says « Finder · macOS »); on Windows the drawing stays until a
 * capture of the Explorer exists. `available` is the list of file names the
 * build found in that folder. */
export const FINDER_CAPTURE_FILE = "finder-macos.png";

export function finderCaptureFor(
  platform: GuidePlatform,
  available: readonly string[],
): string | null {
  if (platform === "windows") return null;
  return available.includes(FINDER_CAPTURE_FILE) ? FINDER_CAPTURE_FILE : null;
}

/** The symbolic link of the tip (macOS, Linux): the frames left on an
 * external disk, a link in place of `lights/`. `disk` is the placeholder of
 * the language (« <disque externe> »); the paths are the example's. */
export function linkCommand(rootDisplay: string, disk: string): string {
  return `ln -s "${disk}/2026-10-07/Light" \\\n  "${rootDisplay}/${EXAMPLE_NIGHT}/M31/lights"`;
}

/** NINA's two settings: the night's folder as the image path,
 * and the file name pattern that puts the lights in `<cible>/lights/`. */
export const NINA_PATTERN =
  "$$TARGETNAME$$\\lights\\$$DATETIME$$_$$FILTER$$_$$EXPOSURETIME$$s_$$FRAMENR$$";

export function ninaPath(rootDisplay: string): string {
  return `${rootDisplay}/${EXAMPLE_NIGHT}`;
}

/** `ASSISTANT-IA.md`, at the root of the working folder. */
export function assistantFile(root: string): string {
  return `${root.replace(/[\\/]$/, "")}/ASSISTANT-IA.md`;
}
