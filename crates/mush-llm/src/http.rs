//! Shared reqwest plumbing for the two wire adapters: a rustls client, status →
//! [`LlmError`] mapping, and `Retry-After` parsing. Compiled only when an
//! adapter feature is on.

use std::time::Duration;

use futures::StreamExt;
use reqwest::StatusCode;
use reqwest::header::HeaderMap;

use crate::provider::{LlmError, MAX_PROVIDER_RESPONSE_BYTES};

/// Upper bound (seconds) on a wire `Retry-After` hint accepted from a
/// provider response. [`crate::service::LlmService`] further clamps delays to
/// `retry.max_delay_ms` and the remaining call deadline.
pub const MAX_RETRY_AFTER_SECS: u64 = 60;

/// Validate a provider `base_url` before any secret-bearing request is built.
///
/// Requires `https://` for all remote hosts so API keys are never sent in
/// cleartext over the network. Cleartext `http://` is permitted only for
/// explicit loopback hosts (`localhost`, `127.0.0.0/8`, `::1`) used by tests
/// and local model runners (Ollama/vLLM). Embedded URL credentials are always
/// rejected.
pub fn validate_base_url(base_url: &str) -> Result<String, LlmError> {
    let trimmed = base_url.trim();
    let url = reqwest::Url::parse(trimmed)
        .map_err(|_| LlmError::InvalidRequest("invalid provider base_url".to_string()))?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(LlmError::InvalidRequest(
            "provider base_url must not contain embedded credentials".to_string(),
        ));
    }
    let Some(host) = url.host_str() else {
        return Err(LlmError::InvalidRequest(
            "provider base_url must include a host".to_string(),
        ));
    };
    match url.scheme() {
        "https" => {}
        "http" if is_loopback_host(host) => {}
        "http" => {
            return Err(LlmError::InvalidRequest(
                "non-loopback provider base_url must use https://".to_string(),
            ));
        }
        _ => {
            return Err(LlmError::InvalidRequest(
                "provider base_url must use https:// (or http:// for loopback)".to_string(),
            ));
        }
    }
    Ok(trimmed.trim_end_matches('/').to_string())
}

fn is_loopback_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") || host.eq_ignore_ascii_case("localhost.") {
        return true;
    }
    let bare = host
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(host);
    bare.parse::<std::net::IpAddr>()
        .is_ok_and(|ip| ip.is_loopback())
}

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
        .map_err(|e| LlmError::Network(snippet(&e.to_string())))
}

/// Refuse a response whose declared body length exceeds the provider boundary.
/// A missing, invalid, or misleading header is handled by per-chunk accounting.
pub fn ensure_response_size(resp: &reqwest::Response) -> Result<(), LlmError> {
    if resp
        .content_length()
        .is_some_and(|len| len > MAX_PROVIDER_RESPONSE_BYTES as u64)
    {
        return Err(response_too_large());
    }
    Ok(())
}

/// Account one received body chunk without ever crossing the provider boundary.
/// Callers must stop polling the response after this returns an error.
pub fn account_response_chunk(seen: &mut usize, chunk_len: usize) -> Result<(), LlmError> {
    let Some(total) = seen.checked_add(chunk_len) else {
        return Err(response_too_large());
    };
    if total > MAX_PROVIDER_RESPONSE_BYTES {
        return Err(response_too_large());
    }
    *seen = total;
    Ok(())
}

/// Read an ordinary provider response under the same wire-size bound used by
/// the streaming adapters.
pub async fn response_text(resp: reqwest::Response) -> Result<String, LlmError> {
    ensure_response_size(&resp)?;
    let capacity = resp.content_length().unwrap_or(0) as usize;
    let mut body = Vec::with_capacity(capacity);
    let mut seen = 0usize;
    let mut chunks = resp.bytes_stream();
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk.map_err(|e| transport_error(&e))?;
        account_response_chunk(&mut seen, chunk.len())?;
        body.extend_from_slice(&chunk);
    }
    String::from_utf8(body).map_err(|e| LlmError::MalformedOutput(format!("response utf-8: {e}")))
}

