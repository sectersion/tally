import { afterAll, beforeEach, expect, test } from "bun:test";
import { createServer } from "../src/app.ts";
import { hashToken } from "../src/auth.ts";
import { testConfig } from "../src/config.ts";
import { connect } from "../src/db.ts";
import { linkRepoTx } from "../src/github.ts";
import { commitWindows, createProject } from "../src/projects.ts";
import { createSession } from "../src/session.ts";
import { resetUpdCache } from "../src/upd.ts";

const sql = connect(process.env.DATABASE_URL ?? "");
const ORIGIN = "http://localhost:8080";
const GENESIS = "0".repeat(64);

let ships: unknown[] = [];
const fetchFn = (async (input: RequestInfo | URL) => {
  const u = new URL(String(input));
  if (u.host === "ships.hackclub.com") return Response.json(ships);
  return new Response("nope", { status: 404 });
}) as typeof fetch;
beforeEach(() => {
  resetUpdCache();
  ships = [];
});

const srv = createServer({ cfg: testConfig(), sql, port: 0, fetchFn });
const base = `http://localhost:${srv.port}`;
afterAll(async () => {
  srv.stop(true);
  await sql.close();
});

const h = () => `u${crypto.randomUUID().replaceAll("-", "").slice(0, 12)}`;
const rnd = () => Math.floor(Math.random() * 1e12);
const LEAK = {
  email: "leak-email@example.test",
  name: "LeakRealName",
  verification_status: "leak-verified-status",
};
async function owner(o: { profilePublic?: boolean; displayName?: string } = {}) {
  const id = crypto.randomUUID();
  const handle = h();
  await sql`insert into users (id, hca_sub, handle, email, name, verification_status, display_name, profile_public)
    values (${id}, ${`sub!leak!${id}`}, ${handle}, ${LEAK.email}, ${LEAK.name}, ${LEAK.verification_status},
      ${o.displayName ?? null}, ${o.profilePublic ?? true})`;
  return { id, handle };
}
async function projectWithOwner(o: { public?: boolean; profilePublic?: boolean } = {}) {
  const u = await owner({ profilePublic: o.profilePublic });
  const name = `${u.handle}/p${rnd()}`;
  const r = await createProject(sql, u.id, name);
  if (!("id" in r)) throw new Error("setup");
  if (o.public === false) await sql`update projects set public = false where id = ${r.id}`;
  return { u, id: r.id, name };
}
async function auth(userId: string) {
  const { setCookie, csrf } = await createSession(sql, testConfig(), userId);
  return { cookie: setCookie.split(";")[0] as string, csrf };
}
const get = (path: string, a?: { cookie: string }) =>
  fetch(`${base}${path}`, { redirect: "manual", headers: a ? { cookie: a.cookie } : {} });
const post = (path: string, a: { cookie: string; csrf: string }, body: Record<string, string>) =>
  fetch(`${base}${path}`, {
    method: "POST",
    redirect: "manual",
    headers: { cookie: a.cookie, origin: ORIGIN },
    body: new URLSearchParams({ csrf: a.csrf, ...body }),
  });

let hashNo = 0;
/** One chain with entries at the given ms timestamps. */
async function ledger(projectId: string, rows: { ts: number; v: number; u?: number }[]) {
  const [c] =
    await sql`insert into chains (project_id, ruleset, head_hash) values (${projectId}, ${`r${rnd()}`}, ${GENESIS}) returning id`;
  for (const [i, r] of rows.entries())
    await sql`insert into ledger_entries (chain_id, seq, hash, prev_hash, action_id, ts, kind, verified_s, unverified_s, entry)
      values (${c.id}, ${i}, ${`${++hashNo}`.padStart(64, "0")}, ${GENESIS}, ${`a${i}`}, ${r.ts}, 'k', ${r.v}, ${r.u ?? 0}, '{}'::json)`;
  await sql`update chains set verified_s = ${rows.reduce((n, r) => n + r.v, 0)},
    unverified_s = ${rows.reduce((n, r) => n + (r.u ?? 0), 0)} where id = ${c.id}`;
}
async function receipt(
  projectId: string,
  sha: string,
  receivedAt: Date,
  claimed: number | null = null,
  repoId = rnd(),
) {
  const d = crypto.randomUUID();
  await sql`insert into webhook_deliveries (delivery_id, event) values (${d}, 'push')`;
  await sql`insert into push_receipts (delivery_id, sha, github_repo_id, project_id, ref, is_head, forced, claimed_ts, received_at)
    values (${d}, ${sha}, ${repoId}, ${projectId}, 'refs/heads/main', true, false, ${claimed}, ${receivedAt})`;
}
async function linkRepo(projectId: string, fullName: string) {
  const id = rnd();
  await sql`insert into github_repos (id, full_name) values (${id}, ${fullName})`;
  await sql`insert into project_repos (project_id, github_repo_id) values (${projectId}, ${id})`;
  return id;
}
const NOW = Date.now();

