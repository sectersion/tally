// Local dev entry: like main.ts but with in-memory bundle storage (no S3/SSE probe).
// ponytail: memory storage loses bundles on restart; use main.ts for real deploys.
import { createServer } from "./app.ts";
import { loadConfig } from "./config.ts";
import { connect, migrate } from "./db.ts";
import { createLive } from "./live.ts";
import { log, setLogLevel } from "./log.ts";
import { memoryStorage } from "./storage.ts";
import { sweepBundles } from "./submissions.ts";

const cfg = loadConfig();
setLogLevel(cfg.logLevel);
const sql = connect(cfg.databaseUrl);
const applied = await migrate(sql);
if (applied.length) log("info", "migrated", { applied });
const storage = memoryStorage();
const sweep = () =>
  sweepBundles(sql, storage).catch((e) =>
    log("error", "sweep failed", { reason: e instanceof Error ? e.message : String(e) }),
  );
void sweep();
setInterval(sweep, 3_600_000);
const live = createLive(sql);
const s = createServer({ sql, port: cfg.port, cfg, live, storage });
log("info", "listening", { port: s.port });
