import type { SQL } from "bun";
import type { Config } from "./config.ts";
import {
  CSP,
  formatHours,
  formatMinutes,
  html,
  LandingCSP,
  notFound,
  page,
  type Safe,
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

const LANDING = html`<section class="mesh"><div class="mx-auto max-w-[1080px] px-4 pb-20 pt-20 sm:pt-24">
<div class="max-w-[720px]">
<h1 class="mb-5 text-[2.6rem] font-light leading-[1.05] tracking-[-0.025em] text-ink sm:text-[3.5rem]">Time you can prove.</h1>
<p class="mb-8 max-w-[32em] text-[18px] leading-relaxed text-ink-2">Tally logs the hours you spend building with AI, seals them against edits, and has a person review them.</p>
<div class="flex flex-wrap items-center gap-3">
<a class="pill pill-primary" href="#install">Install Tally</a>
<a class="pill pill-outline" href="/auth/login">Sign in with Hack Club</a>
</div>
</div>
</div></section>
<section class="py-16 sm:py-20"><div class="mx-auto max-w-[1080px] px-4">
<h2 class="mb-4 max-w-[18em] text-[2rem] font-light tracking-[-0.02em] text-ink sm:text-[2.75rem] sm:leading-[1.1]">Agent activity earns zero.</h2>
<p class="mb-12 max-w-[46em] text-[17px] leading-relaxed text-ink-2">Hackatime counts how active your agent is. Tally counts how often <b>you</b> decide something, and how long you spent getting to that decision. Watching the agent generate earns nothing. Reading, testing and redirecting it does.</p>
<div class="grid gap-12 sm:grid-cols-2 sm:gap-16">
<div>
<h3 class="ledger-head">Earns time</h3>
<ul class="ledger">
<li><b>Prompt</b><span>capped by how specific and relevant it is, up to about 20 min</span></li>
<li><b>Redirect or interrupt</b><span>you caught the agent going the wrong way</span></li>
<li><b>Edit that survives</b><span>still in the diff when the session ends</span></li>
<li><b>Test run</b><span>most when it is a real fail to pass loop</span></li>
<li><b>Commit</b><span>the server witnesses the push as it happens</span></li>
</ul>
</div>
<div>
<h3 class="ledger-head is-zero">Earns zero</h3>
<ul class="ledger is-zero">
<li><b>Agent generating</b><span>you are not in the loop</span></li>
<li><b>Idle or away</b><span>the OS idle timer answers, not you</span></li>
<li><b>Background runs</b><span>a build finishing while you sleep</span></li>
<li><b>Backfilled history</b><span>the chain cannot be rewritten later</span></li>
<li><b>Repeated try again</b><span>a minute each, however many you send</span></li>
</ul>
</div>
</div>
</div></section>
<section class="band-cream py-16 sm:py-20"><div class="mx-auto max-w-[1080px] px-4">
<div class="grid gap-10 lg:grid-cols-[0.85fr_1.15fr] lg:items-center lg:gap-14">
<div>
<h2 class="mb-4 text-[2rem] font-light tracking-[-0.02em] text-ink sm:text-[2.5rem] sm:leading-[1.1]">The log is the point.</h2>
<p class="mb-4 max-w-[30em] text-[17px] leading-relaxed text-ink-2">Anyone can recompute an hour from the ledger without having been there. That is what makes the number safe to put in a program.</p>
<p class="max-w-[30em] text-[17px] leading-relaxed text-ink-2">A person still reviews every project before its hours count. The judge only sorts turns so the reviewer reads the weakest claims first.</p>
</div>
<figure class="m-0 min-w-0">
<div class="console" aria-hidden="true"><b>$</b> tally verify tally/tally
<i>ok</i> 61h 24m verified, 214 claims re-derived
   12,407 prompt     +3m  <em>capped 20m</em>
   12,406 fail-pass  +14m <em>test pre-existed</em>
   12,405 edit       +6m  <em>survived session</em>
   12,404 agent-busy <s>+0m</s> <em>idle 12 min</em>
   12,403 idle       <s>+0m</s> <em>lunch break</em>
<i>ok</i> seal matches, rule 2026.11-1
   signed by reviewer <b>@z</b></div>
<figcaption class="mt-3 text-[13px] leading-relaxed text-ink-2">Sample output. The numbers are a shape, not a live total.</figcaption>
</figure>
</div>
</div></section>
<section class="py-16 sm:py-20"><div class="mx-auto max-w-[1080px] px-4">
<h2 class="mb-4 max-w-[20em] text-[2rem] font-light tracking-[-0.02em] text-ink sm:text-[2.5rem] sm:leading-[1.1]">Why a log instead of a bigger review team.</h2>
<p class="mb-12 max-w-[44em] text-[17px] leading-relaxed text-ink-2">At Hack Club&#39;s last Stardance, 123,400 hours went in through Hackatime and 27,600 came out approved. Some of that gap is people trying it on. Most of it is that a submission gives a reviewer nothing to check against.</p>
<dl class="stats">
<div><dt>123,400</dt><dd>hours submitted</dd></div>
<div><dt>27,600</dt><dd>hours approved</dd></div>
<div><dt>22%</dt><dd>of them approved</dd></div>
</dl>
</div></section>
<section class="py-16 sm:py-20"><div class="mx-auto max-w-[1080px] px-4">
<h2 class="mb-4 text-[2rem] font-light tracking-[-0.02em] text-ink sm:text-[2.5rem] sm:leading-[1.1]">What Tally won&#39;t claim.</h2>
<div class="grid gap-8 sm:grid-cols-3 sm:gap-10">
<p class="text-[15px] leading-relaxed text-ink-2"><b class="mb-1 block text-ink">It is not fraud proof.</b>Faking hours costs about as much as doing them. Reviewers still sample, starting with the least confident turns.</p>
<p class="text-[15px] leading-relaxed text-ink-2"><b class="mb-1 block text-ink">Thinking away from the keyboard is not counted.</b>You can claim it in one click, and the reviewer sees that you did.</p>
<p class="text-[15px] leading-relaxed text-ink-2"><b class="mb-1 block text-ink">Web builders are mostly unverified.</b>Lovable, Bolt and v0 capture timing only, for now.</p>
</div>
</div></section>
<section class="bg-canvas-soft py-16 sm:py-20"><div class="mx-auto max-w-[1080px] px-4">
<h2 class="mb-4 max-w-[22em] text-[2rem] font-light tracking-[-0.02em] text-ink sm:text-[2.5rem] sm:leading-[1.1]">Your prompts never leave your machine.</h2>
<p class="mb-12 max-w-[40em] text-[17px] leading-relaxed text-ink-2">No keystrokes, no screenshots, no window titles, no URLs. Not recorded at all, not filtered later.</p>
<dl class="take">
<div><dt>While logging</dt><dd>Project name, action type, timestamp, seconds claimed, reason code, numeric text features, head hashes.</dd></div>
<div><dt>When you submit</dt><dd>Scrubbed session text for the project you chose, to a self-hosted judge, deleted after review.</dd></div>
<div><dt>If you publish</dt><dd>Your ledger and its labels, on a <code>tally-data</code> branch you control.</dd></div>
</dl>
</div></section>
<section id="install" class="py-16 sm:py-20"><div class="mx-auto max-w-[1080px] px-4">
<div class="grid gap-10 lg:grid-cols-[0.8fr_1.2fr] lg:items-start lg:gap-14">
<div>
<h2 class="mb-4 text-[2rem] font-light tracking-[-0.02em] text-ink sm:text-[2.5rem] sm:leading-[1.1]">Install.</h2>
<p class="max-w-[26em] text-[17px] leading-relaxed text-ink-2">One binary, no daemon to babysit. It reads Claude Code and Copilot session logs today; Codex, Cursor, Aider and Gemini CLI are detected but not parsed yet, and <code>tally doctor</code> says which is which. Your editor plugins keep talking to Hackatime if you want them to.</p>
</div>
<div>
<div class="snippet"><b>macOS, Linux</b><code>curl -fsSL https://raw.githubusercontent.com/sectersion/tally/main/install/install.sh | sh</code></div>
<div class="snippet"><b>Windows</b><code>irm https://raw.githubusercontent.com/sectersion/tally/main/install/install.ps1 | iex</code></div>
<div class="snippet"><b>Then</b><code>tally setup &amp;&amp; tally status --brief</code></div>
<p class="mt-4 text-[13px] leading-relaxed text-mute">Pre-alpha, so the installer builds from source until the first signed release exists. <code>tally setup --dry-run</code> shows exactly what it would change before it changes it, and <code>sh -s -- --uninstall</code> removes the binary without touching your data.</p>
</div>
</div>
</div></section>
<section class="bg-canvas-soft py-16 sm:py-20"><div class="mx-auto max-w-[1080px] px-4">
<div class="grid gap-10 lg:grid-cols-[0.8fr_1.2fr] lg:gap-14">
<h2 class="text-[2rem] font-light tracking-[-0.02em] text-ink sm:text-[2.5rem] sm:leading-[1.1]">Who built this.</h2>
<div class="max-w-[38em]">
<p class="mb-4 text-[17px] leading-relaxed text-ink-2">Built by <b>sectersion</b> for the Hack Club AI time tracking bounty. The point of the bounty is a number a program can trust, so the tool has to be readable by someone who did not write it.</p>
<p class="text-[17px] leading-relaxed text-ink-2">If a claim on this page does not hold up, open an issue and it gets fixed or the claim gets deleted.</p>
<div class="mt-6 flex flex-wrap gap-x-6 gap-y-2 text-[14px]">
<a class="link" href="https://github.com/Sectersion/tally" rel="noopener">Read the code</a>
<a class="link" href="https://github.com/Sectersion/tally/blob/main/PITCH.md" rel="noopener">Pitch and threat model</a>
<a class="link" href="https://github.com/Sectersion/tally/issues" rel="noopener">Open an issue</a>
</div>
</div>
</div>
</div></section>
<footer class="mx-auto flex max-w-[1080px] flex-wrap items-center gap-x-6 gap-y-3 px-4 py-14 text-[13px] text-mute"><span>AGPL-3.0</span><a class="link" href="https://github.com/Sectersion/tally" rel="noopener">Source</a><a class="link" href="https://github.com/Sectersion/tally/blob/main/PITCH.md" rel="noopener">PITCH.md</a><a class="link" href="https://github.com/Sectersion/tally/blob/main/README.md" rel="noopener">README</a><span>%VERIFIED_MIN% minutes verified so far</span></footer>`;

export async function landingPage(sql: SQL): Promise<Response> {
  // Real counts, not marketing copy: the product is a proof-of-honesty ledger.
  const [c] = await sql`select coalesce(sum(verified_s), 0) as s from chains`;
  const verifiedMin = Math.floor(Number(c?.s ?? 0) / 60);
  return new Response(
    `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>Tally — time you can prove</title><meta name="description" content="Tally counts the hours you spend making decisions with AI, seals them in a tamper-evident log, and has a person review them."><link rel="stylesheet" href="/static/landing.css"></head><body>${LANDING.__html.replace(
      "%VERIFIED_MIN%",
      verifiedMin.toLocaleString("en-US"),
    )}</body></html>`,
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
    const [y, m, dd] = d.day.split("-").map(Number) as [number, number, number];
    const dt = new Date(Date.UTC(y, m - 1, dd));
    const dow = (dt.getUTCDay() + 6) % 7;
    const wk = new Date(dt);
    wk.setUTCDate(dt.getUTCDate() - dow);
    const key = wk.toISOString().slice(0, 10);
    weeks.set(key, (weeks.get(key) ?? 0) + d.verified_s);
  }
  const weekRows = [...weeks.entries()]
    .toSorted((a, b) => (a[0] < b[0] ? 1 : -1))
    .map(([week, s]) => html`<tr><td>week of ${week}</td><td>${formatHours(s)}</td></tr>`);
  const days = none;
  // ponytail: pie is a two-slice svg (verified vs unverified); a legend can come later
  const pie = (() => {
    const total = hours.verified_s + hours.unverified_s;
    if (total <= 0) return none;
    const r = 42;
    const vf = hours.verified_s / total;
    // pie slices start at 12 o'clock, sweep clockwise
    const pieSlice = (from: number, frac: number, cls: string) => {
      if (frac >= 0.999) return html`<circle class="${cls}" cx="60" cy="60" r="${r}" />`;
      if (frac <= 0.0001) return none;
      const a = (from - 0.25) * 2 * Math.PI;
      const x = 60 + r * Math.cos(a),
        y = 60 + r * Math.sin(a);
      const a2 = (from + frac - 0.25) * 2 * Math.PI;
      const x2 = 60 + r * Math.cos(a2),
        y2 = 60 + r * Math.sin(a2);
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
  active.length
    ? html`<p class="avg">avg ${formatHours(avg)}/day over ${active.length} active days</p>`
    : none
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
