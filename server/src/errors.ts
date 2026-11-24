import type { ZodType } from "zod";
import { MAX_BODY_BYTES } from "./config";

export class HttpError extends Error {
  constructor(
    public status: number,
    public body: { error: string; [k: string]: unknown },
    /** When set, the response is a 303-style redirect to this location. */
    public location?: string,
  ) {
    super(body.error);
  }
}

export function json(status: number, body: unknown, headers?: HeadersInit): Response {
  return Response.json(body, { status, headers });
}

const tooLarge = () => new HttpError(413, { error: "body_too_large", max: MAX_BODY_BYTES });

/** Reads the body, giving up as soon as it passes MAX_BODY_BYTES (also for chunked bodies). */
export async function readCapped(req: Request): Promise<Uint8Array> {
  if (Number(req.headers.get("content-length")) > MAX_BODY_BYTES) throw tooLarge();
  const chunks: Uint8Array[] = [];
  let n = 0;
  if (req.body)
    for await (const c of req.body as unknown as AsyncIterable<Uint8Array>) {
      n += c.byteLength;
      if (n > MAX_BODY_BYTES) throw tooLarge();
      chunks.push(c);
    }
  return Buffer.concat(chunks);
}

export async function readJson(req: Request): Promise<unknown> {
  const text = new TextDecoder().decode(await readCapped(req));
  try {
    return JSON.parse(text);
  } catch {
    throw new HttpError(400, { error: "invalid_json" });
  }
}

const cap = (s: string) => s.slice(0, 64);

/** Issues carry only a path and a fixed message: never zod's own text, which can quote input. */
export function parse<T>(schema: ZodType<T>, value: unknown): T {
  const r = schema.safeParse(value);
  if (r.success) return r.data;
  const issues = r.error.issues.flatMap((i) => {
    if (i.code === "unrecognized_keys")
      return i.keys.map((k) => ({
        path: cap([...i.path, k].join(".")),
        message: "unrecognized key",
      }));
    return [{ path: cap(i.path.map(String).join(".")), message: i.code }];
  });
  throw new HttpError(400, { error: "invalid_body", issues: issues.slice(0, 20) });
}
