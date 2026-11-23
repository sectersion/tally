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
await appendEntries(sql, projectId, { prev_hash: GENESIS, entries: chains[name] ?? [] });
const uid = crypto.randomUUID();
await sql`insert into users (id, hca_sub, handle) values (${uid}, ${`sub-${uid}`}, ${name.split("/")[0]})`;
await sql`update projects set owner_id = ${uid} where id = ${projectId}`;
const rid = 900_000_000 + Math.floor(Math.random() * 1e6);
await sql`insert into github_repos (id, full_name) values (${rid}, ${name})`;
await sql`insert into project_repos (project_id, github_repo_id) values (${projectId}, ${rid})`;
for (let i = 0; i < 20; i++) {
  const d = `load-${rid}-${i}`;
  await sql`insert into webhook_deliveries (delivery_id, event) values (${d}, 'push')`;
  await sql`insert into push_receipts (delivery_id, sha, github_repo_id, project_id, ref, is_head, forced, claimed_ts)
    values (${d}, ${`${i}`.padStart(40, "a")}, ${rid}, ${projectId}, 'refs/heads/main', true, false, ${Date.now() - i * 60_000})`;
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
