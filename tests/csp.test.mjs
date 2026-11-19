// Loads the notch and the glow page under the app's real Content Security
// Policy (from tauri.conf.json) and checks that both still start and that
// nothing was blocked. The Rust side is faked as in ui.test.mjs.
//   node tests/csp.test.mjs
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import { readFileSync } from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";

const { chromium } = createRequire(import.meta.url)("playwright");
const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const csp = JSON.parse(readFileSync(path.join(root, "src-tauri/tauri.conf.json"), "utf8")).app.security.csp;
assert.ok(csp && csp.includes("script-src 'self'"), "a strict script policy is set");

const fake = () => {
  const listeners = {};
  window.__calls = [];
  window.__TAURI__ = {
    core: {
      invoke: async (cmd) => {
        window.__calls.push(cmd);
        if (cmd === "get_settings") return { provider: "claude", claude_model: "haiku", include_screen: true, dock_edge: "top", opacity: 0.92, keep_warm_minutes: 5, wake_mode: "ctrl_alt", always_allow: [], commands: [], browser_profiles: {}, setup_done: true };
        if (cmd === "status") return { cli_found: true, hook_supported: true, detail: "" };
        return null;
      },
    },
    event: { listen: async (name, fn) => { (listeners[name] ||= []).push(fn); window.__listening = Object.keys(listeners); return () => {}; } },
  };
};

const browser = await chromium.launch();
const page = await browser.newPage();
const violations = [];
page.on("console", (m) => /Content Security Policy|Refused to/i.test(m.text()) && violations.push(m.text()));
page.on("pageerror", (e) => violations.push(e.message));
// Serve ui/ over http with the policy as a header, as the app's web view would apply it.
await page.route("http://agentdesk.test/**", (route) => {
  const file = path.join(root, "ui", new URL(route.request().url()).pathname);
  const type = file.endsWith(".js") ? "text/javascript" : file.endsWith(".css") ? "text/css" : "text/html";
  route.fulfill({ status: 200, contentType: type, headers: { "Content-Security-Policy": csp }, body: readFileSync(file) });
});
await page.addInitScript(fake);

await page.goto("http://agentdesk.test/index.html");
await page.waitForFunction(() => window.__calls.includes("status"));
assert.ok(await page.evaluate(() => window.__listening.includes("ai")), "the notch started and listens");
await page.goto("http://agentdesk.test/glow.html");
await page.waitForFunction(() => (window.__listening || []).includes("glow"));
assert.deepEqual(violations, [], "nothing blocked by the policy");

await browser.close();
console.log("csp tests passed");
