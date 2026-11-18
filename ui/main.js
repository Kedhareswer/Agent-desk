// Overlay behaviour. All system work (keys, microphone, speech-to-text,
// screenshots, the AI process) happens in the Rust side; this file draws
// the notch and the answer panel.
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (id) => document.getElementById(id);
const el = {
  shell: $("shell"), notch: $("notch"), content: $("content"), topLine: $("top-line"), label: $("label"), prompt: $("prompt"), meter: $("meter"),
  stop: $("stop"), openSettings: $("open-settings"), close: $("close"),
  panel: $("panel"), notice: $("notice"), thread: $("thread"), actions: $("actions"),
  insert: $("insert"), copy: $("copy"), newChat: $("new-chat"), stat: $("stat"),
  settings: $("settings"), settingsMsg: $("settings-msg"), shortcutInput: $("shortcut-input"), settingsCancel: $("settings-cancel"), opacityOut: $("opacity-out"),
};

// Each state has a label and the colour the notch glows in.
const STATES = {
  idle: ["", "#8ab4ff"],
  listening: ["Listening", "#ffc18f"],
  transcribing: ["Transcribing", "#ff99a8"],
  preparing: ["Getting ready", "#ffeb85"],
  thinking: ["Thinking", "#c299ff"],
  answering: ["Answering", "#82ff9e"],
  done: ["Done", "#8ab4ff"],
  error: ["Couldn't do that", "#ff8f8f"],
};
const MODELS = { haiku: "Haiku", sonnet: "Sonnet", opus: "Opus" };
const TEXT_FIELDS = ["wake_mode", "shortcut", "provider", "claude_model", "codex_model", "claude_path", "codex_path", "voice_model", "dock_edge"];
const TOGGLES = ["include_screen", "hide_from_capture", "hide_on_blur", "autostart", "edge_glow"];
const METER_COLUMNS = 10;

const state = {
  settings: null,
  context: { title: "", process: "", has_screen: false },
  phase: "idle",
  busy: false,
  turn: 0,
  early: [],           // answer events that arrived before their question was announced
  answer: "",          // text of the answer being streamed or last finished
  answerNode: null,
  startFresh: false,   // the engine lost the conversation; clear before the next question
  lastActivity: Date.now(),
  askedAt: 0,
  levels: new Array(METER_COLUMNS).fill(0),
};

/* ---------- notch ---------- */

function hexToRgba(hex, alpha) {
  const n = parseInt(hex.slice(1), 16);
  return `rgba(${n >> 16}, ${(n >> 8) & 255}, ${n & 255}, ${alpha})`;
}

// Show a state in the notch. `top` is the small grey line above the label.
function setPhase(phase, top, label) {
  const [text, colour] = STATES[phase] || STATES.idle;
  state.phase = phase;
  el.notch.dataset.state = phase;
  el.shell.style.setProperty("--glow", colour);
  el.shell.style.setProperty("--glow-dim", hexToRgba(colour, 0.22));
  el.label.textContent = label || text;
  if (top !== undefined) el.topLine.textContent = top;
  el.stop.hidden = !state.busy;
  // Idle shows the text field; anything else shows the status label.
  setView(phase === "idle" ? "input" : "status");
}

function setView(view) {
  el.notch.dataset.view = view;
  if (view === "input") growPrompt();
}

function contextLine() {
  const c = state.context;
  if (!state.settings.include_screen || !c.has_screen) return "";
  const app = (c.process || "").replace(/\.exe$/i, "");
  return `Seeing ${c.title || app || "your screen"}`;
}

function modelName() {
  const s = state.settings;
  return s.provider === "codex" ? `Codex${s.codex_model ? " " + s.codex_model : ""}` : `Claude ${MODELS[s.claude_model] || s.claude_model}`;
}

function buildMeter() {
  el.meter.innerHTML = "<i></i>".repeat(METER_COLUMNS * 3);
}

// Columns scroll left; each lights 0 to 3 dots from the bottom up.
function pushLevel(level) {
  state.levels.push(level);
  state.levels.shift();
  const dots = el.meter.children;
  state.levels.forEach((v, col) => {
    const lit = Math.min(3, Math.round(v * 3.4));
    for (let row = 0; row < 3; row++) dots[col * 3 + row].classList.toggle("lit", 2 - row < lit);
  });
}

