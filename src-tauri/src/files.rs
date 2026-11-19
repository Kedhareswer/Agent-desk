//! Folders and documents the AI may look at: list a folder, read a text
//! file, a Word document (.docx) or a PDF (as page pictures, rendered by
//! Windows). Reading sends the content to the AI through the user's own
//! CLI, so places that hold secrets are refused outright.

use std::path::{Path, PathBuf};

/// Text read from a file is cut to this many characters.
pub const MAX_TEXT: usize = 20_000;
/// At most this many PDF pages are sent, as pictures.
pub const MAX_PDF_PAGES: u32 = 5;

/// Expand `~`, `%USERPROFILE%`-style variables and the usual folder names
/// ("Documents", "Downloads", "Desktop") into a full path.
pub fn resolve(raw: &str, home: &Path) -> PathBuf {
    let raw = raw.trim().trim_matches('"');
    let mut s = raw.to_string();
    // %VAR% from the environment.
    while let Some(start) = s.find('%') {
        let Some(len) = s[start + 1..].find('%') else {
            break;
        };
        let name = &s[start + 1..start + 1 + len];
        let value = std::env::var(name).unwrap_or_default();
        if value.is_empty() {
            break;
        }
        s.replace_range(start..start + len + 2, &value);
    }
    if let Some(rest) = s.strip_prefix("~") {
        return home.join(rest.trim_start_matches(['/', '\\']));
    }
    let p = PathBuf::from(&s);
    if p.is_absolute() {
        return p;
    }
    // "Documents/report.docx" → in the user's home folder.
    home.join(p)
}

/// Places that hold keys, passwords, browser data and app internals. The
/// AI never reads from them, whatever it is asked.
pub fn is_off_limits(path: &Path) -> bool {
    let lower = path.to_string_lossy().replace('/', "\\").to_lowercase();
    let parts: Vec<&str> = lower.split('\\').collect();
    let dirs = [
        ".ssh",
        ".aws",
        ".azure",
        ".gnupg",
        ".kube",
        ".docker",
        "appdata",
        ".config",
        ".password-store",
        "credentials",
        ".codex",
        ".claude",
    ];
    if parts.iter().any(|p| dirs.contains(p)) {
        return true;
    }
    let name = parts.last().copied().unwrap_or("");
    let secret_names = [
        "id_rsa",
        "id_ed25519",
        "id_ecdsa",
        ".env",
        ".netrc",
        ".npmrc",
        ".pypirc",
        ".git-credentials",
        "credentials.json",
        "secrets.json",
    ];
    let secret_ext = [
        ".pem",
        ".key",
        ".pfx",
        ".p12",
        ".kdbx",
        ".ppk",
        ".jks",
        ".keystore",
        ".ovpn",
    ];
    secret_names.contains(&name)
        || name.starts_with(".env.")
        || secret_ext.iter().any(|e| name.ends_with(e))
        || lower.starts_with("c:\\windows")
}

/// Opening one of these runs a program, so it waits for the user's OK.
pub fn runs_something(path: &Path) -> bool {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    [
        "exe",
        "bat",
        "cmd",
        "com",
        "ps1",
        "vbs",
        "vbe",
        "js",
        "jse",
        "wsf",
        "wsh",
        "msi",
        "msc",
        "scr",
        "hta",
        "jar",
        "py",
        "pyw",
        "reg",
        "lnk",
        "url",
        "appref-ms",
        "cpl",
        "inf",
        "sh",
    ]
    .contains(&ext.as_str())
}

/// What kind of file this is, for reading.
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Kind {
    Text,
    Word,
    Pdf,
    Other,
}

pub fn kind_of(path: &Path) -> Kind {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "txt" | "md" | "markdown" | "csv" | "tsv" | "json" | "log" | "xml" | "yaml" | "yml"
        | "toml" | "ini" | "html" | "htm" | "rtf" | "srt" | "vtt" | "eml" => Kind::Text,
        "docx" => Kind::Word,
        "pdf" => Kind::Pdf,
        _ => Kind::Other,
    }
}

