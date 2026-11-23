import { afterAll, expect, test } from "bun:test";
import { createHmac } from "node:crypto";
import { readFileSync } from "node:fs";
import { createServer } from "../src/app.ts";
import { MAX_BODY_BYTES, testConfig } from "../src/config.ts";
import { connect } from "../src/db.ts";
import { witnessedCommits } from "../src/github.ts";

const SECRET = "test-secret";
type Fixture = { headers: Record<string, string>; body: string };
const fx = (n: string): Fixture =>
  JSON.parse(readFileSync(new URL(`./fixtures/github/${n}`, import.meta.url), "utf8"));
const sql = connect(process.env.DATABASE_URL ?? "");
const srv = createServer({
  cfg: testConfig({ githubWebhookSecret: SECRET }),
  sql,
  port: 0,
});
const base = `http://localhost:${srv.port}`;
afterAll(async () => {
  srv.stop(true);
  await sql.close();
});

const REPO = 1001;
const INST = 5001;
const SHAS = [
  "cccccccccccccccccccccccccccccccccccccccc",
  "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
];
const sign = (body: string) => `sha256=${createHmac("sha256", SECRET).update(body).digest("hex")}`;

/** Posts a fixture's raw body and headers. A fresh delivery id by default, so fixtures are reusable. */
async function deliver(
  name: string,
  o: { delivery?: string; body?: string; headers?: Record<string, string | null> } = {},
) {
  const f = fx(name);
  const body = o.body ?? f.body;
  const headers: Record<string, string> = {
    ...f.headers,
    "x-github-delivery": o.delivery ?? crypto.randomUUID(),
    ...(o.body === undefined ? {} : { "x-hub-signature-256": sign(body) }),
  };
  for (const [k, v] of Object.entries(o.headers ?? {})) {
    if (v === null) delete headers[k];
    else headers[k] = v;
  }
  const res = await fetch(`${base}/github/webhook`, { method: "POST", headers, body });
  return { res, delivery: headers["x-github-delivery"] as string };
}

/** Repo 1001 on installation 5001, live-linked to a new project (reset each call so reruns work). */
async function linked() {
  await sql`insert into github_installations (id, account_login) values (${INST}, 'octo')
    on conflict (id) do update set revoked_at = null`;
  await sql`insert into github_repos (id, full_name, installation_id) values (${REPO}, 'octo/scratch', ${INST})
    on conflict (id) do update set full_name = 'octo/scratch', installation_id = ${INST}`;
  await sql`update project_repos set unlinked_at = now() where github_repo_id = ${REPO} and unlinked_at is null`;
  const id = crypto.randomUUID();
  const u = crypto.randomUUID();
  await sql`insert into users (id, hca_sub, handle) values (${u}, ${`sub!${u}`}, ${`u${u.replaceAll("-", "").slice(0, 12)}`})`;
  await sql`insert into projects (id, owner_id, public_name, public) values (${id}, ${u}, ${`w/${id}`}, true)`;
  await sql`insert into project_repos (project_id, github_repo_id) values (${id}, ${REPO})`;
  return { id, u };
}
const unlinkedRepo = async () => {
  await linked();
  await sql`update project_repos set unlinked_at = now() where github_repo_id = ${REPO} and unlinked_at is null`;
};
const count = async (d: string) =>
  Number(
    (await sql`select count(*)::int as n from webhook_deliveries where delivery_id = ${d}`)[0].n,
  );
const seq = async (id: string) =>
  Number((await sql`select change_seq from projects where id = ${id}`)[0].change_seq);

