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
//!   change the offer holds. The offer lasts until the leader's next command or
//!   [`VOID_WINDOW_IC_S`] of IC time, whichever is first. The change and its void are written
//!   together in `mush-faction`'s receipts: the grant names who may void it, and a void is a
//!   record of the same cause beside it.
//! - **A void end** (a panic, a stall, a ledger overflow) commits nothing the arena derived, so it
//!   writes no standing either.
//!
//! The tally and the offers are RAM. A restart mid-field loses the rounds counted before it, and a
//! restart inside a void window closes the window: the change stands.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::{CanonicalId, Ctx, ObjId, World};

use super::FieldBook;
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

/// **The change, told to each principal it moved** (`{n}` the field's ordinal, `{site}` its
/// name, `{party}` the victim's letter, `{delta}` the change, `{factions}` the factions).
pub const STANDING_CHANGED: &str = "Your party's fire on party {party} in field {n} at {site} \
moves your standing with {factions} by {delta}.";
/// **The void's offer**, told to the shooter party's leader.
pub const VOID_OFFERED: &str = "Party {party} was a friend of yours: +field/void cancels the \
standing your party's fire on it cost, until your next command or ten IC minutes.";
/// **The void**, told to the leader.
pub const VOIDED: &str =
    "Voided as friendly fire: {n} standing change(s) your party's fire caused.";
/// Nothing to void.
pub const NOTHING_TO_VOID: &str = "Nothing your party's fire cost is yours to void now.";
/// The reason every void records.
const VOID_REASON: &str = "Voided by the party's leader as friendly fire.";

/// **The rounds each pair of parties exchanged**, per site with a field running: by (shooter,
/// victim).
#[derive(Clone, Debug, Default)]
pub(crate) struct FireTally(BTreeMap<CanonicalId, BTreeMap<(PartyId, PartyId), u32>>);

/// One leader's open void offer.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Offer {
    /// The causes it may void.
    causes: Vec<String>,
    /// The wall second it opened: an input after it closes it.
    opened_at: i64,
    /// The IC second it closes at.
    until_ic: i64,
}

/// **Every open void offer**, by leader.
#[derive(Clone, Debug, Default)]
pub(crate) struct VoidOffers(BTreeMap<ObjId, Offer>);

/// **A chunk of a field's black box passed** (the bridge's ledger branch): count each round that
/// reached another party's unit. Any other task's chunk is not a field's and is ignored.
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
    if let Some(tally) = ctx.w.module_state_mut::<FireTally>() {
        let counts = tally.0.entry(site).or_default();
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
/// fire to its leader, and forget the site's tally. Called by `close::on_ended` while the host
/// still holds the field. `void` is a void end, which writes nothing.
pub(crate) fn at_demotion(ctx: &mut Ctx<'_>, site: &CanonicalId, void: bool) {
    let counts = ctx
        .w
        .module_state_mut::<FireTally>()
        .and_then(|tally| tally.0.remove(site))
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
    let now_ic = ctx.w.world_ic_now().at;
    let opened_at = ctx.w.now();
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
        if let Some(leader) = voidable_by
            && !voidable.is_empty()
        {
            offer(ctx, leader, voidable, opened_at, now_ic);
            let line = VOID_OFFERED.replace("{party}", &victim.letter().to_string());
            ctx.out.sys_to_actor(leader, "tacsim.field", line);
        }
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

/// Open, or widen, `leader`'s offer: a later demotion's causes join it and its window restarts.
fn offer(ctx: &mut Ctx<'_>, leader: ObjId, causes: Vec<String>, opened_at: i64, now_ic: i64) {
    let Some(offers) = ctx.w.module_state_mut::<VoidOffers>() else {
        return;
    };
    let entry = offers.0.entry(leader).or_insert_with(|| Offer {
        causes: Vec::new(),
        opened_at,
        until_ic: now_ic,
    });
    for cause in causes {
        if !entry.causes.contains(&cause) {
            entry.causes.push(cause);
        }
    }
    entry.opened_at = opened_at;
    entry.until_ic = now_ic.saturating_add(VOID_WINDOW_IC_S);
}

/// Whether `offer` is still open for `leader` now: inside its IC window, and no input of the
/// leader's since it opened.
fn open(w: &World, leader: ObjId, offer: &Offer) -> bool {
    w.world_ic_now().at <= offer.until_ic
        && w.session_stat(leader)
            .is_none_or(|stat| stat.last_input <= offer.opened_at)
}

/// **The offers' sweep** (once a second on the field tick's period): an offer closes when its IC
/// window ends or once its leader has entered anything since it opened. The leader's
/// `+field/void` takes its offer in the same dispatch that stamps the input, so the void itself
/// is never the command that closes it. Two inputs inside one sweep's second can let a void
/// through after another command; a command in the very second of the demotion is not seen.
pub(crate) fn tick(ctx: &mut Ctx<'_>, _tag: &str, _scene: Option<mush_core::SceneId>) {
    let closed: Vec<ObjId> = ctx
        .w
        .module_state::<VoidOffers>()
        .map(|offers| {
            offers
                .0
                .iter()
                .filter(|(leader, offer)| !open(ctx.w, **leader, offer))
                .map(|(leader, _)| *leader)
                .collect()
        })
        .unwrap_or_default();
    if closed.is_empty() {
        return;
    }
    if let Some(offers) = ctx.w.module_state_mut::<VoidOffers>() {
        for leader in closed {
            offers.0.remove(&leader);
        }
    }
}

/// **`+field/void`**: the actor voids every change its open offer holds.
pub(crate) fn void(ctx: &mut Ctx<'_>) {
    let actor = ctx.actor;
    let offer = ctx
        .w
        .module_state_mut::<VoidOffers>()
        .and_then(|offers| offers.0.remove(&actor));
    // The deadline is read here too: the sweep runs once a second. The input stamp is this
    // command's own, so it is not read.
    let Some(offer) = offer.filter(|offer| ctx.w.world_ic_now().at <= offer.until_ic) else {
        ctx.out.info(actor, NOTHING_TO_VOID.to_owned());
        return;
    };
    let mut voided = 0_usize;
    for cause in &offer.causes {
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
        w.set_now(1_000);
        w.session_start(leader);
        let ic = w.world_ic_now().at;
        let offer = Offer {
            causes: vec!["field:x:1:ab:2".to_owned()],
            opened_at: 1_000,
            until_ic: ic.saturating_add(VOID_WINDOW_IC_S),
        };
        assert!(open(&w, leader, &offer), "nothing typed since the demotion");
        // The leader's next command.
        w.set_now(1_005);
        w.session_input(leader);
        assert!(!open(&w, leader, &offer), "the next command closes it");
        // A leader with no session (an NPC) keeps it only for the IC window.
        let npc = ObjId(u32::MAX);
        assert!(open(&w, npc, &offer));
        let past = Offer {
            until_ic: ic.saturating_sub(1),
            ..offer
        };
        assert!(!open(&w, npc, &past), "the IC window ends it");
    }
}
