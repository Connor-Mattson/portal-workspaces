//! Finding the AI CLIs already set up on this machine, so adding a profile is one click.
//!
//! Only looks at directory names and small settings and sign-in files, for who each account is
//! (email, plan). Never contacts anything.

use std::path::Path;

use pw_model::{Provider, UsageProfile};

use crate::{claude, codex, display_path, title_case};

/// A profile the user could add, with a line saying which account it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub profile: UsageProfile,
    pub detail: String,
}

/// Every install found under `home`: each `~/.claude*` config dir, Codex and Antigravity.
pub fn discover(home: &Path) -> Vec<Candidate> {
    let mut found = Vec::new();
    let mut claude_dirs: Vec<String> = std::fs::read_dir(home)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter(|e| e.path().is_dir())
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|name| name == ".claude" || name.starts_with(".claude-") || name.starts_with(".claude_"))
                .collect()
        })
        .unwrap_or_default();
    claude_dirs.sort();
    for name in claude_dirs {
        let dir = home.join(&name);
        // A config dir someone signed in from has credentials (Linux) or settings (macOS, where
        // the credentials are in the Keychain).
        let signed_in = dir.join(".credentials.json").exists()
            || dir.join(".claude.json").exists()
            || (name == ".claude" && home.join(".claude.json").exists());
        if !signed_in {
            continue;
        }
        let (label, instructions) = match name.strip_prefix(".claude").map(|s| s.trim_start_matches(['-', '_'])) {
            Some("") | None => ("Claude".to_owned(), String::new()),
            Some(suffix) => (title_case(&suffix.replace(['-', '_'], " ")), format!("CLAUDE_CONFIG_DIR=~/{name}")),
        };
        let profile = UsageProfile::new(label, Provider::Claude, instructions);
        if let Ok(detail) = describe(&profile, home) {
            found.push((claude::account(&profile.env(home).expect("described")), Candidate { profile, detail }));
        }
    }
    // The default ~/.claude is often the same account as one of the named dirs; offer it once,
    // under the name the user gave it.
    let named: Vec<Option<String>> =
        found.iter().filter(|(_, c)| !c.profile.instructions.is_empty()).map(|(a, _)| a.clone()).collect();
    let mut found: Vec<Candidate> = found
        .into_iter()
        .filter(|(account, c)| !c.profile.instructions.is_empty() || account.is_none() || !named.contains(account))
        .map(|(_, c)| c)
        .collect();

    for (provider, marker) in [(Provider::Codex, ".codex"), (Provider::Antigravity, ".gemini/antigravity-cli")] {
        if home.join(marker).is_dir() {
            let profile = UsageProfile::new(provider.label(), provider, "");
            if let Ok(detail) = describe(&profile, home) {
                found.push(Candidate { profile, detail });
            }
        }
    }
    found
}

/// One line saying what a profile will read ("~/.claude-work · me@work.dev · Pro"), or why it
/// can't. Cheap enough to call as the user types.
pub fn describe(profile: &UsageProfile, home: &Path) -> Result<String, String> {
    let env = profile.env(home)?;
    if !env.dir.is_dir() {
        return Err(format!("{} doesn't exist.", display_path(&env.dir)));
    }
    let mut parts = vec![display_path(&env.dir)];
    match profile.provider {
        Provider::Claude => {
            let creds = std::fs::read_to_string(env.dir.join(".credentials.json")).ok();
            if creds.is_none() && !cfg!(target_os = "macos") {
                return Err(format!("No Claude sign-in in {}. Run {} and /login.", parts[0], env.command_hint()));
            }
            parts.extend(claude::account(&env));
            parts.extend(creds.as_deref().and_then(claude::parse_credentials).and_then(|c| c.plan));
        }
        Provider::Codex => {
            let auth = std::fs::read_to_string(env.dir.join("auth.json")).ok().as_deref().and_then(codex::parse_auth);
            if auth.is_none() && !env.dir.join("sessions").is_dir() {
                return Err(format!("No Codex sign-in in {}. Run {} and sign in.", parts[0], env.command_hint()));
            }
            if let Some(auth) = auth {
                parts.extend(auth.email);
                parts.extend(auth.plan);
            }
        }
        Provider::Antigravity => {
            if !env.dir.join("antigravity-cli").is_dir() {
                return Err(format!("No Antigravity CLI data in {}.", parts[0]));
            }
        }
    }
    Ok(parts.join(" · "))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn finds_each_install() {
        let home = tempfile::tempdir().unwrap();
        let h = home.path();
        for dir in [
            ".claude",
            ".claude-work",
            ".claude-personal",
            ".claude-empty",
            ".codex/sessions",
            ".gemini/antigravity-cli",
        ] {
            fs::create_dir_all(h.join(dir)).unwrap();
        }
        let creds = r#"{"claudeAiOauth":{"accessToken":"t","subscriptionType":"pro"}}"#;
        for dir in [".claude", ".claude-work", ".claude-personal"] {
            fs::write(h.join(dir).join(".credentials.json"), creds).unwrap();
        }
        fs::write(h.join(".claude-work/.claude.json"), r#"{"oauthAccount":{"emailAddress":"me@work.dev"}}"#).unwrap();
        fs::write(h.join(".claude-personal/.claude.json"), r#"{"oauthAccount":{"emailAddress":"me@home.dev"}}"#)
            .unwrap();

        let found = discover(h);
        let names: Vec<_> = found.iter().map(|c| c.profile.name.as_str()).collect();
        assert_eq!(names, ["Claude", "Personal", "Work", "Codex", "Antigravity"]);
        let work = &found[2];
        assert_eq!(found[0].profile.instructions, "");

        // Once the default dir turns out to be the personal account, it's offered only as Personal.
        fs::write(h.join(".claude.json"), r#"{"oauthAccount":{"emailAddress":"me@home.dev"}}"#).unwrap();
        let names: Vec<_> = discover(h).into_iter().map(|c| c.profile.name).collect();
        assert_eq!(names, ["Personal", "Work", "Codex", "Antigravity"]);
        assert_eq!(work.profile.instructions, "CLAUDE_CONFIG_DIR=~/.claude-work");
        assert!(work.detail.ends_with(".claude-work · me@work.dev · Pro"), "{}", work.detail);
    }

    #[test]
    fn describe_explains_problems() {
        let home = tempfile::tempdir().unwrap();
        let p = UsageProfile::new("x", Provider::Claude, "~/.claude-nope");
        assert!(describe(&p, home.path()).unwrap_err().contains("doesn't exist"));
        let p = UsageProfile::new("x", Provider::Codex, "");
        assert!(describe(&p, home.path()).is_err());
    }
}
