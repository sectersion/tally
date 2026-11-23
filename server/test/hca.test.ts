import { expect, test } from "bun:test";
import { createLocalJWKSet, exportJWK, generateKeyPair, SignJWT } from "jose";
import { loadConfig } from "../src/config.ts";
import { authorizeUrl, exchangeCode } from "../src/hca.ts";

const cfg = loadConfig({ DATABASE_URL: "x", NODE_ENV: "test", HCA_CLIENT_ID: "cid" });
const { publicKey, privateKey } = await generateKeyPair("RS256");
const jwks = createLocalJWKSet({ keys: [{ ...(await exportJWK(publicKey)), alg: "RS256" }] });

async function tok(over: Record<string, unknown>): Promise<string> {
  const claims = {
    iss: "https://auth.hackclub.com",
    aud: "cid",
    sub: "ident!abc",
    exp: Math.floor(Date.now() / 1000) + 600,
    nonce: "n1",
    email: "a@b.c",
    name: "Ann",
    verification_status: "verified",
    ...over,
  };
  return new SignJWT(claims).setProtectedHeader({ alg: "RS256" }).sign(privateKey);
}

async function exchange(idToken: string | null, nonce = "n1") {
  const calls: { url: string; init?: RequestInit }[] = [];
  const fetchFn = (async (url: string, init?: RequestInit) => {
    calls.push({ url, init });
    if (url.endsWith("/oauth/userinfo"))
      return Response.json({ sub: "ident!ui", email: "u@x.y", name: "Ui" });
    return Response.json(
      idToken ? { id_token: idToken, access_token: "at" } : { access_token: "at" },
    );
  }) as unknown as typeof fetch;
  const user = await exchangeCode(cfg, "code1", "ver1", nonce, fetchFn, jwks);
  return { user, calls };
}

test("valid id token gives the user", async () => {
  const { user, calls } = await exchange(await tok({}));
  expect(user).toEqual({
    sub: "ident!abc",
    email: "a@b.c",
    name: "Ann",
    verification_status: "verified",
  });
  const body = String(calls[0]?.init?.body);
  expect(body).toContain("code_verifier=ver1");
  expect(body).toContain("client_secret=");
});

test("wrong audience is rejected", async () => {
  await expect(exchange(await tok({ aud: "other" }))).rejects.toThrow();
});
test("wrong issuer is rejected", async () => {
  await expect(exchange(await tok({ iss: "https://evil" }))).rejects.toThrow();
});
test("expired token is rejected", async () => {
  await expect(exchange(await tok({ exp: 1 }))).rejects.toThrow();
});
test("wrong nonce is rejected", async () => {
  await expect(exchange(await tok({ nonce: "x" }))).rejects.toThrow();
});
test("a token without a nonce claim is accepted (Task 1: not verified to be echoed)", async () => {
  expect((await exchange(await tok({ nonce: undefined }))).user.sub).toBe("ident!abc");
});

test("no id_token falls back to userinfo", async () => {
  const { user, calls } = await exchange(null);
  expect(user.sub).toBe("ident!ui");
  expect(calls[1]?.url).toBe("https://auth.hackclub.com/oauth/userinfo");
});

test("authorize url", () => {
  const u = new URL(authorizeUrl(cfg, "s", "n", "c"));
  expect(u.origin + u.pathname).toBe("https://auth.hackclub.com/oauth/authorize");
  expect(Object.fromEntries(u.searchParams)).toEqual({
    response_type: "code",
    client_id: "cid",
    scope: "openid profile email verification_status",
    redirect_uri: "http://localhost:8080/auth/callback",
    state: "s",
    nonce: "n",
    code_challenge: "c",
    code_challenge_method: "S256",
  });
});
