import { expect, test } from "bun:test";
import { Throttle } from "../src/throttle";
import { isTrackable } from "../src/payload";

test("isTrackable accepts only saved file documents", () => {
  expect(isTrackable({ scheme: "file", fsPath: "C:\\My Code\\é.rs", isUntitled: false })).toBe(true);
  for (const scheme of ["untitled", "vscode-remote", "output", "vscode-userdata"])
    expect(isTrackable({ scheme, fsPath: "x", isUntitled: false })).toBe(false);
  expect(isTrackable({ scheme: "file", fsPath: "x", isUntitled: true })).toBe(false);
});

test("Throttle sends first, writes, and after the window", () => {
  const t = new Throttle(120000);
  expect(t.shouldSend("a", false, 0)).toBe(true);
  expect(t.shouldSend("a", false, 1000)).toBe(false);
  expect(t.shouldSend("a", true, 2000)).toBe(true);
  expect(t.shouldSend("a", false, 2000 + 120000)).toBe(true);
  expect(t.shouldSend("b", false, 3000)).toBe(true);
});

test("10000 calls in one window send once", () => {
  const t = new Throttle(120000);
  let n = 0;
  for (let i = 0; i < 10000; i++) if (t.shouldSend("a", false, i)) n++;
  expect(n).toBe(1);
});
