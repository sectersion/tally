import type { SQL } from "bun";
import { esc } from "./html.ts";
import { badge, verifiedSeconds } from "./ledger.ts";
import { log } from "./log.ts";

type Color = "blue" | "lightgrey";
const FILL: Record<Color, string> = { blue: "#007ec6", lightgrey: "#9f9f9f" };
const LABEL = badge(0).label;
const CHAR = 6.5;
const PAD = 10;
const H = 20;

/** Flat shields-style badge. Width is estimated (no font metrics): 6.5 px per character plus padding. */
export function renderSvg(label: string, message: string, color: Color): string {
  const w = (s: string) => Math.ceil(s.length * CHAR + 2 * PAD);
  const lw = w(label);
  const mw = w(message);
  const text = (x: number, s: string) =>
    `<text x="${x}" y="14" fill="#fff" font-family="Verdana,DejaVu Sans,sans-serif" font-size="11" text-anchor="middle">${esc(s)}</text>`;
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${lw + mw}" height="${H}" role="img" aria-label="${esc(label)}: ${esc(message)}"><rect width="${lw}" height="${H}" fill="#555"/><rect x="${lw}" width="${mw}" height="${H}" fill="${FILL[color]}"/>${text(lw / 2, label)}${text(lw + mw / 2, message)}</svg>`;
}

const SECURITY = {
  "x-content-type-options": "nosniff",
  "content-security-policy": "default-src 'none'",
};

const respond = (
  ext: "svg" | "json",
  status: number,
  message: string,
  color: Color,
  extra: Record<string, string>,
) =>
  ext === "svg"
    ? new Response(renderSvg(LABEL, message, color), {
        status,
        headers: { "content-type": "image/svg+xml; charset=utf-8", ...SECURITY, ...extra },
      })
    : Response.json(
        { ...badge(0), message, color },
        { status, headers: { ...SECURITY, ...extra } },
      );

/** 429 in the badge's own format: it renders inside <img>, so JSON would show a broken image. */
export const rateLimited = (ext: "svg" | "json") =>
  respond(ext, 429, "rate limited", "lightgrey", {
    "cache-control": "no-store",
    "retry-after": "60",
  });

/**
 * `/badge/<public_name>.svg|.json`. Resolves by Tally name (survives GitHub renames); one lookup
 * query, then the same verifiedSeconds as /v1/projects/:id/badge.json. Never reads unverified time.
 */
export async function badgeResponse(
  sql: SQL,
  name: string,
  ext: "svg" | "json",
  ifNoneMatch: string | null,
): Promise<Response> {
  const out = respond.bind(null, ext);
  try {
    // owner_id null (an unowned project) is unknown, like the page and summary routes.
    const [p] = await sql`select id, public, change_seq from projects
      where lower(public_name) = lower(${name}) and owner_id is not null`;
    if (!p) return out(404, "unknown", "lightgrey", { "cache-control": "public, max-age=60" });
    // Private: no etag (it would leak the project id and change_seq).
    if (!p.public)
      return out(200, "private", "lightgrey", { "cache-control": "public, max-age=60" });
    const etag = `"${p.id}-${p.change_seq}"`;
    const cache = {
      "cache-control": "public, max-age=300, stale-while-revalidate=3600",
      etag,
    };
    const tags = (ifNoneMatch ?? "").split(",").map((t) => t.trim().replace(/^W\//, ""));
    if (tags.includes(etag) || tags.includes("*"))
      return new Response(null, { status: 304, headers: cache });
    const s = (await verifiedSeconds(sql, p.id)) ?? 0;
    const b = badge(s);
    return out(200, b.message, "blue", cache);
  } catch (e) {
    log("error", e instanceof Error ? e.message : String(e)); // message only
    return out(503, "unavailable", "lightgrey", { "cache-control": "no-store" });
  }
}
