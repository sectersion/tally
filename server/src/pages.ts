import type { SQL } from "bun";
import type { Config } from "./config.ts";
import {
  CSP,
  LandingCSP,
  formatHours,
  formatMinutes,
  html,
  notFound,
  page,
  Safe,
  type Viewer,
} from "./html.ts";
import { bumpChange, commitWindows, projectDaily, projectHours } from "./projects.ts";
import type { Session } from "./session.ts";
import { updLabel } from "./upd.ts";

export const viewerOf = (s: Session | null): Viewer | null =>
  s && { handle: s.handle, csrf: s.csrf };
const utc = (d: Date | number) => `${new Date(d).toISOString().slice(0, 19).replace("T", " ")} UTC`;
const csrfField = (s: { csrf: string }) =>
  html`<input type="hidden" name="csrf" value="${s.csrf}">`;
const none = html``;

/** Fixed texts for ?error= codes; anything else shows nothing (no reflected input). */
const ERRORS: Record<string, string> = {
  bad: "That handle is not valid: 1-39 lowercase letters, digits or dashes.",
  taken: "That handle is taken.",
  reserved: "That handle is reserved.",
  bad_name: "Project names look like handle/name.",
  not_allowed:
    "Name it handle/name, or owner/repo for a GitHub repo you can reach through the App.",
  repo_linked: "That repo is already linked to another project.",
  display_name: "Display names are plain text, up to 64 characters.",
  confirm: "Account not deleted: the confirmation did not match.",
};
const errorLine = (req: Request) => {
  const code = new URL(req.url).searchParams.get("error") ?? "";
  const msg = Object.hasOwn(ERRORS, code) ? ERRORS[code] : undefined; // not "constructor" etc.
  return msg ? html`<p role="alert">${msg}</p>` : none;
};

const handleForm = (s: Session) =>
  html`<h2>Set your handle first</h2><p>Your handle is permanent: it starts every project name (handle/name).</p>
<form method="post" action="/settings/handle">${csrfField(s)}<input name="handle" required maxlength="39" aria-label="Handle"> <button>Save handle</button></form>`;

/** SVG markup that is ours, not user data. */
const svg = (s: string) => new Safe(s);

// ponytail: stats are placeholder copy; wire to live counts when a counts endpoint exists
const LANDING = html`<section class="mesh"><div class="mx-auto max-w-[660px] px-4 py-24 text-center">
  <h1 class="mb-5 text-4xl font-light tracking-[-0.025em] text-ink sm:text-5xl">Time you can prove.</h1>
  <p class="mx-auto mb-7 max-w-[30em] text-[17px] leading-relaxed text-ink-2">Tally records the hours you spend building with AI, locks them in a log nobody can quietly edit, and has a person review them.</p>
  <div class="flex flex-wrap justify-center gap-3">
    <a class="pill pill-primary" href="/auth/login">${svg(`<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M15 3h4a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2h-4"/><path d="M10 17l5-5-5-5"/><path d="M15 12H3"/></svg>`)}Sign in with Hack Club</a>
    <a class="pill pill-outline" href="https://github.com/Sectersion/tally" rel="noopener">${svg(`<svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M12 .5C5.65.5.5 5.65.5 12c0 5.08 3.29 9.39 7.86 10.91.58.11.79-.25.79-.56 0-.27-.01-1.17-.02-2.12-3.2.69-3.87-1.36-3.87-1.36-.52-1.33-1.28-1.68-1.28-1.68-1.04-.71.08-.7.08-.7 1.15.08 1.76 1.18 1.76 1.18 1.03 1.75 2.69 1.25 3.35.95.1-.74.4-1.25.72-1.54-2.55-.29-5.23-1.28-5.23-5.68 0-1.26.45-2.28 1.18-3.09-.12-.29-.51-1.46.11-3.04 0 0 .96-.31 3.15 1.18a10.9 10.9 0 0 1 2.87-.39c.97 0 1.95.13 2.87.39 2.19-1.49 3.15-1.18 3.15-1.18.62 1.58.23 2.75.11 3.04.73.81 1.18 1.83 1.18 3.09 0 4.41-2.69 5.38-5.25 5.67.41.35.77 1.05.77 2.12 0 1.53-.01 2.76-.01 3.14 0 .31.21.67.8.56A11.52 11.52 0 0 0 23.5 12C23.5 5.65 18.35.5 12 .5z"/></svg>`)}View source</a>
  </div>
  <div class="mt-16 flex flex-wrap justify-center gap-x-14 gap-y-8 [font-variant-numeric:tabular-nums]">
    <div><b class="block text-[28px] font-normal tracking-[-0.02em]">128,447</b><span class="text-[13px] text-mute">minutes verified so far</span></div>
    <div><b class="block text-[28px] font-normal tracking-[-0.02em]">0</b><span class="text-[13px] text-mute">keystrokes recorded, ever</span></div>
    <div><b class="block text-[28px] font-normal tracking-[-0.02em]">100%</b><span class="text-[13px] text-mute">of hours re-derivable by anyone</span></div>
  </div>
