// Records the README demos from the real interface (ui/), driven by the
// same kind of fake app bridge the UI test uses. The real notch is hidden
// from screen capture by design, so it cannot be filmed directly.
//
//   node scripts/record-demo.mjs [out-dir]     (default docs/media)
//
// Needs Playwright (dev dependency) and ffmpeg on PATH. Writes demo.gif
// (it in action) and how-to.gif (how to use it).
import { createRequire } from "node:module";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const { chromium } = createRequire(import.meta.url)("playwright");
const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const outDir = path.resolve(process.argv[2] || path.join(root, "docs/media"));
fs.mkdirSync(outDir, { recursive: true });
const SIZE = { width: 1280, height: 720 };

// The app side, faked: settings, status, and asks that stream an answer.
const fake = () => {
  const listeners = {};
  let turn = 0;
  let settings = {
    wake_mode: "ctrl_alt", shortcut: "Ctrl+Alt+Space", provider: "claude", claude_model: "haiku", codex_model: "",
    include_screen: true, hide_from_capture: true, hide_on_blur: false, autostart: false, claude_path: "", codex_path: "",
    keep_warm_minutes: 5, voice_model: "base.en", dock_edge: "top", dock_along: 0.5, opacity: 1, edge_glow: true,
    live_shortcut: "Ctrl+Alt+L", live_system_audio: true, auto_update: true,
  };
  const emit = (name, payload) => (listeners[name] || []).forEach((fn) => fn({ payload }));
  window.__demo = { emit, nextTurn: () => ++turn, typedAnswer: "" };
  window.__TAURI__ = {
    core: {
      invoke: async (cmd, args) => {
        if (cmd === "get_settings") return settings;
        if (cmd === "save_settings") { settings = args.settings; return { settings, warning: null }; }
        if (cmd === "status") return { provider: "claude", cli_found: true, detail: "", hook_supported: true, voice_model_ready: true };
        if (cmd === "ask") {
          const id = ++turn;
          const words = window.__demo.typedAnswer.split(/(?<=\s)/);
          (async () => {
            await new Promise((r) => setTimeout(r, 700));
            for (const w of words) { emit("ai", { turn: id, kind: "delta", text: w }); await new Promise((r) => setTimeout(r, 28)); }
            emit("ai", { turn: id, kind: "done", text: "" });
          })();
          return id;
        }
        return null;
      },
    },
    event: { listen: async (name, fn) => { (listeners[name] ||= []).push(fn); return () => {}; } },
  };
};

// A stand-in desktop behind the transparent notch, and a caption for keys.
const stage = () => document.addEventListener("DOMContentLoaded", () => {
  const css = document.createElement("style");
  css.textContent = `
    html { background: radial-gradient(1200px 700px at 70% 10%, #2b3550 0%, #151a26 55%, #0d1018 100%) !important; }
    #shell { margin: 0 auto; position: relative; z-index: 2; }
    .demo-window { position: fixed; left: 50%; top: 250px; transform: translateX(-50%); width: 860px; height: 390px; z-index: 1;
      background: #f7f7f8; border-radius: 10px; box-shadow: 0 30px 80px rgba(0,0,0,.45); overflow: hidden;
      font: 14px/1.5 "Segoe UI Variable", "Segoe UI", system-ui, sans-serif; color: #1d1f24; }
    .demo-window header { height: 38px; background: #e9eaee; display: flex; align-items: center; padding: 0 14px; gap: 8px; color: #555; font-size: 12.5px; }
    .demo-window header b { color: #222; font-weight: 600; }
    .demo-window main { padding: 22px 28px; }
    .demo-window h2 { margin: 0 0 6px; font-size: 19px; }
    .demo-window .meta { color: #6b6f78; font-size: 12.5px; margin-bottom: 16px; }
    .demo-window p { margin: 0 0 10px; max-width: 680px; }
    .demo-window textarea { width: 100%; height: 120px; margin-top: 8px; border: 1px solid #d6d8de; border-radius: 6px; padding: 10px; font: inherit; resize: none; background: #fff; }
    .demo-caption { position: fixed; left: 50%; bottom: 26px; transform: translateX(-50%); z-index: 3; padding: 10px 18px; border-radius: 999px;
      background: rgba(255,255,255,.12); backdrop-filter: blur(8px); color: #f2f3f7; font: 600 15px "Segoe UI Variable", "Segoe UI", system-ui, sans-serif;
      opacity: 0; transition: opacity .3s; white-space: nowrap; }
    .demo-caption.on { opacity: 1; }
    .demo-caption kbd { display: inline-block; min-width: 22px; padding: 2px 8px; margin: 0 2px; border-radius: 6px; background: #f2f3f7; color: #111; font: 600 13px "Segoe UI", sans-serif; text-align: center; }`;
  document.head.append(css);
  const win = document.createElement("div");
  win.className = "demo-window";
  win.innerHTML = `<header><b>Mail</b> · Inbox</header><main>
    <h2>Launch checklist for Friday?</h2>
    <div class="meta">From Priya · to you · 9:14 AM</div>
    <p>Hi! Can you confirm we're still on track for Friday's launch? I need to know whether the pricing page and the screenshots are ready, and if anything is blocking us.</p>
    <p>Thanks, Priya</p>
    <textarea id="reply" placeholder="Write a reply…"></textarea></main>`;
  document.body.append(win);
  const cap = document.createElement("div");
  cap.className = "demo-caption";
  document.body.append(cap);
});

