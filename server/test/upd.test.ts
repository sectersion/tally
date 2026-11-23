import { afterAll, beforeEach, expect, test } from "bun:test";
import { createServer } from "../src/app.ts";
import { testConfig } from "../src/config.ts";
import { connect } from "../src/db.ts";
import { createProject } from "../src/projects.ts";
import { createSession } from "../src/session.ts";
import { findUpdEntries, normalizeRepoUrl, resetUpdCache, updLabel } from "../src/upd.ts";

const sql = connect(process.env.DATABASE_URL ?? "");
const ORIGIN = "http://localhost:8080";
const fixture = (await Bun.file(
  new URL("./fixtures/upd/ysws_entry.json", import.meta.url),
).json()) as Record<string, unknown>;
const entry = (
  id: string,
  code_url: string | null,
  ysws = "Horizons",
  approved_at = 1790553600,
) => ({
  ...fixture,
  id,
  code_url,
  ysws,
  approved_at,
});

let list: unknown[] = [];
let shipsCalls = 0;
let shipsDown = false;
const fetchFn = (async (input: RequestInfo | URL) => {
  const u = new URL(String(input));
  if (u.host === "ships.hackclub.com" && u.pathname === "/api/v1/ysws_entries") {
    shipsCalls++;
    return shipsDown ? new Response("x", { status: 500 }) : Response.json(list);
  }
  return new Response("nope", { status: 404 });
}) as typeof fetch;

beforeEach(() => {
  resetUpdCache();
  shipsCalls = 0;
  shipsDown = false;
});

const srv = createServer({ cfg: testConfig(), sql, port: 0, fetchFn });
const base = `http://localhost:${srv.port}`;
afterAll(async () => {
  srv.stop(true);
  await sql.close();
});

const rnd = () => Math.floor(Math.random() * 1e12);
async function user() {
  const id = crypto.randomUUID();
  const handle = `u${id.replaceAll("-", "").slice(0, 12)}`;
  await sql`insert into users (id, hca_sub, handle) values (${id}, ${`sub!${id}`}, ${handle})`;
  const { setCookie, csrf } = await createSession(sql, testConfig(), id);
  return { id, handle, cookie: setCookie.split(";")[0] as string, csrf };
}
type U = Awaited<ReturnType<typeof user>>;
const post = (path: string, u: U, body: Record<string, string>, origin = ORIGIN, csrf = u.csrf) =>
  fetch(`${base}${path}`, {
    method: "POST",
    redirect: "manual",
    headers: { cookie: u.cookie, origin },
    body: new URLSearchParams({ csrf, ...body }),
  });
/** A project owned by u, linked to a GitHub repo named fullName when given. */
async function project(u: U, fullName?: string) {
  const name = `${u.handle}/p${rnd()}`;
  const r = await createProject(sql, u.id, name);
  if (!("id" in r)) throw new Error("setup");
  if (fullName) {
    const repoId = rnd();
    await sql`insert into github_repos (id, full_name) values (${repoId}, ${fullName})`;
    await sql`insert into project_repos (project_id, github_repo_id) values (${r.id}, ${repoId})`;
  }
  return { id: r.id, path: `/p/${name}` };
}
const rec = async (id: string) =>
  (await sql`select upd_record_id from projects where id = ${id}`)[0].upd_record_id as
    | string
    | null;
const seq = async (id: string) =>
  Number((await sql`select change_seq from projects where id = ${id}`)[0].change_seq);

test("normalizeRepoUrl", () => {
  expect(normalizeRepoUrl("https://github.com/Owner/Repo.git")).toBe("owner/repo");
  expect(normalizeRepoUrl("https://github.com/owner/repo/")).toBe("owner/repo");
  expect(normalizeRepoUrl("https://github.com/owner/repo/tree/main/x")).toBe("owner/repo");
  expect(normalizeRepoUrl("https://github.com/owner/repo/blob/main/x.ts")).toBe("owner/repo");
  expect(normalizeRepoUrl("https://gitlab.com/owner/repo")).toBeNull();
  expect(normalizeRepoUrl("https://gist.github.com/owner/abc")).toBeNull();
  expect(normalizeRepoUrl("https://github.com/owner")).toBeNull();
  expect(normalizeRepoUrl("not a url")).toBeNull();
});

test("findUpdEntries matches on the repo only, and caches", async () => {
  list = [
    entry("recMATCH", "https://github.com/Octo/Repo.git/"),
    entry("recNEAR", "https://github.com/octo/repo2"),
    entry("recOTHER", "https://gitlab.com/octo/repo"),
    entry("recNULL", null),
  ];
  const r = await findUpdEntries("octo/repo", fetchFn);
  expect(r).toEqual([{ id: "recMATCH", ysws: "Horizons", approved_at: 1790553600 }]);
  expect(await findUpdEntries("OCTO/repo2", fetchFn)).toHaveLength(1);
  expect(shipsCalls).toBe(1);
});

