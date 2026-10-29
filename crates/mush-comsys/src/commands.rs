//! The TinyMUX comsys command surface: `addcom`, `delcom`, `comlist`,
//! `comtitle`, `clist`, and the `@channel/*` admin verbs.
//!
//! All handlers work purely against the [`ComsysState`] slot and mirror
//! every mutation to module ops — no object writes, no I/O.

use std::sync::Arc;

use mush_core::commands::movement;
use mush_core::lock::MAX_LOCK_DEPTH;
use mush_core::output::display_name;
use mush_core::{
    ArgShape, CmdError, CommandSpec, Ctx, LockExpr, LookupErr, ModError, ObjId, Powers, Registrar,
    Role, World, eval_lock, parse_lock, perm,
};
use smol_str::SmolStr;

use crate::data::{
    Channel, ComsysState, MAX_CHANNEL_NAME_LEN, MAX_HISTORY_CAP, MAX_TITLE_LEN, Membership,
    chan_key, hist_key, member_key, persist_channel, persist_del, persist_membership, persist_meta,
    valid_channel_name,
};
use crate::speak::{channel_notice, render_recall};

/// Longest accepted channel alias.
const MAX_ALIAS_LEN: usize = 16;

/// Default `@channel/recall` depth.
const DEFAULT_RECALL: usize = 10;

// ----------------------------------------------------------------------
// Shared helpers
// ----------------------------------------------------------------------

/// The mutable comsys state slot (installed at engine construction).
fn state<'a>(ctx: &'a mut Ctx<'_>) -> &'a mut ComsysState {
    ctx.w
        .module_state_mut::<ComsysState>()
        .expect("comsys state slot installed at registration")
}

/// A cloned snapshot of the channel with this name.
fn chan_snapshot(ctx: &Ctx<'_>, name: &str) -> Result<Channel, CmdError> {
    ctx.w
        .module_state::<ComsysState>()
        .and_then(|st| st.channel_by_name(name).cloned())
        .ok_or_else(|| CmdError::msg("No such channel."))
}

/// A cloned snapshot of the actor's membership under `alias`.
fn membership_snapshot(ctx: &Ctx<'_>, alias: &str) -> Result<Membership, CmdError> {
    ctx.w
        .module_state::<ComsysState>()
        .and_then(|st| st.membership_by_alias(ctx.actor, alias).cloned())
        .ok_or_else(|| CmdError::msg("No channel is on that alias.  (See comlist.)"))
}

/// Whether `who` may administer channel `c`: the `COMSYS_ADMIN` power
/// (Wizard+ implicitly), Royalty+, or passing the channel's admin lock.
fn chan_admin(w: &World, who: ObjId, c: &Channel) -> bool {
    perm::has_power(w, who, Powers::COMSYS_ADMIN)
        || perm::has_role(w, who, Role::Royalty)
        || eval_lock(w, &c.perms.admin, who, MAX_LOCK_DEPTH)
}

/// Whether `who` may see channel `c` (clist / who / addcom discovery):
/// the see lock, with an admin bypass so staff can administer channels
/// hidden from them. Message **delivery** never uses the bypass.
///
/// **16b cut Z.4**: on a [`crate::data::ChannelFlags::closed`] channel the
/// question is membership and not the see lock — a member sees it (its see lock
/// may be `#false`, which is what hides it from everybody else), a non-member
/// does not, whatever the lock says. The admin bypass is unchanged.
fn chan_visible(w: &World, who: ObjId, c: &Channel) -> bool {
    let seen = if c.flags.closed {
        w.module_state::<ComsysState>()
            .is_some_and(|st| st.members.contains_key(&(who, c.id)))
    } else {
        eval_lock(w, &c.perms.see, who, MAX_LOCK_DEPTH)
    };
    seen || chan_admin(w, who, c)
}

/// Whether an addcom alias would shadow something dispatch already means:
/// the `home` keyword, a registered command (exact or unambiguous prefix),
/// or an exit reachable from here. Alias stages run **before** all of
/// those (treaty ruling 5), so the channel alias wins — the warning tells
/// the player what they just hid.
fn shadows_existing(ctx: &Ctx<'_>, alias: &str) -> bool {
    if alias.eq_ignore_ascii_case("home") {
        return true;
    }
    match ctx.reg.lookup(alias) {
        Ok(_) | Err(LookupErr::Ambiguous(_)) => true,
        Err(LookupErr::NotFound) => movement::find_exit(ctx.w, ctx.actor, alias).is_some(),
    }
}

