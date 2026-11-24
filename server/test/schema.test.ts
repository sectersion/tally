import { expect, test } from "bun:test";
import { MAX_BODY_BYTES } from "../src/config";
import { HttpError, json, parse, readJson } from "../src/errors";
import { CommitBody, DeviceCodeBody, UploadBody } from "../src/schema";
import fixture from "./fixtures/chain.json";

const GENESIS = "0".repeat(64);
const chain = Object.values(fixture as Record<string, Record<string, unknown>[]>)[0] ?? [];

function catchErr(f: () => unknown): HttpError {
  try {
    f();
  } catch (e) {
    if (e instanceof HttpError) return e;
    throw e;
  }
  throw new Error("did not throw");
}
async function catchAsync(f: () => Promise<unknown>): Promise<HttpError> {
  try {
    await f();
  } catch (e) {
    if (e instanceof HttpError) return e;
    throw e;
  }
  throw new Error("did not throw");
}
const bad = (body: unknown) => catchErr(() => parse(UploadBody, body));

test("a __proto__ feature key is rejected, not silently dropped", () => {
  const e = { ...chain[0], features: JSON.parse('{"__proto__":5}') };
  expect(bad({ prev_hash: GENESIS, entries: [e] }).status).toBe(400);
});

test("fixture entries parse", () =>
  expect(() => parse(UploadBody, { prev_hash: GENESIS, entries: chain })).not.toThrow());

test("unknown entry field is rejected with its path", () => {
  const err = bad({ prev_hash: GENESIS, entries: [{ ...chain[0], extra: 1 }] });
  expect(err.status).toBe(400);
  expect(err.body.error).toBe("invalid_body");
  expect(JSON.stringify(err.body.issues)).toContain("entries.0.extra");
});

test("unknown top-level field is rejected", () =>
  expect(bad({ prev_hash: GENESIS, entries: chain, foo: 1 }).status).toBe(400));

test("float seconds are rejected", () =>
  expect(bad({ prev_hash: GENESIS, entries: [{ ...chain[0], verified_s: 1.5 }] }).status).toBe(
    400,
  ));

test("uppercase hash is rejected", () => {
  const h = String(chain[0]?.hash).toUpperCase();
  expect(bad({ prev_hash: GENESIS, entries: [{ ...chain[0], hash: h }] }).status).toBe(400);
});

test("empty entries is rejected", () =>
  expect(bad({ prev_hash: GENESIS, entries: [] }).status).toBe(400));

test("issues do not echo input values", () => {
  const err = bad({ prev_hash: "LEAKME", entries: [{ ...chain[0], kind: "LEAKME".repeat(20) }] });
  expect(err.status).toBe(400);
  expect(JSON.stringify(err.body)).not.toContain("LEAKME");
});

test("too many features is rejected", () => {
  const features = Object.fromEntries(Array.from({ length: 33 }, (_, i) => [`f${i}`, 1]));
  expect(bad({ prev_hash: GENESIS, entries: [{ ...chain[0], features }] }).status).toBe(400);
});

test("CommitBody is strict", () => {
  expect(() => parse(CommitBody, { hash: GENESIS, ts: 1 })).not.toThrow();
  expect(catchErr(() => parse(CommitBody, { hash: GENESIS, ts: 1, x: 1 })).status).toBe(400);
});

test("json sets content-type", () =>
  expect(json(201, { a: 1 }).headers.get("content-type")).toContain("application/json"));

test("readJson rejects a body over the limit", async () => {
  const req = new Request("http://x/", { method: "POST", body: "x".repeat(MAX_BODY_BYTES + 1) });
  expect((await catchAsync(() => readJson(req))).body.error).toBe("body_too_large");
});

test("readJson stops a chunked body over the limit", async () => {
  const chunk = new Uint8Array(MAX_BODY_BYTES / 2 + 1);
  const body = new ReadableStream({
    start(c) {
      c.enqueue(chunk);
      c.enqueue(chunk);
      c.close();
    },
  });
  const req = new Request("http://x/", { method: "POST", body, duplex: "half" } as RequestInit);
  expect((await catchAsync(() => readJson(req))).body.error).toBe("body_too_large");
});

test("readJson rejects bad json and parses good json", async () => {
  const post = (b: string) => new Request("http://x/", { method: "POST", body: b });
  const err = await catchAsync(() => readJson(post("{nope")));
  expect(err.status).toBe(400);
  expect(err.body.error).toBe("invalid_json");
  expect(await readJson(post('{"a":1}'))).toEqual({ a: 1 });
});

test("device names reject control and format characters", () => {
  expect(DeviceCodeBody.safeParse({ device_name: "Ada's laptop" }).success).toBe(true);
  for (const bad of ["evil\u202egnp", "zero\u200bwidth", "ctl\u0007x", "new\nline"])
    expect(DeviceCodeBody.safeParse({ device_name: bad }).success).toBe(false);
});
