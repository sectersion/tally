import { createServer } from "./app.ts";
import { loadConfig } from "./config.ts";
import { connect, migrate } from "./db.ts";
import { createLive } from "./live.ts";
import { log, setLogLevel } from "./log.ts";

try {
  await import("./wasm.ts");
} catch {
  console.error("server/wasm missing: run bun run wasm");
  process.exit(1);
}

const cfg = loadConfig();
setLogLevel(cfg.logLevel);
const sql = connect(cfg.databaseUrl);
const applied = await migrate(sql);
if (applied.length) log("info", "migrated", { applied });
const live = createLive(sql);
const s = createServer({ sql, port: cfg.port, cfg, live });
log("info", "listening", { port: s.port });

// Deploys drain in seconds: end every SSE stream first (browsers reconnect to the new process
// after `retry`), then give in-flight requests up to 10 s, then close the pool (up to 5 s). Always exits.
process.once("SIGTERM", async () => {
  log("info", "shutting down");
  try {
    live.closeAll();
    await Promise.race([s.stop(), Bun.sleep(10_000)]);
    await s.stop(true);
    await Promise.race([sql.close(), Bun.sleep(5_000)]);
  } catch (e) {
    log("error", "shutdown failed", { reason: e instanceof Error ? e.message : String(e) });
  } finally {
    process.exit(0);
  }
});
