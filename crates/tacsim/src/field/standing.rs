//! **Standing at demotion, and the leader's void** (doc 36 cut GC.10; Q-36-11, F-2).
//!
//! Hostility in a field is temporal (Q-CF-8): it lives in the kernel's books and ends with the
//! field. Standing is where a lasting consequence of fire belongs, and fire is evidence where a
//! private mark is not, so a mark alone never moves standing.
//!
//! - **What counts.** Each round that reaches a unit of another party is fire that party observed
//!   being fired at it: the kernel's own reading (`Relations::note_fire` runs on the round's
//!   arrival, hit or miss). The host counts the `impact` lines of the field's black box as they
//!   pass (`note_chunk`), per shooter party and victim party. Fire within a party counts
//!   nothing.
//! - **At demotion** (`at_demotion`), each pair that counted adjusts the standing, with every
//!   faction the victim party's principals openly stand in ([`mush_faction::open_factions`]), of
//!   each of the shooter party's principals (its leader and, for a roster, its organizer;
//!   `parties::principals`): [`STANDING_PER_ROUND`] a round, bounded at
//!   [`STANDING_FIELD_CAP`] a pair a field. A raider's organization (doc 36 cut GC.11) is such a
//!   faction, its leader the principal, so raider fire costs the raider standing with its
//!   victims' factions and fire at raiders costs the shooter standing with the raider's.
//! - **The void** (the owner's amendment of Q-36-11). Where the victim party was a standing
//!   friend of the shooter's (by owner or organization, `parties::friends_of`), the fire
//!   was friendly fire: the change is written naming the shooter party's leader as the one who
//!   may void it, and the leader is offered the void at demotion. `+field/void` cancels every
//!   open change naming the leader. The offer lasts until the leader's next command or
//!   [`VOID_WINDOW_IC_S`] of IC time, whichever is first. The change and its void are written
//!   together in `mush-faction`'s receipts: the grant names who may void it and carries its
//!   window (doc 40 cut FR.2, Q-40-16), and a void is a record of the same cause beside it.
//! - **A director raid's friendly fire** (doc 40 cut FR.2, Q-40-18): when the shooter party is a
//!   raider the raid director landed (`FieldParty::director`), its leader voids it in the
//!   demotion's own batch, with [`DIRECTOR_VOID_REASON`]; nobody is offered it. A staff-seeded
//!   raider's leader is offered the void as any leader is, and nothing takes it for it.
//! - **A void end** (a panic, a stall, a ledger overflow) commits nothing the arena derived, so it
//!   writes no standing either.
//!
//! **What is durable** (doc 40 cut FR.2; Q-38-24, Q-40-15..17). The tally rides the live record
//! ([`super::FieldLive::tally`]), written at the snapshot cadence: a restart resumes from the
//! snapshot with the rounds it had counted there and counts the pulses it re-fights again. The
//! offer rides the grant, so it outlives the field and a restart. "The leader's next command" is
//! read in the command path: `+field/void` refuses an offer when the leader's input before this
//! one (mush-core's `SessionStat::prev_input`) came after the offer opened. No sweep runs. The
//! stamps are wall seconds, so a command typed in the demotion's own second is not seen as after
//! it; and a reconnect seeds the stamp afresh, so a command typed before a drop is forgiven.
//!
//! **A leader who is not there** (doc 45 cut AP.2; Q-45-20). Ten IC minutes is twenty-five real
//! seconds at the game's rate, so a leader who was not connected when the field demoted lost the
//! offer before they could answer it. For a player leader who is not on watch at the demotion
//! (not connected, or linkdead) the grant's window is written to end [`VOID_ABSENT_IC_S`] (28 IC
//! days) after the demotion instead, and the offer **runs from their return**: on the
//! connection or the revival `LeaderReturns` tells them what waits ([`VOID_WAITING`]) and
//! stamps the IC second, and `+field/void` takes the offer until their next command or
//! [`VOID_WINDOW_IC_S`] after that stamp, whichever is first, and never past the 28 IC days.
//! A leader who lets it lapse and returns again inside the 28 IC days is offered it again. For a
//! connected leader nothing changes: the same window, the same lines. An NPC leader is never
//! waited for. Standing itself moves exactly as before, whoever is connected.
//!
//! A waiting offer is told from an ordinary one by what is left of it: an ordinary offer never
//! has more than [`VOID_WINDOW_IC_S`] to run, so a window with more is waiting on its leader
//! (`waiting`). Inside the last ten IC minutes of the 28 IC days a waiting offer is an ordinary
//! one, open until its end or the leader's next command. The return stamps are RAM
//! (`Returned`): a restart ends every session, and the next connection stamps again.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::{CanonicalId, Ctx, EventHook, ObjId, World};

