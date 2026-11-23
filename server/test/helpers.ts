import type { SQL } from "bun";
import { createServer } from "../src/app.ts";
import { createProjectToken } from "../src/auth.ts";
import { testConfig } from "../src/config.ts";
import type { Limiter } from "../src/ratelimit.ts";
import type { LedgerEntry } from "../src/schema.ts";
import fixture from "./fixtures/chain.json";

// Fixture claims (acme/demo-N): each name is usable once per database (run `bun run db:reset` locally).
// | N          | claimed by              |
// | 1          | wasm and schema tests   |
// | 2..7, -alt | Task 6 (upload)         |
// | 3-2099     | Task 6 (upload)         |
// | 8          | Task 7                  |
// | 9          | Task 8                  |
// | 10         | entries: first batch    |
// | 11         | badge: stored sums      |
// | 12         | entries: mixed rulesets |
// | 13         | entries: mid-chain resume |
// | 14-big     | entries: 3e9 verified_s |
// | 15         | projects: change_seq bump |
// | 16         | projects: hours vs chains |
// | 17, 18     | live: upload, second process |

export const chains = fixture as unknown as Record<string, LedgerEntry[]>;

export async function seed(sql: SQL, publicName = `t/${crypto.randomUUID()}`) {
  const { projectId, token } = await createProjectToken(sql, publicName);
  return { projectId, token, publicName };
}

export const seedFixture = (sql: SQL, name: string) => seed(sql, name);

export function start(sql: SQL, limiter?: Limiter) {
  const s = createServer({ cfg: testConfig(), sql, port: 0, limiter });
  return { url: `http://localhost:${s.port}`, stop: () => s.stop(true) };
}
