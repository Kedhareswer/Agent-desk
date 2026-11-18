//! Talks to the AI through the command-line tools the user is already
//! signed in to (`claude` from Claude Code, `codex` from OpenAI Codex).
//! No API key is stored or needed: the CLIs use the user's own subscription.
//!
//! Claude runs as one long-lived process per conversation, so follow-up
//! questions skip the start-up cost. It is started when the overlay opens
//! (while the user is still typing) and closed after a few idle minutes so
//! nothing sits in memory when the assistant is not in use.

use base64::Engine as _;
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Instant,
};

pub const SYSTEM_PROMPT: &str = "\
You are Agent Desk, an assistant that lives in a small overlay on the user's Windows desktop. \
The user summons you with a hotkey while working in another app. Each request may include a \
screenshot of their screen and the title of the window they were using.

How to answer:
- Be fast and brief. Lead with the answer; no preamble, no recap of the question.
- Use the screenshot as context. Do not describe the screen unless asked.
- If the user asks you to draft, reply, rewrite or fix text, output ONLY the finished text, \
with no introduction and no quotation marks, because it may be pasted straight into their app. \
Match the tone and language of the conversation on screen.
- If the request is empty or vague (\"handle this\", \"help\"), work out the most useful thing \
from the screen: usually drafting a reply to the open message, or answering the question shown.
- Never invent facts about the user (dates, prices, commitments). Where a detail is needed and \
unknown, leave a short [placeholder].
- Use Markdown only when it helps (code blocks for code, short lists). Plain text for drafts.";

/// One update sent to the overlay while an answer is produced.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct AiEvent {
    pub turn: u64,
    /// "delta" | "done" | "error"
    pub kind: &'static str,
    pub text: String,
}

type Emit = Arc<dyn Fn(AiEvent) + Send + Sync>;

#[derive(Debug, PartialEq)]
pub enum Parsed {
    Delta(String),
    Done,
    Error(String),
    Thread(String),
    Ignore,
}

/// Interpret one line of `claude --output-format stream-json`.
pub fn parse_claude_line(line: &str) -> Parsed {
    let Ok(v) = serde_json::from_str::<Value>(line) else { return Parsed::Ignore };
    match v["type"].as_str() {
        Some("stream_event") => {
            let ev = &v["event"];
            match ev["type"].as_str() {
                Some("content_block_delta") if ev["delta"]["type"] == "text_delta" => {
                    Parsed::Delta(ev["delta"]["text"].as_str().unwrap_or("").to_string())
                }
                // The text is complete here. The CLI's own "result" line can
                // arrive seconds later, so the answer is marked done now.
                Some("message_delta") => match ev["delta"]["stop_reason"].as_str() {
                    Some("tool_use") | Some("pause_turn") | None => Parsed::Ignore,
                    Some(_) => Parsed::Done,
                },
                _ => Parsed::Ignore,
            }
        }
        Some("result") if v["is_error"].as_bool() == Some(true) => {
            let msg = v["result"].as_str().unwrap_or("The Claude CLI reported an error.");
            Parsed::Error(msg.to_string())
        }
        _ => Parsed::Ignore,
    }
}

/// Interpret one line of `codex exec --json`.
pub fn parse_codex_line(line: &str) -> Parsed {
    let Ok(v) = serde_json::from_str::<Value>(line) else { return Parsed::Ignore };
    match v["type"].as_str() {
        Some("thread.started") => match v["thread_id"].as_str() {
            Some(id) => Parsed::Thread(id.to_string()),
            None => Parsed::Ignore,
        },
        Some("item.completed") if v["item"]["type"] == "agent_message" => {
            Parsed::Delta(format!("{}\n\n", v["item"]["text"].as_str().unwrap_or("")))
        }
        Some("turn.completed") => Parsed::Done,
        Some("turn.failed") => Parsed::Error(v["error"]["message"].as_str().unwrap_or("Codex could not finish the turn.").to_string()),
        Some("error") => Parsed::Error(v["message"].as_str().unwrap_or("Codex reported an error.").to_string()),
        _ => Parsed::Ignore,
    }
}

