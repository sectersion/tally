import { readdirSync } from "node:fs";
import { SQL } from "bun";

export function connect(url: string): SQL {
  // Fail fast instead of queueing on a held chain lock with a pool connection (R2E-2).
  return new SQL(url, { connection: { lock_timeout: "2000", statement_timeout: "10000" } });
}

export async function migrate(
  sql: SQL,
  dir = new URL("../migrations/", import.meta.url),
): Promise<string[]> {
  await sql`create table if not exists schema_migrations (name text primary key, applied_at timestamptz not null default now())`;
  const files = readdirSync(dir)
    .filter((f) => /^\d{3}_.*\.sql$/.test(f))
    .sort();
  const applied: string[] = [];
  for (const name of files) {
    const text = await Bun.file(new URL(name, dir)).text();
    await sql.begin(async (tx) => {
      // serialize concurrent migrators; re-check inside the lock
      await tx`select pg_advisory_xact_lock(7461)`;
      const [done] = await tx`select 1 as x from schema_migrations where name = ${name}`;
      if (done) return;
      await tx.unsafe(text);
      await tx`insert into schema_migrations (name) values (${name})`;
      applied.push(name);
    });
  }
  return applied;
}