/// Read a non-success body without allowing failure diagnostics themselves to
/// become an unbounded allocation. The HTTP status remains authoritative when
/// the body is oversized or unreadable; the read failure becomes its bounded
/// diagnostic snippet.
pub async fn error_response_text(resp: reqwest::Response) -> String {
    match response_text(resp).await {
        Ok(body) => body,
        Err(err) => snippet(&err.to_string()),
    }
}

/// Map an SSE framing/parser failure to the existing transport error type,
/// scrubbing and bounding any provider-controlled diagnostic text.
pub fn stream_protocol_error(message: &str) -> LlmError {
    LlmError::Network(snippet(message))
}

fn response_too_large() -> LlmError {
    LlmError::MalformedOutput(format!(
        "provider response exceeds {MAX_PROVIDER_RESPONSE_BYTES}-byte limit"
    ))
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
        LlmError::Network(snippet(&e.to_string()))
    }
}

/// Parse a `Retry-After` header (integer seconds only; HTTP-date form is
/// ignored and left to the backoff schedule), clamped to
/// [`MAX_RETRY_AFTER_SECS`] so a malicious upstream cannot request an
/// unbounded sleep.
pub fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    let v = headers.get(reqwest::header::RETRY_AFTER)?;
    let s = v.to_str().ok()?.trim();
    s.parse::<u64>()
        .ok()
        .map(|secs| Duration::from_secs(secs.min(MAX_RETRY_AFTER_SECS)))
}

/// A ≤400-char, scrubbed snippet of a response body or provider diagnostic
/// text for error messages.
pub fn snippet(body: &str) -> String {
    // Bound work before scrubbing as well as after it. In particular, a
    // malicious multi-megabyte token must not create a second multi-megabyte
    // allocation merely to produce a 400-character diagnostic.
    let mut input = String::new();
    let mut input_truncated = false;
    for (i, c) in body.chars().enumerate() {
        if i >= 800 {
            input_truncated = true;
            break;
        }
        input.push(c);
    }

    let scrubbed = scrub(&input);
    let mut out = String::new();
    let mut output_truncated = false;
    for c in scrubbed.chars() {
        if out.chars().count() >= 400 {
            output_truncated = true;
            break;
        }
        out.push(c);
    }
    if input_truncated || output_truncated {
        if out.chars().count() == 400 {
            out.pop();
        }
        out.push('…');
    }
    out
}

/// Belt-and-braces scrub: drop anything that looks like a bearer token, JWT,
/// `x-api-key` value, or API key from a string bound for logs/errors. Keys
/// should never reach here (they live only in headers), but a provider echoing
/// one in an error body must not leak.
///
/// Recognizes:
/// - `Bearer <token>` / `Basic <token>` (case-insensitive) across full RFC 6750
///   / base64 / JWT token punctuation (`[A-Za-z0-9._~+/=-]+`), including short
///   non-`sk-` credentials;
/// - `x-api-key` header/JSON values across the same credential alphabet;
/// - standalone 3-segment JWTs (`eyJ….….…`);
/// - standalone `sk-`/`sk_` keys or >40-char `[A-Za-z0-9_-]` secrets even when
///   embedded in compact JSON punctuation (`{"key":"sk-…"}`).
fn scrub(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;

    while i < bytes.len() {
        if is_word_byte(bytes[i]) {
            if let Some(jwt_len) = jwt_prefix_len(&s[i..]) {
                out.push_str("[redacted]");
                i += jwt_len;
                continue;
            }
            let mut end = i;
            while end < bytes.len() && is_word_byte(bytes[end]) {
                end += 1;
            }
            let word = &s[i..end];

            // `Bearer <b64token>` / `Basic <b64token>`: redact the whole
            // credential run including `.`, `+`, `/`, `=`, `~` segments.
            if word.eq_ignore_ascii_case("bearer") || word.eq_ignore_ascii_case("basic") {
                let mut ws = end;
                while ws < bytes.len() && matches!(bytes[ws], b' ' | b'\t') {
                    ws += 1;
                }
                if ws > end && ws < bytes.len() && is_cred_value_byte(bytes[ws]) {
                    let mut val_end = ws;
                    while val_end < bytes.len() && is_cred_value_byte(bytes[val_end]) {
                        val_end += 1;
                    }
                    out.push_str(&s[i..ws]);
                    out.push_str("[redacted]");
                    i = val_end;
                    continue;
                }
            }

            // `x-api-key: <token>` or `"x-api-key":"<token>"`
            if word.eq_ignore_ascii_case("x-api-key")
                && let Some((val_start, val_end)) = header_value_span(bytes, end)
            {
                out.push_str(&s[i..val_start]);
                out.push_str("[redacted]");
                i = val_end;
                continue;
            }

            if looks_secret(word) {
                out.push_str("[redacted]");
            } else {
                out.push_str(word);
            }
            i = end;
        } else if let Some(ch) = s[i..].chars().next() {
            out.push(ch);
            i += ch.len_utf8();
        } else {
            break;
        }
    }
    out
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_'
}