test("project page shows averages, a trend chart and weekly totals", async () => {
  const { id, name } = await projectWithOwner();
  await ledger(id, [
    { ts: NOW - 2 * 86_400_000, v: 3600 },
    { ts: NOW - 86_400_000, v: 7200 },
    { ts: NOW, v: 5400 },
  ]);
  const t = await (await get(`/p/${name}`)).text();
  expect(t).toContain("avg 1.5 h/day");
  expect(t).toContain('id="trend"');
  expect(t).toContain("<svg");
  expect(t).not.toContain("Verified by week"); // one week is not a summary
  expect(t).toContain("4.5 h verified");
  expect(t).not.toMatch(/\sstyle\s*=/i);
});

test("project page has no daily table; a pie shows verified vs unverified", async () => {
  const { id, name } = await projectWithOwner();
  await ledger(id, [{ ts: NOW, v: 3600, u: 1800 }]);
  const t = await (await get(`/p/${name}`)).text();
  expect(t).not.toContain("Daily verified time");
  expect(t).not.toContain("<table"); // the daily rows are gone; pushes keep <thead> tables
  expect(t).toContain('id="pie"');
  expect(t).toContain('role="img"');
});

test("docs page renders from /docs and links from the project page", async () => {
  const { name } = await projectWithOwner();
  const r = await get("/docs");
  expect(r.status).toBe(200);
  const t = await r.text();
  expect(t).toContain("Getting started");
  expect(t).toContain("How verified time is computed");
  const p = await (await get(`/p/${name}`)).text();
  expect(p).toContain('href="/docs"');
});

test("zero-hour project page renders", async () => {
  // Review Focus 6
  const { name } = await projectWithOwner();
  const r = await get(`/p/${name}`);
  expect(r.status).toBe(200);
  const t = await r.text();
  expect(t).toContain("0.0 h verified");
  expect(t).toContain("No pushes witnessed yet");
  expect(t).not.toContain("unverified (not counted)");
  expect(t).toContain("Ledger head last committed never");
});

test("unverified shown separately and never added", async () => {
  const { id, name } = await projectWithOwner();
  await ledger(id, [{ ts: NOW - 3_600_000, v: 3600, u: 1800 }]);
  const t = await (await get(`/p/${name}`)).text();
  expect(t).toContain("1.0 h verified");
  expect(t).toContain("30 min unverified (not counted)");
  expect(t).not.toMatch(/\b1\.5 h\b(?= verified)/); // never shows the sum as verified
  expect(t).toContain("1.5 h claimed"); // the pie labels it honestly
});

test("under a minute of unverified time is <1 min, never 0 min", async () => {
  const { id, name } = await projectWithOwner();
  await ledger(id, [{ ts: NOW - 3_600_000, v: 3600, u: 59 }]);
  const t = await (await get(`/p/${name}`)).text();
  expect(t).toContain("&lt;1 min unverified (not counted)");
  expect(t).not.toContain("0 min");
});

