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
    actions_enabled: true, always_allow: ["press_keys"], commands: [], browser_profiles: { chrome: "projects" },
    claude_config_dir: "", codex_home: "", setup_done: true,
  };
  const view = (s) => ({ ...s, describe: s.when.kind === "once" ? `once, ${s.when.date} ${s.when.time}` : `at ${s.when.time}`, next: s.paused ? "" : "soon", last: "" });
  let schedules = [{ id: 1, name: "Morning brief", prompt: "Open my calendar and summarise today", paused: false, last_run: null, created: 0,
    when: { kind: "repeat", time: "09:00", days: [0, 1, 2, 3, 4] }, describe: "every weekday at 09:00", next: "Fri 20 Nov, 09:00", last: "" }];
  const emit = (name, payload) => (listeners[name] || []).forEach((fn) => fn({ payload }));
  window.__fake = { calls, emit, cliFound: true, failAsk: null, nextTurn: () => ++turn };
  // Finished answers tuck away after 5 s; the test turns that on where it checks it.
  window.__tuckMs = 1e9;
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
        if (cmd === "setup_info") return {
          provider: settings.provider, hook_supported: true, downloading: null, download_pct: 0,
          claude: { found: true, path: "C:/Users/sam/.local/bin/claude.exe", version: "2.1.327 (Claude Code)", signed_in: true, account: "sam@example.com", plan: "max", config_dir: "C:/Users/sam/.claude", sign_in_command: "claude auth login", problem: "" },
          codex: { found: true, path: "C:/codex.exe", version: "codex-cli 0.170.0", signed_in: false, account: "Not logged in", plan: "", config_dir: "C:/Users/sam/.codex", sign_in_command: "codex login", problem: "Not signed in." },
          speech: [
            { id: "tiny.en", label: "Tiny, English (fastest)", size_mb: 77, downloaded: false, partial: 0 },
            { id: "base.en", label: "Base, English", size_mb: 147, downloaded: true, partial: 0 },
            { id: "small.en", label: "Small, English (most accurate)", size_mb: 487, downloaded: false, partial: 40 },
          ],
        };
        if (cmd === "list_models") return args.provider === "codex"
          ? [{ id: "gpt-6.2-sol", name: "GPT-6.2-Sol", description: "Fast, go-to model." }, { id: "gpt-6.2-astra", name: "GPT-6.2-Astra", description: "Most capable." }]
          : [{ id: "haiku", name: "Haiku", description: "Fastest. Best for voice. Now: claude-haiku-5-5." }, { id: "sonnet", name: "Sonnet", description: "Balanced." }, { id: "opus", name: "Opus", description: "Most capable." }];
        if (cmd === "take_recovery") return window.__fake.recovery || null;
        if (cmd === "resume_task") { turn += 1; return turn; }
        if (cmd === "list_schedules") return schedules;
        if (cmd === "save_schedule") {
          const item = { ...args.schedule, id: args.schedule.id || schedules.length + 1 };
          schedules = schedules.filter((x) => x.id !== item.id).concat([view(item)]).sort((a, b) => a.id - b.id);
          return schedules;
        }
        if (cmd === "delete_schedule") return (schedules = schedules.filter((x) => x.id !== args.id));
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
// The settings and close buttons line up with the row they sit in.
const toolsInLine = () => page.evaluate(() => {
  const mid = (n) => { const r = n.getBoundingClientRect(); return r.top + r.height / 2; };
  const view = document.getElementById("notch").dataset.view;
  return Math.abs(mid(document.getElementById("tools")) - mid(document.getElementById(view === "input" ? "prompt" : "label")));
});
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
assert.equal(await page.locator("#meter").count(), 0, "only the label while listening, no sound meter");
assert.ok(await page.isHidden("#panel"), "nothing but the notch while listening");
await page.waitForTimeout(600);
const small = await page.locator("#notch").boundingBox();
assert.ok(small.height < 70 && small.width < 330, "collapsed, it is just a notch");
for (const level of [0.1, 0.5, 0.9]) await emit("level", level); // still sent by the app; ignored
await snap("1-listening");

await emit("phase", { phase: "transcribing", text: "" });
assert.equal(await page.textContent("#label"), "Transcribing");

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

// Settings and close sit at the right edge; the status stays centred.
await page.hover("#label");
await page.waitForTimeout(250); // the tools fade in
await snap("3b-done-hover");
{
  const tools = await page.locator("#tools").boundingBox();
  const right = big.x + big.width - (tools.x + tools.width);
  assert.ok(right >= 6 && right <= 20, `tools at the right edge (${right}px in)`);
  const row = await page.evaluate(() => {
    const boxes = [...document.querySelectorAll("#row > *")].filter((n) => n.id !== "tools" && n.offsetParent).map((n) => n.getBoundingClientRect());
    return { l: Math.min(...boxes.map((b) => b.left)), r: Math.max(...boxes.map((b) => b.right)) };
  });
  assert.ok(Math.abs((row.l + row.r) / 2 - (big.x + big.width / 2)) <= 2, "the status is centred");
  assert.ok(tools.x > row.r + 8, "the tools do not touch the status");
  assert.ok((await toolsInLine()) <= 1, "the tools are level with the status");
  assert.ok(await page.evaluate(() => getComputedStyle(document.getElementById("tools")).opacity === "1"), "shown on hover");
}

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

