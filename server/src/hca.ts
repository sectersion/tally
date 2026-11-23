import { createRemoteJWKSet, type JWTVerifyGetKey, jwtVerify } from "jose";
import type { Config } from "./config.ts";

// Endpoints per docs/spikes/hack-club-auth.md.
const ISSUER = "https://auth.hackclub.com";
const AUTHORIZE = `${ISSUER}/oauth/authorize`;
const TOKEN = `${ISSUER}/oauth/token`;
const USERINFO = `${ISSUER}/oauth/userinfo`;
const SCOPE = "openid profile email verification_status";

export type HcaUser = {
  sub: string;
  email: string | null;
  name: string | null;
  verification_status: string | null;
};
type HcaCfg = Pick<Config, "publicBaseUrl" | "hcaClientId" | "hcaClientSecret">;

const redirectUri = (cfg: HcaCfg) => `${cfg.publicBaseUrl}/auth/callback`;
const remoteJwks = createRemoteJWKSet(new URL(`${ISSUER}/oauth/discovery/keys`));

export function authorizeUrl(cfg: HcaCfg, state: string, nonce: string, challenge: string): string {
  const q = new URLSearchParams({
    response_type: "code",
    client_id: cfg.hcaClientId,
    scope: SCOPE,
    redirect_uri: redirectUri(cfg),
    state,
    nonce,
    code_challenge: challenge,
    code_challenge_method: "S256",
  });
  return `${AUTHORIZE}?${q}`;
}

const str = (v: unknown) => (typeof v === "string" ? v : null);

/** Identity only: the access and refresh tokens are used here and never stored. */
export async function exchangeCode(
  cfg: HcaCfg,
  code: string,
  verifier: string,
  nonce: string,
  fetchFn: typeof fetch = fetch,
  jwks: JWTVerifyGetKey = remoteJwks,
): Promise<HcaUser> {
  const res = await fetchFn(TOKEN, {
    method: "POST",
    headers: { "content-type": "application/x-www-form-urlencoded" },
    body: new URLSearchParams({
      grant_type: "authorization_code",
      code,
      redirect_uri: redirectUri(cfg),
      client_id: cfg.hcaClientId,
      client_secret: cfg.hcaClientSecret,
      code_verifier: verifier,
    }),
  });
  if (!res.ok) throw new Error(`hca token exchange failed: ${res.status}`);
  const tok = (await res.json()) as { id_token?: unknown; access_token?: unknown };
  if (typeof tok.id_token === "string") {
    const { payload } = await jwtVerify(tok.id_token, jwks, {
      issuer: ISSUER,
      audience: cfg.hcaClientId,
    });
    // Task 1 could not confirm HCA echoes the nonce: check it whenever the token carries one.
    if (payload.nonce !== undefined && payload.nonce !== nonce) throw new Error("nonce mismatch");
    if (!payload.sub) throw new Error("id_token has no sub");
    return {
      sub: payload.sub,
      email: str(payload.email),
      name: str(payload.name),
      verification_status: str(payload.verification_status),
    };
  }
  if (typeof tok.access_token !== "string") throw new Error("hca token response has no token");
  const ui = await fetchFn(USERINFO, { headers: { authorization: `Bearer ${tok.access_token}` } });
  if (!ui.ok) throw new Error(`hca userinfo failed: ${ui.status}`);
  const p = (await ui.json()) as Record<string, unknown>;
  const sub = str(p.sub);
  if (!sub) throw new Error("userinfo has no sub");
  return {
    sub,
    email: str(p.email),
    name: str(p.name),
    verification_status: str(p.verification_status),
  };
}

let override: typeof exchangeCode | null = null;
export function setHcaExchangeForTests(f: typeof exchangeCode | null): void {
  override = f;
}
/** What the routes call: exchangeCode unless a test replaced it. */
export const exchange: typeof exchangeCode = (...a) => (override ?? exchangeCode)(...a);
