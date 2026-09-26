//! Programmatic membership API for **other modules** (cross-namespace writes
//! go through the owning module's public API — the module-layering rule).
//!
//! First consumer: `mush-faction`, which binds a channel to a faction and
//! keeps eligible members joined. These calls are module-driven: they skip
//! the join lock (the driving module owns the policy decision) but never
//! skip persistence or the membership invariants.
//!
//! Second consumer (16b cut Z.4; 16c N8): the simulator (`tacsim`), whose two
//! side nets are made once through [`ensure_channel`] as **closed** channels
//! ([`crate::ChannelFlags::closed`]), joined with [`force_join`] and
//! [`set_title`] when a match starts, and swept with [`members_of`] and
//! [`force_leave`].

use mush_core::{Ctx, ObjId, World};
use smol_str::SmolStr;

use crate::data::{
    Channel, ChannelFlags, ChannelId, ChannelPerms, ComsysState, MAX_TITLE_LEN, Membership,
    member_key, persist_channel, persist_del, persist_membership, persist_meta, valid_channel_name,
};

/// **Make a channel once** (16b cut Z.4): the channel named `name`
/// (case-insensitive) is answered when it exists, with its `perms` and `flags`
/// re-asserted to the arguments — the driving module owns the policy, so a lock
/// a staffer opened with `@channel/perm` is closed again — and its colour,
/// description, history and members kept; else it is allocated with
/// `description`, `flags` and `perms` (default colour and history), persisted
/// (the channel document and the meta document) and answered.
///
/// Writes nothing when the channel exists and already carries `perms` and
/// `flags`, so a boot that calls this every time persists the channel once.
/// `None` when comsys is not composed or `name` is not a usable channel name
/// (the one rule `@channel/create` applies), and then nothing is written.
pub fn ensure_channel(
    ctx: &mut Ctx<'_>,
    name: &str,
    description: &str,
    flags: ChannelFlags,
    perms: ChannelPerms,
) -> Option<ChannelId> {
    if !valid_channel_name(name) {
        return None;
    }
    let st = ctx.w.module_state_mut::<ComsysState>()?;
    if let Some(id) = st.channel_by_name(name).map(|c| c.id) {
        let c = st.channels.get_mut(&id)?;
        if c.perms != perms || c.flags != flags {
            c.perms = perms;
            c.flags = flags;
            let doc = c.clone();
            persist_channel(ctx.out, &doc);
        }
        return Some(id);
    }
    let id = st.alloc_chan_id();
    let mut c = Channel::new(id, name);
    c.description = description.to_string();
    c.flags = flags;
    c.perms = perms;
    st.channels.insert(id, c.clone());
    let next = st.next_chan_id;
    persist_channel(ctx.out, &c);
    persist_meta(ctx.out, next);
    Some(id)
}

/// **Set (or clear) a member's comtitle on `channel`** (16b cut Z.4),
/// persisting the membership — module-driven, so the `NO_TITLES` refusal
/// `comtitle` makes is not made here (a title on such a channel is kept and
/// suppressed on delivery, as it always is).
///
/// `None`, or a title that is empty after trimming, clears it. `false` when
/// `player` is not a member, or the title is longer than `MAX_TITLE_LEN` (sixty)
/// characters (`comtitle`'s rule), and then nothing changes. A title that is
/// already the member's writes nothing.
pub fn set_title(
    ctx: &mut Ctx<'_>,
    player: ObjId,
    channel: ChannelId,
    title: Option<&str>,
) -> bool {
    let title = title.map(str::trim).filter(|t| !t.is_empty());
    if title.is_some_and(|t| t.chars().count() > MAX_TITLE_LEN) {
        return false;
    }
    let Some(m) = ctx
        .w
        .module_state_mut::<ComsysState>()
        .and_then(|st| st.members.get_mut(&(player, channel)))
    else {
        return false;
    };
    if m.title.as_deref() == title {
        return true;
    }
    m.title = title.map(str::to_string);
    let doc = m.clone();
    persist_membership(ctx.out, &doc);
    true
}

/// **The members of `channel`, by player** (16b cut Z.4), in player order. A
/// read; empty when the channel does not exist or comsys is not composed.
#[must_use]
pub fn members_of(w: &World, channel: ChannelId) -> Vec<ObjId> {
    w.module_state::<ComsysState>()
        .map(|st| st.members_of(channel).map(|m| m.player).collect())
        .unwrap_or_default()
}

/// The id of the channel with this (case-insensitive) name, if any.
#[must_use]
pub fn channel_id_by_name(w: &World, name: &str) -> Option<ChannelId> {
    w.module_state::<ComsysState>()
        .and_then(|st| st.channel_by_name(name))
        .map(|c| c.id)
}

