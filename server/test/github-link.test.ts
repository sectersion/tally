import { afterAll, expect, test } from "bun:test";
import { createServer } from "../src/app.ts";
import { testConfig } from "../src/config.ts";
import { connect } from "../src/db.ts";
import { gh } from "../src/github.ts";
import { createProject } from "../src/projects.ts";
import { createSession } from "../src/session.ts";

const sql = connect(process.env.DATABASE_URL ?? "");
const ORIGIN = "http://localhost:8080";

/** Fixed responses by URL path; per-user answers are looked up by bearer token. */
const routes = new Map<string, unknown>();
const userRepos = new Map<string, { inst: number; repos: { id: number; full_name: string }[] }>();
let outbound = 0;
const dead = new Set<string>(); // tokens GitHub answers 401 for
const fetchFn = (async (input: RequestInfo | URL, init?: RequestInit) => {
  outbound++;
  const u = new URL(String(input));
  const tok = new Headers(init?.headers).get("authorization")?.replace("Bearer ", "") ?? "";
  if (dead.has(tok)) return new Response("bad", { status: 401 });
  const a = userRepos.get(tok);
  if (a && u.pathname === "/user/installations")
    return Response.json({ installations: [{ id: a.inst, account: { login: "octo" } }] });
  if (a && u.pathname === `/user/installations/${a.inst}/repositories`)
    return Response.json({ repositories: a.repos });
  const r = routes.get(u.pathname);
  return r === undefined ? new Response("nope", { status: 404 }) : Response.json(r);
}) as typeof fetch;

const srv = createServer({ cfg: testConfig(), sql, port: 0, fetchFn });
const base = `http://localhost:${srv.port}`;
afterAll(async () => {
  srv.stop(true);
  await sql.close();
});

const rnd = () => Math.floor(Math.random() * 1e12);
const h = () => `u${crypto.randomUUID().replaceAll("-", "").slice(0, 12)}`;
async function user(withToken: boolean | "expired" = true) {
  const id = crypto.randomUUID();
  const handle = h();
  await sql`insert into users (id, hca_sub, handle) values (${id}, ${`sub!${id}`}, ${handle})`;
  const ghId = rnd();
  if (withToken)
    await sql`insert into github_user_tokens (user_id, github_user_id, access_token, expires_at)
      values (${id}, ${ghId}, ${`tok-${id}`}, ${withToken === "expired" ? new Date(Date.now() - 1000) : new Date(Date.now() + 3_600_000)})`;
  const { setCookie, csrf } = await createSession(sql, testConfig(), id);
  return { id, handle, ghId, cookie: setCookie.split(";")[0] as string, csrf };
}
type U = Awaited<ReturnType<typeof user>>;
/** Makes GitHub say this user's token reaches these repos (one installation per user). */
const access = (u: U, repos: { id: number; full_name: string }[]) =>
  userRepos.set(`tok-${u.id}`, { inst: rnd(), repos });

const post = (path: string, u: U, body: Record<string, string>, origin = ORIGIN) =>
  fetch(`${base}${path}`, {
    method: "POST",
    redirect: "manual",
    headers: { cookie: u.cookie, origin },
    body: new URLSearchParams({ csrf: u.csrf, ...body }),
  });
async function project(u: U) {
  const r = await createProject(sql, u.id, `${u.handle}/p${rnd()}`);
  if (!("id" in r)) throw new Error("setup");
  return { id: r.id };
}
const settingsPath = async (id: string) =>
  `/p/${(await sql`select public_name from projects where id = ${id}`)[0].public_name}`;
const seq = async (id: string) =>
  Number((await sql`select change_seq from projects where id = ${id}`)[0].change_seq);
const live = async (repoId: number) =>
  (await sql`select project_id from project_repos where github_repo_id = ${repoId} and unlinked_at is null`) as {
    project_id: string;
  }[];

