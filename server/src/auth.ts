import type { SQL } from "bun";
import { HttpError } from "./errors.ts";

export function newToken(): string {
  return `tly_${Buffer.from(crypto.getRandomValues(new Uint8Array(32))).toString("base64url")}`;
}

export function hashToken(token: string): string {
  return new Bun.CryptoHasher("sha256").update(token).digest("hex");
}

export async function createProjectToken(
  sql: SQL,
  publicName: string,
  label = "",
): Promise<{ projectId: string; tokenId: number; token: string }> {
  const token = newToken();
  return sql.begin(async (tx) => {
    const [p] =
      await tx`insert into projects (id, public_name) values (${crypto.randomUUID()}, ${publicName})
      on conflict (public_name) do update set public_name = excluded.public_name returning id`;
    const [t] = await tx`insert into device_tokens (project_id, token_hash, label)
      values (${p.id}, ${hashToken(token)}, ${label}) returning id`;
    return { projectId: p.id as string, tokenId: Number(t.id), token };
  });
}

export async function revokeToken(sql: SQL, tokenId: number): Promise<boolean> {
  const rows =
    await sql`update device_tokens set revoked_at = now() where id = ${tokenId} and revoked_at is null returning id`;
  return rows.length > 0;
}

export async function requireToken(
  sql: SQL,
  req: Request,
  projectId: string,
): Promise<{ tokenId: number }> {
  const m = /^Bearer (\S+)$/.exec(req.headers.get("authorization") ?? "");
  const unauthorized = new HttpError(401, { error: "unauthorized" });
  if (!m) throw unauthorized;
  const [row] = await sql`select t.id, t.project_id, t.user_id, t.revoked_at, p.owner_id
      from device_tokens t left join projects p on p.id = ${projectId}
      where t.token_hash = ${hashToken(m[1] as string)}`;
  if (!row || row.revoked_at) throw unauthorized;
  // A project token is bound to its project; a user token (no project) to the projects its user owns.
  const ok =
    row.project_id === null
      ? row.user_id !== null && row.user_id === row.owner_id
      : row.project_id === projectId;
  if (!ok) throw new HttpError(403, { error: "forbidden" });
  return { tokenId: Number(row.id) };
}
