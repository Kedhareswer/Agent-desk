# Agent Desk

A voice-first AI assistant for Windows that stays out of the way until you
call it. Hold **Ctrl + Alt**, say what you need, and let go. A small black
notch on the edge of your screen shows what it is doing, then grows into the
answer. It already sees the window you were working in, answers through your
existing **Claude** or **ChatGPT (Codex)** subscription, and can paste the
answer straight back.

No API key and no extra bill: Agent Desk drives the `claude` or `codex`
command-line tool you are already signed in to. Speech is turned into text
on your own computer.

## What it does

- **Hold to talk.** Hold Ctrl + Alt, speak, release. It transcribes what you
  said and starts working, with no Enter key. A quick tap opens the same
  notch for typing instead.
- **Work with me (live mode).** Press Ctrl + Alt + L and it follows along:
  it listens to your microphone and to the computer's audio (the other
  people in a call), looks at your screen now and then, and speaks up only
  when there is something to help with, such as a question someone just
  asked. Press the keys again to stop.
- **One notch.** Collapsed, it shows the state (Listening, Thinking,
  Answering, Done). When there is something to read, the same shape grows to
  hold it, then shrinks back.
- **Sees your screen.** A screenshot of the active monitor and the window
  title go with your first question, so "draft a reply" is enough.
- **Pastes back.** Ctrl + Alt + Enter inserts the answer into the app you
  are in, without touching the notch.
- **Goes where you want.** Drag it to the top, bottom, left or right edge of
  any screen. A transparency slider makes it more or less see-through.
- **Soft edge glow.** A thin band of light around the screen while it is
  listening (can be turned off).
- **Stays light.** A native Tauri app, not Electron. When idle it is one
  small tray process; the AI process and the speech model are released after
  a few idle minutes.
- **Stays private on screen shares.** The notch and glow are excluded from
  screenshots, recordings and screen sharing (a tray toggle turns this off).

## Requirements

- Windows 10 version 2004 or later, or Windows 11 (WebView2 is preinstalled
  on Windows 11).
- One of:
  - [Claude Code](https://claude.com/claude-code), signed in with your Claude
    subscription (run `claude` once in a terminal), or
  - the Codex CLI (`npm install -g @openai/codex`), signed in with your
    ChatGPT plan (run `codex` once).
- A microphone. The first time you talk, a speech model is downloaded once
  (148 MB by default; a 75 MB and a 488 MB option are in Settings).

## Install

Download `agent-desk-windows` from the latest run of the **Build** workflow
on GitHub (Actions tab) and run the installer inside. The app is not code
signed, so Windows SmartScreen will ask you to confirm.

## Keys

| Key | Action |
| --- | --- |
| Ctrl + Alt (hold) | Talk; release to send |
| Ctrl + Alt (tap) | Open the notch to type, or hide it |
| Ctrl + Alt + L | Turn "work with me" live mode on or off |
| Ctrl + Alt + Enter | Paste the answer into the app in front |
| Enter | Ask what you typed (empty = "help with what's on my screen") |
| Shift + Enter | New line |
| Ctrl + Shift + C | Copy the answer |
| Ctrl + N | New conversation |
| Ctrl + , | Settings |
| Esc | Stop the answer, then hide |

Your own shortcut (default Ctrl + Alt + Space) works the same way: hold to
talk, tap to type.

## How it works

| Part | File |
| --- | --- |
| Tray, overlay window, commands | `src-tauri/src/main.rs` |
| Claude and Codex CLI bridge | `src-tauri/src/backend.rs` |
| Hold, tap and double-tap key detection | `src-tauri/src/hotkey.rs` |
| Microphone capture | `src-tauri/src/audio.rs` |
| Speech-to-text (whisper.cpp) | `src-tauri/src/stt.rs` |
| Docking to a screen edge | `src-tauri/src/dock.rs` |
| Foreground window, screenshot, paste | `src-tauri/src/platform.rs` |
| Settings file | `src-tauri/src/settings.rs` |
| Notch and glow interface | `ui/` |

Claude runs as `claude -p` in streaming JSON mode with all tools, MCP servers
and session history turned off, inside an empty folder: it can only chat.
Codex runs as `codex exec --json` in a read-only sandbox.

Neither Claude nor Codex accepts audio, so speech is transcribed locally
with [whisper.cpp](https://github.com/ggml-org/whisper.cpp) and sent as
text. Audio never leaves the computer.

In live mode each thing that is said is cut out of the audio stream at the
pauses, transcribed, and passed to the AI marked "Me" (microphone) or
"Them" (computer audio), with a fresh screenshot every 15 seconds at most.
The AI is told to answer with a single full stop when it has nothing to
add, and the notch stays collapsed for those. Live mode uses more of your
subscription's allowance than asking by hand, because every remark is a
request. With speakers instead of headphones, the microphone also picks up
the other people, so they may be heard twice.

Listening to a call records other people. Depending on where you and they
are, that can require telling them or getting their consent.

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

Building needs CMake and LLVM/Clang as well as Rust and Node, because the
speech engine (whisper.cpp) is compiled from source.

The UI test needs Playwright (`npm install --no-save playwright`). A live
end-to-end test against a signed-in Claude CLI runs with
`cargo test --manifest-path src-tauri/Cargo.toml -- --ignored live_claude`.

The app also builds and runs on Linux for development, without the
Windows-only parts (screenshot, paste, modifier-only gestures).
`src-tauri/cross/build-windows.sh` cross-compiles a Windows test build from
Linux.