test("form failures are html pages with their status; /v1 stays json", async () => {
  const { u, name } = await projectWithOwner();
  const a = await auth(u.id);
  const r = await post("/device", a, { decision: "maybe" });
  expect(r.status).toBe(400);
  expect(r.headers.get("content-type")).toBe("text/html; charset=utf-8");
  expect(await r.text()).toContain("Choose approve or deny.");
  const l = await post(`/p/${name}/upd-link`, a, { record_id: "rec1" });
  expect(l.status).toBe(400);
  expect(await l.text()).toContain("link a GitHub repo first");
  const c = await post("/settings/profile", { ...a, csrf: "wrong" }, {});
  expect(c.status).toBe(403);
  expect(c.headers.get("content-type")).toBe("text/html; charset=utf-8");
  const v = await fetch(`${base}/v1/device/code`, { method: "POST", body: "{" });
  expect(v.status).toBe(400);
  expect(v.headers.get("content-type")).toContain("application/json");
});

test("numbers section carries the live hooks", async () => {
  const { id, name } = await projectWithOwner();
  await ledger(id, [{ ts: NOW, v: 60 }]);
  const [{ change_seq }] = await sql`select change_seq from projects where id = ${id}`;
  const t = await (await get(`/p/${name}`)).text();
  expect(t).toContain(`<section id="numbers" data-project="${id}" data-seq="${change_seq}">`);
  expect(t).toContain('<script src="/static/live.js" defer></script>');
  expect(t).toMatch(/<svg id="pie"/);
  expect(t).toContain(`![tracked with Tally](${ORIGIN}/badge/${name}.svg)`);
});

test("project names match case-insensitively", async () => {
  const { name } = await projectWithOwner();
  expect((await get(`/p/${name.toUpperCase()}`)).status).toBe(200);
});

test("private profile is the same 404 as a missing one", async () => {
  // Review Focus 7
  const u = await owner({ profilePublic: false });
  const a = await get(`/u/${u.handle}`);
  const b = await get(`/u/nobody-${h()}`);
  expect(a.status).toBe(404);
  expect(b.status).toBe(404);
  expect(await a.text()).toBe(await b.text());
});

test("public profile lists only public projects with hours", async () => {
  const pub = await projectWithOwner();
  await ledger(pub.id, [{ ts: NOW, v: 7200, u: 600 }]);
  const hidden = `${pub.u.handle}/hidden${rnd()}`;
  const r = await createProject(sql, pub.u.id, hidden);
  if (!("id" in r)) throw new Error("setup");
  await sql`update projects set public = false where id = ${r.id}`;
  const res = await get(`/u/${pub.u.handle.toUpperCase()}`);
  expect(res.status).toBe(200);
  const t = await res.text();
  expect(t).toContain(pub.name);
  expect(t).toContain("2.0 h");
  expect(t).toContain("10 min (not counted)");
  expect(t).not.toContain(hidden);
});

test("public project of a private profile stays up, without the owner link", async () => {
  const { u, name } = await projectWithOwner({ profilePublic: false });
  const r = await get(`/p/${name}`);
  expect(r.status).toBe(200);
  const t = await r.text();
  expect(t).not.toContain(`href="/u/${u.handle}"`);
  expect(t).not.toContain("Owner:");
  const pub = await projectWithOwner();
  expect(await (await get(`/p/${pub.name}`)).text()).toContain(
    `Owner: <a href="/u/${pub.u.handle}">`,
  );
});

test("private project is 404 to others and visible to the owner", async () => {
  const { u, name } = await projectWithOwner({ public: false });
  const anon = await get(`/p/${name}`);
  expect(anon.status).toBe(404);
  const missing = await get(`/p/${h()}/nope`);
  expect(await anon.text()).toBe(await missing.text());
  const other = await auth((await owner()).id);
  expect((await get(`/p/${name}`, other)).status).toBe(404);
  const me = await get(`/p/${name}`, await auth(u.id));
  expect(me.status).toBe(200);
  const t = await me.text();
  expect(t).toContain("This project is private");
  expect(t).toContain(u.handle); // the header shows the viewer's handle
  expect(t).toContain('action="/auth/logout"');
});

test("an unowned project is 404", async () => {
  const name = `${h()}/orphan`;
  await sql`insert into projects (id, public_name) values (${crypto.randomUUID()}, ${name})`;
  expect((await get(`/p/${name}`)).status).toBe(404);
});