fn valid_alias(alias: &str) -> bool {
    !alias.is_empty()
        && alias.len() <= MAX_ALIAS_LEN
        && alias
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

fn valid_hex(hex: &str) -> bool {
    hex.strip_prefix('#')
        .is_some_and(|d| (d.len() == 6 || d.len() == 3) && d.chars().all(|c| c.is_ascii_hexdigit()))
}

/// Render a channel's flag letters for listings (`L`oud, `T` = no titles,
/// `H` = logged history, `P` = persisted history, `I` = in-character, `C` =
/// closed: members only, 16b cut Z.4), `-` when none.
fn flag_letters(c: &Channel) -> String {
    let mut s = String::new();
    if c.flags.loud {
        s.push('L');
    }
    if c.flags.closed {
        s.push('C');
    }
    if c.flags.ic {
        s.push('I');
    }
    if c.flags.no_titles {
        s.push('T');
    }
    if c.flags.logged {
        s.push('H');
    }
    if c.history.persist {
        s.push('P');
    }
    if s.is_empty() {
        s.push('-');
    }
    s
}

// ----------------------------------------------------------------------
// addcom / delcom / comlist / comtitle / clist
// ----------------------------------------------------------------------

fn addcom_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let alias = ctx.args.lhs().trim().to_string();
    let chan_name = ctx
        .args
        .rhs()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| CmdError::msg("Usage: addcom <alias>=<channel>"))?;
    if !valid_alias(&alias) {
        return Err(CmdError::msg(format!(
            "That is not a usable alias.  (1-{MAX_ALIAS_LEN} letters, digits, - _ .)"
        )));
    }
    let actor = ctx.actor;
    let c = chan_snapshot(ctx, &chan_name)?;
    if !chan_visible(ctx.w, actor, &c) {
        // Invisible channels are indistinguishable from missing ones.
        return Err(CmdError::msg("No such channel."));
    }
    if !eval_lock(ctx.w, &c.perms.join, actor, MAX_LOCK_DEPTH) && !chan_admin(ctx.w, actor, &c) {
        return Err(CmdError::msg(format!(
            "Channel {} does not admit you.",
            c.name
        )));
    }
    // The alias must not already point at a *different* channel.
    if let Some(other) = ctx
        .w
        .module_state::<ComsysState>()
        .and_then(|st| st.membership_by_alias(actor, &alias))
        .map(|m| m.channel)
        && other != c.id
    {
        return Err(CmdError::msg(format!(
            "You already use the alias '{alias}' for another channel."
        )));
    }
    let (mem, is_new) = {
        let st = state(ctx);
        match st.members.get_mut(&(actor, c.id)) {
            Some(m) => {
                m.alias = SmolStr::new(&alias);
                (m.clone(), false)
            }
            None => {
                if c.flags.ic
                    && st
                        .indexed_members(c.id)
                        .take(crate::transit::MAX_MEMBERS)
                        .count()
                        >= crate::transit::MAX_MEMBERS
                {
                    return Err(CmdError::msg("IC channel member limit reached."));
                }
                let m = Membership {
                    player: actor,
                    channel: c.id,
                    alias: SmolStr::new(&alias),
                    title: None,
                    gagged: false,
                    // New joiners start read-up-to-date: history before the
                    // join is never "unread".
                    last_read: c.seq,
                    ic: None,
                };
                (st.insert_member(m), true)
            }
        }
    };
    persist_membership(ctx.out, &mem);
    if is_new {
        let who = display_name(ctx.w, actor);
        channel_notice(
            ctx.w,
            ctx.out,
            c.id,
            Some(actor),
            &format!("{who} has joined this channel."),
        );
        ctx.out.info(
            actor,
            format!("Channel {} added with alias '{alias}'.", c.name),
        );
    } else {
        ctx.out.info(
            actor,
            format!("Alias for channel {} is now '{alias}'.", c.name),
        );
    }
    if shadows_existing(ctx, &alias) {
        ctx.out.info(
            actor,
            format!(
                "Warning: '{alias}' also matches a command or exit; \
                 the channel alias will win when you type it."
            ),
        );
    }
    Ok(())
}

