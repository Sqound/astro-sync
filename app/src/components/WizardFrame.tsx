import { useEffect, useRef, type ReactNode } from "react";

import { Icon, type IconName } from "./Icon";
import { PageTop } from "./PageTop";
import { useI18n } from "../i18n";
import type { ThreadStep } from "../lib/wizardSteps";
import {
  buttonClassName,
  helpTextClassName,
  inactiveButtonClassName,
} from "../theme";

/**
 * The frame every wizard of the window shares: the night's (« Créer une
 * session », a row of « Sessions ») and the import (« Détecter les sessions
 * »). The head stays at the top (the way back, the eyebrow, the title, its
 * line), the steps sit in `.as-wizard__frame` under their thread, the foot
 * of a step stays at the bottom (`WizardFoot`). A step done is a button of
 * the thread when the wizard lets the person go back to it; a new step opens
 * at the top of the page.
 */
export function WizardFrame({
  back,
  eyebrow,
  title,
  line,
  steps,
  stepKey,
  className,
  children,
}: {
  /** The link at the head: where it goes and what it says. */
  back: { label: string; onClick: () => void; disabled?: boolean };
  eyebrow?: string | null;
  title?: string | null;
  line?: ReactNode;
  steps: readonly WizardThreadStep[];
  /** Changes with the step shown: the page goes back to its top. */
  stepKey: string | number | null;
  className?: string;
  children: ReactNode;
}) {
  const page = useRef<HTMLDivElement>(null);
  useEffect(() => {
    // `.as-content` scrolls, not the page: it kept the place of the step
    // before.
    const scroller = page.current?.closest<HTMLElement>(".as-content");
    if (scroller) scroller.scrollTop = 0;
  }, [stepKey]);
  return (
    <div
      ref={page}
      className={`as-page as-wizard${className ? ` ${className}` : ""}`}
    >
      <PageTop>
        <button
          type="button"
          className="as-button as-button--link as-back"
          disabled={back.disabled}
          onClick={back.onClick}
        >
          <Icon name="back" className="as-button__icon" />
          {back.label}
        </button>
        {eyebrow && <p className="as-page__eyebrow">{eyebrow}</p>}
        {title && <h1 className="as-page__title">{title}</h1>}
        {line && <p className={helpTextClassName}>{line}</p>}
      </PageTop>
      <section className="as-wizard__frame">
        <WizardThread steps={steps} />
        {children}
      </section>
    </div>
  );
}

/** A step of the thread: the night's (`ThreadStep`, its label a key) or the
 * import's, and the way back to it when the step is done and may be
 * reopened. */
export interface WizardThreadStep extends ThreadStep {
  /** Reopens the step (a step done the wizard lets the person go back to). */
  go?: () => void;
}

/** The thread of the steps: done (ticked, a button when it can be
 * reopened), current (numbered in gold), waiting. */
export function WizardThread({
  steps,
}: {
  steps: readonly WizardThreadStep[];
}) {
  const { t } = useI18n();
  return (
    <ol className="as-thread">
      {steps.map((step, index) => {
        const body = (
          <>
            <span className="as-thread__number">
              {step.status === "done" ? (
                <Icon name="check" strokeWidth={2.4} />
              ) : (
                step.n
              )}
            </span>
            <span className="as-thread__label">{t(step.labelKey)}</span>
          </>
        );
        return (
          <li
            key={step.step}
            className={`as-thread__step is-${step.status}`}
            aria-current={step.status === "current" ? "step" : undefined}
          >
            {index > 0 && (
              <span className="as-thread__rule" aria-hidden="true" />
            )}
            {step.status === "done" && step.go ? (
              <button type="button" className="as-thread__go" onClick={step.go}>
                {body}
              </button>
            ) : (
              <span className="as-thread__still">{body}</span>
            )}
          </li>
        );
      })}
    </ol>
  );
}

/** A button of the foot: its words and what it does. */
export interface FootAction {
  label: string;
  onClick: () => void;
  className?: string;
  /** The id of the sentence that says why the action waits. */
  describedBy?: string;
}

/**
 * The foot of every step: the message above, then one
 * row -- « Précédent » (a link) at the far left when there is a step to go
 * back to, the secondary action (a tertiary button) and the primary at the
 * right, greyed while it cannot go. One primary per zone, never two links.
 */
export function WizardFoot({
  message,
  previous,
  secondary,
  primary,
}: {
  message?: ReactNode;
  previous?: (() => void) | null;
  secondary?: FootAction | null;
  primary?: (FootAction & { enabled?: boolean; icon?: IconName }) | null;
}) {
  const { t } = useI18n();
  const enabled = primary ? primary.enabled !== false : false;
  return (
    <div className="as-step-foot as-wizard-foot">
      {message && <div className="as-wizard-foot__message">{message}</div>}
      <div className="as-wizard-foot__row">
        {previous && (
          <button
            type="button"
            className={`${buttonClassName("link")} as-wizard-foot__previous`}
            onClick={previous}
          >
            {t("wizard.previous")}
          </button>
        )}
        <span className="as-wizard-foot__gap" />
        {secondary && (
          <button
            type="button"
            className={`${buttonClassName("tertiary")}${secondary.className ? ` ${secondary.className}` : ""}`}
            onClick={secondary.onClick}
          >
            {secondary.label}
          </button>
        )}
        {primary && (
          <button
            type="button"
            className={`${enabled ? buttonClassName("primary") : inactiveButtonClassName}${primary.className ? ` ${primary.className}` : ""}`}
            disabled={!enabled}
            aria-describedby={primary.describedBy}
            onClick={primary.onClick}
          >
            {primary.icon && (
              <Icon name={primary.icon} className="as-button__icon" />
            )}
            {primary.label}
          </button>
        )}
      </div>
    </div>
  );
}
