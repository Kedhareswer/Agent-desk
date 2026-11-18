# Changelog

All notable changes to Agent Desk are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project
uses [Semantic Versioning](https://semver.org/).

Write user-facing changes under **Unreleased** as you make them.
`npm run release:version -- <version>` moves them under the new version and
the release workflow uses that section as the GitHub Release notes.

## [Unreleased]

## [0.1.2] - 2026-11-19

### Added

- Automatic updates for copies installed with the installer: Agent Desk
  checks GitHub Releases in the background, downloads and verifies a new
  version, and the tray offers "Restart to update". Turn it off in
  Settings. Scoop, winget, Chocolatey and portable copies keep updating
  through their own tool, and the tray says how.

## [0.1.1] - 2026-11-19

### Fixed

- The notch could start broken: typing, Settings and window sizing did
  nothing until the app was restarted. The interface now waits for the app
  to finish starting.
- The first time you talk, the notch now shows "Downloading speech model
  N%" instead of sitting on "Transcribing" for minutes.
- "Show desktop" (Win + D) no longer hides the notch for good or moves it
  to the corner of the screen.

## [0.1.0] - 2026-11-19

### Added

- Push to talk with Ctrl + Alt, local speech to text with whisper.cpp.
- Morphing notch on any screen edge, with an edge glow while listening.
- Answers through the Claude Code or Codex CLI.
- "Work with me" live mode.
- Insert the last answer into the app you came from.
- CI on every push and pull request: formatting, Clippy, Rust unit tests,
  UI tests with screenshots, and a Windows installer build.
- Tagged releases: installer and portable zip on GitHub Releases, with
  checksums.
- Install with Scoop, winget or Chocolatey.
- Weekly dependency security audit and Dependabot updates.


[Unreleased]: https://github.com/Kedhareswer/Agent-desk/compare/v0.1.2...HEAD
[0.1.2]: https://github.com/Kedhareswer/Agent-desk/releases/tag/v0.1.2
[0.1.1]: https://github.com/Kedhareswer/Agent-desk/releases/tag/v0.1.1
[0.1.0]: https://github.com/Kedhareswer/Agent-desk/releases/tag/v0.1.0
