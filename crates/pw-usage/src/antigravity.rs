//! Antigravity (`agy`): the quota summary its CLI shows, read with the CLI's own Google sign-in.
//!
//! The sign-in lives in the OS keyring (Secret Service on Linux, the Keychain on macOS). It is read,
//! never written. When it has expired, `agy models` (a metadata call, no model) lets the CLI renew
//! it.

use std::time::{Duration, SystemTime};

use pw_model::PollEnv;
use serde_json::Value;

use crate::http::{Endpoint, Http, Response, Secret};
use crate::report::{Failure, Report, Span, Window, from_unix, parse_time};
use crate::vendor::{self, VendorCmd};
use crate::{ProviderState, display_path};

struct Token {
    secret: Secret,
    expires_at: Option<SystemTime>,
}

impl Token {
    fn expired(&self, now: SystemTime) -> bool {
        self.expires_at.is_some_and(|t| t <= now + Duration::from_secs(60))
    }
}

pub fn poll(env: &PollEnv, state: &mut ProviderState, http: &Http) -> Result<Report, Failure> {
    let cli = env.dir.join("antigravity-cli");
    if !cli.is_dir() {
        return Err(Failure::NotFound(format!("No Antigravity CLI data in {}. Run agy once.", display_path(&env.dir))));
    }
    let mut token = load_token()?;
    if token.expired(SystemTime::now()) {
        if state.may_renew() {
            renew(env);
            token = load_token()?;
        }
        if token.expired(SystemTime::now()) {
            return Err(Failure::Idle { hint: "agy".into() });
        }
    }

    if state.project.is_none() {
        state.project = Some(load_project(http, &token)?);
    }
    let body = serde_json::json!({ "project": state.project }).to_string();
    let mut response = fetch(http, &token, &body)?;
    if matches!(response.status, 401 | 403) && state.may_renew() {
        renew(env);
        token = load_token()?;
        response = fetch(http, &token, &body)?;
    }
    if !matches!(response.status, 200..=299) {
        tracing::debug!(status = response.status, body = %response.body.chars().take(400).collect::<String>(), "antigravity quota");
    }
    match response.status {
        200..=299 => parse_quota(&response.body, SystemTime::now()),
        401 | 403 => {
            Err(Failure::SignedOut("Google didn't accept the Antigravity sign-in. Run agy to sign in.".into()))
        }
        429 => Err(Failure::RateLimited { retry_after: response.retry_after.unwrap_or(Duration::from_secs(300)) }),
        status => Err(Failure::Unavailable(format!("Antigravity's quota service answered HTTP {status}."))),
    }
}

