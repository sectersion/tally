import { expect, spyOn, test } from "bun:test";
import { createServer } from "../src/app.ts";
import { loadConfig } from "../src/config.ts";
import { log } from "../src/log.ts";

test("healthz is ok", async () => {
  const s = createServer({ sql: null as never, port: 0 });
  const r = await fetch(`http://localhost:${s.port}/healthz`);
  expect(r.status).toBe(200);
  expect(await r.json()).toEqual({ ok: true });
  s.stop(true);
});

test("unknown route is a JSON 404", async () => {
  const s = createServer({ sql: null as never, port: 0 });
  const r = await fetch(`http://localhost:${s.port}/nope`);
  expect(r.status).toBe(404);
  expect(await r.json()).toEqual({ error: "not_found" });
  s.stop(true);
});

test("config requires DATABASE_URL", () => {
  expect(() => loadConfig({})).toThrow("DATABASE_URL is required");
  expect(() => loadConfig({ DATABASE_URL: "x" })).toThrow("PUBLIC_BASE_URL is required");
  expect(loadConfig({ DATABASE_URL: "x", NODE_ENV: "test" })).toMatchObject({
    port: 8080,
    databaseUrl: "x",
    logLevel: "info",
    publicBaseUrl: "http://localhost:8080",
  });
  expect(() => loadConfig({ DATABASE_URL: "x", LOG_LEVEL: "loud" })).toThrow();
});

test("log redacts secrets", () => {
  const spy = spyOn(process.stdout, "write").mockImplementation(() => true);
  log("info", "x", { Authorization: "Bearer abc", ok: 1 });
  const out = String(spy.mock.calls[0]?.[0]);
  spy.mockRestore();
  expect(out).toContain("[redacted]");
  expect(out).not.toContain("abc");
  expect(JSON.parse(out).ok).toBe(1);
});
