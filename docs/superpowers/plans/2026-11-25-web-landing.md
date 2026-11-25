# Landing Page Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the logged-out `/` with the approved landing page (Stripe-derived light look: mesh hero, Inter 300, indigo pill CTAs, cream ledger band) that leads with trust and privacy.

**Architecture:** `landingPage()` in `server/src/pages.ts` builds its own HTML shell (not `page()`), which keeps returning the existing `page()` for signed-in users. Styling is Tailwind CSS precompiled at build time into `server/static/landing.css` — no CDN, no runtime JS. Inter woff2 self-hosted in `server/static/fonts/`. The landing response's CSP adds `font-src 'self'`.

**Tech Stack:** Bun + TypeScript server (backend and renderer stay as-is, no framework), Tailwind CSS v4 CLI as the only new dependency (dev-only), self-hosted Inter. Server-rendered HTML via existing `html` tagged template (`Safe`).

**Spec:** `docs/superpowers/specs/2026-11-25-web-landing-design.md` (approved; look updated in conversation: light, Stripe language, no nav bar, colored trust cards, ledger console mockup)

## Global Constraints

- No external requests at runtime: Tailwind precompiled ahead of time into one static CSS file; font self-hosted; no CDN, no analytics, no inline styles/scripts. CSP stays `default-src 'none'`; landing response may add `font-src 'self'` and nothing else.
- No inline style or script attributes anywhere in the landing markup (Tailwind classes are fine; any CSS Tailwind can't express — the mesh gradient — lives in landing.css via `@apply`-free plain rules or utility classes composed in the template).
- Mockup at `.claude/scratchpad/lp.html` is the visual reference; markup and copy follow it, with sections: mesh hero + stats, "Agent activity earns zero" + three hairline cards, cream band with navy console ledger mockup, privacy rows, setup steps, "What Tally won't claim", footer.
- Stats numbers are placeholders wired later to live counts; do not invent a backend call in this plan (`ponytail:` comment in markup marks the seam).
- Repo URL for "View source": `https://github.com/Sectersion/tally`, `rel="noopener"`. PITCH.md link → GitHub blob URL.
- Buttons: primary "Sign in with Hack Club" → `/auth/login` with inline SVG arrow icon; "View source" with GitHub octocat SVG. Both inline SVG (no asset requests).
- Type: Inter weight 300 display with negative tracking, `font-feature-settings` per mockup; `tnum` on stats and console.
- Works at 320px; `prefers-reduced-motion` respected; contrast ≥ 4.5:1.
- Signed-in users never see the landing page (existing behavior).

## Review Focus

- A visitor with JS/cookies disabled gets full static HTML (zero `<script>` in the landing shell). Test in Task 1.
- `font-src 'self'` appears only on the landing response; other pages keep strict CSP. Test in Task 1.
- No session-dependent content leaks (no `csrf` substring in landing markup). Test in Task 1.
- Build output, not Tailwind source, ships: `landing.css` must contain the compiled utilities, no `@import "tailwindcss"`. Checked in Task 2 build step.
- 320px must not overflow horizontally. Manual check in Task 4.

---

### Task 1: `landingPage()` shell + route wiring

**Files:**
- Modify: `server/src/pages.ts` (add `landingPage` near `homePage`)
- Modify: `server/src/html.ts` (export `LandingCSP = CSP + " font-src 'self'"`)
- Modify: `server/src/app.ts:419` home handler (`if (!s) return landingPage();`)
- Test: `server/test/landing.test.ts`

**Interfaces:**
- Consumes: `html`, `Safe` from `./html.ts`.
- Produces: `landingPage(): Response` — static, no args. `LandingCSP: string`.

- [ ] **Step 1: Write failing test** — `server/test/landing.test.ts`:

```ts
import { describe, test, expect } from "bun:test";
import { landingPage } from "../src/pages.ts";
import { CSP } from "../src/html.ts";

describe("landingPage", () => {
  const res = landingPage();
  const body = res.body;
  test("structure and no script/inline style/csrf", () => {
    expect((body.match(/<h1/g) ?? []).length).toBe(1);
    expect((body.match(/href="\/auth\/login"/g) ?? []).length).toBe(1);
    expect(body).toContain("View source");
    expect(body).toContain("github.com/Sectersion/tally");
    expect(body).not.toContain("<script");
    expect(body).not.toContain("csrf");
    expect(body).not.toContain("style=");
  });
  test("CSP adds font-src only on landing", () => {
    expect(res.headers.get("content-security-policy")).toBe(CSP + " font-src 'self'");
    expect(res.headers.get("content-type")).toContain("text/html");
  });
});
```

- [ ] **Step 2: Run, verify fails** — `bun test server/test/landing.test.ts` → FAIL (no `landingPage`).

- [ ] **Step 3: Implement.** `html.ts`: `export const LandingCSP = CSP + " font-src 'self'";`
  `pages.ts`: `export function landingPage(): Response` — `new Response` with `page()`'s header block but `LandingCSP` and `cache-control: public, max-age=60`; own full document linking `/static/landing.css`, markup per the mockup with Tailwind utility classes. No `page()` call, no nav.
  `app.ts` home handler: `if (!s) return landingPage();` before `homePage`.

- [ ] **Step 4: Run** — `bun test server/test/landing.test.ts server/test/` → new PASS, existing suite still passes.

- [ ] **Step 5: Commit** — `git add -A server/src server/test/landing.test.ts && git commit -m "feat(web): landing page shell and route"`

### Task 2: Tailwind build pipeline + stylesheet

**Files:**
- Create: `server/tailwind/landing.src.css` (source: `@import "tailwindcss"` + mesh/console custom rules)
- Create: `server/static/landing.css` (build output, committed)
- Modify: `server/package.json` (devDependencies: `tailwindcss@^4`, script `"build:landing": "tailwindcss -i ./tailwind/landing.src.css -o ./static/landing.css --minify"`)
- Test: checked by build output grep (Step 3) and landing.test.ts still passing

**Interfaces:**
- Consumes: class names fixed by Task 1 markup.
- Produces: `server/static/landing.css` compiled utilities + custom mesh/console rules, referenced by `landingPage()`.

- [ ] **Step 1: Install** — in `server/`: `bun add -d tailwindcss@4` (latest 4.x stable published >2 weeks ago).
- [ ] **Step 2: Write source** — `landing.src.css`:
  `@import "tailwindcss";` then plain custom classes for what utilities can't express: `.mesh::before` five-stop radial mesh, `.console` navy block + monospace rules, `h1/h2` negative tracking helpers where needed.
- [ ] **Step 3: Build** — `bun run build:landing`; verify: `grep -c "mesh" server/static/landing.css` ≥ 1 and no `@import "tailwindcss"` in output. Commit the output file.
- [ ] **Step 4: Verify page** — run server, load `/`, compare against `lp.html` mockup: mesh band, cards, console, rows, steps, limits, footer.
- [ ] **Step 5: Commit** — `git add server/tailwind server/static/landing.css server/package.json bun.lock && git commit -m "feat(web): tailwind-built landing stylesheet"`

### Task 3: font + STATIC allowlist

**Files:**
- Create: `server/static/fonts/inter-var.woff2` (Inter variable, latin subset, SIL OFL)
- Create: `server/static/fonts/LICENSE.inter` (OFL text + source URL)
- Modify: `server/src/app.ts:112` STATIC set — add `"landing.css", "fonts/inter-var.woff2"`

- [ ] **Step 1: Add font** — download Inter variable woff2 (latin subset) from the rsms/inter GitHub release into `server/static/fonts/inter-var.woff2`; save OFL license + source URL in `LICENSE.inter`.
- [ ] **Step 2: Add `"landing.css", "fonts/inter-var.woff2"` to STATIC.**
- [ ] **Step 3: Verify** — load `/`; type renders Inter; font request same-origin.
- [ ] **Step 4: Commit** — `git add server/static/fonts server/src/app.ts && git commit -m "feat(web): self-hosted Inter font, static allowlist"`

### Task 4: visual pass + accessibility sweep

**Files:**
- Modify: `server/tailwind/landing.src.css`, `server/src/pages.ts` (markup tweaks only)

- [ ] **Step 1: Manual check at 320px, 768px, desktop** — no horizontal scroll, contrast ≥ 4.5:1, focus-visible rings on both pills, tab order sane, reduced-motion honored.
- [ ] **Step 2: Rebuild + full suite** — `bun run build:landing && bun test server/test/` → PASS.
- [ ] **Step 3: Commit** — `git add -A && git commit -m "polish(web): landing visual pass"`