test("gh sets headers, follows Link next, 502 on non-2xx", async () => {
  const calls: { u: string; h: Headers }[] = [];
  const f = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const u = String(input);
    calls.push({ u, h: new Headers(init?.headers) });
    if (u.includes("page=2")) return Response.json([2]);
    if (u.includes("/boom")) return new Response("x", { status: 500 });
    return Response.json([1], {
      headers: { link: '<https://api.github.com/x?page=2>; rel="next"' },
    });
  }) as typeof fetch;
  expect(await gh("t0k", "/x", f)).toEqual([[1], [2]]);
  expect(calls[0]?.h.get("accept")).toBe("application/vnd.github+json");
  expect(calls[0]?.h.get("x-github-api-version")).toBe("2022-11-28");
  expect(calls[0]?.h.get("user-agent")).toBe("tally-server");
  expect(calls[0]?.h.get("authorization")).toBe("Bearer t0k");
  await expect(gh("t0k", "/boom", f)).rejects.toMatchObject({ status: 502 });
});

test("linking a repo the user can access works and bumps change_seq", async () => {
  const u = await user();
  const p = await project(u);
  const repo = { id: rnd(), full_name: `octo/r${rnd()}` };
  access(u, [repo]);
  const before = await seq(p.id);
  const r = await post(`${await settingsPath(p.id)}/link-repo`, u, { repo_id: String(repo.id) });
  expect(r.status).toBe(303);
  expect((await live(repo.id))[0]?.project_id).toBe(p.id);
  expect(await seq(p.id)).toBe(before + 1);
  const [row] =
    await sql`select full_name, installation_id from github_repos where id = ${repo.id}`;
  expect(row.full_name).toBe(repo.full_name);
  expect(row.installation_id).not.toBeNull();
});

test("a repo id not in the user's GitHub list is 403 (form is not trusted)", async () => {
  const u = await user();
  const p = await project(u);
  access(u, [{ id: rnd(), full_name: `octo/mine${rnd()}` }]);
  const other = rnd();
  const r = await post(`${await settingsPath(p.id)}/link-repo`, u, { repo_id: String(other) });
  expect(r.status).toBe(403);
  expect(await live(other)).toEqual([]);
  const [x] = await sql`select count(*)::int as n from github_repos where id = ${other}`;
  expect(x.n).toBe(0);
});

test("second project linking the same repo is 409", async () => {
  const a = await user();
  const b = await user();
  const pa = await project(a);
  const pb = await project(b);
  const repo = { id: rnd(), full_name: `octo/shared${rnd()}` };
  access(a, [repo]);
  access(b, [repo]);
  expect(
    (await post(`${await settingsPath(pa.id)}/link-repo`, a, { repo_id: String(repo.id) })).status,
  ).toBe(303);
  const r = await post(`${await settingsPath(pb.id)}/link-repo`, b, { repo_id: String(repo.id) });
  expect(r.status).toBe(409);
  expect(await r.json()).toMatchObject({ error: "repo already linked to another project" });
  expect((await live(repo.id)).map((x) => x.project_id)).toEqual([pa.id]);
});

test("unlink then link to another project works", async () => {
  const a = await user();
  const b = await user();
  const pa = await project(a);
  const pb = await project(b);
  const repo = { id: rnd(), full_name: `octo/moves${rnd()}` };
  access(a, [repo]);
  access(b, [repo]);
  await post(`${await settingsPath(pa.id)}/link-repo`, a, { repo_id: String(repo.id) });
  const un = await post(`${await settingsPath(pa.id)}/unlink-repo`, a, {});
  expect(un.status).toBe(303);
  expect(await live(repo.id)).toEqual([]);
  const r = await post(`${await settingsPath(pb.id)}/link-repo`, b, { repo_id: String(repo.id) });
  expect(r.status).toBe(303);
  expect((await live(repo.id))[0]?.project_id).toBe(pb.id);
});

test("project named owner/repo needs access to that repo", async () => {
  const u = await user();
  const name = `octo/needs${rnd()}`;
  access(u, [{ id: rnd(), full_name: "octo/other" }]);
  const no = await post("/projects", u, { name });
  expect(no.headers.get("location")).toBe("/settings?error=not_allowed");
  const repo = { id: rnd(), full_name: name };
  access(u, [repo]);
  const ok = await post("/projects", u, { name: name.toUpperCase() });
  expect(ok.headers.get("location")).toBe(`/p/${name.toUpperCase()}`);
  const [p] = await sql`select id from projects where lower(public_name) = lower(${name})`;
  expect((await live(repo.id))[0]?.project_id).toBe(p.id);
});

