//! Codex: the account's limits from the usage endpoint Codex's `/status` reads, with the CLI's own
//! ChatGPT sign-in. The numbers are account-wide, so turns run on other machines count too.
//!
//! The sign-in in `$CODEX_HOME/auth.json` is read, never written. When it has expired, Codex's own
//! app server renews it (`account/read`, no model). Without a ChatGPT sign-in here (an API key, or
//! credentials kept in the OS keyring), the limits come from the CLI's session logs instead: each
//! turn's `token_count` event in `sessions/**/rollout-*.jsonl` carries the account's `rate_limits`
//! as of that turn. Those windows can't fill up without new turns on this machine, and
//! [`crate::Window::used_at`] empties a window once its reset time passes.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use pw_model::PollEnv;
use serde_json::Value;

use crate::http::{Endpoint, Http, Response, Secret};
use crate::report::{Failure, Report, Span, Window, from_unix, parse_time};
use crate::vendor::{self, VendorCmd};
use crate::{ProviderState, display_path, title_case};

/// How much of a log's end to read. A turn's events are a few KB; this covers many turns.
const TAIL: u64 = 256 * 1024;
/// How many of the newest logs to look through before giving up.
const FILES: usize = 6;

pub fn poll(env: &PollEnv, state: &mut ProviderState, http: &Http) -> Result<Report, Failure> {
    if !env.dir.is_dir() {
        return Err(Failure::NotFound(format!("{} doesn't exist.", display_path(&env.dir))));
    }
    match live(env, state, http) {
        Ok(report) => Ok(report),
        // No usable sign-in: what this machine's CLI last recorded is the next best thing.
        Err(failure @ (Failure::NotFound(_) | Failure::Idle { .. } | Failure::SignedOut(_))) => {
            from_logs(env).ok_or(failure)
        }
        // The service is unreachable or busy: keep showing the last live reading.
        Err(failure) => Err(failure),
    }
}

fn live(env: &PollEnv, state: &mut ProviderState, http: &Http) -> Result<Report, Failure> {
    let mut auth = load_auth(env)?;
    if auth.expired(SystemTime::now()) {
        if state.may_renew() {
            renew(env);
            auth = load_auth(env)?;
        }
        if auth.expired(SystemTime::now()) {
            return Err(Failure::Idle { hint: env.command_hint() });
        }
    }

    let mut response = fetch(http, &auth)?;
    if matches!(response.status, 401 | 403) && state.may_renew() {
        renew(env);
        auth = load_auth(env)?;
        response = fetch(http, &auth)?;
    }
    match response.status {
        200..=299 => {
            let mut report = parse_usage(&response.body, SystemTime::now())?;
            report.account = auth.email;
            report.plan = report.plan.or(auth.plan);
            Ok(report)
        }
        401 | 403 => Err(Failure::SignedOut(format!(
            "ChatGPT didn't accept Codex's sign-in. Run {} and sign in again.",
            env.command_hint()
        ))),
        429 => Err(Failure::RateLimited { retry_after: response.retry_after.unwrap_or(Duration::from_secs(300)) }),
        status => Err(Failure::Unavailable(format!("Codex's usage service answered HTTP {status}."))),
    }
}

fn fetch(http: &Http, auth: &Auth) -> Result<Response, Failure> {
    let mut headers = vec![("Accept", "application/json")];
    if let Some(id) = &auth.account_id {
        headers.push(("ChatGPT-Account-Id", id));
    }
    http.get(Endpoint::CodexUsage, &auth.token, &headers).map_err(Failure::Unavailable)
}

fn renew(env: &PollEnv) {
    if let Err(err) = vendor::run(VendorCmd::CodexAccountRead, env) {
        tracing::info!(%err, dir = %env.dir.display(), "codex couldn't renew its sign-in");
    }
}

/// Codex's ChatGPT sign-in.
pub(crate) struct Auth {
    token: Secret,
    account_id: Option<String>,
    expires_at: Option<SystemTime>,
    pub email: Option<String>,
    pub plan: Option<String>,
}

