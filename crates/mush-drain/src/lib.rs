//! **mush-drain** (doc 33 §4.5, D-33-6): restarting a daemon without dropping
//! anyone. Built for `sshdoorway` (doc 29 §9) and lifted out of it in cut
//! GG.3 so that `mush-gateway` gets the same mechanism from GG.4 on, and the
//! next review finding is fixed once.
//!
//! Two generations of a daemon alternate under systemd (`<unit>@a`,
//! `<unit>@b`). They share their TCP ports with `SO_REUSEPORT`; the swap
//! script starts the idle generation, checks it stays up, then sends
//! `SIGUSR1` to the old one, which stops accepting, keeps its sessions, warns
//! them on a schedule and exits when empty or at the deadline. Established
//! sessions never move between processes; only new connections do.
//!
//! | Module | Content |
//! |---|---|
//! | [`drain`] | the generation's lifecycle and registry, generic over the [`Session`] |
//! | [`kill`] | a stream the drain and a wedge path can cut from outside |
//! | [`listen`] | `SO_REUSEPORT` / `IPV6_V6ONLY` listeners and the `tcp_migrate_req` check |
//! | [`notify`] | `sd_notify` |
//! | [`marker`] | the `draining-<gen>` marker that keeps a drained generation from serving again |
//! | [`lifecycle`] | the signals: `SIGUSR1` drain, `SIGTERM`/`SIGINT` finish, `SIGHUP`/`SIGUSR2` hooks, the hard exit; a [`Trigger`] channel stands in for the signals in-process |
//!
//! The deploy half, `deploy/drain/swap.sh`, is parameterised by unit stem,
//! marker directory, a pre-start check and an optional symlink to re-point.
//!
//! Two rules carried from the doorway's reviews (29a §7 H2/H3, §8 C-1) hold
//! here for every daemon: a `Type=notify` process that exits **before**
//! `READY=1` is a protocol failure that `Restart=on-failure` restarts, so a
//! start that finds its marker sends `READY=1` and then exits 0; and a drain
//! never sends `STOPPING=1`, because a unit that reports itself stopping
//! breaks the swap script's two-generation guard.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unreachable,
    clippy::panic_in_result_fn
)]

pub mod drain;
pub mod kill;
pub mod lifecycle;
pub mod listen;
pub mod marker;
pub mod notify;

pub use drain::{Drain, LiveGuard, Phase, Session, Warning, schedule};
pub use lifecycle::{DrainConfig, Host, Signals, Trigger};
