import type { SQL } from "bun";
import { hashToken, newToken } from "./auth.ts";

const ALPHABET = "BCDFGHJKLMNPQRSTVWXZ";

/** 8 letters as XXXX-XXXX; rejection sampling (bytes < 240) keeps the 20-letter alphabet unbiased. */
export function newUserCode(): string {
  let s = "";
  while (s.length < 8) {
    for (const b of crypto.getRandomValues(new Uint8Array(16))) {
      if (b < 240 && s.length < 8) s += ALPHABET[b % 20];
    }
  }
  return `${s.slice(0, 4)}-${s.slice(4)}`;
}

export async function createDeviceCode(sql: SQL, deviceName: string) {
  for (;;) {
    const device_code = newToken();
    const user_code = newUserCode();
    try {
      await sql`insert into device_codes (code_hash, user_code, device_name, expires_at)
        values (${hashToken(device_code)}, ${user_code}, ${deviceName}, now() + interval '10 minutes')`;
      return { device_code, user_code };
    } catch (e) {
      const x = e as { errno?: unknown; code?: unknown };
      if (x.errno !== "23505" && x.code !== "23505") throw e; // user_code collision: draw again
    }
  }
}

export type PollResult =
  | { status: 200; body: { access_token: string; token_type: "Bearer" } }
  | { status: 400; body: { error: string } };

export function pollDeviceToken(sql: SQL, deviceCode: string): Promise<PollResult> {
  return sql.begin(async (tx): Promise<PollResult> => {
    const h = hashToken(deviceCode);
    const [r] = await tx`select device_name, user_id, denied,
        (last_poll_at is not null and last_poll_at > now() - interval '5 seconds') as too_fast
      from device_codes where code_hash = ${h} and expires_at > now() and not consumed for update`;
    if (!r) return { status: 400, body: { error: "expired_token" } };
    if (r.denied) {
      await tx`update device_codes set consumed = true where code_hash = ${h}`;
      return { status: 400, body: { error: "access_denied" } };
    }
    await tx`update device_codes set last_poll_at = now() where code_hash = ${h}`;
    if (r.too_fast) return { status: 400, body: { error: "slow_down" } };
    if (!r.user_id) return { status: 400, body: { error: "authorization_pending" } };
    const token = newToken();
    await tx`insert into device_tokens (user_id, label, token_hash)
      values (${r.user_id}, ${r.device_name}, ${hashToken(token)})`;
    await tx`update device_codes set consumed = true where code_hash = ${h}`;
    return { status: 200, body: { access_token: token, token_type: "Bearer" } };
  });
}

/** "bcdf ghjk", "bcdfghjk" and "BCDF-GHJK" are the same code. */
export const normUserCode = (c: string) => {
  const s = c.toUpperCase().replace(/[^A-Z]/g, "");
  return s.length === 8 ? `${s.slice(0, 4)}-${s.slice(4)}` : s;
};

/** The pending device name for a code that can still be decided, else null. */
export async function pendingDevice(sql: SQL, userCode: string): Promise<string | null> {
  const [r] = await sql`select device_name from device_codes
    where user_code = ${normUserCode(userCode)} and expires_at > now() and not consumed
      and not denied and user_id is null`;
  return r ? (r.device_name as string) : null;
}

/** One atomic update: a code is decided once, while unexpired. */
export async function decide(
  sql: SQL,
  userCode: string,
  userId: string,
  approve: boolean,
): Promise<boolean> {
  const rows = approve
    ? await sql`update device_codes set user_id = ${userId}
        where user_code = ${normUserCode(userCode)} and expires_at > now() and not consumed
          and not denied and user_id is null returning 1 as x`
    : await sql`update device_codes set denied = true
        where user_code = ${normUserCode(userCode)} and expires_at > now() and not consumed
          and not denied and user_id is null returning 1 as x`;
  return rows.length > 0;
}
