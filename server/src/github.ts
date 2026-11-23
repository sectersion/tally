import { createHmac, timingSafeEqual } from "node:crypto";
import type { SQL } from "bun";
import { z } from "zod";
import type { Config } from "./config.ts";
import { HttpError, parse } from "./errors.ts";
import { bumpChange } from "./projects.ts";

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
    try {
      pages.push(await res.json());
    } catch {
      throw new HttpError(502, { error: "github_failed" }); // a 2xx that is not JSON
    }
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

// ---- Webhooks ----

/** X-Hub-Signature-256 over the raw bytes. The SHA-1 X-Hub-Signature is never read. */
export function verifySignature(secret: string, raw: Uint8Array, header: string | null): boolean {
  if (!header || !/^sha256=[0-9a-f]{64}$/.test(header)) return false;
  const want = createHmac("sha256", secret).update(raw).digest();
  return timingSafeEqual(want, Buffer.from(header.slice(7), "hex"));
}

const Id = z.number().int();
const Push = z.object({
  ref: z.string(),
  after: z.string().min(1),
  forced: z.boolean(),
  deleted: z.boolean(),
  repository: z.object({ id: Id }),
  // message, author, committer and pusher are never read, so never stored.
  commits: z.array(z.object({ id: z.string().min(1), timestamp: z.string().optional() })),
  head_commit: z.object({ id: z.string(), timestamp: z.string().optional() }).nullish(),
});
const InstallationEv = z.object({ installation: z.object({ id: Id }) });
const Removed = z.object({
  installation: z.object({ id: Id }),
  repositories_removed: z.array(z.object({ id: Id })),
});
const RepositoryEv = z.object({
  repository: z.object({ id: Id, full_name: z.string(), owner: z.object({ login: z.string() }) }),
  installation: z.object({ id: Id }).optional(),
});
const SenderEv = z.object({ sender: z.object({ id: Id }) });

const claimed = (ts: string | undefined): number | null => {
  const n = ts === undefined ? Number.NaN : Date.parse(ts);
  return Number.isNaN(n) ? null : n;
};

/** Unlinks the live links of these repos with one repo_events row each; returns the affected projects. */
async function unlink(tx: SQL, repoIds: number[], kind: string, detail: string) {
  if (!repoIds.length) return [];
  const rows = await tx`update project_repos set unlinked_at = now()
    where github_repo_id = any(${repoIds}::bigint[]) and unlinked_at is null
    returning project_id, github_repo_id`;
  for (const r of rows)
    await tx`insert into repo_events (github_repo_id, kind, detail) values (${r.github_repo_id}, ${kind}, ${detail})`;
  for (const pid of new Set<string>(rows.map((r: { project_id: string }) => r.project_id))) {
    await tx`update projects set upd_record_id = null where id = ${pid}`; // matched the old repo
    await bumpChange(tx, pid);
  }
  return rows;
}

/** Runs inside the delivery's transaction. False means the event is not one we act on. */
export async function dispatch(
  tx: SQL,
  deliveryId: string,
  event: string,
  payload: unknown,
): Promise<boolean> {
  const action = z.object({ action: z.string() }).safeParse(payload).data?.action;
  if (event === "push") {
    const p = parse(Push, payload);
    if (p.deleted || /^0+$/.test(p.after)) return true; // branch deletion
    const [link] = await tx`select project_id from project_repos
      where github_repo_id = ${p.repository.id} and unlinked_at is null`;
    const projectId: string | null = link?.project_id ?? null;
    const shas = new Map<string, number | null>();
    for (const c of p.commits) shas.set(c.id, claimed(c.timestamp));
    if (!shas.has(p.after))
      shas.set(p.after, p.head_commit?.id === p.after ? claimed(p.head_commit.timestamp) : null);
    for (const [sha, ts] of shas)
      await tx`insert into push_receipts (delivery_id, sha, github_repo_id, project_id, ref, is_head, forced, claimed_ts)
        values (${deliveryId}, ${sha}, ${p.repository.id}, ${projectId}, ${p.ref}, ${sha === p.after}, ${p.forced}, ${ts})`;
    if (projectId) await bumpChange(tx, projectId);
    return true;
  }
  if (event === "installation" && (action === "deleted" || action === "suspend")) {
    const id = parse(InstallationEv, payload).installation.id;
    await tx`update github_installations set revoked_at = now() where id = ${id}`;
    const repos = await tx`select id from github_repos where installation_id = ${id}`;
    await unlink(
      tx,
      repos.map((r: { id: string | number }) => Number(r.id)),
      "uninstalled",
      action,
    );
    return true;
  }
  if (event === "installation" && action === "unsuspend") {
    // Clears the flag only; links stay cut and the user relinks.
    await tx`update github_installations set revoked_at = null where id = ${parse(InstallationEv, payload).installation.id}`;
    return true;
  }
  if (event === "installation_repositories" && action === "removed") {
    const p = parse(Removed, payload);
    // Only repos still on this installation: a stale removal after a transfer must not cut the new link.
    const mine = await tx`select id from github_repos where installation_id = ${p.installation.id}
      and id = any(${p.repositories_removed.map((r) => r.id)}::bigint[])`;
    await unlink(
      tx,
      mine.map((r: { id: string | number }) => Number(r.id)),
      "removed",
      `installation ${p.installation.id}`,
    );
    return true;
  }
  if (event === "repository" && (action === "renamed" || action === "transferred")) {
    const p = parse(RepositoryEv, payload);
    const [old] = await tx`select full_name from github_repos where id = ${p.repository.id}`;
    if (!old) return true;
    let installationId: number | null = null;
    if (action === "transferred" && p.installation) {
      installationId = p.installation.id;
      await tx`insert into github_installations (id, account_login) values (${installationId}, ${p.repository.owner.login})
        on conflict (id) do nothing`;
    }
    await tx`update github_repos set full_name = ${p.repository.full_name},
      installation_id = coalesce(${installationId}::bigint, installation_id) where id = ${p.repository.id}`;
    await tx`insert into repo_events (github_repo_id, kind, detail)
      values (${p.repository.id}, ${action}, ${`${old.full_name} -> ${p.repository.full_name}`})`;
    return true;
  }
  if (event === "github_app_authorization" && action === "revoked") {
    await tx`delete from github_user_tokens where github_user_id = ${parse(SenderEv, payload).sender.id}`;
    return true;
  }
  return false;
}

/** Earliest receipt per commit for a project, newest first. */
export async function witnessedCommits(sql: SQL, projectId: string, limit = 50) {
  const rows = await sql`select * from (
      select distinct on (sha) sha, received_at, claimed_ts, ref, forced from push_receipts
      where project_id = ${projectId} order by sha, received_at
    ) t order by received_at desc limit ${limit}`;
  return rows as {
    sha: string;
    received_at: Date;
    claimed_ts: string | null;
    ref: string;
    forced: boolean;
  }[];
}
