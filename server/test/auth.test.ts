import { afterAll, expect, test } from "bun:test";
import { createProjectToken, hashToken, newToken, requireToken, revokeToken } from "../src/auth.ts";
import { connect } from "../src/db.ts";
import { HttpError } from "../src/errors.ts";

const sql = connect(process.env.DATABASE_URL ?? "");
afterAll(() => sql.close());
const name = () => `t/${crypto.randomUUID()}`;
const req = (t?: string, scheme = "Bearer") =>
  new Request("http://x/", { headers: t ? { authorization: `${scheme} ${t}` } : {} });
const status = async (p: Promise<unknown>) => {
  try {
    await p;
  } catch (e) {
    if (e instanceof HttpError) return e.status;
    throw e;
  }
  return 200;
};
const cli = (...args: string[]) =>
  Bun.spawnSync(["bun", "scripts/token.ts", ...args], { env: process.env });

test("newToken format", () => {
  expect(newToken()).toMatch(/^tly_[A-Za-z0-9_-]{43}$/);
});

test("token is stored only as a hash", async () => {
  const { tokenId, token } = await createProjectToken(sql, name());
  const [row] = await sql`select token_hash from device_tokens where id = ${tokenId}`;
  expect(row.token_hash).toBe(hashToken(token));
  expect(row.token_hash).not.toContain(token);
});

test("same public name reuses the project", async () => {
  const n = name();
  const a = await createProjectToken(sql, n);
  const b = await createProjectToken(sql, n, "laptop");
  expect(b.projectId).toBe(a.projectId);
  expect(b.tokenId).not.toBe(a.tokenId);
});

test("requireToken: missing header 401, wrong project 403, revoked 401, valid ok", async () => {
  const a = await createProjectToken(sql, name());
  const b = await createProjectToken(sql, name());
  expect(await status(requireToken(sql, req(), a.projectId))).toBe(401);
  expect(await status(requireToken(sql, req(a.token, "Basic"), a.projectId))).toBe(401);
  expect(await status(requireToken(sql, req("tly_nope"), a.projectId))).toBe(401);
  expect(await status(requireToken(sql, req(a.token), b.projectId))).toBe(403);
  expect(await requireToken(sql, req(a.token), a.projectId)).toEqual({ tokenId: a.tokenId });
  expect(await revokeToken(sql, a.tokenId)).toBe(true);
  expect(await revokeToken(sql, a.tokenId)).toBe(false);
  expect(await status(requireToken(sql, req(a.token), a.projectId))).toBe(401);
});

test("CLI create prints project_id, token_id, token", () => {
  const p = cli("create", name());
  expect(p.exitCode).toBe(0);
  expect(p.stdout.toString()).toMatch(
    /^project_id: .+\ntoken_id: \d+\ntoken: tly_[A-Za-z0-9_-]{43}\n$/,
  );
});

test("CLI revoke and usage", () => {
  const id = /token_id: (\d+)/.exec(cli("create", name()).stdout.toString())?.[1] ?? "";
  expect(cli("revoke", id).stdout.toString()).toBe("revoked\n");
  expect(cli("revoke", id).exitCode).toBe(1);
  expect(cli().exitCode).toBe(2);
});