/// The paragraphs of a .docx (a zip of XML), as plain text.
pub fn docx_text(bytes: &[u8]) -> Result<String, String> {
    use std::io::Read;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|_| "That is not a readable Word document.".to_string())?;
    let mut xml = String::new();
    zip.by_name("word/document.xml")
        .map_err(|_| "That Word document has no text part.".to_string())?
        .read_to_string(&mut xml)
        .map_err(|e| e.to_string())?;
    Ok(word_xml_text(&xml))
}

/// Pull text out of WordprocessingML: paragraphs become lines, tabs and
/// breaks are kept, everything else is dropped.
pub fn word_xml_text(xml: &str) -> String {
    let mut out = String::new();
    let mut rest = xml;
    while let Some(lt) = rest.find('<') {
        let text = &rest[..lt];
        if !text.is_empty() {
            out.push_str(&unescape(text));
        }
        let Some(gt) = rest[lt..].find('>') else {
            break;
        };
        let tag = &rest[lt + 1..lt + gt];
        let name = tag
            .trim_start_matches('/')
            .split([' ', '/'])
            .next()
            .unwrap_or("");
        match name {
            "w:p" if tag.starts_with('/') => out.push('\n'),
            "w:tab" => out.push('\t'),
            "w:br" | "w:cr" => out.push('\n'),
            _ => {}
        }
        // Text between tags only counts inside <w:t>; skip instruction text.
        rest = &rest[lt + gt + 1..];
        if name == "w:instrText" && !tag.starts_with('/') {
            if let Some(end) = rest.find("</w:instrText>") {
                rest = &rest[end..];
            }
        }
    }
    out.lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// Cut long text, saying so.
pub fn clip(text: &str) -> String {
    if text.chars().count() <= MAX_TEXT {
        return text.to_string();
    }
    let cut: String = text.chars().take(MAX_TEXT).collect();
    format!("{cut}\n\n[… cut after {MAX_TEXT} characters]")
}

/// One line per entry: "folder  Reports" / "file  notes.txt  12 KB".
pub fn list_folder(dir: &Path, limit: usize) -> Result<String, String> {
    let entries =
        std::fs::read_dir(dir).map_err(|e| format!("Could not open {}: {e}", dir.display()))?;
    let mut rows: Vec<(bool, String, u64)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || name.eq_ignore_ascii_case("desktop.ini") {
                return None;
            }
            let meta = e.metadata().ok()?;
            Some((meta.is_dir(), name, meta.len()))
        })
        .collect();
    rows.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(a.1.to_lowercase().cmp(&b.1.to_lowercase()))
    });
    let total = rows.len();
    let mut out = format!("{} ({total} items)\n", dir.display());
    for (is_dir, name, size) in rows.iter().take(limit) {
        if *is_dir {
            out.push_str(&format!("folder  {name}\n"));
        } else {
            out.push_str(&format!("file    {name}  {}\n", human_size(*size)));
        }
    }
    if total > limit {
        out.push_str(&format!("… and {} more\n", total - limit));
    }
    Ok(out)
}

