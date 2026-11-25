import { formatHours } from "./ledger.ts";

export { formatHours };

const ENT: Record<string, string> = {
  "&": "&amp;",
  "<": "&lt;",
  ">": "&gt;",
  '"': "&quot;",
  "'": "&#39;",
};
/** The one escaping function. */
export const esc = (v: string | number): string =>
  String(v).replace(/[&<>"']/g, (c) => ENT[c] as string);

/** Markup that is already escaped. A class, so a `{ __html }` object from user data is not Safe. */
export class Safe {
  readonly __html: string;
  constructor(s: string) {
    this.__html = s;
  }
}

type Part = string | number | Safe | readonly Part[];
const part = (v: Part): string =>
  v instanceof Safe
    ? v.__html
    : Array.isArray(v)
      ? v.map(part).join("")
      : esc(v as string | number);

/** Escapes every interpolation unless it is Safe (or an array of parts). */
export const html = (s: TemplateStringsArray, ...v: Part[]): Safe =>
  new Safe(s.reduce((a, lit, i) => a + lit + (i < v.length ? part(v[i] as Part) : ""), ""));

/** Whole minutes; 1-59 s is "<1 min", never "0 min". */
export const formatMinutes = (s: number) =>
  s > 0 && s < 60 ? "<1 min" : `${Math.floor(s / 60)} min`;

export const CSP =
  "default-src 'none'; style-src 'self'; script-src 'self'; connect-src 'self'; img-src 'self'; font-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'";
/** Same as CSP now that all pages self-host Inter; kept as its own name for landing-specific additions. */
export const LandingCSP = CSP;

/** The signed-in viewer, if any. handle is null until they pick one. */
export type Viewer = { handle: string | null; csrf: string };

export function page(
  title: string,
  body: Safe,
  opts: { status?: number; live?: string; viewer?: Viewer | null } = {},
): Response {
  const v = opts.viewer;
  const nav = v
    ? html`<a href="/settings">${v.handle ?? "Account"}</a>
<form method="post" action="/auth/logout"><input type="hidden" name="csrf" value="${v.csrf}"><button>Log out</button></form>`
    : html`<a href="/auth/login">Sign in</a>`;
  const live = opts.live === undefined ? "" : `<script src="/static/live.js" defer></script>`;
  const doc = html`<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>${title}</title><link rel="stylesheet" href="/static/style.css">${new Safe(live)}<header><a href="/">Tally</a> ${nav}</header><main>${body}</main></html>`;
  return new Response(doc.__html, {
    status: opts.status ?? 200,
    headers: {
      "content-type": "text/html; charset=utf-8",
      "content-security-policy": CSP,
      "x-content-type-options": "nosniff",
      "referrer-policy": "same-origin",
      "cache-control": v ? "no-store" : "public, max-age=60",
    },
  });
}

/** One 404 for every missing or private thing. */
export const notFound = () => page("Not found", html`<h1>Not found</h1>`, { status: 404 });
