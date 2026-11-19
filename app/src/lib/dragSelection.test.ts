import assert from "node:assert/strict";
import test from "node:test";

import {
  DRAGGING_CLASS,
  DRAG_SELECTOR,
  END_EVENTS,
  createDragGuard,
  listensInCapture,
  startsDrag,
  type DragEventLike,
  type DragHost,
} from "./dragSelection.ts";

/** A window that keeps its listeners, its root classes and its selection. */
function fakeHost() {
  const listeners = new Map<string, Set<(event: DragEventLike) => void>>();
  const classes = new Set<string>();
  let cleared = 0;
  const host: DragHost = {
    addEventListener: (type, listener) => {
      if (!listeners.has(type)) listeners.set(type, new Set());
      listeners.get(type)!.add(listener);
    },
    removeEventListener: (type, listener) => {
      listeners.get(type)?.delete(listener);
    },
    rootClasses: {
      add: (name) => classes.add(name),
      remove: (name) => classes.delete(name),
    },
    clearSelection: () => {
      cleared += 1;
    },
  };
  const fire = (type: string, event: Partial<DragEventLike> = {}) => {
    let prevented = false;
    const full: DragEventLike = {
      preventDefault: () => {
        prevented = true;
      },
      ...event,
    };
    for (const listener of [...(listeners.get(type) ?? [])]) listener(full);
    return prevented;
  };
  return {
    host,
    classes,
    fire,
    cleared: () => cleared,
    count: (type: string) => listeners.get(type)?.size ?? 0,
  };
}

test("only the main button on something draggable starts a drag", () => {
  assert.equal(startsDrag({ button: 0, matchesDraggable: true }), true);
  assert.equal(startsDrag({ button: 2, matchesDraggable: true }), false);
  assert.equal(startsDrag({ button: 0, matchesDraggable: false }), false);
});

test("the sliders and the curtain are what a drag is made on", () => {
  assert.match(DRAG_SELECTOR, /input\[type="range"\]/);
  assert.match(DRAG_SELECTOR, /\[role="slider"\]/);
  assert.match(DRAG_SELECTOR, /\[data-as-drag\]/);
});

test("from the pointer going down to its release nothing can be selected, and the selection is let go", () => {
  const window = fakeHost();
  let draggable = true;
  createDragGuard(window.host, () => draggable);
  // Not dragging yet: a selection may start (a path, an error message).
  assert.equal(window.fire("selectstart"), false);
  window.fire("pointerdown", { button: 0 });
  assert.ok(window.classes.has(DRAGGING_CLASS));
  assert.equal(window.cleared(), 1);
  // The pointer goes out of the frame: the selection that starts is refused.
  assert.equal(window.fire("selectstart"), true);
  // The release, anywhere: the class goes, the selection is cleared.
  window.fire("pointerup");
  assert.ok(!window.classes.has(DRAGGING_CLASS));
  assert.equal(window.cleared(), 2);
  assert.equal(window.fire("selectstart"), false);
  // Something that is not draggable starts nothing.
  draggable = false;
  window.fire("pointerdown", { button: 0 });
  assert.ok(!window.classes.has(DRAGGING_CLASS));
});

test("every way a drag can end ends it, and no listener stays behind", () => {
  for (const ending of END_EVENTS) {
    const window = fakeHost();
    const remove = createDragGuard(window.host, () => true);
    window.fire("pointerdown", { button: 0 });
    assert.ok(window.classes.has(DRAGGING_CLASS), ending);
    window.fire(ending);
    assert.ok(!window.classes.has(DRAGGING_CLASS), ending);
    assert.equal(window.count("selectstart"), 0, ending);
    for (const type of END_EVENTS) assert.equal(window.count(type), 0, ending);
    remove();
    assert.equal(window.count("pointerdown"), 0, ending);
  }
});

test("a second pointer going down during a drag does not stack listeners", () => {
  const window = fakeHost();
  createDragGuard(window.host, () => true);
  window.fire("pointerdown", { button: 0 });
  window.fire("pointerdown", { button: 0 });
  assert.equal(window.count("selectstart"), 1);
  window.fire("pointerup");
  assert.ok(!window.classes.has(DRAGGING_CLASS));
});

test("the focus moving from a field to a slider does not end the drag", () => {
  // A blur of an element reaches the window only in the capture phase; the
  // guard listens for the window's own blur, not in the capture phase.
  assert.equal(listensInCapture("blur"), false);
  assert.equal(listensInCapture("pointerup"), true);
});
