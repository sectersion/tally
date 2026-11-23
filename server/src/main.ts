import { createServer } from "./app.ts";
import { loadConfig } from "./config.ts";
import { connect, migrate } from "./db.ts";
import { createLive } from "./live.ts";
import { log } from "./log.ts";

try {
  await import("./wasm.ts");
} catch {
  console.error("server/wasm missing: run bun run wasm");
  process.exit(1);
}

const cfg = loadConfig();
const sql = connect(cfg.databaseUrl);
const applied = await migrate(sql);
if (applied.length) log("info", "migrated", { applied });
const live = createLive(sql);
const s = createServer({ sql, port: cfg.port, cfg, live });
log("info", "listening", { port: s.port });

// Deploys drain in seconds: end every SSE stream first (browsers reconnect to the new process
// after `retry`), then let in-flight requests finish and close the pool.
process.once("SIGTERM", async () => {
  log("info", "shutting down");
  live.closeAll();
  await s.stop();
  await sql.close();
  process.exit(0);
});