/// Find a CLI: an explicit path from settings, then PATH, then the usual
/// install locations.
pub fn resolve_cli(name: &str, explicit: &str) -> Result<PathBuf, String> {
    if !explicit.trim().is_empty() {
        let p = PathBuf::from(explicit.trim());
        return if p.exists() { Ok(p) } else { Err(format!("{} was not found at {}", name, p.display())) };
    }
    if let Ok(p) = which::which(name) {
        return Ok(p);
    }
    let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_default();
    let appdata = std::env::var("APPDATA").unwrap_or_default();
    let candidates = [
        format!("{home}/.local/bin/{name}.exe"),
        format!("{home}/.local/bin/{name}"),
        format!("{appdata}/npm/{name}.cmd"),
        format!("{home}/.claude/local/{name}.exe"),
    ];
    candidates.iter().map(PathBuf::from).find(|p| p.is_file()).ok_or_else(|| {
        let (product, install) = if name == "claude" {
            ("Claude Code", "https://claude.com/claude-code")
        } else {
            ("OpenAI Codex CLI", "npm install -g @openai/codex")
        };
        format!("The `{name}` command was not found. Install {product} ({install}), sign in once by running `{name}` in a terminal, then try again. You can also set its full path in Settings.")
    })
}

fn command(program: &PathBuf) -> Command {
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no console flash
    }
    cmd
}

struct ClaudeProc {
    child: Child,
    stdin: ChildStdin,
    alive: Arc<AtomicBool>,
    /// Model and path this process was started with.
    key: String,
}

#[derive(Default)]
struct State {
    claude: Option<ClaudeProc>,
    codex_child: Option<Arc<Mutex<Child>>>,
    codex_thread: Option<String>,
    last_used: Option<Instant>,
}

pub struct Engine {
    data_dir: PathBuf,
    emit: Emit,
    state: Arc<Mutex<State>>,
    turn: Arc<AtomicU64>,
    busy: Arc<AtomicBool>,
}

pub struct Ask {
    pub prompt: String,
    pub jpeg: Option<Vec<u8>>,
    pub provider: String,
    pub claude_model: String,
    pub claude_path: String,
    pub codex_model: String,
    pub codex_path: String,
}

impl Engine {
    pub fn new(data_dir: PathBuf, emit: Emit) -> Self {
        Self {
            data_dir,
            emit,
            state: Arc::new(Mutex::new(State::default())),
            turn: Arc::new(AtomicU64::new(0)),
            busy: Arc::new(AtomicBool::new(false)),
        }
    }

