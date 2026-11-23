/**
 * Whole-surface leak check: a user with distinctive private values, then every public route,
 * none of which may carry any of them. users has no slack id column, so there is none to seed.
 */
import { afterAll, expect, test } from "bun:test";
import { createHmac } from "node:crypto";
import { createServer } from "../src/app.ts";
import { hashToken } from "../src/auth.ts";
import { testConfig } from "../src/config.ts";
import { connect } from "../src/db.ts";
import { createProject } from "../src/projects.ts";

const SECRET = "privacy-secret";
const sql = connect(process.env.DATABASE_URL ?? "");
const srv = createServer({ cfg: testConfig({ githubWebhookSecret: SECRET }), sql, port: 0 });
const base = `http://localhost:${srv.port}`;
afterAll(async () => {
  srv.stop(true);
  await sql.close();
});

const tag = crypto.randomUUID().replaceAll("-", "").slice(0, 10);
const P = {
  email: "leak-check@example.invalid",
  sub: `ident!LEAKCHECK${tag}`,
  name: "LeakcheckRealName",
  verification_status: "verified-leakcheck",
  reason: "leakcheck_reason",
  feature: "leakcheck_feature",
  label: "leakcheck-label",
  message: "leakcheck-message",
  authorName: "Leakcheck Author",
  authorEmail: "leakcheck-author@example.invalid",
};
const NEEDLES = [...Object.values(P), "LEAKCHECK"];

async function firstEvent(url: string): Promise<string> {
  const ac = new AbortController();
  const res = await fetch(url, { signal: ac.signal });
  expect(res.status).toBe(200);
  const reader = res.body?.getReader() as ReadableStreamDefaultReader<Uint8Array>;
  const dec = new TextDecoder();
  let buf = "";
  try {
    while (!buf.includes("event: changed")) {
      const { value, done } = await reader.read();
      if (done) break;
      buf += dec.decode(value);
    }
  } finally {
    ac.abort();
  }
  expect(buf).toContain("event: changed");
  return buf;
}

test("no public route shows a private value, and receipts never store them", async () => {
  const uid = crypto.randomUUID();
  const handle = `lc${tag}`;
  await sql`insert into users (id, hca_sub, email, name, verification_status, handle, profile_public)
    values (${uid}, ${P.sub}, ${P.email}, ${P.name}, ${P.verification_status}, ${handle}, true)`;
  const name = `${handle}/leakcheck`;
  const r = await createProject(sql, uid, name);
  if (!("id" in r)) throw new Error("setup");
  const pid = r.id;

  // Ledger: one entry whose reasons and features carry the needles.
  const [c] = await sql`insert into chains (project_id, ruleset, head_hash, count, verified_s)
    values (${pid}, ${`r${tag}`}, ${"0".repeat(64)}, 1, 1800) returning id`;
  const entry = { reasons: [P.reason], features: { [P.feature]: 7 } };
  await sql`insert into ledger_entries (chain_id, seq, hash, prev_hash, action_id, ts, kind, verified_s, unverified_s, entry)
    values (${c.id}, 0, ${`${tag}`.padStart(64, "0")}, ${"0".repeat(64)}, 'a0', ${Date.now() - 60_000}, 'prompt', 1800, 60, ${JSON.stringify(entry)}::json)`;

  // Token labels, project and user scoped.
  await sql`insert into device_tokens (project_id, token_hash, label) values (${pid}, ${hashToken(`p${tag}`)}, ${P.label})`;
  await sql`insert into device_tokens (user_id, token_hash, label) values (${uid}, ${hashToken(`u${tag}`)}, ${P.label})`;

  // A linked repo and a validly signed push through the real webhook route.
  const repo = Math.floor(Math.random() * 1e12);
  const inst = Math.floor(Math.random() * 1e12);
  await sql`insert into github_installations (id, account_login) values (${inst}, 'leak')`;
  await sql`insert into github_repos (id, full_name, installation_id) values (${repo}, ${`leak/r${tag}`}, ${inst})`;
  await sql`insert into project_repos (project_id, github_repo_id) values (${pid}, ${repo})`;
  const sha = "d".repeat(40);
  const commit = {
    id: sha,
    timestamp: new Date(Date.now() - 30_000).toISOString(),
    message: P.message,
    author: { name: P.authorName, email: P.authorEmail },
    committer: { name: P.authorName, email: P.authorEmail },
  };
  const body = JSON.stringify({
    ref: "refs/heads/main",
    before: "a".repeat(40),
    after: sha,
    forced: false,
    deleted: false,
    created: false,
    repository: { id: repo, full_name: `leak/r${tag}`, name: `r${tag}`, owner: { login: "leak" } },
    installation: { id: inst },
    sender: { login: "leak", id: 1 },
    pusher: { name: P.authorName, email: P.authorEmail },
    commits: [commit],
    head_commit: commit,
  });
  const delivery = crypto.randomUUID();
  const hook = await fetch(`${base}/github/webhook`, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      "x-github-event": "push",
      "x-github-delivery": delivery,
      "x-hub-signature-256": `sha256=${createHmac("sha256", SECRET).update(body).digest("hex")}`,
    },
    body,
  });
  expect(hook.status).toBe(200);
  expect(await hook.json()).toEqual({ ok: true });
  const [{ n }] =
    await sql`select count(*)::int as n from push_receipts where delivery_id = ${delivery} and project_id = ${pid}`;
  expect(n).toBe(1); // the push really landed, so the page has a commit to show

  const routes = [
    "/",
    `/u/${handle}`,
    `/p/${name}`,
    `/p/${name}/fragment`,
    `/badge/${name}.svg`,
    `/badge/${name}.json`,
    `/v1/projects/${pid}/badge.json`,
    `/v1/projects/${pid}/summary`,
  ];
  const seen: string[] = [];
  for (const path of routes) {
    const res = await fetch(`${base}${path}`);
    expect([path, res.status]).toEqual([path, 200]);
    seen.push(`${path}\n${JSON.stringify([...res.headers])}\n${await res.text()}`);
  }
  seen.push(await firstEvent(`${base}/p/${name}/events`));
  expect(seen.find((t) => t.startsWith(`/p/${name}\n`))).toContain(name); // a real page, not an error
  const leaks = seen.flatMap((t) =>
    NEEDLES.filter((bad) => t.includes(bad)).map((bad) => `${t.split("\n")[0]}: ${bad}`),
  );
  expect(leaks).toEqual([]);

  // Stored evidence keeps no message, author name or email.
  for (const table of ["push_receipts", "webhook_deliveries"]) {
    const rows = await sql.unsafe(`select t::text as row from ${table} t`);
    const all = rows.map((x: { row: string }) => x.row).join("\n");
    for (const bad of [P.message, P.authorEmail, P.authorName]) expect(all).not.toContain(bad);
  }
});
