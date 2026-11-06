import assert from "node:assert/strict";
import test from "node:test";

import {
  allowsNativeMenu,
  isReloadShortcut,
  type MenuContext,
} from "./contextMenu.ts";

const page: MenuContext = {
  tagName: "DIV",
  inputType: null,
  editable: false,
  selectedText: "",
  onSelection: false,
};

test("the native menu is held back on the page, where « Reload » is", () => {
  assert.equal(allowsNativeMenu(page), false);
  assert.equal(allowsNativeMenu({ ...page, tagName: "P" }), false);
  assert.equal(allowsNativeMenu({ ...page, tagName: "BUTTON" }), false);
  // A tick is not a text field.
  assert.equal(
    allowsNativeMenu({ ...page, tagName: "INPUT", inputType: "checkbox" }),
    false,
  );
  assert.equal(
    allowsNativeMenu({ ...page, tagName: "INPUT", inputType: "radio" }),
    false,
  );
});

test("it stays in a text field and on a selected text (copy a path, paste a name)", () => {
  assert.equal(
    allowsNativeMenu({ ...page, tagName: "INPUT", inputType: "text" }),
    true,
  );
  assert.equal(
    allowsNativeMenu({ ...page, tagName: "INPUT", inputType: "password" }),
    true,
  );
  assert.equal(allowsNativeMenu({ ...page, tagName: "TEXTAREA" }), true);
  assert.equal(allowsNativeMenu({ ...page, editable: true }), true);
  // Some text selected, and the click is on it: a path, a prompt.
  assert.equal(
    allowsNativeMenu({
      ...page,
      selectedText: "~/Astro Sync",
      onSelection: true,
    }),
    true,
  );
  // A selection elsewhere does not open the page menu; blank selections neither.
  assert.equal(
    allowsNativeMenu({
      ...page,
      selectedText: "~/Astro Sync",
      onSelection: false,
    }),
    false,
  );
  assert.equal(
    allowsNativeMenu({ ...page, selectedText: "  ", onSelection: true }),
    false,
  );
});

test("the reload shortcuts do nothing: F5, Ctrl+R, Cmd+R, with Shift too", () => {
  const key = (name: string, extra: Partial<Record<string, boolean>> = {}) => ({
    key: name,
    ctrlKey: false,
    metaKey: false,
    altKey: false,
    ...extra,
  });
  assert.equal(isReloadShortcut(key("F5")), true);
  assert.equal(isReloadShortcut(key("r", { ctrlKey: true })), true);
  assert.equal(isReloadShortcut(key("R", { metaKey: true })), true);
  // A plain « r » is typed, and Cmd+C or Cmd+V stay.
  assert.equal(isReloadShortcut(key("r")), false);
  assert.equal(isReloadShortcut(key("c", { metaKey: true })), false);
  assert.equal(isReloadShortcut(key("v", { ctrlKey: true })), false);
  // Alt-modified combinations are not the shortcut.
  assert.equal(
    isReloadShortcut(key("r", { ctrlKey: true, altKey: true })),
    false,
  );
});