test("project page never contains email, sub, slack id or verification status", async () => {
  const { u, id, name } = await projectWithOwner();
  await sql`insert into device_tokens (project_id, token_hash, label) values (${id}, ${hashToken(`t${rnd()}`)}, 'leak-token-label')`;
  await linkRepo(id, `octo/leak${rnd()}`);
  await receipt(id, "a".repeat(40), new Date(NOW - 60_000), NOW - 120_000);
  await ledger(id, [{ ts: NOW - 90_000, v: 30 }]);
  for (const path of [`/p/${name}`, `/u/${u.handle}`]) {
    const t = await (await get(path)).text();
    for (const bad of [
      LEAK.email,
      LEAK.name,
      LEAK.verification_status,
      "sub!leak",
      "leak-token-label",
    ])
      expect(t).not.toContain(bad);
  }
});

test("commit windows split verified time at each witnessed push", async () => {
  const { id } = await projectWithOwner();
  const T = 1_800_000_000_000;
  await ledger(id, [
    { ts: T + 10_000, v: 60 },
    { ts: T + 100_000, v: 120 },
  ]);
  await receipt(id, "a".repeat(40), new Date(T + 50_000));
  await receipt(id, "b".repeat(40), new Date(T + 200_000));
  const w = await commitWindows(sql, id);
  expect(w.map((x) => [x.sha[0], x.verified_s])).toEqual([
    ["b", 120],
    ["a", 60],
  ]);
});

test("commit windows return the newest 50; the oldest returned keeps its lower bound", async () => {
  const { id } = await projectWithOwner();
  const T = 1_800_000_000_000;
  // Commit i is received at T + 10 s * i; entry i (v = i + 1) sits 5 s before it.
  await ledger(
    id,
    Array.from({ length: 55 }, (_, i) => ({ ts: T + i * 10_000 - 5_000, v: i + 1 })),
  );
  for (let i = 0; i < 55; i++)
    await receipt(id, `${i}`.padStart(40, "0"), new Date(T + i * 10_000));
  const w = await commitWindows(sql, id);
  expect(w).toHaveLength(50);
  expect(w[0]?.sha).toBe("54".padStart(40, "0"));
  expect(w[49]?.sha).toBe("5".padStart(40, "0"));
  expect(w.map((x) => x.verified_s)).toEqual(Array.from({ length: 50 }, (_, k) => 55 - k));
});

test("commits of one push: the head commit carries the window", async () => {
  const { id } = await projectWithOwner();
  const T = 1_800_000_000_000;
  await ledger(id, [{ ts: T - 1000, v: 60 }]);
  const d = crypto.randomUUID();
  await sql`insert into webhook_deliveries (delivery_id, event) values (${d}, 'push')`;
  for (const [sha, head] of [
    ["0".repeat(40), false],
    ["f".repeat(40), true],
  ] as const)
    await sql`insert into push_receipts (delivery_id, sha, github_repo_id, project_id, ref, is_head, forced, received_at)
      values (${d}, ${sha}, 1, ${id}, 'refs/heads/main', ${head}, false, ${new Date(T)})`;
  const w = await commitWindows(sql, id);
  expect(w.map((x) => [x.sha[0], x.verified_s])).toEqual([
    ["0", 0],
    ["f", 60],
  ]);
});

test("commit window flags are informational", async () => {
  const { id } = await projectWithOwner();
  const T = 1_800_000_000_000;
  await ledger(id, [{ ts: T, v: 60 }]);
  await receipt(id, "c".repeat(40), new Date(T + 1000), T + 10 * 60_000); // claims the future
  await receipt(id, "d".repeat(40), new Date(T + 2000), T - 8 * 86_400_000); // pushed late
  const w = await commitWindows(sql, id);
  expect(w.map((x) => [x.sha[0], x.flag, x.verified_s])).toEqual([
    ["d", "pushed_late", 0],
    ["c", "claimed_after_receipt", 60],
  ]);
});

