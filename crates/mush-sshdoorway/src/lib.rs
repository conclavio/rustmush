//! **sshdoorway** (doc 29): the port-22 SSH front door.
//!
//! Players connect to port 22 with a username that names a game server:
//! `ssh dev1+faenix@conclav.io`. The doorway terminates that SSH session,
//! dials the named instance's **trusted ingress** (a Unix socket), tells it
//! the player's real address and any key the player proved they hold (the
//! `mush-sshdoorway-proto` preamble), and relays the session. Everything after
//! the first `+` reaches the instance unchanged, so accounts, `+char` and
//! `@sshkey` keep working. Unknown names get a menu.
//!
//! | Module | Content |
//! |---|---|
//! | [`config`] | `sshdoorway.toml` and its validation |
//! | [`route`] | the username grammar |
//! | [`limits`] | the doorway's own abuse gates |
//! | [`upstream`] | dialing, the preamble client, the upstream russh client |
//! | [`server`] | the accept path, the auth relay, the menu, the splice |
//! | [`drain`] | the doorway's session as the shared drain sees it |
//! | [`kill`] | `mush_drain::kill`: a stream the drain and the wedge path can cut |
//! | [`listen`] | `mush_drain::listen`: `SO_REUSEPORT` / `IPV6_V6ONLY` listeners |
//! | [`logging`] | the file sink (reopened on `SIGUSR2`) and the journald sink |
//! | [`notify`] | `mush_drain::notify`: `sd_notify` |
//!
//! The graceful restart itself — the registry, the warning schedule, the
//! kill switch, the listeners, `sd_notify`, the drain marker and the signal
//! lifecycle — is `mush_drain` (doc 33 §4.5, cut GG.3), lifted out of here
//! with no behaviour change; `drain` adapts the doorway's session to it and
//! `main.rs` gives the lifecycle the doorway's hooks.
//!
//! The doorway holds no game state, no account data and no Redis connection
//! (D-29-16).

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unreachable,
    clippy::panic_in_result_fn
)]

pub mod config;
pub mod drain;
pub mod hostkey;
pub mod limits;
pub mod logging;
pub mod route;
pub mod server;
pub mod upstream;

pub use mush_drain::{kill, listen, notify};

pub use config::Config;
pub use server::Doorway;
