import { connect, migrate } from "../src/db.ts";

const sql = connect(process.env.DATABASE_URL ?? "");
await migrate(sql);
await sql.close();