/// The display name of a channel by id, if it exists.
#[must_use]
pub fn channel_name(w: &World, id: ChannelId) -> Option<SmolStr> {
    w.module_state::<ComsysState>()
        .and_then(|st| st.channels.get(&id))
        .map(|c| c.name.clone())
}

/// A channel's lock gates, if it exists — so a binding module can inspect
/// (and warn about) world-open see/join locks without touching comsys state.
#[must_use]
pub fn channel_perms(w: &World, id: ChannelId) -> Option<crate::data::ChannelPerms> {
    w.module_state::<ComsysState>()
        .and_then(|st| st.channels.get(&id))
        .map(|c| c.perms.clone())
}

/// Reduce a free-form name to a legal channel alias (lowercase, alias
/// charset, bounded), falling back to `"fac"` when nothing survives.
#[must_use]
pub fn alias_from_name(name: &str) -> String {
    let cleaned: String = name
        .to_ascii_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_' || *c == '.')
        .take(14)
        .collect();
    if cleaned.is_empty() {
        "fac".to_string()
    } else {
        cleaned
    }
}

/// Join `player` to `channel` under `alias` (or the first free `alias2`…
/// `alias9` variant), skipping the join lock — the calling module owns the
/// policy. Already-joined players are left untouched. Returns `false` when
/// the channel does not exist or no alias variant is free.
///
/// The new member starts read-up-to-date and hears a quiet system notice so
/// the join is never invisible to them.
pub fn force_join(ctx: &mut Ctx<'_>, player: ObjId, channel: ChannelId, alias: &str) -> bool {
    let Some((name, seq)) = ctx
        .w
        .module_state::<ComsysState>()
        .and_then(|st| st.channels.get(&channel))
        .map(|c: &Channel| (c.name.clone(), c.seq))
    else {
        return false;
    };
    if ctx
        .w
        .module_state::<ComsysState>()
        .is_some_and(|st| st.members.contains_key(&(player, channel)))
    {
        return true;
    }
    // Pick a free alias: the desired one, else numbered variants.
    let base = alias_from_name(alias);
    let candidates = std::iter::once(base.clone()).chain((2..=9).map(|n| format!("{base}{n}")));
    let mut chosen: Option<String> = None;
    for cand in candidates {
        let taken = ctx
            .w
            .module_state::<ComsysState>()
            .and_then(|st| st.membership_by_alias(player, &cand))
            .is_some();
        if !taken {
            chosen = Some(cand);
            break;
        }
    }
    let Some(alias) = chosen else {
        tracing::warn!(player = player.0, channel = %channel, "comsys: no free alias for forced join");
        return false;
    };
    // **16b cut Z.4**: when the alias asked for was taken, the member is told
    // what it already means to them as well as what they got instead — a pilot
    // who types `ra` and reaches `Public` should not have to find out by doing it.
    let shadowed: Option<SmolStr> = (alias != base)
        .then(|| {
            ctx.w.module_state::<ComsysState>().and_then(|st| {
                let taken = st.membership_by_alias(player, &base)?;
                st.channels.get(&taken.channel).map(|c| c.name.clone())
            })
        })
        .flatten();
    let m = Membership {
        player,
        channel,
        alias: SmolStr::new(&alias),
        title: None,
        gagged: false,
        last_read: seq,
    };
    if let Some(st) = ctx.w.module_state_mut::<ComsysState>() {
        st.members.insert((player, channel), m.clone());
    }
    persist_membership(ctx.out, &m);
    let line = match shadowed {
        Some(other) => format!(
            "You have been joined to channel {name} (alias: {alias}; {base} is your alias for \
             {other})."
        ),
        None => format!("You have been joined to channel {name} (alias: {alias})."),
    };
    ctx.out.sys_to_actor(player, "channel", line);
    true
}

/// Remove `player` from `channel` (module-driven). Returns whether a
/// membership existed. The member hears a quiet system notice.
pub fn force_leave(ctx: &mut Ctx<'_>, player: ObjId, channel: ChannelId) -> bool {
    let existed = ctx
        .w
        .module_state_mut::<ComsysState>()
        .is_some_and(|st| st.members.remove(&(player, channel)).is_some());
    if !existed {
        return false;
    }
    persist_del(ctx.out, member_key(player, channel));
    let name = channel_name(ctx.w, channel).unwrap_or_else(|| SmolStr::new("?"));
    ctx.out.sys_to_actor(
        player,
        "channel",
        format!("You have been removed from channel {name}."),
    );
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alias_sanitizing() {
        assert_eq!(alias_from_name("The Undertow"), "theundertow");
        assert_eq!(alias_from_name("Camarilla of Chicago!!"), "camarillaofchi");
        assert_eq!(alias_from_name("!!!"), "fac");
        assert_eq!(alias_from_name(""), "fac");
    }
}