test("expired github token redirects to /github/connect", async () => {
  const u = await user("expired");
  const p = await project(u);
  const r = await post(`${await settingsPath(p.id)}/link-repo`, u, { repo_id: "1" });
  expect(r.status).toBe(303);
  expect(r.headers.get("location")).toBe(
    `/github/connect?return_to=${encodeURIComponent(`${await settingsPath(p.id)}/settings`)}`,
  );
});

test("callback with a code and no state is discarded: redirect, no fetch, no token row", async () => {
  const u = await user(false);
  const before = outbound;
  const r = await fetch(`${base}/github/callback?code=stray`, {
    redirect: "manual",
    headers: { cookie: u.cookie },
  });
  expect(r.status).toBe(303);
  expect(r.headers.get("location")).toBe("/github/connect");
  expect(outbound).toBe(before);
  const [n] = await sql`select count(*)::int as n from github_user_tokens where user_id = ${u.id}`;
  expect(n.n).toBe(0);
});

test("a token GitHub rejects (401) is deleted and the user is sent to reconnect", async () => {
  const u = await user();
  const p = await project(u);
  dead.add(`tok-${u.id}`);
  const path = `${await settingsPath(p.id)}/settings`;
  const r = await post(`${await settingsPath(p.id)}/link-repo`, u, { repo_id: "1" });
  expect(r.status).toBe(303);
  expect(r.headers.get("location")).toStartWith("/github/connect?return_to=");
  const [n] = await sql`select count(*)::int as n from github_user_tokens where user_id = ${u.id}`;
  expect(n.n).toBe(0);
  // the settings page does the same
  const u2 = await user();
  const p2 = await project(u2);
  dead.add(`tok-${u2.id}`);
  const g = await fetch(`${base}${await settingsPath(p2.id)}/settings`, {
    redirect: "manual",
    headers: { cookie: u2.cookie },
  });
  expect(g.status).toBe(303);
  expect(g.headers.get("location")).toStartWith("/github/connect");
  expect(path).toContain("/settings");
});

test("linking a repo already live on this project says so", async () => {
  const u = await user();
  const p = await project(u);
  const repo = { id: rnd(), full_name: `octo/same${rnd()}` };
  access(u, [repo]);
  const path = `${await settingsPath(p.id)}/link-repo`;
  await post(path, u, { repo_id: String(repo.id) });
  const r = await post(path, u, { repo_id: String(repo.id) });
  expect(r.status).toBe(409);
  expect(await r.json()).toMatchObject({ error: "repo already linked to this project" });
});

test("a project keeps one live repo: linking a second is 409 until the first is unlinked", async () => {
  const u = await user();
  const p = await project(u);
  const r1 = { id: rnd(), full_name: `octo/one${rnd()}` };
  const r2 = { id: rnd(), full_name: `octo/two${rnd()}` };
  access(u, [r1, r2]);
  const path = await settingsPath(p.id);
  expect((await post(`${path}/link-repo`, u, { repo_id: String(r1.id) })).status).toBe(303);
  const r = await post(`${path}/link-repo`, u, { repo_id: String(r2.id) });
  expect(r.status).toBe(409);
  expect(await r.json()).toMatchObject({
    error: "project already has a linked repo; unlink it first",
  });
  expect(await live(r2.id)).toEqual([]);
  await post(`${path}/unlink-repo`, u, {});
  expect((await post(`${path}/link-repo`, u, { repo_id: String(r2.id) })).status).toBe(303);
});

test("gh never follows a Link next URL off the GitHub API origin", async () => {
  const urls: string[] = [];
  const f = (async (input: RequestInfo | URL) => {
    urls.push(String(input));
    return Response.json([1], { headers: { link: '<https://evil.example/x>; rel="next"' } });
  }) as typeof fetch;
  expect(await gh("t", "/x", f)).toEqual([[1]]);
  expect(urls).toHaveLength(1);
});