/// Cloud Code decides which product a request is for from its User-Agent ("antigravity…") and
/// client metadata; without them it treats the call as a retired client and hides the project.
/// The agent string still says who is asking.
const HEADERS: &[(&str, &str)] = &[
    ("User-Agent", concat!("antigravity-quota (portal-workspaces/", env!("CARGO_PKG_VERSION"), ")")),
    ("Client-Metadata", r#"{"ideType":"ANTIGRAVITY","pluginType":"GEMINI"}"#),
];

/// The Cloud project behind the account: consumer sign-ins get a managed one, which
/// `loadCodeAssist` reports (the CLI asks the same at startup).
fn load_project(http: &Http, token: &Token) -> Result<String, Failure> {
    let body = r#"{"metadata":{"ideType":"ANTIGRAVITY","pluginType":"GEMINI"}}"#;
    let response = http
        .post_json(Endpoint::AntigravityLoadCodeAssist, &token.secret, HEADERS, body)
        .map_err(Failure::Unavailable)?;
    match response.status {
        200..=299 => {}
        401 | 403 => {
            return Err(Failure::SignedOut("Google didn't accept the Antigravity sign-in. Run agy to sign in.".into()));
        }
        status => return Err(Failure::Unavailable(format!("Antigravity's account service answered HTTP {status}."))),
    }
    let doc: Value = serde_json::from_str(&response.body).unwrap_or_default();
    let project = doc.get("cloudaicompanionProject");
    project
        .and_then(Value::as_str)
        .or_else(|| project.and_then(|p| p.get("id")).and_then(Value::as_str))
        .map(str::to_owned)
        .ok_or_else(|| Failure::NotFound("Antigravity hasn't set up this account yet. Run agy once.".into()))
}

/// Asks the host the CLI uses, then production (accounts are served from one or the other).
fn fetch(http: &Http, token: &Token, body: &str) -> Result<Response, Failure> {
    let first =
        http.post_json(Endpoint::AntigravityQuota, &token.secret, HEADERS, body).map_err(Failure::Unavailable)?;
    if matches!(first.status, 200..=299 | 401 | 429) {
        return Ok(first);
    }
    let second =
        http.post_json(Endpoint::AntigravityQuotaProd, &token.secret, HEADERS, body).map_err(Failure::Unavailable)?;
    Ok(if matches!(second.status, 200..=299) { second } else { first })
}

fn renew(env: &PollEnv) {
    if let Err(err) = vendor::run(VendorCmd::AgyModels, env) {
        tracing::info!(%err, "agy couldn't renew its sign-in");
    }
}

fn load_token() -> Result<Token, Failure> {
    let raw = keyring_item()?;
    raw.as_deref()
        .and_then(parse_token)
        .ok_or_else(|| Failure::NotFound("No Antigravity sign-in in the keyring. Run agy and sign in.".into()))
}

/// Where agy keeps its sign-in: go-keyring's item for service `gemini`, user `antigravity`.
const KEYRING_SERVICE: &str = "gemini";
const KEYRING_USER: &str = "antigravity";

#[cfg(target_os = "linux")]
fn keyring_item() -> Result<Option<String>, Failure> {
    crate::secrets::secret_service(&[("service", KEYRING_SERVICE), ("username", KEYRING_USER)])
        .map_err(Failure::Unavailable)
}

#[cfg(target_os = "macos")]
fn keyring_item() -> Result<Option<String>, Failure> {
    crate::secrets::keychain(KEYRING_SERVICE, Some(KEYRING_USER)).map_err(Failure::Unavailable)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn keyring_item() -> Result<Option<String>, Failure> {
    Ok(None)
}

/// The stored token: an OAuth token as JSON (Go's `oauth2.Token` or similar), possibly
/// base64-wrapped by the keyring library, or a bare access token.
fn parse_token(raw: &str) -> Option<Token> {
    let raw = raw.trim();
    let decoded;
    let text = match raw.strip_prefix("go-keyring-base64:").or_else(|| raw.strip_prefix("go-keyring-encoded:")) {
        Some(b64) => {
            use base64::Engine;
            decoded = String::from_utf8(base64::engine::general_purpose::STANDARD.decode(b64).ok()?).ok()?;
            decoded.as_str()
        }
        None => raw,
    };
    let Ok(doc) = serde_json::from_str::<Value>(text) else {
        return text.starts_with("ya29.").then(|| Token { secret: Secret::new(text), expires_at: None });
    };
    let access = ["access_token", "accessToken"].iter().find_map(|k| find_str(&doc, k))?;
    let expires_at = ["expiry", "expires_at", "expiresAt", "expiry_date"].iter().find_map(|k| {
        let v = find_value(&doc, k)?;
        v.as_str().and_then(parse_time).or_else(|| {
            // Milliseconds or seconds since the epoch.
            v.as_f64().and_then(|n| from_unix(if n > 1e11 { n / 1000.0 } else { n }))
        })
    });
    Some(Token { secret: Secret::new(access), expires_at })
}

fn find_value<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    match value {
        Value::Object(map) => map.get(key).or_else(|| map.values().find_map(|v| find_value(v, key))),
        Value::Array(items) => items.iter().find_map(|v| find_value(v, key)),
        _ => None,
    }
}

fn find_str<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    find_value(value, key)?.as_str().filter(|s| !s.is_empty())
}