/* ----- a finished answer tucks itself away ----- */

// It stays long enough to read: at least 10 s, longer for long answers.
assert.deepEqual(await page.evaluate(() => [readingMs("It's 9:14 PM."), readingMs("word ".repeat(80)), readingMs("word ".repeat(400))]), [10000, 20000, 40000]);

await page.evaluate(() => Object.assign(window, { __tuckMs: 300, __tuckAfterHoverMs: 200 }));
{
  const t = await page.evaluate(() => window.__fake.nextTurn());
  await emit("asked", { turn: t, text: "what time is it in Tokyo" });
  assert.equal(await page.locator(".msg.ai").count(), 1, "the previous answer is gone; only the latest shows");
  await emit("ai", { turn: t, kind: "delta", text: "It's 9:14 PM in Tokyo." });
  await page.waitForFunction(() => document.querySelector(".msg.ai")?.textContent === "It's 9:14 PM in Tokyo.");
  await page.hover("#thread"); // the mouse is on it when the answer finishes
  await emit("ai", { turn: t, kind: "done", text: "" });
  const hides = (await calls("hide")).length;
  await page.waitForTimeout(900);
  assert.ok(await page.isVisible("#thread"), "it stays open while the mouse is over it");
  assert.equal((await calls("hide")).length, hides);
  await page.mouse.move(5, 690); // away from the notch
  await page.waitForFunction((n) => window.__fake.calls.filter((c) => c[0] === "hide").length === n + 1, hides, { timeout: 3000 });
  assert.ok(await page.isHidden("#panel"), "it shrank back to the notch before going");
  assert.equal(await notch("state"), "done");
  await page.evaluate(() => Object.assign(window, { __tuckMs: 1e9, __tuckAfterHoverMs: 1e9 }));
}

/* ----- typing: tap to open ----- */

await wake(true);
assert.equal(await notch("view"), "input");
await page.fill("#prompt", "make it shorter");
await page.keyboard.press("Enter");
assert.deepEqual((await calls("ask")).at(-1), { prompt: "make it shorter", withScreen: true });
assert.equal(await notch("state"), "answering");
// Only this question's answer is shown, and the piece sent before the turn
// id was known was kept.
await page.waitForFunction(() => document.querySelectorAll(".msg.ai").length === 1 && document.querySelector(".msg.ai").textContent.startsWith("Hi Ben,"));
assert.equal(await page.locator(".msg.user").count(), 1);

// Model output is never treated as HTML.
const typedTurn = voiceTurn + 2; // one more turn ran in the tuck check
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
const hidesBeforeEsc = (await calls("hide")).length;
await page.keyboard.press("Escape");
assert.equal((await calls("hide")).length, hidesBeforeEsc + 1);

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
assert.ok((await toolsInLine()) <= 1, "the tools are level with the text field");
{
  const n = await page.locator("#notch").boundingBox();
  const t = await page.locator("#tools").boundingBox();
  const p = await page.locator("#prompt").boundingBox();
  assert.ok(n.x + n.width - (t.x + t.width) <= 20 && t.x >= p.x + p.width, "typing: the tools are at the right, after the field");
}
await snap("5-typing");

// A failed ask (for example, CLI missing) is shown, not swallowed.
await page.evaluate(() => (window.__fake.failAsk = "The `claude` command was not found."));
await page.fill("#prompt", "hello");
await page.keyboard.press("Enter");
await page.waitForSelector(".msg.error");
assert.match(await page.textContent(".msg.error"), /not found/);
await page.evaluate(() => (window.__fake.failAsk = null));
await page.keyboard.press("Control+n");

/* ----- acting on the computer ----- */

