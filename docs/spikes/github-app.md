# Spike: GitHub App install and webhooks

Checked 2026-11-23, public docs only. No GitHub App was created and no real delivery was captured (needs the maintainer's logged-in GitHub). **The fixtures in `server/test/fixtures/github/` are hand-built from the documented payload shapes with invented values, not captured.** They are regenerated and signed with the test secret `test-secret` by `server/scripts/sign-fixture.ts` (`bun scripts/sign-fixture.ts` from `server/`). The `push` fixture is `push.push.json` because push has no action.

## Confirmed facts

| Fact | Result | Source (checked 2026-11-23) |
|---|---|---|
| `X-Hub-Signature-256: sha256=<hex HMAC-SHA256 of the raw body>`, compare in constant time | Confirmed. Docs say never use `==`; use e.g. `crypto.timingSafeEqual`. Payload is handled as UTF-8. Header absent if no secret is set. Docs give a test vector. | https://docs.github.com/en/webhooks/using-webhooks/validating-webhook-deliveries |
| `X-GitHub-Event`, `X-GitHub-Delivery` | Both exist; Delivery is "a globally unique identifier (GUID)". **Docs do not say whether a redelivery reuses the GUID.** Dedupe on it anyway (a repeat is then skipped; if redeliveries get fresh GUIDs, `push_receipts` would double-count a sha, so Task 6 should also tolerate a repeated `(repo, sha)`). | https://docs.github.com/en/webhooks/webhook-events-and-payloads |
| `push` payload: `ref`, `before`, `after`, `forced`, `deleted`, `repository`, `installation`, `commits[]`, `head_commit` | Confirmed for `ref`, `before`, `after`, `forced`, `deleted`, `created`, `commits`, `head_commit` (object or null), `base_ref`, `compare`, `pusher`. `repository.id`/`full_name` appear in the example payload; `installation` is documented as a common field for Apps (its `id` is not spelled out). Fields inside `commits[]` are not listed on the page. **Corrected:** the docs cap `commits` at 2048 (not 20); use the Commits API for more. No push event for more than 5000 branches pushed at once; tag events skipped when more than three tags are pushed together. | same page, push section |
| `installation` (`created`, `deleted`, `suspend`, `unsuspend`; also `new_permissions_accepted`), `installation_repositories` (`added`, `removed`) | Confirmed. Apps always receive both and cannot unsubscribe. | same page |
| `repository` `renamed`, `transferred` | Confirmed, each with a `changes` object. **`transferred` is only sent to the account that receives the repo**, which needs the App installed and subscribed to Repository. The old owner's installation does not hear it. | same page |
| `github_app_authorization` `revoked` | Event confirmed ("someone revoked their authorization"; always delivered, cannot unsubscribe; revoking does not uninstall). **The action value `revoked` is not on the page**; treat any action of this event as a revocation. Revoked tokens give 401. | same page |
| User tokens: authorize at `https://github.com/login/oauth/authorize`, exchange by `POST https://github.com/login/oauth/access_token` | Confirmed. `client_id` required; `state`, `redirect_uri`, PKCE recommended. Exchange needs `client_id`, `client_secret`, `code`. | https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/generating-a-user-access-token-for-a-github-app |
| Expiry 8 h with refresh tokens | Confirmed with a caveat: by default the token expires after 8 h (`expires_in` 28800) and a `ghr_` refresh token (6 months) comes with it; the expiring-token behaviour is described as optional. We do not store the refresh token. Read `expires_in`; if absent (non-expiring token), cap our stored `expires_at` at 8 h anyway. | same page |
| `GET /user/installations`, `GET /user/installations/{id}/repositories` list only what the user can access | Confirmed: installations the user can access with explicit permission, and repos in one installation the user has explicit permission on. Needs a user access token (implied by the titles, not spelled out). | https://docs.github.com/en/rest/apps/installations |
| Setup URL gets `installation_id`, `setup_action` | Only `installation_id` is documented; `setup_action` is not mentioned. Docs warn bad actors can hit the URL with a spoofed `installation_id`, and say to verify via a user access token. Matches the plan: ignore the query. "Redirect on update" also sends users to the setup URL after installation changes. | https://docs.github.com/en/apps/creating-github-apps/registering-a-github-app/about-the-setup-url |

## The three questions

| Question | Result |
|---|---|
| Which permission makes GitHub deliver `push` | Could not verify; fallback is to assume Contents: read-only plus Metadata: read-only (always on) delivers it. |
| Does `repository` `renamed` reach an App that subscribes only to `push` | Could not verify; fallback is to assume `repository` events reach an App only if subscribed, so subscribe to both Push and Repository. |
| Does `transferred` keep `repository.id` | Could not verify with a real delivery; fallback is to assume it does, since GitHub describes the numeric id as stable. Key by id, and update `full_name` on the event. Note the new owner's installation id differs, so Task 6 should set `github_repos.installation_id` from the payload's `installation.id`. |

## Production App settings

- Webhook URL `https://<host>/github/webhook`, a random secret (`GITHUB_WEBHOOK_SECRET`), SSL verification on.
- Setup URL `https://<host>/github/setup`; Callback URL `https://<host>/github/callback`; **leave "Request user authorization (OAuth) during installation" unchecked**. Leave "Expire user authorization tokens" on.
  - Documented (https://docs.github.com/en/apps/creating-github-apps/registering-a-github-app/registering-a-github-app, checked 2026-11-23): with that box checked you cannot enter a Setup URL, and users are sent to the Callback URL as part of the authorization flow. Without it, users are redirected to the Setup URL after install (about-the-setup-url page).
  - Not documented: whether the install-time authorization carries a `state`, and the exact redirect order. Assumed: it does not carry one of ours, so Tally never exchanges a stateless `code`.
  - Flow: install, then Setup URL, then `/github/connect` (stateful, browser-bound), then `/github/callback`. As a safety net, `/github/callback` with a `code` and no `state` or binding cookie discards the code and redirects to `/github/connect`.
- Permissions: Contents read-only, Metadata read-only. Events: Push, Repository.
- No private key is needed or stored: Tally uses user-to-server tokens only.
- Env: `GITHUB_APP_SLUG`, `GITHUB_APP_CLIENT_ID`, `GITHUB_APP_CLIENT_SECRET`, `GITHUB_WEBHOOK_SECRET`.

## Manual steps for the maintainer

Skipped here because they need a logged-in GitHub account.

1. Create a development GitHub App "Tally (dev)" with the settings above, webhook URL a tunnel to `localhost:8080/github/webhook`, Setup URL `http://localhost:8080/github/setup`, Callback URL `http://localhost:8080/github/callback`, "Request user authorization during installation" unchecked.
2. Install it on a scratch repo. Push twice, force-push once, rename the repo, transfer it to a second account, remove it from the installation, uninstall. From the App's "Advanced" tab compare each real delivery with the hand-built fixtures (field names, `installation` presence, redelivery GUID, whether `transferred` keeps `repository.id`, the `github_app_authorization` action value) and correct the fixtures if they differ. Replace emails and names before re-signing with `test-secret`.
3. Answer the three questions above from the real deliveries and update this document.
4. (Part B step 8) Install the dev App through `/github/install` against a local server and link the scratch repo from the project settings page.