use super::{FieldBook, FireRow};
use crate::arena::PartyId;
use crate::arena::ledger::{EventKind, LedgerEvent};
use crate::arena_task::MatchId;

/// **Standing a round costs** (provisional): one point of the -100..100 scale.
pub const STANDING_PER_ROUND: i16 = -1;
/// **The most one pair of parties' fire costs in one field** (provisional): ten points, so a long
/// exchange is bounded and one mistake never makes a faction hostile by itself.
pub const STANDING_FIELD_CAP: i16 = -10;
/// **How long a leader may void friendly fire's standing** (provisional): ten IC minutes, unless
/// the leader's next command comes first.
pub const VOID_WINDOW_IC_S: i64 = 600;
/// **How long a void waits for a leader who was not connected at the demotion** (doc 45 cut
/// AP.2; Q-45-20; the owner's rule that a wait on a player's response is at least 28 IC days):
/// 28 IC days from the demotion, taken or not.
pub const VOID_ABSENT_IC_S: i64 = 28 * 24 * 3_600;

/// **The change, told to each principal it moved** (`{n}` the field's ordinal, `{site}` its
/// name, `{party}` the victim's letter, `{delta}` the change, `{factions}` the factions).
pub const STANDING_CHANGED: &str = "Your party's fire on party {party} in field {n} at {site} \
moves your standing with {factions} by {delta}.";
/// **The void's offer**, told to the shooter party's leader.
pub const VOID_OFFERED: &str = "Party {party} was a friend of yours: +field/void cancels the \
standing your party's fire on it cost, until your next command or ten IC minutes.";
/// **The offer that waited** (doc 45 cut AP.2), told to a leader on their return (`{n}` how many
/// changes wait).
pub const VOID_WAITING: &str = "While you were away your party's fire on a friend cost \
standing: {n} change(s) wait on you. +field/void cancels them, until your next command or ten \
IC minutes.";
/// **The void**, told to the leader.
pub const VOIDED: &str =
    "Voided as friendly fire: {n} standing change(s) your party's fire caused.";
/// Nothing to void.
pub const NOTHING_TO_VOID: &str = "Nothing your party's fire cost is yours to void now.";
/// The reason every void records.
const VOID_REASON: &str = "Voided by the party's leader as friendly fire.";
/// **The reason a director raid's void records** (doc 40 cut FR.2, Q-40-18).
pub const DIRECTOR_VOID_REASON: &str =
    "Voided by the raid's leader as friendly fire, at the field's demotion.";

/// A site's tally as the live record's rows ([`super::FieldLive::tally`]), in pair order.
#[must_use]
pub(crate) fn rows(counts: Option<&BTreeMap<(PartyId, PartyId), u32>>) -> Vec<FireRow> {
    counts
        .into_iter()
        .flatten()
        .map(|((shooter, victim), rounds)| FireRow {
            shooter: *shooter,
            victim: *victim,
            rounds: *rounds,
        })
        .collect()
}

/// A site's tally from the live record's rows (a resume's seed).
#[must_use]
pub(crate) fn seed(rows: &[FireRow]) -> BTreeMap<(PartyId, PartyId), u32> {
    let mut counts = BTreeMap::new();
    for row in rows {
        let count: &mut u32 = counts.entry((row.shooter, row.victim)).or_default();
        *count = count.saturating_add(row.rounds);
    }
    counts
}

/// **A chunk of a field's black box passed** (the bridge's ledger branch): count each round that
/// reached another party's unit. Any other task's chunk is not a field's and is ignored. The count
/// is RAM until the next snapshot lays it on the live record (`super::on_snapshot`).
pub(crate) fn note_chunk(ctx: &mut Ctx<'_>, task: MatchId, events: &[LedgerEvent]) {
    let Some((site, pairs)) = ctx.w.module_state::<FieldBook>().and_then(|book| {
        let site = book.site_of_task(task)?.clone();
        let live = &book.hosted(&site)?.live;
        let pairs: Vec<(PartyId, PartyId)> = events
            .iter()
            .filter_map(|event| match &event.kind {
                EventKind::Impact { unit, target, .. } => {
                    Some((live.party_of(*unit)?, live.party_of(*target)?))
                }
                _ => None,
            })
            .filter(|(shooter, victim)| shooter != victim)
            .collect();
        Some((site, pairs))
    }) else {
        return;
    };
    if pairs.is_empty() {
        return;
    }
    if let Some(book) = ctx.w.module_state_mut::<FieldBook>() {
        let counts = book.tally.entry(site).or_default();
        for pair in pairs {
            let count = counts.entry(pair).or_default();
            *count = count.saturating_add(1);
        }
    }
}

