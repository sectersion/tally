import { afterAll, expect, test } from "bun:test";
import { createServer } from "../src/app.ts";
import { hashToken } from "../src/auth.ts";
import { MAX_BODY_BYTES, testConfig } from "../src/config.ts";
import { connect } from "../src/db.ts";
import { createDeviceCode } from "../src/device.ts";
import { createProject } from "../src/projects.ts";
import { createSession } from "../src/session.ts";

const sql = connect(process.env.DATABASE_URL ?? "");
const wide = { perMin: 6000, burst: 1000 };
const srv = createServer({ cfg: testConfig(), sql, port: 0, limits: { form: wide } });
const base = `http://localhost:${srv.port}`;
const ORIGIN = "http://localhost:8080";
afterAll(async () => {
  srv.stop(true);
  await sql.close();
});

const id = crypto.randomUUID();
const handle = `c${id.replaceAll("-", "").slice(0, 12)}`;
await sql`insert into users (id, hca_sub, handle) values (${id}, ${`sub!${id}`}, ${handle})`;
const r = await createProject(sql, id, `${handle}/p`);
if (!("id" in r)) throw new Error("setup");
const pid = r.id;
const repo = Math.floor(Math.random() * 1e12);
await sql`insert into github_repos (id, full_name) values (${repo}, ${`octo/csrf${repo}`})`;
await sql`insert into project_repos (project_id, github_repo_id) values (${pid}, ${repo})`;
await sql`update projects set upd_record_id = 'recCSRF' where id = ${pid}`;
const [tok] =
  await sql`insert into device_tokens (user_id, token_hash, label) values (${id}, ${hashToken(`t${repo}`)}, 'l') returning id`;
const code = await createDeviceCode(sql, "lap");
const { setCookie, csrf } = await createSession(sql, testConfig(), id);
const cookie = setCookie.split(";")[0] as string;

/** Everything a session POST could write for this user. */
const snapshot = async () =>
  (
    await sql`select (select row_to_json(u) from users u where u.id = ${id})::text as u,
      (select json_agg(p order by p.id) from projects p where p.owner_id = ${id})::text as p,
      (select json_agg(t order by t.id) from device_tokens t where t.user_id = ${id})::text as t,
      (select json_agg(l) from project_repos l where l.project_id = ${pid})::text as l,
      (select count(*)::int from sessions where user_id = ${id}) as s,
      (select json_agg(d) from device_codes d where d.user_code = ${code.user_code})::text as d`
  )[0];

const P = `/p/${handle}/p`;
// Each body is one that would write if the csrf check passed.
const ROUTES: [string, Record<string, string>][] = [
  ["/auth/logout", {}],
  ["/settings/profile", { display_name: "changed", profile_public: "on" }],
  ["/settings/delete", { confirm: handle }],
  ["/settings/handle", { handle: "newhandle" }],
  ["/device", { decision: "approve", user_code: code.user_code }],
  [`/settings/tokens/${tok.id}/revoke`, {}],
  ["/projects", { name: `${handle}/other` }],
  [`${P}/link-repo`, { repo_id: "1" }],
  [`${P}/unlink-repo`, {}],
  [`${P}/upd-link`, { record_id: "recX" }],
  [`${P}/upd-unlink`, {}],
  [`${P}/settings`, {}],
];

test("every session POST is 403 and writes nothing without csrf or with a foreign origin", async () => {
  const before = await snapshot();
  for (const [path, body] of ROUTES) {
    const cases = [
      { origin: ORIGIN, body: new URLSearchParams(body) },
      { origin: "https://evil.example", body: new URLSearchParams({ csrf, ...body }) },
    ];
    for (const c of cases) {
      const res = await fetch(`${base}${path}`, {
        method: "POST",
        redirect: "manual",
        headers: { cookie, origin: c.origin },
        body: c.body,
      });
      expect({ path, origin: c.origin, status: res.status }).toEqual({
        path,
        origin: c.origin,
        status: 403,
      });
    }
  }
  expect(await snapshot()).toEqual(before);
});

test("a session form POST over MAX_BODY_BYTES is 413 before parsing", async () => {
  const res = await fetch(`${base}/settings/profile`, {
    method: "POST",
    redirect: "manual",
    headers: { cookie, origin: ORIGIN },
    body: new URLSearchParams({ csrf, display_name: "x".repeat(2 * MAX_BODY_BYTES) }),
  });
  expect(res.status).toBe(413);
});
