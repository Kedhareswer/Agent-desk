# Changelog

All notable changes to Agent Desk are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project
uses [Semantic Versioning](https://semver.org/).

Write user-facing changes under **Unreleased** as you make them.
`npm run release:version -- <version>` moves them under the new version and
the release workflow uses that section as the GitHub Release notes.

## [Unreleased]

### Fixed

- Ctrl + Alt sometimes did nothing while Ctrl + Alt + Space worked. Key
  presses and releases were handled on separate threads, so on a busy
  computer a release could be handled before its press and leave a
  recording running that nothing ended. They are now handled strictly in
  order. Windows can also silently drop a slow keyboard hook (for example
  while speech is being transcribed on every core); the hook now runs at
  high priority, is put back every 15 seconds, and speech leaves a core
  free. A key release the hook never saw no longer leaves the gesture
  stuck, and the right Alt key works too (AltGr still does not wake it).
- A slightly slow tap with nothing said now opens the notch for typing
  instead of saying "I didn't hear anything".
- `scoop update agent-desk` refused to run while Agent Desk was open. The
  tray now has **Update with Scoop**, which closes Agent Desk, lets Scoop
  update it and opens the new version.

### Changed

- New README recordings of the current version, including Agent Desk
  working on the computer (`docs/media/act.gif`), at full size and 15
  frames a second.

## [0.1.6] - 2026-11-19

### Added

- The notch no longer gets in the way of the mouse. Clicks, scrolling and
  dragging go straight through it to the app underneath, as if it were not
  there; only its buttons take the mouse (and the whole notch while you
  type or have Settings or the history open). Hold Ctrl over a long answer
  to scroll or select it.
- Read aloud: the speaker button (or Ctrl + R) reads the answer with
  Windows' own voices, on your computer.
- Pin: the pin button (or Ctrl + P) keeps an answer open until you unpin it
  or ask something new.
- "Try again" on an error asks the same question again.
- The history can be searched, and each earlier answer has "Ask again" and
  "Copy".

### Fixed

- One side of the notch was cut off whenever the question in its top line
  was long. The shape could grow wider than its window; it now always fits
  and long questions end in "...".
- Scrolling could land on the notch instead of the page when the answer
  covered the middle of the window. It now aims past the notch.

## [0.1.5] - 2026-11-19

### Added

- Earlier answers. The clock button in the notch (or Ctrl + H) lists the
  answers you got before, newest first, so a question replaced by a new one
  or an answer that tucked away can be read again. They are kept on this
  computer only; "Clear" removes them.

### Changed

- Steps on your computer now share one line that changes in place ("Open
  github.com in Chrome", then "Scroll down", ...) instead of a growing list
  of finished steps. The line goes once the answer is done, unless a step
  failed or you said no.
- A finished answer stays on screen for at least 10 seconds, longer for
  long answers (up to 40 s), before the notch tucks away. Moving the mouse
  over it still keeps it open.
- Agent Desk's pointer now glides visibly to where it works and stays on
  screen while Agent Desk is busy, instead of disappearing after each click.

### Fixed

- Scrolling did nothing unless Agent Desk had clicked first: it scrolled
  wherever your own mouse happened to be. It now scrolls the window in
  front (its pointer shows where) and tells the AI what came into view.
- The notch could stay on "Working on it", with the edge glowing, after a
  step that ran outside an answer. Such a step now ends the working state
  itself, and an answer that shows no sign of life for 3 minutes is stopped
  with a message, so the notch is never left working forever.

## [0.1.4] - 2026-11-19

### Added

- Crash recovery. If Agent Desk closes unexpectedly, Windows starts it again,
  and the notch offers to continue the request it was working on
  ("Continue" or "Not now"). It asks instead of redoing it by itself, and the
  AI is told to check the screen first so nothing is done twice. If only the
  AI process stops part-way, the request is sent again once, automatically.
- Agent Desk no longer moves your mouse around. When it acts, its own
  pointer, labelled "Agent Desk", glides to the spot, and buttons, links,
  check boxes and text boxes are used directly through Windows'
  accessibility interface. Where a real click is unavoidable, your cursor
  is borrowed for a split second and put back where you left it.
- The screen edge glows in Agent Desk's working colour while it acts on
  your computer, so you can tell it is busy even when the notch is small.
- `crash.log` in the settings folder records what failed and where (never
  what you said), so crashes can be diagnosed.

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


[Unreleased]: https://github.com/Kedhareswer/Agent-desk/compare/v0.1.6...HEAD
[0.1.6]: https://github.com/Kedhareswer/Agent-desk/releases/tag/v0.1.6
[0.1.5]: https://github.com/Kedhareswer/Agent-desk/releases/tag/v0.1.5
[0.1.4]: https://github.com/Kedhareswer/Agent-desk/releases/tag/v0.1.4
[0.1.3]: https://github.com/Kedhareswer/Agent-desk/releases/tag/v0.1.3
[0.1.2]: https://github.com/Kedhareswer/Agent-desk/releases/tag/v0.1.2
[0.1.1]: https://github.com/Kedhareswer/Agent-desk/releases/tag/v0.1.1
[0.1.0]: https://github.com/Kedhareswer/Agent-desk/releases/tag/v0.1.0
