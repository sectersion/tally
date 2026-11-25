/** Dev server: same app, no S3/SSE requirement (submissions answer 503 without storage).
 *  Usage: bun scripts/dev.ts  (env: PORT, DATABASE_URL) */
import { createServer } from "../src/app.ts";
import { loadConfig } from "../src/config.ts";
import { connect, migrate } from "../src/db.ts";
import { createLive } from "../src/live.ts";
import { log, setLogLevel } from "../src/log.ts";
import { encrypted, s3Storage } from "../src/storage.ts";

const cfg = loadConfig();
setLogLevel(cfg.logLevel);
const sql = connect(cfg.databaseUrl);
const applied = await migrate(sql);
if (applied.length) log("info", "migrated", { applied });
// ponytail: dev-only storage; submissions routes fail on write, add MinIO when testing uploads locally
const bucket = s3Storage(cfg);
const storage = encrypted(bucket, cfg.bundleKey);
const live = createLive(sql);
const s = createServer({ sql, port: cfg.port, cfg, live, storage });
log("info", "listening", { port: s.port });