fn is_cred_value_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'+' | b'/' | b'=' | b'~')
}

/// Match a standalone 3-segment base64url JWT starting with `eyJ`.
fn jwt_prefix_len(s: &str) -> Option<usize> {
    if !s.starts_with("eyJ") {
        return None;
    }
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut dots = 0usize;
    let mut seg_len = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'.' {
            if seg_len == 0 || dots >= 2 {
                break;
            }
            dots += 1;
            seg_len = 0;
            i += 1;
        } else if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'=') {
            seg_len += 1;
            i += 1;
        } else {
            break;
        }
    }
    if dots == 2 && seg_len > 0 {
        Some(i)
    } else {
        None
    }
}

/// Match `: <value>`, `= <value>`, or JSON `":"<value>` immediately after a
/// secret header key name, returning `(val_start, val_end)`.
fn header_value_span(bytes: &[u8], mut k: usize) -> Option<(usize, usize)> {
    if k < bytes.len() && matches!(bytes[k], b'"' | b'\'') {
        k += 1;
    }
    while k < bytes.len() && matches!(bytes[k], b' ' | b'\t') {
        k += 1;
    }
    if k >= bytes.len() || !matches!(bytes[k], b':' | b'=') {
        return None;
    }
    k += 1;
    while k < bytes.len() && matches!(bytes[k], b' ' | b'\t') {
        k += 1;
    }
    if k < bytes.len() && matches!(bytes[k], b'"' | b'\'') {
        k += 1;
    }
    let val_start = k;
    while k < bytes.len() && is_cred_value_byte(bytes[k]) {
        k += 1;
    }
    (k > val_start).then_some((val_start, k))
}

