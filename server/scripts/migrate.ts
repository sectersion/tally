import { connect, migrate } from "../src/db.ts";

const sql = connect(process.env.DATABASE_URL ?? "");
console.log((await migrate(sql)).join("\n") || "up to date");
await sql.close();
