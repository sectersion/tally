import type { SQL } from "bun";
import { type AccessibleRepo, linkRepoTx } from "./github.ts";

const NAME = /^[A-Za-z0-9](?:[A-Za-z0-9-]{0,38})\/[A-Za-z0-9._-]{1,100}$/;
const HANDLE = /^[a-z0-9](?:[a-z0-9-]{0,37}[a-z0-9])?$/;
const RESERVED = new Set(
  "auth api v1 badge static settings device github admin u p healthz".split(" "),
);

export const validName = (name: string): boolean => NAME.test(name);

export const unique = (e: unknown): boolean => {
  if (typeof e !== "object" || e === null) return false;
  const x = e as { errno?: unknown; code?: unknown };
  return x.errno === "23505" || x.code === "23505";
};

const repoNamed = (repos: AccessibleRepo[], name: string) =>
  repos.find((r) => r.full_name.toLowerCase() === name.toLowerCase());

const constraint = (e: unknown): string | undefined =>
  (e as { constraint?: string } | null)?.constraint;

/**
 * The handle rule: the first segment must be the user's handle. The GitHub repo rule (name equals
 * the full_name of a repo the user can reach) lives in createProject, which fetches the list live.
 */
export async function nameAllowed(sql: SQL, userId: string, name: string): Promise<boolean> {
  const [u] = await sql`select handle from users where id = ${userId}`;
  const owner = (name.split("/")[0] ?? "").toLowerCase();
  return !!u?.handle && u.handle === owner;
}

/** A handle can be set once: renaming would orphan `handle/` project names. */
export async function setHandle(
  sql: SQL,
  userId: string,
  raw: string,
): Promise<"ok" | "bad" | "taken" | "reserved"> {
  const handle = raw.toLowerCase();
  if (!HANDLE.test(handle)) return "bad";
  if (RESERVED.has(handle)) return "reserved";
  try {
    const r =
      await sql`update users set handle = ${handle} where id = ${userId} and handle is null returning id`;
    return r.length ? "ok" : "bad";
  } catch (e) {
    if (unique(e)) return "taken";
    throw e;
  }
}

export async function createProject(
  sql: SQL,
  userId: string,
  name: string,
  /** Live GitHub lookup for owner/repo names; only called when the handle rule does not apply. */
  listRepos?: () => Promise<AccessibleRepo[]>,
): Promise<{ id: string } | { error: "bad_name" | "not_allowed" | "taken" | "repo_linked" }> {
  if (!validName(name)) return { error: "bad_name" };
  let repo: AccessibleRepo | undefined;
  if (!(await nameAllowed(sql, userId, name))) {
    repo = listRepos ? repoNamed(await listRepos(), name) : undefined;
    if (!repo) return { error: "not_allowed" };
  }
  const id = crypto.randomUUID();
  try {
    await sql.begin(async (tx) => {
      await tx`insert into projects (id, public_name, owner_id) values (${id}, ${name}, ${userId})`;
      if (repo) {
        await linkRepoTx(tx, id, repo);
        await bumpChange(tx, id);
      }
    });
    return { id };
  } catch (e) {
    if (unique(e))
      return { error: constraint(e) === "project_repos_one_live" ? "repo_linked" : "taken" };
    throw e;
  }
}

/** The one source of project hours; unverified seconds stay separate and are never added in. */
export async function projectHours(
  sql: SQL,
  projectId: string,
): Promise<{ verified_s: number; unverified_s: number }> {
  const [r] = await sql`select coalesce(sum(e.verified_s), 0)::bigint as v,
      coalesce(sum(e.unverified_s), 0)::bigint as u
    from ledger_entries e join chains c on c.id = e.chain_id where c.project_id = ${projectId}`;
  return { verified_s: Number(r.v), unverified_s: Number(r.u) };
}

export async function projectDaily(
  sql: SQL,
  projectId: string,
  days = 90,
): Promise<{ day: string; verified_s: number; unverified_s: number }[]> {
  const rows =
    await sql`select to_char(to_timestamp(e.ts / 1000.0) at time zone 'UTC', 'YYYY-MM-DD') as day,
      sum(e.verified_s)::bigint as v, sum(e.unverified_s)::bigint as u
    from ledger_entries e join chains c on c.id = e.chain_id
    where c.project_id = ${projectId}
      and e.ts >= (extract(epoch from now()) * 1000)::bigint - ${days}::bigint * 86400000
    group by 1 order by 1`;
  return rows.map((r: { day: string; v: unknown; u: unknown }) => ({
    day: r.day,
    verified_s: Number(r.v),
    unverified_s: Number(r.u),
  }));
}

/** Call inside the caller's transaction so the notify fires only on commit. */
export async function bumpChange(tx: SQL, projectId: string): Promise<void> {
  await tx`update projects set change_seq = change_seq + 1 where id = ${projectId}`;
  await tx`select pg_notify('tally_project', ${projectId})`;
}