</div></section>
<section class="py-20"><div class="mx-auto max-w-[660px] px-4 text-center">
  <h2 class="mb-4 text-3xl font-light tracking-[-0.02em] sm:text-[3rem]">Agent activity earns zero.</h2>
  <p class="mx-auto max-w-[32em] text-[17px] text-ink-2">Hackatime counts how active your agent is. Tally counts how often <b>you</b> decide something — a prompt, an approval, an edit that survives, a test run — and how long you spent getting there. Watching the agent generate earns nothing. Reading and testing its output does.</p>
  <div class="mx-auto mt-11 grid max-w-[980px] gap-5 text-left sm:grid-cols-3">
    <div class="card"><span class="tag">Substance caps</span><h3>Earned by decisions</h3><p>Each human action claims only the time you were present, capped by substance. A &quot;try again&quot; earns a minute; a real fail→pass loop earns twenty.</p></div>
    <div class="card"><span class="tag">Hash chain</span><h3>Locked in a chain</h3><p>Every entry seals the one before it, so history can&#39;t be backfilled or quietly rewritten.</p></div>
    <div class="card"><span class="tag">Human review</span><h3>Checked by a person</h3><p>A human reviews every project before its hours count. The judge only routes work to them.</p></div>
  </div>
</div></section>
<section class="band-cream py-20"><div class="mx-auto max-w-[1080px] px-4">
  <div class="mb-8 text-center">
    <h2 class="mb-4 text-3xl font-light tracking-[-0.02em] sm:text-[3rem]">The log is the point.</h2>
    <p class="mx-auto max-w-[28em] text-[17px] text-ink-2">Every entry seals the one before it — and anyone can check the math.</p>
  </div>
  <div class="console" aria-hidden="true"><b>$</b> tally verify --project tally/tally
<i>✓</i> 61h 24m verified · 214 claims re-derived
    entry 12,407  prompt        +3m  <em>capped 20m, relevant</em>
    entry 12,406  fail→pass     +14m <em>test pre-existed</em>
    entry 12,405  edit          +6m  <em>edit survived the session</em>
    entry 12,404  agent-busy    <s>+0m</s>  <em>you were idle 12 min</em>
    entry 12,403  idle          <s>+0m</s>  <em>lunch break</em>
<i>✓</i> seal matches · rule version 2026.11-1 · signed by reviewer <b>@z</b></div>
</div></section>
<section class="py-20"><div class="mx-auto max-w-[660px] px-4 text-center">
  <h2 class="mb-4 text-3xl font-light tracking-[-0.02em] sm:text-[3rem]">Your prompts never leave your machine.</h2>
  <p class="mx-auto max-w-[32em] text-[17px] text-ink-2">Logging sends project names, action types, timestamps, seconds claimed, hashes. That&#39;s the whole list. No keystrokes, no screenshots, no window titles, no URLs — none of it is ever recorded.</p>
  <p class="mx-auto mt-4 max-w-[32em] text-[17px] text-ink-2">Session text goes out only when you run <code>tally submit</code>, to a self-hosted judge, deleted after review.</p>
  <div class="rows">
    <div class="row"><h3>Never recorded</h3><p>Keystrokes, screenshots, window titles, URLs, browser permissions.</p></div>
    <div class="row"><h3>When logging</h3><p>Project names, action types, timestamps, seconds claimed, hashes. That&#39;s the whole list.</p></div>
    <div class="row"><h3>When submitting</h3><p>Scrubbed session text, your choice of project only. Self-hosted judge. Deleted after review.</p></div>
  </div>
