//! Settings → AI agents: connect ScreenForge's MCP server (`sf mcp`, shipped next to the app's
//! executable) to the user's own agent. Claude Code is set up by running its CLI; Cursor and
//! VS Code by their install links; anything else from the copied command.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{channel, Receiver};

/// Name the server is registered under in every client.
const NAME: &str = "screenforge";

/// The command line tool next to this executable (resolving a symlink), when it ships with it.
pub fn cli_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
    let sf = exe.parent()?.join(format!("sf{}", std::env::consts::EXE_SUFFIX));
    sf.is_file().then_some(sf)
}

fn quoted(path: &Path) -> String {
    let s = path.display().to_string();
    if s.chars().any(|c| c.is_whitespace() || "'\"$`\\".contains(c)) {
        format!("'{}'", s.replace('\'', r"'\''"))
    } else {
        s
    }
}

/// The command that adds the server to Claude Code, for copying.
pub fn claude_command(sf: &Path) -> String {
    format!("claude mcp add --scope user {NAME} -- {} mcp", quoted(sf))
}

/// Percent-encode everything but RFC 3986's unreserved characters.
fn encode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// Cursor's one-click MCP install link.
pub fn cursor_link(sf: &Path) -> String {
    use base64::Engine;
    let config = serde_json::json!({ "command": sf, "args": ["mcp"] }).to_string();
    format!("cursor://anysphere.cursor-deeplink/mcp/install?name={NAME}&config={}", encode(&base64::engine::general_purpose::STANDARD.encode(config)))
}

/// VS Code's MCP install link.
pub fn vscode_link(sf: &Path) -> String {
    let config = serde_json::json!({ "name": NAME, "command": sf, "args": ["mcp"] }).to_string();
    format!("vscode:mcp/install?{}", encode(&config))
}

/// Where Claude Code is. An app started from the Dock does not get the shell's PATH, so the
/// usual install places are tried first, then the user's login shell is asked.
fn find_claude() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let known = [home.join(".local/bin/claude"), home.join(".claude/local/claude"), PathBuf::from("/opt/homebrew/bin/claude"), PathBuf::from("/usr/local/bin/claude"), home.join(".npm-global/bin/claude"), home.join(".bun/bin/claude")];
    if let Some(p) = known.into_iter().find(|p| p.is_file()) {
        return Some(p);
    }
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    let out = Command::new(shell).args(["-lic", "command -v claude"]).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).output().ok()?;
    let line = String::from_utf8_lossy(&out.stdout).lines().last()?.trim().to_string();
    let p = PathBuf::from(line);
    (p.is_absolute() && p.is_file()).then_some(p)
}

#[derive(Debug, Clone, PartialEq)]
pub enum Claude {
    /// Not looked at yet.
    Unknown,
    Working,
    /// ScreenForge is one of Claude Code's MCP servers.
    Connected,
    NotConnected,
    /// Claude Code is not installed (or not where it could be found).
    Missing,
    Failed(String),
}

/// Claude Code's side, checked on a background thread when the menu first opens.
pub struct Agents {
    pub claude: Claude,
    rx: Option<Receiver<Claude>>,
}

impl Default for Agents {
    fn default() -> Self {
        Agents { claude: Claude::Unknown, rx: None }
    }
}

impl Agents {
    /// Take a finished check or add. Call every frame the menu shows.
    pub fn poll(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.rx {
            match rx.try_recv() {
                Ok(c) => {
                    self.claude = c;
                    self.rx = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(150)),
                Err(_) => self.rx = None,
            }
        }
    }

    /// Ask Claude Code whether ScreenForge is connected (once).
    pub fn check(&mut self) {
        if self.claude != Claude::Unknown {
            return;
        }
        self.run(|claude| {
            let ok = Command::new(claude).args(["mcp", "get", NAME]).stdin(std::process::Stdio::null()).output().map(|o| o.status.success()).unwrap_or(false);
            if ok { Claude::Connected } else { Claude::NotConnected }
        });
    }

    /// Add ScreenForge to Claude Code for every project (user scope).
    pub fn add(&mut self, sf: PathBuf) {
        self.run(move |claude| {
            let out = match Command::new(claude).args(["mcp", "add", "--scope", "user", NAME, "--"]).arg(&sf).arg("mcp").stdin(std::process::Stdio::null()).output() {
                Ok(o) => o,
                Err(e) => return Claude::Failed(e.to_string()),
            };
            let said = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
            if out.status.success() || said.contains("already exists") {
                Claude::Connected
            } else {
                Claude::Failed(said.lines().find(|l| !l.trim().is_empty()).unwrap_or("claude mcp add failed").trim().to_string())
            }
        });
    }

    fn run(&mut self, f: impl FnOnce(PathBuf) -> Claude + Send + 'static) {
        let (tx, rx) = channel();
        self.claude = Claude::Working;
        self.rx = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send(match find_claude() {
                Some(c) => f(c),
                None => Claude::Missing,
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_carry_the_command() {
        let sf = Path::new("/Applications/ScreenForge.app/Contents/MacOS/sf");
        assert_eq!(claude_command(sf), "claude mcp add --scope user screenforge -- /Applications/ScreenForge.app/Contents/MacOS/sf mcp");
        assert_eq!(claude_command(Path::new("/a b/sf")), "claude mcp add --scope user screenforge -- '/a b/sf' mcp");
        let v = vscode_link(sf);
        assert!(v.starts_with("vscode:mcp/install?%7B"));
        assert!(!v[19..].contains('/'));
        let c = cursor_link(sf);
        let b64 = c.split("config=").nth(1).unwrap().replace("%3D", "=").replace("%2B", "+").replace("%2F", "/");
        use base64::Engine;
        let json: serde_json::Value = serde_json::from_slice(&base64::engine::general_purpose::STANDARD.decode(b64).unwrap()).unwrap();
        assert_eq!(json["args"][0], "mcp");
        assert_eq!(json["command"], sf.display().to_string());
    }
}
