# Package managers

Every tagged release (see "Releasing" in [CONTRIBUTING.md](../CONTRIBUTING.md))
runs `scripts/packaging.mjs`, which writes manifests for Scoop, winget and
Chocolatey with the release's SHA-256 hashes. They are attached to the
workflow run as the `release` artifact under `packaging/`. Names and
descriptions live in the `ID` block at the top of that script.

To generate them by hand from two release files in a folder:

```bash
node scripts/packaging.mjs 0.2.0 dist
```

## Scoop (automatic)

The release workflow commits `bucket/agent-desk.json` to the default
branch, so this repository is its own Scoop bucket. Users install with:

```powershell
scoop bucket add agent-desk https://github.com/Kedhareswer/Agent-desk
scoop install agent-desk
```

Scoop uses the portable zip (just `agent-desk.exe`). Settings and the speech
model live in the user's app data folder, so updates keep them.

## winget (one-time setup)

1. Run a release. Download the `release` artifact from the run and take
   `packaging/winget/` from it.
2. Submit those three files by hand once, as a pull request to
   [microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs) under
   `manifests/k/Kedhareswer/AgentDesk/<version>/`
   (or use `wingetcreate submit packaging/winget`).
3. After it is merged, create a classic personal access token with the
   `public_repo` scope, fork `microsoft/winget-pkgs` to that account, and add:
   - repository secret `WINGET_TOKEN` = the token
   - repository variable `PUBLISH_WINGET` = `true`

From then on each release opens the winget pull request itself. Users
install with `winget install Kedhareswer.AgentDesk`.

## Chocolatey (one-time setup)

1. Create an account on [chocolatey.org](https://community.chocolatey.org/)
   and copy your API key.
2. Add repository secret `CHOCOLATEY_API_KEY` and repository variable
   `PUBLISH_CHOCOLATEY` = `true`.

Each release then packs and pushes `agent-desk`. New packages go through
Chocolatey's moderation, which can take a few days. Users install with
`choco install agent-desk`.

## Notes

- The installer is per-user (no admin prompt) and is not code signed, so
  SmartScreen asks users to confirm. Code signing would remove that and is
  needed for a smooth winget and Chocolatey review.
- Pre-release tags (`v1.2.3-beta.1`) only publish to GitHub Releases.
