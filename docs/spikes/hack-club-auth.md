# Spike: Hack Club Auth flow

Checked 2026-11-23, docs and discovery document only. No app was registered and no live login, code exchange or token decode was done (needs the maintainer's account). `id_token_claims.json` is built from the docs, not a real token. `discovery.json` is the live document fetched with curl.

## Confirmed facts

| Fact | Result | Source (checked 2026-11-23) |
|---|---|---|
| Issuer `https://auth.hackclub.com`; discovery at `/.well-known/openid-configuration` | Confirmed | https://auth.hackclub.com/docs/oidc-guide, live discovery |
| authorize `/oauth/authorize`, token `/oauth/token`, userinfo `/oauth/userinfo`, jwks `/oauth/discovery/keys`, revocation `/oauth/revoke` | Confirmed. Also present: `introspection_endpoint` `/oauth/introspect` (not in the plan) | discovery.json |
| `response_types_supported ["code"]`; grants `authorization_code`, `client_credentials`, `refresh_token` (no device grant); auth methods `client_secret_basic`, `client_secret_post` | Confirmed. Also `response_modes_supported` query, fragment, form_post; `subject_types_supported` public | discovery.json |
| ID token is an RS256 JWT; `sub` like `ident!abc123` | Confirmed | oidc-guide |
| Community apps may request `openid profile email name slack_id verification_status`; others need HQ | Confirmed (HQ-only: phone, birthdate, basic_info, legal_name, address) | oidc-guide, https://auth.hackclub.com/docs/oauth-guide, https://auth.hackclub.com/docs/api |
| `verification_status` scope gives `verification_status` and `ysws_eligible` claims; values `needs_submission`, `pending`, `verified`, `ineligible` | Confirmed. The oidc-guide shows only `verified`; the full value list is on the api page | oidc-guide, api |
| Apps registered at `/developer/apps`; new apps are `community_untrusted` with a scarier consent screen | Confirmed. Promotion to `community_trusted` is by asking Nora; `hq_official` is for HQ programs. The warning text is not described | oauth-guide |
| `GET /api/v1/me` returns `{identity, scopes}`; no GitHub username | Confirmed. `identity` has id, first_name, last_name, primary_email, slack_id, verification_status, ysws_eligible and more | api |

Discovery notes: `scopes_supported` lists only openid, profile, phone, birthdate (not email, slack_id or verification_status, though the docs say they work). `claims_supported` includes iss, sub, aud, exp, iat, email, email_verified, name, given_name, family_name, nickname, updated_at, slack_id, verification_status, ysws_eligible, groups. `code_challenge_methods_supported` is absent.

## Answers to the unknowns

| Unknown | Docs say | Decision |
|---|---|---|
| PKCE | Not mentioned anywhere; absent from discovery | Could not verify; fallback is to send `code_challenge` (S256) and `code_verifier` anyway (harmless if ignored), plus the client secret. |
| `nonce` | Not mentioned; example ID token has no `nonce` | Could not verify; fallback is to send `nonce` but require it only if the ID token echoes it. |
| `state` | Not mentioned (sample URLs omit it) | Could not verify; fallback is to send `state` and verify it ourselves. OAuth 2.0 expects it returned unchanged. |
| `id_token` in token response | Yes. The oidc-guide shows `access_token`, `token_type`, `id_token` | Documented but not tested live. Use the `id_token`; fall back to `/oauth/userinfo` if it is absent. |
| Token lifetimes | oauth-guide: access token `expires_in` 15778800 s (about 6 months); refresh tokens rotate. The example ID token has `exp - iat` of 3600 s, but that is only an example | ID token lifetime could not verify; fallback is to read `exp` from the token and never assume a value. Treat the access token as long-lived. |
| Logout | No `end_session_endpoint` in discovery; docs silent. `/oauth/revoke` exists | Confirmed none. Logout is local only (Task 2): clear our session. Optionally call `/oauth/revoke`. |

## Fixtures

- `server/test/fixtures/hca/discovery.json`: live document, fetched 2026-11-23.
- `server/test/fixtures/hca/id_token_claims.json`: claim names from the docs with fake values. Not from a real token.

## Manual steps for the maintainer

Skipped here because they need the maintainer's Hack Club account and a browser.

1. Register an app at https://auth.hackclub.com/developer/apps with redirect URI `http://localhost:8080/auth/callback` and scopes `openid profile email verification_status`. Record its trust level (expected `community_untrusted`). Keep the secret out of the repo.
2. In a browser, open `/oauth/authorize?...&state=s1&nonce=n1&code_challenge=<S256>&code_challenge_method=S256`. Record whether the consent screen appears and whether `state=s1` returns.
3. Exchange the code with `curl -u client_id:secret -d grant_type=authorization_code -d code=... -d redirect_uri=... -d code_verifier=...`, then repeat with a wrong verifier. Record whether PKCE is enforced, whether `id_token` is present, and `expires_in`.
4. Decode the ID token payload. Check whether `nonce` equals `n1` and what `exp - iat` is. Replace `id_token_claims.json` with the real claim names (values faked) and update the Decision rows above.
