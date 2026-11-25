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
  /** BUNDLE_KEY: 32 bytes, base64. Encrypts submitted turns before they reach the bucket. */
  bundleKey: string;
  bundleBucket: string;
  s3: { accessKeyId: string; secretAccessKey: string; endpoint?: string; region?: string };
};

export const MAX_BODY_BYTES = 1_048_576;
export const MAX_BATCH = 1000;
/** Submissions: compressed body and decompressed bundle caps. */
export const BUNDLE_MAX_COMPRESSED = 8 * 1024 * 1024;
export const BUNDLE_MAX_BYTES = 16 * 1024 * 1024;
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
  bundleKey: "BUNDLE_KEY",
  bundleBucket: "BUNDLE_BUCKET",
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
    bundleKey: Buffer.alloc(32, 7).toString("base64"),
    bundleBucket: "test",
    s3: { accessKeyId: "test", secretAccessKey: "test" },
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
  if (URL.parse(v.publicBaseUrl)?.origin !== v.publicBaseUrl)
    throw new Error("PUBLIC_BASE_URL must be an origin like https://tally.example");
  if (Buffer.from(v.bundleKey, "base64").length !== 32)
    throw new Error("BUNDLE_KEY must be 32 bytes, base64");
  const accessKeyId = env.S3_ACCESS_KEY_ID;
  const secretAccessKey = env.S3_SECRET_ACCESS_KEY;
  if (!accessKeyId || !secretAccessKey)
    throw new Error("S3_ACCESS_KEY_ID and S3_SECRET_ACCESS_KEY are required");
  return {
    s3: { accessKeyId, secretAccessKey, endpoint: env.S3_ENDPOINT, region: env.S3_REGION },
    port,
    databaseUrl,
    logLevel: logLevel as LogLevel,
    trustProxy: trustProxy === "1",
    ...v,
  };
}