impl Auth {
    fn expired(&self, now: SystemTime) -> bool {
        self.expires_at.is_some_and(|t| t <= now + Duration::from_secs(60))
    }
}

fn load_auth(env: &PollEnv) -> Result<Auth, Failure> {
    fs::read_to_string(env.dir.join("auth.json")).ok().as_deref().and_then(parse_auth).ok_or_else(|| {
        Failure::NotFound(format!(
            "No ChatGPT sign-in for Codex in {}. Run {} and sign in with ChatGPT.",
            display_path(&env.dir),
            env.command_hint()
        ))
    })
}

/// `auth.json` in ChatGPT mode: the access token (a JWT, whose `exp` says when it expires), the
/// account id, and an id token naming the account and plan. API-key sign-ins have no tokens.
pub(crate) fn parse_auth(text: &str) -> Option<Auth> {
    let doc: Value = serde_json::from_str(text).ok()?;
    let tokens = doc.get("tokens")?;
    let token = tokens.get("access_token")?.as_str().filter(|t| !t.is_empty())?;
    let access = jwt_claims(token);
    let id = tokens.get("id_token").and_then(Value::as_str).and_then(jwt_claims);
    let account_id = tokens
        .get("account_id")
        .and_then(Value::as_str)
        .or_else(|| access.as_ref()?.pointer("/https:~1~1api.openai.com~1auth/chatgpt_account_id")?.as_str())
        .map(str::to_owned);
    let expires_at = access.as_ref().and_then(|c| c.get("exp")).and_then(Value::as_f64).and_then(from_unix);
    let email = id
        .as_ref()
        .and_then(|c| c.get("email").or_else(|| c.pointer("/https:~1~1api.openai.com~1profile/email")))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let plan = id
        .as_ref()
        .and_then(|c| c.pointer("/https:~1~1api.openai.com~1auth/chatgpt_plan_type"))
        .and_then(Value::as_str)
        .and_then(plan_name);
    Some(Auth { token: Secret::new(token), account_id, expires_at, email, plan })
}

/// The claims of a JWT (its middle part). The signature isn't checked: the server does that.
fn jwt_claims(token: &str) -> Option<Value> {
    use base64::Engine;
    let payload = token.split('.').nth(1)?.trim_end_matches('=');
    serde_json::from_slice(&base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload).ok()?).ok()
}

fn plan_name(plan: &str) -> Option<String> {
    (!plan.is_empty() && !plan.eq_ignore_ascii_case("unknown")).then(|| title_case(&plan.replace('_', " ")))
}

/// The usage endpoint's answer: the account's `rate_limit` (primary = 5 hours, secondary = weekly)
/// plus any separately metered limits, each scoped by name.
pub(crate) fn parse_usage(body: &str, now: SystemTime) -> Result<Report, Failure> {
    let unreadable = || Failure::Unavailable("Couldn't read Codex's usage answer.".into());
    let doc: Value = serde_json::from_str(body).map_err(|_| unreadable())?;
    let mut windows = Vec::new();
    push_limit(&mut windows, doc.get("rate_limit"), None, now);
    push_limit(&mut windows, doc.get("code_review_rate_limit"), Some("Code review"), now);
    for extra in doc.get("additional_rate_limits").and_then(Value::as_array).into_iter().flatten() {
        let name = ["limit_name", "metered_feature"].iter().find_map(|k| extra.get(*k).and_then(Value::as_str));
        push_limit(&mut windows, extra.get("rate_limit"), Some(name.unwrap_or("Other")), now);
    }
    if windows.is_empty() {
        return Err(unreadable());
    }
    let mut report = Report::new(windows, now, true);
    report.plan = doc.get("plan_type").and_then(Value::as_str).and_then(plan_name);
    Ok(report)
}

