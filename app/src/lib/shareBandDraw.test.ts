import assert from "node:assert/strict";
import test from "node:test";

import { createTranslator } from "../i18n/locale.ts";
import {
  bandText,
  copyLayout,
  type BandInput,
  type ShareOptions,
} from "./shareBand.ts";
import {
  bandGeometry,
  drawShareCopy,
  type DrawContext,
  type DrawEnvironment,
} from "./shareBandDraw.ts";

const INPUT: BandInput = {
  name: "Les Pléiades",
  designation: "M45",
  night: { count: 1, dates: ["2026-10-24"] },
  frames: 430,
  exposureSeconds: 30,
  totalSeconds: 12_900,
};

interface Call {
  op: string;
  args: unknown[];
}

/** A context that records what is drawn, and measures a character at 0.55
 * of the font size. */
function recorder() {
  const calls: Call[] = [];
  let font = "10px x";
  const ctx = {
    fillStyle: "",
    strokeStyle: "",
    lineWidth: 1,
    lineJoin: "miter",
    textAlign: "left",
    textBaseline: "alphabetic",
    globalCompositeOperation: "source-over",
    get font() {
      return font;
    },
    set font(value: string) {
      font = value;
    },
    save() {},
    restore() {},
    translate() {},
    scale() {},
    rotate() {},
    beginPath() {},
    arc() {},
    stroke() {},
    fill() {},
    fillRect(...args: number[]) {
      calls.push({ op: "fillRect", args: [...args, this.fillStyle] });
    },
    fillText(...args: unknown[]) {
      const [text, x] = args as [string, number];
      const extent = this.measureText(text).width;
      const start = this.textAlign === "right" ? x - extent : x;
      calls.push({
        op: "fillText",
        args: [...args, this.fillStyle, start, start + extent, this.textAlign],
      });
    },
    drawImage(...args: unknown[]) {
      calls.push({ op: "drawImage", args });
    },
    measureText(text: string) {
      const size = Number(/(\d+(?:\.\d+)?)px/.exec(font)?.[1] ?? 10);
      return { width: text.length * size * 0.55 };
    },
  } as unknown as DrawContext;
  const env: DrawEnvironment = {
    fontFamily: "x",
    makeLayer: () => ({ image: {} as CanvasImageSource, ctx }),
    makePath: () => ({}) as Path2D,
  };
  return { ctx, env, calls };
}

const draw = (
  width: number,
  imageHeight: number,
  options: ShareOptions,
  input: BandInput = INPUT,
) => {
  const text = bandText(createTranslator("fr"), "fr", input);
  const layout = copyLayout(width, imageHeight, text, options);
  const { ctx, env, calls } = recorder();
  drawShareCopy(ctx, {} as CanvasImageSource, layout, text, options, env);
  return { layout, calls };
};

test("the image is drawn first, whole, at the top left", () => {
  const { layout, calls } = draw(1080, 1620, { data: true, mark: true });
  assert.deepEqual(calls[0], {
    op: "drawImage",
    args: [{}, 0, 0, layout.width, layout.imageHeight],
  });
  assert.equal(
    calls.filter(
      (call) =>
        call.op === "drawImage" &&
        call.args.length === 5 &&
        call.args[1] === 0 &&
        call.args[2] === 0,
    ).length,
    1,
  );
});

test("nothing of the band is drawn over the image, in any variant", () => {
  for (const options of [
    { data: true, mark: true },
    { data: true, mark: false },
    { data: false, mark: true },
  ]) {
    for (const [width, imageHeight] of [
      [600, 900],
      [1080, 1620],
      [2160, 3991],
    ]) {
      const { layout, calls } = draw(width, imageHeight, options);
      for (const call of calls.slice(1)) {
        if (call.op === "fillRect") {
          assert.ok(
            (call.args[1] as number) >= layout.imageHeight,
            `${JSON.stringify(options)} ${width}: rect at ${call.args[1]}`,
          );
        }
        if (call.op === "fillText") {
          // Text is centred on its line, so it is inside the band.
          const y = call.args[2] as number;
          assert.ok(
            y > layout.imageHeight && y < layout.height,
            `text at ${y}`,
          );
        }
        if (call.op === "drawImage") {
          // The figure of the mark, laid inside the band.
          assert.ok(
            (call.args[2] as number) >= layout.imageHeight,
            "figure over the image",
          );
        }
      }
    }
  }
});

