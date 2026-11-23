import { afterAll, expect, test } from "bun:test";
import { createServer } from "../src/app.ts";
import { testConfig } from "../src/config.ts";
import { connect } from "../src/db.ts";
import { createLimiter } from "../src/ratelimit.ts";
import { seedFixture } from "./helpers.ts";

const sql = connect(process.env.DATABASE_URL ?? "");
afterAll(() => sql.close());

test("bucket allows burst then refuses, then refills", () => {
  let t = 0;
  const l = createLimiter(60, 3, () => t);
  expect([l.take("a"), l.take("a"), l.take("a"), l.take("a")]).toEqual([true, true, true, false]);
  t += 1_000;
  expect(l.take("a")).toBe(true);
  expect(l.take("b")).toBe(true); // keys are independent
});

test("HTTP 429 after the burst", async () => {
  const p = await seedFixture(sql, "acme/demo-10");
  const s = createServer({ cfg: testConfig(), sql, port: 0, limiter: createLimiter(60, 2) });
  try {
    const get = () =>
      fetch(`http://localhost:${s.port}/v1/ledger/${p.projectId}/head`, {
        headers: { authorization: `Bearer ${p.token}` },
      });
    expect([(await get()).status, (await get()).status]).toEqual([200, 200]);
    const r = await get();
    expect(r.status).toBe(429);
    expect(r.headers.get("retry-after")).toBe("60");
    expect(await r.json()).toEqual({ error: "rate_limited" });
  } finally {
    s.stop(true);
  }
});

test("failed auth is limited by ip", async () => {
  const p = await seedFixture(sql, "acme/demo-11");
  const s = createServer({ cfg: testConfig(), sql, port: 0, limiter: createLimiter(60, 2) });
  try {
    const bad = async () =>
      (await fetch(`http://localhost:${s.port}/v1/ledger/${p.projectId}/head`)).status;
    expect([await bad(), await bad(), await bad()]).toEqual([401, 401, 429]);
  } finally {
    s.stop(true);
  }
});

test("request log has route pattern and no secrets", async () => {
  const p = await seedFixture(sql, "acme/demo-12");
  const s = createServer({ cfg: testConfig(), sql, port: 0 });
  const lines: string[] = [];
  const orig = process.stdout.write;
  process.stdout.write = ((c: string) => {
    lines.push(String(c));
    return true;
  }) as typeof process.stdout.write;
  try {
    await fetch(`http://localhost:${s.port}/v1/ledger/${p.projectId}/head?ruleset=x`, {
      headers: { authorization: `Bearer ${p.token}` },
    });
  } finally {
    process.stdout.write = orig;
    s.stop(true);
  }
  const all = lines.join("");
  const l = lines.map((x) => JSON.parse(x)).find((x) => x.msg === "request");
  expect(l).toMatchObject({
    level: "info",
    method: "GET",
    route: "/v1/ledger/:project/head",
    status: 200,
    project: p.projectId,
  });
  expect(typeof l.ms).toBe("number");
  expect(all).not.toContain(p.token);
  expect(all).not.toContain("ruleset=x");
});
