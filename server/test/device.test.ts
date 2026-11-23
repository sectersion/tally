import { afterAll, expect, test } from "bun:test";
import { createServer } from "../src/app.ts";
import { hashToken } from "../src/auth.ts";
import { testConfig } from "../src/config.ts";
import { connect } from "../src/db.ts";
import { createProject } from "../src/projects.ts";
import type { Limits } from "../src/ratelimit.ts";
import { createSession } from "../src/session.ts";
import { seed } from "./helpers.ts";

const sql = connect(process.env.DATABASE_URL ?? "");
const mk = (limits?: Partial<Limits>) => {
  const s = createServer({ cfg: testConfig(), sql, port: 0, limits });
  return { url: `http://localhost:${s.port}`, stop: () => s.stop(true) };
};
// The tests make many codes and form posts; the limits have their own tests.
const srv = mk({ device: { perMin: 6000, burst: 1000 }, form: { perMin: 6000, burst: 1000 } });
const ORIGIN = "http://localhost:8080";
afterAll(async () => {
  srv.stop();
  await sql.close();
});

const h = () => `u${crypto.randomUUID().replaceAll("-", "").slice(0, 12)}`;
async function user() {
  const id = crypto.randomUUID();
  const handle = h();
  await sql`insert into users (id, hca_sub, handle) values (${id}, ${`sub!${id}`}, ${handle})`;
  return { id, handle };
}
type Json = { status: number; [k: string]: string | number | undefined };
const post = async (path: string, body: unknown): Promise<Json> => {
  const r = await fetch(`${srv.url}${path}`, { method: "POST", body: JSON.stringify(body) });
  return { ...((await r.json()) as Record<string, string>), status: r.status };
};
const code = (device_name = "lap") => post("/v1/device/code", { device_name });
const poll = (device_code: string) => post("/v1/device/token", { device_code });
const rewindPoll = (device_code: string) =>
  sql`update device_codes set last_poll_at = now() - interval '6 s' where code_hash = ${hashToken(device_code)}`;
async function decideAs(
  u: { id: string },
  userCode: string,
  decision = "approve",
  o: { csrf?: boolean; origin?: string } = {},
) {
  const { setCookie, csrf } = await createSession(sql, testConfig(), u.id);
  const body = new URLSearchParams({ user_code: userCode, decision });
  if (o.csrf !== false) body.set("csrf", csrf);
  return fetch(`${srv.url}/device`, {
    method: "POST",
    headers: { cookie: setCookie.split(";")[0] as string, origin: o.origin ?? ORIGIN },
    body,
  });
}
const approveAs = (u: { id: string }, userCode: string) => decideAs(u, userCode);
const head = (project: string, token: string) =>
  fetch(`${srv.url}/v1/ledger/${project}/head`, { headers: { authorization: `Bearer ${token}` } });

test("code, pending, approve, token exactly once", async () => {
  const c = await code();
  expect(c.user_code).toMatch(/^[BCDFGHJKLMNPQRSTVWXZ]{4}-[BCDFGHJKLMNPQRSTVWXZ]{4}$/);
  expect(c.verification_uri_complete).toBe(`${ORIGIN}/device?code=${c.user_code}`);
  expect((await poll(c.device_code as string)).error).toBe("authorization_pending");
  const u = await user();
  expect((await approveAs(u, c.user_code as string)).status).toBe(200);
  await rewindPoll(c.device_code as string);
  const t = await poll(c.device_code as string);
  expect(t.token_type).toBe("Bearer");
  await rewindPoll(c.device_code as string);
  expect((await poll(c.device_code as string)).error).toBe("expired_token");
});

test("a user token uploads to the user's projects only", async () => {
  const u = await user();
  const other = await user();
  const mine = await createProject(sql, u.id, `${u.handle}/a`);
  const theirs = await createProject(sql, other.id, `${other.handle}/a`);
  if (!("id" in mine) || !("id" in theirs)) throw new Error("setup");
  const c = await code();
  await approveAs(u, c.user_code as string);
  await rewindPoll(c.device_code as string);
  const t = (await poll(c.device_code as string)).access_token as string;
  expect((await head(mine.id, t)).status).toBe(200);
  expect((await head(theirs.id, t)).status).toBe(403);
  expect((await head(crypto.randomUUID(), t)).status).toBe(403);
});

test("plan 06 project tokens still work", async () => {
  const s = await seed(sql);
  expect((await head(s.projectId, s.token)).status).toBe(200);
  expect((await head((await seed(sql)).projectId, s.token)).status).toBe(403);
});

test("polling within 5 s is slow_down", async () => {
  const c = await code();
  expect((await poll(c.device_code as string)).error).toBe("authorization_pending");
  expect((await poll(c.device_code as string)).error).toBe("slow_down");
});

test("approval without csrf or origin is 403 and the code stays pending", async () => {
  const u = await user();
  const c = await code();
  expect((await decideAs(u, c.user_code as string, "approve", { csrf: false })).status).toBe(403);
  expect(
    (await decideAs(u, c.user_code as string, "approve", { origin: "http://evil.test" })).status,
  ).toBe(403);
  const [r] =
    await sql`select user_id, denied from device_codes where user_code = ${c.user_code as string}`;
  expect(r.user_id).toBeNull();
  expect(r.denied).toBe(false);
});

