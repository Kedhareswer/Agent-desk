// Overlay behaviour. All system work (keys, microphone, speech-to-text,
// screenshots, the AI process) happens in the Rust side; this file draws
// the notch and the answer panel.
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (id) => document.getElementById(id);
const el = {
  shell: $("shell"), notch: $("notch"), content: $("content"), topLine: $("top-line"), label: $("label"), prompt: $("prompt"),
  stop: $("stop"), openSettings: $("open-settings"), close: $("close"),
  panel: $("panel"), notice: $("notice"), thread: $("thread"), actions: $("actions"),
  insert: $("insert"), copy: $("copy"), newChat: $("new-chat"), stat: $("stat"),
  setup: $("setup"),
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
  confirm: ["Needs your OK", "#ffd58a"],
  acting: ["Working on it", "#9fe3c4"],
  live: ["Working with you", "#7fe3ff"],
  error: ["Couldn't do that", "#ff8f8f"],
};
const MODELS = { haiku: "Haiku", sonnet: "Sonnet", opus: "Opus" };
const TEXT_FIELDS = ["wake_mode", "shortcut", "live_shortcut", "provider", "claude_model", "codex_model", "claude_path", "codex_path", "voice_model", "dock_edge"];
const TOGGLES = ["include_screen", "hide_from_capture", "hide_on_blur", "autostart", "edge_glow", "live_system_audio", "auto_update", "actions_enabled"];
const DAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const ACTION_STATUS = { ask: "", running: "Working…", done: "Done", denied: "Not done", failed: "Failed" };

const state = {
  settings: null,
  context: { title: "", process: "", has_screen: false },
  phase: "idle",
  busy: false,
  turn: 0,
  early: [],           // answer events that arrived before their question was announced
  answer: "",          // text of the answer being streamed or last finished
  segment: "",         // the part of it after the last action, shown in answerNode
  answerNode: null,
  actionNodes: {},     // action id → its row in the thread
  asking: 0,           // id of the action waiting for the user's OK
  draft: { allow: [], commands: [] }, // settings lists while editing
  schedules: [],
  editing: null,       // schedule being edited (id 0 = new)
  startFresh: false,   // the engine lost the conversation; clear before the next question
  lastActivity: Date.now(),
  askedAt: 0,
  live: false,         // "work with me" mode is on
  quiet: null,         // a live check-in whose answer is not shown yet
  tucked: false,       // the finished answer was put away; only the notch remains
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

// What the notch shows when nothing is in progress.
function rest(top) {
  if (state.live) setPhase("live", top ?? el.topLine.textContent);
  else setPhase(state.answer.trim() ? "done" : "idle", top ?? contextLine());
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
  const settingsOpen = !el.settings.hidden || !el.setup.hidden;
  const hasThread = !state.tucked && [...el.thread.children].some((n) => !n.hidden && !n.classList.contains("current"));
  const hasAnswer = !state.tucked && !state.busy && state.answer.trim().length > 0;
  el.thread.hidden = !hasThread || settingsOpen;
  el.actions.hidden = !hasAnswer || settingsOpen;
  el.panel.hidden = el.thread.hidden && el.actions.hidden && el.notice.hidden && !settingsOpen;
  el.notch.classList.toggle("settings-open", settingsOpen);
  el.openSettings.classList.toggle("active", settingsOpen);
  el.stop.hidden = !state.busy;
}

/* ---------- putting a finished answer away ---------- */

// A finished answer stays on screen for a few seconds, then the notch
// shrinks back and tucks away. The mouse over it keeps it open; so do
// typing, settings, a step waiting for an OK, errors, notices and live mode.
let tuckTimer = 0;
const TUCK_MS = () => window.__tuckMs ?? 5000;
const AFTER_HOVER_MS = () => window.__tuckAfterHoverMs ?? 2000;

function canTuck() {
  const typing = document.activeElement === el.prompt && el.notch.dataset.view === "input";
  return state.phase === "done" && !state.busy && !state.live && !state.asking && !typing &&
    el.settings.hidden && el.setup.hidden && el.notice.hidden;
}

function scheduleTuck(ms = TUCK_MS()) {
  clearTimeout(tuckTimer);
  tuckTimer = setTimeout(tuck, ms);
}

function tuck() {
  tuckTimer = 0;
  if (!canTuck()) return;
  if (el.notch.matches(":hover")) return void (tuckTimer = -1); // leaving it starts the clock again
  state.tucked = true;
  refreshPanel(); // the shape shrinks back to the notch...
  setTimeout(() => {
    if (!state.tucked || !canTuck()) return;
    if (el.notch.matches(":hover")) tuckTimer = -1; // the mouse came back: go once it leaves
    else invoke("hide"); // ...then goes
  }, 480);
}

function untuck() {
  clearTimeout(tuckTimer);
  tuckTimer = 0;
  if (!state.tucked) return;
  state.tucked = false;
  refreshPanel();
}

el.notch.addEventListener("mouseenter", () => {
  if (tuckTimer > 0) {
    clearTimeout(tuckTimer);
    tuckTimer = -1;
  }
});
el.notch.addEventListener("mouseleave", () => tuckTimer === -1 && scheduleTuck(AFTER_HOVER_MS()));

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
    const wasHidden = state.answerNode.hidden;
    const text = state.segment.trimStart();
    state.answerNode.hidden = !text;
    state.answerNode.innerHTML = window.renderMarkdown(text);
    if (wasHidden !== state.answerNode.hidden) refreshPanel(); // the notch grows now
    if (stick) el.thread.scrollTop = el.thread.scrollHeight;
  });
}

