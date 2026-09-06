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

/// The result of `/follow` (D2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FollowOutcome {
    /// A follow connection is attached (or already was, to this character).
    Following {
        /// The character's name.
        character: String,
        /// Whether this call attached it (`false`: it was already following).
        attached: bool,
    },
    /// This Discord user is not linked to an account.
    NotLinked,
    /// The user's Discord identity is banned.
    Banned,
    /// The account has several characters and none (or an unknown one) was
    /// named; `available` lists them.
    WhichCharacter {
        /// The account's character names.
        available: Vec<String>,
    },
    /// Discord refused to open the DM (the user disallows DMs from the bot).
    DmsClosed,
    /// The game refused the attach (busy, shutting down, a bad character).
    Failed(String),
}

/// The result of `/detach`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DetachOutcome {
    /// The follow is closed.
    Detached {
        /// The character that was followed.
        character: String,
    },
    /// This user was not following anyone.
    NotFollowing,
}

/// The result of a DM line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LineOutcome {
    /// The line ran as the followed character (its output, if any, arrives on
    /// the DM through the writer).
    Accepted,
    /// The line was `QUIT`: the follow is closed.
    Closed,
    /// This user is not following anyone; nothing ran.
    NotFollowing,
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

    /// Attach a follow connection for `user_id`'s character (`character`
    /// names one of the account's, or `None` for the default), opening the
    /// DM channel if needed (D2).
    fn follow<'a>(
        &'a self,
        user_id: u64,
        character: Option<&'a str>,
    ) -> BoxFuture<'a, FollowOutcome>;

    /// Close `user_id`'s follow connection cleanly (as `QUIT` would).
    fn detach(&self, user_id: u64) -> BoxFuture<'_, DetachOutcome>;

    /// Run one DM line as `user_id`'s followed character.
    fn dm_line<'a>(&'a self, user_id: u64, line: &'a str) -> BoxFuture<'a, LineOutcome>;
}