test("bad signature is 401 and writes nothing", async () => {
  await linked();
  const f = fx("push.push.json");
  const sig = sign(f.body);
  const ok = await deliver("push.push.json", { headers: { "x-hub-signature-256": sig } });
  expect(ok.res.status).toBe(200); // control: the genuine body passes
  const d = crypto.randomUUID();
  const r = await fetch(`${base}/github/webhook`, {
    method: "POST",
    headers: { "x-github-event": "push", "x-github-delivery": d, "x-hub-signature-256": sig },
    body: f.body.replace("octo/scratch", "octo/scratcH"), // one byte flipped
  });
  expect(r.status).toBe(401);
  expect(await r.json()).toEqual({ error: "bad_signature" });
  expect(await count(d)).toBe(0);
  const short = await deliver("push.push.json", {
    headers: { "x-hub-signature-256": "sha256=abcd" },
  });
  expect(short.res.status).toBe(401);
  const sha1 = await deliver("push.push.json", {
    headers: { "x-hub-signature-256": null, "x-hub-signature": `sha1=${"0".repeat(40)}` },
  });
  expect(sha1.res.status).toBe(401);
});

test("missing signature is 401 and nothing is parsed or written", async () => {
  const { res, delivery } = await deliver("push.push.json", {
    headers: { "x-hub-signature-256": null },
  });
  expect(res.status).toBe(401);
  expect(await count(delivery)).toBe(0);
  // Not JSON at all, unsigned: still 401 (not 400), so nothing was parsed.
  const r = await fetch(`${base}/github/webhook`, {
    method: "POST",
    headers: { "x-github-event": "push", "x-github-delivery": crypto.randomUUID() },
    body: "{not json",
  });
  expect(r.status).toBe(401);
});

test("body over the cap is 413", async () => {
  const body = "x".repeat(MAX_BODY_BYTES + 1);
  const { res, delivery } = await deliver("push.push.json", { body });
  expect(res.status).toBe(413);
  expect(await count(delivery)).toBe(0);
});

test("push to a linked repo stores receipts with server time, no message or author", async () => {
  const p = await linked();
  const before = Date.now();
  const s0 = await seq(p.id);
  const { res, delivery } = await deliver("push.push.json");
  expect(res.status).toBe(200);
  expect(await res.json()).toEqual({ ok: true });
  const rows = await sql`select * from push_receipts where delivery_id = ${delivery} order by sha`;
  expect(rows.length).toBe(2); // the head is also in commits[]: one row, marked head
  expect(rows.map((r: { sha: string }) => r.sha)).toEqual([...SHAS].sort());
  expect(rows.filter((r: { is_head: boolean }) => r.is_head).length).toBe(1);
  expect(rows.every((r: { project_id: string }) => r.project_id === p.id)).toBe(true);
  expect(rows[0].received_at.getTime()).toBeGreaterThanOrEqual(before - 1000);
  expect(rows[0].received_at.getTime()).toBeLessThanOrEqual(Date.now() + 1000);
  expect(Number(rows.find((r: { sha: string }) => r.sha === SHAS[0])?.claimed_ts)).toBe(
    Date.parse("2026-11-23T10:00:00Z"),
  );
  const dump = JSON.stringify(rows);
  for (const w of ["message", "author", "committer", "pusher", "invented", "example.com"])
    expect(dump).not.toContain(w);
  expect(await seq(p.id)).toBe(s0 + 1);
});

test("redelivery with the same delivery id is a duplicate and keeps the first time", async () => {
  await linked();
  const first = await deliver("push.push.json");
  const [a] =
    await sql`select received_at from push_receipts where delivery_id = ${first.delivery} limit 1`;
  await Bun.sleep(20);
  const again = await deliver("push.push.json", { delivery: first.delivery });
  expect(again.res.status).toBe(200);
  expect(await again.res.json()).toEqual({ duplicate: true });
  const rows =
    await sql`select received_at from push_receipts where delivery_id = ${first.delivery}`;
  expect(rows.length).toBe(2);
  expect(rows[0].received_at.getTime()).toBe(a.received_at.getTime());
});

test("same push under a new delivery id: the page shows the earliest receipt", async () => {
  const p = await linked();
  await deliver("push.push.json");
  const [first] =
    await sql`select min(received_at) as t from push_receipts where project_id = ${p.id}`;
  await Bun.sleep(20);
  await deliver("push.push.json");
  expect(
    Number(
      (await sql`select count(*)::int as n from push_receipts where project_id = ${p.id}`)[0].n,
    ),
  ).toBe(4);
  const w = await witnessedCommits(sql, p.id);
  expect(w.map((x) => x.sha).sort()).toEqual([...SHAS].sort());
  expect(Math.max(...w.map((x) => x.received_at.getTime()))).toBe(first.t.getTime());
});

