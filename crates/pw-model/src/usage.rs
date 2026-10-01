//! AI usage profiles: which accounts the drawer tracks, and how to find each one on disk.
//!
//! A profile names a provider (Claude Code, Codex, Antigravity) and, optionally, how that
//! account differs from the default install. The "how" is whatever the user already has, e.g.
//! their shell alias `alias claude-work='CLAUDE_CONFIG_DIR=~/.claude-work claude'`, and
//! [`PollEnv::parse`] turns it into a home directory and environment.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::ids::ProfileId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Claude,
    Codex,
    Antigravity,
}

impl Provider {
    pub const ALL: [Provider; 3] = [Provider::Claude, Provider::Codex, Provider::Antigravity];

    pub fn label(self) -> &'static str {
        match self {
            Provider::Claude => "Claude",
            Provider::Codex => "Codex",
            Provider::Antigravity => "Antigravity",
        }
    }

    /// The CLI's own variable for "use this directory instead of the default".
    pub fn home_var(self) -> Option<&'static str> {
        match self {
            Provider::Claude => Some("CLAUDE_CONFIG_DIR"),
            Provider::Codex => Some("CODEX_HOME"),
            Provider::Antigravity => None,
        }
    }

    /// Where the CLI keeps its data when nothing overrides it, relative to the user's home.
    pub fn default_home(self, home: &Path) -> PathBuf {
        home.join(match self {
            Provider::Claude => ".claude",
            Provider::Codex => ".codex",
            Provider::Antigravity => ".gemini",
        })
    }

    /// The command people type, for hints like "run claude to refresh".
    pub fn command(self) -> &'static str {
        match self {
            Provider::Claude => "claude",
            Provider::Codex => "codex",
            Provider::Antigravity => "agy",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageProfile {
    pub id: ProfileId,
    pub name: String,
    pub provider: Provider,
    /// Optional: an alias, env assignments or a directory saying where this account lives.
    #[serde(default)]
    pub instructions: String,
}

impl UsageProfile {
    pub fn new(name: impl Into<String>, provider: Provider, instructions: impl Into<String>) -> Self {
        Self { id: ProfileId::new(), name: name.into(), provider, instructions: instructions.into() }
    }

    pub fn env(&self, home: &Path) -> Result<PollEnv, String> {
        PollEnv::parse(self.provider, &self.instructions, home)
    }
}

/// A profile's resolved location: the CLI's data directory and the environment to run its CLI
/// with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollEnv {
    pub provider: Provider,
    /// The CLI's data directory (e.g. `~/.claude-work`).
    pub dir: PathBuf,
    /// Extra environment for the CLI, with `~` and `$HOME` already expanded.
    pub vars: Vec<(String, String)>,
    /// The alias name, if the instructions were an alias (used in hints: "run claude-work").
    pub alias: Option<String>,
}

impl PollEnv {
    /// Understands, in order of how people usually have it:
    /// - nothing (the default install),
    /// - an alias line: `alias claude-work='CLAUDE_CONFIG_DIR=~/.claude-work claude'`,
    /// - assignments, with or without the command: `CLAUDE_CONFIG_DIR=~/.claude-work claude`,
    ///   `export CODEX_HOME=~/.codex-2`,
    /// - a bare directory: `~/.claude-work`.
    pub fn parse(provider: Provider, instructions: &str, home: &Path) -> Result<PollEnv, String> {
        let mut text = instructions.trim();
        let mut alias = None;
        if let Some(rest) = text.strip_prefix("alias ") {
            let (name, body) = rest.trim_start().split_once('=').ok_or("An alias needs `name=value`.")?;
            alias = Some(name.trim().to_owned());
            text = body.trim();
        }
        let text = unquote(text);

        let mut vars = Vec::new();
        let mut path = None;
        for token in text.split_whitespace() {
            if matches!(token, "export" | "env") {
                continue;
            }
            if let Some((key, value)) = token.split_once('=').filter(|(k, _)| is_env_name(k)) {
                vars.push((key.to_owned(), expand(unquote(value), home)));
                continue;
            }
            if vars.is_empty() && path.is_none() && looks_like_path(token) {
                path = Some(expand(unquote(token), home));
            }
            // Anything else is the command itself (`claude`, `codex --profile x`, …); the rest of
            // the line is its arguments and doesn't say where the account lives.
            break;
        }

        let from_var = provider.home_var().and_then(|var| vars.iter().find(|(k, _)| k == var)).map(|(_, v)| v.clone());
        let dir = match (from_var, path) {
            (Some(dir), _) => PathBuf::from(dir),
            (None, Some(dir)) => {
                if let Some(var) = provider.home_var() {
                    vars.push((var.to_owned(), dir.clone()));
                }
                PathBuf::from(dir)
            }
            (None, None) if vars.is_empty() && !text.is_empty() && alias.is_none() && !is_command(text, provider) => {
                return Err(format!(
                    "Couldn't find a folder or {} in that. Paste your alias, or a path like ~/{}.",
                    provider.home_var().unwrap_or("an assignment"),
                    provider.default_home(Path::new("")).display()
                ));
            }
            (None, None) => provider.default_home(home),
        };
        Ok(PollEnv { provider, dir, vars, alias })
    }

