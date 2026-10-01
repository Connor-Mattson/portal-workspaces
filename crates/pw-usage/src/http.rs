//! The only way this crate talks to the network.
//!
//! Requests can only go to an [`Endpoint`], and every endpoint is a read-only usage or quota
//! query that runs no model. There is deliberately no way to pass a free-form URL: adding a host
//! means adding a variant here, next to the rule it has to follow.

use std::fmt;
use std::io::Read;
use std::time::Duration;

/// Every URL the monitor may call. **None of them may run a model or spend tokens.**
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endpoint {
    /// Claude subscription usage (5-hour and weekly utilization). Read-only.
    ClaudeUsage,
    /// Codex's ChatGPT plan usage (5-hour and weekly windows), what its `/status` shows. Read-only.
    CodexUsage,
    /// Antigravity's quota summary (5-hour and weekly buckets), on the host its CLI uses.
    AntigravityQuota,
    /// The same, on the production host (some accounts are served from there).
    AntigravityQuotaProd,
    /// Which Google Cloud project serves the account (metadata about the sign-in, no model).
    AntigravityLoadCodeAssist,
}

impl Endpoint {
    fn url(self) -> &'static str {
        match self {
            Endpoint::ClaudeUsage => "https://api.anthropic.com/api/oauth/usage",
            Endpoint::CodexUsage => "https://chatgpt.com/backend-api/wham/usage",
            Endpoint::AntigravityQuota => {
                "https://daily-cloudcode-pa.googleapis.com/v1internal:retrieveUserQuotaSummary"
            }
            Endpoint::AntigravityQuotaProd => "https://cloudcode-pa.googleapis.com/v1internal:retrieveUserQuotaSummary",
            Endpoint::AntigravityLoadCodeAssist => {
                "https://daily-cloudcode-pa.googleapis.com/v1internal:loadCodeAssist"
            }
        }
    }
}

/// An access token. Never printed.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(token: impl Into<String>) -> Self {
        Self(token.into())
    }

    fn bearer(&self) -> String {
        format!("Bearer {}", self.0)
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

#[derive(Debug)]
pub struct Response {
    pub status: u16,
    pub body: String,
    pub retry_after: Option<Duration>,
}

/// A small blocking client: one connection pool, short timeouts. Used from the monitor thread.
#[derive(Clone)]
pub struct Http {
    agent: ureq::Agent,
}

impl Default for Http {
    fn default() -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(15)))
            .timeout_connect(Some(Duration::from_secs(8)))
            .http_status_as_error(false)
            .user_agent(concat!("portal-workspaces/", env!("CARGO_PKG_VERSION")))
            .build();
        Self { agent: config.into() }
    }
}

impl Http {
    pub fn get(&self, endpoint: Endpoint, token: &Secret, headers: &[(&str, &str)]) -> Result<Response, String> {
        let mut request = self.agent.get(endpoint.url()).header("Authorization", token.bearer());
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        finish(request.call())
    }

    pub fn post_json(
        &self,
        endpoint: Endpoint,
        token: &Secret,
        headers: &[(&str, &str)],
        body: &str,
    ) -> Result<Response, String> {
        let mut request =
            self.agent.post(endpoint.url()).header("Authorization", token.bearer()).content_type("application/json");
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        finish(request.send(body))
    }
}

/// Bodies we read are small JSON documents; anything bigger is not what we asked for.
const MAX_BODY: u64 = 1 << 20;

fn finish(result: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<Response, String> {
    let mut response = result.map_err(|err| match err {
        ureq::Error::Timeout(_) => "The request timed out.".to_owned(),
        ureq::Error::HostNotFound | ureq::Error::ConnectionFailed | ureq::Error::Io(_) => {
            "Couldn't reach the server (offline?).".to_owned()
        }
        other => format!("Request failed: {other}"),
    })?;
    let status = response.status().as_u16();
    let retry_after = response
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(Duration::from_secs);
    let mut body = String::new();
    response
        .body_mut()
        .as_reader()
        .take(MAX_BODY)
        .read_to_string(&mut body)
        .map_err(|err| format!("Couldn't read the response: {err}"))?;
    Ok(Response { status, body, retry_after })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_are_not_printed() {
        let s = Secret::new("sk-ant-oat01-very-secret");
        assert!(!format!("{s:?}").contains("very-secret"));
    }

    #[test]
    fn endpoints_are_quota_queries_only() {
        for ep in [
            Endpoint::ClaudeUsage,
            Endpoint::CodexUsage,
            Endpoint::AntigravityQuota,
            Endpoint::AntigravityQuotaProd,
            Endpoint::AntigravityLoadCodeAssist,
        ] {
            let url = ep.url();
            let path = url.strip_prefix("https://").and_then(|rest| rest.split_once('/')).unwrap().1.to_lowercase();
            // Model calls on these hosts look like /v1/messages, :generateContent, /codex/responses,
            // /backend-api/conversation…
            for forbidden in ["messages", "generate", "complet", "predict", "chat", "responses", "conversation"] {
                assert!(!path.contains(forbidden), "{url} looks like a model call");
            }
        }
    }
}