fn delcom_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let alias = ctx.args.raw().trim().to_string();
    if alias.is_empty() {
        return Err(CmdError::msg("Usage: delcom <alias>"));
    }
    let actor = ctx.actor;
    let mem = membership_snapshot(ctx, &alias)?;
    let chan_name = chan_snapshot_by_id(ctx, mem.channel).map(|c| c.name.to_string());
    let who = display_name(ctx.w, actor);
    channel_notice(
        ctx.w,
        ctx.out,
        mem.channel,
        Some(actor),
        &format!("{who} has left this channel."),
    );
    if let Some(st) = ctx.w.module_state_mut::<ComsysState>() {
        st.remove_member(ctx.out, actor, mem.channel);
    }
    persist_del(ctx.out, member_key(actor, mem.channel));
    match chan_name {
        Some(n) => ctx
            .out
            .info(actor, format!("Channel {n} removed (alias '{alias}').")),
        None => ctx.out.info(actor, "Channel removed."),
    }
    Ok(())
}

/// A cloned channel by id (helper for paths that hold a membership).
fn chan_snapshot_by_id(ctx: &Ctx<'_>, id: crate::data::ChannelId) -> Option<Channel> {
    ctx.w
        .module_state::<ComsysState>()
        .and_then(|st| st.channels.get(&id).cloned())
}

fn comlist_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let rows: Vec<(String, String, String, &'static str)> = {
        let st = ctx
            .w
            .module_state::<ComsysState>()
            .ok_or_else(|| CmdError::msg("Comsys is not loaded."))?;
        st.memberships_of(actor)
            .map(|m| {
                let chan = st
                    .channels
                    .get(&m.channel)
                    .map_or_else(|| "(gone)".to_string(), |c| c.name.to_string());
                (
                    m.alias.to_string(),
                    chan,
                    m.title.clone().unwrap_or_default(),
                    if m.gagged { "gagged" } else { "on" },
                )
            })
            .collect()
    };
    if rows.is_empty() {
        ctx.out
            .info(actor, "You are on no channels.  (addcom <alias>=<channel>)");
        return Ok(());
    }
    ctx.out.info(
        actor,
        format!("{:<18} {:<22} {:<26} Status", "Alias", "Channel", "Title"),
    );
    let n = rows.len();
    for (alias, chan, title, status) in rows {
        ctx.out.info(
            actor,
            format!("{alias:<18} {chan:<22} {title:<26} {status}"),
        );
    }
    ctx.out.info(
        actor,
        format!("-- {n} channel{} --", if n == 1 { "" } else { "s" }),
    );
    Ok(())
}

fn comtitle_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let alias = ctx.args.lhs().trim().to_string();
    let title = ctx
        .args
        .rhs()
        .map(str::trim)
        .ok_or_else(|| CmdError::msg("Usage: comtitle <alias>=<title>  (empty title clears)"))?
        .to_string();
    if title.chars().count() > MAX_TITLE_LEN {
        return Err(CmdError::msg(format!(
            "That title is too long ({MAX_TITLE_LEN} characters max)."
        )));
    }
    let actor = ctx.actor;
    let mem = membership_snapshot(ctx, &alias)?;
    let c = chan_snapshot_by_id(ctx, mem.channel)
        .ok_or_else(|| CmdError::msg("That channel no longer exists."))?;
    if c.flags.no_titles && !title.is_empty() {
        return Err(CmdError::msg(format!("Titles are disabled on {}.", c.name)));
    }
    let updated = {
        let st = state(ctx);
        let m = st
            .members
            .get_mut(&(actor, mem.channel))
            .expect("membership snapshot was current");
        m.title = if title.is_empty() {
            None
        } else {
            Some(title.clone())
        };
        m.clone()
    };
    persist_membership(ctx.out, &updated);
    if title.is_empty() {
        ctx.out.info(actor, format!("Title on {} cleared.", c.name));
    } else {
        ctx.out
            .info(actor, format!("Title on {} set to \"{title}\".", c.name));
    }
    Ok(())
}

fn clist_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let rows: Vec<(String, usize, String, String)> = {
        let st = ctx
            .w
            .module_state::<ComsysState>()
            .ok_or_else(|| CmdError::msg("Comsys is not loaded."))?;
        let mut rows: Vec<_> = st
            .channels
            .values()
            .filter(|c| chan_visible(ctx.w, actor, c))
            .map(|c| {
                (
                    c.name.to_string(),
                    st.members_of(c.id).count(),
                    flag_letters(c),
                    c.description.clone(),
                )
            })
            .collect();
        rows.sort();
        rows
    };
    if rows.is_empty() {
        ctx.out.info(actor, "No channels are visible to you.");
        return Ok(());
    }
    ctx.out.info(
        actor,
        format!(
            "{:<22} {:>7} {:<6} Description",
            "Channel", "Members", "Flags"
        ),
    );
    let n = rows.len();
    for (name, members, flags, desc) in rows {
        ctx.out
            .info(actor, format!("{name:<22} {members:>7} {flags:<6} {desc}"));
    }
    ctx.out.info(
        actor,
        format!("-- {n} channel{} --", if n == 1 { "" } else { "s" }),
    );
    Ok(())
}