function applyLook() {
  const s = state.settings;
  if (el.shell.dataset.edge !== s.dock_edge) {
    el.shell.dataset.edge = s.dock_edge;
    windowSize.w = windowSize.h = 0; // margins differ per edge: size afresh
    morph();
  }
  document.documentElement.style.setProperty("--alpha", s.opacity);
}

/* ---------- panel ---------- */

function refreshPanel() {
  const settingsOpen = !el.settings.hidden;
  const hasThread = el.thread.children.length > 0;
  const hasAnswer = !state.busy && state.answer.trim().length > 0;
  el.thread.hidden = !hasThread || settingsOpen;
  el.actions.hidden = !hasAnswer || settingsOpen;
  el.panel.hidden = el.thread.hidden && el.actions.hidden && el.notice.hidden && !settingsOpen;
  el.notch.classList.toggle("settings-open", settingsOpen);
  el.openSettings.classList.toggle("active", settingsOpen);
  el.stop.hidden = !state.busy;
}

function showNotice(text) {
  el.notice.textContent = text || "";
  el.notice.hidden = !text;
  refreshPanel();
}

function addMessage(kind, text) {
  const node = document.createElement("div");
  node.className = `msg ${kind}`;
  node.textContent = text;
  el.thread.appendChild(node);
  refreshPanel();
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
    state.answerNode.hidden = !state.answer;
    state.answerNode.innerHTML = window.renderMarkdown(state.answer);
    if (stick) el.thread.scrollTop = el.thread.scrollHeight;
  });
}

function clearThread() {
  el.thread.innerHTML = "";
  state.answer = "";
  state.answerNode = null;
  state.startFresh = false;
  refreshPanel();
}

function growPrompt() {
  el.prompt.style.height = "auto";
  // While the field is hidden it measures as zero; leave the height alone then.
  const h = el.prompt.scrollHeight;
  el.prompt.style.height = h ? Math.min(h, 84) + "px" : "";
}

function flash(text) {
  const before = el.stat.textContent;
  el.stat.textContent = text;
  el.stat.classList.add("flash");
  setTimeout(() => {
    el.stat.classList.remove("flash");
    if (el.stat.textContent === text) el.stat.textContent = before;
  }, 1400);
}

/* ---------- asking ---------- */

// Put a new question on screen. The answer fills in as events arrive.
function beginTurn(shown) {
  if (state.startFresh) clearThread();
  showNotice("");
  if (!el.settings.hidden) closeSettings();
  // The notch's top line already shows the current question, so only
  // earlier questions are listed in the conversation.
  el.thread.querySelector(".msg.user.current")?.classList.remove("current");
  addMessage("user", shown).classList.add("current");
  state.answer = "";
  state.answerNode = addMessage("ai", "");
  state.answerNode.hidden = true;
  state.askedAt = performance.now();
  state.lastActivity = Date.now();
  el.stat.textContent = "";
  state.busy = true;
  setPhase("thinking", shown);
  refreshPanel();
}

// The turn id is now known: accept its events, including any early ones.
function adoptTurn(turn) {
  state.turn = turn;
  state.context.has_screen = false; // sent with this question
  const waiting = state.early.splice(0);
  waiting.forEach(onAiEvent);
}

async function send() {
  if (state.busy || !el.settings.hidden) return;
  const text = el.prompt.value.trim();
  const withScreen = state.settings.include_screen && state.context.has_screen;
  beginTurn(text || "Help with what's on my screen");
  el.prompt.value = "";
  growPrompt();
  state.turn = Infinity; // hold events until the id is known
  try {
    adoptTurn(await invoke("ask", { prompt: text, withScreen }));
  } catch (err) {
    state.turn = 0;
    fail(String(err));
  }
}

function onAiEvent(payload) {
  if (payload.turn > state.turn || state.turn === Infinity) return void state.early.push(payload);
  if (payload.turn !== state.turn) return;
  if (payload.kind === "delta") {
    if (!state.answer) {
      setPhase("answering");
      state.answerNode?.classList.add("streaming");
    }
    state.answer += payload.text;
    renderAnswer();
  } else if (payload.kind === "done") {
    state.lastActivity = Date.now();
    state.busy = false;
    state.answerNode?.classList.remove("streaming");
    el.stat.textContent = `${modelName()} · ${((performance.now() - state.askedAt) / 1000).toFixed(1)}s`;
    setPhase("done");
    refreshPanel();
  } else if (payload.kind === "error") {
    fail(payload.text);
  }
}

