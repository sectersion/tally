import type { SQL } from "bun";
import { z } from "zod";
import type { Config } from "./config.ts";
import { HttpError } from "./errors.ts";

// Endpoints and behaviour per docs/spikes/github-app.md. User-to-server tokens only: no App key.
const API = "https://api.github.com";
const AUTHORIZE = "https://github.com/login/oauth/authorize";
const TOKEN = "https://github.com/login/oauth/access_token";
const MAX_PAGES = 10;
const MAX_TOKEN_S = 28_800; // GitHub's 8 h default; also caps a non-expiring token

type GhCfg = Pick<Config, "publicBaseUrl" | "githubAppClientId" | "githubAppClientSecret">;
export type AccessibleRepo = {
  id: number;
  full_name: string;
  installation_id: number;
  account_login: string;
};

/** One GitHub API call (all of them go through here). Returns each page's JSON, following Link rel="next". */
export async function gh(
  token: string,
  path: string,
  fetchFn: typeof fetch = fetch,
): Promise<unknown[]> {
  const pages: unknown[] = [];
  let next: string | null = path.startsWith("http")
    ? path
    : `${API}${path}${path.includes("?") ? "&" : "?"}per_page=100`;
  for (let i = 0; next && i < MAX_PAGES; i++) {
    const res: Response = await fetchFn(next, {
      headers: {
        accept: "application/vnd.github+json",
        "x-github-api-version": "2022-11-28",
        "user-agent": "tally-server",
        authorization: `Bearer ${token}`,
      },
    });
    if (res.status === 401) throw new HttpError(401, { error: "github_token_invalid" });
    if (!res.ok) throw new HttpError(502, { error: "github_failed" });
    pages.push(await res.json());
    const n = /<([^>]+)>;\s*rel="next"/.exec(res.headers.get("link") ?? "")?.[1] ?? null;
    next = n?.startsWith(`${API}/`) ? n : null; // never send the token to another origin
  }
  return pages;
}

export const authorizeUrl = (cfg: GhCfg, state: string, challenge: string): string =>
  `${AUTHORIZE}?${new URLSearchParams({
    client_id: cfg.githubAppClientId,
    redirect_uri: `${cfg.publicBaseUrl}/github/callback`,
    state,
    code_challenge: challenge,
    code_challenge_method: "S256",
  })}`;

const TokenResp = z.object({
  access_token: z.string().min(1),
  expires_in: z.number().positive().optional(),
});
const Me = z.object({ id: z.number().int() });

/** Exchanges the code and returns the access token and GitHub user id. The refresh token is never read. */
export async function exchangeCode(
  cfg: GhCfg,
  code: string,
  verifier: string,
  fetchFn: typeof fetch = fetch,
): Promise<{ token: string; expiresIn: number; githubUserId: number }> {
  const res = await fetchFn(TOKEN, {
    method: "POST",
    headers: { accept: "application/json", "user-agent": "tally-server" },
    body: new URLSearchParams({
      client_id: cfg.githubAppClientId,
      client_secret: cfg.githubAppClientSecret,
      code,
      redirect_uri: `${cfg.publicBaseUrl}/github/callback`,
      code_verifier: verifier,
    }),
  });
  const t = TokenResp.safeParse(res.ok ? await res.json() : null); // errors come back as 200 + {error}
  if (!t.success) throw new HttpError(502, { error: "github_failed" });
  const [me] = await gh(t.data.access_token, "/user", fetchFn);
  const u = Me.safeParse(me);
  if (!u.success) throw new HttpError(502, { error: "github_failed" });
  return {
    token: t.data.access_token,
    expiresIn: Math.min(t.data.expires_in ?? MAX_TOKEN_S, MAX_TOKEN_S),
    githubUserId: u.data.id,
  };
}

const Installs = z.object({
  installations: z.array(
    z.object({ id: z.number().int(), account: z.object({ login: z.string() }) }),
  ),
});
const Repos = z.object({
  repositories: z.array(z.object({ id: z.number().int(), full_name: z.string() })),
});

function parsed<T>(schema: z.ZodType<T>, v: unknown): T {
  const r = schema.safeParse(v);
  if (!r.success) throw new HttpError(502, { error: "github_failed" });
  return r.data;
}

/** Every repo the token's user can reach through an installation of the App, fetched live. */
export async function accessibleRepos(
  token: string,
  fetchFn: typeof fetch = fetch,
): Promise<AccessibleRepo[]> {
  const out: AccessibleRepo[] = [];
  for (const page of await gh(token, "/user/installations", fetchFn)) {
    for (const i of parsed(Installs, page).installations) {
      for (const rp of await gh(token, `/user/installations/${i.id}/repositories`, fetchFn))
        for (const r of parsed(Repos, rp).repositories)
          out.push({
            id: r.id,
            full_name: r.full_name,
            installation_id: i.id,
            account_login: i.account.login,
          });
    }
  }
  return out;
}

/** The user's unexpired GitHub token, or null (expired means "connect GitHub again"). */
export async function userToken(sql: SQL, userId: string): Promise<string | null> {
  const [t] =
    await sql`select access_token from github_user_tokens where user_id = ${userId} and expires_at > now()`;
  return t?.access_token ?? null;
}

/** The live accessible list for a user, or [] when they have no valid token. */
export async function userRepos(
  sql: SQL,
  userId: string,
  fetchFn: typeof fetch = fetch,
): Promise<AccessibleRepo[]> {
  const token = await userToken(sql, userId);
  return token ? accessibleRepos(token, fetchFn) : [];
}

/** Inside the caller's transaction: record what GitHub said and link the repo. A 23505 here means "already linked". */
export async function linkRepoTx(tx: SQL, projectId: string, r: AccessibleRepo): Promise<void> {
  await tx`insert into github_installations (id, account_login) values (${r.installation_id}, ${r.account_login})
    on conflict (id) do update set account_login = excluded.account_login, revoked_at = null`;
  await tx`insert into github_repos (id, full_name, installation_id) values (${r.id}, ${r.full_name}, ${r.installation_id})
    on conflict (id) do update set full_name = excluded.full_name, installation_id = excluded.installation_id`;
  await tx`insert into project_repos (project_id, github_repo_id) values (${projectId}, ${r.id})`;
}
