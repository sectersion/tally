import { expect, spyOn, test } from "bun:test";
import { createServer } from "../src/app.ts";
import { loadConfig, testConfig } from "../src/config.ts";
import { log, setLogLevel } from "../src/log.ts";

test("LOG_LEVEL filters lines below it", () => {
  const spy = spyOn(process.stdout, "write").mockImplementation(() => true);
  setLogLevel("warn");
  log("info", "quiet");
  log("debug", "quiet");
  log("warn", "loud");
  log("error", "loud");
  setLogLevel("info");
  const n = spy.mock.calls.length;
  spy.mockRestore();
  expect(n).toBe(2);
});

test("a body over 1 MiB is a JSON 413 even past Bun's old 2 MiB cap", async () => {
  const s = createServer({ cfg: testConfig(), sql: null as never, port: 0 });
  for (const size of [2_000_000, 5_000_000]) {
    const r = await fetch(`http://localhost:${s.port}/v1/device/token`, {
      method: "POST",
      body: "x".repeat(size),
    });
    expect(r.status).toBe(413);
    expect(await r.json()).toEqual({ error: "body_too_large", max: 1_048_576 });
  }
  s.stop(true);
});

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
  expect(loadConfig(all)).toMatchObject({
    port: 8080,
    publicBaseUrl: "https://tally.example",
    trustProxy: false,
  });
  expect(loadConfig({ ...all, TRUST_PROXY: "1" }).trustProxy).toBe(true);
  expect(() => loadConfig({ ...all, TRUST_PROXY: "yes" })).toThrow("invalid TRUST_PROXY");
  expect(() => loadConfig({ ...all, GITHUB_WEBHOOK_SECRET: "" })).toThrow(
    "GITHUB_WEBHOOK_SECRET is required",
  );
  expect(() => loadConfig({ DATABASE_URL: "x", LOG_LEVEL: "loud" })).toThrow();
  for (const bad of [
    "tally.example",
    "https://tally.example/app",
    "https://tally.example?x=1",
    "not a url",
  ])
    expect(() => loadConfig({ ...all, PUBLIC_BASE_URL: bad })).toThrow(
      "PUBLIC_BASE_URL must be an origin like https://tally.example",
    );
  expect(loadConfig({ ...all, PUBLIC_BASE_URL: "http://localhost:8080/" }).publicBaseUrl).toBe(
    "http://localhost:8080",
  );
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

test("NUL in a public path parameter is a 404, not a database error", async () => {
  const s = createServer({ cfg: testConfig(), sql: null as never, port: 0 });
  for (const p of ["/p/%00/a", "/u/%00", "/v1/projects/%00/summary", "/badge/%00/x.svg"]) {
    expect((await fetch(`http://localhost:${s.port}${p}`)).status).toBe(404);
  }
  s.stop(true);
});
