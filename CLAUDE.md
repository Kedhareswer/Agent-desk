# CLAUDE.md

Guidance for AI agents (and people) working in this repository. Read this
before changing anything. `README.md` is the user-facing description; this
file is about how the code is built and the rules it follows.

## What this is

Agent Desk is a voice-first AI assistant for **Windows** that lives in the
background as a tray app. The user holds **Ctrl + Alt**, speaks, and lets
go; a black **notch** on the edge of the screen shows what it is doing and
grows into the answer. It answers through the user's **existing Claude or
ChatGPT subscription** by driving the `claude` (Claude Code) or `codex`
(OpenAI Codex) command-line tool they are already signed in to. There is no
API key and no server of our own.

Core promises, in priority order. Do not trade one away without the user
asking:

1. **Fast.** First words in about 1 to 3 seconds. Pre-warm, keep processes
   alive, never block on something that could start earlier.
2. **Light.** Native Tauri app, not Electron. Idle cost is one small tray
   process. AI process and speech model are released after a few idle
   minutes (`keep_warm_minutes`).
3. **Out of the way.** No taskbar window. Never steal focus unless the user
   is typing. Collapsed, it is only the notch.
4. **Voice first, typing second.** Hold to talk is the main path; a tap
   opens the notch for typing.
5. **Private.** Speech is transcribed on the device. The overlay is excluded
   from screen capture by default. Nothing is sent anywhere except through
   the user's own CLI.

## Modes

