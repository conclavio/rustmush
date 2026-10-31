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
// held at zero. `crates/mush-contract-tests/tests/restriction_lints.rs` carries
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
mod server;
pub mod tui;
mod welcome;

pub use caps::{DEFAULT_HEIGHT, DEFAULT_WIDTH, detect_caps, detect_color_depth, detect_utf8};
pub use editor::{EchoMode, EditorAction, HISTORY_CAP, LineEditor, MAX_LINE_CHARS};
pub use hooks::{
    Admission, AuthResult, BlockReason, CharSummary, CreateErr, LocalReply, NewConn, PixelEvent,
    SshHooks,
};
pub use line::LineBuffer;
pub use server::{
    BoundSshServer, DEFAULT_AUTH_DEADLINE, DISCONNECT_GRACE, SshConfig, SshError, SshServer,
};
pub use tui::pixel::PixelConfig;
pub use welcome::{WelcomeCmd, parse_welcome};
