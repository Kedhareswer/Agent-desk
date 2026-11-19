//! The setup screen's facts: is the CLI installed, which account is it
//! signed in to, which models can it use, which speech models are on disk.
//!
//! Everything comes from the CLIs themselves (`claude auth status --json`,
//! `codex login status`, `codex debug models`), so the screen shows what
//! Agent Desk will really use. Commands run hidden, in parallel, with a
//! time limit, and only while the setup screen is open.

use serde::Serialize;
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

#[derive(Serialize, Default, Clone, Debug, PartialEq)]
pub struct CliInfo {
    pub found: bool,
    pub path: String,
    pub version: String,
    pub signed_in: bool,
    /// Who it is signed in as: an email or a plain description.
    pub account: String,
    /// Subscription, when the CLI says (e.g. "max").
    pub plan: String,
    /// The folder holding this sign-in (Claude's config folder).
    pub config_dir: String,
    /// What to run in a terminal to sign in, ready to copy.
    pub sign_in_command: String,
    /// A plain hint when something is missing.
    pub problem: String,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ModelChoice {
    pub id: String,
    pub name: String,
    pub description: String,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct SpeechModel {
    pub id: String,
    pub label: String,
    pub size_mb: u64,
    pub downloaded: bool,
    /// Percent already on disk from an unfinished download.
    pub partial: u8,
}

/// Run a CLI hidden, with extra environment, for at most `secs`.
fn run(program: &Path, args: &[&str], env: &[(&str, String)], secs: u64) -> Option<String> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in env {
        if !v.is_empty() {
            cmd.env(k, v);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd.spawn().ok()?;
    let out = child.stdout.take()?;
    let err = child.stderr.take()?;
    let read = |mut r: Box<dyn std::io::Read + Send>| {
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = r.read_to_string(&mut s);
            s
        })
    };
    let (o, e) = (read(Box::new(out)), read(Box::new(err)));
    let start = std::time::Instant::now();
    loop {
        if let Ok(Some(_)) = child.try_wait() {
            break;
        }
        if start.elapsed() > Duration::from_secs(secs) {
            let _ = child.kill();
            return None;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Some(format!(
        "{}{}",
        o.join().unwrap_or_default(),
        e.join().unwrap_or_default()
    ))
}

/// Read `claude auth status --json`.
pub fn parse_claude_auth(json: &str, info: &mut CliInfo) {
    let start = json.find('{').unwrap_or(0);
    let Ok(v) = serde_json::from_str::<Value>(&json[start..]) else {
        return;
    };
    info.signed_in = v["loggedIn"].as_bool().unwrap_or(false);
    info.account = v["email"].as_str().unwrap_or("").to_string();
    if info.account.is_empty() {
        info.account = v["authMethod"].as_str().unwrap_or("").to_string();
    }
    info.plan = v["subscriptionType"].as_str().unwrap_or("").to_string();
    info.config_dir = v["configDirectory"].as_str().unwrap_or("").to_string();
}

/// The first line that looks like a version number.
pub fn parse_version(out: &str) -> String {
    out.lines()
        .map(str::trim)
        .find(|l| {
            l.chars().next().is_some_and(|c| c.is_ascii_digit()) || l.contains(char::is_numeric)
        })
        .unwrap_or("")
        .chars()
        .take(60)
        .collect()
}

/// A PowerShell line that signs in to this account folder.
pub fn sign_in_command(provider: &str, account_dir: &str) -> String {
    let (var, cmd) = if provider == "codex" {
        ("CODEX_HOME", "codex login")
    } else {
        ("CLAUDE_CONFIG_DIR", "claude auth login")
    };
    if account_dir.trim().is_empty() {
        cmd.to_string()
    } else {
        format!("$env:{var} = \"{}\"; {cmd}", account_dir.trim())
    }
}

/// Facts about one CLI. `account_dir` is the chosen sign-in folder (empty
/// for the default one).
pub fn cli_info(provider: &str, explicit_path: &str, account_dir: &str) -> CliInfo {
    let name = if provider == "codex" {
        "codex"
    } else {
        "claude"
    };
    let mut info = CliInfo {
        sign_in_command: sign_in_command(provider, account_dir),
        ..Default::default()
    };
    let path = match crate::backend::resolve_cli(name, explicit_path) {
        Ok(p) => p,
        Err(e) => {
            info.problem = e;
            return info;
        }
    };
    info.found = true;
    info.path = path.display().to_string();
    let env_var = if provider == "codex" {
        "CODEX_HOME"
    } else {
        "CLAUDE_CONFIG_DIR"
    };
    let env = [(env_var, account_dir.trim().to_string())];
    // Version and sign-in in parallel: each CLI takes a moment to start.
    let (p2, env2) = (path.clone(), env.clone());
    let version =
        std::thread::spawn(move || run(&p2, &["--version"], &env2, 15).map(|o| parse_version(&o)));
    if provider == "codex" {
        if let Some(out) = run(&path, &["login", "status"], &env, 15) {
            let line = out
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .unwrap_or("")
                .to_string();
            info.signed_in = line.to_lowercase().starts_with("logged in");
            info.account = line;
        }
        info.config_dir = if account_dir.trim().is_empty() {
            home().join(".codex").display().to_string()
        } else {
            account_dir.trim().to_string()
        };
    } else if let Some(out) = run(&path, &["auth", "status", "--json"], &env, 15) {
        parse_claude_auth(&out, &mut info);
    }
    info.version = version.join().ok().flatten().unwrap_or_default();
    if !info.signed_in {
        info.problem = format!(
            "Not signed in. Run `{}` in a terminal, then press Check again.",
            info.sign_in_command
        );
    }
    info
}

fn home() -> PathBuf {
    PathBuf::from(
        std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .unwrap_or_default(),
    )
}

/// Claude models by the names the CLI accepts. The CLI resolves each name
/// to its newest model; `resolved` is the full name it reported for the
/// one in use, shown next to it.
pub fn claude_models(current: &str, resolved: &str) -> Vec<ModelChoice> {
    let mut list = vec![
        ModelChoice {
            id: "haiku".into(),
            name: "Haiku".into(),
            description: "Fastest. Best for voice.".into(),
        },
        ModelChoice {
            id: "sonnet".into(),
            name: "Sonnet".into(),
            description: "Balanced speed and skill.".into(),
        },
        ModelChoice {
            id: "opus".into(),
            name: "Opus".into(),
            description: "Most capable, slower.".into(),
        },
    ];
    for m in &mut list {
        if m.id == current && !resolved.is_empty() {
            m.description = format!("{} Now: {resolved}.", m.description);
        }
    }
    // A full model name typed in Settings stays selectable.
    if !current.is_empty() && !list.iter().any(|m| m.id == current) {
        list.push(ModelChoice {
            id: current.into(),
            name: current.into(),
            description: "Chosen by its full name.".into(),
        });
    }
    list
}

/// Models from `codex debug models`, as the Codex CLI lists them.
pub fn parse_codex_models(json: &str) -> Vec<ModelChoice> {
    let start = json.find('{').unwrap_or(0);
    let Ok(v) = serde_json::from_str::<Value>(&json[start..]) else {
        return Vec::new();
    };
    let mut list: Vec<(i64, ModelChoice)> = v["models"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|m| m["visibility"].as_str().unwrap_or("list") == "list")
                .filter_map(|m| {
                    let id = m["slug"].as_str()?.to_string();
                    Some((
                        m["priority"].as_i64().unwrap_or(999),
                        ModelChoice {
                            name: m["display_name"].as_str().unwrap_or(&id).to_string(),
                            description: m["description"].as_str().unwrap_or("").to_string(),
                            id,
                        },
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    list.sort_by_key(|(p, _)| *p);
    list.into_iter().map(|(_, m)| m).collect()
}

pub fn codex_models(explicit_path: &str, account_dir: &str) -> Vec<ModelChoice> {
    let Ok(path) = crate::backend::resolve_cli("codex", explicit_path) else {
        return Vec::new();
    };
    let env = [("CODEX_HOME", account_dir.trim().to_string())];
    run(&path, &["debug", "models"], &env, 20)
        .map(|o| parse_codex_models(&o))
        .unwrap_or_default()
}

/// The speech models and how much of each is on disk.
pub fn speech_models(dir: &Path) -> Vec<SpeechModel> {
    let labels = [
        ("tiny.en", "Tiny, English (fastest)"),
        ("base.en", "Base, English"),
        ("small.en", "Small, English (most accurate)"),
        ("base", "Base, any language"),
        ("small", "Small, any language"),
    ];
    crate::stt::MODELS
        .iter()
        .map(|(id, _, size)| {
            let path = crate::stt::model_path(dir, id);
            let partial = std::fs::metadata(path.with_extension("part"))
                .map(|m| (m.len() * 100 / size.max(&1)).min(99) as u8)
                .unwrap_or(0);
            SpeechModel {
                id: id.to_string(),
                label: labels
                    .iter()
                    .find(|l| l.0 == *id)
                    .map(|l| l.1)
                    .unwrap_or(id)
                    .to_string(),
                size_mb: size / 1_000_000,
                downloaded: crate::stt::model_ready(dir, id),
                partial,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_sign_in_status() {
        let out = r#"{
  "loggedIn": true, "authMethod": "claude.ai", "configDirectory": "C:\\Users\\sam\\.claude",
  "email": "sam@example.com", "subscriptionType": "max" }"#;
        let mut info = CliInfo::default();
        parse_claude_auth(out, &mut info);
        assert!(info.signed_in);
        assert_eq!(info.account, "sam@example.com");
        assert_eq!(info.plan, "max");
        assert_eq!(info.config_dir, r"C:\Users\sam\.claude");
        let mut none = CliInfo::default();
        parse_claude_auth(r#"{"loggedIn": false}"#, &mut none);
        assert!(!none.signed_in);
        parse_claude_auth("not json", &mut none);
        assert!(!none.signed_in);
    }

    #[test]
    fn sign_in_commands_use_the_chosen_account_folder() {
        assert_eq!(sign_in_command("claude", ""), "claude auth login");
        assert_eq!(sign_in_command("codex", " "), "codex login");
        assert_eq!(
            sign_in_command("claude", r"D:\claude-work"),
            r#"$env:CLAUDE_CONFIG_DIR = "D:\claude-work"; claude auth login"#
        );
        assert_eq!(
            sign_in_command("codex", r"D:\codex-work"),
            r#"$env:CODEX_HOME = "D:\codex-work"; codex login"#
        );
    }

    #[test]
    fn versions() {
        assert_eq!(
            parse_version("2.1.327 (Claude Code)\n"),
            "2.1.327 (Claude Code)"
        );
        assert_eq!(parse_version("codex-cli 0.170.0"), "codex-cli 0.170.0");
        assert_eq!(parse_version(""), "");
    }

    #[test]
    fn codex_catalog() {
        let json = r#"{"models":[
            {"slug":"gpt-b","display_name":"GPT B","visibility":"list","priority":2,"description":"Capable."},
            {"slug":"hidden","display_name":"Hidden","visibility":"hide","priority":0},
            {"slug":"gpt-a","display_name":"GPT A","visibility":"list","priority":1,"description":"Fast."}]}"#;
        let list = parse_codex_models(json);
        assert_eq!(
            list.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            ["gpt-a", "gpt-b"]
        );
        assert_eq!(list[0].name, "GPT A");
        assert!(parse_codex_models("garbage").is_empty());
    }

    #[test]
    fn claude_choices() {
        let list = claude_models("haiku", "claude-haiku-5-5");
        assert_eq!(list.len(), 3);
        assert!(list[0].description.ends_with("Now: claude-haiku-5-5."));
        let custom = claude_models("claude-sonnet-5-5", "");
        assert_eq!(custom.last().unwrap().id, "claude-sonnet-5-5");
    }

    #[test]
    fn speech_model_states() {
        let dir = std::env::temp_dir().join(format!("agent-desk-setup-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("models")).unwrap();
        // A quarter of tiny.en on disk, unfinished.
        let size = crate::stt::MODELS[0].2;
        std::fs::write(
            crate::stt::model_path(&dir, "tiny.en").with_extension("part"),
            vec![0u8; (size / 4) as usize],
        )
        .unwrap();
        let list = speech_models(&dir);
        assert_eq!(list.len(), crate::stt::MODELS.len());
        assert_eq!(list[0].id, "tiny.en");
        assert!(!list[0].downloaded);
        assert!((24..=25).contains(&list[0].partial), "{}", list[0].partial);
        assert_eq!(list[1].partial, 0);
        let _ = std::fs::remove_dir_all(dir);
    }
}
