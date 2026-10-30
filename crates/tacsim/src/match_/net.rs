//! **The side nets** (16b cut Z.4; 16c `N8`; doc 16 §1, §3.7 "Boarding", §5).
//!
//! Each side's pilots share one channel for the match, `RedNet` or `BlueNet`,
//! and on it each of them is their **callsign**: the alias they speak with
//! (`ra contact east`) and the title the line carries (`[RedNet] RA Ada: contact
//! east`). That is what `help cockpit` has promised since cut Q.1 — *a callsign
//! on the radio is a person you can look up* — and until this cut there was no
//! radio.
//!
//! # It is comsys, and nothing else
//!
//! The nets are ordinary comsys channels, made and joined through comsys's own
//! module API (`mush_comsys::api`), so the line is comsys's line on the clients'
//! own channel screens and neither client changes. This module writes **nothing
//! to a net**: no position, no section state, no contact, no order. The only
//! writer is a pilot's own line, which is the whole of the lead's ruling for N8,
//! and the owner's three choices ride on it as they stand — the nets are not
//! jammed by ECM, the alias is the callsign, and there is no channel across the
//! sides.
//!
//! # Why a net is **closed**
//!
//! Comsys visibility is the see lock for everybody, and delivery consults the same
//! lock per member. An open net would let a blue pilot `clist` it mid-match and
//! read its member count, or `@channel/who RedNet` and read which red pilot
//! climbed out (`EXIT` leaves the net) or dropped (`[connected]` goes) — which is
//! exactly the fact 16b cut D.1 took out of `SIM STATUS`: *they learned that the
//! human on `RA` had dropped and a machine had the tank, which no sensor of theirs
//! told them*. So each net is made **closed** (`ChannelFlags::closed`, members
//! only) with its see lock shut: it does not exist for the other side on any
//! comsys surface, and its members are delivered to regardless.
//!
//! # Who is on it, and when
//!
//! - **Joined** in the lobby's check-out arm (`lobby::run`, after
//!   [`crate::match_::start`] answered `Ok`), for every human pilot the centre's
//!   slots hold — the `Boarding → Running` trigger, beside `Good hunting`, and not
//!   `SIM START`. A staff `@arena/start` shares `start` and never passes through
//!   the lobby, so it joins nobody; machines have no pilot and never join.
//! - **Left** in [`crate::match_::cockpit::hand_seat_back`], the one way a
//!   cockpit seat is let go: `EXIT`, a seated `SIM LEAVE`, the check-in, and the
//!   boot's sweep. A drop past the link-dead grace does not hand the seat back
//!   and does not leave, so the reclaim needs no rejoin.
//! - **Swept** at every join: whoever is on a side's net and not on that side's
//!   roster is taken off first — a member some missed path left behind, or the
//!   last match's alias — so membership repairs itself at the next check-out
//!   (comsys's forced join leaves an existing member exactly as it is, and an
//!   admin's `addcom` passes the shut join lock).
//!
//! With comsys not composed every function here answers `None` or `false` and
//! does nothing: the layer works without a radio.

use mush_comsys::{ChannelFlags, ChannelId, ChannelPerms};
use mush_core::{Ctx, LockExpr, ObjId, World};

use crate::arena::state::PartyId;

/// The red side's net.
pub const RED_NET: &str = "RedNet";

/// The blue side's net.
pub const BLUE_NET: &str = "BlueNet";

/// The name of `team`'s net.
#[must_use]
pub const fn net_name(team: PartyId) -> &'static str {
    match team {
        PartyId::RED => RED_NET,
        _ => BLUE_NET,
    }
}

/// The description `clist` prints beside a net (to its members; nobody else is
/// shown the net at all).
fn description(team: PartyId) -> String {
    format!(
        "The {} side's net for a match in the simulator; joined with your pod and left with it.",
        team.as_str()
    )
}

/// **The locks a net is made with**, re-asserted at every boot.
///
/// - `join` shut: nobody `addcom`s onto a net. The lobby's forced join passes it
///   by design — comsys's API skips the join lock because the driving module owns
///   the policy — and so does an admin's `addcom`, which the sweep answers.
/// - `see` shut: with the channel closed this is what keeps the other side from
///   listing it, counting it, or reading who is on it and who is connected.
/// - `speak` open: a member may speak, and only a member has the alias.
/// - `admin` shut: staff still pass on their power, as on every channel.
fn perms() -> ChannelPerms {
    ChannelPerms {
        join: LockExpr::Const(false),
        speak: LockExpr::Const(true),
        see: LockExpr::Const(false),
        admin: LockExpr::Const(false),
    }
}

