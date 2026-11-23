import { SQL } from "bun";
import { createServer } from "./app.ts";
import { loadConfig } from "./config.ts";
import { log } from "./log.ts";

const cfg = loadConfig();
const sql = new SQL(cfg.databaseUrl);
const s = createServer({ sql, port: cfg.port });
log("info", "listening", { port: s.port });
