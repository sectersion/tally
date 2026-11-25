# Releasing Tally

How a release is built, what the maintainer must provide, and what has and has not been verified. Anything about prices or vendor procedure is marked **maintainer to verify**: it was not checked against the vendor.

## Versioning

`0.y.z` SemVer: **minor** (`y`) = breaking change (ledger/heartbeat protocol is the compat surface that matters), **patch** (`z`) = a release. Pin `x` at 0 until 1.0.0, which is set by hand. Nightly builds are `0.y.z-nightly.YYYYMMDD` (prerelease, sorts below the release) — no version bump per commit.

## How to cut a release

1. `sh scripts/release-bump.sh patch` (or `minor` for a breaking change) — bumps all crates, commits, tags.
2. Push: `git push origin main "v0.y.z"`.
3. `.github/workflows/release.yml` builds six archives (`x86_64`/`aarch64` for Windows MSVC, macOS, and Linux musl), writes `SHA256SUMS`, runs `scripts/check-release-artifacts.sh`, and creates a **draft** GitHub release.
4. Read the draft: the checksums, and whether it starts with `UNSIGNED BUILD`. Publish it yourself.

`workflow_dispatch` is a dry run: the same jobs, artifacts attached to the run, nothing released.

Binaries are built with the `dist` Cargo profile (thin LTO, one codegen unit, stripped, `panic = "abort"`). It is separate from `release` on purpose: wasm-pack builds `tally-scoring` for the server with `release`, and a stripped abort-on-panic wasm would change that build.

## 1. Accounts and cost

- **Windows**: a code-signing certificate from a certificate authority, or a cloud signing service. *Maintainer to verify* current options and price; recent CAs require the key on hardware or in a cloud HSM, which a PFX in a GitHub secret does not satisfy.
- **macOS**: an Apple Developer Program membership (paid, annual) for a "Developer ID Application" certificate and notarization. *Maintainer to verify* the price.
- PITCH budgets about $20 per month for both. *Maintainer to verify.*

## 2. GitHub Actions secrets the workflow reads

Windows (both needed, or the Windows archives are built unsigned):

| Secret | Content |
|---|---|
| `WINDOWS_CERT_PFX_BASE64` | the `.pfx` file, base64 |
| `WINDOWS_CERT_PASSWORD` | its password |

macOS (all six needed, or the macOS archives are built unsigned):

| Secret | Content |
|---|---|
| `APPLE_CERT_P12_BASE64` | "Developer ID Application" certificate with its private key, exported as `.p12`, base64 |
| `APPLE_CERT_PASSWORD` | the `.p12` password |
| `APPLE_SIGNING_IDENTITY` | the identity string from `security find-identity -v -p codesigning` |
| `APPLE_ID` | the Apple ID used for `notarytool` |
| `APPLE_TEAM_ID` | the team id |
| `APPLE_APP_SPECIFIC_PASSWORD` | an app-specific password for `notarytool` |

If you choose a cloud signing service for Windows instead of a PFX, the "Sign (Windows Authenticode)" step in `release.yml` changes; record the choice here.

The timestamp server in the workflow (`http://timestamp.digicert.com`) is a public service: *maintainer to verify* it is the one you want.

## 3. What unsigned means for users

- Windows: SmartScreen shows "unrecognized app" and the user must choose "Run anyway".
- macOS: Gatekeeper blocks a downloaded unsigned binary opened from Finder.
- The `curl | sh` installer (`install/install.sh`) does not set the quarantine flag, so macOS and Linux installs work either way.

Without the secrets the workflow still runs: archives are named `tally-<version>-<target>-unsigned.<ext>` and the release body starts with `UNSIGNED BUILD`. A signed archive name (no `-unsigned`) is accepted only if the job that ran the signing step left a `<archive>.SIGNED` marker next to it, so an unsigned build cannot be presented as signed.

## Verification status

| Item | Status |
|---|---|
| `x86_64-unknown-linux-musl` release build of `tally-daemon` (all deps, including `x11rb`, `zbus`, `wayland-client`, bundled SQLite) | OBSERVED 2026-11-24 in a `rust:1` container with `musl-tools`: builds, 13.9 MB, `tally --version` prints `tally 0.1.0`. |
| `aarch64-unknown-linux-musl` via `cargo-zigbuild` | NOT VERIFIED: not built. |
| `aarch64-pc-windows-msvc`, both macOS targets | NOT VERIFIED: need the CI runners. |
| `release.yml` | YAML parses (six targets, three jobs). **Never run**: a dry run needs the branch pushed. `scripts/test-check-release-artifacts.sh` passes locally. |
| Signed paths (Windows and macOS) | **Cannot be tested until the maintainer provides secrets.** |

The musl build prints one harmless warning, `dropping unsupported crate type cdylib` (from `tally-scoring`, whose `cdylib` is for wasm).
