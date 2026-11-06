import { useI18n } from "../i18n";
import {
  analysisSendingNoticeKey,
  type AnalysisSendingNotice,
} from "../lib/capabilities";
import {
  chipClassName,
  helpTextClassName,
  noticeClassName,
  type BadgeTone,
} from "../theme";
import { Icon } from "./Icon";

/**
 * The sentence that says the analysis is here and free while the sending of
 * its report is held. On the hosted service the held feature wears its tag,
 * « Synchronisation avancée » with a padlock: not interactive --
 * Astro Sync has no offers screen, buying stays in the application and on
 * the web -- and labelled with its full name. A private server keeps its
 * own sentence and no tag: it has no offer to name.
 */
export function SendingHeldNotice({
  notice,
  tone,
}: {
  notice: AnalysisSendingNotice;
  /** A tinted block of this tone; without it, a muted line. */
  tone?: BadgeTone;
}) {
  const { t } = useI18n();
  const Tag = tone ? "div" : "p";
  return (
    <Tag className={tone ? noticeClassName(tone) : helpTextClassName}>
      {notice === "hosted" && (
        <>
          <span
            className={chipClassName}
            role="img"
            aria-label={t("chip.synchronization")}
          >
            <Icon name="lock" />
            {t("chip.synchronization")}
          </span>{" "}
        </>
      )}
      {t(analysisSendingNoticeKey(notice))}
    </Tag>
  );
}
