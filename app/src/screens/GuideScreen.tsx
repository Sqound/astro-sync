import { useEffect, useState } from "react";

import { useGuideAsk } from "../components/GuideAskWindow";
import { Icon } from "../components/Icon";
import { Notice } from "../components/Notice";
import { PageTop } from "../components/PageTop";
import { useI18n } from "../i18n";
import { folderOpen, workingFolderGet } from "../lib/bridge";
import { formatError } from "../lib/format";
import {
  ANCHOR_LABEL_KEY,
  EXAMPLE_NIGHT,
  EXAMPLE_TARGETS,
  GUIDE_ANCHORS,
  NINA_PATTERN,
  STRUCTURE_NOTES,
  STRUCTURE_ROWS,
  assistantFile,
  finderCaptureFor,
  finderColumns,
  guidePlatform,
  linkCommand,
  ninaPath,
  sectionOf,
  type GuideAnchor,
} from "../lib/guide";
import { displayPath, homeOf } from "../lib/home";
import { baseName, folderTree } from "../lib/wizardSteps";
import { buttonClassName, helpTextClassName } from "../theme";
import { copyText } from "./night/parts";

// The real captures of the guide, found at build time: a missing file is
// not an error, the drawing stays.
const GUIDE_CAPTURES = import.meta.glob("../assets/guide/*.png", {
  eager: true,
  query: "?url",
  import: "default",
}) as Record<string, string>;

function captureUrl(file: string | null): string | null {
  if (!file) return null;
  const key = Object.keys(GUIDE_CAPTURES).find((path) =>
    path.endsWith(`/${file}`),
  );
  return key ? GUIDE_CAPTURES[key] : null;
}

/**
 * « Guide »: a page that reads. Anchors at the top
 * (outside the levels, the chosen one as a segment), the structure and its
 * table, the Finder drawn grey and annotated (replaced by the real capture
 * of the Finder on macOS and Linux), the four tips; no action but links («
 * Copier la commande », « Copier le modèle », « Ouvrir ASSISTANT-IA.md »)
 * and, since the guide question, the one secondary button at the
 * top, « Poser une question à une IA », above the anchors.
 */
