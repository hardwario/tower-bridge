# Releasing

## Steps

1. Set the new version in `package.json`, `Cargo.toml` (the workspace: core,
   service and CLI) and `src-tauri/Cargo.toml`, and give the newest
   `CHANGELOG.md` heading that version without "(unreleased)".
   `node scripts/check-version.mjs v1.2.3` checks that they agree.
2. Commit, then tag and push: `git tag v1.2.3 && git push origin v1.2.3`.
3. The Release workflow (`.github/workflows/release.yml`) tests, builds every
   platform, signs the Windows binaries and creates a **draft** release with
   all files and their `.sha256` checksums.
4. Check the draft on GitHub and publish it.

A dry run builds everything without publishing: Actions, Release, Run
workflow. Started on a branch it skips the signing; started on a release tag
it signs too, but still publishes nothing.

## Files

| File | Built by |
|---|---|
| `tower-bridge-<version>-windows-x64-portable.exe` | `build` (windows), signed by `sign` |
| `tower-bridge-<version>-macos-universal.dmg` | `build` (macos) |
| `tower-bridge-<version>-linux-x86_64.AppImage`, `70-hardwario-tower-bridge.rules` | `build` (linux) |
| `tower-bridge-cli-<version>-x86_64-pc-windows-msvc.zip` | `cli`, the exe inside signed by `sign` |
| `tower-bridge-cli-<version>-<target>.tar.gz` (Linux x86_64 and aarch64, macOS universal) | `cli` |

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
  trusts GitHub's OIDC tokens for
  `repo:hardwario/tower-bridge:environment:signing`.
  The job logs in with `azure/login` through OIDC (`id-token: write`); there
  is no client secret.

## macOS

The dmg is signed and notarized when the repository has the Apple secrets
(`APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`,
`APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID`); without them it is ad-hoc
signed and users must allow it once in System Settings.