fn human_size(bytes: u64) -> String {
    match bytes {
        b if b < 1024 => format!("{b} B"),
        b if b < 1024 * 1024 => format!("{} KB", b / 1024),
        b => format!("{:.1} MB", b as f64 / 1_048_576.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_resolve_from_home() {
        let home = Path::new(r"C:\Users\sam");
        assert_eq!(
            resolve("Documents\\Reports", home),
            Path::new(r"C:\Users\sam\Documents\Reports")
        );
        assert_eq!(
            resolve("~/Downloads/a.pdf", home),
            Path::new(r"C:\Users\sam\Downloads/a.pdf")
        );
        assert_eq!(
            resolve("\"D:\\Work\\plan.docx\"", home),
            Path::new(r"D:\Work\plan.docx")
        );
    }

    #[test]
    fn secrets_are_off_limits() {
        for bad in [
            r"C:\Users\sam\.ssh\id_rsa",
            r"C:\Users\sam\AppData\Local\Google\Chrome\User Data\Default\Login Data",
            r"C:\Users\sam\project\.env",
            r"C:\Users\sam\project\.env.local",
            r"C:\Users\sam\certs\server.pem",
            r"C:\Users\sam\Passwords.kdbx",
            r"C:\Windows\System32\config\SAM",
            r"C:\Users\sam\.aws\credentials",
        ] {
            assert!(is_off_limits(Path::new(bad)), "{bad}");
        }
        for ok in [
            r"C:\Users\sam\Documents\falcon-update.docx",
            r"D:\Work\notes.txt",
            r"C:\Users\sam\Desktop\env-notes.md",
        ] {
            assert!(!is_off_limits(Path::new(ok)), "{ok}");
        }
    }

    #[test]
    fn programs_need_an_ok_to_open() {
        assert!(runs_something(Path::new("setup.EXE")));
        assert!(runs_something(Path::new("deploy.ps1")));
        assert!(runs_something(Path::new("Shortcut.lnk")));
        assert!(!runs_something(Path::new("report.docx")));
        assert!(!runs_something(Path::new("Reports")));
    }

    #[test]
    fn word_text_comes_out_in_paragraphs() {
        let xml = r#"<w:document><w:body><w:p><w:r><w:t>Project Falcon</w:t></w:r></w:p><w:p><w:r><w:t xml:space="preserve">Shipped: </w:t></w:r><w:r><w:t>login &amp; billing</w:t></w:r><w:r><w:tab/><w:t>done</w:t></w:r></w:p><w:p><w:r><w:instrText> PAGE </w:instrText></w:r></w:p></w:body></w:document>"#;
        assert_eq!(
            word_xml_text(xml),
            "Project Falcon\nShipped: login & billing\tdone"
        );
    }

    #[test]
    fn docx_from_a_real_zip() {
        use std::io::Write;
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            z.start_file(
                "word/document.xml",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            z.write_all(b"<w:document><w:body><w:p><w:r><w:t>Blocked: API keys</w:t></w:r></w:p></w:body></w:document>").unwrap();
            z.finish().unwrap();
        }
        assert_eq!(docx_text(buf.get_ref()).unwrap(), "Blocked: API keys");
        assert!(docx_text(b"not a zip").is_err());
    }

    #[test]
    fn kinds_and_clipping() {
        assert_eq!(kind_of(Path::new("a.MD")), Kind::Text);
        assert_eq!(kind_of(Path::new("a.docx")), Kind::Word);
        assert_eq!(kind_of(Path::new("a.pdf")), Kind::Pdf);
        assert_eq!(kind_of(Path::new("a.xlsx")), Kind::Other);
        let long = "x".repeat(MAX_TEXT + 10);
        assert!(clip(&long).ends_with("characters]"));
        assert_eq!(clip("short"), "short");
    }

    #[test]
    fn folder_listing() {
        let dir = std::env::temp_dir().join(format!("agent-desk-files-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Reports")).unwrap();
        std::fs::write(dir.join("b.txt"), "hello").unwrap();
        std::fs::write(dir.join("A.docx"), vec![0u8; 2048]).unwrap();
        std::fs::write(dir.join(".hidden"), "x").unwrap();
        let out = list_folder(&dir, 10).unwrap();
        let lines: Vec<&str> = out.lines().skip(1).collect();
        assert_eq!(
            lines,
            [
                "folder  Reports",
                "file    A.docx  2 KB",
                "file    b.txt  5 B"
            ]
        );
        assert!(list_folder(&dir, 1).unwrap().contains("… and 2 more"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
