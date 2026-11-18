// Drives the overlay UI in a real browser with the Rust side replaced by a
// small fake, so the interface can be tested on any machine.
//   node tests/ui.test.mjs [screenshot-dir]
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import path from "node:path";
import assert from "node:assert/strict";

// Playwright is resolved with require so a global install (NODE_PATH) works.
const { chromium } = createRequire(import.meta.url)("playwright");

const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const shots = process.argv[2];

const fake = () => {
  const listeners = {};
  const calls = [];
  let turn = 0;
  let settings = {
    wake_mode: "ctrl_alt", shortcut: "Ctrl+Alt+Space", provider: "claude", claude_model: "haiku", codex_model: "",
    include_screen: true, hide_from_capture: true, hide_on_blur: false, autostart: false,
    claude_path: "", codex_path: "", keep_warm_minutes: 5,
    voice_model: "base.en", dock_edge: "top", dock_along: 0.5, opacity: 0.92, edge_glow: true,
    live_shortcut: "Ctrl+Alt+L", live_system_audio: true, auto_update: true,
  };
  const emit = (name, payload) => (listeners[name] || []).forEach((fn) => fn({ payload }));
  window.__fake = { calls, emit, cliFound: true, failAsk: null, nextTurn: () => ++turn };
  // The real page can load before the app has finished starting, when every
  // command fails. Start the same way.
  let starting = 3;
  window.__TAURI__ = {
    core: {
      invoke: async (cmd, args) => {
        calls.push([cmd, args]);
        if (starting > 0) { starting -= 1; throw "state not managed"; }
        if (cmd === "get_settings") return settings;
        if (cmd === "save_settings") { settings = args.settings; return { settings, warning: null }; }
        if (cmd === "status") return { provider: settings.provider, cli_found: window.__fake.cliFound, detail: window.__fake.cliFound ? "C:/claude.exe" : "The `claude` command was not found.", hook_supported: true, voice_model_ready: true };
        if (cmd === "ask") {
          if (window.__fake.failAsk) throw window.__fake.failAsk;
          turn += 1;
          // The first piece arrives before the caller learns the turn id.
          emit("ai", { turn, kind: "delta", text: "Hi Ben," });
          return turn;
        }
        return null;
      },
    },
    event: { listen: async (name, fn) => { (listeners[name] ||= []).push(fn); return () => {}; } },
  };
};

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 720, height: 700 }, deviceScaleFactor: shots ? 2 : 1 });
const errors = [];
page.on("pageerror", (e) => errors.push(e.message));
await page.addInitScript(fake);
// A stand-in desktop behind the transparent overlay, for screenshots.
await page.addInitScript(() => document.addEventListener("DOMContentLoaded", () => {
  document.documentElement.style.background = "linear-gradient(135deg, #d9d4c7, #b9c4d6)";
  Object.assign(document.getElementById("shell").style, { margin: "0 auto" });
}));
await page.goto("file://" + path.join(root, "ui/index.html"));
// Screenshots wait for the entrance animations to settle.
const snap = async (name) => shots && (await page.waitForTimeout(700), page.screenshot({ path: path.join(shots, name + ".png") }));
const calls = (cmd) => page.evaluate((c) => window.__fake.calls.filter((x) => x[0] === c).map((x) => x[1]), cmd);
const emit = (name, payload) => page.evaluate(([n, p]) => window.__fake.emit(n, p), [name, payload]);
const notch = (attr) => page.getAttribute("#notch", "data-" + attr);
const wake = (typing) => emit("wake", { title: "Inbox - Gmail", process: "chrome.exe", has_screen: true, typing });

/* ----- start-up before the app is ready ----- */

// Commands failed while the app was starting; the page keeps asking.
// Start-up has finished once the page asks for the status (the last step).
await page.waitForFunction(() => window.__fake.calls.some((c) => c[0] === "status"));
assert.ok((await calls("get_settings")).length >= 2, "settings were asked for again after failing");
{
  const all = await page.evaluate(() => window.__fake.calls.map((c) => c[0]));
  const loaded = all.lastIndexOf("get_settings");
  assert.ok(all.indexOf("resize", loaded) > loaded, "the window size is sent again once the app is ready");
}

