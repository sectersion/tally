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
  /** TRUST_PROXY=1: take the client ip from the last x-forwarded-for hop. */
  trustProxy: boolean;
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

/** Dummy config for tests only; loadConfig never fills in missing values. */
export function testConfig(over: Partial<Config> = {}): Config {
  return {
    port: 0,
    databaseUrl: "-",
    logLevel: "info",
    publicBaseUrl: "http://localhost:8080",
    hcaClientId: "test",
    hcaClientSecret: "test",
    githubAppSlug: "test",
    githubAppClientId: "test",
    githubAppClientSecret: "test",
    githubWebhookSecret: "test",
    trustProxy: false,
    ...over,
  };
}

export function loadConfig(env: Record<string, string | undefined> = process.env): Config {
  const databaseUrl = env.DATABASE_URL;
  if (!databaseUrl) throw new Error("DATABASE_URL is required");
  const logLevel = env.LOG_LEVEL ?? "info";
  if (!LEVELS.includes(logLevel)) throw new Error(`invalid LOG_LEVEL: ${logLevel}`);
  const port = Number(env.PORT ?? "8080");
  if (!Number.isInteger(port) || port < 0 || port > 65535)
    throw new Error(`invalid PORT: ${env.PORT}`);
  const trustProxy = env.TRUST_PROXY ?? "0";
  if (trustProxy !== "0" && trustProxy !== "1")
    throw new Error(`invalid TRUST_PROXY: ${trustProxy}`);
  const v = {} as Record<keyof typeof REQUIRED, string>;
  for (const [key, name] of Object.entries(REQUIRED) as [keyof typeof REQUIRED, string][]) {
    const val = env[name];
    if (!val) throw new Error(`${name} is required`);
    v[key] = val;
  }
  v.publicBaseUrl = v.publicBaseUrl.replace(/\/+$/, "");
  return {
    port,
    databaseUrl,
    logLevel: logLevel as LogLevel,
    trustProxy: trustProxy === "1",
    ...v,
  };
}
