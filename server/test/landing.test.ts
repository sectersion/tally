import { describe, expect, test } from "bun:test";
import { connect, migrate } from "../src/db.ts";
import { CSP } from "../src/html.ts";
import { landingPage } from "../src/pages.ts";
import { seed } from "./helpers.ts";

const sql = await (async () => {
  const s = await connect(
    process.env.DATABASE_URL ?? "postgres://postgres:postgres@localhost:5432/postgres",
  );
  await migrate(s);
  return s;
})();

describe("landingPage", () => {
  test("structure and no script/inline style/csrf", async () => {
    const res = await landingPage(sql);
    const body = await res.text();
    expect((body.match(/<h1/g) ?? []).length).toBe(1);
    expect((body.match(/href="\/auth\/login"/g) ?? []).length).toBe(1);
    expect(body).toContain("Read the code");
    expect(body).toContain("github.com/Sectersion/tally");
    expect(body).not.toContain("<script");
    expect(body).not.toContain("csrf");
    expect(body).not.toContain("style=");
  });
  test("console mockup is aria-hidden", async () => {
    const body = await (await landingPage(sql)).text();
    expect(body).toContain('<div class="console" aria-hidden="true">');
  });
  test("CSP self-hosts fonts on all pages; landing equals base CSP", async () => {
    const res = await landingPage(sql);
    expect(res.headers.get("content-security-policy")).toBe(CSP);
    expect(CSP).toContain("font-src 'self'");
    expect(res.headers.get("content-type")).toContain("text/html");
  });
  test("landing shows real verified minutes from the ledger, not placeholders", async () => {
    const { projectId } = await seed(sql, `t/landing-${crypto.randomUUID()}`);
    const hash = `landing${crypto.randomUUID()}`.slice(0, 32);
    const [chain] = await sql`insert into chains (project_id, ruleset, head_hash, count, verified_s)
      values (${projectId}, 'test', ${hash}, 1, 300) returning id`;
    await sql.begin(async (tx) => {
      await tx`alter table ledger_entries disable trigger ledger_entries_no_mutate`;
      await tx`insert into ledger_entries (chain_id, seq, hash, prev_hash, action_id, ts, kind, verified_s, unverified_s, entry)
        values (${chain.id}, 0, ${hash}, ${hash + "-p"}, ${hash + "-a"}, 1000, 'edit', 300, 0, '{}'::json)`;
      await tx`alter table ledger_entries enable trigger ledger_entries_no_mutate`;
    });
    const body = await (await landingPage(sql)).text();
    // No exact sum: other test chains share the DB. Placeholder is gone and our 5 minutes are included.
    expect(body).not.toContain("128,447");
    expect(body).not.toContain("%VERIFIED_MIN%");
    const [c] = await sql`select sum(verified_s) as s from chains`;
    expect(body).toContain(
      `<span>${Math.floor(Number(c.s) / 60).toLocaleString("en-US")} minutes verified so far</span>`,
    );
    await sql.begin(async (tx) => {
      await tx`alter table ledger_entries disable trigger ledger_entries_no_mutate`;
      await tx`delete from ledger_entries where chain_id = ${chain.id}`;
      await tx`alter table ledger_entries enable trigger ledger_entries_no_mutate`;
    });
    await sql`delete from chains where id = ${chain.id}`;
  });
});
