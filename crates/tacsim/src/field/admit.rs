//! **Admission** (doc 32 cut CF.2; Q-CF-16, Q-CF-21, Q-CF-31) and the field host's tick.
//!
//! The `tacsim.field` tick (Coarse, one second; no tick fires while the W-clock is frozen) walks
//! the lane sites. A site whose record is promoted by a hosted door ([`PromotedBy::hosted`]) and
//! has no field running, no resume pending and no close in flight is admitted — once no ramp
//! handoff is in flight on it (Q-H4): every settled rig placed where its leg stopped, then every
//! unit on the ground — the carriers on the pad included, each at a pad pose of its own (doc 32
//! cut CF.3) — checked out under the field's custody mark and handed to a fresh task. A
//! refusal anywhere demotes the site again at once with the reason told to whoever promoted it:
//! no partial admission ever (Q-CF-21).
//!
//! The tick also runs the exits at the ramp and the extractions due (doc 32 cut CF.5), and submits
//! the host's owed `close` writes (a principal with an action already in
//! flight waits a second), and forgets a close once the record shows it landed. Every owed close
//! is durable from the batch that owes it: the site's closing tombstone
//! ([`super::record::FieldClosing`]), written beside the check-ins and deleted with the forget,
//! so a crash between the two lanes never re-admits a finished field (doc 32 §15).

use std::collections::BTreeMap;

use mush_core::{CanonicalId, Ctx, ObjId, PerceivedRef};
use serde_json::json;

use super::parties::{self, Entrant};
use super::record::{FIELD_LIVE_FORMAT, FieldLive};
use super::{FieldBook, Hosted, PendingFire};
use crate::arena::FieldHost;
use crate::arena::party::{EndCondition, Mark};
use crate::arena::state::{Controller, CustodyOut, CustodyUnit, MatchMode, Pilot};
use crate::surface::site_record::{FieldId, PromotedBy, SiteKey, SiteRecord};

/// **A `close` the host owes the record** (Q-CF-20): submitted by the tick as `actor`.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OwedClose {
    /// Who submits it: the character who opened the field.
    pub actor: ObjId,
    /// The field's ordinal.
    pub ordinal: u32,
    /// The units it leaves: the record's new unit index.
    pub units: Vec<crate::surface::site_record::LeftUnit>,
    /// **What each principal learned on the ground** (doc 32 cut CF.4; Q-CF-28).
    pub known: Vec<super::known::Learned>,
}

/// The owed closes, by site (module state beside [`FieldBook`]).
#[derive(Clone, Debug, Default)]
pub(crate) struct OwedCloses(pub BTreeMap<CanonicalId, OwedClose>);

/// The character a promotion names: the leader who marked, or the pilot who ordered fire.
#[must_use]
pub(crate) fn promoter(by: &PromotedBy) -> Option<ObjId> {
    match by {
        PromotedBy::Staff { .. } => None,
        PromotedBy::Mark { leader, .. } => Some(*leader),
        PromotedBy::Fire { pilot, .. } => Some(*pilot),
    }
}

/// **`tacsim.field`**: the held resumes first, then the owed closes, then admission.
pub(crate) fn tick(ctx: &mut Ctx<'_>, _tag: &str, _scene: Option<mush_core::SceneId>) {
    if !ctx
        .w
        .module_state::<crate::TacsimStore>()
        .is_some_and(crate::TacsimStore::enabled)
        || ctx.w.world_timeline().is_frozen()
    {
        return;
    }
    super::boot::run_held(ctx);
    submit_owed(ctx);
    // Doc 32 cut CF.5: the exits at the ramp and the extractions due.
    super::exit::tick(ctx);
    super::crew::tick(ctx);
    // Doc 32 cut CF.6b: every NPC under an ordered withdrawal asks out at its ramp.
    super::home::tick(ctx);
    let Some(handles) = ctx
        .w
        .module_state::<crate::port::families::PortHandles>()
        .cloned()
    else {
        return;
    };
    let sites: Vec<CanonicalId> = crate::surface::chart::lane_sites(ctx.w)
        .into_iter()
        .map(|site| site.entry)
        .collect();
    for site in sites {
        let key = SiteKey(site.clone());
        let Some(record) = ctx
            .action_state(&handles.families.sites, &key)
            .ok()
            .flatten()
            .cloned()
            .filter(|record| record.key() == key)
        else {
            forget_close(ctx, &site);
            continue;
        };
        let Some(open) = record.promoted.clone() else {
            forget_close(ctx, &site);
            continue;
        };
        let owed_ordinal = ctx
            .w
            .module_state::<OwedCloses>()
            .and_then(|owed| owed.0.get(&site).map(|close| close.ordinal));
        if owed_ordinal.is_some_and(|ordinal| ordinal != open.ordinal) {
            forget_close(ctx, &site);
        }
        if !open.by.hosted() {
            continue;
        }
        let busy = ctx.w.module_state::<FieldBook>().is_none_or(|book| {
            book.hosted(&site).is_some() || book.pending(&site) || book.closing.contains(&site)
        });
        if busy {
            continue;
        }
        if let Err(why) = admit(ctx, &site, &record) {
            if why.is_empty() {
                continue; // a ramp handoff is still in flight
            }
            tracing::warn!(site = %site, reason = %why, "tacsim: a field could not admit; it demotes");
            if let Some(actor) = promoter(&open.by) {
                ctx.out.sys_to_actor(
                    actor,
                    "tacsim.field",
                    format!(
                        "The field on {} could not open: {why}",
                        site_name(ctx, &site)
                    ),
                );
            }
            owe_close(ctx, &site, &record, None, Vec::new());
        }
    }
}