// ----------------------------------------------------------------------
// @channel/<switch>
// ----------------------------------------------------------------------

fn channel_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let switches: Vec<String> = ctx.switches.iter().map(str::to_string).collect();
    let [switch] = switches.as_slice() else {
        return Err(CmdError::msg(
            "Pick exactly one switch: @channel/create /destroy /perm /color /set \
             /who /recall /gag /ungag /list.  (See \"help @channel\".)",
        ));
    };
    match switch.as_str() {
        "create" => channel_create(ctx),
        "destroy" => channel_destroy(ctx),
        "perm" => channel_perm(ctx),
        "color" => channel_color(ctx),
        "set" => channel_set(ctx),
        "who" => channel_who(ctx),
        "recall" => channel_recall(ctx),
        "gag" => channel_gag(ctx, true),
        "ungag" => channel_gag(ctx, false),
        "list" => channel_list(ctx),
        "transit" => channel_transit(ctx),
        _ => unreachable!("dispatch rejects unknown switches"),
    }
}

fn channel_transit(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if !perm::has_role(ctx.w, ctx.actor, Role::Wizard) {
        return Err(CmdError::msg("Permission denied."));
    }
    let st = ctx
        .w
        .module_state::<ComsysState>()
        .ok_or_else(|| CmdError::msg("Comsys unavailable."))?;
    let copies: usize = st
        .ic
        .pending
        .iter()
        .filter_map(|id| st.ic.messages.get(id))
        .map(|m| m.pending.len())
        .sum();
    let oldest = st
        .ic
        .pending
        .iter()
        .filter_map(|id| st.ic.messages.get(id))
        .map(|m| m.at.at)
        .min()
        .map_or(0, |at| ctx.w.world_ic_now().at.saturating_sub(at));
    let p = &st.ic.progress;
    ctx.out.info(ctx.actor,format!("IC channel transport: {} pending lines, {copies} copies; oldest {oldest} IC seconds. Last fire visited {} copies (limit {}). Delivered {}, refused {}, expired {}.",st.ic.pending.len(),p.visited,crate::transit::MAX_COPIES,p.delivered,p.refused,p.expired));
    Ok(())
}

fn channel_create(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    if !perm::has_role(ctx.w, actor, Role::Royalty)
        && !perm::has_power(ctx.w, actor, Powers::COMSYS_ADMIN)
    {
        return Err(CmdError::msg("Permission denied."));
    }
    let name = ctx.args.lhs().trim().to_string();
    let desc = ctx.args.rhs().unwrap_or("").trim().to_string();
    if !valid_channel_name(&name) {
        return Err(CmdError::msg(format!(
            "That is not a usable channel name.  \
             (1-{MAX_CHANNEL_NAME_LEN} letters, digits, - _)"
        )));
    }
    if ctx
        .w
        .module_state::<ComsysState>()
        .is_some_and(|st| st.channel_by_name(&name).is_some())
    {
        return Err(CmdError::msg("A channel by that name already exists."));
    }
    let (c, next) = {
        let st = state(ctx);
        let id = st.alloc_chan_id();
        let mut c = Channel::new(id, &name);
        c.description = desc;
        st.channels.insert(id, c.clone());
        (c, st.next_chan_id)
    };
    persist_channel(ctx.out, &c);
    persist_meta(ctx.out, next);
    ctx.out.info(
        actor,
        format!(
            "Channel {} created (id {}).  Join it with: addcom <alias>={}",
            c.name, c.id, c.name
        ),
    );
    Ok(())
}

