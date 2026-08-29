//! Shared reqwest plumbing for the two wire adapters: a rustls client, status →
//! [`LlmError`] mapping, and `Retry-After` parsing. Compiled only when an
//! adapter feature is on.

use std::time::Duration;

use reqwest::StatusCode;
use reqwest::header::HeaderMap;

use crate::provider::LlmError;

/// A shared rustls-backed HTTP client. Per-request timeouts are applied at the
/// call site, so none is set here.
///
/// Redirects are **disabled**: this client carries API keys in headers, and
/// reqwest does not strip a custom `x-api-key` (Anthropic) on a cross-host
/// redirect the way it strips `Authorization`. Refusing to follow any 3xx keeps
/// a secret-bearing request from being replayed to another host.
pub fn client() -> Result<reqwest::Client, LlmError> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| LlmError::Network(scrub(&e.to_string())))
}

/// Map a non-success HTTP status + response body to an [`LlmError`]. The body
/// is scrubbed and truncated before it ever reaches an error string.
pub fn status_error(status: StatusCode, headers: &HeaderMap, body: &str) -> LlmError {
    let code = status.as_u16();
    match code {
        401 | 403 => LlmError::Auth,
        429 => LlmError::RateLimited {
            retry_after: retry_after(headers),
        },
        503 | 529 => LlmError::Overloaded,
        500 | 502 | 504 => LlmError::Network(format!("http {code}: {}", snippet(body))),
        400 | 422 => LlmError::InvalidRequest(snippet(body)),
        _ if status.is_server_error() => LlmError::Overloaded,
        _ => LlmError::InvalidRequest(format!("http {code}: {}", snippet(body))),
    }
}

/// Map a reqwest transport error (send/body) to an [`LlmError`].
pub fn transport_error(e: &reqwest::Error) -> LlmError {
    if e.is_timeout() {
        LlmError::Timeout
    } else {
        LlmError::Network(scrub(&e.to_string()))
    }
}

/// Parse a `Retry-After` header (integer seconds only; HTTP-date form is
/// ignored and left to the backoff schedule).
pub fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    let v = headers.get(reqwest::header::RETRY_AFTER)?;
    let s = v.to_str().ok()?.trim();
    s.parse::<u64>().ok().map(Duration::from_secs)
}

/// A ≤400-char, scrubbed snippet of a response body for error messages.
fn snippet(body: &str) -> String {
    let scrubbed = scrub(body);
    let mut out = String::new();
    for (i, c) in scrubbed.chars().enumerate() {
        if i >= 400 {
            out.push('…');
            break;
        }
        out.push(c);
    }
    out
}

/// Belt-and-braces scrub: drop anything that looks like a bearer token or key
/// from a string bound for logs/errors. Keys should never reach here (they live
/// only in headers), but a provider echoing one in an error body must not leak.
///
/// Tokenizes on key-character boundaries (anything outside `[A-Za-z0-9_-]`)
/// rather than whitespace, so a key wrapped in compact JSON punctuation —
/// `{"key":"sk-…"}` — is isolated as its own token and redacted, not hidden
/// inside a `{`-prefixed word.
fn scrub(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut token = String::new();
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            token.push(ch);
        } else {
            flush_token(&mut token, &mut out);
            out.push(ch);
        }
    }
    flush_token(&mut token, &mut out);
    out
}

/// Emit the accumulated token, redacted if it looks like a secret.
fn flush_token(token: &mut String, out: &mut String) {
    if token.is_empty() {
        return;
    }
    if looks_secret(token) {
        out.push_str("[redacted]");
    } else {
        out.push_str(token);
    }
    token.clear();
}

fn looks_secret(w: &str) -> bool {
    w.starts_with("sk-")
        || w.starts_with("sk_")
        || w.starts_with("Bearer")
        || (w.len() > 40
            && w.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrub_redacts_key_in_compact_json() {
        // No whitespace around the key (a compact-JSON error body).
        let body = r#"{"error":{"message":"bad key sk-abcdEFGH1234567890ijkl"}}"#;
        let out = scrub(body);
        assert!(
            !out.contains("sk-abcdEFGH1234567890ijkl"),
            "key leaked: {out}"
        );
        assert!(out.contains("[redacted]"), "no redaction marker: {out}");
        // Structure around the token is preserved, not swallowed.
        assert!(out.contains("\"error\""), "punctuation lost: {out}");
    }

    #[test]
    fn scrub_redacts_whitespace_delimited_key_and_bearer() {
        let out = scrub("Authorization: Bearer sk-XXXXXXXXXXXXXXXXXXXXXXXX failed");
        assert!(!out.contains("sk-XXXXXXXXXXXXXXXXXXXXXXXX"), "{out}");
        assert!(out.contains("failed"), "non-secret words survive: {out}");
    }

    #[test]
    fn scrub_leaves_ordinary_text_alone() {
        let s = "invalid request: model z-ai/glm-5.3 not found";
        assert_eq!(scrub(s), s);
    }
}
