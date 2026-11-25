import { createServer } from "./app.ts";
import { loadConfig } from "./config.ts";
import { connect, migrate } from "./db.ts";
import { createLive } from "./live.ts";
import { log } from "./log.ts";
import { encrypted, requireSse, s3Storage } from "./storage.ts";
import { sweepBundles } from "./submissions.ts";

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
const bucket = s3Storage(cfg);
await requireSse(bucket); // refuses to start on a bucket without server-side encryption
const storage = encrypted(bucket, cfg.bundleKey);
const sweep = () =>
  sweepBundles(sql, storage).catch((e) =>
    log("error", "sweep failed", { reason: e instanceof Error ? e.message : String(e) }),
  );
void sweep();
const sweeper = setInterval(sweep, 3_600_000);
const live = createLive(sql);
const s = createServer({ sql, port: cfg.port, cfg, live, storage });
log("info", "listening", { port: s.port });

// Deploys drain in seconds: end every SSE stream first (browsers reconnect to the new process
// after `retry`), then give in-flight requests up to 10 s, then close the pool (up to 5 s). Always exits.
process.once("SIGTERM", async () => {
  log("info", "shutting down");
  try {
    clearInterval(sweeper);
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
