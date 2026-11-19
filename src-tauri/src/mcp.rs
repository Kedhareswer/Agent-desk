//! A tiny MCP (Model Context Protocol) server inside the tray process, so
//! the `claude` and `codex` CLIs can call Agent Desk's actions as tools.
//!
//! It speaks the "streamable HTTP" transport in its simplest form: each
//! JSON-RPC request is one HTTP POST and the answer is one JSON body. It
//! listens on 127.0.0.1 only, on a random port, and every request must carry
//! a random bearer token that only the CLI processes we start are given.
//! Requests that come from a web page (they carry an `Origin` header) are
//! refused, so a website cannot reach it through the browser.
//!
//! No HTTP or MCP library is used: the protocol subset is small and this
//! keeps the app light.

use serde_json::{json, Value};
use std::{
    collections::hash_map::RandomState,
    hash::{BuildHasher, Hasher},
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    sync::Arc,
    time::Duration,
};

/// What a tool call returns: MCP content blocks, and whether it failed.
pub struct ToolResult {
    pub content: Vec<Value>,
    pub is_error: bool,
}

impl ToolResult {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content: vec![json!({ "type": "text", "text": text.into() })],
            is_error: false,
        }
    }
    pub fn error(text: impl Into<String>) -> Self {
        Self {
            is_error: true,
            ..Self::text(text)
        }
    }
}

/// Runs a tool: (name, arguments) → result. It may block, for example while
/// the user decides whether to allow the action.
pub type Handler = Arc<dyn Fn(&str, &Value) -> ToolResult + Send + Sync>;

pub struct Server {
    pub port: u16,
    pub token: String,
}

impl Server {
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/mcp", self.port)
    }
}

/// 128 random bits as hex. The standard library seeds each `RandomState`
/// from the operating system's random source.
/// 128 random bits as hex, from the operating system's random source
/// (BCryptGenRandom on Windows).
fn random_token() -> String {
    let mut bytes = [0u8; 16];
    fill_random(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(windows)]
fn fill_random(buf: &mut [u8]) {
    use windows::Win32::Security::Cryptography::{
        BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG,
    };
    let ok = unsafe { BCryptGenRandom(None, buf, BCRYPT_USE_SYSTEM_PREFERRED_RNG) }.is_ok();
    if !ok {
        fill_fallback(buf);
    }
}

#[cfg(not(windows))]
fn fill_random(buf: &mut [u8]) {
    use std::io::Read;
    let read = std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(buf));
    if read.is_err() {
        fill_fallback(buf);
    }
}

/// Only if the system source fails: the standard library seeds every
/// RandomState from the operating system too.
fn fill_fallback(buf: &mut [u8]) {
    for (i, chunk) in buf.chunks_mut(8).enumerate() {
        let mut h = RandomState::new().build_hasher();
        h.write_usize(i);
        chunk.copy_from_slice(&h.finish().to_le_bytes()[..chunk.len()]);
    }
}

/// Start listening. `tools` is the `tools/list` answer.
/// At most this many connections are served at once; more are refused.
const MAX_CONNECTIONS: usize = 16;

pub fn start(tools: Value, handler: Handler) -> std::io::Result<Server> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    let token = random_token();
    let (tools, secret) = (Arc::new(tools), token.clone());
    let open = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    std::thread::spawn(move || {
        use std::sync::atomic::Ordering;
        for stream in listener.incoming().flatten() {
            if open.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
                open.fetch_sub(1, Ordering::SeqCst);
                respond(&stream, "503 Service Unavailable", None);
                continue;
            }
            let (tools, handler, secret, open) =
                (tools.clone(), handler.clone(), secret.clone(), open.clone());
            // A tool call can wait for the user, so each connection gets
            // its own thread.
            std::thread::spawn(move || {
                serve(stream, &secret, &tools, &handler);
                open.fetch_sub(1, Ordering::SeqCst);
            });
        }
    });
    Ok(Server { port, token })
}

#[derive(Debug, Default, PartialEq)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Read one HTTP/1.1 request.
/// Header bytes accepted per request (request line included).
const MAX_HEAD: usize = 16 * 1024;
/// Tool calls are small. Anything huge is not from our CLIs.
const MAX_BODY: usize = 4 << 20;

/// Read the request line and headers only. The body is read later, and
/// only from a caller that showed the token.
pub fn read_head(reader: &mut impl BufRead) -> Option<Request> {
    let mut used = 0usize;
    let mut next_line = |reader: &mut dyn BufRead| -> Option<String> {
        let mut line = Vec::new();
        let n = reader
            .take((MAX_HEAD - used) as u64 + 1)
            .read_until(b'\n', &mut line)
            .ok()?;
        used += n;
        if n == 0 || used > MAX_HEAD {
            return None;
        }
        String::from_utf8(line).ok()
    };
    let first = next_line(reader)?;
    let mut parts = first.split_whitespace();
    let mut req = Request {
        method: parts.next()?.to_string(),
        path: parts.next()?.to_string(),
        ..Default::default()
    };
    loop {
        let line = next_line(reader)?;
        let l = line.trim_end();
        if l.is_empty() {
            break;
        }
        if let Some((k, v)) = l.split_once(':') {
            req.headers
                .push((k.trim().to_string(), v.trim().to_string()));
        }
        if req.headers.len() > 100 {
            return None;
        }
    }
    Some(req)
}

