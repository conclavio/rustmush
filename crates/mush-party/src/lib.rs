//! `mush-party` — the **ephemeral party** subsystem (E6a, `docs/design/08f` §3).
//!
//! A party is a **roster with a name and a leader**: the people who mean to do a
//! thing together. It is game-neutral — no Shadowrun vocabulary, no sheet, no dice —
//! and it is deliberately the smallest record in the E-series.
//!
//! ## The one rule everything else follows: a party is not a principal (D-E6-5)
//!
//! The party never learns about money. It holds no account, names no instrument,
//! posts nothing, and cannot be a party to a contract. When a payer takes a crew on
//! for a job, the **ruleset** freezes the roster by calling [`lock`] with an opaque
//! [`ContractRef`] — a `u64` this crate stores and never interprets — and the
//! contract does **not** point back at the party. That is enforced by the dependency
//! list, not by a comment: `mush-party` depends on `mush-core`, `serde`, `serde_json`
//! and `smol_str`, and `cargo tree` shows no `mush-economy`, `mush-org` or
//! `mush-faction` edge. (`tracing` is the fifth entry: the logging facade a loud drop
//! at boot needs.)
//!
//! ## The state machine (§3.2)
//!
//! ```text
//! Forming ──lock(contract)──────────────────────────────────▶ Locked{contract}
//! Forming ──leader disbands | last member leaves | idle──────▶ Disbanded{how}
//! Locked  ──release(Settled|Released)────────────────────────▶ Disbanded{how}
//! Locked  ──unlock()  (the contract was AMENDED first)───────▶ Forming
//! ```
//!
//! **`Locked` refuses every membership change** — `/invite`, `/uninvite`, `/accept`,
//! `/decline`, `/leave`, `/kick`, `/role`, `/disband`. A substitution is therefore not
//! a party operation at all: the ruleset amends the contract, calls [`unlock`], the
//! players change the roster, and the ruleset locks again. **`Disbanded` is terminal**
//! and refuses everything, forever.
//!
//! ## What is where
//!
//! | file | what |
//! |---|---|
//! | `record.rs` | [`Party`], its caps, and the `TryFrom<PartyWire>` that re-checks every one on load |
//! | `api.rs` | the ruleset seam: [`lock`], [`release`], [`unlock`], [`parties_of`], [`may_view`] |
//! | `module.rs` | [`PartyStore`], [`PartyModule`], the persistence keys, the staff predicate |
//! | `boot.rs` | hydration: the wire, the saturating allocator floor, the loud drop |
//! | `tick.rs` | idle expiry and invitation pruning, bounded per fire |
//! | `commands.rs` | the `+party` surface and the disambiguation rule |
//! | `hooks.rs` | a destroyed object leaves every live, unlocked roster |
//!
//! ## Disciplines this crate keeps
//!
//! - **Existence hiding on every refusal.** A party the actor may not view answers
//!   exactly [`NO_SUCH_PARTY`] — identical text for nonexistent and unviewable
//!   (08a decision 13). Viewers are a member, an invitee, or staff.
//! - **Every persisted container doc runs the constructor invariants.** [`Party`]
//!   deserializes only through its wire (the E5a fold rule), and a document that
//!   fails it is dropped **loudly** at boot with a `Del` fixup — never a boot failure,
//!   because a party is ephemeral by construction and taking a world down for one
//!   would be the larger failure.
//! - **No `?`, `unwrap`, `expect` or panicking index in a tick or hook handler**, and
//!   `saturating_*`/`checked_*` arithmetic everywhere: those run in the world task.
//! - **Bounded strings and collections everywhere**, re-checked on load.
//!
//! ## Wiring
//!
//! ```ignore
//! Engine::with_modules(vec![Box::new(mush_party::PartyModule::new()), /* … */]);
//! ```
//!
//! There is no ordering requirement against any other module: this one reads wall
//! time and `mush-core`'s own staff predicate and asks nothing of anybody.

mod api;
mod boot;
mod commands;
mod error;
mod hooks;
mod ids;
mod module;
mod record;
mod tick;

pub use api::{NO_SUCH_PARTY, lock, may_view, parties_of, release, unlock};
pub use commands::LIST_CAP;
pub use error::PartyError;
pub use ids::{ContractRef, PartyId, parse_party_id};
pub use module::{
    MAX_PARTY_EXPIRIES_PER_FIRE, META_KEY, NS, PartyModule, PartyStore, TICK_PERIOD_SECS, TICK_TAG,
    is_character, is_staff, party_key, render_actor,
};
pub use record::{
    DisbandCause, Invitation, MAX_PARTIES, MAX_PARTIES_PER_LEADER, MAX_PARTY_INVITES,
    MAX_PARTY_NAME_BYTES, MAX_PARTY_ROLE_BYTES, MAX_PARTY_SIZE, PARTY_IDLE_TTL, Party, PartyStatus,
    check_label,
};