    /// What to tell people to run to wake this account up.
    pub fn command_hint(&self) -> String {
        self.alias.clone().unwrap_or_else(|| {
            let mut parts: Vec<String> = self.vars.iter().map(|(k, v)| format!("{k}={v}")).collect();
            parts.push(self.provider.command().to_owned());
            parts.join(" ")
        })
    }
}

fn is_command(text: &str, provider: Provider) -> bool {
    text.split_whitespace().next() == Some(provider.command())
}

fn is_env_name(key: &str) -> bool {
    let mut chars = key.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn looks_like_path(token: &str) -> bool {
    let token = unquote(token);
    ["/", "~", "./", "../", "$HOME", "${HOME}"].iter().any(|p| token.starts_with(p))
}

fn unquote(s: &str) -> &str {
    let s = s.trim();
    for q in ['\'', '"'] {
        if let Some(inner) = s.strip_prefix(q).and_then(|r| r.strip_suffix(q)) {
            return inner;
        }
    }
    s
}

/// Expands a leading `~`, `$HOME` or `${HOME}`.
fn expand(value: &str, home: &Path) -> String {
    for prefix in ["${HOME}", "$HOME", "~"] {
        if let Some(rest) = value.strip_prefix(prefix)
            && (rest.is_empty() || rest.starts_with('/'))
        {
            let rest = rest.trim_start_matches('/');
            let path = if rest.is_empty() { home.to_path_buf() } else { home.join(rest) };
            return path.to_string_lossy().into_owned();
        }
    }
    value.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: &str = "/home/me";

    fn parse(provider: Provider, text: &str) -> Result<PollEnv, String> {
        PollEnv::parse(provider, text, Path::new(HOME))
    }

    #[test]
    fn empty_means_default_install() {
        let env = parse(Provider::Claude, "  ").unwrap();
        assert_eq!(env.dir, PathBuf::from("/home/me/.claude"));
        assert!(env.vars.is_empty());
        assert_eq!(parse(Provider::Codex, "").unwrap().dir, PathBuf::from("/home/me/.codex"));
        assert_eq!(parse(Provider::Antigravity, "").unwrap().dir, PathBuf::from("/home/me/.gemini"));
    }

    #[test]
    fn alias_line() {
        let env = parse(Provider::Claude, "alias claude-work='CLAUDE_CONFIG_DIR=~/.claude-work claude'").unwrap();
        assert_eq!(env.dir, PathBuf::from("/home/me/.claude-work"));
        assert_eq!(env.vars, vec![("CLAUDE_CONFIG_DIR".to_owned(), "/home/me/.claude-work".to_owned())]);
        assert_eq!(env.alias.as_deref(), Some("claude-work"));
        assert_eq!(env.command_hint(), "claude-work");
    }

    #[test]
    fn assignments_with_and_without_command() {
        for text in ["CLAUDE_CONFIG_DIR=$HOME/.claude-personal claude", "CLAUDE_CONFIG_DIR=\"~/.claude-personal\""] {
            let env = parse(Provider::Claude, text).unwrap();
            assert_eq!(env.dir, PathBuf::from("/home/me/.claude-personal"), "{text}");
        }
        let env = parse(Provider::Codex, "export CODEX_HOME=${HOME}/.codex-2").unwrap();
        assert_eq!(env.dir, PathBuf::from("/home/me/.codex-2"));
        assert_eq!(env.command_hint(), "CODEX_HOME=/home/me/.codex-2 codex");
    }

    #[test]
    fn other_vars_are_kept_but_dont_move_the_dir() {
        let env = parse(Provider::Claude, "HTTPS_PROXY=http://p:3128 claude").unwrap();
        assert_eq!(env.dir, PathBuf::from("/home/me/.claude"));
        assert_eq!(env.vars.len(), 1);
    }

    #[test]
    fn bare_path_sets_the_home_var() {
        let env = parse(Provider::Claude, "~/.claude-work").unwrap();
        assert_eq!(env.dir, PathBuf::from("/home/me/.claude-work"));
        assert_eq!(env.vars, vec![("CLAUDE_CONFIG_DIR".to_owned(), "/home/me/.claude-work".to_owned())]);
        let env = parse(Provider::Antigravity, "/opt/gemini").unwrap();
        assert_eq!(env.dir, PathBuf::from("/opt/gemini"));
        assert!(env.vars.is_empty());
    }

    #[test]
    fn just_the_command_is_the_default() {
        assert_eq!(parse(Provider::Claude, "claude").unwrap().dir, PathBuf::from("/home/me/.claude"));
    }

    #[test]
    fn garbage_is_explained() {
        assert!(parse(Provider::Claude, "use my work account please").is_err());
        assert!(parse(Provider::Claude, "alias nope").is_err());
    }

    #[test]
    fn tilde_only_expands_at_a_boundary() {
        assert_eq!(expand("~", Path::new(HOME)), "/home/me");
        assert_eq!(expand("~bob/x", Path::new(HOME)), "~bob/x");
        assert_eq!(expand("$HOMEDIR", Path::new(HOME)), "$HOMEDIR");
    }
}