test("no switch on: the image alone and nothing else", () => {
  const { calls } = draw(1080, 1620, { data: false, mark: false });
  assert.equal(calls.length, 1);
  assert.equal(calls[0].op, "drawImage");
});

test("the full band writes the target, the night, the time and the mark", () => {
  const { calls } = draw(1080, 1620, { data: true, mark: true });
  const words = calls.filter((c) => c.op === "fillText").map((c) => c.args[0]);
  assert.ok(words.includes("Les Pléiades"));
  assert.ok(words.includes("M45 · Nuit du 24 octobre 2026"));
  assert.ok(words.some((w) => String(w).includes("3 h 35 min")));
  assert.ok(words.includes("Astro Sync"));
  assert.ok(words.includes("planastro.space"));
});

test("the data without the mark leaves the mark out, the mark alone leaves the data out", () => {
  const dataOnly = draw(1080, 1620, { data: true, mark: false })
    .calls.filter((c) => c.op === "fillText")
    .map((c) => c.args[0]);
  assert.ok(!dataOnly.includes("planastro.space"));
  assert.ok(dataOnly.includes("Les Pléiades"));
  const markOnly = draw(1080, 1620, { data: false, mark: true });
  const words = markOnly.calls
    .filter((c) => c.op === "fillText")
    .map((c) => c.args[0]);
  assert.deepEqual(words, ["Astro Sync", "planastro.space"]);
  assert.equal(markOnly.layout.kind, "strip");
});

test("the text keeps clear of the other side and inside the padding, at every width", () => {
  for (const width of [600, 1080, 2160, 4000]) {
    for (const name of [
      INPUT.name,
      "Une très longue désignation d’usage ".repeat(6),
    ]) {
      const { layout, calls } = draw(
        width,
        Math.round(width * 1.5),
        { data: true, mark: true },
        { ...INPUT, name },
      );
      const geo = bandGeometry(layout.bandHeight);
      const texts = calls.filter((c) => c.op === "fillText");
      const lefts = texts.filter(
        (c) => Math.abs((c.args[4] as number) - geo.pad) < 0.01,
      );
      const rights = texts.filter((c) => !lefts.includes(c));
      assert.ok(lefts.length > 0 && rights.length > 0, String(width));
      const leftEnd = Math.max(...lefts.map((c) => c.args[5] as number));
      const rightStart = Math.min(...rights.map((c) => c.args[4] as number));
      assert.ok(
        leftEnd < rightStart,
        `${width}: ${leftEnd} against ${rightStart}`,
      );
      for (const c of texts) {
        assert.ok(
          (c.args[4] as number) >= geo.pad - 0.5,
          `${width}: starts at ${c.args[4]}`,
        );
        assert.ok(
          (c.args[5] as number) <= width - geo.pad + 0.5,
          `${width}: ends at ${c.args[5]}`,
        );
      }
    }
  }
});

test("a long name is cut with an ellipsis before it reaches the right column", () => {
  const { layout, calls } = draw(
    600,
    900,
    { data: true, mark: true },
    {
      ...INPUT,
      name: "Une très longue désignation d’usage ".repeat(8),
    },
  );
  const left = calls.find(
    (c) =>
      c.op === "fillText" &&
      Math.abs((c.args[4] as number) - bandGeometry(layout.bandHeight).pad) <
        0.01 &&
      String(c.args[0]).startsWith("Une"),
  );
  assert.ok(left, "the name is drawn");
  assert.ok(String(left.args[0]).endsWith("…"), String(left.args[0]));
});