function fail(message) {
  if (state.answerNode && !state.answer) state.answerNode.remove();
  state.answerNode?.classList.remove("streaming");
  state.answerNode = null;
  state.answer = ""; // a cut-off answer is not offered for Insert
  state.busy = false;
  state.startFresh = true;
  addMessage("error", message);
  setPhase("error");
  refreshPanel();
}

async function stop() {
  if (!state.busy) return;
  await invoke("stop");
  state.turn = -1;
  if (state.answerNode && !state.answer) state.answerNode.remove();
  state.answerNode?.classList.remove("streaming");
  state.busy = false;
  state.startFresh = true; // stopping ends the AI session
  setPhase("idle", contextLine());
  refreshPanel();
  focusPrompt();
}

async function newChat() {
  clearThread();
  state.busy = false;
  showNotice("");
  setPhase("idle", contextLine());
  await invoke("new_chat");
  focusPrompt();
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
  flash("Copied");
}

// Switch the notch to its text field and take keyboard focus.
async function focusPrompt() {
  if (state.busy) return;
  if (state.phase !== "idle") setPhase("idle", contextLine());
  await invoke("focus");
  el.prompt.focus();
}

/* ---------- settings ---------- */

async function saveSettings(next) {
  const before = state.settings;
  const result = await invoke("save_settings", { settings: next });
  state.settings = result.settings;
  const engineChanged = before.provider !== next.provider || before.claude_model !== next.claude_model || before.codex_model !== next.codex_model;
  if (engineChanged && el.thread.children.length) state.startFresh = true;
  applyLook();
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

async function openSettings() {
  const f = el.settings.elements;
  const s = state.settings;
  for (const name of TEXT_FIELDS) f[name].value = s[name];
  f.keep_warm_minutes.value = String(s.keep_warm_minutes);
  if (!f.keep_warm_minutes.value) f.keep_warm_minutes.value = "5";
  f.opacity.value = s.opacity;
  showOpacity();
  for (const name of TOGGLES) f[name].checked = s[name];
  el.settingsMsg.hidden = true;
  el.settings.hidden = false;
  showProviderFields();
  refreshPanel();
  await invoke("focus");
}

function closeSettings() {
  el.settings.hidden = true;
  // Undo an unsaved transparency preview.
  applyLook();
  refreshPanel();
}

function showProviderFields() {
  const provider = el.settings.elements.provider.value;
  el.settings.querySelectorAll("[data-for]").forEach((n) => (n.hidden = n.dataset.for !== provider));
}

function showOpacity() {
  const v = Number(el.settings.elements.opacity.value);
  el.opacityOut.textContent = `${Math.round((1 - v) * 100)}% see-through`;
  document.documentElement.style.setProperty("--alpha", v); // live preview
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
  else if (["Space", "Tab", "Backquote", "Minus", "Equal", "Comma", "Period", "Slash", "Semicolon", "Home", "End", "PageUp", "PageDown", "Insert"].includes(code)) key = code;
  if (!key || mods.length === 0) return null;
  return [...mods, key].join("+");
}

/* ---------- wiring ---------- */

el.prompt.addEventListener("input", growPrompt);
el.stop.addEventListener("click", stop);
el.close.addEventListener("click", () => invoke("hide"));
el.newChat.addEventListener("click", newChat);
el.insert.addEventListener("click", insertAnswer);
el.copy.addEventListener("click", copyAnswer);
el.openSettings.addEventListener("click", () => (el.settings.hidden ? openSettings() : closeSettings()));
el.settingsCancel.addEventListener("click", closeSettings);
el.settings.elements.provider.addEventListener("change", showProviderFields);
el.settings.elements.opacity.addEventListener("input", showOpacity);

// Clicking the notch's status line switches to typing.
$("row").addEventListener("click", (e) => {
  if (e.target.closest("button")) return;
  if (el.notch.dataset.view === "status" || document.activeElement !== el.prompt) focusPrompt();
});

// Copy buttons on code blocks.
el.thread.addEventListener("click", async (e) => {
  const b = e.target.closest(".copy-code");
  if (!b) return;
  await invoke("copy_text", { text: b.closest(".code").querySelector("pre").textContent });
  flash("Code copied");
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
  for (const name of TEXT_FIELDS) next[name] = f[name].value.trim();
  next.keep_warm_minutes = Number(f.keep_warm_minutes.value) || 5;
  next.opacity = Number(f.opacity.value);
  for (const name of TOGGLES) next[name] = f[name].checked;
  if (next.dock_edge !== state.settings.dock_edge) next.dock_along = 0.5;
  const warn = (text) => {
    el.settingsMsg.textContent = text;
    el.settingsMsg.hidden = false;
  };
  if (next.wake_mode === "shortcut" && !next.shortcut) return warn("Set a shortcut, or choose wake keys, so you can open Agent Desk.");
  try {
    const warning = await saveSettings(next);
    if (warning) warn(warning);
    else closeSettings();
  } catch (err) {
    warn(String(err));
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

/* ---------- morphing ---------- */

// The notch is one shape. Its content is laid out at full size inside it,
// and the shape animates to fit: small while working, larger when there is
// something to read. The native window grows before the shape does and
// shrinks after, so the animation is never cut off.
const windowSize = { w: 0, h: 0 };
let shrinkTimer;

function margins() {
  const cs = getComputedStyle(el.shell);
  return { x: parseFloat(cs.paddingLeft) + parseFloat(cs.paddingRight), y: parseFloat(cs.paddingTop) + parseFloat(cs.paddingBottom) };
}

function setWindow(w, h) {
  if (w === windowSize.w && h === windowSize.h) return;
  windowSize.w = w;
  windowSize.h = h;
  el.shell.style.width = w + "px";
  el.shell.style.height = h + "px";
  invoke("resize", { width: w, height: h });
}

function morph() {
  const w = Math.ceil(el.content.offsetWidth);
  const h = Math.ceil(el.content.offsetHeight);
  el.notch.style.width = w + "px";
  el.notch.style.height = h + "px";
  const m = margins();
  const need = { w: w + m.x, h: h + m.y };
  clearTimeout(shrinkTimer);
  setWindow(Math.max(windowSize.w, need.w), Math.max(windowSize.h, need.h));
  if (need.w < windowSize.w || need.h < windowSize.h) shrinkTimer = setTimeout(() => setWindow(need.w, need.h), 460);
}
new ResizeObserver(morph).observe(el.content);

listen("ai", ({ payload }) => onAiEvent(payload));

// The overlay was opened (by a tap, a hold, or the tray).
listen("wake", ({ payload }) => {
  // The AI session is closed after it has been idle for a while, so an old
  // conversation cannot be continued; start clean instead of pretending.
  const idleMs = Date.now() - state.lastActivity;
  if (!state.busy && idleMs >= state.settings.keep_warm_minutes * 60000) clearThread();
  state.lastActivity = Date.now();
  state.context = payload;
  if (!el.settings.hidden) closeSettings();
  if (payload.typing && !state.busy) {
    setPhase("idle", contextLine());
    el.prompt.focus();
    el.prompt.select();
  }
});
listen("refocus", ({ payload }) => {
  if (payload && !state.busy) {
    setPhase("idle", contextLine());
    el.prompt.focus();
  }
});

// Push-to-talk progress from the Rust side.
listen("phase", ({ payload }) => {
  const { phase, text } = payload;
  if (phase === "listening") {
    state.levels.fill(0);
    pushLevel(0);
    showNotice("");
    if (!el.settings.hidden) closeSettings();
    setPhase("listening", contextLine());
  } else if (phase === "transcribing") {
    setPhase("transcribing", "");
  } else if (phase === "preparing") {
    setPhase("preparing", text);
  } else if (phase === "cancelled") {
    // The keys turned out to be an ordinary shortcut. Go back to how it was.
    if (el.panel.hidden) invoke("hide");
    else setPhase(state.busy ? "thinking" : state.answer ? "done" : "idle", "");
  } else if (phase === "error") {
    state.busy = false;
    setPhase("error", "");
    showNotice(text);
  }
});
listen("level", ({ payload }) => state.phase === "listening" && pushLevel(payload));

// Speech was understood and sent as a question.
listen("asked", ({ payload }) => {
  beginTurn(payload.text);
  adoptTurn(payload.turn);
});

listen("insert-request", insertAnswer);
listen("open-settings", openSettings);
listen("settings-changed", ({ payload }) => {
  state.settings = payload;
  applyLook();
});

(async function init() {
  state.settings = await invoke("get_settings");
  buildMeter();
  applyLook();
  setPhase("idle", "");
  refreshPanel();
  growPrompt();
  checkStatus();
})();
