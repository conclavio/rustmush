//! # mush-comsys
//!
//! The TinyMUX-style channel system (design doc 04 §7), built as a
//! [`GameModule`] on mush-core's merged module API — one of the M3 proving
//! modules for treaty ruling 4.
//!
//! Surface: `addcom pub=Public`, alias-as-verb speaking (`pub Hello`,
//! `pub :waves`, `pub ;grins`), `delcom`, `comlist`, `comtitle`, `clist`,
//! and `@channel/create|destroy|perm|color|set|who|recall|gag|ungag|list`.
//!
//! ## How it hangs off the module API
//!
//! - **Speaking** rides the dedicated [`AliasStage`] dispatch seam
//!   (treaty ruling 5): the stage claims a line whose first word is one of
//!   the speaker's channel aliases, *before* the `home` keyword, exits, and
//!   the command table. `addcom` therefore warns when a new alias shadows a
//!   command or an exit — the check runs **at alias-creation time only**
//!   (against the command registry via `Ctx::reg` and the exits reachable
//!   from the actor's current room); the stage itself never re-checks.
//! - **State** lives in one [`ComsysState`] module state slot; every
//!   mutation mirrors to module ops (kv doc per channel `chan:{id}`, per
//!   membership `member:{player}:{chan}`, a `meta` doc for the channel-id
//!   allocator, history via `StreamAppend hist:{id}` for LOGGED+persist
//!   channels) so persistence is write-through with command atomicity.
//! - **Boot**: [`GameModule::on_world_loaded`] rebuilds the whole state
//!   from [`ModuleBootData`] (kv docs + stream tails). *For the driver*:
//!   comsys expects `streams` to carry `hist:{id}` for every kv doc keyed
//!   `chan:{id}` (load with `stream_range(ns, "hist:{id}", cap)` using the
//!   channel doc's `history.cap`). On a completely empty namespace (first
//!   boot) the default `Public` channel (LOUD + LOGGED + persisted history)
//!   is created and **every existing live Player** is auto-joined under the
//!   alias `pub`.
//! - **Auto-join at character creation** uses the `ObjCreated` world-event
//!   observer, filtered to `ObjKind::Player` — chosen over the `Connected`
//!   hook so the join happens exactly once and `delcom` sticks (a
//!   Connected-hook join would silently re-join players who left).
//!   The driver's player-creation path must emit
//!   [`WorldEvent::ObjCreated`] (as `@pcreate`-style commands do via the
//!   dispatch helpers) for this to fire on real character creation.
//! - **Login unread notice**: the `Connected` hook reports
//!   `[Public] 3 messages since you last read.` for LOUD channels, driven
//!   by the per-membership `last_read` cursor, which advances (and mirrors)
//!   only when a message is actually delivered to a connected member.
//! - **Cleanup**: an `ObjDestroyed` observer drops a destroyed actor's
//!   memberships (and their persisted docs).
//!
//! Delivery itself emits [`mush_proto::OutputEvent::Channel`] frames — one
//! per receiving member as `Audience::Actor` — carrying the channel's
//! monotonically increasing `seq` (persisted in the channel doc), speaker,
//! comtitle, say/pose flavor, text, and timestamp.
//!
//! [`AliasStage`]: mush_core::AliasStage
//! [`WorldEvent::ObjCreated`]: mush_core::WorldEvent

pub mod api;
mod commands;
mod data;
mod query;
mod speak;

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use mush_core::module::{
    EventHook, GameModule, HookPoint, ModError, ModuleBootData, ModuleCtx, Registrar, WorldEvent,
    WorldEventKind,
};
use mush_core::{Ctx, Flags, ObjId, ObjKind, lock::MAX_LOCK_DEPTH};
use smol_str::SmolStr;

