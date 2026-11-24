import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";

// happy-dom has no layout, so assert the rule that stops a long unbroken project name widening the page (QA R2F-3).
test("body wraps long unbroken text so it cannot widen the page", () => {
  const css = readFileSync(new URL("./styles.css", import.meta.url), "utf8");
  const body = css.match(/^body\s*\{([^}]*)\}/m)?.[1] ?? "";
  expect(body).toContain("overflow-wrap: anywhere");
});

test("forced-colors repaints SVG ink so chart labels do not vanish on the forced black canvas", () => {
  const css = readFileSync(new URL("./styles.css", import.meta.url), "utf8");
  expect(css).toMatch(/@media \(forced-colors: active\)[^]*svg text\.dow \{ fill: CanvasText/);
});