</div></section>
<section class="bg-canvas-soft py-20"><div class="mx-auto max-w-[660px] px-4 text-center">
  <h2 class="mb-4 text-3xl font-light tracking-[-0.02em] sm:text-[3rem]">Start tracking in 30 seconds.</h2>
  <div class="steps">
    <div class="step"><b>1</b><span>Sign in with Hack Club</span></div>
    <div class="step"><b>2</b><span>Install the daemon — it finds Claude Code, Codex, Cursor, Aider and your editor plugins on its own</span></div>
    <div class="step"><b>3</b><span>Keep building. Hours show up on your public project page</span></div>
  </div>
</div></section>
<section class="py-20"><div class="mx-auto max-w-[660px] px-4 text-center">
  <h2 class="mb-4 text-3xl font-light tracking-[-0.02em] sm:text-[3rem]">What Tally won&#39;t claim.</h2>
  <div class="limits">
    <p><b>We don&#39;t prevent fraud.</b> We make faking hours cost about as much as real work. Reviewers still sample, starting with the least confident turns.</p>
    <p><b>Thinking away from the keyboard isn&#39;t counted.</b> You can claim it in one click; reviewers see the claim.</p>
    <p><b>Web builders are mostly unverified.</b> Lovable, Bolt, v0 capture timing only, for now.</p>
  </div>
</div></section>
<footer class="mx-auto flex max-w-[1080px] flex-wrap gap-5 px-4 pb-14 pt-14 text-[13px] text-mute"><span>AGPL-3.0</span><a class="hover:text-primary" href="https://github.com/Sectersion/tally" rel="noopener">Source</a><a class="hover:text-primary" href="https://github.com/Sectersion/tally/blob/main/PITCH.md" rel="noopener">PITCH.md</a></footer>`;

export function landingPage(): Response {
  return new Response(
    `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>Tally — time you can prove</title><meta name="description" content="Tally records verified coding time with AI, in a tamper-evident log, reviewed by a person."><link rel="stylesheet" href="/static/landing.css"></head><body>${LANDING.__html}</body></html>`,
    {
      headers: {
        "content-type": "text/html; charset=utf-8",
        "content-security-policy": LandingCSP,
        "x-content-type-options": "nosniff",
        "referrer-policy": "same-origin",
        "cache-control": "public, max-age=60",
      },
    },
  );
}

export async function homePage(sql: SQL, req: Request, s: Session | null): Promise<Response> {
  if (!s)
    return page(
      "Tally",
      html`<h1>Tally</h1><p>Tally records verified coding time and shows it on a public project page.</p>
<p><a href="/auth/login">Sign in with Hack Club</a></p>`,
    );
  const ps = await sql`select public_name, public from projects where owner_id = ${s.userId}
    order by lower(public_name)`;
  const list = ps.length
    ? html`<ul>${ps.map(
        (p: { public_name: string; public: boolean }) =>
          html`<li><a href="/p/${p.public_name}">${p.public_name}</a>${p.public ? "" : " (private)"}</li>`,
      )}</ul>`
    : html`<p>No projects yet.</p>`;
  return page(
    "Tally",
    html`<h1>Your projects</h1>${errorLine(req)}${s.handle ? none : handleForm(s)}${list}
