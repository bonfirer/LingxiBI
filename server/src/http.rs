//! Shared construction of outbound HTTP clients.
//!
//! Every outbound request this server makes targets an operator-configured
//! endpoint: the LLM provider (`llm_config.base_url`), a Feishu/Lark webhook, or
//! GitHub during OAuth. Two `reqwest` defaults are wrong for that situation:
//!
//! - **No timeout at all.** A hung or deliberately slow upstream could pin a
//!   worker and a connection indefinitely. That is amplified by the bundled
//!   nginx config, which uses `proxy_read_timeout 3600s`.
//! - **Redirects are followed** (up to 10 hops). A permitted host could bounce a
//!   request to `127.0.0.1` or a cloud metadata endpoint, which would defeat any
//!   host allowlist applied to the configured URL — including the Feishu webhook
//!   allowlist.

use std::time::Duration;

/// Total budget for a short control-plane request (webhook post, OAuth exchange,
/// provider reachability check). NOT for LLM completions — see [`llm_client`].
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Connect timeout, so an unroutable or filtered host fails fast.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Total budget for a non-streaming LLM completion.
///
/// These are genuinely slow: generating a full HTML dashboard runs with
/// `max_tokens = 65536` and has been observed taking over 100 seconds, with the
/// whole response arriving at the end. Anything in the tens of seconds breaks
/// report generation outright, so this is generous — its job is only to stop a
/// wedged request from pinning a connection forever.
const LLM_TIMEOUT: Duration = Duration::from_secs(600);

/// Maximum idle gap between bytes of a *streamed* response.
///
/// Streaming completions are legitimately long-lived, so a total timeout would
/// cut off healthy generations; bounding the idle time instead still kills a
/// stalled upstream. Sized to tolerate a reasoning model thinking for a while
/// before it emits its first token.
const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(300);

/// How many redirects to follow for ordinary outbound calls.
///
/// Zero would be stricter, but some OpenAI-compatible gateways answer with a
/// 301/307 normalization redirect, and refusing those breaks working deployments
/// for no real gain — nothing constrains `base_url` to a host allowlist in the
/// first place, so a redirect grants no reach the configured URL didn't already
/// have. Kept small so it cannot become a long redirect chain.
///
/// Where an allowlist *is* the control, use [`webhook_client`] instead.
const MAX_REDIRECTS: usize = 2;

/// Outbound client for a host protected by an allowlist — currently the Feishu
/// webhook.
///
/// Redirects are refused outright: the host allowlist is the SSRF control there,
/// and following a redirect would let an allowed host bounce the request to an
/// internal address, defeating it.
pub fn webhook_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .connect_timeout(CONNECT_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// Outbound client for short control-plane requests: the GitHub OAuth exchange
/// and the LLM provider reachability check.
///
/// Redirect handling matches [`llm_client`] on purpose. When these differed, the
/// Settings "test connection" button could report a provider as unreachable while
/// real completions against the same URL worked fine.
pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .connect_timeout(CONNECT_TIMEOUT)
        .redirect(reqwest::redirect::Policy::limited(MAX_REDIRECTS))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// Outbound client for non-streaming LLM completions: a long total budget, since
/// the provider sends nothing until the whole answer is ready.
pub fn llm_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(LLM_TIMEOUT)
        .connect_timeout(CONNECT_TIMEOUT)
        .redirect(reqwest::redirect::Policy::limited(MAX_REDIRECTS))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// Outbound client for streaming (SSE) LLM responses: no total timeout, but the
/// stream must keep producing bytes.
pub fn streaming_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(STREAM_IDLE_TIMEOUT)
        .redirect(reqwest::redirect::Policy::limited(MAX_REDIRECTS))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}
