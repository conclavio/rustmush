//! The comsys data model (design doc 04 §7.2) and its persistence keys.
//!
//! Live state is one [`ComsysState`] in a typed module state slot on the
//! `World`; **every mutation mirrors to module ops** so the persisted
//! `mod:comsys:*` namespace is always write-through current:
//!
//! | key                    | document                                    |
//! |------------------------|---------------------------------------------|
//! | `meta`                 | `{ "next_chan_id": u64 }`                   |
//! | `chan:{id}`            | one [`Channel`] (including its `seq`)       |
//! | `member:{player}:{id}` | one [`Membership`] (numeric player dbref)   |
//! | `hist:{id}`            | *stream* of [`ChanMsg`] (LOGGED + persist)  |
//!
//! [`ComsysState::rings`] (the in-RAM recall buffers) are **not** a kv doc —
//! they rebuild from the `hist:{id}` stream tails at boot.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

use mush_core::lock::MAX_LOCK_DEPTH;
use mush_core::{Deferred, LockExpr, ModuleOp, ModuleWrite, ObjId, Outbox, World, eval_lock};
use mush_proto::NarrativeKind;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

/// The comsys persistence namespace (`mod:comsys:*`).
pub const NS: &str = "comsys";

/// The default channel, auto-created on first boot (LOUD + LOGGED).
pub const DEFAULT_CHANNEL: &str = "Public";

/// The alias every player gets for [`DEFAULT_CHANNEL`] at auto-join.
pub const DEFAULT_ALIAS: &str = "pub";

/// Default history cap (ring length and stream `MAXLEN`).
pub const DEFAULT_HISTORY_CAP: u32 = 200;

/// Largest history ring and persisted stream tail accepted from either a
/// command or an authoritative channel document. Boot validates this before
/// asking the store for a stream range, so a corrupt/type-clobbered cap cannot
/// turn one channel record into an effectively unbounded allocation request.
pub const MAX_HISTORY_CAP: u32 = 10_000;

/// Longest accepted `comtitle` (characters).
pub const MAX_TITLE_LEN: usize = 60;

/// Longest accepted channel name (bytes; the charset is ASCII).
pub(crate) const MAX_CHANNEL_NAME_LEN: usize = 32;

/// The kv key of the comsys meta document.
pub const META_KEY: &str = "meta";

/// A channel id: monotonic per world, never recycled while the world
/// lives (the allocator resume point persists in the meta doc).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ChannelId(pub u64);

impl fmt::Display for ChannelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A channel's four lock gates (all classic [`LockExpr`] syntax):
/// who may `addcom` it, speak on it, see/receive it, and administer it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChannelPerms {
    /// Who may join (`addcom`).
    pub join: LockExpr,
    /// Who may speak.
    pub speak: LockExpr,
    /// Who may see the channel: hides it from `clist`/`@channel/who` *and*
    /// gates message delivery — except on a [`ChannelFlags::closed`] channel,
    /// where membership decides both and this lock only keeps the channel from
    /// everybody else (see [`Channel::hears`]).
    pub see: LockExpr,
    /// Who may administer (destroy/perm/color/set) — on top of the
    /// always-admitted `COMSYS_ADMIN` power and Royalty+.
    pub admin: LockExpr,
}

impl Default for ChannelPerms {
    /// Open join/speak/see; admin closed (staff and the `COMSYS_ADMIN`
    /// power still pass via the bypass in the admin check).
    fn default() -> ChannelPerms {
        ChannelPerms {
            join: LockExpr::Const(true),
            speak: LockExpr::Const(true),
            see: LockExpr::Const(true),
            admin: LockExpr::Const(false),
        }
    }
}

/// Channel color: an ANSI theme color *name* (the SSH renderer maps it)
/// plus a `#rrggbb` hex for the web client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelColor {
    /// ANSI color name, e.g. `"cyan"`.
    pub ansi: SmolStr,
    /// Web hex color, e.g. `"#00ffff"`.
    pub web_hex: SmolStr,
}

