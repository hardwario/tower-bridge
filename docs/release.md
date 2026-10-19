# Releasing

## Steps

1. Set the new version in `package.json`, `Cargo.toml` (the workspace: core,
   service and CLI) and `src-tauri/Cargo.toml`, and give the newest
   `CHANGELOG.md` heading that version without "(unreleased)".
   `node scripts/check-version.mjs v1.2.3` checks that they agree.
2. Commit, then tag and push: `git tag v1.2.3 && git push origin v1.2.3`.
3. The Release workflow (`.github/workflows/release.yml`) tests, builds every
   platform, signs the Windows binaries, signs and notarizes the macOS ones,
   and creates a **draft** release with all files and their `.sha256`
   checksums.
4. Check the draft on GitHub and publish it.

A dry run builds everything without publishing: Actions, Release, Run
workflow. Started on a branch it skips the signing; started on a release tag
it signs too, but still publishes nothing.

## Files

| File | Built by |
|---|---|
| `tower-bridge-<version>-windows-x64-portable.exe` | `build` (windows), signed by `sign` |
| `tower-bridge-<version>-macos-universal.dmg` | `build` (macos, the app), signed, notarized and packed by `sign-macos` |
| `tower-bridge-<version>-linux-x86_64.AppImage`, `70-hardwario-tower-bridge.rules` | `build` (linux) |
| `tower-bridge-cli-<version>-x86_64-pc-windows-msvc.zip` | `cli`, the exe inside signed by `sign` |
| `tower-bridge-cli-<version>-universal-apple-darwin.tar.gz` | `cli`, the binary inside signed and notarized by `sign-macos` |
| `tower-bridge-cli-<version>-<target>.tar.gz` (Linux x86_64 and aarch64) | `cli` |

## Windows signing

The `sign` job signs both Windows binaries with Authenticode, using Microsoft
Azure Artifact Signing (formerly Trusted Signing), account `hardwario`,
certificate profile `hardwario` (Public Trust, "CN=HARDWARIO a.s."), endpoint
`https://weu.codesigning.azure.net/`:

1. `tower-bridge.exe` (the portable app, released as
   `tower-bridge-<version>-windows-x64-portable.exe`);
2. `tower-bridge-cli.exe`, before it is zipped.

Both link the runtime statically and there is no installer, so there are no
other files of ours to sign. Signatures use SHA-256 and an RFC 3161 timestamp
from `http://timestamp.acs.microsoft.com`. Microsoft's certificates are valid
for about three days, so only the timestamp keeps a signature valid after
that. A verification step fails the job unless both files have a valid
signature by "HARDWARIO a.s." with a timestamp, and the release takes the
Windows files from the `sign` job only.

When: for a pushed release tag (`v*`) and for a dry run started on one.
Nothing else reaches the job.

What it depends on:

- The GitHub environment **`signing`**: deployment limited to the tags `v*`
  (optionally with a required reviewer), with the variables `AZURE_CLIENT_ID`,
  `AZURE_TENANT_ID` and `AZURE_SUBSCRIPTION_ID` (not secrets).
- On the Azure side: the Entra app `github-codesign`, whose client ID is
  `AZURE_CLIENT_ID`, with the role "Artifact Signing Certificate Profile
  Signer" on the `hardwario` profile only, and a federated credential that
  trusts GitHub's OIDC tokens for the subject
  `repo:hardwario@28539912/tower-bridge@1425884115:environment:signing`.
  The repository uses GitHub's immutable subject (with the owner and
  repository IDs), so a re-created repository of the same name is not
  trusted.
  The job logs in with `azure/login` through OIDC (`id-token: write`); there
  is no client secret.

## macOS signing and notarization

The `sign-macos` job signs with the team's Developer ID Application
certificate (team C32QFQ68V9) and notarizes with an App Store Connect API key,
in this order:

1. `TOWER Bridge.app`: signed with the hardened runtime and a secure
   timestamp, notarized, and the ticket stapled to it, so it also opens
   offline;
2. the dmg built from it (the app and a link to Applications): signed,
   notarized and stapled;
3. `tower-bridge-cli`: signed with the hardened runtime and notarized. A bare
   binary cannot carry a stapled ticket; macOS checks it online.

A verification step fails the job unless Gatekeeper (`spctl`) accepts the app
and the dmg, both tickets are stapled, and the CLI is signed with Developer ID,
a timestamp and the hardened runtime. The app needs no entitlements: it is
not sandboxed, and the USB HID access and WebKit need none outside the
sandbox. The build job never sees the certificate: it makes an ad-hoc signed
app, and only `sign-macos` imports the certificate, into a temporary keychain
it deletes at the end.

When: as for Windows, for release tags only, in the same `signing`
environment, which holds:

- secrets `APPLE_CERTIFICATE` (the `.p12` with certificate and private key,
  base64), `APPLE_CERTIFICATE_PASSWORD` and `APPLE_API_KEY_P8` (the API key's
  `.p8` file);
- variables `APPLE_API_KEY` (the key ID) and `APPLE_API_ISSUER` (the issuer
  ID).

Only the Account Holder can create Developer ID certificates, in
developer.apple.com (the API refuses it); the API key works for notarization.