/* ----- voice: hold, speak, release ----- */

await wake(false);
await emit("phase", { phase: "listening", text: "" });
assert.equal(await notch("state"), "listening");
assert.equal(await page.textContent("#label"), "Listening");
assert.equal(await page.textContent("#top-line"), "Seeing Inbox - Gmail");
assert.ok(await page.isVisible("#meter"));
assert.ok(await page.isHidden("#panel"), "nothing but the notch while listening");
await page.waitForTimeout(600);
const small = await page.locator("#notch").boundingBox();
assert.ok(small.height < 70 && small.width < 330, "collapsed, it is just a notch");
for (const level of [0.1, 0.5, 0.9, 1, 0.6, 0.2, 0.8, 0.4]) await emit("level", level);
assert.ok((await page.locator("#meter i.lit").count()) > 5);
await snap("1-listening");

await emit("phase", { phase: "transcribing", text: "" });
assert.equal(await page.textContent("#label"), "Transcribing");
assert.ok(await page.isHidden("#meter"));

// The answer starts before the question is announced; nothing may be lost.
const voiceTurn = await page.evaluate(() => window.__fake.nextTurn());
await emit("ai", { turn: voiceTurn, kind: "delta", text: "Sure. " });
await emit("asked", { turn: voiceTurn, text: "draft a reply to this email" });
assert.equal(await page.textContent(".msg.user"), "draft a reply to this email");
assert.deepEqual(await calls("ask"), [], "speech is sent by the app itself, not by the interface");
await emit("ai", { turn: voiceTurn, kind: "delta", text: "Yes, we're **on track** for Friday.\n\n- Pages are done\n- Waiting on `photos`\n\n```js\nconst x = 1 < 2;\n```" });
await page.waitForFunction(() => document.querySelector(".msg.ai strong"));
assert.equal(await notch("state"), "answering");
assert.match(await page.textContent(".msg.ai"), /^Sure\. Yes/);
assert.ok(await page.isVisible("#stop"));
assert.ok(await page.isHidden("#actions"));
await snap("2-answering");
await emit("ai", { turn: 99, kind: "delta", text: "STALE" });
await emit("ai", { turn: voiceTurn, kind: "done", text: "" });
await page.waitForSelector("#actions:not([hidden])");
assert.equal(await notch("state"), "done");
assert.doesNotMatch(await page.textContent(".msg.ai"), /STALE/);
// The answer is inside the notch itself, which has grown to hold it.
assert.ok(await page.evaluate(() => document.getElementById("notch").contains(document.getElementById("thread"))));
await page.waitForTimeout(600);
const big = await page.locator("#notch").boundingBox();
const answer = await page.locator("#thread").boundingBox();
assert.ok(big.width >= 600 && big.height > 200, "the notch grew");
assert.ok(answer.y >= big.y && answer.y + answer.height <= big.y + big.height + 1, "the answer sits inside the notch");
assert.equal(big.y, small.y, "it stays attached to the screen edge");
assert.match(await page.textContent("#stat"), /^Claude Haiku · \d+\.\ds$/);
assert.equal(await page.textContent(".msg.ai pre code"), "const x = 1 < 2;");
assert.equal(await page.locator(".msg.ai li").count(), 2);
await snap("3-done");

// The global insert key pastes the raw answer, not HTML.
await emit("insert-request", null);
const inserted = (await calls("insert_text"))[0].text;
assert.ok(inserted.startsWith("Sure. Yes") && inserted.includes("**on track**"));
await page.click(".copy-code");
assert.deepEqual((await calls("copy_text")).at(-1), { text: "const x = 1 < 2;" });
await page.click("#copy");
assert.equal((await calls("copy_text")).length, 2);

