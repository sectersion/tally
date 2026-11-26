import type { SQL } from "bun";

export type LeaderboardRow = {
  public_name: string;
  handle: string | null;
  profile_public: boolean;
  verified_s: number;
};

/** Verified seconds summed over entries in the current UTC day or ISO week, per public project. */
export async function leaderboard(sql: SQL, range: "day" | "week"): Promise<LeaderboardRow[]> {
  const start =
    range === "day"
      ? sql`extract(epoch from date_trunc('day', now() at time zone 'UTC')) * 1000`
      : sql`extract(epoch from date_trunc('week', now() at time zone 'UTC')) * 1000`;
  const rows = await sql`select p.public_name, u.handle, u.profile_public,
      coalesce(sum(e.verified_s), 0)::bigint as v
    from ledger_entries e
    join chains c on c.id = e.chain_id
    join projects p on p.id = c.project_id and p.public and p.owner_id is not null
    join users u on u.id = p.owner_id
    where e.ts >= (${start})::bigint
    group by p.id, p.public_name, u.handle, u.profile_public
    order by v desc, p.public_name limit 25`;
  return rows.map(
    (r: { public_name: string; handle: string | null; profile_public: boolean; v: unknown }) => ({
      public_name: r.public_name,
      handle: r.handle,
      profile_public: r.profile_public,
      verified_s: Number(r.v),
    }),
  );
}
