// Overlay behaviour. All system work happens in the Rust side; this file
// only draws the conversation and forwards what the user asks for.
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (id) => document.getElementById(id);
const el = {
  shell: $("shell"), card: $("card"), prompt: $("prompt"), send: $("send"), stop: $("stop"),
  screenChip: $("screen-chip"), modelChip: $("model-chip"), newChat: $("new-chat"), openSettings: $("open-settings"),
  notice: $("notice"), thread: $("thread"), actions: $("actions"), insert: $("insert"), copy: $("copy"),
  suggest: $("suggest"), stat: $("stat"), toast: $("toast"), escLabel: $("esc-label"), footInsert: $("foot-insert"), footCopy: $("foot-copy"),
  settings: $("settings"), settingsMsg: $("settings-msg"), shortcutInput: $("shortcut-input"), settingsCancel: $("settings-cancel"),
};

const MODELS = { haiku: "Haiku · fastest", sonnet: "Sonnet · balanced", opus: "Opus · most capable" };
const DOTS = '<span class="dots" aria-label="Thinking"><i></i><i></i><i></i></span>';

const state = {
  settings: null,
  context: { title: "", process: "", has_screen: false },
  screenOn: true,
  busy: false,
  turn: 0,
  waitingForTurn: false, // the question was sent but its id is not known yet
  early: [],             // answer events that arrived before the id did
  answer: "",          // text of the answer being streamed or last finished
  answerNode: null,
  startFresh: false,   // the engine lost the conversation; clear before the next question
  lastActivity: Date.now(),
  askedAt: 0,
};

const setChip = (chip, icon, text) => {
  chip.querySelector("use").setAttribute("href", "#" + icon);
  chip.querySelector("span").textContent = text;
};

let toastTimer;
function toast(text) {
  el.toast.textContent = text;
  el.toast.classList.add("show");
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => el.toast.classList.remove("show"), 1500);
}

/* ---------- drawing ---------- */

function setBusy(busy) {
  state.busy = busy;
  el.card.classList.toggle("busy", busy);
  el.send.hidden = busy;
  el.stop.hidden = !busy;
  if (state.answerNode) state.answerNode.classList.toggle("streaming", busy);
  refreshActions();
}

function refreshActions() {
  const settingsOpen = !el.settings.hidden;
  const empty = el.thread.children.length === 0;
  const hasAnswer = !state.busy && state.answer.trim().length > 0;
  el.actions.hidden = !hasAnswer || settingsOpen;
  el.footInsert.hidden = el.footCopy.hidden = !hasAnswer || settingsOpen;
  el.newChat.hidden = empty;
  el.suggest.hidden = !empty || settingsOpen || state.busy;
  el.openSettings.classList.toggle("active", settingsOpen);
  el.escLabel.textContent = settingsOpen ? "Close settings" : state.busy ? "Stop" : "Hide";
}

function showNotice(text) {
  el.notice.querySelector("span").textContent = text || "";
  el.notice.hidden = !text;
}

function renderChips() {
  const s = state.settings;
  const c = state.context;
  const app = (c.process || "").replace(/\.exe$/i, "");
  const where = c.title ? `${c.title}` : app;
  if (!s.include_screen) {
    setChip(el.screenChip, "i-eye-off", "Screen off");
    el.screenChip.classList.remove("on");
  } else if (c.has_screen) {
    setChip(el.screenChip, state.screenOn ? "i-eye" : "i-eye-off", state.screenOn ? `Seeing: ${where || "your screen"}` : "Screen not included");
    el.screenChip.classList.toggle("on", state.screenOn);
  } else {
    setChip(el.screenChip, "i-eye-off", el.thread.children.length ? "Screen already sent" : "No screenshot");
    el.screenChip.classList.remove("on");
  }
  setChip(el.modelChip, "i-bolt", modelName());
}

function modelName() {
  const s = state.settings;
  return s.provider === "codex" ? `Codex${s.codex_model ? " · " + s.codex_model : ""}` : `Claude ${MODELS[s.claude_model] || s.claude_model}`;
}

