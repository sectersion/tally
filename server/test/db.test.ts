import { afterAll, expect, test } from "bun:test";
import { copyFileSync, mkdtempSync, readdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { connect, migrate } from "../src/db.ts";

const sql = connect(process.env.DATABASE_URL ?? "");
afterAll(() => sql.close());

async function seed(ruleset = "r1") {
  const pid = crypto.randomUUID();
  await sql`insert into projects (id, public_name) values (${pid}, ${`t/${crypto.randomUUID()}`})`;
  const [c] =
    await sql`insert into chains (project_id, ruleset, head_hash) values (${pid}, ${ruleset}, 'h') returning id`;
  return { pid, id: c.id };
}

test("migrate is idempotent", async () => {
  await migrate(sql);
  expect(await migrate(sql)).toEqual([]);
});

test("ledger_entries rejects UPDATE, DELETE and TRUNCATE", async () => {
  const { id } = await seed();
  await sql`insert into ledger_entries (chain_id, seq, hash, prev_hash, action_id, ts, kind, verified_s, unverified_s, entry)
    values (${id}, 0, 'h0', 'h', 'a', 1, 'k', 1, 0, '{}')`;
  await expect(
    sql`update ledger_entries set verified_s = 0 where chain_id = ${id}`,
  ).rejects.toThrow(/append-only/);
  await expect(sql`delete from ledger_entries where chain_id = ${id}`).rejects.toThrow(
    /append-only/,
  );
  await expect(sql`truncate ledger_entries`).rejects.toThrow(/append-only/);
});

test("commits rejects UPDATE, DELETE and TRUNCATE", async () => {
  const { pid } = await seed();
  await sql`insert into commits (project_id, hash, ts) values (${pid}, 'c', 1)`;
  await expect(sql`update commits set ts = 2 where project_id = ${pid}`).rejects.toThrow(
    /append-only/,
  );
  await expect(sql`delete from commits where project_id = ${pid}`).rejects.toThrow(/append-only/);
  await expect(sql`truncate commits`).rejects.toThrow(/append-only/);
});

test("public_name is unique", async () => {
  const name = `t/${crypto.randomUUID()}`;
  await sql`insert into projects (id, public_name) values (${crypto.randomUUID()}, ${name})`;
  await expect(
    sql`insert into projects (id, public_name) values (${crypto.randomUUID()}, ${name})`,
  ).rejects.toThrow();
});

test("one chain per project and ruleset", async () => {
  const { pid } = await seed("r1");
  await expect(
    sql`insert into chains (project_id, ruleset, head_hash) values (${pid}, 'r1', 'h')`,
  ).rejects.toThrow();
  await sql`insert into chains (project_id, ruleset, head_hash) values (${pid}, 'r2', 'h')`;
});

test("pool sets lock_timeout and statement_timeout", async () => {
  const [r] =
    await sql`select current_setting('lock_timeout') as l, current_setting('statement_timeout') as s`;
  expect([r.l, r.s]).toEqual(["2s", "10s"]);
});

test("migration 103 unlinks all but the newest live repo of a double-linked project", async () => {
  const name = `mig103_${crypto.randomUUID().replaceAll("-", "").slice(0, 12)}`;
  const url = new URL(process.env.DATABASE_URL ?? "");
  await sql.unsafe(`create database ${name}`);
  url.pathname = `/${name}`;
  const db = connect(url.toString());
  try {
    const dir = mkdtempSync(join(tmpdir(), "mig103-"));
    const src = new URL("../migrations/", import.meta.url);
    for (const f of readdirSync(src)) if (f < "103") copyFileSync(new URL(f, src), join(dir, f));
    await migrate(db, pathToFileURL(`${dir}/`));
    await db`insert into projects (id, public_name) values ('p', 't/p')`;
    await db`insert into github_repos (id, full_name) values (1,'a/b'),(2,'a/c')`;
    await db`insert into project_repos (project_id, github_repo_id, linked_at) values ('p',1,now() - interval '1 day'),('p',2,now())`;
    await migrate(db);
    const live =
      await db`select github_repo_id::int as id from project_repos where unlinked_at is null`;
    expect(live.map((r: { id: number }) => r.id)).toEqual([2]);
  } finally {
    await db.close();
    await sql.unsafe(`drop database ${name}`);
  }
});
