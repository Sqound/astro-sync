import { useEffect, useId, useRef, useState } from "react";

import { Icon } from "./Icon";

export interface LanguageOption {
  value: string;
  /** The language's own name (« 日本語 »). */
  name: string;
  /** Its tag (« ja »), written small at the right. */
  tag: string;
}

/**
 * A list of languages to choose from: the
 * field shows the chosen language; opened, a list names each language in its
 * own tongue with its tag at the right and a tick on the chosen one. A
 * combobox: the arrows move, Entrée chooses, Échap closes the list (and not the
 * window around it), a click outside closes it.
 */
export function LanguageList({
  value,
  options,
  label,
  onChange,
}: {
  value: string;
  options: readonly LanguageOption[];
  /** The accessible name. */
  label: string;
  onChange: (value: string) => void;
}) {
  const id = useId().replace(/[^A-Za-z0-9_-]/g, "");
  const root = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  const chosen = Math.max(
    0,
    options.findIndex((option) => option.value === value),
  );
  const [active, setActive] = useState(chosen);

  useEffect(() => {
    if (!open) return;
    const away = (event: MouseEvent) => {
      if (!root.current?.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", away);
    return () => document.removeEventListener("mousedown", away);
  }, [open]);

  function show() {
    setActive(chosen);
    setOpen(true);
  }

  function pick(index: number) {
    setOpen(false);
    const option = options[index];
    if (option && option.value !== value) onChange(option.value);
  }

  function onKeyDown(event: React.KeyboardEvent) {
    if (event.key === "Escape" && open) {
      // Only the list closes: the window around it stays.
      event.preventDefault();
      event.stopPropagation();
      setOpen(false);
      return;
    }
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      if (!open) {
        show();
        return;
      }
      const step = event.key === "ArrowDown" ? 1 : -1;
      setActive((index) => (index + step + options.length) % options.length);
    } else if (event.key === "Home" && open) {
      event.preventDefault();
      setActive(0);
    } else if (event.key === "End" && open) {
      event.preventDefault();
      setActive(options.length - 1);
    } else if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      if (open) pick(active);
      else show();
    } else if (event.key === "Tab") {
      setOpen(false);
    }
  }

  return (
    <div className="as-language-list" ref={root}>
      <button
        type="button"
        className="as-field as-language-list__field"
        role="combobox"
        aria-label={label}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={`${id}-list`}
        aria-activedescendant={open ? `${id}-${active}` : undefined}
        onClick={() => (open ? setOpen(false) : show())}
        onKeyDown={onKeyDown}
      >
        <span>{options[chosen]?.name}</span>
        <Icon
          name="chevron"
          className={
            open
              ? "as-icon as-language-list__chevron is-open"
              : "as-icon as-language-list__chevron"
          }
        />
      </button>
      {open && (
        <ul
          id={`${id}-list`}
          role="listbox"
          aria-label={label}
          className="as-language-list__options"
        >
          {options.map((option, index) => (
            <li
              key={option.value}
              id={`${id}-${index}`}
              role="option"
              aria-selected={option.value === value}
              className={
                index === active
                  ? "as-language-list__option is-active"
                  : "as-language-list__option"
              }
              lang={option.tag}
              onMouseEnter={() => setActive(index)}
              onMouseDown={(event) => event.preventDefault()}
              onClick={() => pick(index)}
            >
              <span>{option.name}</span>
              <span className="as-language-list__tag" aria-hidden="true">
                {option.value === value ? <Icon name="check" /> : option.tag}
              </span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