test("a replayed push does not move the commit's received time", async () => {
  const { id, name } = await projectWithOwner();
  await linkRepo(id, `octo/replay${rnd()}`);
  const first = new Date("2026-01-02T03:04:05Z");
  const later = new Date("2026-01-09T03:04:05Z");
  await receipt(id, "e".repeat(40), first, Date.parse("2026-01-02T03:00:00Z"));
  await receipt(id, "e".repeat(40), later);
  const w = await commitWindows(sql, id);
  expect(w).toHaveLength(1);
  expect(w[0]?.received_at.getTime()).toBe(first.getTime());
  const t = await (await get(`/p/${name}`)).text();
  expect(t).toContain("2026-01-02 03:04:05 UTC");
  expect(t).not.toContain("2026-01-09 03:04:05 UTC");
  expect(t).toContain("2026-01-02 03:00:00 UTC (claimed)");
  expect(t).toContain("<th>Ledger-claimed verified time since the previous push</th>");
  expect(t).toContain("the ledger times are claimed by the device");
});

test("renamed repo shows the new name and 'was' the old", async () => {
  const { id, name } = await projectWithOwner();
  const repo = await linkRepo(id, "octo/new-name");
  await sql`insert into repo_events (github_repo_id, kind, detail) values (${repo}, 'renamed', 'octo/old-name -> octo/new-name')`;
  await receipt(id, "f".repeat(40), new Date(NOW), null, repo);
  const t = await (await get(`/p/${name}`)).text();
  expect(t).toContain("octo/new-name</a> (was octo/old-name)");
  expect(t).toContain(`href="https://github.com/octo/new-name/commit/${"f".repeat(40)}"`);
});

test("a removed GitHub App shows a revocation note", async () => {
  const { id, name } = await projectWithOwner();
  const repo = await linkRepo(id, `octo/gone${rnd()}`);
  await sql.begin(async (tx) => {
    // as github.ts unlink(): one transaction, so both carry the same now()
    await tx`update project_repos set unlinked_at = now() where github_repo_id = ${repo}`;
    await tx`insert into repo_events (github_repo_id, kind, detail) values (${repo}, 'uninstalled', 'deleted')`;
  });
  const t = await (await get(`/p/${name}`)).text();
  expect(t).toMatch(/GitHub App removed on \d{4}-\d{2}-\d{2}; pushes after this are not witnessed/);
});

test("ledger head commit time is shown", async () => {
  const { id, name } = await projectWithOwner();
  await sql`insert into commits (project_id, hash, ts, received_at) values (${id}, ${"1".repeat(64)}, 1, '2026-03-04T05:06:07Z')`;
  expect(await (await get(`/p/${name}`)).text()).toContain(
    "Ledger head last committed 2026-03-04 05:06:07 UTC",
  );
});

test("names with markup are escaped", async () => {
  const u = await owner({ displayName: "<img src=x>" });
  const t = await (await get(`/u/${u.handle}`)).text();
  expect(t).not.toContain("<img src=x>");
  expect(t).toContain("&lt;img src=x&gt;");
});

test("the Unified Projects DB label is escaped", async () => {
  const { id, name } = await projectWithOwner();
  const full = `octo/upd${rnd()}`;
  await linkRepo(id, full);
  await sql`update projects set upd_record_id = 'recX' where id = ${id}`;
  ships = [{ id: "recX", code_url: `https://github.com/${full}`, ysws: "<b>Prog", approved_at: 1 }];
  const t = await (await get(`/p/${name}`)).text();
  expect(t).toContain("Unified Projects DB: &lt;b&gt;Prog (record recX)");
  expect(t).not.toContain("<b>Prog");
});

test("settings without session redirects to sign-in", async () => {
  const r = await get("/settings");
  expect(r.status).toBe(303);
  expect(r.headers.get("location")).toBe("/auth/login?return_to=%2Fsettings");
});

