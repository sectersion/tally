import { afterAll, beforeAll, expect, test } from "bun:test";
import { hashToken } from "../src/auth.ts";
import { connect } from "../src/db.ts";
import { HttpError } from "../src/errors.ts";
import { setHcaExchangeForTests } from "../src/hca.ts";
import { createLimiter } from "../src/ratelimit.ts";
import { requireRole } from "../src/session.ts";
import { start } from "./helpers.ts";

const sql = connect(process.env.DATABASE_URL ?? "");
const wide = { perMin: 6000, burst: 1000 };
// these tests make many auth calls from one ip
const srv = start(sql, createLimiter(6000, 1000), { auth: wide, form: wide });
const ORIGIN = "http://localhost:8080"; // testConfig()'s PUBLIC_BASE_URL
const sub = `ident!${crypto.randomUUID()}`;

beforeAll(() => {
  setHcaExchangeForTests(async () => ({
    sub,
    email: "a@b.c",
    name: "Ann",
    verification_status: "verified",
  }));
});
afterAll(async () => {
  setHcaExchangeForTests(null);
  srv.stop();
  await sql.close();
});

const noFollow = { redirect: "manual" } as const;

async function login(returnTo = "/") {
  const r = await fetch(
    `${srv.url}/auth/login?return_to=${encodeURIComponent(returnTo)}`,
    noFollow,
  );
  expect(r.status).toBe(303);
  const state = new URL(r.headers.get("location") as string).searchParams.get("state") as string;
  return { state, oauth: r.headers.get("set-cookie") as string };
}
const callback = (state: string, cookie?: string) =>
  fetch(`${srv.url}/auth/callback?code=c1&state=${state}`, {
    ...noFollow,
    headers: cookie ? { cookie } : {},
  });
/** Runs /auth/login then /auth/callback with the browser's oauth cookie. */
async function signIn(returnTo = "/") {
  const { state, oauth } = await login(returnTo);
  return { cb: await callback(state, `tally_oauth=${state}`), state, oauth };
}
const cookieOf = (r: Response) => (r.headers.get("set-cookie") ?? "").split(";")[0] as string;
async function session() {
  const { cb } = await signIn();
  const cookie = cookieOf(cb);
  const [row] =
    await sql`select csrf from sessions where token_hash = ${hashToken(cookie.split("=")[1] as string)}`;
  return { cookie, csrf: row.csrf as string, cb };
}
const logout = (cookie: string, form: Record<string, string>, origin?: string) =>
  fetch(`${srv.url}/auth/logout`, {
    method: "POST",
    ...noFollow,
    headers: { cookie, ...(origin ? { origin } : {}) },
    body: new URLSearchParams(form),
  });
const sessionCount = async (cookie: string) =>
  (
    await sql`select 1 from sessions where token_hash = ${hashToken(cookie.split("=")[1] as string)}`
  ).length;

test("unknown state is 400 and creates no user", async () => {
  const n = async () => (await sql`select 1 from users where hca_sub = ${sub}`).length;
  const before = await n();
  const r = await callback("nope", "tally_oauth=nope");
  expect(r.status).toBe(400);
  expect(await n()).toBe(before);
  expect(before).toBe(0); // first test: nothing has signed in yet
});

test("a state works once", async () => {
  const { cb, state } = await signIn();
  expect(cb.status).toBe(303);
  expect((await callback(state, `tally_oauth=${state}`)).status).toBe(400);
});

test("login sets the oauth cookie bound to the state", async () => {
  const { state, oauth } = await login();
  expect(oauth).toContain(`tally_oauth=${state}`);
  for (const f of ["HttpOnly", "SameSite=Lax", "Path=/auth/callback", "Max-Age=600"])
    expect(oauth).toContain(f);
  expect(oauth).not.toContain("Secure"); // localhost
});

test("a correct state without the oauth cookie is 400 and creates no user", async () => {
  const count = async () => Number((await sql`select count(*) as n from users`)[0].n);
  const before = await count();
  const { state } = await login();
  expect((await callback(state)).status).toBe(400);
  expect(await count()).toBe(before);
  // the state was not consumed, so the right browser can still finish
  expect((await callback(state, `tally_oauth=${state}`)).status).toBe(303);
});

test("a wrong oauth cookie is 400", async () => {
  const { state } = await login();
  expect((await callback(state, "tally_oauth=tly_other")).status).toBe(400);
  expect((await callback(state, "tally_oauth=é")).status).toBe(400); // non-ASCII must not 500
});

test("a good callback clears the oauth cookie", async () => {
  const { cb } = await signIn();
  const cookies = cb.headers.getSetCookie();
  const cleared = cookies.find((c) => c.startsWith("tally_oauth=;")) as string;
  expect(cleared).toContain("Max-Age=0");
  expect(cleared).toContain("Path=/auth/callback");
});

test("callback redirects to return_to and upserts the user", async () => {
  const { cb } = await signIn("/settings");
  expect(cb.headers.get("location")).toBe("/settings");
  expect((await sql`select 1 from users where hca_sub = ${sub}`).length).toBe(1);
});

