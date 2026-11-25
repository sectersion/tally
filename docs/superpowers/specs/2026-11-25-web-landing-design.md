# Landing page (MVP) design

Status: approved 2026-11-25. Size: a small change (one page, one stylesheet, a new page shell). The signed-in app, the reviewer queue (#112) and the homepage overview (#115) are separate pieces that come after this.

## Intent and why

The logged-out front door of the Tally server (`/`) is one heading and one sentence. The maintainer wants the whole web side to feel calm, trustworthy and fast, and extremely polished, "like an iceberg": a simple surface with more underneath than meets the eye. The page should be tasteful in the way a good native macOS app's site is.

## What the maintainer decided

- Feel: calm, trustworthy, fast. Modern. **No liquid glass**, no blur, no gimmicks.
- Iceberg: a very simple surface; the depth is there when you go looking.
- Reference: https://screen.movie/ (black canvas, one light source, large tightly tracked type, one pill button, almost no chrome, 20px corners). Taste is the key word: every element must answer "is this needed, does it add to the site?"
- Reviewers should see risk at a glance (this belongs to the reviewer queue, not this page).
- The dashboard design skill (`dashboard-design`) is used when the signed-in dashboard starts, not for this page.

## Goals

- A logged-out `/` that someone understands in ten seconds and trusts.
- One clear action: **Sign in with Hack Club**.
- Fits the existing server: server-rendered HTML, one stylesheet, no framework, no external requests.

## Non-goals

- Pricing, testimonials, urgency copy, feature tours, comparison tables, a blog.
- Changing the signed-in `/`, project pages, profiles or settings (the next pieces restyle them).
- A light theme, a theme toggle, animation beyond a gentle fade.

## Anti-goals

- Anything that reads as a generic SaaS template (a hero plus three icon cards).
- Anything that overclaims. The page says only what the product does.
- A look that needs an image or a font from a third party.

## Constraints

- The CSP starts from `default-src 'none'` and allows only `'self'` for styles, scripts and images, so a self-hosted font needs `font-src 'self'` added. No inline styles or scripts.
- Static files are served through an allowlist (`STATIC` in `server/src/app.ts`); new files go on it.
- `page()` wraps everything in a 720px column with a header, so the landing needs its own shell. Every other page keeps its current look until its own redesign.
- Works on a phone (320px) and respects `prefers-reduced-motion`.

## Approaches

1. **Cinematic dark front door, plain app behind it (my pick).** Black canvas, one light source made of CSS gradients, huge type, one button, then one calm section below the waterline. The signed-in app can be quieter later. Cheapest, closest to the reference, and has no images to host.
2. **One look everywhere, light.** Warm paper or cool grey for both the front door and the app. Consistent, but loses the focal point the reference has.
3. **Dark everywhere.** Strong and consistent, but a reviewer reading flags for an hour needs the highest legibility, and dark everywhere would be a harder fit for dense tables.

I recommend 1. Not yet agreed with the maintainer: the open question is whether the app behind the front door should share the dark look.

## Design (my calls, for approval)

Page, top to bottom, nothing else:

1. **Nav:** a small floating pill, wordmark left, "Sign in" right. Flat fill and a hairline border; no backdrop blur.
2. **Hero (the surface):** a single light source behind the headline built from CSS radial gradients (cool blue, a hint of warm). Headline, one sentence, then two pill buttons: primary white **Sign in with Hack Club** (→ `/auth/login`) and a secondary outline **View source** (→ GitHub repo). Draft copy:
   - Headline: **Time you can prove.**
   - Sentence: Tally records the hours you spend building with AI, locks them in a log nobody can quietly edit, and has a person review them.
Trust and privacy lead everything below the hero (maintainer: trust/security over everything else):

**Waterline: "How it works"** — three short statements:
- **Recorded on your machine.** A Rust daemon reads your harness logs, editor heartbeats, git commits and OS idle time. Only hashes and numbers leave it — never your prompts or code.
- **Earned by decisions.** Agent activity earns zero. Each human action — a prompt, an approval, an edit that survives — claims only the time you were present, capped by substance.
- **Locked in a chain.** Every entry seals the one before it on the server, so history can't be backfilled or quietly rewritten.

**Section: "What we take"** (privacy, from PITCH.md):
- Never: keystrokes, screenshots, window titles, URLs, browser permissions.
- When logging: project names, action types, timestamps, seconds claimed, hashes. That's the whole list.
- Session text leaves your machine only on `tally submit`, to a self-hosted judge, deleted after review.

**Section: "Why trust it"**:
- Hours re-derivable from stored timeline, labels and rule version — anyone can check.
- Server-witnessed pushes anchor the clock; commit timestamps can't.
- A human reviews every project; the judge only routes work to them.
- We don't claim to prevent fraud. We claim faking hours costs about as much as real work.

4. **Footer:** one line, AGPL-3.0, link to the source repository, link to PITCH.md. Nothing else.

Type: Inter (variable, SIL OFL), self-hosted as a woff2 with `font-src 'self'`; system sans fallback. Large display sizes use tight negative tracking, body 16px. Colour: near-black background, white and one muted grey for text, one ice-blue accent. Corners 20px, a single 1px hairline colour. Motion: a 400ms fade-in of the hero, off when the user prefers reduced motion.

Code shape: `landingPage()` in `server/src/pages.ts` with its own shell (not `page()`), `server/static/landing.css`, the font file, both added to `STATIC`, `font-src 'self'` added to the CSP for the landing response only. Tests: the existing page tests keep passing, plus one for the landing markup (a single `h1`, one sign-in link, no inline style or script) and one for the CSP header.

## Left to the builder

Exact gradient values, the font subset, spacing scale, breakpoints, and the footer link target. If any of the copy above reads as an overclaim against `PITCH.md`, tighten it.