fn channel_destroy(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let c = chan_snapshot(ctx, ctx.args.lhs().trim())?;
    if !chan_admin(ctx.w, actor, &c) {
        return Err(CmdError::msg("Permission denied."));
    }
    let who = display_name(ctx.w, actor);
    channel_notice(
        ctx.w,
        ctx.out,
        c.id,
        Some(actor),
        &format!("Channel destroyed by {who}."),
    );
    let removed: Vec<ObjId> = {
        let st = ctx
            .w
            .module_state_mut::<ComsysState>()
            .ok_or_else(|| CmdError::msg("Comsys unavailable."))?;
        let members: Vec<ObjId> = st.members_of(c.id).map(|m| m.player).collect();
        for &p in &members {
            st.remove_member(ctx.out, p, c.id);
        }
        st.channels.remove(&c.id);
        st.rings.remove(&c.id);
        members
    };
    for p in removed {
        persist_del(ctx.out, member_key(p, c.id));
    }
    persist_del(ctx.out, chan_key(c.id));
    // Del is type-agnostic: this also drops the history stream.
    persist_del(ctx.out, hist_key(c.id));
    if let Some(st) = ctx.w.module_state_mut::<ComsysState>() {
        let ids: Vec<_> = st
            .ic
            .messages
            .range((c.id, 0)..=(c.id, u64::MAX))
            .map(|(k, _)| *k)
            .collect();
        for id in ids {
            st.ic.messages.remove(&id);
            st.ic.pending.remove(&id);
            st.ic.references.remove(&id);
            persist_del(ctx.out, format!("ic:msg:{}:{}", id.0.0, id.1).into());
        }
    }
    ctx.out
        .info(actor, format!("Channel {} destroyed.", c.name));
    Ok(())
}

fn channel_perm(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let c = chan_snapshot(ctx, ctx.args.lhs().trim())?;
    if !chan_admin(ctx.w, actor, &c) {
        return Err(CmdError::msg("Permission denied."));
    }
    let usage = "Usage: @channel/perm <channel>=<join|speak|see|admin>=<lock expression>";
    let (which, expr_src) = ctx
        .args
        .rhs()
        .and_then(|r| r.split_once('='))
        .map(|(a, b)| (a.trim().to_lowercase(), b.trim().to_string()))
        .ok_or_else(|| CmdError::msg(usage))?;
    if expr_src.is_empty() {
        return Err(CmdError::msg(usage));
    }
    let w = &*ctx.w;
    // The one gate where a *player* can be told why a lock is refused, and
    // the reason the persisted `chan:{id}` document stays inside
    // `MAX_DOC_DEPTH`: `parse_lock` bounds both the frames it spends and the
    // height of the tree it builds (`MAX_LOCK_EXPR_DEPTH`), so a flat
    // `#1&#1&…` chain thousands of terms long — which fits in one input line
    // and evades any frame counter — never becomes a `ChannelPerms` field.
    // The store's own depth refusal is only the floor under this.
    let expr: LockExpr = parse_lock(&expr_src, |name| {
        if name.eq_ignore_ascii_case("me") {
            Some(actor)
        } else {
            mush_core::match_object(w, actor, name)
                .ok()
                .or_else(|| w.resolve_player(name))
        }
    })
    .map_err(|e| CmdError::msg(format!("I don't understand that lock.  ({e})")))?;
    let updated = {
        let st = state(ctx);
        let ch = st
            .channels
            .get_mut(&c.id)
            .ok_or_else(|| CmdError::msg("No such channel."))?;
        match which.as_str() {
            "join" => ch.perms.join = expr,
            "speak" => ch.perms.speak = expr,
            "see" => ch.perms.see = expr,
            "admin" => ch.perms.admin = expr,
            _ => return Err(CmdError::msg(usage)),
        }
        ch.clone()
    };
    persist_channel(ctx.out, &updated);
    ctx.out.info(
        actor,
        format!("{} lock on {} set to: {expr_src}", which, updated.name),
    );
    Ok(())
}

fn channel_color(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let c = chan_snapshot(ctx, ctx.args.lhs().trim())?;
    if !chan_admin(ctx.w, actor, &c) {
        return Err(CmdError::msg("Permission denied."));
    }
    let usage = "Usage: @channel/color <channel>=<ansi name>,<#rrggbb>";
    let (ansi, hex) = ctx
        .args
        .rhs()
        .and_then(|r| r.split_once(','))
        .map(|(a, b)| (a.trim().to_lowercase(), b.trim().to_string()))
        .ok_or_else(|| CmdError::msg(usage))?;
    if ansi.is_empty() || ansi.chars().any(char::is_whitespace) {
        return Err(CmdError::msg(usage));
    }
    if !valid_hex(&hex) {
        return Err(CmdError::msg(
            "That is not a hex color.  (#rrggbb, e.g. #00b7c3)",
        ));
    }
    let updated = {
        let st = state(ctx);
        let ch = st
            .channels
            .get_mut(&c.id)
            .ok_or_else(|| CmdError::msg("No such channel."))?;
        ch.color.ansi = SmolStr::new(&ansi);
        ch.color.web_hex = SmolStr::new(hex.to_lowercase());
        ch.clone()
    };
    persist_channel(ctx.out, &updated);
    ctx.out.info(
        actor,
        format!(
            "Color of {} set to {} ({}).",
            updated.name, updated.color.ansi, updated.color.web_hex
        ),
    );
    Ok(())
}