// Nothing heard: say so, and do not ask anything.
await emit("phase", { phase: "listening", text: "" });
await emit("phase", { phase: "error", text: "I didn't hear anything. Hold the keys while you speak." });
assert.equal(await notch("state"), "error");
assert.match(await page.textContent("#notice"), /didn't hear/);

// Ctrl+Alt+something is an ordinary shortcut: go back to the answer.
await emit("phase", { phase: "listening", text: "" });
await emit("phase", { phase: "cancelled", text: "" });
assert.equal(await notch("state"), "done");

// The speech model downloads on first use.
await emit("phase", { phase: "preparing", text: "Downloading speech model 34%" });
assert.equal(await page.textContent("#top-line"), "Downloading speech model 34%");

/* ----- typing: tap to open ----- */

await wake(true);
assert.equal(await notch("view"), "input");
await page.fill("#prompt", "make it shorter");
await page.keyboard.press("Enter");
assert.deepEqual((await calls("ask")).at(-1), { prompt: "make it shorter", withScreen: true });
await page.waitForFunction(() => document.querySelectorAll(".msg.ai").length === 2);
assert.equal(await notch("state"), "answering");
// The piece sent before the turn id was known was kept.
await page.waitForFunction(() => document.querySelectorAll(".msg.ai")[1].textContent.startsWith("Hi Ben,"));

// Model output is never treated as HTML.
const typedTurn = voiceTurn + 1;
await emit("ai", { turn: typedTurn, kind: "delta", text: ' <img src=x onerror="window.__pwned=1"> ok' });
await emit("ai", { turn: typedTurn, kind: "done", text: "" });
await page.waitForFunction(() => !document.querySelector(".streaming"));
assert.equal(await page.evaluate(() => window.__pwned), undefined);
assert.equal(await page.locator(".msg.ai img").count(), 0);

// Clicking the status line switches to typing and asks for keyboard focus.
assert.equal(await notch("view"), "status");
await page.click("#label");
assert.equal(await notch("view"), "input");
assert.ok((await calls("focus")).length >= 1);

// An empty Enter still asks, and the follow-up carries no second screenshot.
await page.keyboard.press("Enter");
assert.deepEqual((await calls("ask")).at(-1), { prompt: "", withScreen: false });
// Escape while answering stops; Escape again hides.
await page.keyboard.press("Escape");
assert.equal((await calls("stop")).length, 1);
assert.ok(await page.isHidden("#stop"));
await page.keyboard.press("Escape");
assert.equal((await calls("hide")).length, 1);

// After a stop the next question starts a clean conversation.
await page.fill("#prompt", "again");
await page.keyboard.press("Enter");
assert.equal(await page.locator(".msg.user").count(), 1);
await emit("ai", { turn: typedTurn + 2, kind: "error", text: "You are not signed in." });
await page.waitForSelector(".msg.error");
assert.equal(await notch("state"), "error");
assert.ok(await page.isHidden("#actions"), "a cut-off answer is not offered for Insert");
await snap("4-error");

// New chat clears everything and leaves only the notch.
await page.keyboard.press("Control+n");
assert.ok(await page.isHidden("#panel"));
assert.equal((await calls("new_chat")).length, 1);
await page.waitForTimeout(600);
assert.ok((await page.locator("#notch").boundingBox()).height < 70, "it shrinks back to a notch");
assert.ok((await page.locator("#prompt").boundingBox()).height > 15, "the text field is usable after a turn");
await snap("5-typing");

// A failed ask (for example, CLI missing) is shown, not swallowed.
await page.evaluate(() => (window.__fake.failAsk = "The `claude` command was not found."));
await page.fill("#prompt", "hello");
await page.keyboard.press("Enter");
await page.waitForSelector(".msg.error");
assert.match(await page.textContent(".msg.error"), /not found/);
await page.evaluate(() => (window.__fake.failAsk = null));
await page.keyboard.press("Control+n");

/* ----- live mode: "work with me" ----- */

await emit("live", true);
assert.equal(await notch("state"), "live");
assert.equal(await page.textContent("#label"), "Working with you");
assert.ok(await page.isVisible("#meter"));
assert.ok(await page.isHidden("#panel"), "only the notch until there is something to say");

// What is heard shows in the small line, so it is clear it is following.
await emit("live-heard", { who: "Them", text: "so how was everyone's weekend" });
assert.equal(await page.textContent("#top-line"), "Them: so how was everyone's weekend");

// Small talk: the AI answers "." and nothing appears.
let liveTurn = await page.evaluate(() => window.__fake.nextTurn());
await emit("asked", { turn: liveTurn, text: "Them: so how was everyone's weekend", quiet: true });
await emit("ai", { turn: liveTurn, kind: "delta", text: "." });
assert.ok(await page.isHidden("#panel"));
assert.equal(await notch("state"), "live");
await emit("ai", { turn: liveTurn, kind: "done", text: "" });
assert.ok(await page.isHidden("#panel"));
assert.equal(await page.locator(".msg").count(), 0, "no trace of a check-in with nothing to say");

// A real question: the notch grows with the answer, then keeps listening.
liveTurn = await page.evaluate(() => window.__fake.nextTurn());
await emit("live-heard", { who: "Them", text: "how would you do breadth-first search in Python?" });
await emit("asked", { turn: liveTurn, text: "Them: how would you do breadth-first search in Python?", quiet: true });
await emit("ai", { turn: liveTurn, kind: "delta", text: "Use a queue: visit a node, then enqueue its unvisited neighbours.\n\n```python\nfrom collections import deque\n\ndef bfs(graph, start):\n    seen, queue = {start}, deque([start])\n    while queue:\n        node = queue.popleft()\n        for n in graph[node]:\n            if n not in seen:\n                seen.add(n); queue.append(n)\n    return seen\n```" });
await page.waitForSelector(".msg.ai pre");
assert.equal(await notch("state"), "answering");
assert.equal(await page.textContent("#top-line"), "Them: how would you do breadth-first search in Python?");
await emit("ai", { turn: liveTurn, kind: "done", text: "" });
assert.equal(await notch("state"), "live", "back to listening after answering");
assert.ok(await page.isVisible("#actions"));
await snap("9-live");

// A later check-in with nothing to say leaves the last answer in place.
liveTurn = await page.evaluate(() => window.__fake.nextTurn());
await emit("asked", { turn: liveTurn, text: "Me: right, thanks", quiet: true });
await emit("ai", { turn: liveTurn, kind: "delta", text: "." });
await emit("ai", { turn: liveTurn, kind: "done", text: "" });
assert.equal(await page.locator(".msg.ai").count(), 1);
await emit("insert-request", null);
assert.match((await calls("insert_text")).at(-1).text, /^Use a queue/);

// "New" only clears the screen in live mode; the AI keeps its context.
const chatsBefore = (await calls("new_chat")).length;
await page.click("#new-chat");
assert.ok(await page.isHidden("#panel"));
assert.equal((await calls("new_chat")).length, chatsBefore);
assert.equal(await notch("state"), "live");

// Turning live mode off with nothing on screen hides the notch.
const hidesBefore = (await calls("hide")).length;
await emit("live", false);
assert.equal((await calls("hide")).length, hidesBefore + 1);
await wake(true);

/* ----- settings ----- */

await page.keyboard.press("Control+,");
await page.waitForSelector("#settings:not([hidden])");
await snap("6-settings");
await page.focus("#shortcut-input");
await page.keyboard.press("Control+Shift+KeyK");
assert.equal(await page.inputValue("#shortcut-input"), "Ctrl+Shift+K");
await page.selectOption('[name="provider"]', "codex");
assert.ok(await page.isHidden('[name="claude_model"]'));
assert.ok(await page.isVisible('[name="codex_model"]'));
await page.selectOption('[name="voice_model"]', "tiny.en");
await page.selectOption('[name="dock_edge"]', "bottom");
await page.uncheck('[name="hide_from_capture"]');
assert.ok(await page.isChecked('[name="auto_update"]'), "update checks are on by default");
await page.uncheck('[name="auto_update"]');
// The transparency slider previews live and is saved.
await page.fill('[name="opacity"]', "0.5");
assert.equal(await page.textContent("#opacity-out"), "50% see-through");
assert.equal(await page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue("--alpha").trim()), "0.5");
await page.click('#settings button[type="submit"]');
await page.waitForSelector("#settings", { state: "hidden" });
const saved = (await calls("save_settings")).at(-1).settings;
assert.equal(saved.shortcut, "Ctrl+Shift+K");
assert.equal(saved.live_shortcut, "Ctrl+Alt+L");
assert.equal(saved.live_system_audio, true);
assert.equal(saved.provider, "codex");
assert.equal(saved.voice_model, "tiny.en");
assert.equal(saved.dock_edge, "bottom");
assert.equal(saved.dock_along, 0.5);
assert.equal(saved.opacity, 0.5);
assert.equal(saved.hide_from_capture, false);
assert.equal(saved.auto_update, false);
assert.equal(saved.keep_warm_minutes, 5);
assert.equal(await page.getAttribute("#shell", "data-edge"), "bottom");