function clearThread() {
  el.thread.innerHTML = "";
  state.answer = "";
  state.segment = "";
  state.answerNode = null;
  state.actionNodes = {};
  state.asking = 0;
  state.startFresh = false;
  state.tucked = false;
  clearTimeout(tuckTimer);
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
// A `quiet` turn is a live-mode check-in: nothing appears unless the AI
// actually has something to say.
function beginTurn(shown, quiet = false) {
  if (state.startFresh) clearThread();
  if (!quiet) showNotice("");
  if (!quiet) closeOverlays();
  untuck();
  // Only the latest question and answer are shown; the AI still remembers
  // the earlier ones. The question itself is in the notch's top line.
  // A live check-in keeps the last answer until it has something to say.
  const old = [...el.thread.children];
  if (!quiet) old.forEach((n) => n.remove());
  const before = { answer: state.answer, node: state.answerNode, old };
  const userNode = addMessage("user", shown);
  userNode.classList.add("current");
  state.answer = "";
  state.segment = "";
  state.asking = 0;
  state.answerNode = addMessage("ai", "");
  state.answerNode.hidden = true;
  state.askedAt = performance.now();
  state.lastActivity = Date.now();
  state.busy = true;
  if (quiet) {
    userNode.hidden = true;
    state.quiet = { userNode, before };
    refreshPanel();
  } else {
    state.quiet = null;
    el.stat.textContent = "";
    setPhase("thinking", shown);
    refreshPanel();
  }
}

// In live mode the AI answers "." when it has nothing to add.
const nothingToSay = (text) => text.replace(/[.\s]/g, "") === "";

// The turn id is now known: accept its events, including any early ones.
function adoptTurn(turn) {
  state.turn = turn;
  state.context.has_screen = false; // sent with this question
  const waiting = state.early.splice(0);
  waiting.forEach((p) => ("status" in p ? onAction(p) : onAiEvent(p)));
}

/* ---------- actions on the computer ---------- */

// One step the AI takes. It appears in the answer, in order, before it
// runs; a risky one waits here for Allow.
function onAction(p) {
  if (p.turn > state.turn || state.turn === Infinity) return void state.early.push(p);
  if (p.turn !== state.turn) return;
  let node = state.actionNodes[p.id];
  if (!node) {
    node = document.createElement("div");
    node.className = "msg act";
    node.dataset.id = p.id;
    node.innerHTML = '<div class="act-line"><span class="act-dot"></span><span class="act-text"></span><span class="act-status"></span></div>';
    node.querySelector(".act-text").textContent = p.summary;
    node.querySelector(".act-text").title = p.summary;
    // Text written so far stays above the step; what comes next goes below.
    if (state.answerNode && state.segment.trim()) {
      state.answerNode.classList.remove("streaming");
      // Show it now: the queued render would draw into the next node.
      state.answerNode.innerHTML = window.renderMarkdown(state.segment.trimStart());
      state.answerNode.hidden = false;
      el.thread.appendChild(node);
      state.answerNode = addMessage("ai", "");
      state.answerNode.hidden = true;
      state.segment = "";
    } else if (state.answerNode) {
      el.thread.insertBefore(node, state.answerNode);
    } else {
      el.thread.appendChild(node);
    }
    state.actionNodes[p.id] = node;
  }
  node.dataset.status = p.status;
  node.querySelector(".act-status").textContent = p.status === "failed" ? p.result || "Failed" : ACTION_STATUS[p.status] ?? "";
  node.querySelector(".act-detail")?.remove();
  node.querySelector(".act-buttons")?.remove();
  if (p.status === "ask") {
    if (p.detail && p.detail !== p.summary) {
      const pre = document.createElement("pre");
      pre.className = "act-detail";
      pre.textContent = p.detail;
      node.appendChild(pre);
    }
    const buttons = document.createElement("div");
    buttons.className = "act-buttons";
    buttons.innerHTML = `<button class="primary" data-decision="allow">Allow</button>${p.always === false ? "" : '<button data-decision="always">Always allow</button>'}<button data-decision="deny">Don't</button>`;
    node.appendChild(buttons);
    state.asking = p.id;
    setPhase("confirm"); // the step itself is in the panel; the question stays on top
  } else {
    if (state.asking === p.id) state.asking = 0;
    if (p.status === "running") setPhase("acting");
    else if (state.busy) setPhase("thinking");
  }
  refreshPanel();
  el.thread.scrollTop = el.thread.scrollHeight;
}

async function answerAction(id, decision) {
  const node = state.actionNodes[id];
  node?.querySelector(".act-buttons")?.remove();
  if (state.asking === id) state.asking = 0;
  await invoke("answer_action", { id, decision });
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
    state.answer += payload.text;
    state.segment += payload.text;
    if (state.quiet) {
      if (nothingToSay(state.answer)) return; // still nothing worth showing
      state.quiet.userNode.hidden = false;
      state.quiet.before.old.forEach((n) => n.remove());
      el.topLine.textContent = state.quiet.userNode.textContent;
      state.quiet = null;
      el.stat.textContent = "";
    }
    if (state.phase !== "answering") {
      setPhase("answering");
      state.answerNode?.classList.add("streaming");
    }
    renderAnswer();
  } else if (payload.kind === "done") {
    state.lastActivity = Date.now();
    state.busy = false;
    if (state.quiet) {
      // Nothing to add: remove the placeholders and keep the last answer.
      state.quiet.userNode.remove();
      state.answerNode?.remove();
      state.answer = state.quiet.before.answer;
      state.segment = state.answer;
      state.answerNode = state.quiet.before.node;
      state.quiet = null;
    } else {
      state.answerNode?.classList.remove("streaming");
      if (state.answerNode && !state.segment.trim()) state.answerNode.remove(); // ended on an action
      el.stat.textContent = `${modelName()} · ${((performance.now() - state.askedAt) / 1000).toFixed(1)}s`;
    }
    rest();
    refreshPanel();
    morph(); // fit the window exactly now that it has stopped growing
    if (state.phase === "done") scheduleTuck();
  } else if (payload.kind === "error") {
    fail(payload.text);
  }
}