// "Open Chrome and open GitHub": a harmless step is shown, then just runs.
await wake(false);
let actTurn = await page.evaluate(() => window.__fake.nextTurn());
await emit("asked", { turn: actTurn, text: "open Chrome and open GitHub" });
const step = (id, status, summary = "Open github.com in Chrome") => emit("action", { id, turn: actTurn, kind: "open_url", summary, detail: "https://github.com", status, result: "" });
await step(1, "running");
assert.equal(await notch("state"), "acting");
assert.equal(await page.textContent("#label"), "Working on it");
assert.equal(await page.textContent(".msg.act .act-text"), "Open github.com in Chrome");
assert.ok(await page.isVisible(".msg.act"), "the step shows in the notch before it runs");
assert.equal(await page.locator(".act-buttons").count(), 0, "a harmless step does not ask");
await snap("10-step");
await step(1, "done");
// The next steps take the same line instead of adding rows.
await step(2, "running", "Find “Contributors” on the page");
await page.waitForFunction(() => document.querySelector(".msg.act .act-text").textContent === "Find “Contributors” on the page");
await step(2, "done", "Find “Contributors” on the page");
await step(3, "running", "Scroll down");
await page.waitForFunction(() => document.querySelector(".msg.act .act-text").textContent === "Scroll down");
assert.equal(await page.locator(".msg.act").count(), 1, "one line for all the steps");
assert.equal(await page.textContent(".msg.act .act-status"), "Step 3");
await page.waitForTimeout(300); // the new step has slid in
{
  // One line high, whatever the step.
  const h = await page.locator(".msg.act").boundingBox();
  assert.ok(h.height <= 40, `the step line stays one line (${h.height}px)`);
  const off = await page.evaluate(() => {
    const mid = (n) => { const r = n.getBoundingClientRect(); return r.top + r.height / 2; };
    const line = document.querySelector(".msg.act");
    return [".act-dot", ".act-text", ".act-status"].map((q) => Math.abs(mid(line.querySelector(q)) - mid(line)));
  });
  assert.ok(off.every((d) => d <= 1), `dot, step and count share a centre line (${off})`);
}
await snap("10b-step-3");
await step(3, "done", "Scroll down");
await emit("ai", { turn: actTurn, kind: "delta", text: "You're the top contributor on 3 of your repos." });
await emit("ai", { turn: actTurn, kind: "done", text: "" });
assert.equal(await notch("state"), "done");
await page.waitForFunction(() => !document.querySelector(".msg.act"), null, { timeout: 2000 });
const order = () => page.evaluate(() => [...document.querySelectorAll("#thread > .msg:not(.user)")].map((n) => n.className));
assert.deepEqual(await order(), ["msg ai"], "the step line goes once the answer is done");
await page.waitForFunction(() => document.querySelector(".msg.ai").textContent === "You're the top contributor on 3 of your repos.");
assert.deepEqual(await calls("answer_action"), []);
await snap("10c-acted");

// A step that sends something waits for a one-tap OK and shows exactly what will happen.
actTurn = await page.evaluate(() => window.__fake.nextTurn());
await emit("asked", { turn: actTurn, text: "tell Sam I'm running late" });
await emit("ai", { turn: actTurn, kind: "delta", text: "I'll type it into the chat." });
const send = { id: 2, turn: actTurn, kind: "type_text", summary: "Type “Running late, sorry” and press Enter", detail: "Running late, sorry\n", result: "" };
await emit("action", { ...send, status: "ask" });
assert.equal(await notch("state"), "confirm");
assert.equal(await page.textContent("#label"), "Needs your OK");
assert.equal(await page.textContent(".msg.act[data-status=ask] .act-detail"), "Running late, sorry\n");
assert.deepEqual(await page.locator(".act-buttons button").allTextContents(), ["Allow", "Always allow", "Don't"]);
// Text written before the step stays above it.
assert.deepEqual(await order(), ["msg ai", "msg act"]);
await page.waitForFunction(() => document.querySelector(".msg.ai").textContent === "I'll type it into the chat.");
assert.ok(await page.isVisible(".msg.ai"), "text written before the step is shown");
assert.equal(await page.textContent("#top-line"), "tell Sam I'm running late", "the question stays on top");
await snap("11-confirm");
await page.click(".act-buttons button[data-decision=allow]");
assert.deepEqual((await calls("answer_action")).at(-1), { id: 2, decision: "allow" });
assert.equal(await page.locator(".act-buttons").count(), 0, "answered once");
await emit("action", { ...send, status: "running" });
await emit("action", { ...send, status: "done" });
assert.equal(await page.getAttribute(".msg.act", "data-status"), "done");

// Escape refuses a waiting step (without stopping the answer); Enter allows.
const stopsBefore = (await calls("stop")).length;
await emit("action", { ...send, id: 3, status: "ask" });
await page.keyboard.press("Escape");
assert.deepEqual((await calls("answer_action")).at(-1), { id: 3, decision: "deny" });
assert.equal((await calls("stop")).length, stopsBefore, "Escape on a question does not stop the answer");
await emit("action", { ...send, id: 3, status: "denied" });
assert.equal(await page.getAttribute(".msg.act[data-id='3']", "data-status"), "denied");
await emit("action", { ...send, id: 4, kind: "run_command", summary: "Run “Build site”", detail: "npm run build", status: "ask" });
await page.keyboard.press("Enter");
assert.deepEqual((await calls("answer_action")).at(-1), { id: 4, decision: "allow" });
// "Always allow" is passed on; the app remembers it.
await emit("action", { ...send, id: 5, status: "ask" });
await page.click(".act-buttons button[data-decision=always]");
assert.deepEqual((await calls("answer_action")).at(-1), { id: 5, decision: "always" });
// A failed step says why, on the same line.
await emit("action", { ...send, id: 6, kind: "open_app", summary: "Open Photoshop", detail: "", status: "failed", result: "There is no app called \"Photoshop\" in the Start menu." });
assert.match(await page.textContent(".msg.act[data-id='6'] .act-error"), /no app called/);
assert.equal(await page.locator(".msg.act").count(), 1);
// Stop takes away a pending question.
await emit("action", { ...send, id: 7, status: "ask" });
await page.click("#stop");
assert.equal(await page.locator(".act-buttons").count(), 0);
// Steps from an old turn are ignored.
await emit("action", { ...send, id: 8, turn: 1, status: "ask" });
assert.equal(await page.locator(".msg.act[data-id='8']").count(), 0);

