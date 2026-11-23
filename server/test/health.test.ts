import { expect, spyOn, test } from "bun:test";
import { createServer } from "../src/app.ts";
import { loadConfig, testConfig } from "../src/config.ts";
import { log } from "../src/log.ts";

test("healthz is ok", async () => {
  const s = createServer({ cfg: testConfig(), sql: null as never, port: 0 });
  const r = await fetch(`http://localhost:${s.port}/healthz`);
  expect(r.status).toBe(200);
  expect(await r.json()).toEqual({ ok: true });
  s.stop(true);
});

test("unknown route is a JSON 404", async () => {
  const s = createServer({ cfg: testConfig(), sql: null as never, port: 0 });
  const r = await fetch(`http://localhost:${s.port}/nope`);
  expect(r.status).toBe(404);
  expect(await r.json()).toEqual({ error: "not_found" });
  s.stop(true);
});

test("config requires DATABASE_URL", () => {
  expect(() => loadConfig({})).toThrow("DATABASE_URL is required");
  expect(() => loadConfig({ DATABASE_URL: "x" })).toThrow("PUBLIC_BASE_URL is required");
  expect(() => loadConfig({ DATABASE_URL: "x", NODE_ENV: "test" })).toThrow(
    "PUBLIC_BASE_URL is required",
  );
  const all = {
    DATABASE_URL: "x",
    PUBLIC_BASE_URL: "https://tally.example/",
    HCA_CLIENT_ID: "a",
    HCA_CLIENT_SECRET: "b",
    GITHUB_APP_SLUG: "c",
    GITHUB_APP_CLIENT_ID: "d",
    GITHUB_APP_CLIENT_SECRET: "e",
    GITHUB_WEBHOOK_SECRET: "f",
  };
  expect(loadConfig(all)).toMatchObject({ port: 8080, publicBaseUrl: "https://tally.example" });
  expect(() => loadConfig({ ...all, GITHUB_WEBHOOK_SECRET: "" })).toThrow(
    "GITHUB_WEBHOOK_SECRET is required",
  );
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
