export type LogLevel = "debug" | "info" | "warn" | "error";
export type Config = {
  port: number;
  databaseUrl: string;
  logLevel: LogLevel;
  publicBaseUrl: string;
  hcaClientId: string;
  hcaClientSecret: string;
  githubAppSlug: string;
  githubAppClientId: string;
  githubAppClientSecret: string;
  githubWebhookSecret: string;
};

export const MAX_BODY_BYTES = 1_048_576;
export const MAX_BATCH = 1000;
// Traffic budget: a daemon uploads about every 300 s and commits every 300 s, so 120/min per
// token leaves two orders of magnitude for retries.
export const RATE_PER_MIN = 120;
export const RATE_BURST = 30;

const LEVELS: readonly string[] = ["debug", "info", "warn", "error"];

const REQUIRED = {
  publicBaseUrl: "PUBLIC_BASE_URL",
  hcaClientId: "HCA_CLIENT_ID",
  hcaClientSecret: "HCA_CLIENT_SECRET",
  githubAppSlug: "GITHUB_APP_SLUG",
  githubAppClientId: "GITHUB_APP_CLIENT_ID",
  githubAppClientSecret: "GITHUB_APP_CLIENT_SECRET",
  githubWebhookSecret: "GITHUB_WEBHOOK_SECRET",
} as const;

/** With NODE_ENV=test, any unset REQUIRED var gets this dummy value (default "test"). */
const TEST_VALUES: Record<string, string> = { PUBLIC_BASE_URL: "http://localhost:8080" };

export function loadConfig(env: Record<string, string | undefined> = process.env): Config {
  const databaseUrl = env.DATABASE_URL;
  if (!databaseUrl) throw new Error("DATABASE_URL is required");
  const logLevel = env.LOG_LEVEL ?? "info";
  if (!LEVELS.includes(logLevel)) throw new Error(`invalid LOG_LEVEL: ${logLevel}`);
  const port = Number(env.PORT ?? "8080");
  if (!Number.isInteger(port) || port < 0 || port > 65535)
    throw new Error(`invalid PORT: ${env.PORT}`);
  const v = {} as Record<keyof typeof REQUIRED, string>;
  for (const [key, name] of Object.entries(REQUIRED) as [keyof typeof REQUIRED, string][]) {
    const val = env[name] || (env.NODE_ENV === "test" ? (TEST_VALUES[name] ?? "test") : "");
    if (!val) throw new Error(`${name} is required`);
    v[key] = val;
  }
  v.publicBaseUrl = v.publicBaseUrl.replace(/\/+$/, "");
  return { port, databaseUrl, logLevel: logLevel as LogLevel, ...v };
}