/// Every object with a `bucketId` is a window: `gemini-5h`, `gemini-weekly`, `3p-5h`, `3p-weekly`, …
pub(crate) fn parse_quota(body: &str, now: SystemTime) -> Result<Report, Failure> {
    let doc: Value = serde_json::from_str(body)
        .map_err(|_| Failure::Unavailable("Antigravity's quota service sent something unexpected.".into()))?;
    let mut buckets = Vec::new();
    collect_buckets(&doc, None, &mut buckets);
    let windows: Vec<Window> = buckets.into_iter().filter_map(|(bucket, group)| bucket_window(bucket, group)).collect();
    if windows.is_empty() {
        return Err(Failure::Unavailable("Antigravity's quota response had no limits in it.".into()));
    }
    Ok(Report::new(windows, now, true))
}

fn collect_buckets<'a>(value: &'a Value, group: Option<&'a str>, out: &mut Vec<(&'a Value, Option<&'a str>)>) {
    match value {
        Value::Object(map) if map.contains_key("bucketId") => out.push((value, group)),
        Value::Object(map) => {
            let group = map.get("displayName").and_then(Value::as_str).or(group);
            for v in map.values() {
                collect_buckets(v, group, out);
            }
        }
        Value::Array(items) => items.iter().for_each(|v| collect_buckets(v, group, out)),
        _ => {}
    }
}

fn bucket_window(bucket: &Value, group: Option<&str>) -> Option<Window> {
    let id = bucket.get("bucketId")?.as_str()?;
    let (family, period) = id.rsplit_once('-')?;
    let span = match period {
        "5h" => Span::FiveHour,
        "weekly" | "7d" => Span::Weekly,
        "daily" | "1d" => Span::Other(1440),
        _ => return None,
    };
    // Proto JSON leaves out zero values, so a bucket without a fraction has none left.
    let remaining = bucket
        .pointer("/remaining/remainingFraction")
        .or_else(|| bucket.get("remainingFraction"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    // A full bucket means no window is open yet; its reset time is just "now + window".
    let resets_at =
        if remaining >= 1.0 { None } else { bucket.get("resetTime").and_then(Value::as_str).and_then(parse_time) };
    let scope = match family {
        "gemini" => "Gemini".to_owned(),
        "3p" => "Claude & GPT".to_owned(),
        other => group.map_or_else(|| other.to_owned(), str::to_owned),
    };
    Some(Window::new(span, Some(scope), ((1.0 - remaining) * 100.0) as f32, resets_at))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../fixtures/antigravity_quota.json");

    #[test]
    fn parses_buckets() {
        let report = parse_quota(FIXTURE, SystemTime::UNIX_EPOCH).unwrap();
        let find =
            |span, scope: &str| report.windows.iter().find(|w| w.span == span && w.scope.as_deref() == Some(scope));
        let gemini_5h = find(Span::FiveHour, "Gemini").unwrap();
        assert!((gemini_5h.used - 2.49).abs() < 0.01);
        assert!(gemini_5h.resets_at.is_some());
        // Untouched bucket: 0% and no reset time.
        let partner_5h = find(Span::FiveHour, "Claude & GPT").unwrap();
        assert_eq!(partner_5h.used, 0.0);
        assert_eq!(partner_5h.resets_at, None);
        // A bucket without a fraction is exhausted.
        assert_eq!(find(Span::Weekly, "Claude & GPT").unwrap().used, 100.0);
        assert_eq!(report.windows.len(), 4);
    }

    #[test]
    fn reads_tokens_in_every_shape() {
        let go = r#"{"access_token":"ya29.a","token_type":"Bearer","refresh_token":"1//r","expiry":"2026-10-01T10:00:00.5+02:00"}"#;
        let t = parse_token(go).unwrap();
        assert_eq!(t.expires_at, parse_time("2026-10-01T08:00:00.5Z"));

        use base64::Engine;
        let wrapped = format!("go-keyring-base64:{}", base64::engine::general_purpose::STANDARD.encode(go));
        assert!(parse_token(&wrapped).is_some());

        let nested = r#"{"token":{"accessToken":"ya29.b","expiresAt":1790000000000}}"#;
        assert_eq!(parse_token(nested).unwrap().expires_at, from_unix(1_790_000_000.0));

        assert!(parse_token("ya29.bare").unwrap().expires_at.is_none());
        assert!(parse_token("hunter2").is_none());
    }
}