/// **The change one pair's rounds make** ([`STANDING_PER_ROUND`] each, bounded at
/// [`STANDING_FIELD_CAP`]).
#[must_use]
pub(crate) fn delta_for(rounds: u32) -> i16 {
    let rounds = i64::from(rounds);
    let delta = (rounds * i64::from(STANDING_PER_ROUND)).max(i64::from(STANDING_FIELD_CAP));
    i16::try_from(delta).unwrap_or(STANDING_FIELD_CAP)
}

/// **The receipts' prefix for one field** (`field:<site tag>:<ordinal>:`): the site's id is
/// folded to a tag, since a cause is lowercase and bounded.
#[must_use]
pub fn cause_prefix(site: &CanonicalId, ordinal: u32) -> String {
    // FNV-1a: stable across builds and platforms.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in site.as_str().bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("field:{hash:016x}:{ordinal}:")
}

/// A live character, the only kind a standing receipt names.
fn live(w: &World, who: ObjId) -> bool {
    w.get(who).is_some_and(|object| {
        object.kind.is_actor() && !object.flags.contains(mush_core::Flags::GOING)
    })
}

/// **At demotion** (this module's head): write each pair's standing, offer the void of friendly
/// fire to its leader (or void it, for a director raid), and forget the site's tally. Called by
/// `close::on_ended` while the host still holds the field. `void` is a void end, which writes
/// nothing.
pub(crate) fn at_demotion(ctx: &mut Ctx<'_>, site: &CanonicalId, void: bool) {
    let counts = ctx
        .w
        .module_state_mut::<FieldBook>()
        .and_then(|book| book.tally.remove(site))
        .unwrap_or_default();
    if void || counts.is_empty() {
        return;
    }
    let Some((parties, ordinal)) = ctx
        .w
        .module_state::<FieldBook>()
        .and_then(|book| book.hosted(site))
        .map(|hosted| (hosted.live.parties.clone(), hosted.live.field.ordinal))
    else {
        return;
    };
    let friends: BTreeSet<(PartyId, PartyId)> = super::parties::friends_of(ctx.w, &parties)
        .into_iter()
        .collect();
    let prefix = cause_prefix(site, ordinal);
    let site_name = crate::surface::act::site_name(ctx.w, site);
    let party = |id: PartyId| parties.iter().find(|row| row.party == id);
    // Doc 40 cut FR.2 (Q-40-16): the window rides each grant, written once with it. Doc 45 cut
    // AP.2 (Q-45-20): for a leader who is not there it ends 28 IC days on, and the offer runs
    // from their return.
    let opened_at = ctx.w.now();
    let now_ic = ctx.w.world_ic_now().at;
    let window_for = |w: &World, leader: ObjId| mush_faction::VoidWindow {
        opened_at,
        until_ic: now_ic.saturating_add(if absent(w, leader) {
            VOID_ABSENT_IC_S
        } else {
            VOID_WINDOW_IC_S
        }),
    };
    for ((shooter, victim), rounds) in counts {
        let (Some(by), Some(at)) = (party(shooter), party(victim)) else {
            continue;
        };
        let factions: BTreeSet<mush_faction::FactionId> = super::parties::principals(at)
            .into_iter()
            .flat_map(|who| mush_faction::open_factions(ctx.w, who))
            .collect();
        let effects: Vec<mush_faction::StandingEffect> = factions
            .iter()
            .take(8)
            .map(|faction| mush_faction::StandingEffect {
                faction: *faction,
                delta: delta_for(rounds),
            })
            .collect();
        if effects.is_empty() {
            continue;
        }
        let friendly = friends.contains(&(shooter.min(victim), shooter.max(victim)));
        let voidable_by = by.leader.filter(|leader| friendly && live(ctx.w, *leader));
        let window = voidable_by.map(|leader| window_for(ctx.w, leader));
        let names = faction_names(ctx.w, &effects);
        let reason = format!(
            "Fire by party {} on party {} in field {ordinal} at {site_name}.",
            shooter.letter(),
            victim.letter()
        );
        let mut voidable = Vec::new();
        let subjects: Vec<ObjId> = super::parties::principals(by)
            .into_iter()
            .filter(|who| live(ctx.w, *who))
            .collect();
        for subject in subjects {
            let cause = format!(
                "{prefix}{}{}:{}",
                shooter.letter().to_ascii_lowercase(),
                victim.letter().to_ascii_lowercase(),
                subject.0
            );
            match mush_faction::record_voidable_standing(
                ctx,
                &cause,
                subject,
                &effects,
                &reason,
                voidable_by,
                window,
            ) {
                Ok(_) => {
                    let line = STANDING_CHANGED
                        .replace("{party}", &victim.letter().to_string())
                        .replace("{n}", &ordinal.to_string())
                        .replace("{site}", &site_name)
                        .replace("{factions}", &names)
                        .replace("{delta}", &delta_for(rounds).to_string());
                    ctx.out.sys_to_actor(subject, "tacsim.field", line);
                    if voidable_by.is_some() {
                        voidable.push(cause);
                    }
                }
                Err(error) => {
                    tracing::warn!(site = %site, cause, %error, "tacsim: a field's standing change was refused");
                }
            }
        }
        let Some(leader) = voidable_by.filter(|_| !voidable.is_empty()) else {
            continue;
        };
        if by.director {
            // Q-40-18: a director raid voids its own friendly fire; nobody is offered it.
            for cause in &voidable {
                if let Err(error) =
                    mush_faction::void_standing(ctx, cause, leader, DIRECTOR_VOID_REASON)
                {
                    tracing::warn!(site = %site, cause, %error, "tacsim: a director raid's friendly fire could not be voided");
                }
            }
            continue;
        }
        let line = VOID_OFFERED.replace("{party}", &victim.letter().to_string());
        ctx.out.sys_to_actor(leader, "tacsim.field", line);
    }
}