<form method="post" action="/projects">${csrfField(s)}<input name="name" required maxlength="141" aria-label="Project name" placeholder="${s.handle ?? "handle"}/name"> <button>Create project</button></form>`,
    { viewer: viewerOf(s) },
  );
}

export async function profilePage(sql: SQL, handle: string, s: Session | null) {
  const [u] = await sql`select id, handle, display_name from users
    where lower(handle) = lower(${handle}) and profile_public`;
  if (!u) return notFound();
  const ps = await sql`select p.id, p.public_name,
      (select max(received_at) from push_receipts r where r.project_id = p.id) as last
    from projects p where p.owner_id = ${u.id} and p.public order by lower(p.public_name)`;
  const rows = await Promise.all(
    ps.map(async (p: { id: string; public_name: string; last: Date | null }) => {
      const hrs = await projectHours(sql, p.id);
      return html`<tr><td><a href="/p/${p.public_name}">${p.public_name}</a></td><td>${formatHours(hrs.verified_s)}</td><td>${hrs.unverified_s > 0 ? `${formatMinutes(hrs.unverified_s)} (not counted)` : ""}</td><td>${p.last ? utc(p.last) : "none"}</td></tr>`;
    }),
  );
  const name: string = u.display_name || u.handle;
  return page(
    name,
    html`<h1>${name}</h1>${
      rows.length
        ? html`<table><thead><tr><th>Project</th><th>Verified</th><th>Unverified</th><th>Last witnessed push</th></tr></thead><tbody>${rows}</tbody></table>`
        : html`<p>No public projects.</p>`
    }`,
    { viewer: viewerOf(s) },
  );
}

const FLAGS = {
  claimed_after_receipt: "claims a time after the server received it",
  pushed_late: "pushed more than 7 days after its claimed time",
} as const;

type ProjectRow = {
  id: string;
  public_name: string;
  public: boolean;
  owner_id: string;
  upd_record_id: string | null;
  change_seq: string;
  handle: string | null;
  profile_public: boolean;
};

/** The project at /p/:owner/:name if this viewer may see it (public, or theirs); else null. */
export async function visibleProject(
  sql: SQL,
  owner: string,
  name: string,
  s: Session | null,
): Promise<ProjectRow | null> {
  const [p] = await sql`select p.id, p.public_name, p.public, p.owner_id, p.upd_record_id,
      p.change_seq, u.handle, u.profile_public
    from projects p join users u on u.id = p.owner_id
    where lower(p.public_name) = lower(${`${owner}/${name}`})`;
  const mine = !!s && p?.owner_id === s.userId;
  return !p || (!p.public && !mine) ? null : (p as ProjectRow);
}

/** The project page's <main> content: the one renderer for the page and its live fragment. */
async function projectMain(
  sql: SQL,
  cfg: Pick<Config, "publicBaseUrl">,
  p: ProjectRow,
  s: Session | null,
  fetchFn: typeof fetch,
): Promise<Safe> {
  const mine = !!s && p.owner_id === s.userId;
  const [hours, daily, windows, [repo], revoked, [head]] = await Promise.all([
    projectHours(sql, p.id),
    projectDaily(sql, p.id, 90),
    commitWindows(sql, p.id),
    sql`select r.full_name, (select ev.detail from repo_events ev where ev.github_repo_id = r.id
        and ev.kind in ('renamed', 'transferred') order by ev.id desc limit 1) as moved
      from project_repos l join github_repos r on r.id = l.github_repo_id
      where l.project_id = ${p.id} and l.unlinked_at is null`,
    // Removals that cut one of this project's links, while it was this project's.
    sql`select ev.at from repo_events ev join project_repos l on l.github_repo_id = ev.github_repo_id
      and l.project_id = ${p.id} and ev.at >= l.linked_at and (l.unlinked_at is null or ev.at <= l.unlinked_at)
      where ev.kind in ('uninstalled', 'removed') order by ev.at desc`,
    sql`select max(received_at) as t from commits where project_id = ${p.id}`,
  ]);
  const upd = repo ? await updLabel(p.upd_record_id, repo.full_name, fetchFn) : null;
  const was = (repo?.moved as string | null | undefined)?.split(" -> ")[0];

  const numbers = html`<section id="numbers" data-project="${p.id}" data-seq="${p.change_seq}"><p><strong>${formatHours(hours.verified_s)} verified</strong></p>${
    hours.unverified_s > 0
      ? html`<p>${formatMinutes(hours.unverified_s)} unverified (not counted)</p>`
      : none
  }</section>`;
  const max = Math.max(1, ...daily.map((d) => d.verified_s));
  const active = daily.filter((d) => d.verified_s > 0);
  const avg = active.length ? active.reduce((n, d) => n + d.verified_s, 0) / active.length : 0;
  // ponytail: bar heights are a linear map to a 60px svg; a log or area chart can come later
  const trend = active.length
    ? html`<svg id="trend" viewBox="0 0 ${active.length * 4} 60" preserveAspectRatio="none" width="100%" height="60" role="img" aria-label="Verified hours per day over the last 90 days">${active.map(
        (d, i) =>
          html`<rect x="${i * 4}" y="${60 - Math.round((d.verified_s / max) * 60)}" width="3" height="${Math.max(1, Math.round((d.verified_s / max) * 60))}" />`,
      )}</svg>`
    : none;
  const weeks = new Map<string, number>();
  for (const d of daily) {
    // ISO week of the day (UTC): Monday-start, same as to_char('YYYY-IW')
    const [y, m, dd] = d.day.split("-").map(Number);
    const dt = new Date(Date.UTC(y, m - 1, dd));
    const dow = (dt.getUTCDay() + 6) % 7;
    const wk = new Date(dt);
    wk.setUTCDate(dt.getUTCDate() - dow);
    const key = wk.toISOString().slice(0, 10);
    weeks.set(key, (weeks.get(key) ?? 0) + d.verified_s);
  }
  const weekRows = [...weeks.entries()]
    .toSorted((a, b) => (a[0] < b[0] ? 1 : -1))
    .map(
      ([week, s]) =>
        html`<tr><td>week of ${week}</td><td>${formatHours(s)}</td></tr>`,
    );
  const days = none;
  // ponytail: pie is a two-slice svg (verified vs unverified); a legend can come later
  const pie = (() => {
    const total = hours.verified_s + hours.unverified_s;
    if (total <= 0) return none;
    const r = 42, c = 2 * Math.PI * r;
    const vf = hours.verified_s / total;
    // pie slices start at 12 o'clock, sweep clockwise
    const pieSlice = (from: number, frac: number, cls: string) => {
      if (frac >= 0.999) return html`<circle class="${cls}" cx="60" cy="60" r="${r}" />`;
      if (frac <= 0.0001) return none;
      const a = (from - 0.25) * 2 * Math.PI;
      const x = 60 + r * Math.cos(a), y = 60 + r * Math.sin(a);
      const a2 = (from + frac - 0.25) * 2 * Math.PI;
      const x2 = 60 + r * Math.cos(a2), y2 = 60 + r * Math.sin(a2);
      const large = frac > 0.5 ? 1 : 0;
      return html`<path class="${cls}" d="M60 60 L${x.toFixed(2)} ${y.toFixed(2)} A${r} ${r} 0 ${large} 1 ${x2.toFixed(2)} ${y2.toFixed(2)} Z" />`;
    };
    return html`<svg id="pie" viewBox="0 0 120 120" width="120" height="120" role="img" aria-label="${formatHours(hours.verified_s)} verified of ${formatHours(total)} claimed">${pieSlice(0, vf, "pv")}${pieSlice(vf, 1 - vf, "pu")}</svg>`;
  })();
  const pushes = windows.length
    ? html`<table><thead><tr><th>Commit</th><th>Received by server</th><th>Claimed</th><th>Ledger-claimed verified time since the previous push</th><th>Note</th></tr></thead><tbody>${windows.map(
        (w) =>
          html`<tr><td>${
            w.full_name
              ? html`<a href="https://github.com/${w.full_name}/commit/${w.sha}"><code>${w.sha.slice(0, 7)}</code></a>`
              : html`<code>${w.sha.slice(0, 7)}</code>`
          }</td><td>${utc(w.received_at)}</td><td>${w.claimed_ts === null ? "none" : `${utc(w.claimed_ts)} (claimed)`}</td><td>${formatHours(w.verified_s)}</td><td>${w.flag ? FLAGS[w.flag] : ""}</td></tr>`,
      )}</tbody></table>`
    : html`<p>No pushes witnessed yet</p>`;

  return html`<h1>${p.public_name}</h1>${
    !p.public ? html`<p role="note">This project is private: only you can see this page.</p>` : none
  }${p.profile_public && p.handle ? html`<p>Owner: <a href="/u/${p.handle}">${p.handle}</a></p>` : none}${
    repo
      ? html`<p>GitHub: <a href="https://github.com/${repo.full_name}">${repo.full_name}</a>${was && was !== repo.full_name ? ` (was ${was})` : ""}</p>`
      : none
  }${upd ? html`<p>${upd}</p>` : none}${revoked.map(
    (r: { at: Date }) =>
      html`<p>GitHub App removed on ${r.at.toISOString().slice(0, 10)}; pushes after this are not witnessed</p>`,
  )}${mine ? html`<p><a href="/p/${p.public_name}/settings">Project settings</a></p>` : none}