/// **The flags a net is made with**: closed, and nothing else — not loud (a net
/// counts nothing at login), titles carried (`no_titles` off: the callsign is
/// the title, and a `no_titles` net would print `[RedNet] Ada: …` without it),
/// not logged (a match's traffic is not recalled after it), not in-character,
/// never relayed off the world.
const fn flags() -> ChannelFlags {
    ChannelFlags {
        loud: false,
        no_titles: false,
        logged: false,
        ic: false,
        relay: false,
        closed: true,
    }
}

/// **Both nets, made once** (16b cut Z.4), from the module's `on_all_loaded`
/// beside the boot's pod sweep. Comsys is composed first, so its state is loaded
/// by then.
///
/// On a world that already holds them their perms and flags are re-asserted — a
/// join lock a staffer opened with `@channel/perm` is shut again, a recoloured net
/// keeps its colour — and nothing is made twice. `(None, None)` without comsys.
pub(crate) fn ensure_nets(ctx: &mut Ctx<'_>) -> (Option<ChannelId>, Option<ChannelId>) {
    let [red, blue] = PartyId::OOC.map(|team| ensure_net(ctx, team));
    (red, blue)
}

/// One side's net, through `mush_comsys::api::ensure_channel`.
fn ensure_net(ctx: &mut Ctx<'_>, team: PartyId) -> Option<ChannelId> {
    mush_comsys::api::ensure_channel(ctx, net_name(team), &description(team), flags(), perms())
}

/// The channel id of `team`'s net, if the world holds one.
#[must_use]
pub(crate) fn net_of(w: &World, team: PartyId) -> Option<ChannelId> {
    mush_comsys::api::channel_id_by_name(w, net_name(team))
}

/// **Take everybody off `team`'s net who is not on `roster`**, answering how
/// many went. Each hears comsys's own removal line once.
///
/// [`join_pilot`] runs it first; the lobby's check-out also runs it alone for a
/// side with no human pilot, so a machine-only side's net is emptied too.
pub(crate) fn sweep(ctx: &mut Ctx<'_>, team: PartyId, roster: &[ObjId]) -> usize {
    let Some(net) = net_of(ctx.w, team) else {
        return 0;
    };
    let mut swept = 0;
    for member in mush_comsys::api::members_of(ctx.w, net) {
        if !roster.contains(&member) && mush_comsys::api::force_leave(ctx, member, net) {
            swept += 1;
        }
    }
    swept
}

/// **Put one pilot on their side's net under their callsign** (16b cut Z.4).
///
/// Every member of the net not on `roster` is taken off first ([`sweep`]); then
/// the pilot is joined with the callsign lower-cased as their alias (`ra`) and
/// the callsign as their title (`RA`). They read comsys's own line, `You have
/// been joined to channel RedNet (alias: ra).` — or, when `ra` already names
/// another channel of theirs, comsys's numbered variant with both said: `You have
/// been joined to channel RedNet (alias: ra2; ra is your alias for Public).`
///
/// A pilot already on the net under **another** alias (a membership some missed
/// path kept from an earlier slot) is taken off and joined again, because the
/// forced join leaves an existing member exactly as it is and the alias is the
/// callsign; one already on it under this alias keeps the membership and has the
/// title restated.
///
/// Answers whether the pilot is on the net with the title set; `false` without
/// comsys, without the net, or when comsys refused.
pub(crate) fn join_pilot(
    ctx: &mut Ctx<'_>,
    pilot: ObjId,
    team: PartyId,
    callsign: &str,
    roster: &[ObjId],
) -> bool {
    let Some(net) = net_of(ctx.w, team) else {
        return false;
    };
    sweep(ctx, team, roster);
    let alias = callsign.to_ascii_lowercase();
    let stale = ctx
        .w
        .module_state::<mush_comsys::ComsysState>()
        .and_then(|st| st.members.get(&(pilot, net)))
        .is_some_and(|member| !member.alias.eq_ignore_ascii_case(&alias));
    if stale {
        mush_comsys::api::force_leave(ctx, pilot, net);
    }
    mush_comsys::api::force_join(ctx, pilot, net, &alias)
        && mush_comsys::api::set_title(ctx, pilot, net, Some(callsign))
}

/// **Take one pilot off both nets** (16b cut Z.4), from
/// [`crate::match_::cockpit::hand_seat_back`]. Idempotent: a pilot on neither
/// net reads nothing and nothing is written; one on a net reads comsys's own
/// `You have been removed from channel RedNet.` once.
///
/// Answers whether they were on either.
pub(crate) fn leave_pilot(ctx: &mut Ctx<'_>, pilot: ObjId) -> bool {
    let mut left = false;
    for team in PartyId::OOC {
        if let Some(net) = net_of(ctx.w, team) {
            left |= mush_comsys::api::force_leave(ctx, pilot, net);
        }
    }
    left
}