/// The factions' names, joined for a line.
fn faction_names(w: &World, effects: &[mush_faction::StandingEffect]) -> String {
    let state = w.module_state::<mush_faction::FactionState>();
    effects
        .iter()
        .map(|effect| {
            state
                .and_then(|state| state.factions.get(&effect.faction))
                .map_or_else(
                    || format!("faction {}", effect.faction.0),
                    |faction| faction.name.to_string(),
                )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// **Whether a grant's window is open now** (doc 40 cut FR.2, Q-40-17): inside its IC end, and
/// nothing entered since it opened but the command reading it. `prev_input` is the voider's
/// input before this one (`None` with no session, as an NPC's, or no input yet).
fn open(now_ic: i64, prev_input: Option<i64>, window: mush_faction::VoidWindow) -> bool {
    now_ic <= window.until_ic && prev_input.is_none_or(|prev| prev <= window.opened_at)
}

/// **Whether `window` is an offer waiting on its leader at `now_ic`** (doc 45 cut AP.2; this
/// module's head): still inside its end, with more left than an ordinary offer ever has.
#[must_use]
fn waiting(now_ic: i64, window: mush_faction::VoidWindow) -> bool {
    window.until_ic.saturating_sub(now_ic) > VOID_WINDOW_IC_S
}

/// **Whether a waiting offer is open to a leader who returned at `returned`** (doc 45 cut AP.2):
/// [`VOID_WINDOW_IC_S`] from their return. An ordinary offer asks nothing here, and a leader no
/// stamp was taken for is not refused on its account.
#[must_use]
fn within_return(now_ic: i64, returned: Option<i64>, window: mush_faction::VoidWindow) -> bool {
    !waiting(now_ic, window)
        || returned.is_none_or(|at| now_ic <= at.saturating_add(VOID_WINDOW_IC_S))
}

/// **Whether the field waits for `leader`** (doc 45 cut AP.2; Q-45-20): a player who is not on
/// watch, which is not connected or linkdead. An NPC is never waited for.
#[must_use]
fn absent(w: &World, leader: ObjId) -> bool {
    w.get(leader)
        .is_some_and(|object| object.kind == mush_core::ObjKind::Player)
        && !crate::port_tick::on_watch(w, leader)
}

/// **When each leader with a waiting offer last returned** (doc 45 cut AP.2), as an IC second.
/// RAM: a restart ends every session, and the next connection stamps again.
#[derive(Clone, Debug, Default)]
pub(crate) struct Returned(BTreeMap<ObjId, i64>);

/// **A leader returns** (doc 45 cut AP.2; Q-45-20): on a connection or a revival, a leader with
/// offers that waited is told so and the offer's ten IC minutes start.
#[derive(Debug)]
pub(crate) struct LeaderReturns;

impl LeaderReturns {
    fn returned(ctx: &mut Ctx<'_>, actor: ObjId) {
        let now_ic = ctx.w.world_ic_now().at;
        let waits = mush_faction::voidable_causes(ctx.w, "field:", actor)
            .into_iter()
            .filter(|(_, window)| window.is_some_and(|window| waiting(now_ic, window)))
            .count();
        if waits == 0 {
            return;
        }
        if let Some(book) = ctx.w.module_state_mut::<Returned>() {
            book.0.insert(actor, now_ic);
        }
        ctx.out.sys_to_actor(
            actor,
            "tacsim.field",
            VOID_WAITING.replace("{n}", &waits.to_string()),
        );
    }
}

impl EventHook for LeaderReturns {
    fn connected(&self, ctx: &mut Ctx<'_>, actor: ObjId) {
        Self::returned(ctx, actor);
    }

    fn revived(&self, ctx: &mut Ctx<'_>, actor: ObjId) {
        Self::returned(ctx, actor);
    }
}

/// **`+field/void`**: the actor voids every open change naming it (doc 40 cut FR.2: each grant
/// under `field:` it may void whose window is still open). The close is read here, in the command
/// path: an offer closes at the actor's next command, so a void typed after another command is
/// refused at once.
pub(crate) fn void(ctx: &mut Ctx<'_>) {
    let actor = ctx.actor;
    let now_ic = ctx.w.world_ic_now().at;
    let prev_input = ctx.w.session_stat(actor).and_then(|stat| stat.prev_input);
    // Doc 45 cut AP.2: an offer that waited for the actor runs from their return.
    let returned = ctx
        .w
        .module_state::<Returned>()
        .and_then(|book| book.0.get(&actor).copied());
    let causes: Vec<String> = mush_faction::voidable_causes(ctx.w, "field:", actor)
        .into_iter()
        .filter(|(_, window)| {
            window.is_some_and(|window| {
                open(now_ic, prev_input, window) && within_return(now_ic, returned, window)
            })
        })
        .map(|(cause, _)| cause)
        .collect();
    let mut voided = 0_usize;
    for cause in &causes {
        match mush_faction::void_standing(ctx, cause, actor, VOID_REASON) {
            Ok(_) => voided += 1,
            Err(error) => {
                tracing::warn!(cause, %error, "tacsim: a friendly-fire standing change could not be voided");
            }
        }
    }
    if voided == 0 {
        ctx.out.info(actor, NOTHING_TO_VOID.to_owned());
        return;
    }
    ctx.out.sys_to_actor(
        actor,
        "tacsim.field",
        VOIDED.replace("{n}", &voided.to_string()),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pairs_change_is_a_point_a_round_bounded_at_the_cap() {
        assert_eq!(delta_for(1), -1);
        assert_eq!(delta_for(4), -4);
        assert_eq!(delta_for(10), -10);
        assert_eq!(delta_for(u32::MAX), STANDING_FIELD_CAP);
    }

    #[test]
    fn a_cause_is_lowercase_bounded_and_stable() {
        let site = CanonicalId::new("tacsim.port.ahuna_flank").expect("an id");
        let prefix = cause_prefix(&site, 12);
        assert_eq!(prefix, cause_prefix(&site, 12));
        assert_ne!(prefix, cause_prefix(&site, 13));
        let cause = format!("{prefix}ab:4294967295");
        assert!(cause.len() <= 96, "{cause}");
        assert!(
            cause
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b":".contains(&b)),
            "{cause}"
        );
    }

    #[test]
    fn an_offer_closes_on_the_leaders_next_input_or_its_ic_window() {
        let mut w = World::new_minimal();
        let leader = w
            .create(
                mush_core::ObjKind::Player,
                "Lead",
                mush_core::GOD,
                Some(mush_core::LIMBO),
            )
            .expect("the leader");
        let prev = |w: &World| w.session_stat(leader).and_then(|stat| stat.prev_input);
        w.set_now(1_000);
        w.session_start(leader);
        let ic = w.world_ic_now().at;
        let window = mush_faction::VoidWindow {
            opened_at: 1_000,
            until_ic: ic.saturating_add(VOID_WINDOW_IC_S),
        };
        // The void, typed first: nothing came before it.
        w.set_now(1_003);
        w.session_input(leader);
        assert!(
            open(ic, prev(&w), window),
            "nothing typed since the demotion"
        );
        // Another command, then the void: the other command closed it.
        w.set_now(1_005);
        w.session_input(leader);
        assert!(!open(ic, prev(&w), window), "the next command closes it");
        // A reconnect seeds the stamp afresh: a void typed first after it is not refused.
        w.session_end(leader);
        w.set_now(1_010);
        w.session_start(leader);
        w.session_input(leader);
        assert!(open(ic, prev(&w), window), "a reconnect forgives");
        // A voider with no session (an NPC) keeps it only for the IC window.
        assert!(open(ic, None, window));
        assert!(
            !open(window.until_ic.saturating_add(1), None, window),
            "the IC window ends it"
        );
    }

    /// **Doc 45 cut AP.2.** An offer written for a leader who was away ends 28 IC days after
    /// the demotion and runs ten IC minutes from their return; an ordinary offer is untouched by
    /// any of it.
    #[test]
    fn ap2_a_waiting_offer_runs_from_the_leaders_return_and_ends_at_28_ic_days() {
        let demoted_ic = 1_000_000;
        let ordinary = mush_faction::VoidWindow {
            opened_at: 50,
            until_ic: demoted_ic + VOID_WINDOW_IC_S,
        };
        let waits = mush_faction::VoidWindow {
            opened_at: 50,
            until_ic: demoted_ic + VOID_ABSENT_IC_S,
        };
        // An ordinary offer is never waiting, and no return stamp opens or closes it.
        assert!(!waiting(demoted_ic, ordinary));
        assert!(within_return(demoted_ic + 10, None, ordinary));
        assert!(within_return(demoted_ic + 10, Some(0), ordinary));
        assert!(open(demoted_ic + VOID_WINDOW_IC_S, None, ordinary));
        assert!(!open(demoted_ic + VOID_WINDOW_IC_S + 1, None, ordinary));
        // The leader returns three IC days on: ten IC minutes from then, and their next command
        // still closes it.
        let back = demoted_ic + 3 * 86_400;
        assert!(waiting(back, waits));
        assert!(open(back + 30, None, waits) && within_return(back + 30, Some(back), waits));
        assert!(
            !within_return(back + VOID_WINDOW_IC_S + 1, Some(back), waits),
            "ten IC minutes after the return it has lapsed"
        );
        assert!(
            !open(back + 30, Some(60), waits),
            "another command closed it"
        );
        // A later return inside the 28 IC days offers it again.
        let again = demoted_ic + 20 * 86_400;
        assert!(within_return(again + 5, Some(again), waits));
        // Past the 28 IC days it is gone, returned or not.
        let late = demoted_ic + VOID_ABSENT_IC_S + 1;
        assert!(!open(late, None, waits));
        // Inside the last ten IC minutes of the 28 IC days it is an ordinary offer.
        let last = demoted_ic + VOID_ABSENT_IC_S - 60;
        assert!(!waiting(last, waits));
        assert!(open(last, None, waits) && within_return(last, None, waits));
    }

    /// **Doc 45 cut AP.2.** Only a player who is not on watch is waited for.
    #[test]
    fn ap2_only_an_absent_player_is_waited_for() {
        let mut w = World::new_minimal();
        let make = |w: &mut World, kind, name: &str| {
            w.create(kind, name, mush_core::GOD, Some(mush_core::LIMBO))
                .expect("a character")
        };
        let player = make(&mut w, mush_core::ObjKind::Player, "Lead");
        let npc = make(&mut w, mush_core::ObjKind::Npc, "Raider");
        assert!(absent(&w, player), "a player with no connection");
        assert!(!absent(&w, npc), "an NPC is never waited for");
        w.set_connected(player, true);
        assert!(
            absent(&w, player),
            "linkdead: marked connected, no live transport"
        );
    }

    #[test]
    fn a_tally_round_trips_through_the_live_records_rows() {
        let a = PartyId::nth(0).expect("A");
        let b = PartyId::nth(1).expect("B");
        let mut counts = BTreeMap::new();
        counts.insert((a, b), 4_u32);
        counts.insert((b, a), 1_u32);
        let laid = rows(Some(&counts));
        assert_eq!(laid.len(), 2);
        assert_eq!(seed(&laid), counts);
        assert!(rows(None).is_empty());
    }
}
