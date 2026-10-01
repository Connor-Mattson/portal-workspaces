//! Polls profiles once and prints the windows (never tokens):
//! `cargo run -p pw-usage --example poll -- claude "CLAUDE_CONFIG_DIR=~/.claude-work"`

use std::time::SystemTime;

use pw_model::{Provider, UsageProfile};
use pw_usage::{Poller, ProviderState, Providers};

fn main() {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).init();
    let mut args = std::env::args().skip(1);
    let provider = match args.next().as_deref() {
        Some("claude") => Provider::Claude,
        Some("codex") => Provider::Codex,
        Some("agy" | "antigravity") => Provider::Antigravity,
        _ => return eprintln!("usage: poll claude|codex|agy [instructions]"),
    };
    let profile = UsageProfile::new("cli", provider, args.next().unwrap_or_default());
    let home = std::env::var_os("HOME").map(Into::into).unwrap_or_default();
    match Providers::new(home).poll(&profile, &mut ProviderState::default()) {
        Ok(report) => {
            let now = SystemTime::now();
            println!("account={:?} plan={:?} live={}", report.account.is_some(), report.plan, report.live);
            for w in &report.windows {
                println!(
                    "  {:<24} {:>5.1}%  resets in {:?}",
                    w.title(),
                    w.used_at(now),
                    w.resets_in(now).map(|d| d.as_secs() / 60)
                );
            }
        }
        Err(err) => println!("failed: {err}"),
    }
}
