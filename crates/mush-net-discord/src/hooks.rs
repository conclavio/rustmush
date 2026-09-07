//! [`DiscordHooks`] — what the bot asks the game side to do (the gateway's
//! `discord_bridge` implements it), mirroring `WebHooks`/`SshHooks`.

use futures::future::BoxFuture;

use crate::api::InteractionReply;

/// A component click or a modal submission, as the bridge routes it (D3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interaction {
    /// The routing key (`d:req:<id>:<opt>`, `d:act:<nonce>`, `d:oncall:…`).
    pub custom_id: String,
    /// A select's picked values (one), empty for a button or a modal.
    pub values: Vec<String>,
    /// A modal's `(field, value)` pairs, empty for a click.
    pub fields: Vec<(String, String)>,
    /// The channel of the message the component was on, when known.
    pub channel_id: Option<u64>,
    /// The message the component was on, when known.
    pub message_id: Option<u64>,
}

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

/// The result of a line typed in a relayed Discord channel (D4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RelayLineOutcome {
    /// Nothing to do: an unmapped channel, an outbound-only mapping, a banned
    /// or rate-limited author, an empty line. Silent.
    Ignored,
    /// Spoken on the game channel as the author's character.
    Accepted,
    /// The author's Discord user is not linked to an account; the bot may
    /// hint once in a while.
    NotLinked,
    /// The game refused, with a bounded reason the author may be told.
    Refused(String),
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

    /// Route a component click or a modal submission (D3) by its `custom_id`
    /// namespace: `d:req:` answers a request through the game's `respond`,
    /// `d:act:` opens or submits a notice's reply modal, `d:call:` is the
    /// bridge's own, and anything else crosses to the server's registered
    /// handlers. The reply must be produced within Discord's window.
    fn interaction(
        &self,
        user_id: u64,
        interaction: Interaction,
    ) -> BoxFuture<'_, InteractionReply>;

    /// A message a non-bot author posted in a guild channel (D4): if the
    /// channel is mapped inbound, speak it on the game channel as the
    /// author's character.
    fn relay_line<'a>(
        &'a self,
        user_id: u64,
        channel_id: u64,
        content: &'a str,
    ) -> BoxFuture<'a, RelayLineOutcome>;
}
