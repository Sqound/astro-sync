import type { ReactNode } from "react";

import { useI18n } from "../../../i18n";
import type { Proposal, UserData } from "../../../lib/bridge";
import type { ReviewState } from "../../../lib/detectReview";
import { Icon } from "../../../components/Icon";

/** What every card of the review needs: the proposal, the person's data, the
 * answers so far and the way to change them. */
export interface ReviewContext {
  proposal: Proposal;
  user: UserData;
  state: ReviewState;
  change: (update: (state: ReviewState) => ReviewState) => void;
}

/** « **Remplacer “M33 – RVB” par M31** : … »: a bold lead, the separator
 * of the language, the rest. */
export function Lead({ lead, body }: { lead: ReactNode; body: ReactNode }) {
  const { t } = useI18n();
  return (
    <>
      <strong>{lead}</strong>
      {t("detect.lead.sep")}
      {body}
    </>
  );
}

/** The pieces of a line, joined by the language's separator (` · `); a
 * first piece that is an empty string asks for the separator in front. */
export function useJoin(): (parts: ReactNode[]) => ReactNode {
  const { t } = useI18n();
  return (parts) => {
    const leading = parts[0] === "";
    const kept = parts.filter(Boolean);
    return kept.flatMap((part, index) => [
      index > 0 || leading ? (
        <span key={`sep-${index}`} aria-hidden="true">
          {t("detect.join")}
        </span>
      ) : null,
      <span key={`part-${index}`}>{part}</span>,
    ]);
  };
}

/** The check mark of a « Dans Mes lieux » style pill. */
export function Tick() {
  return <Icon name="check" className="as-icon as-review__tick" />;
}