test("push for an unlinked repo is stored with null project and is not on the page", async () => {
  await unlinkedRepo();
  const { res, delivery } = await deliver("push.push.json");
  expect(res.status).toBe(200);
  const rows = await sql`select project_id from push_receipts where delivery_id = ${delivery}`;
  expect(rows.length).toBe(2);
  expect(rows.every((r: { project_id: string | null }) => r.project_id === null)).toBe(true);
});

test("branch deletion push stores no receipt", async () => {
  await linked();
  const body = { ...JSON.parse(fx("push.push.json").body), deleted: true, commits: [] };
  const { res, delivery } = await deliver("push.push.json", { body: JSON.stringify(body) });
  expect(res.status).toBe(200);
  expect(await count(delivery)).toBe(1);
  expect((await sql`select 1 from push_receipts where delivery_id = ${delivery}`).length).toBe(0);
});

test("unknown events are ignored but the delivery is recorded", async () => {
  const { res, delivery } = await deliver("push.push.json", {
    headers: { "x-github-event": "ping" },
  });
  expect(res.status).toBe(200);
  expect(await res.json()).toEqual({ ignored: true });
  expect(await count(delivery)).toBe(1);
});

test("installation deleted unlinks repos, keeps receipts, records repo_events", async () => {
  const p = await linked();
  await deliver("push.push.json");
  const hoursBefore = await sql`select count(*)::int as n from chains where project_id = ${p.id}`;
  const { res } = await deliver("installation.deleted.json");
  expect(res.status).toBe(200);
  const [i] = await sql`select revoked_at from github_installations where id = ${INST}`;
  expect(i.revoked_at).not.toBeNull();
  expect(
    (await sql`select 1 from project_repos where github_repo_id = ${REPO} and unlinked_at is null`)
      .length,
  ).toBe(0);
  const ev =
    await sql`select kind from repo_events where github_repo_id = ${REPO} order by id desc limit 1`;
  expect(ev[0].kind).toBe("uninstalled");
  expect((await sql`select 1 from push_receipts where project_id = ${p.id}`).length).toBe(2);
  expect(await sql`select count(*)::int as n from chains where project_id = ${p.id}`).toEqual(
    hoursBefore,
  );
});

test("installation_repositories removed unlinks the repo", async () => {
  await linked();
  const { res } = await deliver("installation_repositories.removed.json");
  expect(res.status).toBe(200);
  expect(
    (await sql`select 1 from project_repos where github_repo_id = ${REPO} and unlinked_at is null`)
      .length,
  ).toBe(0);
  const ev =
    await sql`select kind from repo_events where github_repo_id = ${REPO} order by id desc limit 1`;
  expect(ev[0].kind).toBe("removed");
});

test("github_app_authorization revoked deletes the stored user token", async () => {
  const u = crypto.randomUUID();
  await sql`insert into users (id, hca_sub) values (${u}, ${`sub!${u}`})`;
  await sql`delete from github_user_tokens where github_user_id = 42`;
  await sql`insert into github_user_tokens (user_id, github_user_id, access_token, expires_at)
    values (${u}, 42, 'tok', now() + interval '1 hour')`;
  const { res } = await deliver("github_app_authorization.revoked.json");
  expect(res.status).toBe(200);
  expect((await sql`select 1 from github_user_tokens where user_id = ${u}`).length).toBe(0);
});

test("repository renamed updates full_name by id and keeps receipts", async () => {
  const p = await linked();
  await sql`update projects set public_name = ${`keep/${p.id}`} where id = ${p.id}`;
  await deliver("push.push.json");
  const { res } = await deliver("repository.renamed.json");
  expect(res.status).toBe(200);
  const [r] = await sql`select full_name from github_repos where id = ${REPO}`;
  expect(r.full_name).toBe("octo/renamed");
  const [pn] = await sql`select public_name from projects where id = ${p.id}`;
  expect(pn.public_name).toBe(`keep/${p.id}`);
  expect((await sql`select 1 from push_receipts where project_id = ${p.id}`).length).toBe(2);
  const [ev] =
    await sql`select kind, detail from repo_events where github_repo_id = ${REPO} order by id desc limit 1`;
  expect(ev).toEqual({ kind: "renamed", detail: "octo/scratch -> octo/renamed" });
});

