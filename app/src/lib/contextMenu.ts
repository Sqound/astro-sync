// The window's context menu and its reload shortcuts.
//
// Right-clicking a window of the application showed the web view's own menu,
// whose « Reload » (macOS' WKWebView) reloads the whole window: the wizard's
// step, the ticks and the choices are lost -- a trap at the end of a
// processing. The native menu stays only where it is useful (copy, paste in
// a text field; copy on a selected text, a path or a prompt) and is held
// back everywhere else; the reload shortcuts do nothing. Pure TypeScript
// for the decisions (tested under `node --test`); `installMenuGuard` is the
// thin installer `main.tsx` calls.

/** What the window knows about a right-click. */
export interface MenuContext {
  /** The tag of the element clicked, upper case (`""` when none). */
  tagName: string;
  /** The `type` of an `<input>`, `null` for any other element. */
  inputType: string | null;
  /** The element is inside an editable region (`contenteditable`). */
  editable: boolean;
  /** The text the page has selected, `""` when none. */
  selectedText: string;
  /** The click is on the selected text itself. */
  onSelection: boolean;
}

/** The types of `<input>` a person types text into. */
const TEXT_INPUTS = new Set([
  "text",
  "search",
  "url",
  "tel",
  "email",
  "password",
  "number",
]);

/** The native menu is kept in a text field (cut, copy, paste) and on a
 * selected text (copy); everywhere else it is held back. */
export function allowsNativeMenu(context: MenuContext): boolean {
  if (context.editable) return true;
  if (context.tagName === "TEXTAREA") return true;
  if (context.tagName === "INPUT") {
    return TEXT_INPUTS.has((context.inputType ?? "text").toLowerCase());
  }
  return context.selectedText.trim() !== "" && context.onSelection;
}

/** The keys that reload a web view: F5, Ctrl/Cmd + R, with or without Shift. */
export function isReloadShortcut(key: {
  key: string;
  ctrlKey: boolean;
  metaKey: boolean;
  altKey: boolean;
}): boolean {
  if (key.key === "F5") return true;
  return (
    (key.ctrlKey || key.metaKey) && !key.altKey && key.key.toLowerCase() === "r"
  );
}

/** Holds back the native menu where it is not useful and the reload
 * shortcuts, on `target` (the window). */
export function installMenuGuard(target: Window): void {
  target.addEventListener("contextmenu", (event) => {
    const element = event.target instanceof Element ? event.target : null;
    const selection = target.getSelection();
    const allowed = allowsNativeMenu({
      tagName: element?.tagName ?? "",
      inputType: element instanceof HTMLInputElement ? element.type : null,
      editable:
        element?.closest('[contenteditable]:not([contenteditable="false"])') !=
        null,
      selectedText: selection?.toString() ?? "",
      onSelection:
        element !== null &&
        selection !== null &&
        selection.rangeCount > 0 &&
        selection.containsNode(element, true),
    });
    if (!allowed) event.preventDefault();
  });
  target.addEventListener("keydown", (event) => {
    if (isReloadShortcut(event)) event.preventDefault();
  });
}
