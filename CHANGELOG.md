# Changelog

All notable changes to Agent Desk are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project
uses [Semantic Versioning](https://semver.org/).

Write user-facing changes under **Unreleased** as you make them.
`npm run release:version -- <version>` moves them under the new version and
the release workflow uses that section as the GitHub Release notes.

## [Unreleased]

### Fixed

- Agent Desk closed itself right after "Transcribing" on many computers,
  including Intel 12th to 14th generation laptops. The speech engine had been
  built for processors with AVX-512; it is now built for AVX2, which all
  processors from about 2013 on have. On older processors talking shows a
  message instead of crashing, and typing still works.

## [0.1.3] - 2026-11-19

### Security

- Your data cannot leave unasked: once a file, folder or the clipboard has
  been read in a conversation, every link asks before it opens, and so does
  any link with a long query or fragment. Attaching a file always asks
  (Teams uploads it straight away), and clicking Upload asks.
- Codex can no longer run commands: its shell is switched off for every run,
  so it cannot read files around Agent Desk's checks.
- Opening files: only documents, pictures and media open without asking;
  programs, scripts, shortcuts, macro documents, disk images and the like
  ask. Names ending in a dot or space are refused, network paths
  (`\server\share`, WebDAV) are refused before they are touched, and the
  off-limits check runs on the real, canonical path (short names such as
  `APPDAT~1` no longer slip past).
- "Always allow" is narrower: per key combination, per file type, per
  button; never offered for schedules, programs, attachments or links.
- A lone carriage return now counts as Enter, other control characters
  cannot be typed, and copying text with a line break asks.
- More buttons ask first: Allow, Authorize, Grant, Agree, Yes, OK,
  Continue, Upload, Commit, Join, Call, Update; and a click on an unnamed
  spot of a screenshot asks.
- Agent Desk refuses to click its own window, and nothing runs while a step
  is waiting for your answer.
- Scheduled tasks ask before every step, since nobody asked just then.
- The local action server checks the key before reading a request, caps
  request size and open connections, and its key comes from Windows'
  system random generator. The file holding it is deleted on Quit.
- A strict content security policy for the notch; the glow window can no
  longer call any of the app's commands.

### Changed

- The notch shows only "Listening" while you talk; the dot meter is gone.
- A finished answer goes away by itself after 5 seconds. Keep the mouse
  over it to keep it open.
- A follow-up question replaces the previous answer instead of piling up
  under it. The conversation itself is still remembered.
- Settings and close are now at the right edge of the notch.
- Faster speech to text: silence before and after you speak is skipped,
  Whisper only processes as much audio as you said, and its working memory
  is reused between questions.
- Smoother on slower computers: the indicator's glow and the window
  resizing while an answer streams in cost much less.

### Added

- Agent Desk can now act on your computer: open web pages and apps, switch
  windows, type, press shortcuts, run commands you saved, and look at the
  screen again. Say "open Chrome and go to GitHub" and it does it. Every
  step shows in the notch, and anything that sends, deletes, buys or runs
  a command waits for your OK, with an "Always allow" option per type.
- Schedules: "every weekday at 9, open my calendar and summarise today" or
  "remind me at 5 to send the report". Manage them in Settings → Schedules.
- Settings → Actions: turn actions off, undo "Always allow", and save
  commands the AI may run by name.
- Full desktop control: Agent Desk can see the buttons, links and text
  boxes in any app (including web pages in your own browser), move the
  mouse pointer visibly and click, fill in text, scroll, use the clipboard,
  open files and folders, read text, Word and PDF documents, and attach a
  file in Teams, Slack or Outlook in one step. Clicking Send, Delete, Buy
  and the like waits for your OK, button by button.
- Browser profiles: it asks once which Chrome, Edge or Brave profile to use
  and remembers it (reset it in Settings).
- A setup screen on first launch (and from the tray, "Setup and guide"):
  checks that Claude Code or Codex is installed and signed in, shows the
  account and plan, copies the sign-in command, lets you pin one account
  folder, picks the model from the CLI's own list, shows which speech
  models are downloaded with live download progress, and explains how to
  use everything.

### Changed

- Faster actions: clicks, typing and attachments hand back what is on
  screen straight away, so the AI needs fewer round trips.
- Lighter when hidden: the notch and glow web views are asked to release
  memory while they are not shown.
- Claude and Codex now get Agent Desk's own actions as tools (through a
  local MCP server). They still get none of their own tools, and for Codex
  your apps, plugins and MCP servers are switched off during these runs.

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


[Unreleased]: https://github.com/Kedhareswer/Agent-desk/compare/v0.1.3...HEAD
[0.1.3]: https://github.com/Kedhareswer/Agent-desk/releases/tag/v0.1.3
[0.1.2]: https://github.com/Kedhareswer/Agent-desk/releases/tag/v0.1.2
[0.1.1]: https://github.com/Kedhareswer/Agent-desk/releases/tag/v0.1.1
[0.1.0]: https://github.com/Kedhareswer/Agent-desk/releases/tag/v0.1.0
