import { useI18n } from "../i18n";
import { scanPercent } from "../lib/detectFlow";
import { displayPath, homeOf } from "../lib/home";
import { pluralText } from "../lib/written";
import { helpTextClassName, progressClassName } from "../theme";
import { Waiting } from "./Waiting";

/**
 * The head of the reading of « Détecter les sessions » (the page of the import
 * and the sheet of a night). Two moments:
 *
 * - **the search for the files** (`total` 0: a walk reads no byte, so there is
 *   nothing to measure): the star and « Recherche des fichiers… », the count
 *   of what it found so far (« 412 trouvés ») and, in mono, the folder it is
 *   in. No bar, no « 0 sur 0 »;
 * - **the reading of the headers** (`total` known): « Lecture des en-têtes »,
 *   « x fichiers lus sur N » and the bar.
 */
export function ScanHead({
  read,
  total,
  walked,
  folder,
}: {
  read: number;
  total: number;
  walked: number;
  folder: string | null;
}) {
  const { t, locale } = useI18n();
  if (!(total > 0)) {
    return (
      <>
        <div className="as-detect-scan__head">
          <Waiting size="inline" label={t("detect.scan.walk")} />
          <span className={helpTextClassName}>
            {pluralText(t, locale, "detect.scan.walkFound", walked, {
              count: walked.toLocaleString(locale),
            })}
          </span>
        </div>
        {folder && (
          <p className="as-detect-scan__current" title={folder}>
            {t("detect.scan.walkIn", {
              folder: displayPath(folder, homeOf(folder)),
            })}
          </p>
        )}
      </>
    );
  }
  const percent = scanPercent(read, total);
  return (
    <>
      <div className="as-detect-scan__head">
        <span>{t("detect.scan.headers")}</span>
        <span className={helpTextClassName}>
          {t("detect.scan.count", {
            read: read.toLocaleString(locale),
            total: total.toLocaleString(locale),
          })}
        </span>
      </div>
      <div
        className={progressClassName.track}
        role="progressbar"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={percent}
      >
        <div
          className={progressClassName.fill}
          style={{ width: `${percent}%` }}
        />
      </div>
    </>
  );
}