${numbers}${trend}${pie}
${
    active.length ? html`<p class="avg">avg ${formatHours(avg)}/day over ${active.length} active days</p>` : none
  }${days}
${
    weekRows.length > 1
      ? html`<h2>Verified by week</h2><table><thead><tr><th>Week</th><th>Verified</th></tr></thead><tbody>${weekRows}</tbody></table>`
      : none
  }
<h2>Witnessed pushes</h2><p>Each row sums the verified time of ledger entries timestamped after the previous witnessed push and at or before this one. The push times are the server's; the ledger times are claimed by the device. It says nothing about effort. Times marked claimed come from the push and are not checked.</p>${pushes}
<p>Ledger head last committed ${head?.t ? utc(head.t) : "never"}</p>
<h2>Badge</h2><pre>${`![tracked with Tally](${cfg.publicBaseUrl}/badge/${p.public_name}.svg)`}</pre>
<p><a href="/docs">Read the docs</a></p>`;
}

/** GET /docs: plain prose; one page, no nav. */
export function docsPage(): Response {
  return page(
    "Docs",
    html`<h1>Docs</h1>
<h2>Getting started</h2><p>Tally tracks the time you spend building with AI and proves it.</p>
<ol><li>Sign in with Hack Club and create a project.</li><li>Run <code>tally</code> in your terminal and follow the device-link prompt.</li><li>Link a GitHub repo on the project settings page so pushes are witnessed.</li></ol>
<p>Your project page shows your hours publicly; nothing else about you does.</p>
<h2>How verified time is computed</h2><p>The daemon watches your AI sessions and logs <em>decisions</em> — a prompt, an approval, an edit that survives, a test run. Each decision claims the minutes you were present, capped by substance: watching the agent generate earns nothing, a real fail→pass loop earns more.</p>
<p>Every entry is sealed into a hash chain. The server re-derives the totals with the same public ruleset before it counts them, so the numbers on this page are reproducible by anyone.</p>
<h2>What is recorded</h2><p>Project names, action types, timestamps, seconds claimed, hashes. That is the whole list. No keystrokes, no screenshots, no window titles, no URLs. Session text leaves your machine only when you run <code>tally submit</code>, goes to the judge, and is deleted after review.</p>
<h2>Witnessed pushes</h2><p>When you link a GitHub repo, Tally records each push it sees and attributes the ledger entries between pushes to the commit window. A push that arrives more than 7 days after its claimed time is noted, not hidden.</p>
<h2>Badge</h2><p>Every project page has a badge URL — paste it into your README:</p><pre>![tracked with Tally](https://&lt;server&gt;/badge/&lt;handle&gt;/&lt;name&gt;.svg)</pre>`,
  );
}