/// The site's display name.
pub(crate) fn site_name(ctx: &Ctx<'_>, site: &CanonicalId) -> String {
    crate::surface::act::site_name(ctx.w, site)
}

/// A close the record has caught up with, forgotten, and its tombstone with it.
fn forget_close(ctx: &mut Ctx<'_>, site: &CanonicalId) {
    let owed = ctx
        .w
        .module_state_mut::<OwedCloses>()
        .and_then(|owed| owed.0.remove(site))
        .is_some();
    let closing = ctx
        .w
        .module_state_mut::<FieldBook>()
        .is_some_and(|book| book.closing.remove(site));
    if owed || closing {
        super::record::delete_closing(ctx, site);
    }
}

/// **A closing tombstone the boot found** (doc 32 §15): its close owed again and the site
/// closing, so the tick neither re-admits the finished field nor loses what it left.
pub(crate) fn restore_closing(w: &mut mush_core::World, closing: super::record::FieldClosing) {
    let site = closing.site.clone();
    if let Some(book) = w.module_state_mut::<FieldBook>() {
        book.closing.insert(site.clone());
    }
    if let Some(owed) = w.module_state_mut::<super::exit::OwedExits>() {
        for exit in &closing.exits {
            owed.0
                .entry(site.clone())
                .or_default()
                .insert(exit.unit, exit.actor);
        }
    }
    if let Some(owed) = w.module_state_mut::<OwedCloses>() {
        owed.0.insert(site, closing.close);
    }
}

/// **Owe the record a `close`** of the open field with `left` as its unit index (`None`: the
/// index as it stands, for a field that never ran).
pub(crate) fn owe_close(
    ctx: &mut Ctx<'_>,
    site: &CanonicalId,
    record: &SiteRecord,
    left: Option<Vec<crate::surface::site_record::LeftUnit>>,
    known: Vec<super::known::Learned>,
) {
    let Some(open) = record.promoted.as_ref() else {
        return;
    };
    let Some(actor) = promoter(&open.by) else {
        return;
    };
    let units = left.unwrap_or_else(|| record.units.clone());
    if let Some(book) = ctx.w.module_state_mut::<FieldBook>() {
        book.closing.insert(site.clone());
    }
    let close = OwedClose {
        actor,
        ordinal: open.ordinal,
        units,
        known,
    };
    // Doc 32 §15 (the review's CF-1): durable in this batch, beside the check-ins, before the
    // K3 write that cannot share its turn.
    let exits: Vec<super::record::OwedExit> = ctx
        .w
        .module_state::<super::exit::OwedExits>()
        .and_then(|owed| owed.0.get(site))
        .map(|units| {
            units
                .iter()
                .map(|(unit, actor)| super::record::OwedExit {
                    unit: *unit,
                    actor: *actor,
                })
                .collect()
        })
        .unwrap_or_default();
    super::record::write_closing(
        ctx,
        &super::record::FieldClosing {
            format: super::record::FIELD_CLOSING_FORMAT,
            site: site.clone(),
            close: close.clone(),
            exits,
        },
    );
    if let Some(owed) = ctx.w.module_state_mut::<OwedCloses>() {
        owed.0.insert(site.clone(), close);
    }
    submit_owed(ctx);
}