fn channel_set(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let c = chan_snapshot(ctx, ctx.args.lhs().trim())?;
    if !chan_admin(ctx.w, actor, &c) {
        return Err(CmdError::msg("Permission denied."));
    }
    let usage =
        "Usage: @channel/set <channel>=[!]loud|no_titles|logged|persist|ic|relay  or  =cap=<n>";
    let rhs = ctx
        .args
        .rhs()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_lowercase)
        .ok_or_else(|| CmdError::msg(usage))?;
    let (updated, note) = {
        let st = state(ctx);
        let ch = st
            .channels
            .get_mut(&c.id)
            .ok_or_else(|| CmdError::msg("No such channel."))?;
        if let Some(n) = rhs.strip_prefix("cap=") {
            let cap: u32 = n
                .trim()
                .parse()
                .ok()
                .filter(|v| (1..=MAX_HISTORY_CAP).contains(v))
                .ok_or_else(|| CmdError::msg(format!("Cap must be 1-{MAX_HISTORY_CAP}.")))?;
            ch.history.cap = cap;
            let note = format!("History cap on {} set to {cap}.", ch.name);
            let updated = ch.clone();
            // Shrink the live ring immediately.
            if let Some(ring) = st.rings.get_mut(&c.id) {
                while ring.len() > cap as usize {
                    ring.pop_front();
                }
            }
            (updated, note)
        } else {
            let (on, flag) = match rhs.strip_prefix('!') {
                Some(f) => (false, f),
                None => (true, rhs.as_str()),
            };
            match flag {
                "loud" => ch.flags.loud = on,
                "no_titles" => ch.flags.no_titles = on,
                "logged" => ch.flags.logged = on,
                "ic" => {
                    if ch.flags.ic != on && ch.seq != 0 {
                        return Err(CmdError::msg(
                            "IC/OOC mode cannot change after a channel has carried traffic. Create a new channel.",
                        ));
                    }
                    ch.flags.ic = on;
                }
                // Programme D4: the staff's declaration that this channel's
                // lines may leave the world for the mapped Discord channel,
                // and that linked Discord users may speak on it.
                "relay" => ch.flags.relay = on,
                "persist" => ch.history.persist = on,
                _ => return Err(CmdError::msg(usage)),
            }
            let note = format!(
                "Flag {flag} {} on {}.",
                if on { "set" } else { "cleared" },
                ch.name
            );
            (ch.clone(), note)
        }
    };
    persist_channel(ctx.out, &updated);
    if updated.flags.ic
        && updated.history.cap < c.history.cap
        && let Some(st) = ctx.w.module_state_mut::<ComsysState>()
    {
        for m in st.members.values_mut().filter(|m| m.channel == c.id) {
            if let Some(k) = m.ic.as_mut() {
                while k.received.len() > updated.history.cap as usize {
                    if let Some(old) = k.received.pop_front() {
                        st.ic.release(ctx.out, (c.id, old.message));
                    }
                }
                persist_membership(ctx.out, m);
            }
        }
    }
    if c.flags.ic != updated.flags.ic
        && let Some(st) = ctx.w.module_state_mut::<ComsysState>()
    {
        for m in st.members.values_mut().filter(|m| m.channel == c.id) {
            m.ic = updated.flags.ic.then(crate::transit::Knowledge::default);
            m.last_read = 0;
            persist_membership(ctx.out, m);
        }
    }
    ctx.out.info(actor, note);
    Ok(())
}

fn channel_who(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let c = chan_snapshot(ctx, ctx.args.lhs().trim())?;
    if !chan_visible(ctx.w, actor, &c) {
        return Err(CmdError::msg("No such channel."));
    }
    let mut rows: Vec<String> = {
        let st = ctx
            .w
            .module_state::<ComsysState>()
            .ok_or_else(|| CmdError::msg("Comsys is not loaded."))?;
        st.members_of(c.id)
            .map(|m| {
                let mut line = format!("  {}", display_name(ctx.w, m.player));
                if ctx.w.is_connected(m.player) {
                    line.push_str(" [connected]");
                }
                if m.gagged {
                    line.push_str(" [gagged]");
                }
                line
            })
            .collect()
    };
    rows.sort();
    ctx.out.info(
        actor,
        format!(
            "-- Channel {}: {} member{} --",
            c.name,
            rows.len(),
            if rows.len() == 1 { "" } else { "s" }
        ),
    );
    for r in rows {
        ctx.out.info(actor, r);
    }
    Ok(())
}