export async function projectPage(
  sql: SQL,
  cfg: Pick<Config, "publicBaseUrl">,
  owner: string,
  name: string,
  s: Session | null,
  fetchFn: typeof fetch,
): Promise<Response> {
  const p = await visibleProject(sql, owner, name, s);
  if (!p) return notFound();
  const body = await projectMain(sql, cfg, p, s, fetchFn);
  return page(p.public_name, body, { live: p.id, viewer: viewerOf(s) });
}

/** GET /p/:owner/:name/fragment: the page's <main> content for live.js, never cached. */
export async function projectFragment(
  sql: SQL,
  cfg: Pick<Config, "publicBaseUrl">,
  owner: string,
  name: string,
  s: Session | null,
  fetchFn: typeof fetch,
): Promise<Response> {
  const p = await visibleProject(sql, owner, name, s);
  if (!p) return notFound();
  return new Response((await projectMain(sql, cfg, p, s, fetchFn)).__html, {
    headers: {
      "content-type": "text/html; charset=utf-8",
      "content-security-policy": CSP,
      "x-content-type-options": "nosniff",
      "referrer-policy": "same-origin",
      "cache-control": "no-store",
    },
  });
}

const DELETE_PHRASE = "delete my account";

export async function settingsPage(sql: SQL, req: Request, s: Session): Promise<Response> {
  const [u] =
    await sql`select handle, display_name, profile_public from users where id = ${s.userId}`;
  const tokens = await sql`select t.id, t.label, t.created_at, p.public_name from device_tokens t
    left join projects p on p.id = t.project_id
    where t.revoked_at is null and (t.user_id = ${s.userId} or p.owner_id = ${s.userId}) order by t.id`;
  const c = csrfField(s);
  const tokenList: Safe = tokens.length
    ? html`<table><thead><tr><th>Label</th><th>Scope</th><th>Created</th><th></th></tr></thead><tbody>${tokens.map(
        (t: { id: string; label: string; created_at: Date; public_name: string | null }) =>
          html`<tr><td>${t.label || "(no label)"}</td><td>${t.public_name ?? "all your projects"}</td><td>${utc(t.created_at)}</td><td><form class="inline" method="post" action="/settings/tokens/${t.id}/revoke">${c}<button>Revoke</button></form></td></tr>`,
      )}</tbody></table>`
    : html`<p>No active tokens.</p>`;
  const confirmWord: string = u.handle ?? DELETE_PHRASE;
  return page(
    "Settings",
    html`<h1>Settings</h1>${errorLine(req)}${u.handle ? none : handleForm(s)}
<h2>Profile</h2><form method="post" action="/settings/profile">${c}
<p><label>Display name <input name="display_name" maxlength="64" value="${u.display_name ?? ""}"></label></p>
<p><label><input type="checkbox" name="profile_public"${u.profile_public ? html` checked` : none}> Public profile</label></p>
<button>Save</button></form>
<p>Your handle appears in your project names (handle/name) even when your profile is private.</p>
<h2>Devices and tokens</h2>${tokenList}
<h2>GitHub</h2><p><a href="/github/connect?return_to=%2Fsettings">Connect GitHub</a> · <a href="/github/install">Install the GitHub App</a></p>
<h2>Delete my account</h2><p>This signs you out everywhere, revokes every token, hides your projects and profile, and blanks your email and names. Ledger entries and push receipts are append-only evidence and stay stored, but nothing public shows them.</p>
<form method="post" action="/settings/delete">${c}<label>Type <strong>${confirmWord}</strong> to confirm <input name="confirm" required></label> <button>Delete my account</button></form>`,
    { viewer: viewerOf(s) },
  );
}