/* ----- never cut off ----- */

// A long question in the top line used to make the shape wider than the
// window, so one side was cut off. The shape now always fits.
{
  const fits = () => page.evaluate(() => {
    const shell = document.getElementById("shell").getBoundingClientRect();
    const notch = document.getElementById("notch").getBoundingClientRect();
    const content = document.getElementById("content");
    const sent = window.__fake.calls.filter((c) => c[0] === "resize").at(-1)[1];
    return { notchW: notch.width, contentW: content.scrollWidth, shellW: shell.width, sentW: sent.width, sentH: sent.height, h: shell.height };
  });
  const long = "check whether I'm a top contributor on GitHub across all of my repositories and organisations, then compare it with last year and tell me which repos changed the most and why that might be";
  const t = await page.evaluate(() => window.__fake.nextTurn());
  await emit("asked", { turn: t, text: long });
  await page.waitForTimeout(500);
  let f = await fits();
  assert.ok(f.sentW <= 640, `the window asked for fits the 640 px limit (${f.sentW})`);
  assert.ok(f.contentW <= f.notchW + 1, `nothing pokes out of the shape (${f.contentW} > ${f.notchW})`);
  assert.ok(await page.evaluate(() => { const n = document.getElementById("top-line"); return n.scrollWidth > n.clientWidth; }), "the question is shortened with an ellipsis");
  await snap("12b-long-question");
  await emit("ai", { turn: t, kind: "delta", text: "You're the top contributor on 3 repos. ".repeat(40) });
  await emit("ai", { turn: t, kind: "done", text: "" });
  await page.waitForTimeout(600);
  f = await fits();
  assert.ok(f.sentW <= 640 && f.sentH <= 640, `a long answer fits too (${f.sentW}x${f.sentH})`);
  assert.ok(f.contentW <= f.notchW + 1);
  await snap("12c-long-answer");
  // Every view fits the window the app allows (640 x 640).
  for (const [open, close] of [["#open-settings", "#settings-cancel"], ["#open-history", "#open-history"]]) {
    await page.click(open);
    await page.waitForTimeout(600);
    f = await fits();
    assert.ok(f.sentW <= 640 && f.sentH <= 640, `${open}: fits (${f.sentW}x${f.sentH})`);
    assert.ok(f.contentW <= f.notchW + 1, `${open}: nothing pokes out`);
    await page.click(close);
  }
}

/* ----- out of the way: click-through ----- */

{
  const regions = () => page.evaluate(() => window.__fake.calls.filter((c) => c[0] === "set_hit_regions").at(-1)[1].regions);
  await page.waitForTimeout(500);
  let r = await regions();
  const inside = (rect, x, y) => x >= rect[0] && y >= rect[1] && x < rect[0] + rect[2] && y < rect[1] + rect[3];
  const centre = (sel) => page.evaluate((s) => { const b = document.querySelector(s).getBoundingClientRect(); return [b.left + b.width / 2, b.top + b.height / 2]; }, sel);
  // The answer text lets the mouse through; its buttons do not.
  const [tx, ty] = await centre(".msg.ai");
  assert.ok(!r.controls.some((c) => inside(c, tx, ty)), "the answer is click-through");
  assert.ok(r.with_ctrl.some((c) => inside(c, tx, ty)), "...unless Ctrl is held");
  for (const b of ["#copy", "#pin", "#open-history"]) {
    const [bx, by] = await centre(b);
    assert.ok(r.controls.some((c) => inside(c, bx, by)), `${b} takes the mouse`);
  }
  assert.ok(await page.isVisible("#ctrl-hint"), "a long answer says how to scroll it");
  // The app reports hover (the window gets none of its own while
  // click-through): the tools show and the answer is held open.
  await page.evaluate(() => Object.assign(window, { __tuckMs: 300, __tuckAfterHoverMs: 200 }));
  await emit("hover", true);
  assert.ok(await page.evaluate(() => document.getElementById("notch").classList.contains("hover")));
  await page.waitForTimeout(250);
  assert.equal(await page.evaluate(() => getComputedStyle(document.getElementById("tools")).opacity), "1", "tools show on hover");
  const hides = (await calls("hide")).length;
  await page.keyboard.press("Control+p"); // pin...
  await page.keyboard.press("Control+p"); // ...and unpin: the clock starts again
  await page.waitForTimeout(700);
  assert.equal((await calls("hide")).length, hides, "held open while the cursor is over it");
  // Pinned: it stays even after the cursor leaves.
  await page.click("#pin");
  assert.equal(await page.getAttribute("#pin", "aria-pressed"), "true");
  assert.equal(await page.textContent("#pin span"), "Pinned");
  await emit("hover", false);
  await page.waitForTimeout(900);
  assert.equal((await calls("hide")).length, hides, "a pinned answer does not tuck away");
  await snap("13-pinned");
  await page.click("#pin");
  await page.mouse.move(5, 690); // the test's own mouse leaves the notch
  await page.waitForFunction((n) => window.__fake.calls.filter((c) => c[0] === "hide").length > n, hides, { timeout: 3000 });
  await page.evaluate(() => Object.assign(window, { __tuckMs: 1e9, __tuckAfterHoverMs: 1e9 }));
  // Typing: the whole notch takes the mouse.
  await wake(true);
  await page.waitForTimeout(500);
  r = await regions();
  assert.equal(r.controls.length, 1);
  assert.deepEqual(r.controls[0], r.notch, "while typing, the whole notch");
  assert.deepEqual(r.with_ctrl, []);
  await page.keyboard.press("Escape");
}

