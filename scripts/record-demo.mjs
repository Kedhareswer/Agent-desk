// Records the README demos from the real interface (ui/), driven by the
// same kind of fake app bridge the UI test uses. The real notch, its glow
// and its pointer are hidden from screen capture by design, so they cannot
// be filmed directly; the glow and the pointer are drawn here the way
// ui/glow.html and ui/pointer.html draw them.
//
//   node scripts/record-demo.mjs [out-dir]     (default docs/media)
//
// Needs Playwright (dev dependency) and ffmpeg on PATH. Writes:
//   demo.gif    talk, and the answer appears (the top of the README)
//   act.gif     it works on the computer: one step line, its own pointer
//   how-to.gif  type, earlier answers, pin and read aloud, settings, live
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
    live_shortcut: "Ctrl+Alt+L", live_system_audio: true, auto_update: true, actions_enabled: true,
    always_allow: [], commands: [], browser_profiles: {},
  };
  const emit = (name, payload) => (listeners[name] || []).forEach((fn) => fn({ payload }));
  window.__demo = { emit, nextTurn: () => ++turn, typedAnswer: "" };
  window.__TAURI__ = {
    core: {
      invoke: async (cmd, args) => {
        if (cmd === "get_settings") return settings;
        if (cmd === "save_settings") { settings = args.settings; return { settings, warning: null }; }
        if (cmd === "status") return { provider: "claude", cli_found: true, detail: "", hook_supported: true, voice_model_ready: true };
        if (cmd === "list_schedules") return [];
        if (cmd === "take_recovery") return null;
        if (cmd === "ask") {
          const id = ++turn;
          const words = window.__demo.typedAnswer.split(/(?<=\s)/);
          (async () => {
            await new Promise((r) => setTimeout(r, 700));
            for (const w of words) { emit("ai", { turn: id, kind: "delta", text: w }); await new Promise((r) => setTimeout(r, 24)); }
            emit("ai", { turn: id, kind: "done", text: "" });
          })();
          return id;
        }
        return null;
      },
    },
    event: { listen: async (name, fn) => { (listeners[name] ||= []).push(fn); return () => {}; } },
  };
  // The notch stays open while the demo runs; the README explains tucking.
  window.__tuckMs = 1e9;
  // Earlier answers, for the history scene. Kept on the device in the app.
  const now = Date.now();
  localStorage.setItem("agent-desk.history", JSON.stringify([
    { q: "what's the time in Tokyo right now", a: "It's **9:14 PM** in Tokyo (JST, UTC+9).", at: now - 9 * 60e3, steps: 0 },
    { q: "summarise this PDF in three points", a: "1. Revenue grew 18% year on year.\n2. Costs fell after the move to the new warehouse.\n3. The team plans two launches in Q3.", at: now - 42 * 60e3, steps: 1 },
    { q: "open my calendar and tell me what's next", a: "Next up: **Design review** at 3:30 PM with Priya and Sam.", at: now - 3 * 3600e3, steps: 2 },
  ]));
};