fn push_limit(out: &mut Vec<Window>, limit: Option<&Value>, scope: Option<&str>, now: SystemTime) {
    let Some(limit) = limit.filter(|l| l.is_object()) else { return };
    for slot in ["primary_window", "secondary_window"] {
        if let Some(window) = limit.get(slot).and_then(|w| live_window(w, scope.map(str::to_owned), now)) {
            out.push(window);
        }
    }
}

fn live_window(w: &Value, scope: Option<String>, now: SystemTime) -> Option<Window> {
    let used = w.get("used_percent").and_then(Value::as_f64)?;
    let length = w.get("limit_window_seconds").and_then(Value::as_u64)?;
    let span = Span::from_minutes(u32::try_from(length / 60).ok()?);
    let after = w.get("reset_after_seconds").and_then(Value::as_f64);
    let resets_at = w
        .get("reset_at")
        .and_then(Value::as_f64)
        .and_then(from_unix)
        .or_else(|| after.map(|s| now + Duration::from_secs_f64(s.max(0.0))));
    // An unused window that would reset a whole window from now hasn't started yet.
    let unstarted = used <= 0.0 && after.is_some_and(|s| s + 60.0 >= length as f64);
    Some(Window::new(span, scope, used as f32, if unstarted { None } else { resets_at }))
}

fn from_logs(env: &PollEnv) -> Option<Report> {
    newest_logs(&env.dir.join("sessions"), FILES).iter().find_map(|log| read_log(log))
}

/// The newest `rollout-*.jsonl` files under `sessions/YYYY/MM/DD/`, newest first. Directory and
/// file names sort by time, so this never walks the whole history.
fn newest_logs(sessions: &Path, limit: usize) -> Vec<PathBuf> {
    fn sorted_desc(dir: &Path) -> Vec<PathBuf> {
        let mut entries: Vec<PathBuf> =
            fs::read_dir(dir).map_or_else(|_| Vec::new(), |rd| rd.filter_map(Result::ok).map(|e| e.path()).collect());
        entries.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
        entries
    }
    fn walk(dir: &Path, depth: usize, limit: usize, out: &mut Vec<PathBuf>) {
        for entry in sorted_desc(dir) {
            if out.len() >= limit {
                return;
            }
            if entry.is_dir() && depth < 3 {
                walk(&entry, depth + 1, limit, out);
            } else if entry
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("rollout-") && n.ends_with(".jsonl"))
            {
                out.push(entry);
            }
        }
    }
    let mut out = Vec::new();
    walk(sessions, 0, limit, &mut out);
    out
}

fn read_log(path: &Path) -> Option<Report> {
    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(len.saturating_sub(TAIL))).ok()?;
    let mut bytes = Vec::new();
    file.take(TAIL).read_to_end(&mut bytes).ok()?;
    let modified = fs::metadata(path).and_then(|m| m.modified()).unwrap_or_else(|_| SystemTime::now());
    parse_tail(&String::from_utf8_lossy(&bytes), modified)
}

/// The newest record of each limit (Codex may track more than one), from the end of a log.
pub(crate) fn parse_tail(text: &str, fallback_time: SystemTime) -> Option<Report> {
    let mut latest: BTreeMap<String, (SystemTime, Value)> = BTreeMap::new();
    for line in text.lines().rev().filter(|l| l.contains("\"rate_limits\"")) {
        let Ok(event) = serde_json::from_str::<Value>(line) else { continue };
        let Some(limits) = find_key(&event, "rate_limits").filter(|v| v.is_object()) else { continue };
        let at = event.get("timestamp").and_then(Value::as_str).and_then(parse_time).unwrap_or(fallback_time);
        let id = limits.get("limit_id").and_then(Value::as_str).unwrap_or("codex").to_owned();
        latest.entry(id).or_insert((at, limits.clone()));
    }
    let as_of = latest.values().map(|(at, _)| *at).max()?;
    let mut windows = Vec::new();
    let mut plan = None;
    for (id, (at, limits)) in &latest {
        let scope = (id != "codex").then(|| limits.get("limit_name").and_then(Value::as_str).unwrap_or(id).to_owned());
        for slot in ["primary", "secondary"] {
            if let Some(window) = limits.get(slot).and_then(|w| parse_window(w, *at, scope.clone())) {
                windows.push(window);
            }
        }
        plan = plan.or_else(|| {
            limits
                .get("plan_type")
                .and_then(Value::as_str)
                .filter(|p| !p.eq_ignore_ascii_case("unknown"))
                .map(title_case)
        });
    }
    if windows.is_empty() {
        return None;
    }
    let mut report = Report::new(windows, as_of, false);
    report.plan = plan;
    Some(report)
}