/* ----- read aloud, try again ----- */

{
  await page.evaluate(() => {
    window.__spoken = [];
    speechSynthesis.speak = (u) => { window.__spoken.push(u.text); window.__utterance = u; };
    speechSynthesis.cancel = () => {};
  });
  const t = await page.evaluate(() => window.__fake.nextTurn());
  await wake(false);
  await emit("asked", { turn: t, text: "how many repos do I have" });
  await emit("ai", { turn: t, kind: "delta", text: "You have **42** repositories." });
  await emit("ai", { turn: t, kind: "done", text: "" });
  await page.click("#speak");
  assert.deepEqual(await page.evaluate(() => window.__spoken), ["You have 42 repositories."], "reads the text, not the Markdown");
  assert.equal(await page.textContent("#speak span"), "Stop");
  await page.evaluate(() => window.__utterance.onend());
  assert.equal(await page.textContent("#speak span"), "Read", "back to Read when it finishes");

  // A failed question offers to try again with the same words.
  const t2 = await page.evaluate(() => window.__fake.nextTurn());
  await emit("asked", { turn: t2, text: "open my GitHub" });
  await emit("ai", { turn: t2, kind: "error", text: "Claude is not signed in." });
  assert.ok(await page.isVisible(".msg.error .retry"));
  await page.click(".msg.error .retry");
  assert.deepEqual((await calls("ask")).at(-1), { prompt: "open my GitHub", withScreen: false });
  await page.waitForTimeout(100);
  await emit("ai", { turn: await page.evaluate(() => state.turn), kind: "done", text: "" });
  await page.keyboard.press("Control+n");
}

/* ----- never stuck ----- */

// A step that arrives with no answer in progress does not leave the notch
// "Working on it": once it is done the notch goes back to rest.
{
  const t = await page.evaluate(() => window.__fake.nextTurn());
  await emit("asked", { turn: t, text: "what's on my screen" });
  await emit("ai", { turn: t, kind: "delta", text: "A Notepad window." });
  await emit("ai", { turn: t, kind: "done", text: "" });
  await emit("action", { id: 40, turn: t, kind: "click", summary: "Click “Text editor”", detail: "", status: "running", result: "" });
  assert.equal(await notch("state"), "acting");
  await emit("action", { id: 40, turn: t, kind: "click", summary: "Click “Text editor”", detail: "", status: "done", result: "" });
  assert.equal(await notch("state"), "done", "back to rest after a stray step");
  await page.waitForFunction(() => !document.querySelector(".msg.act"), null, { timeout: 2000 });
}
// An answer with no sign of life for too long is stopped and says so.
{
  await page.evaluate(() => (window.__stallMs = 400));
  const stops = (await calls("stop")).length;
  const t = await page.evaluate(() => window.__fake.nextTurn());
  await emit("asked", { turn: t, text: "check my repos" });
  await emit("action", { id: 41, turn: t, kind: "open_url", summary: "Open github.com", detail: "", status: "running", result: "" });
  await page.waitForFunction(() => document.getElementById("notch").dataset.state === "error", null, { timeout: 4000 });
  assert.equal((await calls("stop")).length, stops + 1, "the stuck answer was stopped");
  assert.match(await page.textContent(".msg.error"), /No reply for a while/);
  await emit("ai", { turn: t, kind: "delta", text: "late" });
  assert.equal(await page.locator(".msg.ai", { hasText: "late" }).count(), 0, "late events are ignored");
  // A step waiting for the user's OK is not a stall.
  const t2 = await page.evaluate(() => window.__fake.nextTurn());
  await emit("asked", { turn: t2, text: "send it" });
  await emit("action", { id: 42, turn: t2, kind: "click", summary: "Click “Send”", detail: "", status: "ask", result: "" });
  await page.waitForTimeout(1600);
  assert.equal(await notch("state"), "confirm", "waiting for an OK is not stuck");
  await page.click(".act-buttons button[data-decision=deny]");
  await emit("action", { id: 42, turn: t2, kind: "click", summary: "Click “Send”", detail: "", status: "denied", result: "" });
  await emit("ai", { turn: t2, kind: "done", text: "" });
  await page.evaluate(() => (window.__stallMs = 1e9));
  await page.keyboard.press("Control+n");
}