pub use data::{
    ChanMsg, Channel, ChannelColor, ChannelFlags, ChannelId, ChannelPerms, ComsysState,
    DEFAULT_ALIAS, DEFAULT_CHANNEL, DEFAULT_HISTORY_CAP, HistoryPolicy, MAX_HISTORY_CAP, META_KEY,
    Membership, NS, chan_key, hist_key, member_key,
};
pub use query::{BacklogQuery, MAX_BACKLOG_LIMIT, StateQuery};
pub use speak::ComsysAliases;

use data::{persist_channel, persist_del, persist_membership, persist_meta};

/// The comsys game module. Hand one to `Engine::with_modules`.
pub struct ComsysModule;

impl ComsysModule {
    /// A fresh module instance.
    #[must_use]
    pub fn new() -> ComsysModule {
        ComsysModule
    }
}

impl Default for ComsysModule {
    fn default() -> Self {
        ComsysModule::new()
    }
}

impl GameModule for ComsysModule {
    fn name(&self) -> &'static str {
        "comsys"
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
        reg.state(ComsysState::default());
        reg.alias_stage(Arc::new(ComsysAliases))?;
        reg.hook(HookPoint::Connected, Arc::new(LoudNotice));
        reg.subscribe(WorldEventKind::ObjCreated, Arc::new(on_obj_created));
        reg.subscribe(WorldEventKind::ObjDestroyed, Arc::new(on_obj_destroyed));
        reg.script_api("comsys")?;
        // M4 web hydration: the "comsys" panel feed (channel tabs) plus its
        // two read-only query handlers (see `query.rs`).
        reg.panel_feed("comsys")?;
        reg.query_handler("comsys.state", Arc::new(query::StateQuery))?;
        reg.query_handler("comsys.backlog", Arc::new(query::BacklogQuery))?;
        commands::register(reg)?;
        Ok(())
    }

    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        data: ModuleBootData,
    ) -> Result<(), ModError> {
        load_boot(ctx, data)
    }

    /// The boot recipe from the module docs, expressed to the driver: for
    /// every kv doc `chan:{id}`, load the newest `history.cap` entries of
    /// stream `hist:{id}` (a channel that never persisted history simply
    /// has no stream — the load yields nothing).
    fn boot_stream_requests(
        &self,
        kv: &[(SmolStr, serde_json::Value)],
    ) -> Result<Vec<(SmolStr, u32)>, ModError> {
        let mut requests = Vec::new();
        for (key, value) in kv {
            let Some(id) = key.strip_prefix("chan:") else {
                continue;
            };
            let Some(history) = value.get("history") else {
                continue;
            };
            let cap = history
                .get("cap")
                .and_then(serde_json::Value::as_u64)
                .and_then(|cap| u32::try_from(cap).ok())
                .ok_or_else(|| {
                    ModError::msg(format!(
                        "comsys: channel doc {key:?} has an invalid history cap"
                    ))
                })?;
            if !(1..=MAX_HISTORY_CAP).contains(&cap) {
                return Err(ModError::msg(format!(
                    "comsys: channel doc {key:?} history cap {cap} is outside 1-{MAX_HISTORY_CAP}"
                )));
            }
            requests.push((SmolStr::new(format!("hist:{id}")), cap));
        }
        Ok(requests)
    }
}

/// The `Connected` hook: LOUD channels announce unread counts at login.
struct LoudNotice;

impl EventHook for LoudNotice {
    fn connected(&self, ctx: &mut Ctx<'_>, actor: ObjId) {
        for line in speak::unread_notices(ctx.w, actor) {
            ctx.out.sys_to_actor(actor, "channel", line);
        }
    }
}

/// `ObjCreated` observer: auto-join newly created Players to the default
/// channel (respecting its join lock) under the [`DEFAULT_ALIAS`].
fn on_obj_created(ctx: &mut Ctx<'_>, ev: &WorldEvent) {
    let WorldEvent::ObjCreated {
        obj,
        kind: ObjKind::Player,
        ..
    } = ev
    else {
        return;
    };
    auto_join_default(ctx, *obj);
}