test("link without csrf is 403", async () => {
  const u = await user();
  const p = await project(u);
  const repo = { id: rnd(), full_name: `octo/csrf${rnd()}` };
  access(u, [repo]);
  const path = `${await settingsPath(p.id)}/link-repo`;
  const bad = await fetch(`${base}${path}`, {
    method: "POST",
    redirect: "manual",
    headers: { cookie: u.cookie, origin: ORIGIN },
    body: new URLSearchParams({ repo_id: String(repo.id) }),
  });
  expect(bad.status).toBe(403);
  expect((await post(path, u, { repo_id: String(repo.id) }, "http://evil.example")).status).toBe(
    403,
  );
  expect(await live(repo.id)).toEqual([]);
});

test("only the owner can link or unlink", async () => {
  const a = await user();
  const b = await user();
  const p = await project(a);
  const repo = { id: rnd(), full_name: `octo/own${rnd()}` };
  access(b, [repo]);
  const r = await post(`${await settingsPath(p.id)}/link-repo`, b, { repo_id: String(repo.id) });
  expect(r.status).toBe(404);
});

test("settings page escapes repo names; connect/install/setup redirects", async () => {
  const u = await user();
  const p = await project(u);
  access(u, [{ id: rnd(), full_name: `octo/<script>${rnd()}` }]);
  const page = await fetch(`${base}${await settingsPath(p.id)}/settings`, {
    headers: { cookie: u.cookie },
  });
  const html = await page.text();
  expect(page.status).toBe(200);
  expect(html).not.toContain("<script>");
  expect(html).toContain("&lt;script&gt;");
  const inst = await fetch(`${base}/github/install`, {
    redirect: "manual",
    headers: { cookie: u.cookie },
  });
  expect(inst.headers.get("location")).toBe("https://github.com/apps/test/installations/new");
  const setup = await fetch(`${base}/github/setup?installation_id=1&setup_action=install`, {
    redirect: "manual",
    headers: { cookie: u.cookie },
  });
  expect(setup.headers.get("location")).toBe("/github/connect");
  const con = await fetch(`${base}/github/connect`, {
    redirect: "manual",
    headers: { cookie: u.cookie },
  });
  const loc = new URL(con.headers.get("location") as string);
  expect(loc.origin + loc.pathname).toBe("https://github.com/login/oauth/authorize");
  expect(loc.searchParams.get("client_id")).toBe("test");
  expect(con.headers.get("set-cookie")).toContain(
    `tally_gh_oauth=${loc.searchParams.get("state")}`,
  );
});

test("callback needs the browser-bound state, then stores the token (no refresh token)", async () => {
  const u = await user(false);
  const con = await fetch(`${base}/github/connect`, {
    redirect: "manual",
    headers: { cookie: u.cookie },
  });
  const state = new URL(con.headers.get("location") as string).searchParams.get("state") as string;
  const ghId = rnd();
  routes.set("/login/oauth/access_token", {
    access_token: "ghu_x",
    expires_in: 28800,
    refresh_token: "ghr_secret",
  });
  routes.set("/user", { id: ghId, login: "octo" });
  const noCookie = await fetch(`${base}/github/callback?code=c&state=${state}`, {
    redirect: "manual",
    headers: { cookie: u.cookie },
  });
  expect(noCookie.status).toBe(400);
  const ok = await fetch(`${base}/github/callback?code=c&state=${state}`, {
    redirect: "manual",
    headers: { cookie: `${u.cookie}; tally_gh_oauth=${state}` },
  });
  expect(ok.status).toBe(303);
  const [t] =
    await sql`select github_user_id, access_token, expires_at from github_user_tokens where user_id = ${u.id}`;
  expect(Number(t.github_user_id)).toBe(ghId);
  expect(t.access_token).toBe("ghu_x");
  expect(JSON.stringify(t)).not.toContain("ghr_");
});
