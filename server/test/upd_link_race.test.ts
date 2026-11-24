import { afterAll, expect, test } from "bun:test";
import { createServer } from "../src/app.ts";
import { testConfig } from "../src/config.ts";
import { connect } from "../src/db.ts";
import { createProject } from "../src/projects.ts";
import { createSession } from "../src/session.ts";
import { resetUpdCache } from "../src/upd.ts";

const sql = connect(process.env.DATABASE_URL ?? "");
const ORIGIN = "http://localhost:8080";
let shipsDelayMs = 0;
let list: unknown[] = [];
const fetchFn = (async (input: RequestInfo | URL) => {
  if (String(input).includes("ships.hackclub.com")) {
    await Bun.sleep(shipsDelayMs);
    return Response.json(list);
  }
  return new Response("no", { status: 404 });
}) as typeof fetch;
const wide = { perMin: 600000, burst: 100000 };
const srv = createServer({
  cfg: testConfig(),
  sql,
  port: 0,
  fetchFn,
  limits: { page: wide, form: wide, badge: wide },
});
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
const post = (path: string, u: U, body: Record<string, string>) =>
  fetch(`${base}${path}`, {
    method: "POST",
    redirect: "manual",
    headers: { cookie: u.cookie, origin: ORIGIN },
    body: new URLSearchParams({ csrf: u.csrf, ...body }),
  });
async function project(u: U, fullName: string, repoId = rnd()) {
  const name = `${u.handle}/p${rnd()}`;
  const r = await createProject(sql, u.id, name);
  if (!("id" in r)) throw new Error("setup");
  await sql`insert into github_repos (id, full_name) values (${repoId}, ${fullName}) on conflict (id) do update set full_name = excluded.full_name`;
  await sql`insert into project_repos (project_id, github_repo_id) values (${r.id}, ${repoId})`;
  return { id: r.id, path: `/p/${name}`, name, repoId };
}

test("upd-link racing unlink-repo does not leave a record on an unlinked project", async () => {
  resetUpdCache();
  list = [
    { id: "recRACE", ysws: "Horizons", approved_at: 1790553600, code_url: "https://github.com/octo/mine" },
  ];
  shipsDelayMs = 400;
  const u = await user();
  const p = await project(u, "octo/mine");
  const link = post(`${p.path}/upd-link`, u, { record_id: "recRACE" });
  await Bun.sleep(150); // upd-link is awaiting Ships
  expect((await post(`${p.path}/unlink-repo`, u, {})).status).toBe(303);
  expect((await link).status).toBe(409);
  shipsDelayMs = 0;
  const [row] = await sql`select upd_record_id from projects where id = ${p.id}`;
  expect(row.upd_record_id).toBeNull();
});
