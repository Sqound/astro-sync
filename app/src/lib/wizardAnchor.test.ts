import assert from "node:assert/strict";
import test from "node:test";

import {
  HOLD_MILLISECONDS,
  REVEAL_GAP,
  holdReveal,
  revealCard,
  targetCardId,
  type HoldEnv,
  type RevealElement,
} from "./wizardAnchor.ts";

/* A page of cards under a head that stays on top, in a scrolling area: just
 * enough layout for the card to be "in view" the way the window sees it. */
class Page {
  scrollTop = 0;
  focused: number | null = null;
  readonly log: string[] = [];
  /** The height of each card, top to bottom; the window's height; what lies
   * above the first card. */
  heights: number[];
  viewport: number;
  above: number;
  constructor(heights: number[], viewport = 700, above = 120) {
    this.heights = heights;
    this.viewport = viewport;
    this.above = above;
  }
  offsetOf(index: number): number {
    return this.above + this.heights.slice(0, index).reduce((a, b) => a + b, 0);
  }
  get contentHeight(): number {
    return this.offsetOf(this.heights.length) + 400;
  }
  topOf(index: number): number {
    return this.offsetOf(index) - this.scrollTop;
  }
  card(index: number): RevealElement {
    const page = this;
    return {
      style: { scrollMarginTop: "0px" },
      scrollIntoView(options) {
        assert.equal(options.block, "start");
        const margin = Number.parseInt(this.style.scrollMarginTop, 10);
        const max = Math.max(0, page.contentHeight - page.viewport);
        page.scrollTop = Math.min(
          max,
          Math.max(0, page.offsetOf(index) - margin),
        );
        page.log.push(`scroll:${index}`);
      },
      focus(options) {
        assert.equal(options?.preventScroll, true, "focus never scrolls again");
        page.focused = index;
        page.log.push(`focus:${index}`);
      },
    };
  }
}

test("the id of a card keeps what an id can hold and tells targets apart", () => {
  assert.equal(targetCardId("target-m45"), "as-file-card-target-m45");
  assert.equal(targetCardId("a b/c"), "as-file-card-a_b_c");
  assert.notEqual(targetCardId("t-1"), targetCardId("t-2"));
});

test("a card is scrolled below what covers the top, then focused without scrolling again", () => {
  const page = new Page(Array(10).fill(300));
  const card = page.card(4);
  revealCard(card, 193.2);
  assert.equal(card.style.scrollMarginTop, `${194 + REVEAL_GAP}px`);
  assert.deepEqual(
    page.log,
    ["scroll:4", "focus:4"],
    "scroll first, focus after",
  );
  assert.equal(page.topOf(4), 194 + REVEAL_GAP, "just under the head");
  assert.equal(page.focused, 4);
});

test("a target far down a long list is brought into view and focused", () => {
  // Forty targets: the 36th is more than ten screens down.
  const page = new Page(Array(40).fill(320));
  assert.ok(
    page.topOf(35) > 10 * page.viewport,
    "it starts far below the fold",
  );
  const card = page.card(35);
  revealCard(card, 160);
  const top = page.topOf(35);
  assert.ok(top >= 160 && top + page.heights[35] <= page.viewport + 400);
  assert.equal(top, 160 + REVEAL_GAP, "at the top of the area, under the head");
  assert.equal(page.focused, 35);
});

test("the last cards of a short page cannot reach the top: they are whole in view and focused", () => {
  const page = new Page(Array(6).fill(260));
  const card = page.card(5);
  revealCard(card, 160);
  assert.equal(page.focused, 5);
  assert.ok(page.topOf(5) >= 160);
  assert.ok(page.topOf(5) + page.heights[5] <= page.viewport);
});

/* The environment of the hold: the content above the card changes by hand. */
function env(page: Page, index: number) {
  const card = page.card(index);
  const watchers = new Set<() => void>();
  const interrupts = new Set<() => void>();
  const timers: { ms: number; run: () => void; live: boolean }[] = [];
  const world: HoldEnv = {
    reveal: () => revealCard(card, 160),
    watch: (callback) => {
      watchers.add(callback);
      return () => watchers.delete(callback);
    },
    interrupted: (callback) => {
      interrupts.add(callback);
      return () => interrupts.delete(callback);
    },
    after: (ms, run) => {
      const timer = { ms, run, live: true };
      timers.push(timer);
      return () => {
        timer.live = false;
      };
    },
  };
  return {
    world,
    watchers,
    interrupts,
    timers,
    /** What is above the card changes size (a measure card arrives). */
    grow(card: number, by: number) {
      page.heights[card] += by;
      for (const callback of [...watchers]) callback();
    },
    userScrolls() {
      for (const callback of [...interrupts]) callback();
      page.scrollTop += 50;
    },
    elapse() {
      for (const timer of timers) if (timer.live) timer.run();
    },
  };
}

test("while the cards above settle, the target stays at the top, even far down", () => {
  const page = new Page(Array(30).fill(300));
  const it = env(page, 27);
  holdReveal(it.world);
  assert.equal(page.topOf(27), 160 + REVEAL_GAP);
  // The measure cards of the first targets arrive, one after another.
  it.grow(0, 140);
  assert.equal(page.topOf(27), 160 + REVEAL_GAP, "put back after the first");
  it.grow(1, 140);
  it.grow(2, 140);
  assert.equal(page.topOf(27), 160 + REVEAL_GAP);
  assert.equal(page.focused, 27);
});

test("the hold gives the scroll back as soon as the person takes it", () => {
  const page = new Page(Array(30).fill(300));
  const it = env(page, 12);
  holdReveal(it.world);
  it.userScrolls();
  const where = page.scrollTop;
  it.grow(0, 140);
  assert.equal(page.scrollTop, where, "no yank after the person scrolled");
  assert.equal(it.watchers.size, 0, "nothing is left watching");
  assert.equal(it.interrupts.size, 0, "nothing is left listening");
});

test("the hold ends after its time, and a hold stopped at once leaves nothing behind", () => {
  const page = new Page(Array(30).fill(300));
  const it = env(page, 12);
  holdReveal(it.world);
  assert.deepEqual(
    it.timers.map((timer) => timer.ms),
    [HOLD_MILLISECONDS],
  );
  it.elapse();
  const where = page.scrollTop;
  it.grow(0, 140);
  assert.equal(page.scrollTop, where, "after the time, the card is left alone");

  const again = env(new Page(Array(5).fill(300)), 2);
  const stop = holdReveal(again.world);
  stop();
  stop(); // a second stop is harmless
  assert.equal(again.watchers.size + again.interrupts.size, 0);
  assert.ok(again.timers.every((timer) => !timer.live));
});
