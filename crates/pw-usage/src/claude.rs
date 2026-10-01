//! Claude subscriptions: the usage endpoint Claude Code's `/usage` reads, with the account's own
//! sign-in.
//!
//! Credentials are read, never written. When the access token has expired, `claude auth status`
//! (run with the profile's environment) lets Claude Code renew it the way it always does.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use pw_model::PollEnv;
use serde_json::Value;

use crate::http::{Endpoint, Http, Secret};
use crate::report::{Failure, Report, Span, Window, parse_time};
use crate::vendor::{self, VendorCmd};
use crate::{ProviderState, display_path, title_case};

pub(crate) struct Credentials {
    token: Secret,
    expires_at: Option<SystemTime>,
    pub plan: Option<String>,
}

impl Credentials {
    fn expired(&self, now: SystemTime) -> bool {
        self.expires_at.is_some_and(|t| t <= now + Duration::from_secs(60))
    }
}

pub fn poll(env: &PollEnv, state: &mut ProviderState, http: &Http) -> Result<Report, Failure> {
    if !env.dir.is_dir() {
        return Err(Failure::NotFound(format!("{} doesn't exist.", display_path(&env.dir))));
    }
    let mut creds = load_credentials(env)?;
    if creds.expired(SystemTime::now()) {
        if state.may_renew() {
            renew(env);
            creds = load_credentials(env)?;
        }
        if creds.expired(SystemTime::now()) {
            return Err(Failure::Idle { hint: env.command_hint() });
        }
    }

    let mut response = fetch(http, &creds)?;
    if matches!(response.status, 401 | 403) && state.may_renew() {
        renew(env);
        creds = load_credentials(env)?;
        response = fetch(http, &creds)?;
    }
    let now = SystemTime::now();
    let mut report = match response.status {
        200..=299 => parse_usage(&response.body, now)?,
        401 | 403 => {
            return Err(Failure::SignedOut(format!(
                "Claude didn't accept this sign-in. Run {} and /login.",
                env.command_hint()
            )));
        }
        429 => {
            return Err(Failure::RateLimited { retry_after: response.retry_after.unwrap_or(Duration::from_secs(300)) });
        }
        status => return Err(Failure::Unavailable(format!("Claude's usage service answered HTTP {status}."))),
    };
    report.plan = creds.plan;
    report.account = account(env);
    Ok(report)
}

fn fetch(http: &Http, creds: &Credentials) -> Result<crate::http::Response, Failure> {
    http.get(
        Endpoint::ClaudeUsage,
        &creds.token,
        &[("anthropic-beta", "oauth-2025-04-20"), ("Accept", "application/json")],
    )
    .map_err(Failure::Unavailable)
}

fn renew(env: &PollEnv) {
    if let Err(err) = vendor::run(VendorCmd::ClaudeAuthStatus, env) {
        tracing::info!(%err, dir = %env.dir.display(), "claude couldn't renew its sign-in");
    }
}

/// The OAuth credentials Claude Code stored for this config dir.
pub(crate) fn load_credentials(env: &PollEnv) -> Result<Credentials, Failure> {
    let file = env.dir.join(".credentials.json");
    let text = match std::fs::read_to_string(&file) {
        Ok(text) => Some(text),
        Err(_) => keychain_credentials(env),
    };
    text.as_deref().and_then(parse_credentials).ok_or_else(|| {
        Failure::NotFound(format!(
            "No Claude sign-in for {}. Run {} and /login.",
            display_path(&env.dir),
            env.command_hint()
        ))
    })
}

#[cfg(target_os = "macos")]
fn keychain_credentials(env: &PollEnv) -> Option<String> {
    use sha2::{Digest, Sha256};
    const SERVICE: &str = "Claude Code-credentials";
    // With CLAUDE_CONFIG_DIR set, Claude Code suffixes the item with a hash of the directory.
    let custom = env.vars.iter().find(|(k, _)| k == "CLAUDE_CONFIG_DIR").map(|(_, dir)| {
        let hash = Sha256::digest(dir.as_bytes());
        let hex: String = hash.iter().take(4).map(|b| format!("{b:02x}")).collect();
        format!("{SERVICE}-{hex}")
    });
    custom.and_then(|s| crate::secrets::keychain(&s, None)).or_else(|| crate::secrets::keychain(SERVICE, None))
}

#[cfg(not(target_os = "macos"))]
fn keychain_credentials(_env: &PollEnv) -> Option<String> {
    None
}

pub(crate) fn parse_credentials(text: &str) -> Option<Credentials> {
    let doc: Value = serde_json::from_str(text).ok()?;
    let oauth = doc.get("claudeAiOauth")?;
    let token = oauth.get("accessToken")?.as_str().filter(|t| !t.is_empty())?;
    let expires_at =
        oauth.get("expiresAt").and_then(Value::as_f64).and_then(|ms| crate::report::from_unix(ms / 1000.0));
    let plan = oauth.get("subscriptionType").and_then(Value::as_str).filter(|p| !p.is_empty()).map(title_case);
    Some(Credentials { token: Secret::new(token), expires_at, plan })
}

/// The account's email, from Claude Code's settings file for this config dir.
pub(crate) fn account(env: &PollEnv) -> Option<String> {
    let settings = settings_path(env);
    let doc: Value = serde_json::from_str(&std::fs::read_to_string(settings).ok()?).ok()?;
    doc.pointer("/oauthAccount/emailAddress").and_then(Value::as_str).map(str::to_owned)
}

