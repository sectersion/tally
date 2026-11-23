import type { ZodType } from "zod";
import { MAX_BODY_BYTES } from "./config";

export class HttpError extends Error {
  constructor(
    public status: number,
    public body: { error: string; [k: string]: unknown },
  ) {
    super(body.error);
  }
}

export function json(status: number, body: unknown, headers?: HeadersInit): Response {
  return Response.json(body, { status, headers });
}

const tooLarge = () => new HttpError(413, { error: "body_too_large", max: MAX_BODY_BYTES });

export async function readJson(req: Request): Promise<unknown> {
  const len = Number(req.headers.get("content-length"));
  if (len > MAX_BODY_BYTES) throw tooLarge();
  const text = await req.text();
  if (Buffer.byteLength(text) > MAX_BODY_BYTES) throw tooLarge();
  try {
    return JSON.parse(text);
  } catch {
    throw new HttpError(400, { error: "invalid_json" });
  }
}

/** Issues carry only a path and a fixed message: never zod's own text, which can quote input. */
export function parse<T>(schema: ZodType<T>, value: unknown): T {
  const r = schema.safeParse(value);
  if (r.success) return r.data;
  const issues = r.error.issues.flatMap((i) => {
    if (i.code === "unrecognized_keys")
      return i.keys.map((k) => ({ path: [...i.path, k].join("."), message: "unrecognized key" }));
    return [{ path: i.path.map(String).join("."), message: i.code }];
  });
  throw new HttpError(400, { error: "invalid_body", issues });
}