export function GuideScreen({
  initialAnchor,
}: {
  initialAnchor?: GuideAnchor | null;
}) {
  const { t } = useI18n();
  const [anchor, setAnchor] = useState<GuideAnchor>(
    initialAnchor ?? "structure",
  );
  const [root, setRoot] = useState<string | null>(null);
  const [copied, setCopied] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const guideAsk = useGuideAsk();

  useEffect(() => {
    workingFolderGet()
      .then((state) => setRoot(state.root))
      .catch(() => setRoot(null));
  }, []);

  useEffect(() => {
    if (initialAnchor) go(initialAnchor);
    // Only the anchor the page was opened at.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  function go(next: GuideAnchor) {
    setAnchor(next);
    document
      .getElementById(sectionOf(next))
      ?.scrollIntoView({ block: "start", behavior: "smooth" });
  }

  async function copy(id: string, text: string) {
    setCopied((await copyText(text)) ? id : null);
  }

  const shown = root
    ? displayPath(root, homeOf(root))
    : "~/Documents/Astro Sync";
  const rootName = root ? baseName(root) : "Astro Sync";
  const finderCapture = captureUrl(
    finderCaptureFor(
      guidePlatform(navigator.userAgent),
      Object.keys(GUIDE_CAPTURES).map((path) => path.split("/").pop() ?? path),
    ),
  );

  return (
    <div className="as-page">
      <PageTop>
        <h1 className="as-page__title">
          <Icon name="book" /> {t("guide.title")}
        </h1>
        <p className={helpTextClassName}>{t("guide.lead")}</p>
      </PageTop>
      <section className="as-guide-ask" aria-label={t("guide.ask.button")}>
        <p className={helpTextClassName}>{t("guide.ask.lead")}</p>
        <button
          type="button"
          className={buttonClassName("secondary")}
          disabled={guideAsk.busy}
          onClick={() => void guideAsk.ask()}
        >
          <Icon name="chat" className="as-button__icon" />
          {t("guide.ask.button")}
        </button>
      </section>
      {guideAsk.window}
      <nav className="as-segments" aria-label={t("guide.title")}>
        {GUIDE_ANCHORS.map((item) => (
          <button
            key={item}
            type="button"
            className={item === anchor ? "as-segment is-chosen" : "as-segment"}
            aria-current={item === anchor ? "true" : undefined}
            onClick={() => go(item)}
          >
            {t(ANCHOR_LABEL_KEY[item])}
          </button>
        ))}
      </nav>
      <Notice tone="error">{error}</Notice>

      <section
        id={sectionOf("structure")}
        className="as-form-card as-guide-section"
      >
        <h2 className="as-form-card__title">{t("guide.anchor.structure")}</h2>
        <p className={helpTextClassName}>{t("guide.structure.lead")}</p>
        <div className="as-fold__body">
          <ul className="as-tree">
            {folderTree(rootName, EXAMPLE_NIGHT, EXAMPLE_TARGETS).map(
              (row, index) => (
                <li
                  key={index}
                  style={{ paddingLeft: `${row.depth * 1.25}rem` }}
                >
                  <Icon
                    name={row.file ? "book" : "folder"}
                    className="as-tree__icon"
                  />
                  <code>{row.name}</code>
                  {(row.noteKey || row.note) && (
                    <span className="as-tree__note">
                      {row.noteKey ? t(row.noteKey) : row.note}
                    </span>
                  )}
                </li>
              ),
            )}
          </ul>
        </div>
        <table className="as-guide-table">
          <thead>
            <tr>
              <th>{t("guide.table.folder")}</th>
              <th>{t("guide.table.contents")}</th>
              <th>{t("guide.table.use")}</th>
            </tr>
          </thead>
          <tbody>
            {STRUCTURE_ROWS.map((row) => (
              <tr key={row.contentsKey}>
                <td>
                  <code>
                    {typeof row.path === "string" ? row.path : t(row.path.key)}
                  </code>
                </td>
                <td>{t(row.contentsKey)}</td>
                <td>{t(row.useKey)}</td>
              </tr>
            ))}
          </tbody>
        </table>
        <ul className="as-guide-notes">
          {STRUCTURE_NOTES.map((key) => (
            <li key={key}>
              <Icon name="info" />
              <span>{t(key)}</span>
            </li>
          ))}
        </ul>
      </section>

      <section
        id={sectionOf("finder")}
        className="as-form-card as-guide-section"
      >
        <h2 className="as-form-card__title">{t("guide.anchor.finder")}</h2>
        <p className={helpTextClassName}>{t("guide.finder.lead")}</p>
        {finderCapture ? (
          <figure className="as-finder as-finder--captured">
            <figcaption className="as-finder__caption">
              {t("guide.finder.captured")}
            </figcaption>
            <img
              className="as-finder__image"
              src={finderCapture}
              alt={t("guide.finder.alt")}
            />
          </figure>
        ) : (
          <div className="as-finder" aria-label={t("guide.finder.drawn")}>
            <p className="as-finder__caption">{t("guide.finder.drawn")}</p>
            <div className="as-finder__columns">
              {finderColumns(rootName).map((column, index) => (
                <ul key={index} className="as-finder__column">
                  {column.items.map((item) => (
                    <li
                      key={item.key ?? item.name}
                      className={item.open ? "is-open" : undefined}
                    >
                      <span>{item.key ? t(item.key) : item.name}</span>
                      {item.folder && <span aria-hidden="true">›</span>}
                    </li>
                  ))}
                </ul>
              ))}
            </div>
          </div>
        )}
        <p className={helpTextClassName}>{t("guide.finder.windows")}</p>
        <div className="as-tip">
          <h3 className="as-tip__title">{t("guide.finder.finalTitle")}</h3>
          <p className={helpTextClassName}>{t("guide.finder.finalLine")}</p>
        </div>
        <div className="as-tip">
          <h3 className="as-tip__title">{t("guide.finder.hiddenTitle")}</h3>
          <p className={helpTextClassName}>{t("guide.finder.hiddenLine")}</p>
        </div>
      </section>

      <section className="as-form-card as-guide-section">
        <h2 className="as-form-card__title">{t("guide.tips")}</h2>
        <div id={sectionOf("move")} className="as-tip as-guide-section">
          <h3 className="as-tip__title">{t("guide.anchor.move")}</h3>
          <p className={helpTextClassName}>{t("guide.move.line")}</p>
        </div>
        <div id={sectionOf("links")} className="as-tip as-guide-section">
          <h3 className="as-tip__title">{t("guide.links.title")}</h3>
          <p className={helpTextClassName}>{t("guide.links.line")}</p>
          <pre>
            <code>{linkCommand(shown, t("guide.links.disk"))}</code>
          </pre>
          <button
            type="button"
            className={buttonClassName("link")}
            onClick={() =>
              void copy("links", linkCommand(shown, t("guide.links.disk")))
            }
          >
            <Icon name="copy" className="as-button__icon" />
            {t("guide.copyCommand")}
          </button>
          {copied === "links" && (
            <p className={helpTextClassName}>{t("guide.copied")}</p>
          )}
        </div>
        <div id={sectionOf("nina")} className="as-tip as-guide-section">
          <h3 className="as-tip__title">{t("guide.anchor.nina")}</h3>
          <p className={helpTextClassName}>{t("guide.nina.line")}</p>
          <p className={helpTextClassName}>{t("guide.nina.path")}</p>
          <pre>
            <code>{ninaPath(shown)}</code>
          </pre>
          <p className={helpTextClassName}>{t("guide.nina.pattern")}</p>
          <pre>
            <code>{NINA_PATTERN}</code>
          </pre>
          <button
            type="button"
            className={buttonClassName("link")}
            onClick={() => void copy("nina", NINA_PATTERN)}
          >
            <Icon name="copy" className="as-button__icon" />
            {t("guide.copyPattern")}
          </button>
          {copied === "nina" && (
            <p className={helpTextClassName}>{t("guide.copied")}</p>
          )}
        </div>
        <div id={sectionOf("ai")} className="as-tip as-guide-section">
          <h3 className="as-tip__title">{t("wizard.folder.askAssistant")}</h3>
          <p className={helpTextClassName}>{t("guide.ai.line")}</p>
          <button
            type="button"
            className={buttonClassName("link")}
            disabled={!root}
            onClick={() =>
              root &&
              void folderOpen(assistantFile(root)).catch((caught: unknown) =>
                setError(formatError(t, caught)),
              )
            }
          >
            <Icon name="book" className="as-button__icon" />
            {t("guide.openAssistant")}
          </button>
        </div>
      </section>
    </div>
  );
}
