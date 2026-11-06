import { Icon } from "../../../components/Icon";
import { useI18n } from "../../../i18n";
import type {
  CalibrationNote,
  DetectedTarget,
  Proposal,
} from "../../../lib/bridge";
import { asideFiles } from "../../../lib/detectReview";
import {
  calibrationNoteLevel,
  calibrationNoteText,
  darksToSee,
  nothingReadable,
  nothingReadableText,
  solveWhy,
  unreadCounts,
} from "../../../lib/missingMessages";
import { pluralText } from "../../../lib/written";
import { buttonClassName } from "../../../theme";

/** An information: a grey line, nothing is damaged. */
export function InfoNote({ text }: { text: string }) {
  return (
    <p className="as-review__info">
      <Icon name="info" className="as-icon as-review__info-icon" />
      <span>{text}</span>
    </p>
  );
}

/** One line of a warning banner. */
function WarningLine({ text }: { text: string }) {
  return (
    <div className="as-review__ask-text">
      <Icon name="warning" className="as-icon as-review__ask-icon" />
      <span>{text}</span>
    </div>
  );
}

/** What the poses of a night lack in calibration: one banner for the
 * warnings (a dark that does not fit, a filter without flats), a grey line
 * for each information (no dark or flat at all). The link opens the files
 * set aside, where the darks are. */
export function CalibrationNotes({
  notes,
  onSeeFiles,
}: {
  notes: readonly CalibrationNote[];
  onSeeFiles: () => void;
}) {
  const { t, locale } = useI18n();
  const warnings = notes.filter(
    (note) => calibrationNoteLevel(note) === "warning",
  );
  const informations = notes.filter(
    (note) => calibrationNoteLevel(note) === "information",
  );
  const darks = darksToSee(notes);
  if (notes.length === 0) return null;
  return (
    <>
      {warnings.length > 0 && (
        <div className="as-review__ask" data-calibration-notes="warning">
          {warnings.map((note, index) => (
            <WarningLine
              key={`${note.code}-${index}`}
              text={calibrationNoteText(t, locale, note)}
            />
          ))}
          {darks > 0 && (
            <div className="as-review__end">
              <button
                type="button"
                className={buttonClassName("link")}
                onClick={onSeeFiles}
              >
                {pluralText(t, locale, "calibration.seeDarks", darks)}
              </button>
            </div>
          )}
        </div>
      )}
      {informations.map((note, index) => (
        <InfoNote
          key={`${note.code}-${index}`}
          text={calibrationNoteText(t, locale, note)}
        />
      ))}
    </>
  );
}

/** Why the solver gave no target for a group of poses: the words of its
 * failure, or the position of the field. A link to « Réglages › Dépendances »
 * only for a reason the person corrects there. */
export function SolveWhy({
  target,
  onSettings,
}: {
  target: Pick<DetectedTarget, "solveFailure" | "solvedPosition">;
  onSettings?: () => void;
}) {
  const { t } = useI18n();
  const why = solveWhy(t, target);
  if (!why) return null;
  if (why.level === "information") {
    return (
      <div data-solve-why="information">
        <InfoNote text={why.text} />
      </div>
    );
  }
  return (
    <div className="as-review__why" data-solve-why="warning">
      <WarningLine text={why.text} />
      {why.settings && onSettings && (
        <div className="as-review__end">
          <button
            type="button"
            className={buttonClassName("link")}
            onClick={onSettings}
          >
            {t("wizard.treatments.dependencies")}
          </button>
        </div>
      )}
    </div>
  );
}

/** The folder holds RAW files or videos and no readable pose: a warning
 * under the title of the review, with the way to see what was left out. */
export function NothingReadable({
  proposal,
  open,
  onToggle,
}: {
  proposal: Proposal;
  open: boolean;
  onToggle: () => void;
}) {
  const { t, locale } = useI18n();
  if (!nothingReadable(proposal)) return null;
  const files = asideFiles(proposal).all;
  return (
    <div
      className="as-notice as-notice--warning as-review__nothing"
      role="status"
    >
      <div className="as-review__ask-text">
        <Icon name="warning" className="as-icon as-review__ask-icon" />
        <span>{nothingReadableText(t, locale, unreadCounts(proposal))}</span>
      </div>
      <div className="as-review__end">
        <button
          type="button"
          className={buttonClassName("link")}
          aria-expanded={open}
          onClick={onToggle}
        >
          {t(open ? "detect.aside.hide" : "detect.aside.see", {
            count: files.length,
          })}
        </button>
      </div>
      {open && (
        <ul className="as-review__files">
          {files.map((file, index) => (
            <li key={`${file}-${index}`}>{file}</li>
          ))}
        </ul>
      )}
    </div>
  );
}