function fail(message) {
  if (state.answerNode && !state.answer) state.answerNode.remove();
  state.answerNode?.classList.remove("streaming");
  state.quiet?.userNode.remove();
  state.quiet = null;
  state.answerNode = null;
  state.answer = ""; // a cut-off answer is not offered for Insert
  state.busy = false;
  state.startFresh = true;
  addMessage("error", message);
  setPhase("error");
  refreshPanel();
  morph();
}

async function stop() {
  if (!state.busy) return;
  await invoke("stop");
  state.asking = 0;
  el.thread.querySelectorAll(".act-buttons").forEach((n) => n.remove());
  state.turn = -1;
  if (state.answerNode && !state.answer) state.answerNode.remove();
  state.answerNode?.classList.remove("streaming");
  state.busy = false;
  state.startFresh = true; // stopping ends the AI session
  state.quiet = null;
  morph();
  if (state.live) return void rest();
  setPhase("idle", contextLine());
  refreshPanel();
  focusPrompt();
}

async function newChat() {
  // In live mode this only clears what is on screen: the AI keeps what it
  // has heard so far.
  if (state.live) {
    clearThread();
    showNotice("");
    return void rest();
  }
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
  if (state.busy && !state.quiet) return;
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
  state.draft = { allow: [...(s.always_allow || [])], commands: (s.commands || []).map((c) => ({ ...c })), profiles: { ...(s.browser_profiles || {}) } };
  renderAllowed();
  renderProfiles();
  renderCommands();
  closeScheduleEditor();
  loadSchedules();
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

/* ---------- first-run setup ---------- */

// The setup screen asks the app (and through it the CLIs) for the facts:
// is the tool installed, who is signed in, which models, which speech
// models are downloaded. Nothing is saved until "Start using Agent Desk".
const setup = { info: null, models: {}, provider: "claude", modelPick: {} };

function closeOverlays() {
  if (!el.settings.hidden) closeSettings();
  if (!el.setup.hidden) closeSetup();
}

async function openSetup() {
  if (!el.settings.hidden) closeSettings();
  const s = state.settings;
  setup.provider = s.provider;
  setup.modelPick = { claude: s.claude_model, codex: s.codex_model };
  $("setup-provider").value = s.provider;
  $("setup-actions").checked = s.actions_enabled;
  $("setup-screen").checked = s.include_screen;
  $("setup-autostart").checked = s.autostart;
  $("setup-msg").hidden = true;
  showAccountField();
  el.setup.hidden = false;
  refreshPanel();
  await invoke("focus");
  checkSetup();
}

function closeSetup() {
  el.setup.hidden = true;
  refreshPanel();
}

function showAccountField() {
  const s = state.settings;
  const codex = setup.provider === "codex";
  $("setup-account").value = codex ? s.codex_home : s.claude_config_dir;
  $("setup-account").placeholder = codex ? "Your usual Codex sign-in (~/.codex)" : "Your usual Claude sign-in (~/.claude)";
}

function setCheck(id, ok, title, detail) {
  const row = $(id);
  row.dataset.ok = ok;
  row.querySelector("b").textContent = title;
  row.querySelector("small").textContent = detail || "";
}

async function checkSetup() {
  $("setup-checking").textContent = "Checking…";
  setCheck("check-cli", "wait", "Command-line tool", "Looking for it…");
  setCheck("check-account", "wait", "Signed in", "Asking the tool…");
  try {
    setup.info = await invoke("setup_info");
  } catch (err) {
    setup.info = null;
    $("setup-checking").textContent = String(err);
    return;
  }
  $("setup-checking").textContent = "";
  renderSetupChecks();
  renderSpeech();
  loadModels(setup.provider);
}

function renderSetupChecks() {
  if (!setup.info) return;
  const cli = setup.info[setup.provider];
  const name = setup.provider === "codex" ? "Codex CLI" : "Claude Code";
  const version = (cli.version || "").replace(/\(.*?\)/g, "").replace(/^codex-cli\s*/i, "").trim();
  if (cli.found) setCheck("check-cli", "yes", `${name} ${version}`.trim(), cli.path);
  else setCheck("check-cli", "no", `${name} is not installed`, cli.problem);
  if (!cli.found) setCheck("check-account", "no", "Signed in", "Install it first.");
  else if (cli.signed_in) {
    const plan = cli.plan ? ` · ${cli.plan} plan` : "";
    setCheck("check-account", "yes", `Signed in${cli.account ? " as " + cli.account : ""}${plan}`, `Sign-in kept in ${cli.config_dir}`);
  } else setCheck("check-account", "no", "Not signed in", `Copy the sign-in command, run it in a terminal, then press Check again.`);
  $("copy-signin").hidden = cli.found && cli.signed_in;
  $("copy-signin").dataset.command = cli.sign_in_command;
}

async function loadModels(provider) {
  const select = $("setup-model");
  if (!setup.models[provider]) {
    select.innerHTML = "<option>Loading models from the CLI…</option>";
    select.disabled = true;
    try { setup.models[provider] = await invoke("list_models", { provider }); } catch { setup.models[provider] = []; }
    select.disabled = false;
  }
  if (provider !== setup.provider) return;
  const list = setup.models[provider];
  select.innerHTML = "";
  if (provider === "codex") select.add(new Option("Your Codex default", ""));
  for (const m of list) select.add(new Option(m.name, m.id));
  select.value = setup.modelPick[provider] ?? "";
  if (select.selectedIndex < 0) select.selectedIndex = 0;
  showModelDesc();
}

function showModelDesc() {
  const id = $("setup-model").value;
  const m = (setup.models[setup.provider] || []).find((x) => x.id === id);
  $("model-desc").textContent = m ? m.description : id ? "" : "Whatever your Codex CLI is set to use.";
}

function renderSpeech() {
  const box = $("speech-list");
  box.innerHTML = "";
  const current = setup.voicePick || state.settings.voice_model;
  for (const m of setup.info?.speech || []) {
    const row = document.createElement("label");
    row.className = "speech-row";
    row.dataset.model = m.id;
    const radio = Object.assign(document.createElement("input"), { type: "radio", name: "setup-voice", value: m.id, checked: m.id === current });
    radio.addEventListener("change", () => (setup.voicePick = m.id));
    const text = document.createElement("div");
    text.innerHTML = `<div></div><div class="bar" hidden><i></i></div>`;
    text.firstChild.textContent = `${m.label} · ${m.size_mb} MB`;
    const stateBox = document.createElement("div");
    stateBox.className = "state";
    row.append(radio, text, stateBox);
    box.appendChild(row);
    const downloading = setup.info.downloading === m.id;
    setSpeechState(m.id, m.downloaded ? "done" : downloading ? setup.info.download_pct : m.partial ? `paused:${m.partial}` : "none");
  }
}

// One speech model row: downloaded, downloading (a number), paused, or not yet.
function setSpeechState(id, st) {
  const row = $("speech-list").querySelector(`[data-model="${id}"]`);
  if (!row) return;
  const box = row.querySelector(".state");
  const bar = row.querySelector(".bar");
  box.className = "state";
  box.innerHTML = "";
  bar.hidden = true;
  if (st === "done") {
    box.textContent = "Downloaded";
    box.classList.add("ok");
  } else if (typeof st === "number") {
    box.textContent = `Downloading ${st}%`;
    bar.hidden = false;
    bar.firstChild.style.width = st + "%";
  } else {
    const pct = String(st).startsWith("paused:") ? Number(String(st).slice(7)) : 0;
    const b = Object.assign(document.createElement("button"), { type: "button", className: "small", textContent: pct ? `Resume (${pct}%)` : "Download" });
    b.addEventListener("click", async (e) => {
      e.preventDefault();
      try {
        await invoke("download_speech", { model: id });
        setSpeechState(id, pct);
      } catch (err) {
        box.textContent = String(err);
      }
    });
    box.appendChild(b);
  }
}

async function finishSetup(save) {
  const next = { ...state.settings, setup_done: true };
  if (save) {
    next.provider = setup.provider;
    const model = $("setup-model").value;
    if (setup.provider === "codex") {
      next.codex_model = model;
      next.codex_home = $("setup-account").value.trim();
    } else {
      if (model) next.claude_model = model;
      next.claude_config_dir = $("setup-account").value.trim();
    }
    next.voice_model = setup.voicePick || next.voice_model;
    next.actions_enabled = $("setup-actions").checked;
    next.include_screen = $("setup-screen").checked;
    next.autostart = $("setup-autostart").checked;
  }
  try {
    const warning = await saveSettings(next);
    if (warning) {
      $("setup-msg").textContent = warning;
      $("setup-msg").hidden = false;
      return;
    }
    closeSetup();
    focusPrompt();
  } catch (err) {
    $("setup-msg").textContent = String(err);
    $("setup-msg").hidden = false;
  }
}

/* ---------- actions and schedules in settings ---------- */

function describeAllow(key) {
  const names = { press_keys: "Pressing keys", type_text: "Typing with Enter", schedule_task: "Adding schedules", open_url: "Opening links", open_app: "Opening apps" };
  if (key.startsWith("run_command:")) return `Running “${key.slice(12)}”`;
  return names[key] || key;
}

function renderAllowed() {
  const box = $("allowed-list");
  box.innerHTML = "";
  if (!state.draft.allow.length) return void (box.textContent = "Nothing yet. Every risky step asks first.");
  state.draft.allow.forEach((key, i) => {
    const chip = document.createElement("span");
    chip.className = "chip";
    chip.textContent = describeAllow(key);
    const x = document.createElement("button");
    x.type = "button";
    x.title = "Ask first again";
    x.setAttribute("aria-label", `Ask again before ${describeAllow(key)}`);
    x.textContent = "×";
    x.addEventListener("click", () => { state.draft.allow.splice(i, 1); renderAllowed(); });
    chip.appendChild(x);
    box.appendChild(chip);
  });
}

function renderProfiles() {
  const box = $("profiles-list");
  box.innerHTML = "";
  const entries = Object.entries(state.draft.profiles);
  if (!entries.length) return void (box.textContent = "None yet. You are asked once per browser when it matters.");
  for (const [browser, profile] of entries) {
    const chip = document.createElement("span");
    chip.className = "chip";
    chip.textContent = `${browser[0].toUpperCase()}${browser.slice(1)}: ${profile}`;
    const x = Object.assign(document.createElement("button"), { type: "button", title: "Ask again next time", textContent: "\u00d7" });
    x.setAttribute("aria-label", `Forget the ${browser} profile`);
    x.addEventListener("click", () => { delete state.draft.profiles[browser]; renderProfiles(); });
    chip.appendChild(x);
    box.appendChild(chip);
  }
}

function renderCommands() {
  const box = $("commands-list");
  box.innerHTML = "";
  state.draft.commands.forEach((c, i) => {
    const row = document.createElement("div");
    row.className = "cmd-row";
    const name = Object.assign(document.createElement("input"), { placeholder: "Name, e.g. Build site", value: c.name });
    const cmd = Object.assign(document.createElement("input"), { placeholder: "Command, e.g. npm run build", value: c.command });
    name.className = "cmd-name";
    cmd.className = "cmd-command";
    name.addEventListener("input", () => (c.name = name.value));
    cmd.addEventListener("input", () => (c.command = cmd.value));
    const del = Object.assign(document.createElement("button"), { type: "button", textContent: "Remove" });
    del.addEventListener("click", () => { state.draft.commands.splice(i, 1); renderCommands(); });
    row.append(name, cmd, del);
    box.appendChild(row);
  });
}

async function loadSchedules() {
  try { state.schedules = await invoke("list_schedules"); } catch { state.schedules = []; }
  renderSchedules();
}

function renderSchedules() {
  const box = $("schedule-list");
  box.innerHTML = "";
  for (const s of state.schedules) {
    const finished = s.when.kind === "once" && s.last_run != null;
    const row = document.createElement("div");
    row.className = "sched-row" + (s.paused ? " paused" : "") + (finished ? " finished" : "");
    row.dataset.id = s.id;
    const what = Object.assign(document.createElement("div"), { className: "what", textContent: s.name || s.prompt, title: s.prompt });
    const status = s.paused ? "Paused" : finished ? `Ran ${s.last}` : s.next ? `Next: ${s.next}` : "";
    const when = Object.assign(document.createElement("div"), { className: "when", textContent: [s.describe, status].filter(Boolean).join(" · ") });
    const tools = Object.assign(document.createElement("div"), { className: "tools" });
    const button = (label, fn) => {
      const b = Object.assign(document.createElement("button"), { type: "button", textContent: label });
      b.addEventListener("click", fn);
      tools.appendChild(b);
    };
    button("Run now", () => invoke("run_schedule_now", { id: s.id }).catch((e) => scheduleMsg(String(e))));
    if (!finished) button(s.paused ? "Resume" : "Pause", () => updateSchedule({ ...s, paused: !s.paused }));
    button("Edit", () => openScheduleEditor(s));
    button("Delete", async () => { state.schedules = await invoke("delete_schedule", { id: s.id }); renderSchedules(); });
    row.append(what, when, tools);
    box.appendChild(row);
  }
}

async function updateSchedule(s) {
  const { describe, next, last, ...plain } = s;
  try {
    state.schedules = await invoke("save_schedule", { schedule: plain });
    renderSchedules();
    return true;
  } catch (err) {
    scheduleMsg(String(err));
    return false;
  }
}

function scheduleMsg(text) {
  const m = $("sched-msg");
  m.textContent = text || "";
  m.hidden = !text;
  if (text) $("schedule-editor").hidden = false;
}

function today() {
  const d = new Date();
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

function openScheduleEditor(s) {
  state.editing = s || { id: 0, name: "", prompt: "", paused: false, when: { kind: "repeat", time: "09:00", days: [] } };
  const w = state.editing.when;
  $("sched-prompt").value = state.editing.prompt;
  $("sched-name").value = state.editing.name;
  $("sched-time").value = w.time;
  $("sched-date").value = w.kind === "once" ? w.date : today();
  const days = w.days || [];
  $("sched-repeat").value = w.kind === "once" ? "once" : !days.length ? "daily" : days.join() === "0,1,2,3,4" ? "weekdays" : days.join() === "5,6" ? "weekends" : "custom";
  $("sched-days").innerHTML = DAYS.map((d, i) => `<label><input type="checkbox" value="${i}"${days.includes(i) ? " checked" : ""}>${d}</label>`).join("");
  showRepeatFields();
  scheduleMsg("");
  $("schedule-editor").hidden = false;
  $("add-schedule").hidden = true;
  $("sched-prompt").focus();
}

function closeScheduleEditor() {
  state.editing = null;
  $("schedule-editor").hidden = true;
  $("add-schedule").hidden = false;
}

function showRepeatFields() {
  const r = $("sched-repeat").value;
  $("sched-days").hidden = r !== "custom";
  $("sched-date-label").hidden = r !== "once";
}

async function saveScheduleEditor() {
  const r = $("sched-repeat").value;
  const time = $("sched-time").value;
  const days = { daily: [], weekdays: [0, 1, 2, 3, 4], weekends: [5, 6] }[r] ?? [...$("sched-days").querySelectorAll("input:checked")].map((i) => Number(i.value));
  if (r === "custom" && !days.length) return scheduleMsg("Pick at least one day.");
  const when = r === "once" ? { kind: "once", date: $("sched-date").value, time } : { kind: "repeat", time, days };
  const next = { ...state.editing, name: $("sched-name").value.trim(), prompt: $("sched-prompt").value.trim(), when };
  if (await updateSchedule(next)) closeScheduleEditor();
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
$("open-setup").addEventListener("click", openSetup);
$("setup-provider").addEventListener("change", (e) => {
  setup.modelPick[setup.provider] = $("setup-model").value;
  setup.provider = e.target.value;
  showAccountField();
  renderSetupChecks();
  loadModels(setup.provider);
});
$("setup-model").addEventListener("change", () => {
  setup.modelPick[setup.provider] = $("setup-model").value;
  showModelDesc();
});
$("setup-check").addEventListener("click", () => {
  // A changed account folder only counts once saved, so save it first.
  const key = setup.provider === "codex" ? "codex_home" : "claude_config_dir";
  const value = $("setup-account").value.trim();
  if (value !== state.settings[key]) saveSettings({ ...state.settings, [key]: value }).then(checkSetup);
  else checkSetup();
});
$("copy-signin").addEventListener("click", async () => {
  await invoke("copy_text", { text: $("copy-signin").dataset.command || "" });
  $("setup-checking").textContent = "Sign-in command copied. Paste it into a terminal.";
});
$("setup-done").addEventListener("click", () => finishSetup(true));
$("setup-later").addEventListener("click", () => finishSetup(false));
el.settings.elements.provider.addEventListener("change", showProviderFields);
el.settings.elements.opacity.addEventListener("input", showOpacity);
$("add-command").addEventListener("click", () => {
  state.draft.commands.push({ name: "", command: "" });
  renderCommands();
  $("commands-list").querySelector(".cmd-row:last-child .cmd-name").focus();
});
$("add-schedule").addEventListener("click", () => openScheduleEditor(null));
$("sched-cancel").addEventListener("click", closeScheduleEditor);
$("sched-save").addEventListener("click", saveScheduleEditor);
$("sched-repeat").addEventListener("change", showRepeatFields);
// Enter in the schedule editor must not save the whole settings form.
$("schedule-editor").addEventListener("keydown", (e) => {
  if (e.key === "Enter" && e.target.tagName !== "TEXTAREA") e.preventDefault();
});
$("commands-list").addEventListener("keydown", (e) => e.key === "Enter" && e.preventDefault());

// Allow / Always allow / Don't on an action.
el.thread.addEventListener("click", (e) => {
  const b = e.target.closest(".act-buttons button");
  if (b) answerAction(Number(b.closest(".msg.act").dataset.id), b.dataset.decision);
});

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

for (const input of el.settings.querySelectorAll("input.shortcut")) {
  input.addEventListener("keydown", (e) => {
    if (e.key === "Tab" || e.key === "Escape") return;
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Backspace" || e.key === "Delete") return void (input.value = "");
    const acc = acceleratorFrom(e);
    if (acc) input.value = acc;
  });
}

el.settings.addEventListener("submit", async (e) => {
  e.preventDefault();
  const f = el.settings.elements;
  const next = { ...state.settings };
  for (const name of TEXT_FIELDS) next[name] = f[name].value.trim();
  next.keep_warm_minutes = Number(f.keep_warm_minutes.value) || 5;
  next.opacity = Number(f.opacity.value);
  for (const name of TOGGLES) next[name] = f[name].checked;
  next.always_allow = [...state.draft.allow];
  next.browser_profiles = { ...state.draft.profiles };
  next.commands = state.draft.commands.map((c) => ({ name: c.name.trim(), command: c.command.trim() })).filter((c) => c.name || c.command);
  if (next.dock_edge !== state.settings.dock_edge) next.dock_along = 0.5;
  const warn = (text) => {
    el.settingsMsg.textContent = text;
    el.settingsMsg.hidden = false;
  };
  if (next.wake_mode === "shortcut" && !next.shortcut) return warn("Set a shortcut, or choose wake keys, so you can open Agent Desk.");
  if (next.shortcut && next.shortcut === next.live_shortcut) return warn("The talk shortcut and the live-mode shortcut must be different.");
  if (next.commands.some((c) => !c.name || !c.command)) return warn("Each saved command needs a name and the command to run.");
  const names = next.commands.map((c) => c.name.toLowerCase());
  if (new Set(names).size !== names.length) return warn("Give each saved command a different name.");
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
  // A step waiting for the user's OK: Enter allows it, Escape refuses it.
  if (state.asking && el.settings.hidden && el.notch.dataset.view !== "input" && (e.key === "Enter" || e.key === "Escape") && !ctrl) {
    e.preventDefault();
    return void answerAction(state.asking, e.key === "Enter" ? "allow" : "deny");
  }
  if (e.key === "Escape") {
    e.preventDefault();
    if (!el.setup.hidden) closeSetup();
    else if (!el.settings.hidden) closeSettings();
    else if (state.busy && !state.quiet) stop();
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
  invoke("resize", { width: w, height: h }).catch(() => {
    // Not delivered: forget it so the next morph sends it again.
    windowSize.w = windowSize.h = 0;
  });
}

function morph() {
  const w = Math.ceil(el.content.offsetWidth);
  const h = Math.ceil(el.content.offsetHeight);
  el.notch.style.width = w + "px";
  el.notch.style.height = h + "px";
  const m = margins();
  const need = { w: w + m.x, h: h + m.y };
  // While an answer streams in, the shape grows line by line. Resizing the
  // native window for every line is what made it stutter, so the window
  // grows in steps ahead of the shape and fits exactly once it is done.
  if (state.busy && need.h > windowSize.h) need.h = Math.ceil(need.h / 96) * 96;
  clearTimeout(shrinkTimer);
  setWindow(Math.max(windowSize.w, need.w), Math.max(windowSize.h, need.h));
  if (!state.busy && (need.w < windowSize.w || need.h < windowSize.h)) shrinkTimer = setTimeout(() => setWindow(need.w, need.h), 460);
}
new ResizeObserver(morph).observe(el.content);

listen("ai", ({ payload }) => onAiEvent(payload));
listen("action", ({ payload }) => onAction(payload));
listen("schedules-changed", () => !el.settings.hidden && loadSchedules());

// The overlay was opened (by a tap, a hold, or the tray).
listen("wake", ({ payload }) => {
  // The AI session is closed after it has been idle for a while, so an old
  // conversation cannot be continued; start clean instead of pretending.
  const idleMs = Date.now() - state.lastActivity;
  if (!state.busy && !state.live && idleMs >= state.settings.keep_warm_minutes * 60000) clearThread();
  state.lastActivity = Date.now();
  state.context = payload;
  if (!el.settings.hidden) closeSettings();
  if (payload.typing) untuck();
  if (payload.typing && (!state.busy || state.quiet)) {
    setPhase("idle", contextLine());
    el.prompt.focus();
    el.prompt.select();
  }
});
listen("refocus", ({ payload }) => {
  if (payload) untuck();
  if (payload && (!state.busy || state.quiet)) {
    setPhase("idle", contextLine());
    el.prompt.focus();
  }
});

// Push-to-talk progress from the Rust side.
listen("phase", ({ payload }) => {
  const { phase, text } = payload;
  if (phase === "listening") {
    showNotice("");
    closeOverlays();
    setPhase("listening", contextLine());
  } else if (phase === "transcribing") {
    setPhase("transcribing", "");
  } else if (phase === "preparing") {
    setPhase("preparing", text);
  } else if (phase === "cancelled") {
    // The keys turned out to be an ordinary shortcut. Go back to how it was.
    if (state.live) rest();
    else if (el.panel.hidden) invoke("hide");
    else setPhase(state.busy ? "thinking" : state.answer ? "done" : "idle", "");
  } else if (phase === "error") {
    state.busy = false;
    setPhase("error", "");
    showNotice(text);
  }
});

// Speech was understood and sent as a question.
listen("asked", ({ payload }) => {
  // A scheduled task is a conversation of its own.
  if (payload.scheduled) {
    closeOverlays();
    clearThread();
    showNotice("");
  }
  beginTurn(payload.text, payload.quiet);
  adoptTurn(payload.turn);
});

// "Work with me" was switched on or off.
listen("live", ({ payload }) => {
  if (payload === state.live) return void (payload && state.phase === "preparing" && rest("Listening to you and your screen"));
  state.live = payload;
  if (payload) {
    closeOverlays();
    clearThread();
    showNotice("");
    setPhase("live", "Listening to you and your screen");
  } else {
    // The live conversation ends with the mode.
    state.startFresh = el.thread.children.length > 0;
    state.quiet?.userNode.remove();
    state.quiet = null;
    state.busy = false;
    if (el.panel.hidden) invoke("hide");
    else rest("");
  }
});
// Show the latest thing heard, so it is clear the app is following along.
listen("live-heard", ({ payload }) => {
  if (state.live && state.phase === "live") el.topLine.textContent = `${payload.who}: ${payload.text}`;
});
listen("live-note", ({ payload }) => showNotice(payload));
listen("notice", ({ payload }) => showNotice(payload));

listen("insert-request", insertAnswer);
listen("open-settings", openSettings);
listen("open-setup", openSetup);
listen("speech-progress", ({ payload }) => {
  const { model, pct, done, error } = payload;
  if (!setup.info) return;
  if (done) {
    const m = setup.info.speech.find((x) => x.id === model);
    if (m && !error) m.downloaded = true;
    setSpeechState(model, error ? "none" : "done");
    if (error) $("setup-checking").textContent = error;
  } else setSpeechState(model, pct);
});
listen("settings-changed", ({ payload }) => {
  state.settings = payload;
  applyLook();
});

(async function init() {
  // The page can load before the app has finished starting, and then its
  // commands fail. Keep asking until the app answers.
  for (let wait = 50; ; wait = Math.min(wait * 2, 1000)) {
    try {
      state.settings = await invoke("get_settings");
      break;
    } catch {
      await new Promise((r) => setTimeout(r, wait));
    }
  }
  // A window size sent during start-up may have been lost the same way.
  windowSize.w = windowSize.h = 0;
  morph();
  applyLook();
  setPhase("idle", "");
  refreshPanel();
  growPrompt();
  checkStatus();
})();