/* ----- earlier answers ----- */

// Finished answers are kept and can be read again from the notch.
await wake(false);
await page.hover("#head");
await page.click("#open-history");
assert.ok(await page.isVisible("#history"));
assert.ok(await page.isHidden("#thread"), "the history takes the panel's place");
const hist = await page.locator(".hist").allTextContents();
assert.ok(hist.length >= 2, `earlier answers are listed (${hist.length})`);
const github = hist.find((h) => /open Chrome and open GitHub/.test(h)) || "";
assert.match(github, /top contributor on 3/);
assert.match(github, /3 steps on your computer/);
assert.ok(!hist.some((h) => /No reply for a while|^\s*$/.test(h)), "only real answers are kept");
assert.equal(await page.locator(".hist .msg.ai script, .hist .msg.ai img").count(), 0);
{
  const edges = await page.evaluate(() => {
    // The visible text, not the box around it (buttons have padding).
    const right = (q) => { const r = document.createRange(); r.selectNodeContents(document.querySelector(q)); return r.getBoundingClientRect().right; };
    const left = (q) => document.querySelector(q).getBoundingClientRect().left;
    return { clear: right("#history-clear"), time: right(".hist time"), list: right("#history-list"), head: left(".history-head h3"), q: left(".hist-q span") };
  });
  assert.ok(Math.abs(edges.clear - edges.time) <= 2, `Clear lines up with the times (${edges.clear} vs ${edges.time})`);
  assert.ok(Math.abs(edges.head - edges.q) <= 2, "the heading lines up with the questions");
  const clipped = await page.evaluate(() => [...document.querySelectorAll(".hist .msg.ai")].filter((n) => n.scrollWidth > n.clientWidth + 1).length);
  assert.equal(clipped, 0, "no answer is cut off sideways");
}
await snap("12-history");
await page.fill("#history-search", "tokyo");
assert.deepEqual((await page.locator(".hist").allTextContents()).length, 1, "search narrows the list");
await page.fill("#history-search", "zzzz nothing");
assert.equal(await page.locator(".hist").count(), 0);
assert.equal(await page.textContent("#history-empty"), "Nothing matches.");
await page.fill("#history-search", "tokyo");
const asksBefore = (await calls("ask")).length;
await page.click(".hist-tools button:has-text('Ask again')");
assert.ok(await page.isHidden("#history"), "asking again closes the history");
assert.deepEqual((await calls("ask")).at(-1), { prompt: "what time is it in Tokyo", withScreen: false });
assert.equal((await calls("ask")).length, asksBefore + 1);
await emit("ai", { turn: await page.evaluate(() => state.turn), kind: "done", text: "" });
await page.keyboard.press("Control+h");
await page.keyboard.press("Escape");
assert.ok(await page.isHidden("#history"), "Escape closes it");
await page.keyboard.press("Control+h");
assert.ok(await page.isVisible("#history"), "Ctrl+H opens it");
await page.click("#history-clear");
assert.equal(await page.locator(".hist").count(), 0);
assert.ok(await page.isVisible("#history-empty"));
await page.keyboard.press("Escape");

// A scheduled task starts a conversation of its own.
const schedTurn = await page.evaluate(() => window.__fake.nextTurn());
await emit("asked", { turn: schedTurn, text: "Scheduled: Morning brief", quiet: false, scheduled: true });
assert.equal(await page.locator(".msg.act").count(), 0, "the old conversation is cleared");
assert.equal(await page.textContent("#top-line"), "Scheduled: Morning brief");
await emit("ai", { turn: schedTurn, kind: "delta", text: "You have two meetings today." });
await emit("ai", { turn: schedTurn, kind: "done", text: "" });
await page.keyboard.press("Control+n");

/* ----- live mode: "work with me" ----- */

await emit("live", true);
assert.equal(await notch("state"), "live");
assert.equal(await page.textContent("#label"), "Working with you");
assert.ok(await page.isVisible("#mic"));
assert.ok((await toolsInLine()) <= 1, "the tools are level with the status under a top line");
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

/* ----- first-run setup ----- */