/// **Submit every owed close** whose actor has nothing in flight.
fn submit_owed(ctx: &mut Ctx<'_>) {
    let Some(action) = ctx
        .w
        .module_state::<crate::port::families::PortHandles>()
        .and_then(|handles| handles.field.clone())
    else {
        return;
    };
    let owed: Vec<(CanonicalId, OwedClose)> = ctx
        .w
        .module_state::<OwedCloses>()
        .map(|owed| owed.0.iter().map(|(s, c)| (s.clone(), c.clone())).collect())
        .unwrap_or_default();
    let now = ctx.w.world_ic_now().at;
    for (site, close) in owed {
        if ctx.authoritative_pending(close.actor) {
            continue;
        }
        let Some(key) = super::provider::field_key(close.actor, &site, "close", close.ordinal, now)
        else {
            continue;
        };
        let saved = ctx.actor;
        ctx.actor = close.actor;
        ctx.submit_authoritative(
            action.clone(),
            PerceivedRef::SelfActor,
            Vec::new(),
            json!({ "site": site.to_string(), "op": "close" }),
            key,
        );
        ctx.actor = saved;
    }
}

/// **Admit everyone on `site`'s ground** (this module's head). `Err("")` waits for a ramp
/// handoff; any other `Err` is the reason the field refuses.
fn admit(ctx: &mut Ctx<'_>, site: &CanonicalId, record: &SiteRecord) -> Result<(), String> {
    let open = record.promoted.clone().ok_or_else(String::new)?;
    let field = FieldId {
        site: site.clone(),
        ordinal: open.ordinal,
    };
    let domain =
        parties::site_domain(ctx.w, site).ok_or_else(|| "it is not a lane site".to_owned())?;
    // (1) Every ramp handoff on the site finishes first (Q-H4).
    let handles = ctx
        .w
        .module_state::<crate::port::families::PortHandles>()
        .cloned()
        .ok_or_else(|| "the site records are unavailable".to_owned())?;
    for rig in crate::surface::act::world_rigs(ctx.w) {
        let pending = ctx
            .action_state(
                &handles.families.excursions,
                &crate::surface::excursion::RigKey(rig),
            )
            .ok()
            .flatten()
            .is_some_and(|rig| super::provider::handoff_pending(rig, site));
        if pending {
            return Err(String::new());
        }
    }
    // (2) The settled rigs, where their legs stopped (Q-CF-31).
    for settled in &open.settled {
        let Some((on, pose)) = mush_sim::pose_of(ctx.w, settled.tv) else {
            continue;
        };
        if on != domain {
            continue;
        }
        if pose.x_cm != settled.pose.x_cm || pose.y_cm != settled.pose.y_cm {
            let mut at = pose.clone();
            at.x_cm = settled.pose.x_cm;
            at.y_cm = settled.pose.y_cm;
            at.heading_mdeg = settled.pose.heading_mdeg;
            mush_sim::place(ctx, settled.tv, domain, at)
                .map_err(|error| format!("a settled rig could not be placed ({error})"))?;
        }
    }
    // (3) Everyone on the ground, and the cap (Q-CF-21): the casualties and wrecks an earlier
    // field left are admitted but never counted (Q-CF-26, Q-CF-45; doc 32 §15, the review's
    // CF-3a). A carrier counts: no ruling exempts it.
    let units = parties::on_ground(ctx.w, domain);
    let cap = super::config(ctx.w).max_units;
    let counted = parties::counted(&units, record);
    if u32::try_from(counted).unwrap_or(u32::MAX) > cap {
        return Err(format!(
            "{counted} units are on the ground and a field holds {cap}"
        ));
    }
    if units.is_empty() {
        return Err("nobody is on the ground".to_owned());
    }
    let entrants = parties::entrants(ctx, site, &units);
    let (lettered, rows) = parties::letter(&entrants).map_err(str::to_owned)?;
    let homes = super::home::homes(ctx.w, &entrants, &rows);
    let definition = super::ground::definition(ctx.w, site, record, homes.clone())?;
    // CF.3 (Q-CF-19): the carriers stand on the pad, clear of everyone else.
    let (carriers, others): (Vec<ObjId>, Vec<ObjId>) = units
        .iter()
        .partition(|unit| super::carrier::is_carrier(ctx.w, **unit));
    // Doc 32 §17: two units on one cell (rigs launched down one ramp) are parted first.
    let mut taken: std::collections::BTreeSet<(u16, u16)> = std::collections::BTreeSet::new();
    super::carrier::unstack(ctx, &definition.terrain, &others, &mut taken)?;
    let pads = super::carrier::pad_poses(ctx.w, site, &definition.terrain, &carriers, &mut taken)?;
    let custody = custody(
        ctx, &field, &open.by, &entrants, &lettered, &rows, &pads, definition,
    )?;
    // (4) The check-out, all or nothing.
    let mark = mush_sim::deployed::DeployedMark::field(site.as_str(), field.ordinal);
    let mut out: Vec<ObjId> = Vec::new();
    for unit in &units {
        if let Err(error) = mush_tactics::custody::check_out(ctx, *unit, mark.clone()) {
            for back in out {
                let _ = mush_tactics::custody::check_in(ctx, back, None);
            }
            return Err(format!(
                "#{} could not be taken into the field ({error})",
                unit.0
            ));
        }
        out.push(*unit);
    }
    // (5) The task.
    let cfg = super::config(ctx.w);
    let task = ctx
        .w
        .module_state_mut::<FieldBook>()
        .map(FieldBook::next_task)
        .ok_or_else(|| "the field host is unavailable".to_owned())?;
    let live = FieldLive {
        format: FIELD_LIVE_FORMAT,
        field: field.clone(),
        opened_ic: open.at_ic,
        segment: 1,
        at_ms: 0,
        parties: lettered,
        units: rows,
        homes,
        fire: None,
        owed_exits: Vec::new(),
    };
    if let Some(book) = ctx.w.module_state_mut::<FieldBook>() {
        book.live.insert(
            site.clone(),
            Hosted {
                task,
                live: live.clone(),
            },
        );
    }
    let opened = crate::arena_bridge::open_field_task(
        ctx,
        task,
        custody,
        crate::arena_task::FieldTerms {
            disengage_ms: cfg.disengage_ms(),
        },
    );
    if let Err(error) = opened {
        if let Some(book) = ctx.w.module_state_mut::<FieldBook>() {
            book.live.remove(site);
        }
        for back in out {
            let _ = mush_tactics::custody::check_in(ctx, back, None);
        }
        return Err(match error {
            crate::arena_bridge::OpenMatchError::Arena(
                crate::arena::state::ArenaError::CellImpassable { unit, x, y },
            ) => format!(
                "#{} stands on ({x}, {y}), ground it could never move from",
                unit.0
            ),
            other => format!("its task did not open ({other})"),
        });
    }
    // (6) A fire order waits for the shooter's sensors (Q-CF-14), on the live record too (doc 32
    // §15, the review's CF-6): a restart before the shot owes it again.
    let mut live = live;
    if let PromotedBy::Fire {
        pilot,
        shooter,
        target,
    } = &open.by
        && let Some((_, pose)) = mush_sim::pose_of(ctx.w, *target)
    {
        let (x, y) = crate::surface::excursion::site_cell(pose.x_cm, pose.y_cm);
        if let (Ok(x), Ok(y)) = (u16::try_from(x), u16::try_from(y)) {
            let fire = PendingFire {
                pilot: *pilot,
                shooter: *shooter,
                target_cell: (x, y),
                until_ms: super::FIRE_WINDOW_MS,
            };
            live.fire = Some(fire.clone());
            if let Some(book) = ctx.w.module_state_mut::<FieldBook>() {
                book.fire.insert(site.clone(), fire);
                if let Some(hosted) = book.live.get_mut(site) {
                    hosted.live.fire = live.fire.clone();
                }
            }
        }
    }
    super::record::write_live(ctx, &live);
    let line = super::FIELD_OPENED
        .replace("{n}", &field.ordinal.to_string())
        .replace("{site}", &site_name(ctx, site));
    super::tell_site(ctx, site, &line);
    tracing::info!(
        site = %site,
        ordinal = field.ordinal,
        task = task.0,
        units = units.len(),
        "tacsim: an IC field opened"
    );
    Ok(())
}

