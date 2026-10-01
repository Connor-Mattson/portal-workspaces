# 0005: AI usage limits are polled in-process, without ever spending tokens

- **Status:** accepted
- **Date:** 2026-09-30
- **Code:** `crates/pw-usage/`, `crates/pw-app/src/usage.rs`, `crates/pw-app/src/ui/usage.rs`,
  `crates/pw-model/src/usage.rs`

## Context

The app exists to run CLI agents, and those agents run into their subscriptions' **5-hour** and **weekly**
limits. You may have several accounts per provider (e.g. two Claude accounts via `CLAUDE_CONFIG_DIR` aliases),
and all of them should be visible at a glance in the drawer. None of the providers offers a documented
"usage" API. Checking must never cost what it measures.

## Decision

- **A daemon thread in the app** (`pw_usage::Monitor`) polls every tracked profile every **2 minutes**. It
  backs off after failures (2 → 4 → 8 → 15 min) and honors `Retry-After`. It runs no UI timer: readings
  arrive through an inbox subscription, like terminal events. Readings are not persisted, so the state file
  isn't rewritten every 2 minutes.
- **Profiles** (`UsageProfile`) are persisted in `state.json` (schema 2). The optional *instructions* are
  whatever the user already has: an alias line, `VAR=…` assignments or a folder. `PollEnv::parse` turns them
  into the CLI's data directory plus environment. Nothing in it is executed.
- **Sources, all read-only:**
  - **Claude:** `GET api.anthropic.com/api/oauth/usage`, the endpoint behind Claude Code's `/usage`, with the
    account's OAuth token from `.credentials.json`, or on macOS the Keychain item
    `Claude Code-credentials[-<sha256(dir)[..8]>]`.
  - **Codex:** `GET chatgpt.com/backend-api/wham/usage`, the endpoint behind Codex's `/status`, with the
    ChatGPT sign-in from `$CODEX_HOME/auth.json` (bearer token plus `ChatGPT-Account-Id`). The numbers are
    account-wide, so turns run on other machines count. Without a ChatGPT sign-in (an API key, or
    credentials in the OS keyring), the newest `rate_limits` record in
    `$CODEX_HOME/sessions/**/rollout-*.jsonl`, written after every turn, is used instead.
  - **Antigravity:**
    - Reads agy's keyring item (service `gemini`, user `antigravity`).
    - Calls `loadCodeAssist` to find the account's Cloud project.
    - Calls `retrieveUserQuotaSummary`, which returns the `gemini-5h`, `gemini-weekly`, `3p-5h` and
      `3p-weekly` buckets.
    - Cloud Code picks the product from the User-Agent, so it must contain "antigravity". Ours says
      `antigravity-quota (portal-workspaces/x.y)`.
- **No tokens, enforced by construction:**
  - Every URL is a variant of `http::Endpoint`, and there are no free-form URLs.
  - Every subprocess is a variant of `vendor::VendorCmd`: `claude doctor`, `agy models` and
    `codex app-server`. Each is an authenticated metadata call that runs no model. `codex app-server` is fed
    a fixed script on stdin (`initialize`, then `account/read` with `refreshToken`), and the test pins its
    methods to exactly those.
  - Tests check that neither list grows a model call. Arbitrary user commands are deliberately not supported.
- **Expired sign-ins are renewed by the vendor's own CLI, never by us.**
  - Claude and ChatGPT refresh tokens rotate. If we refreshed at the same time as a running Claude Code or
    Codex, one side would hold a dead token, and that session would be signed out.
  - Running the CLI's token-free command lets it renew under its own locking. This is throttled to once per
    15 min per profile.
  - For Claude that command is `claude doctor`, which fetches remote settings and so renews the token.
    `claude auth status` looked like the natural choice, but it only reads the stored sign-in and never
    renews it (checked against Claude Code 2.1.286). Don't switch back to it.
  - If the token is still expired after that, the profile shows **idle**. Its usage can't change while nobody
    uses the account.
  - An idle card has a **Renew** button. It runs the same command right away (skipping the 15-minute
    throttle) and polls again. Only if that fails does the card ask you to run the CLI yourself.
  - Credentials are only ever read.

## Consequences

- The Claude and Antigravity endpoints are undocumented. Parsing is defensive: any `five_hour*` or
  `seven_day*` object, and any `bucketId`, becomes a window, so new model scopes appear without code changes.
  A shape change shows as "retrying", not as a crash.
- Codex numbers from the logs fallback are only as fresh as this machine's last Codex turn (the card says
  "as of …"). A window whose reset time has passed is shown as empty.
- A window's pace tick (time elapsed in the window) lets you see at a glance whether you're burning faster
  than the window allows.
- Adding a provider means:
  - a module in `pw-usage`
  - a `Provider` variant (`pw-model`)
  - endpoint and command variants that obey the no-token rule.