impl Default for ChannelColor {
    fn default() -> ChannelColor {
        ChannelColor {
            ansi: SmolStr::new_static("cyan"),
            web_hex: SmolStr::new_static("#00ffff"),
        }
    }
}

/// Channel behavior flags (serialized as named booleans for a
/// self-documenting JSON doc).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelFlags {
    /// Announce unread counts at login (`"[Public] 3 messages since …"`).
    #[serde(default)]
    pub loud: bool,
    /// Suppress comtitles on this channel.
    #[serde(default)]
    pub no_titles: bool,
    /// Keep message history (the in-RAM recall ring; persisted to the
    /// `hist:{id}` stream when [`HistoryPolicy::persist`] is also set).
    #[serde(default)]
    pub logged: bool,
    /// In-character traffic (a radio, a hive mind, a newsfeed): the clients
    /// list it apart from the OOC channels and set it as prose (interface
    /// plan I7). Off by default: a channel is OOC chatter unless staff say
    /// otherwise.
    #[serde(default)]
    pub ic: bool,
    /// Mirrored to a Discord channel and back (interface programme D4): each
    /// line also goes to the named relay sink, and linked Discord users may
    /// speak on it as their characters. Staff set it; off by default — a
    /// channel's lines never leave the world unless someone says so.
    #[serde(default)]
    pub relay: bool,
    /// **Members only** (16b cut Z.4; 16c N8): a closed channel does not exist
    /// for anybody who is not on it. `clist` omits it, and `@channel/who` and
    /// `addcom` answer `No such channel.` to a non-member **whatever the see lock
    /// says** (the admin bypass stands, as it does for every channel); and
    /// delivery to a **member** never consults the see lock
    /// ([`Channel::hears`]), so a closed channel's see lock may be `#false` —
    /// which is what keeps its roster, its member count and who on it is
    /// connected from anybody outside it — without silencing the members.
    ///
    /// Made by a driving module through `api::ensure_channel` (the simulator's
    /// side nets); no verb sets it. Off by default, and a document written before
    /// the flag reads as open.
    #[serde(default)]
    pub closed: bool,
}

/// How much history a LOGGED channel keeps and whether it survives reboot.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryPolicy {
    /// Mirror history to the persisted stream (`hist:{id}`).
    pub persist: bool,
    /// Maximum retained messages (ring length and stream `MAXLEN`).
    pub cap: u32,
}

impl Default for HistoryPolicy {
    fn default() -> HistoryPolicy {
        HistoryPolicy {
            persist: false,
            cap: DEFAULT_HISTORY_CAP,
        }
    }
}

/// One chat channel. The whole struct (including the live `seq` counter)
/// is the persisted `chan:{id}` document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Channel {
    /// Stable id (also keys the history stream).
    pub id: ChannelId,
    /// Display name, unique case-insensitively ("Public").
    pub name: SmolStr,
    /// One-line description shown by `clist`.
    #[serde(default)]
    pub description: String,
    /// The lock gates.
    #[serde(default)]
    pub perms: ChannelPerms,
    /// ANSI name + web hex.
    #[serde(default)]
    pub color: ChannelColor,
    /// LOUD / NO_TITLES / LOGGED.
    #[serde(default)]
    pub flags: ChannelFlags,
    /// History retention policy.
    #[serde(default)]
    pub history: HistoryPolicy,
    /// Per-channel message counter (monotonic; drives unread cursors).
    #[serde(default)]
    pub seq: u64,
}

impl Channel {
    /// A fresh channel with default perms/color/flags/history and `seq` 0.
    #[must_use]
    pub fn new(id: ChannelId, name: &str) -> Channel {
        Channel {
            id,
            name: SmolStr::new(name),
            description: String::new(),
            perms: ChannelPerms::default(),
            color: ChannelColor::default(),
            flags: ChannelFlags::default(),
            history: HistoryPolicy::default(),
            seq: 0,
        }
    }

