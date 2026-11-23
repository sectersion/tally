import { connect, migrate } from "../src/db.ts";

const [handle, role] = process.argv.slice(2);
if (!handle || !["user", "reviewer", "admin"].includes(role ?? "")) {
  console.error("usage: user-role.ts <handle> user|reviewer|admin");
  process.exit(2);
}

const sql = connect(process.env.DATABASE_URL ?? "");
await migrate(sql);
const rows =
  await sql`update users set role = ${role} where lower(handle) = lower(${handle}) returning id`;
console.log(rows.length ? `${handle} is now ${role}` : "no such handle");
await sql.close();
process.exit(rows.length ? 0 : 1);