    /// The CLIs run in an empty folder so they do not pick up project
    /// instructions or files from wherever the app was launched.
    fn workdir(&self) -> PathBuf {
        let dir = self.data_dir.join("workspace");
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    fn spawn_claude(&self, model: &str, path: &str) -> Result<ClaudeProc, String> {
        let program = resolve_cli("claude", path)?;
        let workdir = self.workdir();
        let prompt_file = self.data_dir.join("system-prompt.md");
        std::fs::write(&prompt_file, SYSTEM_PROMPT).map_err(|e| e.to_string())?;

        let mut cmd = command(&program);
        cmd.current_dir(&workdir)
            .args(["-p", "--input-format", "stream-json", "--output-format", "stream-json"])
            .args(["--verbose", "--include-partial-messages"])
            // Chat only: no file, shell or web tools, no MCP servers, and
            // nothing written to the user's Claude Code session history.
            .args(["--tools", "", "--strict-mcp-config", "--disable-slash-commands", "--no-session-persistence"])
            .arg("--system-prompt-file")
            .arg(&prompt_file)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if !model.trim().is_empty() {
            cmd.args(["--model", model.trim()]);
        }
        let mut child = cmd.spawn().map_err(|e| format!("Could not start {}: {e}", program.display()))?;
        let stdin = child.stdin.take().ok_or("no stdin")?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let mut stderr = child.stderr.take().ok_or("no stderr")?;

        let errors = Arc::new(Mutex::new(String::new()));
        let errors_w = errors.clone();
        std::thread::spawn(move || {
            let mut text = String::new();
            let _ = stderr.read_to_string(&mut text);
            if let Ok(mut e) = errors_w.lock() {
                *e = text;
            }
        });

        let alive = Arc::new(AtomicBool::new(true));
        let (alive_r, emit, turn, busy) = (alive.clone(), self.emit.clone(), self.turn.clone(), self.busy.clone());
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let turn_id = turn.load(Ordering::SeqCst);
                match parse_claude_line(&line) {
                    Parsed::Delta(text) => emit(AiEvent { turn: turn_id, kind: "delta", text }),
                    Parsed::Done => {
                        busy.store(false, Ordering::SeqCst);
                        emit(AiEvent { turn: turn_id, kind: "done", text: String::new() });
                    }
                    Parsed::Error(text) => {
                        busy.store(false, Ordering::SeqCst);
                        emit(AiEvent { turn: turn_id, kind: "error", text: explain(&text) });
                    }
                    _ => {}
                }
            }
            // The process ended. If the user was not the one who stopped it
            // and an answer was pending, say why.
            let was_alive = alive_r.swap(false, Ordering::SeqCst);
            if was_alive && busy.swap(false, Ordering::SeqCst) {
                std::thread::sleep(std::time::Duration::from_millis(150));
                let detail = errors.lock().map(|e| e.trim().to_string()).unwrap_or_default();
                let text = if detail.is_empty() { "The Claude CLI closed unexpectedly.".to_string() } else { explain(&detail) };
                emit(AiEvent { turn: turn.load(Ordering::SeqCst), kind: "error", text });
            }
        });

        Ok(ClaudeProc { child, stdin, alive, key: format!("{model}|{path}") })
    }

    fn ensure_claude<'a>(&self, state: &'a mut State, model: &str, path: &str) -> Result<&'a mut ClaudeProc, String> {
        let key = format!("{model}|{path}");
        let usable = state.claude.as_ref().is_some_and(|c| c.alive.load(Ordering::SeqCst) && c.key == key);
        if !usable {
            kill_claude(state);
            state.claude = Some(self.spawn_claude(model, path)?);
        }
        Ok(state.claude.as_mut().unwrap())
    }

    /// Start the Claude process ahead of the first question.
    pub fn prewarm(&self, provider: &str, model: &str, path: &str) {
        if provider != "claude" {
            return;
        }
        if let Ok(mut state) = self.state.lock() {
            state.last_used = Some(Instant::now());
            let _ = self.ensure_claude(&mut state, model, path);
        }
    }

    pub fn ask(&self, ask: Ask) -> Result<u64, String> {
        let turn = self.turn.fetch_add(1, Ordering::SeqCst) + 1;
        let mut state = self.state.lock().map_err(|_| "engine lock poisoned")?;
        state.last_used = Some(Instant::now());
        self.busy.store(true, Ordering::SeqCst);
        let result = if ask.provider == "codex" { self.ask_codex(&mut state, &ask, turn) } else { self.ask_claude(&mut state, &ask) };
        if result.is_err() {
            self.busy.store(false, Ordering::SeqCst);
        }
        result.map(|_| turn)
    }

    fn ask_claude(&self, state: &mut State, ask: &Ask) -> Result<(), String> {
        let mut content = vec![json!({ "type": "text", "text": ask.prompt })];
        if let Some(jpeg) = &ask.jpeg {
            content.push(json!({
                "type": "image",
                "source": { "type": "base64", "media_type": "image/jpeg", "data": base64::engine::general_purpose::STANDARD.encode(jpeg) }
            }));
        }
        let line = format!("{}\n", json!({ "type": "user", "message": { "role": "user", "content": content } }));

        // If the kept-alive process died in the meantime, start a new one
        // and send again.
        for attempt in 0..2 {
            let proc = self.ensure_claude(state, &ask.claude_model, &ask.claude_path)?;
            let sent = proc.stdin.write_all(line.as_bytes()).and_then(|_| proc.stdin.flush());
            match sent {
                Ok(()) => return Ok(()),
                Err(e) if attempt == 1 => return Err(format!("Could not reach the Claude CLI: {e}")),
                Err(_) => kill_claude(state),
            }
        }
        Ok(())
    }

    fn ask_codex(&self, state: &mut State, ask: &Ask, turn: u64) -> Result<(), String> {
        let program = resolve_cli("codex", &ask.codex_path)?;
        let workdir = self.workdir();
        let mut cmd = command(&program);
        cmd.current_dir(&workdir).arg("exec");

        // A new conversation carries the instructions; a follow-up resumes
        // the stored thread.
        let prompt = match &state.codex_thread {
            Some(id) => {
                cmd.args(["resume", id]);
                ask.prompt.clone()
            }
            None => format!("{SYSTEM_PROMPT}\n\n---\n\n{}", ask.prompt),
        };
        cmd.args(["--json", "--skip-git-repo-check", "-c", "sandbox_mode=read-only"]);
        if !ask.codex_model.trim().is_empty() {
            cmd.args(["-m", ask.codex_model.trim()]);
        }
        if let Some(jpeg) = &ask.jpeg {
            let shot = self.data_dir.join("screen.jpg");
            std::fs::write(&shot, jpeg).map_err(|e| e.to_string())?;
            cmd.arg("-i").arg(&shot);
        }
        cmd.arg("-").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| format!("Could not start {}: {e}", program.display()))?;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(prompt.as_bytes());
        }
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let mut stderr = child.stderr.take().ok_or("no stderr")?;
        let child = Arc::new(Mutex::new(child));
        state.codex_child = Some(child.clone());

        let (emit, busy, current) = (self.emit.clone(), self.busy.clone(), self.turn.clone());
        let thread_slot = Arc::new(Mutex::new(None::<String>));
        let thread_slot_w = thread_slot.clone();
        let engine_state = self.state.clone();
        std::thread::spawn(move || {
            let mut finished = false;
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                match parse_codex_line(&line) {
                    Parsed::Thread(id) => *thread_slot_w.lock().unwrap() = Some(id),
                    Parsed::Delta(text) => emit(AiEvent { turn, kind: "delta", text }),
                    Parsed::Done => {
                        finished = true;
                        emit(AiEvent { turn, kind: "done", text: String::new() });
                    }
                    Parsed::Error(text) => {
                        finished = true;
                        emit(AiEvent { turn, kind: "error", text: explain(&text) });
                    }
                    Parsed::Ignore => {}
                }
            }
            let mut detail = String::new();
            let _ = stderr.read_to_string(&mut detail);
            let ok = child.lock().ok().and_then(|mut c| c.wait().ok()).is_some_and(|s| s.success());
            // Only report and record if this is still the current question
            // (the user may have pressed Stop or started a new chat).
            if current.load(Ordering::SeqCst) == turn {
                busy.store(false, Ordering::SeqCst);
                if !finished {
                    let text = if ok { "Codex returned no answer.".to_string() } else { explain(detail.trim()) };
                    emit(AiEvent { turn, kind: "error", text });
                }
                if let Some(id) = thread_slot.lock().unwrap().take() {
                    if let Ok(mut s) = engine_state.lock() {
                        s.codex_thread.get_or_insert(id);
                    }
                }
            }
        });
        Ok(())
    }

    /// Stop the answer in progress.
    pub fn stop(&self) {
        self.turn.fetch_add(1, Ordering::SeqCst);
        self.busy.store(false, Ordering::SeqCst);
        if let Ok(mut state) = self.state.lock() {
            kill_claude(&mut state);
            if let Some(child) = state.codex_child.take() {
                if let Ok(mut c) = child.lock() {
                    let _ = c.kill();
                }
            }
        }
    }

    /// Forget the conversation.
    pub fn new_chat(&self) {
        self.stop();
        if let Ok(mut state) = self.state.lock() {
            state.codex_thread = None;
        }
    }

    /// Close an idle Claude process so it does not hold memory.
    pub fn reap_idle(&self, keep_warm_minutes: u32) {
        if self.busy.load(Ordering::SeqCst) {
            return;
        }
        if let Ok(mut state) = self.state.lock() {
            let idle = state.last_used.is_some_and(|t| t.elapsed().as_secs() >= u64::from(keep_warm_minutes) * 60);
            if idle && state.claude.is_some() {
                kill_claude(&mut state);
            }
        }
    }
}