/// `ObjDestroyed` observer: drop the husk's memberships and their docs.
fn on_obj_destroyed(ctx: &mut Ctx<'_>, ev: &WorldEvent) {
    let WorldEvent::ObjDestroyed { obj } = ev else {
        return;
    };
    let removed: Vec<(ObjId, ChannelId)> = {
        let Some(st) = ctx.w.module_state_mut::<ComsysState>() else {
            return;
        };
        let keys: Vec<(ObjId, ChannelId)> = st
            .memberships_of(*obj)
            .map(|m| (m.player, m.channel))
            .collect();
        for k in &keys {
            st.members.remove(k);
        }
        keys
    };
    for (p, c) in removed {
        persist_del(ctx.out, member_key(p, c));
    }
}

/// Join `player` to the default channel if it exists, admits them, and they
/// are not already on it. Quiet: no join announcement (character creation
/// is not a room event).
fn auto_join_default(ctx: &mut Ctx<'_>, player: ObjId) {
    let target = ctx.w.module_state::<ComsysState>().and_then(|st| {
        let c = st.channel_by_name(DEFAULT_CHANNEL)?;
        if st.members.contains_key(&(player, c.id))
            || st.membership_by_alias(player, DEFAULT_ALIAS).is_some()
        {
            return None;
        }
        Some((c.id, c.seq, c.perms.join.clone()))
    });
    let Some((chan, seq, join)) = target else {
        return;
    };
    if !mush_core::eval_lock(ctx.w, &join, player, MAX_LOCK_DEPTH) {
        return;
    }
    let m = Membership {
        player,
        channel: chan,
        alias: SmolStr::new_static(DEFAULT_ALIAS),
        title: None,
        gagged: false,
        last_read: seq,
    };
    if let Some(st) = ctx.w.module_state_mut::<ComsysState>() {
        st.members.insert((player, chan), m.clone());
    }
    persist_membership(ctx.out, &m);
}

