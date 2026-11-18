# Agent Desk

A small AI assistant for Windows that stays out of the way until you call it.
Tap **Ctrl + Alt** in any app and a prompt bar appears. It already sees the
window you were working in, answers through your existing **Claude** or
**ChatGPT (Codex)** subscription, and can paste the answer straight back.

No API key and no extra bill: Agent Desk drives the `claude` or `codex`
command-line tool you are already signed in to.

## What it does

- **Wakes anywhere.** Tap Ctrl + Alt, double-tap Ctrl, or use a shortcut you
  choose (default Ctrl + Alt + Space).
- **Sees your screen.** A screenshot of the active monitor and the window
  title go with your first question, so "draft a reply" or just Enter is
  enough.
- **Answers fast.** The Claude session starts while you type and stays open
  for follow-ups, so the first words arrive in about a second or two.
- **Pastes back.** Ctrl + Enter inserts the answer into the app you came
  from.
- **Stays light.** A native Tauri app, not Electron. When idle it is one
  small tray process; the AI process is closed after a few idle minutes.
- **Stays private on screen shares.** The overlay is excluded from
  screenshots, recordings and screen sharing (a tray toggle turns this off).

## Requirements

- Windows 10 version 2004 or later, or Windows 11 (WebView2 is preinstalled
  on Windows 11).
- One of:
  - [Claude Code](https://claude.com/claude-code), signed in with your Claude
    subscription (run `claude` once in a terminal), or
  - the Codex CLI (`npm install -g @openai/codex`), signed in with your
    ChatGPT plan (run `codex` once).

## Install

Download `agent-desk-windows` from the latest run of the **Build** workflow
on GitHub (Actions tab) and run the installer inside. The app is not code
signed, so Windows SmartScreen will ask you to confirm.

## Keys

| Key | Action |
| --- | --- |
| Ctrl + Alt (tap) | Open or hide the overlay |
| Enter | Ask (empty = "help with what's on my screen") |
| Shift + Enter | New line |
| Ctrl + Enter | Paste the answer into the previous app |
| Ctrl + Shift + C | Copy the answer |
| Ctrl + N | New conversation |
| Ctrl + , | Settings |
| Esc | Stop the answer, then hide |

## How it works

| Part | File |
| --- | --- |
| Tray, overlay window, commands | `src-tauri/src/main.rs` |
| Claude and Codex CLI bridge | `src-tauri/src/backend.rs` |
| Ctrl + Alt and double-Ctrl detection | `src-tauri/src/hotkey.rs` |
| Foreground window, screenshot, paste | `src-tauri/src/platform.rs` |
| Settings file | `src-tauri/src/settings.rs` |
| Overlay interface | `ui/` |

Claude runs as `claude -p` in streaming JSON mode with all tools, MCP servers
and session history turned off, inside an empty folder: it can only chat.
Codex runs as `codex exec --json` in a read-only sandbox.

The modifier-only wake gestures use a low-level keyboard hook because Windows
cannot register a hotkey made only of modifier keys. The hook looks for the
gesture and nothing else; it does not record or change keystrokes, and it is
only installed while one of those gestures is selected.

Hiding from capture uses the operating system's own window setting
(`SetWindowDisplayAffinity` with `WDA_EXCLUDEFROMCAPTURE`, through Tauri's
content protection). It is meant to keep your private assistant out of
screen shares. It does not disguise the app: Agent Desk shows up under its
own name in the tray and Task Manager.

## Develop

```bash
npm install
```

```bash
npm run dev
```

```bash
cargo test --manifest-path src-tauri/Cargo.toml
```

```bash
node tests/ui.test.mjs
```

The UI test needs Playwright (`npm install --no-save playwright`). A live
end-to-end test against a signed-in Claude CLI runs with
`cargo test --manifest-path src-tauri/Cargo.toml -- --ignored live_claude`.

The app also builds and runs on Linux for development, without the
Windows-only parts (screenshot, paste, modifier-only gestures).
