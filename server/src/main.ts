import { createServer } from "./app.ts";
import { loadConfig } from "./config.ts";
import { connect, migrate } from "./db.ts";
import { log } from "./log.ts";

const cfg = loadConfig();
const sql = connect(cfg.databaseUrl);
const applied = await migrate(sql);
if (applied.length) log("info", "migrated", { applied });
const s = createServer({ sql, port: cfg.port });
log("info", "listening", { port: s.port });
