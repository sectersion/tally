import { afterAll, beforeAll, expect, test } from "bun:test";
import { hashToken } from "../src/auth.ts";
import { connect } from "../src/db.ts";
import { HttpError } from "../src/errors.ts";
import { setHcaExchangeForTests } from "../src/hca.ts";
import { requireRole } from "../src/session.ts";
import { start } from "./helpers.ts";

const sql = connect(process.env.DATABASE_URL ?? "");
const srv = start(sql);
const ORIGIN = "http://localhost:8080"; // PUBLIC_BASE_URL under NODE_ENV=test
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

/** Runs /auth/login then /auth/callback; returns the callback response and the login state. */
async function signIn(returnTo = "/") {
  const login = await fetch(
    `${srv.url}/auth/login?return_to=${encodeURIComponent(returnTo)}`,
    noFollow,
  );
  expect(login.status).toBe(303);
  const state = new URL(login.headers.get("location") as string).searchParams.get(
    "state",
  ) as string;
  const cb = await fetch(`${srv.url}/auth/callback?code=c1&state=${state}`, noFollow);
  return { cb, state };
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
  const r = await fetch(`${srv.url}/auth/callback?code=c&state=nope`, noFollow);
  expect(r.status).toBe(400);
  expect(await n()).toBe(before);
  expect(before).toBe(0); // first test: nothing has signed in yet
});

test("a state works once", async () => {
  const { cb, state } = await signIn();
  expect(cb.status).toBe(303);
  const again = await fetch(`${srv.url}/auth/callback?code=c1&state=${state}`, noFollow);
  expect(again.status).toBe(400);
});

test("callback redirects to return_to and upserts the user", async () => {
  const { cb } = await signIn("/settings");
  expect(cb.headers.get("location")).toBe("/settings");
  expect((await sql`select 1 from users where hca_sub = ${sub}`).length).toBe(1);
});

test("return_to cannot leave the site", async () => {
  for (const bad of ["//evil.example", "https://evil", "/\\evil"]) {
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