/// Read the body announced by Content-Length.
pub fn read_body(reader: &mut impl Read, req: &mut Request) -> Option<()> {
    let len: usize = req
        .header("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    if len > MAX_BODY {
        return None;
    }
    let mut body = vec![0; len];
    reader.read_exact(&mut body).ok()?;
    req.body = String::from_utf8(body).ok()?;
    Some(())
}

/// Read a whole request (head and body). Used by the tests.
#[cfg(test)]
pub fn read_request(stream: impl Read) -> Option<Request> {
    let mut reader = BufReader::new(stream);
    let mut req = read_head(&mut reader)?;
    read_body(&mut reader, &mut req)?;
    Some(req)
}

/// Whether a request may use the server at all.
pub fn allowed(req: &Request, token: &str) -> bool {
    // Browsers always send Origin on a cross-site POST; CLIs never do.
    if req.header("origin").is_some() {
        return false;
    }
    let presented = req
        .header("authorization")
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    // Compare without stopping at the first difference.
    !token.is_empty()
        && presented.len() == token.len()
        && presented
            .bytes()
            .zip(token.bytes())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
}

/// Protocol versions this server understands, newest first.
const VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

/// Answer one JSON-RPC message. None means "no reply" (a notification).
pub fn handle_rpc(
    body: &str,
    tools: &Value,
    call: &dyn Fn(&str, &Value) -> ToolResult,
) -> Option<Value> {
    let msg: Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => {
            return Some(
                json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": "Parse error" } }),
            )
        }
    };
    let id = msg.get("id").cloned()?; // notifications have no id
    let method = msg["method"].as_str().unwrap_or("");
    let result = match method {
        "initialize" => {
            let asked = msg["params"]["protocolVersion"].as_str().unwrap_or("");
            let version = VERSIONS
                .iter()
                .find(|v| **v == asked)
                .unwrap_or(&VERSIONS[0]);
            json!({
                "protocolVersion": version,
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": { "name": "agent-desk", "version": env!("CARGO_PKG_VERSION") },
                "instructions": "Actions on the user's Windows desktop. Each one is shown to the user, and risky ones need their OK."
            })
        }
        "ping" => json!({}),
        "tools/list" => json!({ "tools": tools }),
        "tools/call" => {
            let name = msg["params"]["name"].as_str().unwrap_or("");
            let empty = json!({});
            let args = msg["params"].get("arguments").unwrap_or(&empty);
            let r = call(name, args);
            json!({ "content": r.content, "isError": r.is_error })
        }
        _ => {
            return Some(
                json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": format!("Unknown method {method}") } }),
            )
        }
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

fn respond(mut stream: &TcpStream, status: &str, body: Option<&str>) {
    let body = body.unwrap_or("");
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body.as_bytes());
    let _ = stream.flush();
}

