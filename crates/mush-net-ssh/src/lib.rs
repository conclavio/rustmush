//! # mush-net-ssh — the SSH ingress
//!
//! A russh-based SSH server (design 02 §5) that turns real SSH clients
//! into game connections without knowing anything about the game: the
//! binary implements [`SshHooks`] (auth, account creation, character
//! attach, line delivery, disconnect notice, IP guard) and calls
//! [`SshServer::run`].
//!
//! ## What lives here
//!
//! - **[`SshServer`]** — ed25519 host key generated on first boot
//!   ([`SshConfig::host_key_path`]), keepalives 30 s × 3, no inactivity
//!   timeout (idlers are furniture). PTY required; `exec`/`subsystem`
//!   refused. Its own accept loop gates every connection at accept
//!   ([`SshHooks::admit_connection`]) and survives accept errors; every
//!   accepted stream sits in a [`kill::KillSwitch`], cut at the
//!   [`SshConfig::auth_deadline`] deadlines and a few seconds after any
//!   server-initiated disconnect, so a stalled or half-closed client can
//!   never hold a slot (doc 29a §9).
//! - **Username routing** (M1 subset): `ssh play@host` (and `guest`/`new`,
//!   aliases for now) is accepted with no SSH auth and lands on the classic
//!   welcome screen with in-band `connect <account> <password>` /
//!   `create <account> <password>` / `WHO` / `QUIT`. `ssh <account>@host`
//!   authenticates via SSH password or keyboard-interactive against
//!   [`SshHooks::auth_account`]; public-key auth is politely refused until
//!   `@sshkey` lands (M2/M3). One character auto-connects; several show a
//!   numbered character-select.
//! - **[`LineEditor`]** — the server-side raw-mode line discipline
//!   (editing keys, history, masked/no-echo modes, incremental UTF-8,
//!   8 KiB cap). Pure and heavily unit-tested.
//! - **Output pump** — one task per connection consumes the bounded
//!   `Arc<OutputEvent>` queue handed to the binary in [`NewConn`], renders
//!   through `mush-ansi` with the connection's live [`ClientCaps`]
//!   (TERM/COLORTERM/LANG heuristics in [`detect_caps`]; PTY resizes
//!   update them), and erases/redraws the in-progress input line around
//!   async output.
//!
//! No russh (or ssh-key) type appears in this crate's public API — the
//! pre-1.0 churn stays quarantined here (PLAN risk 4).
//!
//! [`ClientCaps`]: mush_proto::ClientCaps

// WS01-S12: the panic-shaped restriction lints this crate measures zero of,
// held at zero. `crates/mush-contract-tests/tests/ci_policy/restriction_lints.rs` carries
// the measurement this set was cut from — including the counts of the lints
// NOT named here — and fails if this attribute loses one of them.
#![deny(clippy::unwrap_used, clippy::panic, clippy::panic_in_result_fn)]

pub mod assets;
mod backlog;
mod caps;
mod editor;
mod hooks;
mod hostkey;
pub mod kill;
mod line;
mod mind_exec;
pub mod resume;
mod server;
pub mod trusted;
pub mod tui;
mod welcome;

pub use caps::{
    DEFAULT_HEIGHT, DEFAULT_WIDTH, MAX_HEIGHT, MAX_WIDTH, detect_caps, detect_color_depth,
    detect_utf8,
};
pub use editor::{
    EchoMode, EditorAction, HISTORY_CAP, LineEditor, MAX_FEED_OUT_BYTES, MAX_LINE_CHARS,
};
pub use hooks::{
    Admission, AuthResult, BlockReason, CharSummary, CreateErr, LocalReply, NewConn, PixelEvent,
    SshHooks,
};
pub use line::LineBuffer;
pub use server::{
    BoundSshServer, DEFAULT_AUTH_DEADLINE, DISCONNECT_GRACE, SshConfig, SshError, SshServer,
};
#[cfg(unix)]
pub use trusted::BoundTrustedServer;
pub use trusted::{TrustedConfig, TrustedIngress};
pub use tui::pixel::PixelConfig;
pub use welcome::{WelcomeCmd, parse_welcome};

/// Whether `c` is a Unicode bidirectional formatting control or line/paragraph
/// separator that can spoof visual text direction or break single-line framing
/// (`SSH-M6` / `P2-CLIENT-6`):
/// - `U+061C` (Arabic Letter Mark)
/// - `U+200E..=U+200F` (LRM, RLM)
/// - `U+2028..=U+2029` (Line Separator, Paragraph Separator)
/// - `U+202A..=U+202E` (LRE, RLE, PDF, LRO, RLO)
/// - `U+2066..=U+2069` (LRI, RLI, FSI, PDI)
#[must_use]
pub(crate) const fn is_forbidden_format_char(c: char) -> bool {
    matches!(
        c,
        '\u{061C}'
            | '\u{200E}'..='\u{200F}'
            | '\u{2028}'..='\u{2029}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2066}'..='\u{2069}'
    )
}

/// Whether `c` is a Unicode control character (`Cc`) or a forbidden BiDi /
/// line-separator formatting control (`P2-CLIENT-6`).
#[must_use]
pub(crate) fn is_stripped_terminal_char(c: char) -> bool {
    c.is_control() || is_forbidden_format_char(c)
}

/// Whether any JSON string value or object key inside `root` carries a
/// forbidden Unicode BiDi control or `U+2028`/`U+2029` line/paragraph
/// separator (`P2-CLIENT-6`). Iterative so deep nesting cannot overflow the
/// stack.
#[must_use]
pub(crate) fn json_contains_forbidden_format_chars(root: &serde_json::Value) -> bool {
    let mut stack = vec![root];
    while let Some(v) = stack.pop() {
        match v {
            serde_json::Value::String(s) => {
                if s.chars().any(is_forbidden_format_char) {
                    return true;
                }
            }
            serde_json::Value::Array(items) => stack.extend(items),
            serde_json::Value::Object(map) => {
                for (k, val) in map {
                    if k.chars().any(is_forbidden_format_char) {
                        return true;
                    }
                    stack.push(val);
                }
            }
            _ => {}
        }
    }
    false
}

/// Reject structured [`mush_proto::InvokeRequest`] payloads carrying Unicode
/// BiDi controls or `U+2028`/`U+2029` separators in any string field or key
/// (`P2-CLIENT-6`).
pub(crate) fn reject_forbidden_invoke_format_chars(
    request: &mush_proto::InvokeRequest,
) -> Result<(), &'static str> {
    let value = serde_json::to_value(request).map_err(|_| "invalid request")?;
    if json_contains_forbidden_format_chars(&value) {
        return Err("request contains forbidden Unicode formatting characters");
    }
    Ok(())
}