function addMessage(kind, text) {
  const node = document.createElement("div");
  node.className = `msg ${kind}`;
  node.textContent = text;
  el.thread.appendChild(node);
  el.thread.hidden = false;
  el.thread.scrollTop = el.thread.scrollHeight;
  return node;
}

let renderQueued = false;
function renderAnswer() {
  if (renderQueued || !state.answerNode) return;
  renderQueued = true;
  requestAnimationFrame(() => {
    renderQueued = false;
    if (!state.answerNode) return;
    const stick = el.thread.scrollHeight - el.thread.scrollTop - el.thread.clientHeight < 40;
    state.answerNode.innerHTML = state.answer ? window.renderMarkdown(state.answer) : DOTS;
    if (stick) el.thread.scrollTop = el.thread.scrollHeight;
  });
}

function clearThread() {
  el.thread.innerHTML = "";
  el.thread.hidden = true;
  state.answer = "";
  state.answerNode = null;
  state.startFresh = false;
  refreshActions();
}

function growPrompt() {
  el.prompt.style.height = "auto";
  el.prompt.style.height = Math.min(el.prompt.scrollHeight, 116) + "px";
}

/* ---------- actions ---------- */

// `preset` is a quick action: its label is shown, its full prompt is sent.
async function send(preset) {
  if (state.busy || !el.settings.hidden) return;
  const text = preset ? preset.prompt : el.prompt.value.trim();
  if (state.startFresh) clearThread();
  const withScreen = state.settings.include_screen && state.screenOn && state.context.has_screen;
  showNotice("");
  addMessage("user", preset ? preset.label : text || "Help with what's on my screen");
  state.askedAt = performance.now();
  el.stat.textContent = "";
  state.answer = "";
  state.answerNode = addMessage("ai", "");
  state.answerNode.innerHTML = DOTS;
  el.prompt.value = "";
  growPrompt();
  setBusy(true);
  state.lastActivity = Date.now();
  state.waitingForTurn = true;
  state.early = [];
  try {
    state.turn = await invoke("ask", { prompt: text, withScreen });
    state.waitingForTurn = false;
    state.context.has_screen = false; // sent (or dropped) with this question
    renderChips();
    state.early.splice(0).forEach(onAiEvent);
  } catch (err) {
    state.waitingForTurn = false;
    fail(String(err));
  }
}

function fail(message) {
  if (state.answerNode && !state.answer) state.answerNode.remove();
  if (state.answerNode) state.answerNode.classList.remove("streaming");
  state.answerNode = null;
  state.answer = ""; // a cut-off answer is not offered for Insert
  addMessage("error", message).insertAdjacentHTML("afterbegin", '<svg class="icon"><use href="#i-warn"/></svg>');
  state.startFresh = true;
  setBusy(false);
}

async function stop() {
  if (!state.busy) return;
  await invoke("stop");
  state.turn = -1;
  if (state.answerNode && !state.answer) state.answerNode.remove();
  state.startFresh = true; // stopping ends the AI session
  setBusy(false);
}

async function newChat() {
  clearThread();
  setBusy(false);
  showNotice("");
  await invoke("new_chat");
  renderChips();
  el.prompt.focus();
}

async function insertAnswer() {
  if (state.busy || !state.answer.trim()) return;
  try {
    await invoke("insert_text", { text: state.answer.trim() });
  } catch (err) {
    showNotice(String(err));
  }
}

async function copyAnswer() {
  if (!state.answer.trim()) return;
  await invoke("copy_text", { text: state.answer.trim() });
  toast("Copied");
}

async function cycleModel() {
  const s = { ...state.settings };
  if (s.provider !== "claude") return openSettings();
  const names = Object.keys(MODELS);
  s.claude_model = names[(names.indexOf(s.claude_model) + 1) % names.length];
  await saveSettings(s);
}

async function saveSettings(next) {
  const before = state.settings;
  const result = await invoke("save_settings", { settings: next });
  state.settings = result.settings;
  const engineChanged = before.provider !== next.provider || before.claude_model !== next.claude_model || before.codex_model !== next.codex_model;
  if (engineChanged && el.thread.children.length) state.startFresh = true;
  renderChips();
  checkStatus();
  return result.warning;
}

