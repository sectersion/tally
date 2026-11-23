import { afterAll, expect, test } from "bun:test";
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