test("a record id not matching the linked repo is 400 and nothing is written", async () => {
  list = [
    entry("recMINE", "https://github.com/octo/mine"),
    entry("recTHEIRS", "https://github.com/octo/theirs"),
  ];
  const u = await user();
  const p = await project(u, "octo/mine");
  const before = await seq(p.id);
  const r = await post(`${p.path}/upd-link`, u, { record_id: "recTHEIRS" });
  expect(r.status).toBe(400);
  expect(await rec(p.id)).toBeNull();
  expect(await seq(p.id)).toBe(before);
});

test("upd link needs a linked repo", async () => {
  list = [entry("recX", "https://github.com/octo/x")];
  const u = await user();
  const p = await project(u);
  const r = await post(`${p.path}/upd-link`, u, { record_id: "recX" });
  expect(r.status).toBe(400);
  expect(await r.json()).toEqual({ error: "link a GitHub repo first" });
  expect(await rec(p.id)).toBeNull();
});

test("link sets upd_record_id and bumps change_seq; unlink clears it", async () => {
  list = [entry("recMINE", "https://github.com/octo/mine")];
  const u = await user();
  const p = await project(u, "octo/mine");
  const before = await seq(p.id);
  expect((await post(`${p.path}/upd-link`, u, { record_id: "recMINE" })).status).toBe(303);
  expect(await rec(p.id)).toBe("recMINE");
  expect(await seq(p.id)).toBe(before + 1);
  expect((await post(`${p.path}/upd-unlink`, u, {})).status).toBe(303);
  expect(await rec(p.id)).toBeNull();
  expect(await seq(p.id)).toBe(before + 2);
});

test("non-owner gets 404, bad csrf or origin gets 403, and nothing is written", async () => {
  list = [entry("recMINE", "https://github.com/octo/mine")];
  const owner = await user();
  const other = await user();
  const p = await project(owner, "octo/mine");
  expect((await post(`${p.path}/upd-link`, other, { record_id: "recMINE" })).status).toBe(404);
  expect(
    (await post(`${p.path}/upd-link`, owner, { record_id: "recMINE" }, ORIGIN, "bad")).status,
  ).toBe(403);
  expect(
    (await post(`${p.path}/upd-link`, owner, { record_id: "recMINE" }, "http://evil.example"))
      .status,
  ).toBe(403);
  expect(await rec(p.id)).toBeNull();
  await sql`update projects set upd_record_id = 'recMINE' where id = ${p.id}`;
  expect((await post(`${p.path}/upd-unlink`, other, {})).status).toBe(404);
  expect((await post(`${p.path}/upd-unlink`, owner, {}, ORIGIN, "bad")).status).toBe(403);
  expect(await rec(p.id)).toBe("recMINE");
});

test("ships being down is a 502 on link", async () => {
  shipsDown = true;
  const u = await user();
  const p = await project(u, "octo/mine");
  expect((await post(`${p.path}/upd-link`, u, { record_id: "recMINE" })).status).toBe(502);
  expect(await rec(p.id)).toBeNull();
});

test("settings page lists matches escaped, with Link; linked shows Unlink", async () => {
  list = [entry("recMINE", "https://github.com/octo/mine", "<b>Prog</b>")];
  const u = await user();
  const p = await project(u, "octo/mine");
  const get = async () =>
    (await fetch(`${base}${p.path}/settings`, { headers: { cookie: u.cookie } })).text();
  const html = await get();
  expect(html).toContain(
    "Found in Unified Projects DB: &#60;b&#62;Prog&#60;/b&#62;, approved 2026-09-28",
  );
  expect(html).not.toContain("<b>Prog");
  expect(html).toContain(`action="${p.path}/upd-link"`);
  await post(`${p.path}/upd-link`, u, { record_id: "recMINE" });
  const after = await get();
  expect(after).toContain(`action="${p.path}/upd-unlink"`);
  expect(after).toContain("record recMINE");
  // other fields of the entry never reach the page
  expect(after).not.toContain("REDACTED");
  expect(after).not.toContain("example-owner");
});

test("updLabel shows only program and record id", async () => {
  list = [entry("recMINE", "https://github.com/octo/mine", "Horizons")];
  expect(await updLabel("recMINE", "octo/mine", fetchFn)).toBe(
    "Unified Projects DB: Horizons (record recMINE)",
  );
  expect(await updLabel("recGONE", "octo/mine", fetchFn)).toBe(
    "Unified Projects DB: (record recGONE)",
  );
  expect(await updLabel(null, "octo/mine", fetchFn)).toBeNull();
});