/// **The check-out message** (Q-CF-3, Q-CF-41): each unit at its placement (a carrier at its
/// pad pose, CF.3) under its party's letter, sealed on the ROM over its epoch, its seated pilot carried; the opening mark from the
/// door, the standing friendships, and the host's own end.
#[allow(clippy::too_many_arguments)] // the admission's parts, each read once
fn custody(
    ctx: &Ctx<'_>,
    field: &FieldId,
    by: &PromotedBy,
    entrants: &[Entrant],
    parties: &[super::record::FieldParty],
    rows: &[super::record::FieldUnit],
    pads: &BTreeMap<ObjId, mush_sim::data::Pose>,
    definition: crate::arena::state::ArenaDefinition,
) -> Result<CustodyOut, String> {
    let store = ctx
        .w
        .module_state::<crate::store::TacsimStore>()
        .ok_or_else(|| "the layer is not loaded".to_owned())?;
    let catalog = store.definitions().clone();
    let cfg = store.config();
    let now = ctx.w.world_ic_now().at;
    let mut units: Vec<CustodyUnit> = Vec::new();
    let mut pilots: Vec<Pilot> = Vec::new();
    let mut slots: BTreeMap<crate::arena::PartyId, u8> = BTreeMap::new();
    for entrant in entrants {
        let party = rows
            .iter()
            .find(|row| row.unit == entrant.unit)
            .map(|row| row.party)
            .ok_or_else(|| format!("#{} has no party", entrant.unit.0))?;
        let cells = mush_tactics::module::unit_of(ctx.w, entrant.unit)
            .cloned()
            .ok_or_else(|| format!("#{} has no tactical unit record", entrant.unit.0))?;
        let pose = match pads.get(&entrant.unit) {
            Some(pad) => pad.clone(),
            None => {
                mush_sim::pose_of(ctx.w, entrant.unit)
                    .ok_or_else(|| format!("#{} is not placed", entrant.unit.0))?
                    .1
            }
        };
        let slot = {
            let next = slots.entry(party).or_insert(0);
            *next = next.saturating_add(1);
            *next
        };
        // A hired NPC in the seat flies the doctrine under its delivered orders (doc 32 cut CF.6,
        // Q-CF-42): its hull is the mind's, and its seat stays bound below so its own frame —
        // what the NPC sees — reaches the host's executor (`super::orders`).
        let controller = match &entrant.pilot {
            Some((pilot, _)) if !super::orders::is_npc(ctx.w, *pilot) => Controller::Pilot,
            _ => Controller::Mind,
        };
        let weapons = crate::match_::weapon_summaries(&catalog, &cells);
        let rule_weapons: Vec<crate::arena::rules::RuleWeapon> = weapons
            .iter()
            .map(|weapon| crate::arena::rules::RuleWeapon {
                id: weapon.id.clone(),
                rounds_max: weapon.count_per_load,
            })
            .collect();
        let sortie = entrant
            .epoch
            .unwrap_or_else(|| crate::arena::mind::SortieId::ic(entrant.unit, now));
        let principal = entrant
            .leader
            .map(|id| crate::deliberation::CharacterRef::Character { id });
        let seal = crate::match_::seal::seal_rom_for(
            entrant.unit,
            &cells,
            &rule_weapons,
            sortie,
            &crate::match_::seal::IcSealConfig {
                minds: &cfg.minds,
                reflex: &cfg.arena.reflex,
                principal,
                impounded: crate::match_::seal::core_impounded(ctx.w, &cells),
            },
        )
        .map_err(|error| format!("#{} could not be sealed ({error})", entrant.unit.0))?;
        units.push(CustodyUnit {
            id: entrant.unit,
            team: party,
            pose,
            mobility: crate::match_::tracked_rules(&cells),
            sensor_height_cm: crate::match_::sensor_height_cm(&catalog, &cells),
            sensor: crate::match_::sensor_profile(&catalog, &cells),
            weapons,
            profile: crate::match_::unit_profile(&catalog, &cells),
            gunnery: crate::arena::resolution::STOCK_GUNNERY,
            controller,
            slot,
            seal,
            unit: cells,
        });
        if let Some((pilot, station)) = &entrant.pilot {
            pilots.push(Pilot {
                unit: entrant.unit,
                pilot: *pilot,
                station: station.clone(),
                epoch: 1,
            });
        }
    }
    let party_of = |unit: ObjId| {
        rows.iter()
            .find(|row| row.unit == unit)
            .map(|row| row.party)
    };
    let mut marks = Vec::new();
    if let PromotedBy::Mark { by, of, .. } = by
        && let (Some(by), Some(of)) = (party_of(*by), party_of(*of))
        && by != of
    {
        marks.push((by, of, Mark::Hostile));
    }
    // A fixture raider's leader holds a standing Hostile mark on every other party (doc 32 cut
    // CF.6, Q-CF-44): the raider fights whoever it finds, with no player connected.
    for raider in parties.iter().filter(|party| party.key.is_fixture()) {
        for other in parties.iter().filter(|other| other.party != raider.party) {
            marks.push((raider.party, other.party, Mark::Hostile));
        }
    }
    let friends = parties::friends(entrants, parties);
    let seed = crate::match_::seed_for(
        crate::arena_task::MatchId(u64::from(field.ordinal)),
        ctx.w.now(),
    );
    Ok(CustodyOut {
        definition,
        units,
        pilots,
        resolution: crate::match_::resolution_for(&catalog),
        seed,
        mode: MatchMode::Skirmish,
        host: FieldHost {
            all_hostile: false,
            marks,
            friends,
            end: EndCondition::Host,
            every_class: true,
            weapons_hold: true,
        },
    })
}