test("return_to cannot leave the site", async () => {
  for (const bad of [
    "//evil.example",
    "https://evil",
    "/\\evil",
    "/\t/evil",
    "/\n/evil",
    "/\r/evil",
  ]) {
    const { cb } = await signIn(bad);
    expect(cb.headers.get("location")).toBe("/");
  }
});

test("cookie flags", async () => {
  const { cb } = await signIn();
  const c = cb.headers.get("set-cookie") as string;
  expect(c).toMatch(/^tally_session=tly_/);
  for (const f of ["HttpOnly", "SameSite=Lax", "Path=/", "Max-Age=2592000"]) expect(c).toContain(f);
  expect(c).not.toContain("Secure"); // PUBLIC_BASE_URL is localhost
});

test("sessions store only the hash", async () => {
  const { cookie } = await session();
  const value = cookie.split("=")[1] as string;
  expect((await sql`select 1 from sessions where token_hash = ${hashToken(value)}`).length).toBe(1);
  expect((await sql`select 1 from sessions where token_hash = ${value}`).length).toBe(0);
});

test("logout without csrf is 403 and keeps the session", async () => {
  const { cookie } = await session();
  expect((await logout(cookie, {}, ORIGIN)).status).toBe(403);
  expect(await sessionCount(cookie)).toBe(1);
});

test("a non-ASCII csrf of the same string length is 403, not 500", async () => {
  const { cookie, csrf } = await session();
  const wrong = "é".repeat(csrf.length);
  expect((await logout(cookie, { csrf: wrong }, ORIGIN)).status).toBe(403);
  expect(await sessionCount(cookie)).toBe(1);
});

test("logout with a foreign Origin is 403 even with the right csrf", async () => {
  const { cookie, csrf } = await session();
  expect((await logout(cookie, { csrf }, "https://evil.example")).status).toBe(403);
  expect((await logout(cookie, { csrf })).status).toBe(403); // no Origin at all
  expect(await sessionCount(cookie)).toBe(1);
});

test("logout with both deletes the session", async () => {
  const { cookie, csrf } = await session();
  const r = await logout(cookie, { csrf }, ORIGIN);
  expect(r.status).toBe(303);
  expect(r.headers.get("location")).toBe("/");
  expect(r.headers.get("set-cookie")).toContain("tally_session=;");
  expect(r.headers.get("set-cookie")).toContain("Max-Age=0");
  expect(await sessionCount(cookie)).toBe(0);
});

test("a valid session reaches /settings; an expired one redirects to login", async () => {
  const { cookie } = await session();
  const ok = await fetch(`${srv.url}/settings`, { headers: { cookie }, ...noFollow });
  expect(ok.status).toBe(200);
  await sql`update sessions set expires_at = now() - interval '1 second' where token_hash = ${hashToken(cookie.split("=")[1] as string)}`;
  const r = await fetch(`${srv.url}/settings`, { headers: { cookie }, ...noFollow });
  expect(r.status).toBe(303);
  expect(r.headers.get("location")).toBe("/auth/login?return_to=%2Fsettings");
});

test("a session cookie does not authorize POST /v1/ledger/:id/entries", async () => {
  const { cookie } = await session();
  const r = await fetch(`${srv.url}/v1/ledger/${crypto.randomUUID()}/entries`, {
    method: "POST",
    headers: { cookie, "content-type": "application/json" },
    body: "{}",
  });
  expect(r.status).toBe(401);
});

test("requireRole rejects user and admits admin", () => {
  const status = (role: string, need: "reviewer" | "admin") => {
    try {
      requireRole({ role }, need);
      return 200;
    } catch (e) {
      return e instanceof HttpError ? e.status : 500;
    }
  };
  expect(status("user", "reviewer")).toBe(404);
  expect(status("reviewer", "admin")).toBe(404);
  expect(status("reviewer", "reviewer")).toBe(200);
  expect(status("admin", "reviewer")).toBe(200);
  expect(status("admin", "admin")).toBe(200);
});

test("creating a session deletes expired ones", async () => {
  const { createSession } = await import("../src/session.ts");
  const { testConfig } = await import("../src/config.ts");
  const id = crypto.randomUUID();
  await sql`insert into users (id, hca_sub) values (${id}, ${`sub!${id}`})`;
  await sql`insert into sessions (token_hash, user_id, csrf, expires_at)
    values (${`old!${id}`}, ${id}, 'c', now() - interval '1 second'),
      (${`live!${id}`}, ${id}, 'c', now() + interval '1 hour')`;
  await createSession(sql, testConfig(), id);
  const left =
    await sql`select token_hash from sessions where user_id = ${id} and token_hash like '%!%'`;
  expect(left.map((r: { token_hash: string }) => r.token_hash)).toEqual([`live!${id}`]);
});

test("a github-connect state is not accepted by /auth/callback", async () => {
  const state = `tly_${crypto.randomUUID()}`;
  await sql`insert into oauth_states (state_hash, verifier, nonce, return_to, expires_at)
    values (${hashToken(state)}, 'v', 'gh:someuser', '/', now() + interval '10 minutes')`;
  expect((await callback(state, `tally_oauth=${state}`)).status).toBe(400);
});
