# Changelog

All notable changes to Agent Desk are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project
uses [Semantic Versioning](https://semver.org/).

Write user-facing changes under **Unreleased** as you make them.
`npm run release:version -- <version>` moves them under the new version and
the release workflow uses that section as the GitHub Release notes.

## [Unreleased]

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


[Unreleased]: https://github.com/Kedhareswer/Agent-desk/commits/HEAD
