import { expect, test } from "bun:test";
import { heartbeat } from "../src/payload";

const d = { scheme: "file", fsPath: "C:\\r\\a.rs", isUntitled: false };
const o = { isWrite: true, lineno: 3, cursorpos: 7, lines: 99999, language: "rust", nowMs: 1700000000500, version: "0.1.0" };

test("heartbeat has exactly the allowed keys", () => {
  const h = heartbeat(d, o) as Record<string, unknown>;
  expect(Object.keys(h).sort()).toEqual(["cursorpos", "entity", "is_write", "language", "lineno", "lines", "plugin", "time", "type"]);
  expect(h.time).toBe(1700000000.5);
  expect(h.plugin).toBe("tally-vscode/0.1.0");
  expect(h.type).toBe("file");
  expect(h.entity).toBe("C:\\r\\a.rs");
});

test("no key carries document text", () => {
  const h = heartbeat(d, o);
  for (const k of Object.keys(h)) expect(["content", "text", "branch", "project"]).not.toContain(k);
  expect(JSON.stringify(h).length).toBeLessThan(250);
});