test("a code is approved once; expired codes cannot be approved", async () => {
  const u = await user();
  const c = await code();
  expect((await approveAs(u, c.user_code as string)).status).toBe(200);
  expect((await approveAs(await user(), c.user_code as string)).status).toBe(400);
  const e = await code();
  await sql`update device_codes set expires_at = now() - interval '1 s' where user_code = ${e.user_code as string}`;
  const r = await approveAs(u, e.user_code as string);
  expect(r.status).toBe(400);
  expect(await r.text()).toContain("This code is no longer valid");
});

test("deny gives access_denied", async () => {
  const c = await code();
  expect((await decideAs(await user(), c.user_code as string, "deny")).status).toBe(200);
  await rewindPoll(c.device_code as string);
  expect((await poll(c.device_code as string)).error).toBe("access_denied");
  await rewindPoll(c.device_code as string);
  expect((await poll(c.device_code as string)).error).toBe("expired_token");
});

test("only hashes are stored", async () => {
  const c = await code();
  await approveAs(await user(), c.user_code as string);
  await rewindPoll(c.device_code as string);
  const t = (await poll(c.device_code as string)).access_token as string;
  const [row] =
    await sql`select count(*)::int as n from device_tokens where token_hash = ${hashToken(t)}`;
  expect(row.n).toBe(1);
  const [raw] = await sql`select count(*)::int as n from device_tokens where token_hash = ${t}`;
  expect(raw.n).toBe(0);
  const [dc] =
    await sql`select count(*)::int as n from device_codes where code_hash = ${c.device_code as string}`;
  expect(dc.n).toBe(0);
});

test("a revoked user token is 401, revoke is csrf-guarded and owner-only", async () => {
  const u = await user();
  const p = await createProject(sql, u.id, `${u.handle}/r`);
  if (!("id" in p)) throw new Error("setup");
  const c = await code();
  await approveAs(u, c.user_code as string);
  await rewindPoll(c.device_code as string);
  const t = (await poll(c.device_code as string)).access_token as string;
  const [tok] = await sql`select id from device_tokens where token_hash = ${hashToken(t)}`;
  const revoke = async (who: { id: string }, csrf = true) => {
    const s = await createSession(sql, testConfig(), who.id);
    return fetch(`${srv.url}/settings/tokens/${tok.id}/revoke`, {
      method: "POST",
      redirect: "manual",
      headers: { cookie: s.setCookie.split(";")[0] as string, origin: ORIGIN },
      body: new URLSearchParams(csrf ? { csrf: s.csrf } : {}),
    });
  };
  expect((await revoke(u, false)).status).toBe(403);
  expect((await revoke(await user())).status).toBe(404);
  expect((await head(p.id, t)).status).toBe(200);
  expect((await revoke(u)).status).toBe(303);
  expect((await head(p.id, t)).status).toBe(401);
});

test("approval page escapes the device name", async () => {
  const c = await code(`<script>"x"</script>`);
  const u = await user();
  const { setCookie } = await createSession(sql, testConfig(), u.id);
  const r = await fetch(`${srv.url}/device?code=${c.user_code}`, {
    headers: { cookie: setCookie.split(";")[0] as string },
  });
  const html = await r.text();
  expect(html).not.toContain("<script>");
  expect(html).toContain("&lt;script&gt;");
});

test("/device without a code shows the entry form; lowercase hyphenless codes are accepted", async () => {
  const u = await user();
  const { setCookie } = await createSession(sql, testConfig(), u.id);
  const cookie = setCookie.split(";")[0] as string;
  const form = await fetch(`${srv.url}/device`, { headers: { cookie } });
  expect(form.status).toBe(200);
  const t = await form.text();
  expect(t).toContain('<form method="get" action="/device">');
  expect(t).toContain('name="code"');
  const c = await code();
  const loose = (c.user_code as string).replace("-", "").toLowerCase();
  const shown = await (
    await fetch(`${srv.url}/device?code=${loose}`, { headers: { cookie } })
  ).text();
  expect(shown).toContain(`<strong>${c.user_code}</strong>`);
  expect((await decideAs(u, ` ${loose} `)).status).toBe(200);
  const [r] =
    await sql`select user_id from device_codes where user_code = ${c.user_code as string}`;
  expect(r.user_id).toBe(u.id);
});

test("device names are 1..64 chars and the body is strict", async () => {
  expect((await post("/v1/device/code", { device_name: "" })).status).toBe(400);
  expect((await post("/v1/device/code", { device_name: "x".repeat(65) })).status).toBe(400);
  expect((await post("/v1/device/code", { device_name: "a", extra: 1 })).status).toBe(400);
});

test("/v1/device/code is limited to 10 per minute per ip", async () => {
  const s = mk();
  try {
    const statuses: number[] = [];
    let r = new Response();
    for (let i = 0; i < 11; i++) {
      r = await fetch(`${s.url}/v1/device/code`, {
        method: "POST",
        body: '{"device_name":"x"}',
      });
      statuses.push(r.status);
    }
    expect(statuses.slice(0, 10).every((c) => c === 200)).toBe(true);
    expect(statuses[10]).toBe(429);
    expect(r.headers.get("retry-after")).toBe("60");
    expect(await r.json()).toEqual({ error: "rate_limited" });
  } finally {
    s.stop();
  }
});
