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
API key and no server of our own. It can also **act on the desktop** (open
pages and apps, switch windows, type, press keys, run saved commands) and
**run scheduled tasks**, through a small fixed set of actions that the user
sees, and confirms when they are risky.

Core promises, in priority order. Do not trade one away without the user
asking:

1. **Fast.** First words in about 1 to 3 seconds. Pre-warm, keep processes
   alive, never block on something that could start earlier.
2. **Light.** Native Tauri app, not Electron. Idle cost is one small tray
   process (measured: ~20 MB for the app; the WebView2 processes behind the
   notch are the larger part and are asked to release memory while hidden). AI process and speech model are released after a few idle
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
| Act | Any request in push to talk or type mode | The AI may call Agent Desk's actions. Each step shows in the notch; risky ones wait for Allow / Always allow / Don't (Enter / Esc). Never in live mode. |
| Scheduled | A schedule's time comes (tray process checks every 20 s) | Starts a fresh conversation with the schedule's prompt, as if the user asked, and shows it in the notch. |

## Layout

```
src-tauri/                  Rust (Tauri 2) – all system work
  src/main.rs               App state, windows, tray, commands, push-to-talk
                            flow, live-mode loop, settings application
  src/backend.rs            Claude/Codex CLI bridge, system prompts
                            (SYSTEM_PROMPT, ACTIONS_PROMPT, LIVE_PROMPT),
                            MCP config for each CLI, stream parsing
  src/actions.rs            Pure: the allowed actions, argument checks,
                            which ones need confirmation, tool definitions,
                            Start-menu and window matching
  src/mcp.rs                Minimal MCP server (HTTP on 127.0.0.1, bearer
                            token, no Origin) the CLIs call actions through
  src/tasks.rs              Runs actions with the notch in the loop
                            (show, confirm, execute), and the schedule loop
  src/schedule.rs           Pure: schedule times (local minute stamps),
                            due/next-run logic, schedules.json storage
  src/uia.rs                Windows UI Automation (elements by name, also
                            inside browser pages), element under a point,
                            press without the mouse (Invoke/Toggle/Select,
                            SetFocus), brief real click that puts the
                            user's cursor back, Start-menu apps incl.
                            Store apps (shell:AppsFolder), PDF pages via
                            Windows.Data.Pdf
  src/files.rs              Folders and documents: list, read text/.docx,
                            off-limits paths, which files run something
  src/setup.rs              Setup screen facts from the CLIs: version,
                            sign-in (`claude auth status --json`, `codex
                            login status`), models (`codex debug models`),
                            speech models on disk
  src/hotkey.rs             Low-level keyboard hook (Windows) + pure gesture
                            state machine (Down / Up(ms) / Cancel / Tap)
  src/audio.rs              Microphone recording, continuous Listener
                            (mic or system loopback), Segmenter, resampling
  src/stt.rs                whisper.cpp transcription, model download
  src/platform.rs           Win32: foreground window, monitor screenshot,
                            force-foreground, paste (Ctrl+V via SendInput),
                            ShellExecute, window list, typing and key
                            chords (SendInput), local time, saved commands
  src/dock.rs               Pure maths for docking to a screen edge
  src/settings.rs           Settings struct, JSON in the app config dir
  src/update.rs             In-app updater (installer copies only), channel
                            detection (installer / Scoop / manual)
  cross/                    Linux → Windows cross-build script and MinGW shim
  src/pointer.rs            Agent Desk's own labelled pointer window
  tauri.conf.json           Windows: "main" (notch), "glow", "pointer"
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
  `focus`, `copy_text`, `insert_text`, `answer_action`, `list_schedules`,
  `save_schedule`, `delete_schedule`, `run_schedule_now`, `setup_info`,
  `list_models`, `download_speech`.
- **Rust → UI:** events emitted to the `main` window:
  - `ai` `{turn, kind: delta|done|error, text}` answer stream
  - `phase` `{phase: listening|transcribing|preparing|cancelled|error, text}`
  - `asked` `{turn, text, quiet, scheduled}` a question the app sent itself
    (speech, live mode or a schedule). `quiet: true` means live check-in:
    show nothing unless the answer is more than ".". `scheduled: true`
    clears the thread first.
  - `action` `{id, turn, kind, summary, detail, status, result}` one desktop
    step; `status` is `ask` (show Allow / Always allow / Don't), `running`,
    `done`, `denied` or `failed`. Same turn rules as `ai`.
  - `schedules-changed` reload the list if Settings is open.
  - `open-setup` show the setup screen (first launch until `setup_done`,
    tray "Setup and guide", Settings button).
  - `speech-progress` `{model, pct, done, error}` speech model download.
  - `wake` `{title, process, has_screen, typing}`, `refocus`, `level`
    (0–1 loudness; not drawn any more), `live` (bool), `live-heard` `{who, text}`, `live-note`,
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
  in an empty `workspace` folder. No built-in tools (no files, shell or
  web), none of the user's MCP servers, no project files, nothing written
  to the user's history.
- **The only tools are Agent Desk's own actions**, and only when
  `actions_enabled` is on and not in live mode. Claude then also gets
  `--mcp-config <config>/mcp.json --allowedTools mcp__agentdesk`, and the
  system prompt gets `ACTIONS_PROMPT`. Never add `Bash`, `Read`, `WebFetch`
  or another server, and never give the AI a free-form shell.
- The process key is `model|path|live|tools`; changing any of these starts a
  new process. Normal and live mode are separate conversations.
- Answers that use tools stop with `stop_reason: tool_use` (`Parsed::Pause`)
  and continue; that is not "done". Text after a step starts a new
  paragraph.
- An answer is "done" on the stream's `message_delta` with a final stop
  reason, not on the later `result` line (that arrives seconds later).
- Codex runs per turn as `codex exec --json --skip-git-repo-check -c
  sandbox_mode=read-only --disable shell_tool --disable unified_exec`
  (`CODEX_NO_SHELL`: Codex's own shell could read any file around Agent
  Desk's checks; verified against codex-cli 0.170 that these remove it),
  resuming the stored thread for follow-ups. With
  actions on, `codex_tool_args` adds the same server
  (`-c mcp_servers.agentdesk.url=…`, token via the `AGENT_DESK_MCP_TOKEN`
  environment variable, `default_tools_approval_mode="approve"` because the
  notch does the asking) and switches off the user's apps, plugins and every
  `[mcp_servers.<name>]` in `~/.codex/config.toml` for that run. Same
  actions, same checks and confirmations for both providers.

## Action rules

- **Fixed allowlist** (`actions::tool_definitions`): `open_url` (http/https
  only, optional chrome/edge/brave/firefox and profile),
  `list_browser_profiles`, `open_app` (Start menu incl. Store apps; names
  only, never paths), `focus_window`, `find_on_screen` (UI Automation
  elements, numbered), `click` (element number or x/y in the last
  screenshot; Agent Desk's own pointer glides there), `fill` (click a box,
  replace its text; new lines are Shift+Enter), `scroll`, `type_text`,
  `press_keys` (one chord), `open_path`, `list_folder`, `read_file`
  (text, .docx, PDF as page images), `attach_file` (attach button → upload
  from this device → Open dialog, in one step), `copy_to_clipboard`,
  `read_clipboard`, `run_saved_command` (only commands the user saved in
  Settings, by name), `look_at_screen`, `wait` (≤ 10 s), `schedule_task`.
- **Clicks are judged by what is under the pointer**: the label is read
  (element name, or `ElementFromPoint` for x/y) before asking. Labels like
  Send, Post, Delete, Buy, Pay, Submit, Merge, Save (`uia::
  is_consequential_label`) ask first; "Always allow" for a click is per
  label (`click:send`).
- **Speed**: every model round trip costs 1–2 s; a screenshot adds only
  ~0.3–0.5 s and shrinking it does not help (measured). So `click`, `fill`
  and `attach_file` return the fresh element list of the window in front,
  and the prompt tells the model to use `look_at_screen` only as a last
  resort.
- **Files**: paths resolve from the user's home folder; `files::
  is_off_limits` refuses `.ssh`, `AppData`, `.env*`, key files and the
  Windows folder. Claude Code does not pass PDFs returned by an MCP tool to
  the model (it saves them to disk), so PDFs are sent as page images.
- **Browser profiles**: chosen once per browser and stored in
  `browser_profiles`; Chrome cannot be driven through its remote debugging
  protocol on the user's real profile, so the browser is driven through UI
  Automation and input like any other app. Adding an action means: parse and check it in
  `actions.rs`, classify it in `needs_confirmation`, carry it out in
  `tasks::execute`, and add tests.
- **Confirmation policy** (`Action::needs_confirmation`): asks first for
  consequential clicks (above), opening a program or script,
  typing that contains a new line (Enter), every key chord except plain
  navigation and editing (Tab, arrows, Esc, Home/End, Page Up/Down,
  Ctrl+T/L/F/C/V/A/Z,
  Ctrl+Tab, Ctrl+1–9, Alt+Left/Right/Tab), every saved command, and every
  new schedule. The AI can mark any step `consequential` to ask too; it can
  never skip a question. "Always allow" stores `allow_key()` in
  `always_allow` (per action type; per command name for saved commands) and
  the user can undo it in Settings. No answer in 2 minutes, Stop, New or
  Quit all count as "no".
- **Conversation rules** (`Action::must_ask`): once `read_file`,
  `list_folder` or `read_clipboard` ran in a conversation (`private_data`,
  reset by New), every `open_url` asks and cannot be always-allowed, so
  data cannot leave in a link. A link with a query or fragment over 64
  characters always asks. In a scheduled turn every step asks. While a step
  waits for the user, every other action is refused, and clicks on Agent
  Desk's own windows are refused (`refuse_own_window`).
- **Always allow** (`can_always_allow`, `allow_key`) is per chord for keys,
  per extension for `open_path`, per label for clicks, per name for saved
  commands; never for schedules, programs, attachments or links. The
  `action` event carries `always: false` to hide the button.
- **Typed text** goes through `clean_typed_text`: `\r` and `\r\n` become
  `\n` (Enter), other control characters except tab are refused.
- **Paths** go through `files::check_path` after resolving: network and
  device paths refused before any file system call, names ending in a dot
  or space refused, off-limits checked on the canonical path too. Opening
  asks unless the type is in `SAFE_TO_OPEN` (documents, pictures, media) or
  it is a folder.
- **Window security**: a strict CSP in `tauri.conf.json` (no inline
  scripts; `glow.js` holds the glow page's script). App commands are
  declared in `build.rs` (`AppManifest`), so each is a permission; only the
  `main` window gets them (`capabilities/default.json`), the glow window
  only listens for events (`capabilities/glow.json`). A new command must be
  added to both `build.rs` and `default.json`. `tests/csp.test.mjs` loads
  both pages under the policy.
- **Shown before it runs.** Every step is emitted as an `action` event
  before anything happens, even harmless ones.
- **Instructions on screen are data.** `ACTIONS_PROMPT` tells the model to
  act only on the user's own words and to treat screen, page and tool text
  as information. Live mode never gets tools (other people's speech).
- **The server** (`mcp.rs`) binds `127.0.0.1` on a random port, requires a
  random bearer token per app run (BCryptGenRandom), and refuses any
  request carrying an `Origin` header (a web page). The token is checked
  before the body is read; headers are capped at 16 KB, bodies at 4 MB,
  open connections at 16. `mcp.json` is deleted on Quit. No HTTP or MCP
  library.
- Typing and keys never go into the notch: `prepare_keyboard` refocuses the
  window the user came from (or the last `focus_window` target).

## Setup screen

- Shown inside the notch on first launch until `setup_done`, and from the
  tray ("Setup and guide") or Settings. It is one more view of the same
  shape, like Settings.
- Facts come from the CLIs (`setup.rs`), run hidden with a time limit only
  while the screen is open. Claude has no model list command, so the list
  is the names the CLI accepts (haiku, sonnet, opus) plus the full model
  name the running CLI reported in its `init` line; Codex models come from
  `codex debug models` (visibility "list").
- "Account folder" pins one sign-in: `claude_config_dir` is passed as
  `CLAUDE_CONFIG_DIR`, `codex_home` as `CODEX_HOME`, to every CLI process
  (and is part of the Claude process key).

## Schedule rules

- Stored in `<config>/schedules.json` (`schedule::save` writes then
  renames). Run by `tasks::watch_schedules` in the tray process every 20 s;
  no service, no Task Scheduler.
- Times are local wall-clock minutes (`platform::local_now` →
  `schedule::stamp`). `Repeat {time, days}` (0 = Monday, empty = every day)
  catches up only within `GRACE_MINUTES` (30) and never for a time before it
  was created; `Once {date, time}` runs the first time it is seen due, late
  if need be, then shows as finished.
- `last_run` is recorded before running so nothing runs twice. Schedules
  wait while live mode is on, the user is talking, or an answer streams.
- A run starts a fresh conversation (`new_chat`) whose prompt begins
  `[Scheduled task "<name>", <when>]`.
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

## Crash recovery

`recovery.rs`. Keep all three parts:

- `RegisterApplicationRestart` makes Windows restart the app after a crash
  or hang (only once it has run for a minute, so a start-up crash cannot
  loop). A panic hook and an unhandled-exception filter write one line to
  `<config>/crash.log`: what failed and where, never user text.
- `send_question` goes through `send_tracked`, which keeps the request in
  `<config>/pending-task.json` until its answer is done, errors or is
  stopped (`task_finished`). On start a recent one (15 min) is offered once
  in the notch (`take_recovery` / `resume_task` / `dismiss_recovery`). It is
  never resent without the user saying Continue, because steps may already
  have run; the resent prompt tells the AI to check the screen first.
- When the Claude process dies mid-answer the backend emits `lost`, and
  main.rs resends the request once to a fresh process (`answer_lost`); a
  second loss becomes an ordinary error. The UI never sees `lost`.
- whisper.cpp is built for AVX2/FMA/F16C (`GGML_*` in the workflows),
  never `GGML_NATIVE`: GitHub's runners have AVX-512 and a native build
  crashes on CPUs without it. `stt::cpu_supported` checks before loading.

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
  Transcribing, Thinking, Answering, Working on it, Needs your OK, Done,
  Working with you, or an error.
  Small grey top line for context (what it sees, the question, the last
  thing heard). No sound meter: the label says Listening. Tools (settings,
  close) sit at the right edge and appear on hover only. Answer actions
  (Insert, Copy, New) appear only when an answer is finished.
- **Latest answer only.** A new question replaces what is on screen (the AI
  still has the earlier turns). A live check-in keeps the last answer until
  it has something to say.
- **Tucks away.** 5 s after Done the notch shrinks back and hides. The mouse
  over it holds it open (2 s after it leaves); typing, settings, a step
  waiting for an OK, an error, a notice and live mode never tuck.
- **Cheap to draw.** No `filter`/`backdrop-filter` on anything that
  animates, no `will-change` on the morphing shape. While an answer streams
  the native window grows in 96 px steps and fits exactly when it is done,
  so it is not resized for every line.
- **State colours** (`STATES` in `main.js`) drive `--glow`: listening
  `#ffc18f`, transcribing `#ff99a8`, preparing `#ffeb85`, thinking
  `#c299ff`, answering `#82ff9e`, acting `#9fe3c4`, confirm `#ffd58a`,
  done/idle `#8ab4ff`, live `#7fe3ff`, error `#ff8f8f`.