// A stand-in desktop behind the transparent notch, Agent Desk's glow and
// pointer, and a caption for keys.
const stage = () => document.addEventListener("DOMContentLoaded", () => {
  const css = document.createElement("style");
  css.textContent = `
    /* Nearly flat: a GIF has few colours, and a soft gradient would band. */
    html { background: #161b27 !important; }
    #shell { margin: 0 auto; position: relative; z-index: 4; }
    .demo-window { position: fixed; left: 50%; top: 272px; transform: translateX(-50%); width: 900px; height: 384px; z-index: 1;
      background: #f7f7f8; border-radius: 10px; box-shadow: 0 30px 80px rgba(0,0,0,.45); overflow: hidden;
      font: 14px/1.5 "Segoe UI Variable", "Segoe UI", system-ui, sans-serif; color: #1d1f24; }
    .demo-window header { height: 38px; background: #e9eaee; display: flex; align-items: center; padding: 0 14px; gap: 8px; color: #555; font-size: 12.5px; }
    .demo-window header b { color: #222; font-weight: 600; }
    .demo-window header .url { margin-left: 10px; flex: 1; max-width: 520px; background: #fff; border-radius: 6px; padding: 3px 10px; color: #333; }
    .demo-window main { padding: 22px 28px; transition: transform .6s cubic-bezier(.22,1,.36,1); }
    .demo-window h2 { margin: 0 0 6px; font-size: 19px; }
    .demo-window .meta { color: #6b6f78; font-size: 12.5px; margin-bottom: 16px; }
    .demo-window p { margin: 0 0 10px; max-width: 700px; }
    .demo-window textarea { width: 100%; height: 120px; margin-top: 8px; border: 1px solid #d6d8de; border-radius: 6px; padding: 10px; font: inherit; resize: none; background: #fff; }
    .repo { display: flex; justify-content: space-between; padding: 12px 0; border-bottom: 1px solid #e3e4e8; }
    .repo b { color: #2457c5; } .repo span { color: #6b6f78; font-size: 13px; }
    .tabs { display: flex; gap: 18px; border-bottom: 1px solid #e3e4e8; margin: 4px 0 8px; padding-bottom: 8px; color: #555; }
    .tabs .on { color: #111; font-weight: 600; }
    .demo-glow { position: fixed; inset: 0; z-index: 3; pointer-events: none; opacity: 0; transition: opacity .35s ease;
      box-shadow: inset 0 0 3px 1px rgba(170,245,210,.9), inset 0 0 18px 4px rgba(90,220,170,.5), inset 0 0 44px 8px rgba(60,200,190,.22); }
    .demo-glow.listening { box-shadow: inset 0 0 3px 1px rgba(150,215,255,.85), inset 0 0 18px 4px rgba(60,150,255,.5), inset 0 0 44px 8px rgba(110,90,255,.22); }
    .demo-glow.on { opacity: 1; }
    .demo-pointer { position: fixed; left: 0; top: 0; z-index: 5; display: flex; align-items: flex-start; gap: 1px; opacity: 0; transition: opacity .2s; pointer-events: none; }
    .demo-pointer.on { opacity: 1; }
    .demo-pointer svg { width: 20px; height: 20px; flex: none; filter: drop-shadow(0 1px 2px rgba(0,0,0,.55)); }
    .demo-pointer span { margin-top: 13px; padding: 2px 8px; border-radius: 999px; white-space: nowrap; font: 600 11.5px/1.5 "Segoe UI Variable Text", "Segoe UI", sans-serif; color: #0b0c0f; background: #9fe3c4; box-shadow: 0 1px 4px rgba(0,0,0,.45); }
    .demo-caption { position: fixed; left: 50%; bottom: 22px; transform: translateX(-50%); z-index: 6; padding: 10px 18px; border-radius: 999px;
      background: rgba(20,24,34,.82); border: 1px solid rgba(255,255,255,.14); color: #f2f3f7; font: 600 15px "Segoe UI Variable", "Segoe UI", system-ui, sans-serif;
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
  const glow = document.createElement("div");
  glow.className = "demo-glow";
  document.body.append(glow);
  const pointer = document.createElement("div");
  pointer.className = "demo-pointer";
  pointer.innerHTML = `<svg viewBox="0 0 20 20"><path d="M0.8 0.8 L0.8 15.5 L4.9 11.9 L7.7 18.3 L10.4 17.1 L7.7 10.9 L13.2 10.9 Z" fill="#9fe3c4" stroke="#0b0c0f" stroke-width="1.3" stroke-linejoin="round"/></svg><span>Agent Desk</span>`;
  document.body.append(pointer);
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
    glow: (mode) => page.evaluate((m) => {
      const g = document.querySelector(".demo-glow");
      g.classList.toggle("listening", m === "listening");
      g.classList.toggle("on", !!m);
    }, mode),
    desktop: (html) => page.evaluate((h) => (document.querySelector(".demo-window").innerHTML = h), html),
    scrollPage: (px) => page.evaluate((y) => (document.querySelector(".demo-window main").style.transform = `translateY(${-y}px)`), px),
    // Agent Desk's pointer glides from where it is (or the user's cursor).
    async glide(x, y) {
      await page.evaluate(async ([x, y]) => {
        const p = document.querySelector(".demo-pointer");
        const from = p.classList.contains("on") ? [parseFloat(p.dataset.x), parseFloat(p.dataset.y)] : [x - 260, y + 160];
        p.classList.add("on");
        const steps = 30;
        for (let i = 1; i <= steps; i++) {
          const k = 1 - (1 - i / steps) ** 3;
          const px = from[0] + (x - from[0]) * k, py = from[1] + (y - from[1]) * k;
          p.style.transform = `translate(${px}px, ${py}px)`;
          await new Promise((r) => setTimeout(r, 14));
        }
        p.dataset.x = x;
        p.dataset.y = y;
      }, [x, y]);
    },
    // Glide to an element on the stand-in desktop, tip on its middle.
    async glideTo(selector) {
      const [x, y] = await page.evaluate((s) => { const r = document.querySelector(s).getBoundingClientRect(); return [r.left + r.width / 2, r.top + r.height / 2]; }, selector);
      await this.glide(x + 1, y + 1);
    },
    hidePointer: () => page.evaluate(() => document.querySelector(".demo-pointer").classList.remove("on")),
    wake: (typing, title = "Inbox - Mail") => page.evaluate(([typing, title]) => window.__demo.emit("wake", { title, process: "olk.exe", has_screen: true, typing }), [typing, title]),
    turn: () => page.evaluate(() => window.__demo.nextTurn()),
    async stream(turn, text, ms = 22) {
      for (const w of text.split(/(?<=\s)/)) { await this.emit("ai", { turn, kind: "delta", text: w }); await page.waitForTimeout(ms); }
      await this.emit("ai", { turn, kind: "done", text: "" });
    },
    async listen(seconds) {
      await this.emit("phase", { phase: "listening", text: "" });
      await this.glow("listening");
      await page.waitForTimeout(seconds * 1000);
      await this.glow(null);
      await this.emit("phase", { phase: "transcribing", text: "" });
    },
  };
  await scene(t);
  const video = page.video();
  await context.close();
  await browser.close();
  const webm = await video.path();
  const gif = path.join(outDir, name + ".gif");
  // Full size and 15 frames a second; a two-pass palette made from the
  // whole clip keeps text crisp, and only changed areas are stored.
  const filters = "fps=15,scale=1280:-1:flags=lanczos";
  execFileSync("ffmpeg", ["-y", "-loglevel", "error", "-i", webm, "-vf", `${filters},palettegen=max_colors=128:stats_mode=diff`, path.join(tmp, "p.png")]);
  execFileSync("ffmpeg", ["-y", "-loglevel", "error", "-i", webm, "-i", path.join(tmp, "p.png"), "-lavfi", `${filters} [x]; [x][1:v] paletteuse=dither=sierra2_4a:diff_mode=rectangle`, gif]);
  console.log(`${path.relative(root, gif)}  ${(fs.statSync(gif).size / 1e6).toFixed(1)} MB`);
  fs.rmSync(tmp, { recursive: true, force: true });
}

const REPLY = "Here's a reply you can send:\n\nHi Priya, yes, we're **on track for Friday**. The pricing page is done and live on staging. Screenshots are the one open item; I'll have them by Thursday noon. Nothing else is blocking us.\n\nWant it shorter or more formal?";

const PROFILE = `<header><b>Chrome</b><span class="url">github.com/your-name?tab=repositories</span></header><main>
  <h2>your-name</h2><div class="meta">12 repositories · 4 organisations</div>
  <div class="tabs"><span>Overview</span><span class="on" id="repos-tab">Repositories</span><span>Projects</span><span>Stars</span></div>
  <div class="repo"><b>agent-desk</b><span>Rust · updated today · 214 commits by you</span></div>
  <div class="repo"><b>notes-sync</b><span>TypeScript · updated 3 days ago · 88 commits by you</span></div>
  <div class="repo"><b>pixel-board</b><span>JavaScript · updated last week · 51 commits by you</span></div>
  <div class="repo"><b>team-wiki</b><span>Markdown · updated last month · 12 commits by you</span></div>
  <div class="repo"><b>infra-scripts</b><span>Shell · updated in March · 7 commits by you</span></div>
  <div class="repo"><b>design-tokens</b><span>CSS · updated in February · 3 commits by you</span></div>
  <div class="repo"><b>old-site</b><span>HTML · updated last year · 1 commit by you</span></div></main>`;

// 1. In action: hold Ctrl + Alt, ask about what is on screen, get an answer.
await record("demo", async (t) => {
  await t.wait(500);
  await t.caption("Hold <kbd>Ctrl</kbd> + <kbd>Alt</kbd> and speak");
  await t.wait(800);
  await t.wake(false);
  await t.listen(2.4);
  await t.caption("Let go");
  await t.wait(900);
  const id = await t.turn();
  await t.emit("asked", { turn: id, text: "draft a reply to this email" });
  await t.caption("");
  await t.wait(800);
  await t.stream(id, REPLY);
  await t.wait(900);
  await t.caption("<b>Insert</b> pastes it into the app you were in");
  await t.emit("hover", true);
  await t.wait(2800);
});

// 2. Acting on the computer: one step line, its own pointer, the glow.
const NEW_TAB = `<header><b>Chrome</b><span class="url">New Tab</span></header><main style="display:grid;place-items:center;height:300px;color:#8a8f99">
  <div style="width:420px;height:40px;border-radius:20px;background:#fff;border:1px solid #dcdde2"></div></main>`;

await record("act", async (t) => {
  await t.desktop(NEW_TAB);
  await t.wait(400);
  await t.caption("“Check if I'm the top contributor on my GitHub repos”");
  await t.wake(false, "New Tab - Google Chrome");
  await t.listen(2.2);
  await t.wait(700);
  const id = await t.turn();
  await t.emit("asked", { turn: id, text: "check if I'm the top contributor on my GitHub repos" });
  await t.wait(700);
  const step = (n, status, summary) => t.emit("action", { id: n, turn: id, kind: "x", summary, detail: "", status, result: "" });
  await t.caption("Each step shows on one line while it works");
  await t.glow("acting");
  await step(1, "running", "Open github.com in Chrome");
  await t.wait(900);
  await t.desktop(PROFILE);
  await step(1, "done", "Open github.com in Chrome");
  await t.wait(500);
  await step(2, "running", "Click “Repositories”");
  await t.caption("Its own pointer does the clicking. Your mouse stays yours.");
  await t.glideTo("#repos-tab");
  await t.wait(700);
  await step(2, "done", "Click “Repositories”");
  await step(3, "running", "Scroll down");
  await t.glide(640, 560);
  await t.scrollPage(150);
  await t.wait(1100);
  await step(3, "done", "Scroll down");
  await step(4, "running", "Read the contributors on 7 repositories");
  await t.wait(1400);
  await step(4, "done", "Read the contributors on 7 repositories");
  await t.caption("");
  await t.stream(id, "You're the **top contributor on 5 of your 7** repositories. On **team-wiki** and **old-site** someone else has more commits.");
  await t.glow(null);
  await t.hidePointer();
  await t.wait(2600);
});

// 3. How to use it: type, earlier answers, pin and read aloud, settings, live.
await record("how-to", async (t) => {
  await t.wait(400);
  await t.caption("Tap <kbd>Ctrl</kbd> + <kbd>Alt</kbd> to type instead");
  await t.wait(800);
  await t.wake(true);
  await t.wait(600);
  await t.page.evaluate((a) => (window.__demo.typedAnswer = a), "Use `reversed()` for a new iterator or `items[::-1]` for a new list:\n\n```python\nitems = [1, 2, 3]\nprint(items[::-1])  # [3, 2, 1]\n```\n\n`items.reverse()` flips the list in place.");
  await t.page.type("#prompt", "how do I reverse a list in python?", { delay: 40 });
  await t.caption("<kbd>Enter</kbd> to send");
  await t.page.keyboard.press("Enter");
  await t.wait(3800);
  await t.caption("Pin it open, or have it read aloud");
  await t.emit("hover", true);
  await t.page.hover("#pin");
  await t.wait(500);
  await t.page.click("#pin");
  await t.wait(1600);
  await t.caption("Earlier answers: <kbd>Ctrl</kbd> + <kbd>H</kbd>, with search");
  await t.page.keyboard.press("Control+h");
  await t.wait(1300);
  await t.page.type("#history-search", "tokyo", { delay: 90 });
  await t.wait(1600);
  await t.page.keyboard.press("Escape");
  await t.wait(400);
  await t.caption("Settings open inside the notch: <kbd>Ctrl</kbd> + <kbd>,</kbd>");
  await t.page.keyboard.press("Control+,");
  await t.wait(2400);
  await t.page.keyboard.press("Escape");
  await t.emit("hover", false);
  await t.wait(400);
  await t.caption("<kbd>Ctrl</kbd> + <kbd>Alt</kbd> + <kbd>L</kbd> works with you during a call");
  await t.emit("live", true);
  await t.wait(1200);
  await t.emit("live-heard", { who: "Them", text: "how would you find duplicates in a big list?" });
  await t.wait(800);
  const id = await t.turn();
  await t.emit("asked", { turn: id, text: "Them: how would you find duplicates in a big list?", quiet: true });
  await t.wait(400);
  await t.stream(id, "Suggest a set: walk the list once and keep what you've seen.\n\n```python\nseen, dupes = set(), set()\nfor x in items:\n    (dupes if x in seen else seen).add(x)\n```\n\nIt's O(n) time, unlike sorting.");
  await t.wait(1800);
  await t.caption("<kbd>Esc</kbd> hides it. It stays out of screen shares.");
  await t.wait(2000);
});
