import { HttpError } from "./errors.ts";

const URL_LIST = "https://ships.hackclub.com/api/v1/ysws_entries";
const TTL_MS = 10 * 60_000;

export type UpdEntry = { id: string; ysws: string; approved_at: number | null };

/** `owner/repo` (lowercase) for github.com URLs only; `.git`, a trailing `/` and `/tree|blob/...` are dropped. */
export function normalizeRepoUrl(url: string): string | null {
  let u: URL;
  try {
    u = new URL(url);
  } catch {
    return null;
  }
  if (
    (u.protocol !== "https:" && u.protocol !== "http:") ||
    u.hostname.toLowerCase() !== "github.com"
  )
    return null;
  const [owner, repo] = u.pathname.split("/").filter(Boolean);
  const name = repo?.replace(/\.git$/i, "");
  return owner && name ? `${owner}/${name}`.toLowerCase() : null;
}

// Only the three columns we show plus the repo they match on are kept; the rest of each
// entry (names, Slack ids, descriptions) is dropped at parse time.
// ponytail: one process-wide cache of the ~13 MB list's slim form; a nightly table if the list outgrows 20 MB.
type Slim = UpdEntry & { repo: string };
let cache: { at: number; list: Slim[] } | null = null;
export const resetUpdCache = () => {
  cache = null;
};

async function load(fetchFn: typeof fetch): Promise<Slim[]> {
  if (cache && Date.now() - cache.at < TTL_MS) return cache.list;
  const res = await fetchFn(URL_LIST, { headers: { accept: "application/json" } });
  if (!res.ok) throw new HttpError(502, { error: "upd_unavailable" });
  const raw: unknown = await res.json().catch(() => null);
  if (!Array.isArray(raw)) throw new HttpError(502, { error: "upd_unavailable" });
  const list: Slim[] = [];
  for (const e of raw as Record<string, unknown>[]) {
    const repo = typeof e?.code_url === "string" ? normalizeRepoUrl(e.code_url) : null;
    if (!repo || typeof e.id !== "string" || typeof e.ysws !== "string") continue;
    list.push({
      id: e.id,
      ysws: e.ysws,
      approved_at: typeof e.approved_at === "number" ? e.approved_at : null,
      repo,
    });
  }
  cache = { at: Date.now(), list };
  return list;
}

export async function findUpdEntries(
  fullName: string,
  fetchFn: typeof fetch = fetch,
): Promise<UpdEntry[]> {
  const want = fullName.toLowerCase();
  return (await load(fetchFn))
    .filter((e) => e.repo === want)
    .map(({ id, ysws, approved_at }) => ({ id, ysws, approved_at }));
}

/** Public line for a linked project: program name and record id only, never other fields. Unescaped. */
export async function updLabel(
  recordId: string | null,
  fullName: string,
  fetchFn: typeof fetch = fetch,
): Promise<string | null> {
  if (!recordId) return null;
  // Ships being down must not break a public page: fall back to the record id alone.
  const found = await findUpdEntries(fullName, fetchFn).catch(() => []);
  const e = found.find((x) => x.id === recordId);
  return `Unified Projects DB: ${e ? `${e.ysws} ` : ""}(record ${recordId})`;
}
