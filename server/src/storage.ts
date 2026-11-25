import { S3Client, type SQL } from "bun";
import type { Config } from "./config.ts";

export type Storage = {
  put(key: string, bytes: Uint8Array): Promise<void>;
  get(key: string): Promise<Uint8Array | null>;
  /** Idempotent: deleting a missing object is not an error. */
  del(key: string): Promise<void>;
};

/** Bundles live in Postgres (bytea, ≤ BUNDLE_MAX_BYTES). ponytail: swap to S3 when bundles outgrow one table. */
export function pgStorage(sql: SQL): Storage {
  return {
    put: async (k, b) => void (await sql`insert into bundles (key, bytes) values (${k}, ${b}) on conflict (key) do update set bytes = excluded.bytes`),
    get: async (k) => {
      const [r] = await sql`select bytes from bundles where key = ${k}`;
      return r ? (r.bytes as Uint8Array) : null;
    },
    del: async (k) => void (await sql`delete from bundles where key = ${k}`),
  };
}

export function memoryStorage(): Storage & { objects: Map<string, Uint8Array> } {
  const objects = new Map<string, Uint8Array>();
  return {
    objects,
    put: async (k, b) => void objects.set(k, b.slice()),
    get: async (k) => objects.get(k)?.slice() ?? null,
    del: async (k) => void objects.delete(k),
  };
}

export function s3Storage(
  cfg: Pick<Config, "bundleBucket" | "s3">,
): Storage & { client: S3Client } {
  const client = new S3Client({
    bucket: cfg.bundleBucket,
    accessKeyId: cfg.s3.accessKeyId,
    secretAccessKey: cfg.s3.secretAccessKey,
    endpoint: cfg.s3.endpoint,
    region: cfg.s3.region,
  });
  return {
    client,
    put: async (k, b) => void (await client.write(k, b)),
    get: async (k) => {
      const f = client.file(k);
      return (await f.exists()) ? new Uint8Array(await f.arrayBuffer()) : null;
    },
    del: async (k) => client.delete(k),
  };
}

/** Refuses a bucket that does not report server-side encryption on a fresh object (HEAD response header). */
export async function requireSse(s: ReturnType<typeof s3Storage>): Promise<void> {
  const key = `probe/${crypto.randomUUID()}`;
  await s.put(key, new Uint8Array([0]));
  try {
    const r = await fetch(s.client.presign(key, { method: "HEAD", expiresIn: 60 }), {
      method: "HEAD",
    });
    if (!r.headers.get("x-amz-server-side-encryption"))
      throw new Error("bucket does not apply server-side encryption: enable default SSE");
  } finally {
    await s.del(key);
  }
}

const IV_BYTES = 12;

/** AES-256-GCM on top of any Storage; object = iv ‖ ciphertext, fresh random 96-bit iv per object. */
export function encrypted(raw: Storage, keyB64: string): Storage {
  const key = crypto.subtle.importKey("raw", Buffer.from(keyB64, "base64"), "AES-GCM", false, [
    "encrypt",
    "decrypt",
  ]);
  return {
    async put(k, bytes) {
      const iv = crypto.getRandomValues(new Uint8Array(IV_BYTES));
      const ct = new Uint8Array(
        await crypto.subtle.encrypt(
          { name: "AES-GCM", iv },
          await key,
          bytes as Uint8Array<ArrayBuffer>,
        ),
      );
      const out = new Uint8Array(IV_BYTES + ct.length);
      out.set(iv);
      out.set(ct, IV_BYTES);
      await raw.put(k, out);
    },
    async get(k) {
      const o = await raw.get(k);
      if (!o) return null;
      const iv = o.subarray(0, IV_BYTES) as Uint8Array<ArrayBuffer>;
      return new Uint8Array(
        await crypto.subtle.decrypt(
          { name: "AES-GCM", iv },
          await key,
          o.subarray(IV_BYTES) as Uint8Array<ArrayBuffer>,
        ),
      );
    },
    del: (k) => raw.del(k),
  };
}
