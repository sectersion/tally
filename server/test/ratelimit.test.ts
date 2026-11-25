import { afterAll, expect, test } from "bun:test";
import { createServer } from "../src/app.ts";
import { testConfig } from "../src/config.ts";
import { connect } from "../src/db.ts";
import {
  clientIp,
  createLimiter,
  ipKey,
  LIMITS,
  type Limiter,
  type Limits,
} from "../src/ratelimit.ts";
import { createSession } from "../src/session.ts";
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
  const s = createServer({ cfg: testConfig(), sql, port: 0, limiter: createLimiter(1, 2) });
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
  const s = createServer({ cfg: testConfig(), sql, port: 0, limiter: createLimiter(1, 2) });
  try {
    const bad = async () =>
      (await fetch(`http://localhost:${s.port}/v1/ledger/${p.projectId}/head`)).status;
    expect([await bad(), await bad(), await bad()]).toEqual([401, 401, 429]);
  } finally {
    s.stop(true);
  }
});

test("cross-project 403 is limited by ip", async () => {
  const a = await seedFixture(sql, "acme/demo-13a");
  const b = await seedFixture(sql, "acme/demo-13b");
  const s = createServer({ cfg: testConfig(), sql, port: 0, limiter: createLimiter(1, 2) });
  try {
    const bad = async () =>
      (
        await fetch(`http://localhost:${s.port}/v1/ledger/${b.projectId}/head`, {
          headers: { authorization: `Bearer ${a.token}` },
        })
      ).status;
    expect([await bad(), await bad(), await bad()]).toEqual([403, 403, 429]);
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

// --- the key table, client ip, eviction ---

type Opts = { limits?: Partial<Limits>; limiter?: Limiter; trustProxy?: boolean };
async function withServer(o: Opts, f: (url: string) => Promise<void>) {
  const s = createServer({
    cfg: testConfig({ trustProxy: o.trustProxy ?? false }),
    sql,
    port: 0,
    limits: o.limits,
    limiter: o.limiter,
  });
  try {
    await f(`http://localhost:${s.port}`);
  } finally {
    s.stop(true);
  }
}
const manual = { redirect: "manual" } as const;
/** n requests in order; returns the statuses and the last response (unread). */
async function hammer(n: number, go: () => Promise<Response>) {
  const statuses: number[] = [];
  let last = new Response();
  for (let i = 0; i < n; i++) {
    last = await go();
    statuses.push(last.status);
    if (i < n - 1) await last.arrayBuffer();
  }
  return { statuses, last };
}
const expectHtml429 = async (r: Response) => {
  expect(r.status).toBe(429);
  expect(r.headers.get("retry-after")).toBe("60");
  expect(r.headers.get("content-type")).toBe("text/html; charset=utf-8");
  expect(r.headers.get("cache-control")).toBe("no-store");
  expect(await r.text()).toContain("Too many requests");
};

test("the key table is the brief's", () => {
  expect(LIMITS).toEqual({
    auth: { perMin: 20, burst: 20 },
    device: { perMin: 10, burst: 10 },
    form: { perMin: 60, burst: 60 },
    page: { perMin: 120, burst: 120 },
    badge: { perMin: 600, burst: 600 },
  });
});

// HTTP tests keep the real burst but refill 1/min, so no token comes back mid-test on a slow machine.
const slow = (k: keyof Limits): Partial<Limits> => ({ [k]: { perMin: 1, burst: LIMITS[k].burst } });

test("auth:<ip> is a burst of 20 across login, callback and the GitHub flow", async () => {
  await withServer({ limits: slow("auth") }, async (url) => {
    const { statuses, last } = await hammer(21, () => fetch(`${url}/auth/login`, manual));
    expect(statuses.slice(0, 20).every((c) => c === 303)).toBe(true);
    await expectHtml429(last);
    await expectHtml429(await fetch(`${url}/auth/callback?code=a&state=b`, manual));
    await expectHtml429(await fetch(`${url}/github/connect`, manual));
    await expectHtml429(await fetch(`${url}/github/callback?code=a&state=b`, manual));
  });
});

test("form:<user_id> is a burst of 60 over session POSTs, per user", async () => {
  const mkUser = async () => {
    const id = crypto.randomUUID();
    await sql`insert into users (id, hca_sub) values (${id}, ${`sub!${id}`})`;
    const { setCookie } = await createSession(sql, testConfig(), id);
    return setCookie.split(";")[0] as string;
  };
  const a = await mkUser();
  const b = await mkUser();
  await withServer({ limits: slow("form") }, async (url) => {
    const post = (cookie: string, path = "/settings/handle") =>
      fetch(`${url}${path}`, { method: "POST", redirect: "manual", headers: { cookie } });
    const { statuses, last } = await hammer(61, () => post(a));
    expect(statuses.slice(0, 60).every((c) => c === 403)).toBe(true); // csrf fails, but counts
    await expectHtml429(last);
    await expectHtml429(await post(a, "/projects")); // one bucket for every session POST
    expect((await post(b)).status).toBe(403); // another user is unaffected
  });
});

test("page:<ip> covers /, /u, /p, fragments and events", async () => {
  await withServer({ limits: { page: { perMin: 1, burst: 3 } } }, async (url) => {
    const { statuses, last } = await hammer(4, () => fetch(`${url}/u/nobody`));
    expect(statuses).toEqual([404, 404, 404, 429]);
    await expectHtml429(last);
    for (const p of ["/p/a/b", "/p/a/b/fragment", "/p/a/b/events"])
      expect((await fetch(`${url}${p}`)).status).toBe(429);
    expect((await fetch(`${url}/`)).status).toBe(429);
  });
});

test("badge:<ip> covers /v1/projects/:id/badge.json with the /v1 error body", async () => {
  await withServer({ limits: { badge: { perMin: 1, burst: 2 } } }, async (url) => {
    const { statuses } = await hammer(2, () => fetch(`${url}/badge/x/y.svg`));
    expect(statuses).toEqual([404, 404]);
    const r = await fetch(`${url}/v1/projects/nope/badge.json`);
    expect(r.status).toBe(429);
    expect(r.headers.get("retry-after")).toBe("60");
    expect(await r.json()).toEqual({ error: "rate_limited" });
  });
});

test("webhooks are never limited: unsigned ones stay 401, not 429", async () => {
  const one = { perMin: 1, burst: 1 };
  const limits = { auth: one, device: one, form: one, page: one, badge: one };
  await withServer({ limits, limiter: createLimiter(1, 1) }, async (url) => {
    const { statuses } = await hammer(3, () =>
      fetch(`${url}/github/webhook`, { method: "POST", body: "{}" }),
    );
    expect(statuses).toEqual([401, 401, 401]);
  });
});

const peer = (address: string) => ({
  requestIP: () => ({ address, family: "IPv4" as const, port: 1 }),
});
const xff = (v: string) => new Request("http://x/", { headers: { "x-forwarded-for": v } });

test("clientIp ignores x-forwarded-for unless TRUST_PROXY=1, then takes the last hop", () => {
  const s = peer("10.0.0.1");
  expect(clientIp(xff("1.2.3.4"), s, { trustProxy: false })).toBe("10.0.0.1");
  expect(clientIp(new Request("http://x/"), s, { trustProxy: true })).toBe("10.0.0.1");
  expect(clientIp(xff("6.6.6.6, 1.2.3.4"), s, { trustProxy: true })).toBe("1.2.3.4");
  expect(clientIp(xff("6.6.6.6,1.2.3.4 "), s, { trustProxy: true })).toBe("1.2.3.4");
});

test("clientIp falls back to the peer on a malformed last hop; IPv6 works", () => {
  const s = peer("::1");
  const loop = "0:0:0:0::/64";
  for (const bad of ["", "1.2.3.4, ", "1.2.3.4, nope", "1.2.3.4, 1.2.3.4:80", "999.1.1.1"])
    expect(clientIp(xff(bad), s, { trustProxy: true })).toBe(loop);
  expect(clientIp(xff("1.2.3.4, 2001:db8::7"), s, { trustProxy: true })).toBe("2001:db8:0:0::/64");
  expect(clientIp(new Request("http://x/"), s, { trustProxy: false })).toBe(loop);
});

test("IPv6 is keyed by /64; IPv4-mapped addresses key as IPv4", () => {
  const k = ipKey;
  expect(k("2001:db8:1:2::a")).toBe(k("2001:0DB8:1:2:ffff:ffff:ffff:ffff")); // one /64
  expect(k("2001:db8:1:2:3:4:5:6")).toBe("2001:db8:1:2::/64");
  expect(k("2001:db8:1:2::a")).not.toBe(k("2001:db8:1:3::a")); // the next /64
  expect(k("fe80::1%eth0")).toBe("fe80:0:0:0::/64");
  expect(k("::ffff:1.2.3.4")).toBe("1.2.3.4");
  expect(k("::ffff:102:304")).toBe("1.2.3.4");
  expect(k("1.2.3.4")).toBe("1.2.3.4");
  expect(k("64:ff9b::1.2.3.4")).toBe("64:ff9b:0:0::/64");
  // Two hosts in one /64 share a bucket through the route too.
  const s = peer("10.0.0.1");
  const a = clientIp(xff("2001:db8:1:2::a"), s, { trustProxy: true });
  const b = clientIp(xff("2001:db8:1:2::b"), s, { trustProxy: true });
  expect(a).toBe(b);
});

test("routes key by the trusted last hop only when TRUST_PROXY=1", async () => {
  const page = { perMin: 1, burst: 1 };
  const get = (url: string, hops: string) =>
    fetch(`${url}/u/nobody`, { headers: { "x-forwarded-for": hops } });
  await withServer({ limits: { page }, trustProxy: true }, async (url) => {
    expect((await get(url, "1.1.1.1, 9.9.9.9")).status).toBe(404);
    expect((await get(url, "2.2.2.2, 9.9.9.9")).status).toBe(429); // a spoofed first hop changes nothing
    expect((await get(url, "9.9.9.9, 8.8.8.8")).status).toBe(404);
  });
  await withServer({ limits: { page } }, async (url) => {
    expect((await get(url, "1.1.1.1")).status).toBe(404);
    expect((await get(url, "2.2.2.2")).status).toBe(429); // the header is ignored
  });
});

test("full buckets are evicted, so distinct keys cannot grow the map without bound", () => {
  let t = 0;
  const l = createLimiter(60, 3, () => t);
  for (let i = 0; i < 3; i++) l.take("busy"); // empty
  for (let i = 0; i < 996; i++) l.take(`k${i}`); // 999 calls so far
  expect(l.size()).toBe(997);
  t += 1_000; // one token back: every k bucket is full again, "busy" is not
  l.take("fresh"); // the 1000th call sweeps
  expect(l.size()).toBe(2);
  expect(l.take("busy")).toBe(true);
  expect(l.take("busy")).toBe(false); // its state survived the sweep
});

test("take(key, rate) uses the per-call rate", () => {
  let t = 0;
  const l = createLimiter(60, 3, () => t);
  const r = { perMin: 60, burst: 1 };
  expect([l.take("a", r), l.take("a", r)]).toEqual([true, false]);
  t += 1_000;
  expect(l.take("a", r)).toBe(true);
});