// Messages from the app (such as update results) appear in the notch.
await emit("notice", "Agent Desk 0.1.2 is the latest version.");
assert.match(await page.textContent("#notice"), /latest version/);
await emit("notice", "");
assert.ok(await page.isHidden("#notice"));

// Cancelling settings undoes an unsaved transparency preview.
await page.keyboard.press("Control+,");
await page.fill('[name="opacity"]', "1");
await page.keyboard.press("Escape");
assert.ok(await page.isHidden("#settings"));
assert.equal(await page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue("--alpha").trim()), "0.5");

// Shortcut-only mode needs a shortcut.
await page.keyboard.press("Control+,");
await page.selectOption('[name="wake_mode"]', "shortcut");
await page.focus("#shortcut-input");
await page.keyboard.press("Backspace");
await page.click('#settings button[type="submit"]');
assert.ok(await page.isVisible("#settings-msg"));
await page.keyboard.press("Escape");

// Dragging to another edge (reported by the app) reshapes the notch.
for (const edge of ["left", "right", "top"]) {
  await emit("settings-changed", { ...saved, dock_edge: edge, opacity: 0.92 });
  assert.equal(await page.getAttribute("#shell", "data-edge"), edge);
  if (edge !== "top") await snap("7-" + edge);
}

// The window is asked to match the content, and shrinks back to the notch.
const resizes = await calls("resize");
assert.ok(resizes.length > 5 && resizes.every((r) => r.width > 100 && r.height > 40));
assert.ok(resizes.some((r) => r.height < 90), "notch-only size was requested");
assert.ok(resizes.some((r) => r.height > 200), "panel size was requested");

// A missing CLI is explained up front.
await page.addInitScript(() => (window.__fake.cliFound = false));
await page.reload();
await page.waitForSelector("#notice:not([hidden])");
assert.match(await page.textContent("#notice"), /not found/);

/* ----- the glow page ----- */
const glow = await browser.newPage({ viewport: { width: 736, height: 414 }, deviceScaleFactor: shots ? 2 : 1 });
await glow.addInitScript(fake);
await glow.addInitScript(() => document.addEventListener("DOMContentLoaded", () => (document.documentElement.style.background = "#000")));
await glow.goto("file://" + path.join(root, "ui/glow.html"));
assert.equal(await glow.evaluate(() => getComputedStyle(document.getElementById("glow")).opacity), "0");
await glow.evaluate(() => window.__fake.emit("glow", true));
await glow.waitForTimeout(500);
assert.equal(await glow.evaluate(() => getComputedStyle(document.getElementById("glow")).opacity), "1");
if (shots) await glow.screenshot({ path: path.join(shots, "8-glow.png") });

assert.deepEqual(errors, []);
await browser.close();
console.log("ui tests passed");
