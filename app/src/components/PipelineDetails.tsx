import { useId, useState } from "react";

import { buttonClassName, helpTextClassName } from "../theme/primitives";
import { Icon } from "./Icon";

/** One step of a pipeline's `details`, in the person's language. */
export interface PipelineDetailStep {
  title: string;
  text: string;
}

/** « Détails », folded under a pipeline: its steps in order, what each does
 * and why. Sober on purpose: one link-level button, an ordered list. Nothing
 * when the definition gives no steps. */
export function PipelineDetails({
  label,
  steps,
  initiallyOpen = false,
}: {
  label: string;
  steps: readonly PipelineDetailStep[];
  initiallyOpen?: boolean;
}) {
  const [open, setOpen] = useState(initiallyOpen);
  const panelId = useId();
  if (steps.length === 0) return null;
  return (
    <div className="as-pipeline__details">
      <button
        type="button"
        className={buttonClassName("link")}
        aria-expanded={open}
        aria-controls={panelId}
        onClick={() => setOpen((current) => !current)}
      >
        <Icon
          name="chevron"
          className={open ? "as-how__chevron is-open" : "as-how__chevron"}
        />
        {label}
      </button>
      {open && (
        <ol id={panelId} className="as-pipeline__steps">
          {steps.map((step, index) => (
            <li key={index}>
              <strong>{step.title}</strong>
              <span className={helpTextClassName}>{step.text}</span>
            </li>
          ))}
        </ol>
      )}
    </div>
  );
}