/** Returns an ERRORS code, or null when saved. */
export async function saveProfile(sql: SQL, userId: string, form: FormData) {
  const dn = String(form.get("display_name") ?? "").trim();
  // Cc: control characters; Cf: format characters (bidi overrides, zero-width).
  if ([...dn].length > 64 || /[\p{Cc}\p{Cf}]/u.test(dn)) return "display_name";
  await sql`update users set display_name = ${dn || null},
    profile_public = ${form.get("profile_public") === "on"} where id = ${userId}`;
  return null;
}

/** Erases access and personal fields; ledger rows and receipts stay (append-only evidence). */
export async function deleteAccount(sql: SQL, userId: string, confirm: string): Promise<boolean> {
  const [u] = await sql`select handle from users where id = ${userId}`;
  if (!u || confirm.trim().toLowerCase() !== (u.handle ?? DELETE_PHRASE)) return false;
  await sql.begin(async (tx) => {
    await tx`delete from sessions where user_id = ${userId}`;
    // An approved but unpolled code would otherwise still mint a token.
    await tx`delete from device_codes where user_id = ${userId}`;
    await tx`delete from device_tokens where user_id = ${userId}
      or project_id in (select id from projects where owner_id = ${userId})`;
    await tx`delete from github_user_tokens where user_id = ${userId}`;
    // Free the user's repos for whoever links them next; a UPD record matched the old link.
    const rs = await tx`update project_repos set unlinked_at = now() where unlinked_at is null
      and project_id in (select id from projects where owner_id = ${userId}) returning github_repo_id`;
    for (const r of rs)
      await tx`insert into repo_events (github_repo_id, kind, detail) values (${r.github_repo_id}, 'unlinked', 'account deleted')`;
    const ps =
      await tx`update projects set public = false, upd_record_id = null where owner_id = ${userId} returning id`;
    for (const p of ps) await bumpChange(tx, p.id);
    await tx`update users set email = null, name = null, display_name = null,
      verification_status = null, profile_public = false where id = ${userId}`;
  });
  return true;
}