fn channel_recall(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let alias = ctx.args.lhs().trim().to_string();
    if alias.is_empty() {
        return Err(CmdError::msg("Usage: @channel/recall <alias>[=<count>]"));
    }
    let mem = membership_snapshot(ctx, &alias)?;
    let c = chan_snapshot_by_id(ctx, mem.channel)
        .ok_or_else(|| CmdError::msg("That channel no longer exists."))?;
    // Recall is delivery: the see lock gates it exactly as it gates live
    // messages, with NO admin bypass — a member barred from hearing the
    // channel must not read its retained history either. (16b cut Z.4: through
    // `Channel::hears`, so a closed channel's members pass it.)
    if !c.hears(ctx.w, actor) {
        return Err(CmdError::msg(format!("You cannot listen to {}.", c.name)));
    }
    if !c.flags.logged {
        return Err(CmdError::msg(format!(
            "{} does not log history.  (LOGGED is off.)",
            c.name
        )));
    }
    let n: usize = match ctx.args.rhs() {
        None => DEFAULT_RECALL,
        Some(r) => r
            .trim()
            .parse()
            .ok()
            .filter(|&v| v >= 1)
            .ok_or_else(|| CmdError::msg("Recall how many?  (a positive number)"))?,
    };
    let lines: Vec<String> = {
        let st = ctx
            .w
            .module_state::<ComsysState>()
            .ok_or_else(|| CmdError::msg("Comsys is not loaded."))?;
        let ring = crate::transit::history(st, &mem);
        {
            let skip = ring.len().saturating_sub(n);
            ring.iter()
                .skip(skip)
                .map(|m| format!("[{}] {}", c.name, render_recall(m)))
                .collect()
        }
    };
    if lines.is_empty() {
        ctx.out
            .info(actor, format!("No messages recorded on {}.", c.name));
        return Ok(());
    }
    ctx.out.info(
        actor,
        format!(
            "-- {}: last {} message{} --",
            c.name,
            lines.len(),
            if lines.len() == 1 { "" } else { "s" }
        ),
    );
    for l in lines {
        ctx.out.sys_to_actor(actor, "channel", l);
    }
    ctx.out.info(actor, "-- End of recall --");
    Ok(())
}

fn channel_gag(ctx: &mut Ctx<'_>, gag: bool) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let alias = ctx.args.lhs().trim().to_string();
    if alias.is_empty() {
        return Err(CmdError::msg(if gag {
            "Usage: @channel/gag <alias>"
        } else {
            "Usage: @channel/ungag <alias>"
        }));
    }
    let mem = membership_snapshot(ctx, &alias)?;
    let chan_name = chan_snapshot_by_id(ctx, mem.channel)
        .map_or_else(|| "that channel".to_string(), |c| c.name.to_string());
    if mem.gagged == gag {
        ctx.out.info(
            actor,
            format!(
                "{chan_name} is already {}.",
                if gag { "gagged" } else { "ungagged" }
            ),
        );
        return Ok(());
    }
    let updated = {
        let st = state(ctx);
        let m = st
            .members
            .get_mut(&(actor, mem.channel))
            .expect("membership snapshot was current");
        m.gagged = gag;
        m.clone()
    };
    persist_membership(ctx.out, &updated);
    if gag {
        ctx.out.info(
            actor,
            format!("You gag {chan_name}.  You will hear nothing until you ungag it."),
        );
    } else {
        ctx.out.info(actor, format!("You ungag {chan_name}."));
    }
    Ok(())
}

fn channel_list(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    if !perm::has_role(ctx.w, actor, Role::Royalty)
        && !perm::has_power(ctx.w, actor, Powers::COMSYS_ADMIN)
    {
        return Err(CmdError::msg("Permission denied."));
    }
    let rows: Vec<String> = {
        let st = ctx
            .w
            .module_state::<ComsysState>()
            .ok_or_else(|| CmdError::msg("Comsys is not loaded."))?;
        st.channels
            .values()
            .map(|c| {
                format!(
                    "{:<4} {:<22} {:<6} seq {:<8} cap {:<6} {:<7} {:<9} \
                     join={} speak={} see={} admin={}",
                    c.id,
                    c.name,
                    flag_letters(c),
                    c.seq,
                    c.history.cap,
                    c.color.ansi,
                    c.color.web_hex,
                    c.perms.join,
                    c.perms.speak,
                    c.perms.see,
                    c.perms.admin
                )
            })
            .collect()
    };
    let n = rows.len();
    ctx.out.info(actor, "-- All channels --".to_string());
    for r in rows {
        ctx.out.info(actor, r);
    }
    ctx.out.info(
        actor,
        format!("-- {n} channel{} --", if n == 1 { "" } else { "s" }),
    );
    Ok(())
}