test("transferred keeps the link and follows the new installation", async () => {
  const p = await linked();
  const { res } = await deliver("repository.transferred.json");
  expect(res.status).toBe(200);
  const [r] = await sql`select full_name, installation_id from github_repos where id = ${REPO}`;
  expect(r.full_name).toBe("newowner/scratch");
  expect(Number(r.installation_id)).toBe(5002);
  expect(
    (await sql`select 1 from project_repos where project_id = ${p.id} and unlinked_at is null`)
      .length,
  ).toBe(1);
});

test("renames of unknown repos do nothing", async () => {
  await sql`delete from repo_events where github_repo_id = 999999`;
  const body = fx("repository.renamed.json").body.replace("1001", "999999");
  const { res } = await deliver("repository.renamed.json", { body });
  expect(res.status).toBe(200);
  expect((await sql`select 1 from github_repos where id = 999999`).length).toBe(0);
  expect((await sql`select 1 from repo_events where github_repo_id = 999999`).length).toBe(0);
});

test("receipts cannot be updated or deleted", async () => {
  await linked();
  const { delivery } = await deliver("push.push.json");
  await expect(
    sql`update push_receipts set ref = 'x' where delivery_id = ${delivery}`,
  ).rejects.toThrow(/append-only/);
  await expect(sql`delete from push_receipts where delivery_id = ${delivery}`).rejects.toThrow(
    /append-only/,
  );
  await expect(sql`delete from webhook_deliveries where delivery_id = ${delivery}`).rejects.toThrow(
    /append-only/,
  );
});

test("a stale removal from the old installation after a transfer keeps the link", async () => {
  const p = await linked();
  await deliver("repository.transferred.json"); // repo now on installation 5002
  await sql`delete from repo_events where github_repo_id = ${REPO}`; // earlier tests' rows; the table is not append-only
  const { res } = await deliver("installation_repositories.removed.json"); // from 5001
  expect(res.status).toBe(200);
  expect(
    (await sql`select 1 from project_repos where project_id = ${p.id} and unlinked_at is null`)
      .length,
  ).toBe(1);
  const ev =
    await sql`select 1 from repo_events where github_repo_id = ${REPO} and kind = 'removed'`;
  expect(ev.length).toBe(0);
});

test("a push whose after is all zeros stores nothing", async () => {
  await linked();
  const body = { ...JSON.parse(fx("push.push.json").body), after: "0".repeat(40), commits: [] };
  const { res, delivery } = await deliver("push.push.json", { body: JSON.stringify(body) });
  expect(res.status).toBe(200);
  expect((await sql`select 1 from push_receipts where delivery_id = ${delivery}`).length).toBe(0);
});

test("a failing dispatch rolls back the delivery row, and the redelivery is processed", async () => {
  const p = await linked();
  const bad = JSON.stringify({ ...JSON.parse(fx("push.push.json").body), commits: "nope" });
  const { res, delivery } = await deliver("push.push.json", { body: bad });
  expect(res.status).toBe(400);
  expect(await count(delivery)).toBe(0);
  const ok = await deliver("push.push.json", { delivery });
  expect(ok.res.status).toBe(200);
  expect(await count(delivery)).toBe(1);
  expect((await sql`select 1 from push_receipts where project_id = ${p.id}`).length).toBe(2);
});

test("summary last_commit_at is the newest receipt's server time", async () => {
  const p = await linked();
  await deliver("push.push.json");
  const [t] = await sql`select max(received_at) as t from push_receipts where project_id = ${p.id}`;
  const body = (await (await fetch(`${base}/v1/projects/${p.id}/summary`)).json()) as {
    last_commit_at: string;
  };
  expect(new Date(body.last_commit_at).getTime()).toBe(t.t.getTime());
});