/// Rebuild [`ComsysState`] from persisted boot data; orphaned membership
/// docs (missing channel or destroyed player) are dropped and their keys
/// deleted. An empty **namespace** (no documents at all, not even meta) is
/// a first boot: seed [`DEFAULT_CHANNEL`] and auto-join every existing
/// live Player.
///
/// A channel document that fails to decode FAILS THE BOOT: proceeding
/// would classify every one of that channel's memberships as orphans and
/// permanently delete them — corrupt data is an operational problem to
/// fix, never a license for destructive "recovery". (Membership docs are
/// individually skippable: dropping one loses one player's alias, not a
/// whole channel's roster.)
fn load_boot(ctx: &mut ModuleCtx<'_>, data: ModuleBootData) -> Result<(), ModError> {
    // First boot = completely empty namespace, decided BEFORE any parsing:
    // "all channel docs corrupt" must never look like a fresh install (it
    // would reseed Public and force-join everyone), and staff legitimately
    // destroying the last channel leaves the meta doc behind — also not a
    // first boot.
    let first_boot = data.kv.is_empty();
    let mut channels: BTreeMap<ChannelId, Channel> = BTreeMap::new();
    let mut raw_members: Vec<Membership> = Vec::new();
    let mut next_from_meta: Option<u64> = None;

    for (key, value) in data.kv {
        if key == META_KEY {
            next_from_meta = value
                .get("next_chan_id")
                .and_then(serde_json::Value::as_u64);
        } else if let Some(id) = key.strip_prefix("chan:") {
            let id = id
                .parse::<u64>()
                .map_err(|_| ModError::msg(format!("comsys: bad channel key {key:?}")))?;
            let chan = serde_json::from_value::<Channel>(value)
                .map_err(|e| ModError::msg(format!("comsys: bad channel doc {key:?}: {e}")))?;
            if !(1..=MAX_HISTORY_CAP).contains(&chan.history.cap) {
                return Err(ModError::msg(format!(
                    "comsys: channel doc {key:?} history cap {} is outside 1-{MAX_HISTORY_CAP}",
                    chan.history.cap
                )));
            }
            if chan.id != ChannelId(id) {
                return Err(ModError::msg(format!(
                    "comsys: channel doc {key:?} claims id {}",
                    chan.id
                )));
            }
            channels.insert(chan.id, chan);
        } else if key.starts_with("member:") {
            match serde_json::from_value::<Membership>(value) {
                Ok(m) => raw_members.push(m),
                Err(e) => {
                    tracing::error!(%key, error = %e, "comsys: bad membership doc skipped");
                }
            }
        }
        // Unknown keys are tolerated (future schema additions).
    }

    let mut members: BTreeMap<(ObjId, ChannelId), Membership> = BTreeMap::new();
    let mut orphans: Vec<SmolStr> = Vec::new();
    for m in raw_members {
        let live_player = ctx
            .w
            .get(m.player)
            .is_some_and(|o| !o.flags.contains(Flags::GOING));
        if live_player && channels.contains_key(&m.channel) {
            members.insert((m.player, m.channel), m);
        } else {
            orphans.push(member_key(m.player, m.channel));
        }
    }

    let mut rings: BTreeMap<ChannelId, VecDeque<ChanMsg>> = BTreeMap::new();
    for (key, entries) in data.streams {
        let Some(id) = key
            .strip_prefix("hist:")
            .and_then(|s| s.parse::<u64>().ok())
        else {
            continue;
        };
        let id = ChannelId(id);
        let Some(chan) = channels.get(&id) else {
            continue;
        };
        let mut ring: VecDeque<ChanMsg> = entries
            .into_iter()
            .filter_map(|(_, v)| serde_json::from_value::<ChanMsg>(v).ok())
            .collect();
        let cap = chan.history.cap.max(1) as usize;
        while ring.len() > cap {
            ring.pop_front();
        }
        rings.insert(id, ring);
    }

    let next_chan_id =
        next_from_meta.unwrap_or_else(|| channels.keys().last().map_or(0, |id| id.0 + 1));

    *ctx.w
        .module_state_mut::<ComsysState>()
        .expect("comsys state slot installed at registration") = ComsysState {
        channels,
        members,
        rings,
        next_chan_id,
    };
    for key in orphans {
        persist_del(ctx.out, key);
    }
    if first_boot {
        seed_default_channel(ctx);
    }
    Ok(())
}

/// First boot: create [`DEFAULT_CHANNEL`] (LOUD + LOGGED, persisted
/// history) and join every existing live Player under [`DEFAULT_ALIAS`].
fn seed_default_channel(ctx: &mut ModuleCtx<'_>) {
    let players: Vec<ObjId> = ctx
        .w
        .iter()
        .filter(|o| o.kind == ObjKind::Player && !o.flags.contains(Flags::GOING))
        .map(|o| o.id)
        .collect();
    let (chan, memberships, next) = {
        let st = ctx
            .w
            .module_state_mut::<ComsysState>()
            .expect("comsys state slot installed at registration");
        let id = st.alloc_chan_id();
        let mut c = Channel::new(id, DEFAULT_CHANNEL);
        c.description = "General chatter for everyone.".to_string();
        c.flags = ChannelFlags {
            loud: true,
            no_titles: false,
            logged: true,
            ic: false,
        };
        c.history = HistoryPolicy {
            persist: true,
            cap: DEFAULT_HISTORY_CAP,
        };
        st.channels.insert(id, c.clone());
        let mut ms = Vec::new();
        for p in players {
            let m = Membership {
                player: p,
                channel: id,
                alias: SmolStr::new_static(DEFAULT_ALIAS),
                title: None,
                gagged: false,
                last_read: 0,
            };
            st.members.insert((p, id), m.clone());
            ms.push(m);
        }
        (c, ms, st.next_chan_id)
    };
    persist_channel(ctx.out, &chan);
    persist_meta(ctx.out, next);
    for m in &memberships {
        persist_membership(ctx.out, m);
    }
}
