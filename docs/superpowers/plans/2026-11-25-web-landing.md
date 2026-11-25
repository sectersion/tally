# Landing Page Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the logged-out `/` heading-and-sentence with a cinematic dark landing page that leads with trust and privacy.

**Architecture:** `landingPage()` in `server/src/pages.ts` builds its own HTML shell (not `page()`), which returns the existing `page()` for signed-in users. Styling is one new `server/static/landing.css`; Inter woff2 self-hosted in `server/static/fonts/`. The landing response's CSP adds `font-src 'self'`.

**Tech Stack:** Bun + TypeScript server, server-rendered HTML via existing `html` tagged template (`Safe`), plain CSS. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-11-25-web-landing-design.md` (approved)

## Global Constraints

- No external requests: font self-hosted, no CDN, no analytics. CSP stays `default-src 'none'`; landing response may add `font-src 'self'` and nothing else.
- No inline style or script attributes anywhere in the landing markup.
- Repo URL for "View source": `https://github.com/Sectersion/tally`. Footer link to PITCH.md is the site route that serves it (if none exists yet, link to the GitHub blob URL).
- Hero copy verbatim: headline "Time you can prove."; sentence "Tally records the hours you spend building with AI, locks them in a log nobody can quietly edit, and has a person review them."
- Trust-section copy verbatim from the spec's How-it-works / What-we-take / Why-trust-it bullets.
- Buttons: primary "Sign in with Hack Club" → `/auth/login`; secondary "View source" → GitHub, `rel="noopener"`.
- Works at 320px; `prefers-reduced-motion: reduce` disables the hero fade.
- Corners 20px, 1px hairlines, near-black background, white/muted-grey text, one ice-blue accent.
- Signed-in users never see the landing page (existing behavior).

## Review Focus

- A logged-out visitor with JS/cookies disabled still gets full static HTML (page is pure HTML/CSS; nothing may depend on JS). Test: markup renders as valid HTML with zero `<script>` in the landing shell.
- `font-src` must appear only on the landing response; other pages keep the strict CSP. Test: two responses compared.
- Landing markup must not leak session-dependent content (no viewer nav, no csrf fields). Test: response contains no `csrf` substring.
- Phone width 320px must not overflow horizontally. Test: CSS media query present clamping type/gradient sizes; manual check.
- Spec copy drift: if any bullet overclaims vs PITCH.md, tighten at build time; judge claims must stay routing-only.

---

### Task 1: `landingPage()` shell + route wiring

**Files:**
- Modify: `server/src/pages.ts` (add `landingPage` near `homePage`)
- Modify: `server/src/html.ts` (export `LandingCSP` = `CSP` + ` font-src 'self'`)
- Test: `server/test/landing.test.ts`

**Interfaces:**
- Consumes: `html`, `Safe` from `./html.ts`; existing `homePage(sql, req, s)` flow in `app.ts:419` stays untouched.
- Produces: `landingPage(): Response` — static, no args. `LandingCSP: string`.

- [ ] **Step 1: Write failing test** — `server/test/landing.test.ts`:

```ts
import { describe, test, expect } from "bun:test";
import { landingPage } from "../src/pages.ts";
import { CSP } from "../src/html.ts";

describe("landingPage", () => {
  const res = landingPage();
  const body = res.body;
  test("one h1, one sign-in link, one view-source link, no scripts", () => {
    expect((body.match(/<h1/g) ?? []).length).toBe(1);
    expect((body.match(/href="\/auth\/login"/g) ?? []).length).toBe(1);
    expect(body).toContain("View source");
    expect(body).not.toContain("<script");
    expect(body).not.toContain("csrf");
    expect(body).not.toContain("style=");
  });
  test("CSP adds font-src only on landing", () => {
    expect(res.headers.get("content-security-policy")).toBe(
      CSP + " font-src 'self'");
    expect(res.headers.get("content-type")).toContain("text/html");
  });
});
```

- [ ] **Step 2: Run, verify fails** — `bun test server/test/landing.test.ts` → FAIL (no `landingPage`).

- [ ] **Step 3: Implement.** In `html.ts` add:
  `export const LandingCSP = CSP + " font-src 'self'";`
  In `pages.ts` add `export function landingPage(): Response` returning a `new Response` with the same header set as `page()` (copy `page()`'s header block) but with `LandingCSP`, `cache-control: public, max-age=60`, and its own full document: `<!doctype html>…<link rel="stylesheet" href="/static/landing.css">` + hero/sections/footer markup per spec. No `page()` call, no header nav.
  In `app.ts` home handler, change to `if (!s) return landingPage();` before `homePage`.

- [ ] **Step 4: Run** — `bun test server/test/landing.test.ts` → PASS; `bun test server/test/` → existing tests still pass.

- [ ] **Step 5: Commit** — `git add -A server/src server/test/landing.test.ts && git commit -m "feat(web): landing page shell and route"`

### Task 2: landing.css — hero, sections, footer

**Files:**
- Create: `server/static/landing.css`
- Test: covered by Task 3 + manual visual check

**Interfaces:**
- Consumes: class names fixed in Task 1 markup. Use BEM-lite: `.lp-hero`, `.lp-nav`, `.lp-section`, `.lp-pill`, `.lp-pill--ghost`, `.lp-foot`.

- [ ] **Step 1: Implement stylesheet.** Near-black bg (`#0a0d12`); hero light source via two `radial-gradient`s (ice-blue `#8fd3ff` at low alpha, warm hint `#ffd9a0` lower alpha) as absolutely-positioned layers behind the headline. Display type clamp(2.5rem, 8vw, 4.5rem), `letter-spacing: -0.03em`. `.lp-pill--ghost` = transparent fill + 1px hairline. Waterline: `.lp-section` top hairline + small label. Reduced-motion media query zeroes the fade animation. Max content width 720px; 16px side gutter; everything stacks at 320px.

- [ ] **Step 2: Commit** — `git add server/static/landing.css && git commit -m "feat(web): landing stylesheet"`

### Task 3: font + STATIC allowlist + CSP wiring

**Files:**
- Create: `server/static/fonts/inter-var.woff2` (variable Inter, SIL OFL, latin subset)
- Modify: `server/src/app.ts:112` STATIC set — add `"landing.css", "fonts/inter-var.woff2"`

**Interfaces:**
- Consumes: STATIC allowlist + `/static/:file` route at `app.ts:292-296`.
- Produces: landing.css and the font served under `/static/`.

- [ ] **Step 1: Add font file.** Download Inter variable woff2 (latin subset) from the rsms/inter release into `server/static/fonts/inter-var.woff2`; record the source URL and license in `server/static/fonts/LICENSE.inter` (copy the OFL text from the release).
- [ ] **Step 2: Add `"landing.css", "fonts/inter-var.woff2"` to the STATIC set.**
- [ ] **Step 3: Verify manually** — run the server, load `/`, confirm type renders in Inter and devtools shows the font request is same-origin.
- [ ] **Step 4: Commit** — `git add server/static/fonts server/src/app.ts && git commit -m "feat(web): self-hosted Inter font, static allowlist"`

### Task 4: visual pass + accessibility sweep

**Files:**
- Modify: `server/static/landing.css`, `server/src/pages.ts` (markup tweaks only)

- [ ] **Step 1: Manual check at 320px, 768px, desktop** — no horizontal scroll, contrast ≥ 4.5:1 for body text, focus-visible rings on both buttons, buttons reachable by tab order.
- [ ] **Step 2: Run full suite** — `bun test server/test/` → PASS.
- [ ] **Step 3: Commit** — `git add -A && git commit -m "polish(web): landing visual pass"`