// ----------------------------------------------------------------------
// Registration
// ----------------------------------------------------------------------

/// Register the comsys command surface (called from
/// `ComsysModule::register`).
pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.command(
        CommandSpec {
            name: "addcom".into(),
            shape: ArgShape::EqPair,
            help: "addcom <alias>=<channel>  --  Join a channel under a personal \
                   alias; the alias becomes your speaking verb (addcom pub=Public, \
                   then: pub Hello / pub :waves / pub ;grins).  Joining warns you \
                   when the alias shadows a command or an exit (the alias wins).  \
                   Using addcom again with a new alias re-aliases a channel you \
                   are already on."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(addcom_cmd),
    )?;
    reg.command(
        CommandSpec {
            name: "delcom".into(),
            help: "delcom <alias>  --  Leave the channel you joined under that \
                   alias.  Your title and gag state are forgotten with it."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(delcom_cmd),
    )?;
    reg.classify_commands(mush_core::CommandCompatibility::ReadOnly);
    reg.command(
        CommandSpec {
            name: "comlist".into(),
            shape: ArgShape::None,
            help: "comlist  --  List every channel you are on: alias, channel, \
                   comtitle, and whether you have it gagged."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(comlist_cmd),
    )?;
    reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
    reg.command(
        CommandSpec {
            name: "comtitle".into(),
            shape: ArgShape::EqPair,
            help: "comtitle <alias>=<title>  --  Set the title shown before your \
                   name on that channel (comtitle pub=the Bold).  An empty title \
                   clears it.  Channels flagged NO_TITLES refuse titles."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(comtitle_cmd),
    )?;
    reg.classify_commands(mush_core::CommandCompatibility::ReadOnly);
    reg.command(
        CommandSpec {
            name: "clist".into(),
            shape: ArgShape::None,
            help: "clist  --  List the channels you are allowed to see (their see \
                   lock), with member counts, flags (L=loud, C=closed: members \
                   only, I=in-character, T=no titles, H=history, P=persisted), and \
                   descriptions.  A closed channel is listed only to its members."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(clist_cmd),
    )?;
    reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
    reg.command(
        CommandSpec {
            name: "@channel".into(),
            aliases: vec!["@chan".into()],
            switches: vec![
                "create".into(),
                "destroy".into(),
                "perm".into(),
                "color".into(),
                "set".into(),
                "who".into(),
                "recall".into(),
                "gag".into(),
                "ungag".into(),
                "list".into(),
                "transit".into(),
            ],
            shape: ArgShape::EqPair,
            help: "@channel/<switch>  --  Channel administration and utilities.\n\
                   @channel/create <name>[=<description>] -- create a channel \
                   (Royalty+ or the COMSYS_ADMIN power).\n\
                   @channel/destroy <channel> -- destroy it (channel admins).\n\
                   @channel/perm <channel>=<join|speak|see|admin>=<lock> -- set a \
                   lock gate, classic lock syntax (flag^WIZARD, *Nick, #1, ...).\n\
                   @channel/color <channel>=<ansi name>,<#rrggbb> -- set colors.\n\
                   @channel/set <channel>=[!]loud|no_titles|logged|persist|ic|relay \
                   or =cap=<n> -- flags and history policy; `ic` marks the \
                   channel in-character: lines queue for physical transit; only \
                   arrived lines appear in recall, unread and reconnect. `relay` \
                   enables linked Discord ingress; shared mirrors are OOC only \
                   (see `help discord`).\n\
                   @channel/who <channel> -- list members.\n\
                   @channel/recall <alias>[=<n>] -- replay logged history.\n\
                   @channel/gag <alias>, @channel/ungag <alias> -- stop/resume \
                   hearing a channel without leaving it (you may not speak while \
                   gagged).\n\
                   @channel/list -- every channel with full detail (staff)."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(channel_cmd),
    )?;
    Ok(())
}