async function record(name, scene) {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "agent-desk-demo-"));
  const browser = await chromium.launch();
  const context = await browser.newContext({ viewport: SIZE, recordVideo: { dir: tmp, size: SIZE } });
  const page = await context.newPage();
  await page.addInitScript(fake);
  await page.addInitScript(stage);
  await page.goto("file://" + path.join(root, "ui/index.html"));
  await page.waitForTimeout(600);
  const t = {
    page,
    wait: (ms) => page.waitForTimeout(ms),
    emit: (n, p) => page.evaluate(([n, p]) => window.__demo.emit(n, p), [n, p]),
    caption: (html) => page.evaluate((h) => {
      const c = document.querySelector(".demo-caption");
      c.innerHTML = h || "";
      c.classList.toggle("on", !!h);
    }, html),
    wake: (typing) => page.evaluate((typing) => window.__demo.emit("wake", { title: "Inbox - Mail", process: "olk.exe", has_screen: true, typing }), typing),
    turn: () => page.evaluate(() => window.__demo.nextTurn()),
    async stream(turn, text, ms = 26) {
      for (const w of text.split(/(?<=\s)/)) { await this.emit("ai", { turn, kind: "delta", text: w }); await page.waitForTimeout(ms); }
      await this.emit("ai", { turn, kind: "done", text: "" });
    },
    async talk(seconds) {
      const end = Date.now() + seconds * 1000;
      while (Date.now() < end) { await this.emit("level", 0.25 + Math.random() * 0.75); await page.waitForTimeout(70); }
    },
  };
  await scene(t);
  const video = page.video();
  await context.close();
  await browser.close();
  const webm = await video.path();
  const gif = path.join(outDir, name + ".gif");
  // Two-pass palette keeps text crisp and the file small.
  const filters = "fps=14,scale=1024:-1:flags=lanczos";
  execFileSync("ffmpeg", ["-y", "-loglevel", "error", "-i", webm, "-vf", `${filters},palettegen=stats_mode=diff`, path.join(tmp, "p.png")]);
  execFileSync("ffmpeg", ["-y", "-loglevel", "error", "-i", webm, "-i", path.join(tmp, "p.png"), "-lavfi", `${filters} [x]; [x][1:v] paletteuse=dither=bayer:bayer_scale=4:diff_mode=rectangle`, gif]);
  fs.copyFileSync(webm, path.join(tmp, name + ".webm"));
  console.log(`${path.relative(root, gif)}  ${(fs.statSync(gif).size / 1e6).toFixed(1)} MB`);
  fs.rmSync(tmp, { recursive: true, force: true });
}

const REPLY = "Here's a reply you can send:\n\nHi Priya, yes, we're **on track for Friday**. The pricing page is done and live on staging. Screenshots are the one open item; I'll have them by Thursday noon. Nothing else is blocking us.\n\nWant it shorter or more formal?";

// 1. In action: hold Ctrl + Alt, ask about what is on screen, get an answer.
await record("demo", async (t) => {
  await t.wait(500);
  await t.caption("Hold <kbd>Ctrl</kbd> + <kbd>Alt</kbd> and speak");
  await t.wait(700);
  await t.wake(false);
  await t.emit("phase", { phase: "listening", text: "" });
  await t.talk(2.4);
  await t.caption("Let go");
  await t.emit("phase", { phase: "transcribing", text: "" });
  await t.wait(900);
  const id = await t.turn();
  await t.emit("asked", { turn: id, text: "draft a reply to this email" });
  await t.caption("");
  await t.wait(900);
  await t.stream(id, REPLY);
  await t.wait(900);
  await t.caption("<kbd>Ctrl</kbd> + <kbd>Enter</kbd> pastes it into the app you were in");
  await t.wait(2600);
});

// 2. How to use it: type instead of talking, settings, and live mode.
await record("how-to", async (t) => {
  await t.wait(400);
  await t.caption("Tap <kbd>Ctrl</kbd> + <kbd>Alt</kbd> to type instead");
  await t.wait(900);
  await t.wake(true);
  await t.wait(700);
  await t.page.evaluate((a) => (window.__demo.typedAnswer = a), "Use `reversed()` for a new iterator or `items[::-1]` for a new list:\n\n```python\nitems = [1, 2, 3]\nprint(items[::-1])  # [3, 2, 1]\n```\n\n`items.reverse()` flips the list in place.");
  await t.page.type("#prompt", "how do I reverse a list in python?", { delay: 45 });
  await t.wait(300);
  await t.caption("<kbd>Enter</kbd> to send");
  await t.page.keyboard.press("Enter");
  await t.wait(4200);
  await t.caption("Settings open inside the notch: <kbd>Ctrl</kbd> + <kbd>,</kbd>");
  await t.page.keyboard.press("Control+,");
  await t.wait(2600);
  await t.page.keyboard.press("Escape");
  await t.wait(500);
  await t.caption("<kbd>Ctrl</kbd> + <kbd>Alt</kbd> + <kbd>L</kbd> works with you during a call");
  await t.emit("live", true);
  await t.wait(1400);
  await t.emit("live-heard", { who: "Them", text: "how would you find duplicates in a big list?" });
  await t.wait(900);
  const id = await t.turn();
  await t.emit("asked", { turn: id, text: "Them: how would you find duplicates in a big list?", quiet: true });
  await t.wait(500);
  await t.stream(id, "Suggest a set: walk the list once and keep what you've seen.\n\n```python\nseen, dupes = set(), set()\nfor x in items:\n    (dupes if x in seen else seen).add(x)\n```\n\nIt's O(n) time, unlike sorting.");
  await t.wait(2200);
  await t.caption("<kbd>Esc</kbd> hides it. It stays out of screen shares.");
  await t.wait(2200);
});