fn looks_secret(w: &str) -> bool {
    w.starts_with("sk-")
        || w.starts_with("sk_")
        || (w.starts_with("Bearer") && w.len() > "Bearer".len())
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

    #[test]
    fn response_chunk_accounting_accepts_exact_limit_and_rejects_next_byte() {
        let mut seen = 0;
        account_response_chunk(&mut seen, MAX_PROVIDER_RESPONSE_BYTES).unwrap();
        assert_eq!(seen, MAX_PROVIDER_RESPONSE_BYTES);
        assert!(matches!(
            account_response_chunk(&mut seen, 1),
            Err(LlmError::MalformedOutput(_))
        ));
        assert_eq!(seen, MAX_PROVIDER_RESPONSE_BYTES);
    }

    #[test]
    fn diagnostic_snippet_is_bounded_before_and_after_scrubbing() {
        let out = snippet(&"x".repeat(MAX_PROVIDER_RESPONSE_BYTES));
        assert!(out.chars().count() <= 400);
        assert!(out.ends_with('…'));
    }

    #[test]
    fn p2_core_10_retry_after_clamps_excessive_values() {
        let mut h = HeaderMap::new();
        h.insert(
            reqwest::header::RETRY_AFTER,
            reqwest::header::HeaderValue::from_static("5"),
        );
        assert_eq!(retry_after(&h), Some(Duration::from_secs(5)));

        h.insert(
            reqwest::header::RETRY_AFTER,
            reqwest::header::HeaderValue::from_static("86400"),
        );
        assert_eq!(
            retry_after(&h),
            Some(Duration::from_secs(MAX_RETRY_AFTER_SECS))
        );

        h.insert(
            reqwest::header::RETRY_AFTER,
            reqwest::header::HeaderValue::from_static("18446744073709551615"),
        );
        assert_eq!(
            retry_after(&h),
            Some(Duration::from_secs(MAX_RETRY_AFTER_SECS))
        );

        h.insert(
            reqwest::header::RETRY_AFTER,
            reqwest::header::HeaderValue::from_static("999999999999999999999999"),
        );
        assert_eq!(retry_after(&h), None);
    }

    #[test]
    fn p2_core_26_scrub_redacts_bearer_jwt_base64_and_api_key_headers() {
        let short_bearer = scrub("Authorization: Bearer shorttok failed");
        assert!(!short_bearer.contains("shorttok"), "{short_bearer}");
        assert!(short_bearer.contains("[redacted]"), "{short_bearer}");
        assert!(short_bearer.contains("failed"), "{short_bearer}");

        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U";
        let bearer_jwt = scrub(&format!("Authorization: Bearer {jwt} rejected"));
        assert!(!bearer_jwt.contains("eyJhbGciOiJIUzI1NiJ9"), "{bearer_jwt}");
        assert!(!bearer_jwt.contains("eyJzdWIi"), "{bearer_jwt}");
        assert!(!bearer_jwt.contains("dozjgNry"), "{bearer_jwt}");
        assert!(bearer_jwt.contains("rejected"), "{bearer_jwt}");

        let b64 = scrub("authorization: bearer abc+def/ghi== error");
        assert!(!b64.contains("abc+def/ghi=="), "{b64}");
        assert!(!b64.contains("abc"), "{b64}");
        assert!(b64.contains("error"), "{b64}");

        let json_hdr = scrub(r#"{"headers":{"x-api-key":"my.short+key/123=="}}"#);
        assert!(!json_hdr.contains("my.short+key/123=="), "{json_hdr}");
        assert!(json_hdr.contains("[redacted]"), "{json_hdr}");

        let bare_jwt = scrub(&format!("bad token {jwt} in body"));
        assert!(!bare_jwt.contains("eyJhbGciOiJIUzI1NiJ9"), "{bare_jwt}");
        assert!(!bare_jwt.contains("dozjgNry"), "{bare_jwt}");
        assert!(bare_jwt.contains("in body"), "{bare_jwt}");

        // Ordinary model ids and endpoint URLs remain readable.
        let url_err =
            "error sending request for url (https://openrouter.ai/api/v1/chat/completions): eof";
        assert_eq!(scrub(url_err), url_err);
    }

    #[test]
    fn p2_core_26_validate_base_url_enforces_https_or_loopback() {
        for ok_url in [
            "https://api.anthropic.com",
            "https://openrouter.ai/api/v1/",
            "http://127.0.0.1:11434/v1",
            "http://[::1]:11434/v1",
            "http://localhost:8080/",
            "http://LOCALHOST:8080",
        ] {
            let normalized = validate_base_url(ok_url).expect("allowed base_url");
            assert!(!normalized.ends_with('/'));
        }
        for bad_url in [
            "http://api.openai.com/v1",
            "http://openrouter.ai/api/v1",
            "http://169.254.169.254/latest",
            "http://10.0.0.5:8000",
            "http://0.0.0.0:8000",
            "http://localhost.evil.com",
            "https://user:pass@api.anthropic.com",
            "file:///etc/passwd",
            "not-a-url",
        ] {
            assert!(
                matches!(validate_base_url(bad_url), Err(LlmError::InvalidRequest(_))),
                "expected rejection for {bad_url}"
            );
        }
    }
}
