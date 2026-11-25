import type { SQL } from "bun";
import { HttpError } from "./errors.ts";
import { leaderboard } from "./leaderboard.ts";
import { projectHours } from "./projects.ts";

/** The public profile's user row, or 404: private profiles do not exist to the API. */
async function publicUser(sql: SQL, handle: string) {
  const [u] = await sql`select id, handle, display_name from users
    where lower(handle) = lower(${handle}) and profile_public`;
  if (!u) throw new HttpError(404, { error: "not_found" });
  return u as { id: string; handle: string; display_name: string | null };
}

/** GET /api/v1/users/:handle/stats */
export async function userStats(sql: SQL, handle: string): Promise<Response> {
  const u = await publicUser(sql, handle);
  const [[tot], [cnt], [roll]] = await Promise.all([
    sql`select coalesce(sum(c.verified_s), 0)::bigint as v, coalesce(sum(c.unverified_s), 0)::bigint as uv
      from chains c join projects p on p.id = c.project_id where p.owner_id = ${u.id} and p.public`,
    sql`select count(*)::int as n from projects p where p.owner_id = ${u.id} and p.public`,
    // Daily rollup over the user's public projects, after today (streak) and within 30 days (active count).
    sql`with roll as (
      select to_char(to_timestamp(e.ts / 1000.0) at time zone 'UTC', 'YYYY-MM-DD') as day,
        sum(e.verified_s)::bigint as v
      from ledger_entries e join chains c on c.id = e.chain_id
      join projects p on p.id = c.project_id
      where p.owner_id = ${u.id} and p.public and e.ts >= (extract(epoch from now()) * 1000)::bigint - 31::bigint * 86400000
      group by 1)
    select array_agg(day order by day desc) as days from roll where v > 0`,
  ]);
  const days = (roll?.days as string[] | null) ?? [];
  const streak = (() => {
    if (!days.length) return 0;
    const n = new Date();
    const today = Date.UTC(n.getUTCFullYear(), n.getUTCMonth(), n.getUTCDate());
    const ms = (d: string) => new Date(`${d}T00:00:00Z`).getTime();
    const first = days[0] as string;
    if (ms(first) < today - 86_400_000) return 0;
    let c = 0;
    for (let t = ms(first); days[c] && ms(days[c] as string) === t; c++, t -= 86_400_000);
    return c;
  })();
  return Response.json(
    {
      handle: u.handle,
      display_name: u.display_name,
      verified_s: Number(tot.v),
      unverified_s: Number(tot.uv),
      projects_count: Number(cnt.n),
      streak_days: streak,
      active_days_last_30: days.length,
    },
    { headers: { "cache-control": "public, max-age=60" } },
  );
}

/** GET /api/v1/users/:handle/projects */
export async function userProjects(sql: SQL, handle: string): Promise<Response> {
  const u = await publicUser(sql, handle);
  const ps = await sql`select id, public_name from projects
    where owner_id = ${u.id} and public order by lower(public_name)`;
  const rows = await Promise.all(
    (ps as { id: string; public_name: string }[]).map(async (p) => {
      const [last] =
        await sql`select max(received_at) as t from push_receipts where project_id = ${p.id}`;
      return {
        public_name: p.public_name,
        ...(await projectHours(sql, p.id)),
        last_activity_at: last?.t ?? null,
      };
    }),
  );
  return Response.json(rows, { headers: { "cache-control": "public, max-age=60" } });
}

/** GET /api/v1/leaderboard?range=day|week */
export async function leaderboardApi(sql: SQL, range: string | null): Promise<Response> {
  if (range !== "day" && range !== "week")
    throw new HttpError(400, {
      error: "invalid_body",
      issues: [{ path: "range", message: "must be day or week" }],
    });
  const rows = await leaderboard(sql, range);
  return Response.json(
    rows.map((r, i) => ({
      rank: i + 1,
      public_name: r.public_name,
      handle: r.profile_public ? r.handle : null,
      verified_s: r.verified_s,
    })),
    { headers: { "cache-control": "public, max-age=60" } },
  );
}