    /// **Whether a member of this channel is delivered its traffic** — the
    /// question every delivery read asks of one membership (a live line, a
    /// channel notice, the LOUD login count, `@channel/recall` and the web's two
    /// hydration queries), always with **no** admin bypass.
    ///
    /// The see lock, except on a [`ChannelFlags::closed`] channel, whose members
    /// hear it whatever the see lock says (16b cut Z.4): there the see lock is
    /// what hides the channel from everybody else, and consulting it for a
    /// member would silence the members too.
    ///
    /// Callers have already established that `who` is a member; this never
    /// answers the membership question itself.
    #[must_use]
    pub fn hears(&self, w: &World, who: ObjId) -> bool {
        self.flags.closed || eval_lock(w, &self.perms.see, who, MAX_LOCK_DEPTH)
    }
}

/// Whether `name` is a usable channel name: 1 to [`MAX_CHANNEL_NAME_LEN`]
/// letters, digits, `-` and `_`. Shared by `@channel/create` and the
/// module-driven `api::ensure_channel` (16b cut Z.4), so a name a module makes
/// is one a staffer could have typed.
#[must_use]
pub(crate) fn valid_channel_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_CHANNEL_NAME_LEN
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// One player's membership of one channel — the persisted
/// `member:{player}:{chan}` document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Membership {
    /// The member.
    pub player: ObjId,
    /// The channel.
    pub channel: ChannelId,
    /// The member's personal speaking verb ("pub").
    pub alias: SmolStr,
    /// Their comtitle, if set.
    #[serde(default)]
    pub title: Option<String>,
    /// Gagged: stay joined, hear nothing, may not speak.
    #[serde(default)]
    pub gagged: bool,
    /// Unread cursor: the channel `seq` of the last message delivered to
    /// this member while they were connected.
    #[serde(default)]
    pub last_read: u64,
    /// IC arrival knowledge; absent only on OOC or legacy memberships.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ic: Option<crate::transit::Knowledge>,
}

/// One recorded channel message: the recall-ring entry and the persisted
/// stream payload.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChanMsg {
    /// The channel's sequence number for this message.
    pub seq: u64,
    /// The speaker.
    pub actor: ObjId,
    /// The speaker's display name at send time.
    pub actor_name: String,
    /// Their comtitle at send time (already suppressed for NO_TITLES).
    #[serde(default)]
    pub title: Option<String>,
    /// Say / pose / semipose flavor.
    pub kind: NarrativeKind,
    /// The message text (plain).
    pub text: String,
    /// Send time, Unix milliseconds.
    pub ts: i64,
    /// Where the line came from when not from a connection of the speaker:
    /// `"discord"` for a relayed line (programme D4). Absent for the game's
    /// own lines; such a line is never mirrored back out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<SmolStr>,
}

/// The live comsys state, held in a module state slot on the `World`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ComsysState {
    /// Every channel by id.
    pub channels: BTreeMap<ChannelId, Channel>,
    /// Every membership, keyed `(player, channel)`.
    pub members: BTreeMap<(ObjId, ChannelId), Membership>,
    /// In-RAM recall rings for LOGGED channels (rebuilt from the persisted
    /// streams at boot; never a kv document).
    pub rings: BTreeMap<ChannelId, VecDeque<ChanMsg>>,
    /// Channel id allocator resume point (persisted in the meta doc).
    pub next_chan_id: u64,
    pub ic: crate::transit::State,
    pub channel_members: BTreeSet<(ChannelId, ObjId)>,
}

impl ComsysState {
    pub(crate) fn indexed_members(&self, chan: ChannelId) -> impl Iterator<Item = &Membership> {
        self.channel_members
            .range((chan, ObjId(0))..=(chan, ObjId(u32::MAX)))
            .filter_map(move |(_, p)| self.members.get(&(*p, chan)))
    }
    pub(crate) fn insert_member(&mut self, mut m: Membership) -> Membership {
        if let Some(c) = self.channels.get(&m.channel)
            && c.flags.ic
            && m.ic.is_none()
        {
            m.ic = Some(crate::transit::Knowledge {
                joined: c.seq,
                ..Default::default()
            });
            m.last_read = 0;
        }
        self.channel_members.insert((m.channel, m.player));
        self.members.insert((m.player, m.channel), m.clone());
        m
    }
    pub(crate) fn remove_member(
        &mut self,
        out: &mut Outbox,
        player: ObjId,
        chan: ChannelId,
    ) -> bool {
        self.channel_members.remove(&(chan, player));
        let Some(m) = self.members.remove(&(player, chan)) else {
            return false;
        };
        if let Some(k) = m.ic {
            for r in k.received {
                self.ic.release(out, (chan, r.message));
            }
        }
        true
    }
    /// The channel with this (case-insensitive) name.
    #[must_use]
    pub fn channel_by_name(&self, name: &str) -> Option<&Channel> {
        self.channels
            .values()
            .find(|c| c.name.eq_ignore_ascii_case(name))
    }

