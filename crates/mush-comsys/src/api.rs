//! Programmatic membership API for **other modules** (cross-namespace writes
//! go through the owning module's public API — the module-layering rule).
//!
//! First consumer: `mush-faction`, which binds a channel to a faction and
//! keeps eligible members joined. These calls are module-driven: they skip
//! the join lock (the driving module owns the policy decision) but never
//! skip persistence or the membership invariants.

use mush_core::{Ctx, ObjId, World};
use smol_str::SmolStr;

use crate::data::{
    Channel, ChannelId, ComsysState, Membership, member_key, persist_del, persist_membership,
};

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
    ctx.out.sys_to_actor(
        player,
        "channel",
        format!("You have been joined to channel {name} (alias: {alias})."),
    );
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