test("settings shows tokens with revoke, never their hash; revoke covers project tokens", async () => {
  const { u, id, name } = await projectWithOwner();
  const th = hashToken(`t${rnd()}`);
  const [pt] =
    await sql`insert into device_tokens (project_id, token_hash, label) values (${id}, ${th}, 'proj-label') returning id`;
  const [ut] =
    await sql`insert into device_tokens (user_id, token_hash, label) values (${u.id}, ${hashToken(`t${rnd()}`)}, 'laptop-label') returning id`;
  const a = await auth(u.id);
  const r = await get("/settings", a);
  expect(r.status).toBe(200);
  const t = await r.text();
  expect(t).toContain("proj-label");
  expect(t).toContain("laptop-label");
  expect(t).toContain(name);
  expect(t).not.toContain(th);
  expect(t).toContain(`action="/settings/tokens/${pt.id}/revoke"`);
  expect(t).toContain(`action="/settings/tokens/${ut.id}/revoke"`);
  expect(t).toContain("appears in your project names");
  expect(t).toContain('href="/github/install"');
  const other = await auth((await owner()).id);
  expect((await post(`/settings/tokens/${pt.id}/revoke`, other, {})).status).toBe(404);
  expect((await post(`/settings/tokens/${pt.id}/revoke`, a, {})).status).toBe(303);
  const [row] = await sql`select revoked_at from device_tokens where id = ${pt.id}`;
  expect(row.revoked_at).not.toBeNull();
  expect(await (await get("/settings", a)).text()).not.toContain("proj-label");
});

test("settings profile saves display name and visibility behind csrf", async () => {
  const u = await owner();
  const a = await auth(u.id);
  expect(
    (
      await fetch(`${base}/settings/profile`, {
        method: "POST",
        redirect: "manual",
        headers: { cookie: a.cookie, origin: ORIGIN },
        body: new URLSearchParams({ display_name: "x" }),
      })
    ).status,
  ).toBe(403);
  expect((await post("/settings/profile", a, { display_name: "x".repeat(65) })).status).toBe(303);
  expect((await sql`select display_name from users where id = ${u.id}`)[0].display_name).toBeNull();
  expect((await post("/settings/profile", a, { display_name: " Ada " })).status).toBe(303);
  const [row] = await sql`select display_name, profile_public from users where id = ${u.id}`;
  expect(row).toEqual({ display_name: "Ada", profile_public: false }); // unchecked box
  expect((await get(`/u/${u.handle}`)).status).toBe(404);
  await post("/settings/profile", a, { display_name: "Ada", profile_public: "on" });
  expect(await (await get(`/u/${u.handle}`)).text()).toContain("Ada");
});

test("display names with format characters are rejected", async () => {
  const u = await owner();
  const a = await auth(u.id);
  for (const bad of ["evil‮gnp", "zero​width", "ctl\u0007x"]) {
    const r = await post("/settings/profile", a, { display_name: bad, profile_public: "on" });
    expect(r.headers.get("location")).toBe("/settings?error=display_name");
  }
  expect((await sql`select display_name from users where id = ${u.id}`)[0].display_name).toBeNull();
});

test("an unknown or prototype ?error= renders no message", async () => {
  const a = await auth((await owner()).id);
  const t = await (await get("/settings?error=constructor", a)).text();
  expect(t).not.toContain('role="alert"');
  expect(await (await get("/settings?error=taken", a)).text()).toContain('role="alert"');
});

test("deleting the account voids an approved but unpolled device code", async () => {
  const u = await owner();
  const a = await auth(u.id);
  const c = (await (
    await fetch(`${base}/v1/device/code`, { method: "POST", body: '{"device_name":"lap"}' })
  ).json()) as { device_code: string; user_code: string };
  expect((await post("/device", a, { user_code: c.user_code, decision: "approve" })).status).toBe(
    200,
  );
  expect((await post("/settings/delete", a, { confirm: u.handle })).status).toBe(303);
  const r = await fetch(`${base}/v1/device/token`, {
    method: "POST",
    body: JSON.stringify({ device_code: c.device_code }),
  });
  expect(r.status).toBe(400);
  expect(await r.json()).toEqual({ error: "expired_token" });
  expect(
    (await sql`select count(*)::int as n from device_tokens where user_id = ${u.id}`)[0].n,
  ).toBe(0);
});