await emit("open-setup", null);
await page.waitForSelector("#setup:not([hidden])");
await page.waitForFunction(() => document.querySelector("#check-account").dataset.ok === "yes");
assert.equal(await page.textContent("#check-cli b"), "Claude Code 2.1.327");
assert.match(await page.textContent("#check-account"), /Signed in as sam@example\.com · max plan/);
assert.ok(await page.isHidden("#copy-signin"), "no sign-in command needed when signed in");
// Models come from the CLI, with what each is for.
await page.waitForFunction(() => document.querySelectorAll("#setup-model option").length === 3);
assert.equal(await page.inputValue("#setup-model"), "haiku");
assert.match(await page.textContent("#model-desc"), /Now: claude-haiku-5-5/);
// Speech models: downloaded, paused part-way, not yet.
assert.equal(await page.textContent('.speech-row[data-model="base.en"] .state'), "Downloaded");
assert.equal(await page.textContent('.speech-row[data-model="small.en"] .state'), "Resume (40%)");
await page.click('.speech-row[data-model="tiny.en"] button');
assert.deepEqual((await calls("download_speech")).at(-1), { model: "tiny.en" });
await emit("speech-progress", { model: "tiny.en", pct: 34, done: false, error: "" });
assert.equal(await page.textContent('.speech-row[data-model="tiny.en"] .state'), "Downloading 34%");
assert.equal(await page.evaluate(() => document.querySelector('.speech-row[data-model="tiny.en"] .bar i').style.width), "34%");
await snap("14-setup");
await emit("speech-progress", { model: "tiny.en", pct: 100, done: true, error: "" });
assert.equal(await page.textContent('.speech-row[data-model="tiny.en"] .state'), "Downloaded");
// Codex: not signed in, so the sign-in command is offered for copying.
await page.selectOption("#setup-provider", "codex");
assert.equal(await page.getAttribute("#check-account", "data-ok"), "no");
await page.click("#copy-signin");
assert.deepEqual((await calls("copy_text")).at(-1), { text: "codex login" });
await page.waitForFunction(() => document.querySelectorAll("#setup-model option").length === 3);
assert.equal(await page.textContent("#setup-model option"), "Your Codex default");
await page.selectOption("#setup-model", "gpt-6.2-sol");
// Back to Claude, pick Sonnet, a separate account folder and the tiny voice.
await page.selectOption("#setup-provider", "claude");
await page.selectOption("#setup-model", "sonnet");
await page.fill("#setup-account", "D:\\claude-work");
await page.check('.speech-row[data-model="tiny.en"] input');
await page.click("#setup-done");
await page.waitForSelector("#setup", { state: "hidden" });
const afterSetup = (await calls("save_settings")).at(-1).settings;
assert.equal(afterSetup.setup_done, true);
assert.equal(afterSetup.provider, "claude");
assert.equal(afterSetup.claude_model, "sonnet");
assert.equal(afterSetup.claude_config_dir, "D:\\claude-work");
assert.equal(afterSetup.voice_model, "tiny.en");
// Settings can reopen the guide.
await page.keyboard.press("Control+,");
await page.click("#open-setup");
await page.waitForSelector("#setup:not([hidden])");
await page.keyboard.press("Escape");
assert.ok(await page.isHidden("#setup"));
// Put things back for the settings checks below.
await emit("settings-changed", { ...afterSetup, claude_model: "haiku", claude_config_dir: "", voice_model: "base.en" });

/* ----- settings ----- */

await page.keyboard.press("Control+,");
await page.waitForSelector("#settings:not([hidden])");
await snap("6-settings");

// Actions: on by default, "always allow" can be undone, commands are saved.
assert.ok(await page.isChecked('[name="actions_enabled"]'));
assert.equal(await page.textContent("#allowed-list .chip"), "Pressing keys\u00d7");
await page.click("#allowed-list .chip button");
assert.match(await page.textContent("#allowed-list"), /Every risky step asks first/);
// The remembered browser profile is shown and can be forgotten.
assert.equal(await page.textContent("#profiles-list .chip"), "Chrome: projects\u00d7");
await page.click("#profiles-list .chip button");
assert.match(await page.textContent("#profiles-list"), /asked once per browser/);
await page.click("#add-command");
await page.fill(".cmd-row .cmd-name", "Build site");
await page.keyboard.press("Enter"); // must not submit the settings
assert.ok(await page.isVisible("#settings"));
await page.fill(".cmd-row .cmd-command", "npm run build");