    /// A player's membership whose alias matches (case-insensitive).
    #[must_use]
    pub fn membership_by_alias(&self, player: ObjId, alias: &str) -> Option<&Membership> {
        self.memberships_of(player)
            .find(|m| m.alias.eq_ignore_ascii_case(alias))
    }

    /// All of one player's memberships, in channel-id order.
    pub fn memberships_of(&self, player: ObjId) -> impl Iterator<Item = &Membership> {
        self.members
            .range((player, ChannelId(0))..=(player, ChannelId(u64::MAX)))
            .map(|(_, m)| m)
    }

    /// All members of one channel (full scan; membership counts are small).
    pub fn members_of(&self, chan: ChannelId) -> impl Iterator<Item = &Membership> {
        self.members
            .iter()
            .filter(move |((_, c), _)| *c == chan)
            .map(|(_, m)| m)
    }

    /// Allocate the next channel id (callers persist the meta doc).
    pub fn alloc_chan_id(&mut self) -> ChannelId {
        let id = ChannelId(self.next_chan_id);
        self.next_chan_id += 1;
        id
    }
}

// ----------------------------------------------------------------------
// Persistence keys + op helpers
// ----------------------------------------------------------------------

/// The kv key of a channel document.
#[must_use]
pub fn chan_key(id: ChannelId) -> SmolStr {
    SmolStr::new(format!("chan:{}", id.0))
}

/// The kv key of a membership document.
#[must_use]
pub fn member_key(player: ObjId, chan: ChannelId) -> SmolStr {
    SmolStr::new(format!("member:{}:{}", player.0, chan.0))
}

/// The stream key of a channel's persisted history.
#[must_use]
pub fn hist_key(id: ChannelId) -> SmolStr {
    SmolStr::new(format!("hist:{}", id.0))
}

/// Queue one comsys write on the outbox's deferred side-channel (the
/// engine drains it into the per-command `WriteBatch`).
pub(crate) fn push_op(out: &mut Outbox, op: ModuleWrite) {
    out.defer(Deferred::Persist(ModuleOp {
        ns: SmolStr::new_static(NS),
        op,
    }));
}

/// Persist a comsys-owned plain-data document. These records contain only
/// JSON-compatible scalar values, sequences, and string/integer map keys.
pub(crate) fn persist_json(out: &mut Outbox, key: SmolStr, document: &impl Serialize) {
    let value = serde_json::to_value(document).expect("comsys plain-data document serializes");
    push_op(out, ModuleWrite::JsonSet { key, value });
}

/// Mirror a channel document.
pub(crate) fn persist_channel(out: &mut Outbox, c: &Channel) {
    persist_json(out, chan_key(c.id), c);
}

/// Mirror a membership document.
pub(crate) fn persist_membership(out: &mut Outbox, m: &Membership) {
    persist_json(out, member_key(m.player, m.channel), m);
}

/// Mirror the meta document (the channel-id allocator resume point).
pub(crate) fn persist_meta(out: &mut Outbox, next_chan_id: u64) {
    push_op(
        out,
        ModuleWrite::JsonSet {
            key: SmolStr::new_static(META_KEY),
            value: serde_json::json!({ "next_chan_id": next_chan_id }),
        },
    );
}

/// Delete one persisted key (`Del` is type-agnostic: it removes kv docs
/// *and* streams).
pub(crate) fn persist_del(out: &mut Outbox, key: SmolStr) {
    push_op(out, ModuleWrite::Del { key });
}
