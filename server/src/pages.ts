import type { SQL } from "bun";
import type { Config } from "./config.ts";
import {
  CSP,
  formatHours,
  formatMinutes,
  html,
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
  const days = daily.length
    ? html`<table><thead><tr><th>Day (UTC)</th><th>Verified</th><th></th><th>Unverified (not counted)</th></tr></thead><tbody>${daily
        .toReversed()
        .map(
          (d) =>
            html`<tr><td>${d.day}</td><td>${formatHours(d.verified_s)}</td><td><progress max="${max}" value="${d.verified_s}">${formatHours(d.verified_s)}</progress></td><td>${d.unverified_s > 0 ? formatMinutes(d.unverified_s) : ""}</td></tr>`,
        )}</tbody></table>`
    : html`<p>No verified time in the last 90 days.</p>`;
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
  }${numbers}${p.profile_public && p.handle ? html`<p>Owner: <a href="/u/${p.handle}">${p.handle}</a></p>` : none}${
    repo
      ? html`<p>GitHub: <a href="https://github.com/${repo.full_name}">${repo.full_name}</a>${was && was !== repo.full_name ? ` (was ${was})` : ""}</p>`
      : none
  }${upd ? html`<p>${upd}</p>` : none}${revoked.map(
    (r: { at: Date }) =>
      html`<p>GitHub App removed on ${r.at.toISOString().slice(0, 10)}; pushes after this are not witnessed</p>`,
  )}${mine ? html`<p><a href="/p/${p.public_name}/settings">Project settings</a></p>` : none}
<h2>Daily verified time</h2>${days}
<h2>Witnessed pushes</h2><p>Each row sums the verified time of ledger entries timestamped after the previous witnessed push and at or before this one. The push times are the server's; the ledger times are claimed by the device. It says nothing about effort. Times marked claimed come from the push and are not checked.</p>${pushes}
<p>Ledger head last committed ${head?.t ? utc(head.t) : "never"}</p>
<h2>Badge</h2><pre>${`![tracked with Tally](${cfg.publicBaseUrl}/badge/${p.public_name}.svg)`}</pre>`;
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