async function checkStatus() {
  const st = await invoke("status");
  showNotice(st.cli_found ? "" : st.detail);
  for (const opt of el.settings.elements.wake_mode.options) {
    if (opt.value !== "shortcut") opt.disabled = !st.hook_supported;
  }
}

/* ---------- settings panel ---------- */

function openSettings() {
  const f = el.settings.elements;
  const s = state.settings;
  for (const name of ["wake_mode", "shortcut", "provider", "claude_model", "codex_model", "claude_path", "codex_path"]) f[name].value = s[name];
  f.keep_warm_minutes.value = String(s.keep_warm_minutes);
  if (!f.keep_warm_minutes.value) f.keep_warm_minutes.value = "5";
  for (const name of ["include_screen", "hide_from_capture", "hide_on_blur", "autostart"]) f[name].checked = s[name];
  el.settingsMsg.hidden = true;
  el.settings.hidden = false;
  el.thread.hidden = true;
  showProviderFields();
  refreshActions();
}

function closeSettings() {
  el.settings.hidden = true;
  el.thread.hidden = el.thread.children.length === 0;
  refreshActions();
  el.prompt.focus();
}

function showProviderFields() {
  const provider = el.settings.elements.provider.value;
  el.settings.querySelectorAll("[data-for]").forEach((n) => (n.hidden = n.dataset.for !== provider));
}

// Turn a key press into an accelerator such as "Ctrl+Alt+Space".
function acceleratorFrom(e) {
  const mods = [];
  if (e.ctrlKey) mods.push("Ctrl");
  if (e.altKey) mods.push("Alt");
  if (e.shiftKey) mods.push("Shift");
  if (e.metaKey) mods.push("Super");
  const code = e.code;
  let key = null;
  if (/^Key[A-Z]$/.test(code)) key = code.slice(3);
  else if (/^Digit\d$/.test(code)) key = code.slice(5);
  else if (/^F\d{1,2}$/.test(code)) key = code;
  else if (/^Arrow/.test(code)) key = code.slice(5);
  else if (["Space", "Enter", "Tab", "Backquote", "Minus", "Equal", "Comma", "Period", "Slash", "Semicolon", "Home", "End", "PageUp", "PageDown", "Insert"].includes(code)) key = code;
  if (!key || mods.length === 0) return null;
  return [...mods, key].join("+");
}

/* ---------- wiring ---------- */

el.prompt.addEventListener("input", growPrompt);
el.send.addEventListener("click", () => send());
el.suggest.addEventListener("click", (e) => {
  const b = e.target.closest("button");
  if (b) send({ label: b.textContent, prompt: b.dataset.prompt });
});
// Copy buttons on code blocks.
el.thread.addEventListener("click", async (e) => {
  const b = e.target.closest(".copy-code");
  if (!b) return;
  await invoke("copy_text", { text: b.closest(".code").querySelector("pre").textContent });
  toast("Code copied");
});
el.card.addEventListener("animationend", (e) => e.animationName === "pop" && el.card.classList.remove("pop"));
el.stop.addEventListener("click", stop);
el.newChat.addEventListener("click", newChat);
el.insert.addEventListener("click", insertAnswer);
el.copy.addEventListener("click", copyAnswer);
el.modelChip.addEventListener("click", cycleModel);
el.openSettings.addEventListener("click", () => (el.settings.hidden ? openSettings() : closeSettings()));
el.settingsCancel.addEventListener("click", closeSettings);
el.settings.elements.provider.addEventListener("change", showProviderFields);
el.screenChip.addEventListener("click", () => {
  if (!state.settings.include_screen || !state.context.has_screen) return;
  state.screenOn = !state.screenOn;
  renderChips();
});