test("home page: signed out offers sign-in; signed in lists projects or asks for a handle", async () => {
  const out = await (await get("/")).text();
  expect(out).toContain("Sign in with Hack Club");
  const { u, name } = await projectWithOwner({ public: false });
  const t = await (await get("/", await auth(u.id))).text();
  expect(t).toContain(name);
  expect(t).toContain('action="/projects"');
  const id = crypto.randomUUID();
  await sql`insert into users (id, hca_sub) values (${id}, ${`sub!${id}`})`;
  const noHandle = await (await get("/", await auth(id))).text();
  expect(noHandle).toContain("Set your handle first");
  expect(noHandle).toContain('action="/settings/handle"');
});

test("delete account unlinks the user's repos so someone else can link them", async () => {
  const { u, id } = await projectWithOwner();
  const repo = await linkRepo(id, `octo/del${rnd()}`);
  await sql`update projects set upd_record_id = 'recDEL' where id = ${id}`;
  expect((await post("/settings/delete", await auth(u.id), { confirm: u.handle })).status).toBe(
    303,
  );
  expect(
    (
      await sql`select count(*)::int as n from project_repos where github_repo_id = ${repo} and unlinked_at is null`
    )[0].n,
  ).toBe(0);
  expect(
    (await sql`select kind from repo_events where github_repo_id = ${repo}`).map(
      (r: { kind: string }) => r.kind,
    ),
  ).toEqual(["unlinked"]);
  expect(
    (await sql`select upd_record_id from projects where id = ${id}`)[0].upd_record_id,
  ).toBeNull();
  const other = await projectWithOwner();
  const full = (await sql`select full_name from github_repos where id = ${repo}`)[0].full_name;
  await sql.begin((tx) =>
    linkRepoTx(tx, other.id, {
      id: repo,
      full_name: full,
      installation_id: rnd(),
      account_login: "octo",
    }),
  );
  expect(
    (
      await sql`select project_id from project_repos where github_repo_id = ${repo} and unlinked_at is null`
    )[0].project_id,
  ).toBe(other.id);
});

test("delete account hides everything public", async () => {
  const { u, id, name } = await projectWithOwner();
  await ledger(id, [{ ts: NOW, v: 60 }]);
  await receipt(id, "9".repeat(40), new Date(NOW));
  await sql`insert into device_tokens (user_id, token_hash) values (${u.id}, ${hashToken(`t${rnd()}`)})`;
  await sql`insert into device_tokens (project_id, token_hash) values (${id}, ${hashToken(`t${rnd()}`)})`;
  await sql`insert into github_user_tokens (user_id, github_user_id, access_token, expires_at)
    values (${u.id}, ${rnd()}, 'tok', now() + interval '1 hour')`;
  const a = await auth(u.id);
  expect((await post("/settings/delete", a, { confirm: "wrong" })).status).toBe(303);
  expect((await get(`/p/${name}`)).status).toBe(200);
  const r = await post("/settings/delete", a, { confirm: u.handle });
  expect(r.status).toBe(303);
  expect(r.headers.get("location")).toBe("/");
  expect((await get(`/p/${name}`)).status).toBe(404);
  expect((await get(`/u/${u.handle}`)).status).toBe(404);
  expect((await get("/settings", a)).status).toBe(303); // session gone
  const [usr] =
    await sql`select email, name, display_name, verification_status from users where id = ${u.id}`;
  expect(usr).toEqual({ email: null, name: null, display_name: null, verification_status: null });
  const n = async (q: Promise<{ n: number }[]>) => (await q)[0]?.n;
  expect(await n(sql`select count(*)::int as n from sessions where user_id = ${u.id}`)).toBe(0);
  expect(
    await n(
      sql`select count(*)::int as n from device_tokens where user_id = ${u.id} or project_id = ${id}`,
    ),
  ).toBe(0);
  expect(
    await n(sql`select count(*)::int as n from github_user_tokens where user_id = ${u.id}`),
  ).toBe(0);
  // evidence stays
  expect(
    await n(
      sql`select count(*)::int as n from ledger_entries e join chains c on c.id = e.chain_id where c.project_id = ${id}`,
    ),
  ).toBe(1);
  expect(await n(sql`select count(*)::int as n from push_receipts where project_id = ${id}`)).toBe(
    1,
  );
});