// Schedules show when they run next, and can be paused, added and deleted.
await page.waitForSelector(".sched-row");
assert.match(await page.textContent(".sched-row .what"), /Morning brief/);
assert.match(await page.textContent(".sched-row .when"), /every weekday at 09:00 · Next: Fri 20 Nov, 09:00/);
await page.click(".sched-row button:text('Pause')");
assert.equal((await calls("save_schedule")).at(-1).schedule.paused, true);
assert.equal((await calls("save_schedule")).at(-1).schedule.describe, undefined, "only the schedule itself is sent");
await page.waitForSelector(".sched-row.paused");
await page.click("#add-schedule");
await page.fill("#sched-prompt", "Remind me to send the report");
await page.selectOption("#sched-repeat", "once");
assert.ok(await page.isVisible("#sched-date"));
await page.fill("#sched-time", "17:00");
await page.fill("#sched-date", "2026-11-19");
await snap("12-schedule-editor");
await page.click("#sched-save");
const added = (await calls("save_schedule")).at(-1).schedule;
assert.deepEqual([added.id, added.prompt, added.when], [0, "Remind me to send the report", { kind: "once", date: "2026-11-19", time: "17:00" }]);
await page.waitForSelector("#schedule-editor", { state: "hidden" });
assert.equal(await page.locator(".sched-row").count(), 2);
await page.click("#add-schedule");
await page.fill("#sched-prompt", "Stand-up notes");
await page.selectOption("#sched-repeat", "custom");
await page.click("#sched-save");
assert.match(await page.textContent("#sched-msg"), /at least one day/);
await page.check("#sched-days input[value='0']");
await page.check("#sched-days input[value='2']");
await page.click("#sched-save");
assert.deepEqual((await calls("save_schedule")).at(-1).schedule.when, { kind: "repeat", time: "09:00", days: [0, 2] });
await page.waitForSelector("#schedule-editor", { state: "hidden" });
await page.click(".sched-row:last-child button:text('Delete')");
assert.deepEqual((await calls("delete_schedule")).at(-1), { id: 3 });
await page.waitForFunction(() => document.querySelectorAll(".sched-row").length === 2);
await page.locator("#schedules-section").scrollIntoViewIfNeeded();
await snap("13-schedules");
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
assert.equal(saved.actions_enabled, true);
assert.deepEqual(saved.always_allow, []);
assert.deepEqual(saved.browser_profiles, {});
assert.deepEqual(saved.commands, [{ name: "Build site", command: "npm run build" }]);
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
  // The app places the window against that screen edge; do the same here.
  await page.evaluate((e) => (document.getElementById("shell").style.margin = { left: "160px auto 0 0", right: "160px 0 0 auto", top: "0 auto" }[e]), edge);
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

/* ----- after a crash: offer to continue ----- */
{
  const open = async (recovery) => {
    const p = await browser.newPage({ viewport: { width: 720, height: 700 }, deviceScaleFactor: shots ? 2 : 1 });
    p.on("pageerror", (e) => errors.push(e.message));
    await p.addInitScript(fake);
    await p.addInitScript((r) => document.addEventListener("DOMContentLoaded", () => {
      window.__fake.recovery = r;
      document.documentElement.style.background = "linear-gradient(135deg, #d9d4c7, #b9c4d6)";
      document.getElementById("shell").style.margin = "0 auto";
    }), recovery);
    await p.goto("file://" + path.join(root, "ui/index.html"));
    await p.waitForSelector("#recover:not([hidden])");
    return p;
  };
  let p = await open({ prompt: "open my GitHub and count my repos", seconds_ago: 130 });
  assert.equal(await p.getAttribute("#notch", "data-state"), "interrupted");
  assert.equal(await p.textContent("#label"), "Stopped unexpectedly");
  assert.equal(await p.textContent("#top-line"), "open my GitHub and count my repos");
  assert.match(await p.textContent("#recover-text"), /closed unexpectedly 2 minutes ago/);
  if (shots) { await p.waitForTimeout(700); await p.screenshot({ path: path.join(shots, "15-recover.png") }); }
  await p.click("#recover-go");
  const resumed = await p.evaluate(() => window.__fake.calls.filter((c) => c[0] === "resume_task").length);
  assert.equal(resumed, 1, "Continue resends the request");
  assert.ok(await p.isHidden("#recover"));
  assert.equal(await p.getAttribute("#notch", "data-state"), "thinking");
  assert.equal(await p.textContent("#top-line"), "open my GitHub and count my repos");
  await p.close();
  // Not now: forget it and hide.
  p = await open({ prompt: "x", seconds_ago: 5 });
  assert.match(await p.textContent("#recover-text"), /just now/);
  await p.click("#recover-no");
  const calls2 = await p.evaluate(() => window.__fake.calls.map((c) => c[0]));
  assert.ok(calls2.includes("dismiss_recovery") && calls2.includes("hide"));
  assert.ok(await p.isHidden("#recover"));
  await p.close();
}

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
// While Agent Desk acts on the computer the glow takes its working colour.
await glow.evaluate(() => window.__fake.emit("glow", "acting"));
assert.ok(await glow.evaluate(() => document.getElementById("glow").classList.contains("acting")));
if (shots) { await glow.waitForTimeout(500); await glow.screenshot({ path: path.join(shots, "8-glow-acting.png") }); }
await glow.evaluate(() => window.__fake.emit("glow", false));
await glow.waitForTimeout(500);
assert.equal(await glow.evaluate(() => getComputedStyle(document.getElementById("glow")).opacity), "0");

assert.deepEqual(errors, []);
await browser.close();
console.log("ui tests passed");