- **Actions** appear inside the answer as `.msg.act` rows in the order they
  happen (text before a step stays above it). A step waiting for the user
  shows the exact text or command and three buttons: Allow, Always allow,
  Don't. While a step waits the notch stays open (do not auto-collapse).
- **Background** is `rgba(0,0,0,var(--alpha))`; `--alpha` comes from the
  Transparency setting (0.3–1.0). Text `#f2f3f7`, muted `#8c8c8c`.
- **Type:** Inter / Segoe UI Variable. Label 15 px semibold, top line
  12.5 px, answer 14.5 px. Code in Cascadia Code / Consolas.
- **Motion:** morph 0.42 s `cubic-bezier(0.22,1,0.36,1)`, entrance drop
  from the docked edge. Everything respects `prefers-reduced-motion`.
- **Glow** is subtle and short (a thin inset band), never a full-screen
  wash. It is a separate click-through window, on while listening and,
  in the working colour, while Agent Desk acts on the computer
  (`working()`; off when the answer ends or is stopped).
- **Never take the user's mouse.** Agent Desk shows its own pointer (the
  click-through "pointer" window, an arrow labelled "Agent Desk" whose
  tip is the window's corner, placed 1 px past the target so it never
  covers it). Clicks go through UI Automation (`uia::press_at`,
  `uia::focus_at`); only right/double clicks, screenshot spots and
  elements without those patterns use the real cursor, through
  `uia::at_point`, which puts it back within ~50 ms. Never glide the
  real cursor.
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

- Actions through the whole app: the MCP server, `open_url` in Chrome and
  the real `claude` and `codex` CLIs were run end to end on Windows from a
  test harness, but not from the packaged app with voice. Typing, key
  chords, `focus_window`, `open_app` and the confirmation buttons in the
  non-focusable notch have not been tried by hand.
- Codex: MCP servers whose names need quoting (for example `my-server`)
  cannot be switched off with `-c` and stay available to Codex.
- Schedules have unit tests for timing but have not fired from the real
  tray app yet.

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
