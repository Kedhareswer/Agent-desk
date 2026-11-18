# Contributing

Thanks for helping. Read [CLAUDE.md](CLAUDE.md) first: it explains how the
app is built and the design and privacy rules it follows.

## Set up

You need Rust (stable), Node 22, CMake and LLVM/Clang (whisper.cpp is
compiled from source). On Windows, WebView2 is already installed on
Windows 11.

```bash
npm ci
npx playwright install chromium
npm run dev
```

## Before you open a pull request

```bash
npm run format
npm run lint
npm test
```

- `npm test` runs the Rust unit tests and the UI test.
- `node tests/ui.test.mjs shots` also saves screenshots to `shots/`. Look at
  them: design regressions are the most common failure.
- Add a line under **Unreleased** in [CHANGELOG.md](CHANGELOG.md) for
  anything a user would notice.

CI runs the same checks plus a Windows installer build on every push and
pull request. The installer is attached to the run as an artifact.

## Releasing

1. Make sure CI is green on the default branch.
2. Set the version everywhere and date the changelog:

   ```bash
   npm run release:version -- 0.2.0
   ```

3. Review `CHANGELOG.md`, then commit, tag and push:

   ```bash
   git commit -am "Release v0.2.0"
   git tag v0.2.0
   git push && git push origin v0.2.0
   ```

The **Release** workflow then builds the installer and a portable zip,
publishes a GitHub Release with checksums, updates the Scoop bucket in
this repo, and (when turned on) submits to winget and Chocolatey. A tag
with a suffix such as `v0.2.0-beta.1` becomes a pre-release and skips the
package managers. See [packaging/README.md](packaging/README.md) for the
one-time package manager setup.
