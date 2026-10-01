//! The only provider CLIs the monitor runs, and only to let them renew their own sign-in.
//!
//! Each variant is an authenticated command that runs **no model**: the CLI refreshes its token
//! (with its own locking, so a running session is never signed out) and exits. Never add a
//! command that sends a prompt (`claude -p …`, `agy -p …`, `codex exec …`), or a script that
//! starts a thread or turn.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use pw_model::PollEnv;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VendorCmd {
    /// `claude doctor`: checks the install and fetches remote settings, which renews an expired
    /// token. (`claude auth status` only reads the stored sign-in, so it never renews it.)
    ClaudeDoctor,
    /// `agy models`: lists models (an authenticated metadata call); renews an expired token.
    AgyModels,
    /// `codex app-server`, asked over stdin for the signed-in account with `refreshToken`: Codex
    /// renews its ChatGPT sign-in and answers. See [`CODEX_ACCOUNT_READ`].
    CodexAccountRead,
}

/// The whole conversation with `codex app-server`: the JSON-RPC handshake, then `account/read`.
/// The server exits when its stdin closes, which happens once request 2 is answered.
const CODEX_ACCOUNT_READ: &str = concat!(
    r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"clientInfo":{"name":"portal-workspaces","version":""#,
    env!("CARGO_PKG_VERSION"),
    r#""}}}"#,
    "\n",
    r#"{"jsonrpc":"2.0","method":"initialized"}"#,
    "\n",
    r#"{"jsonrpc":"2.0","id":2,"method":"account/read","params":{"refreshToken":true}}"#,
    "\n",
);

impl VendorCmd {
    fn program(self) -> &'static str {
        match self {
            VendorCmd::ClaudeDoctor => "claude",
            VendorCmd::AgyModels => "agy",
            VendorCmd::CodexAccountRead => "codex",
        }
    }

    fn args(self) -> &'static [&'static str] {
        match self {
            VendorCmd::ClaudeDoctor => &["doctor"],
            VendorCmd::AgyModels => &["models"],
            VendorCmd::CodexAccountRead => &["app-server"],
        }
    }

    /// What the command reads on stdin, and the JSON-RPC id whose answer means it's done.
    fn script(self) -> Option<(&'static str, u64)> {
        match self {
            VendorCmd::CodexAccountRead => Some((CODEX_ACCOUNT_READ, 2)),
            VendorCmd::ClaudeDoctor | VendorCmd::AgyModels => None,
        }
    }
}

const TIMEOUT: Duration = Duration::from_secs(30);

/// Runs `cmd` with the profile's environment and returns its stdout.
pub fn run(cmd: VendorCmd, env: &PollEnv) -> Result<String, String> {
    let program =
        which(cmd.program()).ok_or_else(|| format!("`{}` isn't installed (or not on PATH).", cmd.program()))?;
    let script = cmd.script();
    let mut command = Command::new(&program);
    // `claude doctor` reads the settings files in its working directory, so don't let it pick up
    // whatever project the app happened to be started from.
    if env.dir.is_dir() {
        command.current_dir(&env.dir);
    }
    let mut child = command
        .args(cmd.args())
        .envs(env.vars.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .stdin(if script.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| format!("Couldn't start {}: {err}", program.display()))?;
    let mut stdin = child.stdin.take();
    if let (Some(pipe), Some((input, _))) = (stdin.as_mut(), script) {
        let _ = pipe.write_all(input.as_bytes());
    }
    let stdout = child.stdout.take().expect("piped");
    let (answered, on_answer) = mpsc::channel();
    // Read on a helper thread so a chatty child can't fill the pipe and stall while we wait.
    let reader = std::thread::spawn(move || {
        let mut out = String::new();
        for line in BufReader::new(stdout.take(1 << 20)).lines().map_while(Result::ok) {
            if script.is_some_and(|(_, id)| answers(&line, id)) {
                let _ = answered.send(());
            }
            out.push_str(&line);
            out.push('\n');
        }
        out
    });
    let started = Instant::now();
    let status = loop {
        if on_answer.try_recv().is_ok() {
            stdin = None; // EOF: a scripted server exits once it has answered.
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < TIMEOUT => std::thread::sleep(Duration::from_millis(50)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("`{}` took too long.", cmd.program()));
            }
            Err(err) => return Err(err.to_string()),
        }
    };
    drop(stdin);
    let out = reader.join().unwrap_or_default();
    if status.success() { Ok(out) } else { Err(format!("`{}` exited with {status}.", cmd.program())) }
}

/// Whether a JSON-RPC line is the response to request `id`.
fn answers(line: &str, id: u64) -> bool {
    serde_json::from_str::<serde_json::Value>(line).is_ok_and(|msg| msg.get("id").and_then(|v| v.as_u64()) == Some(id))
}

/// Finds a program on `PATH`, then in the places installers put CLIs. Apps started from a
/// desktop launcher often get a minimal `PATH` without `~/.local/bin`.
pub fn which(program: &str) -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let extra = [
        home.join(".local/bin"),
        home.join(".claude/local"),
        home.join(".npm-global/bin"),
        home.join(".bun/bin"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/usr/bin"),
    ];
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path).chain(extra).map(|dir| dir.join(program)).find(|p| is_executable(p))
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_never_prompt_a_model() {
        for cmd in [VendorCmd::ClaudeDoctor, VendorCmd::AgyModels, VendorCmd::CodexAccountRead] {
            let args = cmd.args();
            for flag in ["-p", "--print", "--prompt", "exec", "-i", "--prompt-interactive", "-c", "--continue"] {
                assert!(!args.contains(&flag), "{cmd:?} must not run a model");
            }
            // A scripted server may only be asked about the account, never to start a thread or turn.
            let Some((input, id)) = cmd.script() else { continue };
            let methods: Vec<String> = input
                .lines()
                .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("valid JSON-RPC"))
                .map(|msg| msg["method"].as_str().expect("a method").to_owned())
                .collect();
            assert_eq!(methods, ["initialize", "initialized", "account/read"], "{cmd:?}");
            // The command is done once the last request (account/read) is answered.
            assert!(input.lines().last().is_some_and(|last| answers(last, id)));
        }
    }

    #[test]
    fn finds_sh() {
        assert!(which("sh").is_some());
        assert!(which("definitely-not-a-real-program-pw").is_none());
    }
}
