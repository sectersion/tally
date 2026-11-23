import { timingSafeEqual } from "node:crypto";
import type { SQL } from "bun";
import { hashToken, newToken } from "./auth.ts";
import type { Config } from "./config.ts";
import { HttpError } from "./errors.ts";

export const COOKIE = "tally_session";
const MAX_AGE = 2_592_000; // 30 days

type Session = { userId: string; csrf: string; role: string };

const attrs = (cfg: Pick<Config, "publicBaseUrl">, maxAge: number) =>
  `HttpOnly; ${cfg.publicBaseUrl.startsWith("http://localhost") ? "" : "Secure; "}SameSite=Lax; Path=/; Max-Age=${maxAge}`;

export const clearCookie = (cfg: Pick<Config, "publicBaseUrl">) => `${COOKIE}=; ${attrs(cfg, 0)}`;

export function sessionToken(req: Request): string | null {
  return (
    new RegExp(`(?:^|;\\s*)${COOKIE}=([^;]+)`).exec(req.headers.get("cookie") ?? "")?.[1] ?? null
  );
}

export async function createSession(
  sql: SQL,
  cfg: Pick<Config, "publicBaseUrl">,
  userId: string,
): Promise<{ setCookie: string; csrf: string }> {
  const token = newToken();
  const csrf = newToken();
  await sql`insert into sessions (token_hash, user_id, csrf, expires_at)
    values (${hashToken(token)}, ${userId}, ${csrf}, now() + ${`${MAX_AGE} seconds`}::interval)`;
  return { setCookie: `${COOKIE}=${token}; ${attrs(cfg, MAX_AGE)}`, csrf };
}

export async function getSession(sql: SQL, req: Request): Promise<Session | null> {
  const token = sessionToken(req);
  if (!token) return null;
  const [row] =
    await sql`select s.user_id, s.csrf, u.role from sessions s join users u on u.id = s.user_id
    where s.token_hash = ${hashToken(token)} and s.expires_at > now()`;
  return row ? { userId: row.user_id, csrf: row.csrf, role: row.role } : null;
}

export async function requireUser(sql: SQL, req: Request): Promise<Session> {
  const s = await getSession(sql, req);
  if (s) return s;
  if (req.method === "GET") {
    const u = new URL(req.url);
    throw new HttpError(
      303,
      { error: "login_required" },
      `/auth/login?return_to=${encodeURIComponent(u.pathname + u.search)}`,
    );
  }
  throw new HttpError(401, { error: "unauthorized" });
}

const same = (a: string, b: string) =>
  a.length === b.length && timingSafeEqual(Buffer.from(a), Buffer.from(b));

/** State-changing browser routes: Origin must be our own and the form must carry the session's csrf token. */
export function requireCsrf(
  cfg: Pick<Config, "publicBaseUrl">,
  req: Request,
  s: { csrf: string },
  form: FormData,
): void {
  const field = form.get("csrf");
  if (
    req.headers.get("origin") !== cfg.publicBaseUrl ||
    typeof field !== "string" ||
    !same(field, s.csrf)
  )
    throw new HttpError(403, { error: "csrf" });
}

export function requireRole(user: { role: string }, role: "reviewer" | "admin"): void {
  if (user.role !== role && user.role !== "admin") throw new HttpError(404, { error: "not_found" });
}