el.shortcutInput.addEventListener("keydown", (e) => {
  if (e.key === "Tab" || e.key === "Escape") return;
  e.preventDefault();
  e.stopPropagation();
  if (e.key === "Backspace" || e.key === "Delete") return void (el.shortcutInput.value = "");
  const acc = acceleratorFrom(e);
  if (acc) el.shortcutInput.value = acc;
});

el.settings.addEventListener("submit", async (e) => {
  e.preventDefault();
  const f = el.settings.elements;
  const next = { ...state.settings };
  for (const name of ["wake_mode", "shortcut", "provider", "claude_model", "codex_model", "claude_path", "codex_path"]) next[name] = f[name].value.trim();
  next.keep_warm_minutes = Number(f.keep_warm_minutes.value) || 5;
  for (const name of ["include_screen", "hide_from_capture", "hide_on_blur", "autostart"]) next[name] = f[name].checked;
  if (next.wake_mode === "shortcut" && !next.shortcut) {
    el.settingsMsg.textContent = "Set a keyboard shortcut, or choose a wake gesture, so you can open Agent Desk.";
    el.settingsMsg.hidden = false;
    return;
  }
  try {
    const warning = await saveSettings(next);
    if (warning) {
      el.settingsMsg.textContent = warning;
      el.settingsMsg.hidden = false;
    } else {
      closeSettings();
    }
  } catch (err) {
    el.settingsMsg.textContent = String(err);
    el.settingsMsg.hidden = false;
  }
});

document.addEventListener("keydown", (e) => {
  const ctrl = e.ctrlKey || e.metaKey;
  if (e.key === "Escape") {
    e.preventDefault();
    if (!el.settings.hidden) closeSettings();
    else if (state.busy) stop();
    else invoke("hide");
  } else if (e.key === "Enter" && ctrl) {
    e.preventDefault();
    insertAnswer();
  } else if (e.key === "Enter" && !e.shiftKey && e.target === el.prompt) {
    e.preventDefault();
    send();
  } else if (ctrl && e.shiftKey && e.key.toLowerCase() === "c") {
    e.preventDefault();
    copyAnswer();
  } else if (ctrl && e.key.toLowerCase() === "n") {
    e.preventDefault();
    newChat();
  } else if (ctrl && e.key === ",") {
    e.preventDefault();
    el.settings.hidden ? openSettings() : closeSettings();
  }
});

// Keep the native window exactly as tall as the card.
new ResizeObserver(() => invoke("resize", { height: Math.ceil(el.shell.getBoundingClientRect().height) })).observe(el.shell);

function onAiEvent(payload) {
  if (state.waitingForTurn) return void state.early.push(payload);
  if (payload.turn !== state.turn) return;
  if (payload.kind === "delta") {
    state.answer += payload.text;
    renderAnswer();
  } else if (payload.kind === "done") {
    state.lastActivity = Date.now();
    el.stat.textContent = `${modelName().split(" · ")[0]} · ${((performance.now() - state.askedAt) / 1000).toFixed(1)}s`;
    setBusy(false);
  } else if (payload.kind === "error") {
    fail(payload.text);
  }
}
listen("ai", ({ payload }) => onAiEvent(payload));

listen("wake", ({ payload }) => {
  // The AI session is closed after it has been idle for a while, so an old
  // conversation cannot be continued; start clean instead of pretending.
  const idleMs = Date.now() - state.lastActivity;
  if (!state.busy && idleMs >= state.settings.keep_warm_minutes * 60000) clearThread();
  state.lastActivity = Date.now();
  state.context = payload;
  state.screenOn = true;
  if (!el.settings.hidden) closeSettings();
  renderChips();
  refreshActions();
  el.card.classList.remove("pop");
  void el.card.offsetWidth; // restart the entrance animation
  el.card.classList.add("pop");
  el.prompt.focus();
  el.prompt.select();
});
listen("refocus", () => el.prompt.focus());
listen("open-settings", openSettings);
listen("settings-changed", ({ payload }) => {
  state.settings = payload;
  renderChips();
});

(async function init() {
  state.settings = await invoke("get_settings");
  renderChips();
  refreshActions();
  growPrompt();
  checkStatus();
  el.prompt.focus();
})();
