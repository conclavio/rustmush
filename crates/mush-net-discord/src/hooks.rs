//! [`DiscordHooks`] — what the bot asks the game side to do (the gateway's
//! `discord_bridge` implements it), mirroring `WebHooks`/`SshHooks`.

use futures::future::BoxFuture;

/// The result of redeeming a link code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkOutcome {
    /// The Discord user is now bound to `account`.
    Linked {
        /// The account's display name.
        account: String,
    },
    /// Refused — for every reason alike (an unknown, expired or used code, a
    /// banned user, an account or user already linked): the reply must not
    /// be an oracle on which accounts exist or are linked.
    Refused,
}

/// The result of `/unlink`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnlinkOutcome {
    /// The link is gone.
    Unlinked {
        /// The account that was linked.
        account: String,
    },
    /// This user was not linked.
    NotLinked,
}

/// The `/whoami` view: the account a Discord user is linked to. Never an
/// identity (the user is asking about themself; the account name is the
/// answer).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WhoAmI {
    /// The account's display name.
    pub account: String,
    /// Unix seconds when the link was made.
    pub linked_at: i64,
    /// Whether auto-follow is on.
    pub auto_follow: bool,
}

/// The game-side hooks the bot calls. Every method takes the invoking
/// Discord user's snowflake, which the Gateway asserted.
pub trait DiscordHooks: Send + Sync {
    /// Redeem a **normalized** link code for `user_id` (D-D-3).
    fn link_redeem<'a>(&'a self, user_id: u64, code: &'a str) -> BoxFuture<'a, LinkOutcome>;

    /// Remove `user_id`'s link.
    fn unlink(&self, user_id: u64) -> BoxFuture<'_, UnlinkOutcome>;

    /// What `user_id` is linked to, if anything.
    fn whoami(&self, user_id: u64) -> BoxFuture<'_, Option<WhoAmI>>;
}