fn parse_window(w: &Value, at: SystemTime, scope: Option<String>) -> Option<Window> {
    let used = w.get("used_percent").and_then(Value::as_f64)?;
    let minutes = w.get("window_minutes").and_then(Value::as_u64).unwrap_or(0);
    let span = Span::from_minutes(u32::try_from(minutes).ok()?);
    // Newer CLIs write an absolute `resets_at`; older ones `resets_in_seconds` from the event.
    let resets_at = w.get("resets_at").and_then(Value::as_f64).and_then(from_unix).or_else(|| {
        w.get("resets_in_seconds").and_then(Value::as_f64).map(|s| at + Duration::from_secs_f64(s.max(0.0)))
    });
    Some(Window::new(span, scope, used as f32, resets_at))
}

fn find_key<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    match value {
        Value::Object(map) => map.get(key).or_else(|| map.values().find_map(|v| find_key(v, key))),
        Value::Array(items) => items.iter().find_map(|v| find_key(v, key)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../fixtures/codex_rollout.jsonl");
    const USAGE: &str = include_str!("../fixtures/codex_usage.json");

    #[test]
    fn reads_live_usage() {
        let now = from_unix(1_790_821_437.0).unwrap();
        let report = parse_usage(USAGE, now).unwrap();
        assert!(report.live);
        assert_eq!(report.plan.as_deref(), Some("Education"));
        let five = report.window(Span::FiveHour).unwrap();
        assert_eq!(five.used, 12.0);
        assert_eq!(five.resets_at, from_unix(1_790_830_437.0));
        let week = report.window(Span::Weekly).unwrap();
        assert_eq!(week.used, 3.0);
        assert_eq!(week.resets_at, from_unix(1_791_310_442.0));
        // ChatGPT's own windows (`chatpass`) aren't Codex limits.
        assert_eq!(report.windows.len(), 2);
    }

    #[test]
    fn unstarted_windows_have_no_reset_and_extra_limits_are_scoped() {
        let body = r#"{
            "plan_type": "plus",
            "rate_limit": {"primary_window": {"used_percent": 0, "limit_window_seconds": 18000,
                                              "reset_after_seconds": 18000, "reset_at": 1790839437}},
            "additional_rate_limits": [{"limit_name": "GPT-5 Codex Mini", "metered_feature": "codex_mini",
                "rate_limit": {"secondary_window": {"used_percent": 40, "limit_window_seconds": 604800,
                                                    "reset_after_seconds": 1000}}}]
        }"#;
        let now = from_unix(1_790_821_437.0).unwrap();
        let report = parse_usage(body, now).unwrap();
        assert_eq!(report.window(Span::FiveHour).unwrap().resets_at, None);
        let mini = report.windows.iter().find(|w| w.scope.as_deref() == Some("GPT-5 Codex Mini")).unwrap();
        assert_eq!((mini.span, mini.used), (Span::Weekly, 40.0));
        assert_eq!(mini.resets_at, Some(now + Duration::from_secs(1000)));
        assert!(parse_usage(r#"{"rate_limit": null}"#, now).is_err());
    }

    #[test]
    fn reads_the_chatgpt_sign_in() {
        use base64::Engine;
        let jwt = |claims: &str| format!("h.{}.s", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(claims));
        let auth = serde_json::json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "access_token": jwt(r#"{"exp":1791569634,"https://api.openai.com/auth":{"chatgpt_account_id":"acct"}}"#),
                "id_token": jwt(r#"{"email":"me@example.com","https://api.openai.com/auth":{"chatgpt_plan_type":"pro"}}"#),
                "refresh_token": "r"
            }
        });
        let auth = parse_auth(&auth.to_string()).unwrap();
        assert_eq!(auth.account_id.as_deref(), Some("acct"));
        assert_eq!(auth.expires_at, from_unix(1_791_569_634.0));
        assert_eq!(auth.email.as_deref(), Some("me@example.com"));
        assert_eq!(auth.plan.as_deref(), Some("Pro"));
        assert!(!auth.expired(from_unix(1_791_000_000.0).unwrap()));
        // An API-key sign-in has no tokens: Codex falls back to its logs.
        assert!(parse_auth(r#"{"OPENAI_API_KEY":"sk-x","tokens":null}"#).is_none());
    }

    #[test]
    fn reads_the_newest_record() {
        let report = parse_tail(FIXTURE, SystemTime::UNIX_EPOCH).unwrap();
        let five = report.window(Span::FiveHour).unwrap();
        assert_eq!(five.used, 6.0);
        assert_eq!(five.resets_at, from_unix(1_790_798_025.0));
        assert_eq!(report.window(Span::Weekly).unwrap().used, 2.0);
        assert_eq!(report.as_of, parse_time("2026-09-30T19:46:10.000Z").unwrap());
        assert!(!report.live);
    }

    #[test]
    fn handles_relative_resets_and_other_limits() {
        let text = concat!(
            r#"{"timestamp":"2026-01-01T00:00:00Z","payload":{"rate_limits":{"primary":{"used_percent":10,"window_minutes":300,"resets_in_seconds":60}}}}"#,
            "\n",
            r#"{"timestamp":"2026-01-01T00:00:00Z","payload":{"rate_limits":{"limit_id":"codex_mini","limit_name":"Mini","secondary":{"used_percent":5,"window_minutes":10080,"resets_at":null}}}}"#,
        );
        let report = parse_tail(text, SystemTime::UNIX_EPOCH).unwrap();
        let five = report.window(Span::FiveHour).unwrap();
        assert_eq!(five.resets_at, Some(parse_time("2026-01-01T00:01:00Z").unwrap()));
        assert_eq!(report.windows.iter().filter(|w| w.scope.as_deref() == Some("Mini")).count(), 1);
    }

    #[test]
    fn finds_logs_newest_first() {
        let home = tempfile::tempdir().unwrap();
        let codex = home.path().join(".codex");
        for (day, name, body) in [
            ("2026/09/29", "rollout-2026-09-29T10-00-00-a.jsonl", FIXTURE),
            ("2026/09/30", "rollout-2026-09-30T09-00-00-b.jsonl", "{\"type\":\"session_meta\"}\n"),
        ] {
            let dir = codex.join("sessions").join(day);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join(name), body).unwrap();
        }
        let logs = newest_logs(&codex.join("sessions"), 6);
        assert!(logs[0].ends_with("rollout-2026-09-30T09-00-00-b.jsonl"));
        // The newest log has no limits yet (a fresh session), so the previous one answers.
        let env = PollEnv::parse(pw_model::Provider::Codex, "", home.path()).unwrap();
        // No ChatGPT sign-in in this home, so nothing is fetched.
        let report = poll(&env, &mut ProviderState::default(), &Http::default()).unwrap();
        assert_eq!(report.window(Span::Weekly).unwrap().used, 2.0);
        assert!(!report.live);
    }

    #[test]
    fn no_sessions_is_not_found() {
        let home = tempfile::tempdir().unwrap();
        let env = PollEnv::parse(pw_model::Provider::Codex, "", home.path()).unwrap();
        assert!(matches!(poll(&env, &mut ProviderState::default(), &Http::default()), Err(Failure::NotFound(_))));
    }
}