/// `.claude.json` lives inside a custom config dir, but next to the default `~/.claude`.
fn settings_path(env: &PollEnv) -> PathBuf {
    let custom = env.vars.iter().any(|(k, _)| k == "CLAUDE_CONFIG_DIR");
    let inside = env.dir.join(".claude.json");
    if custom || inside.exists() {
        return inside;
    }
    env.dir.parent().unwrap_or(Path::new("/")).join(".claude.json")
}

/// Every `five_hour*` / `seven_day*` object with a `utilization` becomes a window, so scoped limits
/// the service adds later (per model, per feature) show up without a code change.
pub(crate) fn parse_usage(body: &str, now: SystemTime) -> Result<Report, Failure> {
    let doc: Value = serde_json::from_str(body)
        .map_err(|_| Failure::Unavailable("Claude's usage service sent something unexpected.".into()))?;
    let object = doc.as_object().ok_or_else(|| Failure::Unavailable("Claude's usage response was empty.".into()))?;
    let mut windows = Vec::new();
    for (key, value) in object {
        let (span, rest) = if let Some(rest) = key.strip_prefix("five_hour") {
            (Span::FiveHour, rest)
        } else if let Some(rest) = key.strip_prefix("seven_day") {
            (Span::Weekly, rest)
        } else {
            continue;
        };
        let Some(used) = value.get("utilization").and_then(Value::as_f64) else { continue };
        let resets_at = value.get("resets_at").and_then(Value::as_str).and_then(parse_time);
        windows.push(Window::new(span, scope(rest), used as f32, resets_at));
    }
    if windows.is_empty() {
        return Err(Failure::Unavailable("Claude's usage response had no limits in it.".into()));
    }
    Ok(Report::new(windows, now, true))
}

fn scope(suffix: &str) -> Option<String> {
    let suffix = suffix.trim_matches('_');
    match suffix {
        "" => None,
        "oauth_apps" => Some("OAuth apps".into()),
        other => Some(title_case(&other.replace('_', " "))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../fixtures/claude_usage.json");

    fn at(secs: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[test]
    fn parses_the_usage_response() {
        let report = parse_usage(FIXTURE, at(1)).unwrap();
        let five = report.window(Span::FiveHour).unwrap();
        assert_eq!(five.used, 22.0);
        assert_eq!(five.resets_at, parse_time("2026-10-01T14:00:00.364238+00:00"));
        assert_eq!(report.window(Span::Weekly).unwrap().used, 49.0);
        // Null scoped windows are skipped; present ones are kept, after the headline ones.
        let scopes: Vec<_> = report.windows.iter().map(|w| w.scope.clone()).collect();
        assert_eq!(scopes, vec![None, None, Some("Opus".to_owned())]);
        assert!(report.live);
    }

    #[test]
    fn a_response_without_limits_is_an_error() {
        assert!(matches!(parse_usage("{\"extra_usage\": {\"utilization\": 3}}", at(1)), Err(Failure::Unavailable(_))));
        assert!(matches!(parse_usage("<html>", at(1)), Err(Failure::Unavailable(_))));
    }

    #[test]
    fn reads_credentials() {
        let creds = parse_credentials(
            r#"{"claudeAiOauth":{"accessToken":"sk-ant-oat01-x","refreshToken":"r","expiresAt":2000,"subscriptionType":"max"}}"#,
        )
        .unwrap();
        assert_eq!(creds.plan.as_deref(), Some("Max"));
        assert_eq!(creds.expires_at, Some(at(2)));
        // A token counts as expired a minute early, so it can't lapse mid-request.
        assert!(creds.expired(at(0)));
        assert!(parse_credentials(r#"{"claudeAiOauth":{"accessToken":""}}"#).is_none());
        assert!(parse_credentials("not json").is_none());
    }

    #[test]
    fn finds_the_account_email() {
        let home = tempfile::tempdir().unwrap();
        let work = home.path().join(".claude-work");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::write(work.join(".claude.json"), r#"{"oauthAccount":{"emailAddress":"me@work.dev"}}"#).unwrap();
        let env = PollEnv::parse(pw_model::Provider::Claude, "~/.claude-work", home.path()).unwrap();
        assert_eq!(account(&env).as_deref(), Some("me@work.dev"));

        // The default dir keeps its settings next to it, as ~/.claude.json.
        std::fs::create_dir_all(home.path().join(".claude")).unwrap();
        std::fs::write(home.path().join(".claude.json"), r#"{"oauthAccount":{"emailAddress":"me@home.dev"}}"#).unwrap();
        let env = PollEnv::parse(pw_model::Provider::Claude, "", home.path()).unwrap();
        assert_eq!(account(&env).as_deref(), Some("me@home.dev"));
    }

    #[test]
    fn missing_dir_is_not_found() {
        let home = tempfile::tempdir().unwrap();
        let env = PollEnv::parse(pw_model::Provider::Claude, "~/.claude-nope", home.path()).unwrap();
        let err = poll(&env, &mut ProviderState::default(), &Http::default()).unwrap_err();
        assert!(matches!(err, Failure::NotFound(_)));
    }
}
