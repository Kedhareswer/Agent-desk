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
    include_screen: true, hide_from_capture: true, hide_on_blur: true, autostart: false,
    claude_path: "", codex_path: "", keep_warm_minutes: 5,
  };
  const emit = (name, payload) => (listeners[name] || []).forEach((fn) => fn({ payload }));
  window.__fake = { calls, emit, cliFound: true, failAsk: null };
  window.__TAURI__ = {
    core: {
      invoke: async (cmd, args) => {
        calls.push([cmd, args]);
        if (cmd === "get_settings") return settings;
        if (cmd === "save_settings") { settings = args.settings; return { settings, warning: null }; }
        if (cmd === "status") return { provider: settings.provider, cli_found: window.__fake.cliFound, detail: window.__fake.cliFound ? "C:/claude.exe" : "The `claude` command was not found.", hook_supported: true };
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
const page = await browser.newPage({ viewport: { width: 720, height: 660 }, deviceScaleFactor: shots ? 2 : 1 });
const errors = [];
page.on("pageerror", (e) => errors.push(e.message));
await page.addInitScript(fake);
await page.addInitScript(() => document.addEventListener("DOMContentLoaded", () => (document.documentElement.style.background = "#3b4252")));
await page.goto("file://" + path.join(root, "ui/index.html"));
const snap = async (name) => shots && page.screenshot({ path: path.join(shots, name + ".png") });
const calls = (cmd) => page.evaluate((c) => window.__fake.calls.filter((x) => x[0] === c).map((x) => x[1]), cmd);
const emit = (name, payload) => page.evaluate(([n, p]) => window.__fake.emit(n, p), [name, payload]);

// Waking shows what the assistant can see.
await emit("wake", { title: "Inbox - Gmail", process: "chrome.exe", has_screen: true });
assert.equal(await page.textContent("#screen-chip"), "Seeing: Inbox - Gmail");
assert.match(await page.textContent("#model-chip"), /Claude Haiku/);
assert.ok(await page.isHidden("#thread"));
await snap("1-idle");

// Asking sends the prompt with the screenshot and streams the answer.
await page.fill("#prompt", "draft a reply");
await page.keyboard.press("Enter");
await page.waitForSelector(".msg.ai");
assert.deepEqual((await calls("ask"))[0], { prompt: "draft a reply", withScreen: true });
assert.ok(await page.isVisible("#stop"));
assert.ok(await page.isHidden("#actions"));
await emit("ai", { turn: 1, kind: "delta", text: "\n\nYes, we're **on track** for Friday.\n\n- Pages are done\n- Waiting on `photos`\n\n```js\nconst x = 1 < 2;\n```" });
await page.waitForFunction(() => document.querySelector(".msg.ai strong"));
await snap("2-streaming");
// The early piece (sent before the turn id was known) was not lost.
assert.match(await page.textContent(".msg.ai"), /^Hi Ben,/);
assert.equal(await page.textContent(".msg.ai pre code"), "const x = 1 < 2;");
assert.equal(await page.locator(".msg.ai li").count(), 2);
await emit("ai", { turn: 99, kind: "delta", text: "STALE" });
await emit("ai", { turn: 1, kind: "done", text: "" });
await page.waitForSelector("#actions:not([hidden])");
assert.doesNotMatch(await page.textContent(".msg.ai"), /STALE/);
assert.equal(await page.textContent("#screen-chip"), "Screen already sent");
await snap("3-answer");

// Insert and copy pass the raw answer text, not HTML.
await page.keyboard.press("Control+Enter");
const inserted = (await calls("insert_text"))[0].text;
assert.ok(inserted.startsWith("Hi Ben,") && inserted.includes("**on track**"));
await page.click("#copy");
assert.equal((await calls("copy_text")).length, 1);

// Model output is never treated as HTML.
await page.fill("#prompt", "x");
await page.keyboard.press("Enter");
await emit("ai", { turn: 2, kind: "delta", text: '<img src=x onerror="window.__pwned=1"> ok' });
await emit("ai", { turn: 2, kind: "done", text: "" });
await page.waitForFunction(() => document.querySelectorAll(".msg.ai").length === 2 && !document.querySelector(".streaming"));
assert.equal(await page.evaluate(() => window.__pwned), undefined);
assert.equal(await page.locator(".msg.ai img").count(), 0);

// An empty Enter still asks, and the follow-up carries no second screenshot.
await page.keyboard.press("Enter");
assert.deepEqual((await calls("ask"))[2], { prompt: "", withScreen: false });
// Escape while answering stops; Escape again hides.
await page.keyboard.press("Escape");
assert.equal((await calls("stop")).length, 1);
assert.ok(await page.isVisible("#send"));
await page.keyboard.press("Escape");
assert.equal((await calls("hide")).length, 1);

// After a stop the next question starts a clean conversation.
await page.fill("#prompt", "again");
await page.keyboard.press("Enter");
assert.equal(await page.locator(".msg.user").count(), 1);
await emit("ai", { turn: 4, kind: "error", text: "You are not signed in." });
await page.waitForSelector(".msg.error");
assert.ok(await page.isVisible("#send"));
await snap("4-error");

// New chat clears everything.
await page.keyboard.press("Control+n");
assert.ok(await page.isHidden("#thread"));
assert.equal((await calls("new_chat")).length, 1);

// A failed ask (for example, CLI missing) is shown, not swallowed.
await page.evaluate(() => (window.__fake.failAsk = "The `claude` command was not found."));
await page.fill("#prompt", "hello");
await page.keyboard.press("Enter");
await page.waitForSelector(".msg.error");
assert.match(await page.textContent(".msg.error"), /not found/);
await page.evaluate(() => (window.__fake.failAsk = null));
await page.keyboard.press("Control+n");

// Settings: record a shortcut, switch provider, save.
await page.keyboard.press("Control+,");
await page.waitForSelector("#settings:not([hidden])");
await snap("5-settings");
await page.focus("#shortcut-input");
await page.keyboard.press("Control+Shift+KeyK");
assert.equal(await page.inputValue("#shortcut-input"), "Ctrl+Shift+K");
await page.selectOption('[name="provider"]', "codex");
assert.ok(await page.isHidden('[name="claude_model"]'));
assert.ok(await page.isVisible('[name="codex_model"]'));
await page.uncheck('[name="hide_from_capture"]');
await page.click('#settings button[type="submit"]');
await page.waitForSelector("#settings", { state: "hidden" });
const saved = (await calls("save_settings")).at(-1).settings;
assert.equal(saved.shortcut, "Ctrl+Shift+K");
assert.equal(saved.provider, "codex");
assert.equal(saved.hide_from_capture, false);
assert.equal(saved.keep_warm_minutes, 5);
assert.equal(await page.textContent("#model-chip"), "Codex");

// Shortcut-only mode needs a shortcut.
await page.keyboard.press("Control+,");
await page.selectOption('[name="wake_mode"]', "shortcut");
await page.focus("#shortcut-input");
await page.keyboard.press("Backspace");
await page.click('#settings button[type="submit"]');
assert.ok(await page.isVisible("#settings-msg"));
await page.keyboard.press("Escape");
assert.ok(await page.isHidden("#settings"));

// The window is asked to match the card's height.
const resizes = await calls("resize");
assert.ok(resizes.length > 3 && resizes.every((r) => r.height > 50));

// A missing CLI is explained up front.
await page.addInitScript(() => (window.__fake.cliFound = false));
await page.reload();
await page.waitForSelector("#notice:not([hidden])");
assert.match(await page.textContent("#notice"), /not found/);

assert.deepEqual(errors, []);
await browser.close();
console.log("ui tests passed");