fn kill_claude(state: &mut State) {
    if let Some(mut proc) = state.claude.take() {
        proc.alive.store(false, Ordering::SeqCst);
        let _ = proc.child.kill();
        let _ = proc.child.wait();
    }
}

/// Turn the CLIs' most common failures into something a person can act on.
fn explain(raw: &str) -> String {
    let lower = raw.to_lowercase();
    if lower.contains("/login") || lower.contains("not logged in") || lower.contains("invalid api key") || lower.contains("unauthorized") || lower.contains("401") {
        return format!("You are not signed in. Open a terminal, run the CLI once (`claude` or `codex`) and sign in with your subscription, then ask again.\n\nDetails: {raw}");
    }
    if lower.contains("rate limit") || lower.contains("usage limit") || lower.contains("429") {
        return format!("Your subscription's usage limit was reached. Try again later or switch provider in Settings.\n\nDetails: {raw}");
    }
    let tail: Vec<&str> = raw.lines().rev().take(6).collect();
    tail.into_iter().rev().collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_text_delta() {
        let line = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hi"}}}"#;
        assert_eq!(parse_claude_line(line), Parsed::Delta("Hi".into()));
    }

    #[test]
    fn claude_done_on_end_turn_only() {
        let end = r#"{"type":"stream_event","event":{"type":"message_delta","delta":{"stop_reason":"end_turn"}}}"#;
        let tool = r#"{"type":"stream_event","event":{"type":"message_delta","delta":{"stop_reason":"tool_use"}}}"#;
        assert_eq!(parse_claude_line(end), Parsed::Done);
        assert_eq!(parse_claude_line(tool), Parsed::Ignore);
    }

    #[test]
    fn claude_result_only_matters_when_it_is_an_error() {
        let ok = r#"{"type":"result","subtype":"success","is_error":false,"result":"4821"}"#;
        let bad = r#"{"type":"result","subtype":"success","is_error":true,"result":"Invalid API key · Please run /login"}"#;
        assert_eq!(parse_claude_line(ok), Parsed::Ignore);
        assert_eq!(parse_claude_line(bad), Parsed::Error("Invalid API key · Please run /login".into()));
    }

    #[test]
    fn claude_noise_is_ignored() {
        assert_eq!(parse_claude_line("not json"), Parsed::Ignore);
        assert_eq!(parse_claude_line(r#"{"type":"system","subtype":"init"}"#), Parsed::Ignore);
    }

    #[test]
    fn codex_events() {
        assert_eq!(parse_codex_line(r#"{"type":"thread.started","thread_id":"abc"}"#), Parsed::Thread("abc".into()));
        assert_eq!(
            parse_codex_line(r#"{"type":"item.completed","item":{"id":"item_0","type":"agent_message","text":"Hello"}}"#),
            Parsed::Delta("Hello\n\n".into())
        );
        assert_eq!(parse_codex_line(r#"{"type":"item.completed","item":{"type":"reasoning","text":"hmm"}}"#), Parsed::Ignore);
        assert_eq!(parse_codex_line(r#"{"type":"turn.completed","usage":{}}"#), Parsed::Done);
        assert_eq!(parse_codex_line(r#"{"type":"turn.failed","error":{"message":"boom"}}"#), Parsed::Error("boom".into()));
    }

    #[test]
    fn sign_in_problems_get_a_plain_hint() {
        assert!(explain("Invalid API key · Please run /login").starts_with("You are not signed in"));
    }

    /// End-to-end check against the real Claude CLI. Needs a signed-in
    /// `claude` on PATH, so it only runs on request:
    ///   cargo test -- --ignored live_claude
    #[test]
    #[ignore]
    fn live_claude_answers_and_remembers() {
        use std::sync::mpsc;
        let dir = std::env::temp_dir().join(format!("agent-desk-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let engine = Engine::new(dir.clone(), Arc::new(move |e| tx.lock().unwrap().send(e).unwrap()));
        let ask = |prompt: &str, jpeg: Option<Vec<u8>>| Ask {
            prompt: prompt.into(),
            jpeg,
            provider: "claude".into(),
            claude_model: "haiku".into(),
            claude_path: String::new(),
            codex_model: String::new(),
            codex_path: String::new(),
        };
        let collect = |turn: u64| {
            let mut text = String::new();
            loop {
                let e: AiEvent = rx.recv_timeout(std::time::Duration::from_secs(90)).expect("timed out");
                assert_eq!(e.turn, turn);
                match e.kind {
                    "delta" => text.push_str(&e.text),
                    "done" => return text,
                    _ => panic!("error from CLI: {}", e.text),
                }
            }
        };

        // A plain grey image with a darker square: enough to prove the
        // screenshot reaches the model.
        let mut rgba = vec![200u8; 64 * 64 * 4];
        for px in rgba.chunks_exact_mut(4).take(64 * 20) {
            px[..3].copy_from_slice(&[200, 30, 30]);
        }
        let jpeg = crate::platform::to_jpeg(crate::platform::RawImage { width: 64, height: 64, rgba }, 64);

        let started = Instant::now();
        let t1 = engine.ask(ask("The secret word is 'marmalade'. What colour is the top band of the image? One word.", jpeg)).unwrap();
        let first = collect(t1);
        eprintln!("first answer after {:?}: {first}", started.elapsed());
        assert!(first.to_lowercase().contains("red"), "{first}");

        let started = Instant::now();
        let t2 = engine.ask(ask("What was the secret word? One word.", None)).unwrap();
        let second = collect(t2);
        eprintln!("follow-up after {:?}: {second}", started.elapsed());
        assert!(second.to_lowercase().contains("marmalade"), "{second}");

        engine.new_chat();
        let _ = std::fs::remove_dir_all(dir);
    }
}