fn serve(stream: TcpStream, token: &str, tools: &Value, handler: &Handler) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    let mut reader = BufReader::new(&stream);
    let Some(mut req) = read_head(&mut reader) else {
        return respond(&stream, "400 Bad Request", None);
    };
    // The token is checked before a single byte of the body is read.
    if !allowed(&req, token) {
        return respond(&stream, "403 Forbidden", None);
    }
    if read_body(&mut reader, &mut req).is_none() {
        return respond(&stream, "400 Bad Request", None);
    }
    if req.path.split('?').next() != Some("/mcp") {
        return respond(&stream, "404 Not Found", None);
    }
    match req.method.as_str() {
        "POST" => match handle_rpc(&req.body, tools, &|n, a| handler(n, a)) {
            Some(reply) => respond(&stream, "200 OK", Some(&reply.to_string())),
            None => respond(&stream, "202 Accepted", None),
        },
        // No server-initiated messages, so no event stream to open.
        "GET" => respond(&stream, "405 Method Not Allowed", None),
        "DELETE" => respond(&stream, "200 OK", None),
        _ => respond(&stream, "405 Method Not Allowed", None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn echo(name: &str, args: &Value) -> ToolResult {
        if name == "fail" {
            ToolResult::error("nope")
        } else {
            ToolResult::text(format!("{name} {args}"))
        }
    }

    #[test]
    fn reads_a_post() {
        let raw = "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer abc\r\ncontent-length: 7\r\n\r\n{\"a\":1}";
        let req = read_request(raw.as_bytes()).unwrap();
        assert_eq!(req.method, "POST");
        assert_eq!(req.path, "/mcp");
        assert_eq!(req.header("Content-Length"), Some("7"));
        assert_eq!(req.body, "{\"a\":1}");
        assert!(allowed(&req, "abc"));
        assert!(!allowed(&req, "abd"));
        assert!(!allowed(&req, "abcd"));
        assert!(read_request("garbage".as_bytes()).is_none());
    }

    #[test]
    fn oversized_heads_are_refused() {
        let big = format!("POST /mcp HTTP/1.1\r\nX: {}\r\n\r\n", "a".repeat(20_000));
        assert!(read_request(big.as_bytes()).is_none());
        let huge_body =
            "POST /mcp HTTP/1.1\r\nAuthorization: Bearer abc\r\nContent-Length: 99999999\r\n\r\n";
        assert!(read_request(huge_body.as_bytes()).is_none());
    }

    #[test]
    fn tokens_are_random_and_long() {
        let (a, b) = (random_token(), random_token());
        assert_eq!(a.len(), 32);
        assert_ne!(a, b);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn strangers_never_get_their_body_read() {
        // A huge declared body from someone without the token is answered
        // 403 at once, without waiting for (or buffering) the body.
        let server = start(json!([]), Arc::new(echo)).unwrap();
        let mut s = TcpStream::connect(("127.0.0.1", server.port)).unwrap();
        write!(s, "POST /mcp HTTP/1.1\r\nContent-Length: 4000000\r\n\r\n").unwrap();
        let mut out = String::new();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        s.read_to_string(&mut out).unwrap();
        assert!(out.starts_with("HTTP/1.1 403"), "{out}");
    }

    #[test]
    fn web_pages_and_strangers_are_refused() {
        let from_page = read_request(
            "POST /mcp HTTP/1.1\r\nOrigin: https://evil.example\r\nAuthorization: Bearer abc\r\n\r\n".as_bytes(),
        )
        .unwrap();
        assert!(!allowed(&from_page, "abc"));
        let no_token = read_request("POST /mcp HTTP/1.1\r\n\r\n".as_bytes()).unwrap();
        assert!(!allowed(&no_token, "abc"));
        assert!(
            !allowed(&no_token, ""),
            "an empty token never matches a missing one by accident"
        );
    }

    #[test]
    fn mcp_handshake_and_calls() {
        let tools = json!([{ "name": "open_url" }]);
        let init = handle_rpc(
            r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"claude-code","version":"2"}}}"#,
            &tools,
            &echo,
        )
        .unwrap();
        assert_eq!(init["id"], 0);
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        assert!(init["result"]["capabilities"]["tools"].is_object());

        // An unknown version gets the newest one we speak.
        let init = handle_rpc(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2099-01-01"}}"#, &tools, &echo).unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2025-06-18");

        assert!(handle_rpc(
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            &tools,
            &echo
        )
        .is_none());

        let list = handle_rpc(
            r#"{"jsonrpc":"2.0","id":"a","method":"tools/list"}"#,
            &tools,
            &echo,
        )
        .unwrap();
        assert_eq!(list["result"]["tools"][0]["name"], "open_url");

        let call = handle_rpc(
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"open_url","arguments":{"url":"https://github.com"}}}"#,
            &tools,
            &echo,
        )
        .unwrap();
        assert_eq!(call["result"]["isError"], false);
        assert_eq!(
            call["result"]["content"][0]["text"],
            r#"open_url {"url":"https://github.com"}"#
        );

        let failed = handle_rpc(
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"fail"}}"#,
            &tools,
            &echo,
        )
        .unwrap();
        assert_eq!(failed["result"]["isError"], true);

        let unknown = handle_rpc(
            r#"{"jsonrpc":"2.0","id":4,"method":"resources/list"}"#,
            &tools,
            &echo,
        )
        .unwrap();
        assert_eq!(unknown["error"]["code"], -32601);
        assert_eq!(
            handle_rpc("{", &tools, &echo).unwrap()["error"]["code"],
            -32700
        );
    }

    #[test]
    fn serves_over_http() {
        let server = start(json!([]), Arc::new(echo)).unwrap();
        assert_eq!(server.token.len(), 32);
        assert_ne!(server.token, random_token());
        let post = |auth: &str, body: &str| {
            let mut s = TcpStream::connect(("127.0.0.1", server.port)).unwrap();
            write!(s, "POST /mcp HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {auth}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len()).unwrap();
            let mut out = String::new();
            s.read_to_string(&mut out).unwrap();
            out
        };
        let ok = post(&server.token, r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#);
        assert!(ok.starts_with("HTTP/1.1 200"), "{ok}");
        let body: Value = serde_json::from_str(ok.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(body, json!({ "jsonrpc": "2.0", "id": 1, "result": {} }));
        assert!(post("wrong", "{}").starts_with("HTTP/1.1 403"));
        let note = post(
            &server.token,
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        );
        assert!(note.starts_with("HTTP/1.1 202"), "{note}");
    }
}