| Mode | Trigger | Behaviour |
| --- | --- | --- |
| Push to talk | Hold Ctrl + Alt (or the user's shortcut, default Ctrl + Alt + Space) | Records while held. On release: transcribe locally, send with a screenshot, answer streams into the notch. No Enter key. |
| Type | Tap Ctrl + Alt (held < 280 ms) | Notch takes focus with a text field. |
| Work with me (live) | Ctrl + Alt + L or tray | Listens continuously to the microphone ("Me") and the computer's audio ("Them"), cuts speech at pauses, transcribes, and sends each batch with a screenshot at most every 15 s. The AI replies with a single "." when it has nothing useful; the UI shows nothing for those. |
| Insert | Ctrl + Alt + Enter (registered while visible) or Ctrl + Enter in the notch | Pastes the last answer into the app the user came from. |

## Layout

```
src-tauri/                  Rust (Tauri 2) – all system work
  src/main.rs               App state, windows, tray, commands, push-to-talk
                            flow, live-mode loop, settings application
  src/backend.rs            Claude/Codex CLI bridge, system prompts
                            (SYSTEM_PROMPT, LIVE_PROMPT), stream parsing
  src/hotkey.rs             Low-level keyboard hook (Windows) + pure gesture
                            state machine (Down / Up(ms) / Cancel / Tap)
  src/audio.rs              Microphone recording, continuous Listener
                            (mic or system loopback), Segmenter, resampling
  src/stt.rs                whisper.cpp transcription, model download
  src/platform.rs           Win32: foreground window, monitor screenshot,
                            force-foreground, paste (Ctrl+V via SendInput)
  src/dock.rs               Pure maths for docking to a screen edge
  src/settings.rs           Settings struct, JSON in the app config dir
  src/update.rs             In-app updater (installer copies only), channel
                            detection (installer / Scoop / manual)
  cross/                    Linux → Windows cross-build script and MinGW shim
  tauri.conf.json           Two windows: "main" (notch) and "glow"
ui/                         Plain HTML/CSS/JS – no framework, no build step
  index.html                The notch: #notch > #clip > #content > #head + #panel
  main.js                   All interface behaviour
  style.css                 Notch design
  markdown.js               Small safe Markdown renderer (escapes first)
  glow.html                 Click-through screen-edge glow
tests/ui.test.mjs           Playwright test of the UI with a fake Tauri bridge
scripts/version.mjs         One version everywhere; changelog → release notes
scripts/packaging.mjs       Scoop / winget / Chocolatey manifests + SHA256SUMS
packaging/README.md         One-time package manager setup (secrets, first
                            winget submission)
bucket/agent-desk.json      Scoop manifest, written by the release workflow
.github/workflows/ci.yml    Every push/PR: fmt, version check, actionlint,
                            UI test (+ screenshots), cargo test, clippy,
                            Windows installer build
.github/workflows/release.yml  Tag v* → installer + portable zip → GitHub
                            Release → Scoop bucket, winget, Chocolatey
.github/workflows/audit.yml Weekly cargo audit + npm audit
```

## How the pieces talk

- **UI → Rust:** `invoke(...)` commands in `main.rs`: `get_settings`,
  `save_settings`, `status`, `ask`, `stop`, `new_chat`, `hide`, `resize`,
  `focus`, `copy_text`, `insert_text`.
- **Rust → UI:** events emitted to the `main` window:
  - `ai` `{turn, kind: delta|done|error, text}` answer stream
  - `phase` `{phase: listening|transcribing|preparing|cancelled|error, text}`
  - `asked` `{turn, text, quiet}` a question the app sent itself (speech or
    live mode). `quiet: true` means live check-in: show nothing unless the
    answer is more than "."
  - `wake` `{title, process, has_screen, typing}`, `refocus`, `level`
    (0–1 loudness), `live` (bool), `live-heard` `{who, text}`, `live-note`,
    `insert-request`, `open-settings`, `settings-changed`.
- **Turn ids:** every `ask` gets an increasing turn id. Events for other
  turns are ignored. Events can arrive **before** the UI knows its turn id
  (the CLI is fast); `main.js` buffers them in `state.early`. Keep that.
- **Busy flag:** `Engine::is_busy()` is true while an answer streams. Live
  mode never sends while busy; what is heard meanwhile is batched.

## AI backend rules

- Claude runs as **one long-lived process** per conversation:
  `claude -p --input-format stream-json --output-format stream-json
  --include-partial-messages --tools "" --strict-mcp-config
  --disable-slash-commands --no-session-persistence --system-prompt-file …`
  in an empty `workspace` folder. It must only chat: no tools, no MCP, no
  project files, nothing written to the user's history.
- The process key is `model|path|live`; changing any of these starts a new
  process. Normal and live mode are separate conversations.
- An answer is "done" on the stream's `message_delta` with a final stop
  reason, not on the later `result` line (that arrives seconds later).
- Codex runs per turn as `codex exec --json --skip-git-repo-check -c
  sandbox_mode=read-only`, resuming the stored thread for follow-ups.
- Screenshots: JPEG, long edge 1568 px, sent once per conversation in normal
  mode (follow-ups rely on context), periodically in live mode.
- Default Claude model is `haiku` for speed. Do not change the default to a
  slower model.
- Error text from the CLIs goes through `explain()` to give the user an
  action (sign in, usage limit). Keep messages plain and actionable.

## Speech rules

- Neither CLI accepts audio, so speech is transcribed locally with
  **whisper.cpp** (`whisper-rs`). Audio never leaves the machine.
- Model is downloaded on first use with the `curl` that ships with Windows,
  into `<config>/models/`. Default `base.en` (148 MB).
- Silence must not reach Whisper (it invents words). Use
  `audio::has_speech` for push-to-talk and `Segmenter` for live mode, and
  strip bracketed tags with `stt::clean`.
- Recording starts on key **down** so the first word is not lost; the UI
  only shows "Listening" once the hold passes the tap threshold.

## Keyboard rules

- Windows cannot register a modifier-only hotkey, so Ctrl + Alt uses a
  low-level keyboard hook (`WH_KEYBOARD_LL`). The hook only detects the
  gesture. It must never block, change, log or store keystrokes, and it is
  only installed while a gesture mode is selected.
- Ctrl + Alt plus any other key is an ordinary shortcut → `Cancel`, never
  wake. Right Alt (AltGr) counts as "other". Injected keys are ignored.
- Keep `hotkey::Detector` pure and covered by unit tests; change behaviour
  there, not in the hook callback.

## Design rules (the notch)

The look follows the user's references: a macOS-style black notch, a 3 × 3
pixel indicator, shimmering label text, curved "ears" joining the screen
edge, a soft screen-edge glow while listening.

- **One shape.** The notch and the answer are the same element. Collapsed it
  is just the notch; when there is something to read it **morphs** (width
  and height animate) into the panel and shrinks back. Never add a second
  floating box, modal or card. Settings also open inside the notch.
- **Minimal by default.** While working, show only state: Listening,
  Transcribing, Thinking, Answering, Done, Working with you, or an error.
  Small grey top line for context (what it sees, the question, the last
  thing heard). Tools (settings, close) appear on hover only. Answer actions
  (Insert, Copy, New) appear only when an answer is finished.
- **State colours** (`STATES` in `main.js`) drive `--glow`: listening
  `#ffc18f`, transcribing `#ff99a8`, preparing `#ffeb85`, thinking
  `#c299ff`, answering `#82ff9e`, done/idle `#8ab4ff`, live `#7fe3ff`,
  error `#ff8f8f`.
- **Background** is `rgba(0,0,0,var(--alpha))`; `--alpha` comes from the
  Transparency setting (0.3–1.0). Text `#f2f3f7`, muted `#8c8c8c`.
- **Type:** Inter / Segoe UI Variable. Label 15 px semibold, top line
  12.5 px, answer 14.5 px. Code in Cascadia Code / Consolas.
- **Motion:** morph 0.42 s `cubic-bezier(0.22,1,0.36,1)`, entrance drop
  from the docked edge. Everything respects `prefers-reduced-motion`.
- **Glow** is subtle and short (a thin inset band), never a full-screen
  wash. It is a separate click-through window and is only on while
  listening.
- **Docking:** top, bottom, left or right edge of any monitor. Shape and
  ears adapt per edge (`data-edge` on `#shell`). Dragging snaps to the
  nearest edge (`dock::snap`). The native window is sized by `main.js`
  (`morph()` → `resize`) to exactly the shape plus a margin so it never
  blocks clicks; it grows before the animation and shrinks after it.
- **Focus:** the notch appears without taking focus for voice and live mode
  (`set_focusable(false)`); it only takes focus when the user types or
  clicks it.
- Model output is untrusted: always render through `renderMarkdown`, which
  escapes HTML first. Never use `innerHTML` with raw model text.
- No frameworks, no bundler, no CDN at runtime. Keep the UI plain files.

## Screen capture hiding

The notch and glow windows use Tauri content protection, which on Windows is
`SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)`. It is on by default with
a tray toggle. This is to keep a private assistant out of screen shares. Do
**not** add process-name randomisation, title spoofing, anti-debugging or
other ways of disguising the app (the reference project `cheating-daddy`
does these; we deliberately do not). The app shows under its own name.

## Building and testing

Requirements: Rust stable, Node 22, CMake, LLVM/Clang (whisper.cpp is built
from source). On Windows, WebView2 (preinstalled on Windows 11).

```bash
npm ci && npx playwright install chromium
npm run dev                                   # run the app
npm test                                      # cargo test + UI test
npm run test:ui -- <screenshot-dir>           # UI test with screenshots
npm run format && npm run lint                # rustfmt, clippy, version check
npx tauri build                               # Windows installer (on Windows)
```

### Versions and releases

`package.json` holds the version; `tauri.conf.json` points at it and
`scripts/version.mjs` keeps `Cargo.toml`, `Cargo.lock` and
`package-lock.json` in step (CI fails if they drift). Release with
`npm run release:version -- X.Y.Z`, commit, then push tag `vX.Y.Z`; the
release workflow refuses a tag that does not match the files. Its notes
come from that version's section of `CHANGELOG.md`, so add user-facing
changes under **Unreleased** as you go. Package manager identity (names,
ids, description) is the `ID` block in `scripts/packaging.mjs`.

CI fails on `clippy::correctness` and `clippy::suspicious` only; other
lints are warnings.

### In-app updates

`tauri-plugin-updater` reads `latest.json` from the latest GitHub Release
(`plugins.updater` in `tauri.conf.json`, which holds the public key). Only
copies installed by our NSIS installer update themselves (`update::channel`
looks for `uninstall.exe` next to the exe); Scoop, winget, Chocolatey and
the zip are left to their own tools so two updaters never fight. Updates are
downloaded in the background but installed only when the user picks
"Restart to update" in the tray, because installing closes the app.

The release workflow signs the installer when the repository secret
`TAURI_SIGNING_PRIVATE_KEY` is set (private key kept outside the repo; the
matching public key is in `tauri.conf.json`) and then uploads `latest.json`
and the `.sig`. Without the secret the release still works but does not
offer itself to the updater. Losing the private key means existing installs
can no longer verify updates, so keep a backup of it.

Ignored tests that need real resources:

```bash
# real Claude CLI, signed in
cargo test --manifest-path src-tauri/Cargo.toml -- --ignored live_claude
cargo test --manifest-path src-tauri/Cargo.toml -- --ignored live_mode
# real Whisper model and a 16 kHz mono WAV
AGENT_DESK_MODELS=<dir with models/> AGENT_DESK_WAV=<file> \
AGENT_DESK_EXPECT="phrase in it" \
cargo test --manifest-path src-tauri/Cargo.toml --release -- --ignored live_whisper
```

From Linux, `src-tauri/cross/build-windows.sh` cross-compiles a Windows
test build (needs `mingw-w64`). It works around whisper.cpp's MinGW library
names and an old MinGW header (`cross/mingw-shim.h`). The real release path
is the Release workflow on `windows-latest`.

The app builds and runs on Linux for development; Windows-only parts
(screenshot, paste, keyboard hook, loopback audio) are stubbed there.

### Before you finish a change

1. `cargo fmt --check` and `cargo test` pass, and `cargo check --target x86_64-pc-windows-gnu`
   (or the cross script) still compiles the Windows code.
2. `tests/ui.test.mjs` passes. Add assertions for any UI behaviour you
   change; it fakes the Rust side with `window.__TAURI__`.
3. Take screenshots with the UI test's screenshot directory and look at
   them. Design regressions are the most common failure here.
4. Keep `README.md` and this file accurate.

## Known gaps (not yet verified on real Windows)

- Microphone capture, the Ctrl + Alt hook, paste into other apps, the
  click-through glow window, dragging between edges, and whether the notch
  appears without stealing focus.
- System-audio (loopback) capture for live mode has never run; if it fails,
  live mode falls back to the microphone and shows a note.
- Smoothness of the native window resizing alongside the morph animation.
- Codex has been parsed against sample events but not run end to end.

## Conventions

- Rust: small modules, pure logic separated from OS calls so it can be unit
  tested on Linux. Comments explain why, in plain English.
- User-facing text: plain, short, actionable, no jargon. Errors say what to
  do next.
- Do not add dependencies lightly; size and idle cost matter.
- Settings are `#[serde(default)]`; new fields need a default in
  `Settings::default()` and handling in `TEXT_FIELDS` / `TOGGLES` in
  `main.js` and the settings form in `index.html`.
- Live mode records other people. Keep the consent note in the README and
  do not make live mode start automatically.
