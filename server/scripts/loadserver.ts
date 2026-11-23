// Scratch server for scripts/load.ts: huge rate limits, TRUST_PROXY (so each load client can pick
// its own x-forwarded-for and dodge the 4-streams-per-ip cap), one seeded project, and
// GET :PORT+1/stats -> {rss, streams}. Never deploy this.
import { createServer } from "../src/app.ts";
import { createProjectToken } from "../src/auth.ts";
import { testConfig } from "../src/config.ts";
import { connect, migrate } from "../src/db.ts";
import { appendEntries, GENESIS } from "../src/ledger.ts";
import { createLive } from "../src/live.ts";
import { chains } from "../test/helpers.ts";

const port = Number(process.env.PORT ?? 8099);
const sql = connect(process.env.DATABASE_URL ?? "");
await migrate(sql);
const name = process.env.LOAD_NAME ?? "acme/demo-1";
const { projectId } = await createProjectToken(sql, name);
const fx = chains[name];
if (fx) await appendEntries(sql, projectId, { prev_hash: GENESIS, entries: fx });
const uid = crypto.randomUUID();
await sql`insert into users (id, hca_sub, handle) values (${uid}, ${`sub-${uid}`}, ${name.split("/")[0]})`;
await sql`update projects set owner_id = ${uid} where id = ${projectId}`;
const rid = 900_000_000 + Math.floor(Math.random() * 1e6);
await sql`insert into github_repos (id, full_name) values (${rid}, ${name})`;
await sql`insert into project_repos (project_id, github_repo_id) values (${projectId}, ${rid})`;
// LOAD_RECEIPTS receipts (default 20) and LOAD_ENTRIES extra synthetic ledger entries (default 0),
// both spread over the last 30 days, for measuring the page on a bigger project.
const DAY = 86_400_000;
const nr = Number(process.env.LOAD_RECEIPTS ?? 20);
for (let i = 0; i < nr; i++) {
  const d = `load-${rid}-${i}`;
  const at = Date.now() - Math.floor((i * 30 * DAY) / nr);
  await sql`insert into webhook_deliveries (delivery_id, event) values (${d}, 'push')`;
  await sql`insert into push_receipts (delivery_id, sha, github_repo_id, project_id, ref, is_head, forced, claimed_ts, received_at)
    values (${d}, ${`${i}`.padStart(40, "a")}, ${rid}, ${projectId}, 'refs/heads/main', true, false, ${at - 60_000}, ${new Date(at)})`;
}
const ne = Number(process.env.LOAD_ENTRIES ?? 0);
if (ne > 0) {
  const [c] =
    await sql`insert into chains (project_id, ruleset, head_hash) values (${projectId}, 'load', ${GENESIS}) returning id`;
  const k = Array.from({ length: ne }, (_, i) => i);
  await sql`insert into ledger_entries (chain_id, seq, hash, prev_hash, action_id, ts, kind, verified_s, unverified_s, entry)
    select ${c.id}, s, lpad(s::text, 64, '0'), ${GENESIS}, 'a' || s, ts, 'k', 60, 0, '{}'::json
    from unnest(${k}::bigint[], ${k.map((i) => Date.now() - Math.floor((i * 30 * DAY) / ne))}::bigint[]) as t(s, ts)`;
}
const big = { perMin: 1e9, burst: 1e9 };
const live = createLive(sql);
createServer({
  sql,
  port,
  cfg: testConfig({ trustProxy: true }),
  live,
  limits: { auth: big, device: big, form: big, page: big, badge: big },
});
Bun.serve({
  port: port + 1,
  fetch: () => Response.json({ rss: process.memoryUsage().rss, ...live.size() }),
});
console.log(`ready ${name} project ${projectId}`);
