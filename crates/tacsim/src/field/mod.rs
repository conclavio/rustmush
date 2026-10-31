//! **The IC field host** (doc 32 cut CF.2, the spine; Q-CF-13 to Q-CF-23, Q-CF-30 to Q-CF-36).
//!
//! A **site** is a piece of ground and its persistent record (`tacsim.field`,
//! [`crate::surface::site_record`]); a **field** is the site while it is promoted — the wall-time
//! host every party on the ground is admitted to; an **engagement** is a Hostile pair fighting in
//! it, and a field may hold none (doc 32 Q-CF-2). This module is the host: it runs one arena
//! kernel task per promoted site through the same seam the OOC range uses (`crate::arena_task`,
//! [`crate::arena_bridge`]), and never touches the match wrapper — no lobby, no centre slot, no
//! entry zone, no elimination, no stock unit, no practice reset, no `SIM` verb (Q-CF-1). What it
//! does borrow is kernel-shaped: the custody check-out (`mush_tactics::custody`), the unit
//! decode (`crate::match_::arena_def`'s readers) and the ROM seal (`crate::match_::seal`).
//!
//! # The lifecycle
//!
//! 1. **Promotion** (Q-CF-14) by either door: a detachment leader's Hostile mark on a party seen
//!    on the presence picture (`+field/mark`, `commands`), or a fire order at a presence row
//!    (`+field/fire`). Either submits `tacsim_field open` ([`provider`]): the site record's flag,
//!    the door that opened it, and every settleable surface act settled in the same bundle
//!    (Q-CF-30, Q-CF-31). **CF.6's hook**: a delivered `ENGAGE` received on the surface is the
//!    leader's mark ([`commands::mark_from_order`]).
//! 2. **Admission** (Q-CF-16, Q-CF-21) on the next `tacsim.field` tick: once no ramp handoff is
//!    in flight on the site, every unit on its ground is placed where the promotion settled it,
//!    checked out under a field custody mark (`mush_sim::deployed::DeployedMark::field`) and
//!    handed to a fresh task with an IC [`crate::arena::FieldHost`] — the opening mark, the
//!    standing friendships, [`crate::arena::party::EndCondition::Host`] — at its surface pose.
//!    Parties take their letters in admission order (`parties`); a bystander nobody marked
//!    reads Unknown and is shootable (Q-CF-9, Q-CF-10). No partial admission ever: a unit that
//!    will not check out refuses the whole field, which demotes at once.
//! 3. **The fight.** Pilots keep their seats; their cockpit verbs reach the task as in the
//!    simulator. Leaders mark and clear marks in the field (`+field/mark <letter>`), which reach
//!    the kernel as `ArenaMsg::Mark`. A fire order's shot resolves in the field's first pulses
//!    under the normal lock rules: the host locks the shooter's own contact on the target once its
//!    sensors hold one, and fires its first weapon; no contact within the window, no shot.
//!    Launches onto the site are refused with the reason (Q-CF-17). **Recovery as exit**
//!    (Q-CF-18, cut CF.5, `exit`): `+tv/<n> recover` or `+sortie/recover` on a unit in the
//!    field asks the kernel to let it go (`ArenaMsg::Withdraw`); beside its carrier's ramp and
//!    under its own power it leaves at the next pulse, is checked in through the IC plan, is
//!    named on the record's promotion, and its recover into the bay runs. The field carries on.
//!    **Crew** (Q-CF-38, Q-CF-39, cut CF.5, [`crew`]): every check-in derives its pilot's
//!    location and condition; a control-section kill incapacitates at once; a stranded character asks for
//!    extraction (`+sortie/extract`) and is at a safe port after `[tacsim.field].extract_s`.
//! 4. **Demotion** (Q-CF-20): the task stops itself with `Disengaged` once no hostile pair has
//!    fired, held another in weapons range or had a round in flight for `[tacsim.field]
//!    .disengage_s` ([`crate::arena_task::FieldTerms`]). `close` checks every unit in through
//!    the IC condition plan (Q-CF-36: only the custody vocabulary, no blanket wreck stamp),
//!    places it at its field pose (Q-CF-31), and submits `tacsim_field close` with every
//!    casualty and wreck at the pose it fell (Q-CF-24).
//! 5. **The IC void rule** (Q-CF-35): a void never restores the check-out snapshot. It demotes
//!    from the last snapshot the world holds; with none, every unit checks in as it went out.
//! 6. **Resume** (Q-CF-33, Q-CF-34): a restart finds the field's live record and snapshot; while
//!    the W-clock is frozen the resume is held, and it runs when the world's clock resumes
//!    (`boot`, a `WorldClockResumed` observer) — never during the countdown.
//!
//! **The clock handoff** (Q-CF-31, Q-CF-32): a drive in flight at promotion stops at the point
//! its leg reached on the campaign clock and the field takes the rig from there on wall time; at
//! demotion the surface takes the field's exact pose and any intent is dropped.

pub(crate) mod admit;
pub(crate) mod boot;
pub mod carrier;
pub(crate) mod close;
pub(crate) mod commands;
pub mod crew;
pub(crate) mod exit;
pub(crate) mod ground;
pub(crate) mod known;
pub(crate) mod orders;
pub(crate) mod parties;
pub mod provider;
pub mod raiders;
pub mod record;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use mush_core::module::{ModError, Registrar};
use mush_core::{ArgShape, CanonicalId, CommandSpec, Ctx, ObjId, TickCadence, World};

use crate::arena_task::MatchId;
pub use commands::mark_from_order;
pub use crew::{CrewCondition, CrewLocation, CrewRecord, crew_location, crew_of};
pub use provider::{FIELD_ACTION, FIELD_OUTCOME};
pub use record::{FieldLive, FieldParty, FieldUnit, PartyKey};

/// The field host's tick: admission, the fire door's lock, and the held resumes' retry.
pub const FIELD_TICK: &str = "tacsim.field";
/// Its period.
pub const FIELD_TICK_PERIOD: Duration = Duration::from_secs(1);
/// The verb.
pub const FIELD_VERB: &str = "+field";
/// **How long a fire order waits for its shooter's sensors**, arena milliseconds (doc 32
/// Q-CF-14): past it the shot never left the barrel.
pub const FIRE_WINDOW_MS: i64 = 10_000;

/// **A field the host runs**: its task and its live record.
#[derive(Clone, Debug)]
pub(crate) struct Hosted {
    /// The arena task's key in the shared table (RAM only, never a match's number: it is taken
    /// from the top of the key space down, [`FieldBook::next_task`]).
    pub task: MatchId,
    /// The live record.
    pub live: FieldLive,
}

/// **A fire order waiting for its first pulses** (Q-CF-14).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PendingFire {
    /// The pilot who ordered it.
    pub pilot: ObjId,
    /// The unit that fires.
    pub shooter: ObjId,
    /// The target's cell at admission.
    pub target_cell: (u16, u16),
    /// The arena millisecond past which it lapses.
    pub until_ms: i64,
}

/// **The host's RAM book** (module state). Everything durable is on the site record and the live
/// record; this is rebuilt by the boot.
#[derive(Clone, Debug, Default)]
pub struct FieldBook {
    live: BTreeMap<CanonicalId, Hosted>,
    /// Boot resumes held while the W-clock is frozen (Q-CF-33).
    held: BTreeMap<CanonicalId, boot::Held>,
    /// Field-marked units the boot found, by site, for the resume or the void.
    boot_marks: BTreeMap<CanonicalId, BTreeSet<ObjId>>,
    /// Sites whose `close` the host has submitted and not yet seen land.
    closing: BTreeSet<CanonicalId>,
    /// Fire orders waiting for the shooter's sensors, by site.
    fire: BTreeMap<CanonicalId, PendingFire>,
    /// The last snapshot document each running field's world holds (the void rule's source).
    snapshots: BTreeMap<CanonicalId, serde_json::Value>,
    /// What the boot read: each site's live record and snapshot, for [`boot::phase`].
    boot_docs: BTreeMap<CanonicalId, boot::BootDocs>,
    /// Task keys handed out, counting down from the top of the key space.
    issued: u64,
}

impl FieldBook {
    /// **The next task key** (RAM only): `u64::MAX`, then one less each time. The OOC centre
    /// counts its matches up from one, so the two never meet, and the field never takes a
    /// match's number (D-23-10).
    pub(crate) fn next_task(&mut self) -> MatchId {
        self.issued = self.issued.saturating_add(1);
        MatchId(u64::MAX.saturating_sub(self.issued.saturating_sub(1)))
    }

    /// The sites being closed (the `close` offer's fixed list).
    pub fn closing(&self) -> impl Iterator<Item = &CanonicalId> {
        self.closing.iter()
    }

    /// The field running on `site`.
    #[must_use]
    pub(crate) fn hosted(&self, site: &CanonicalId) -> Option<&Hosted> {
        self.live.get(site)
    }

    /// The site whose field runs on task `task`.
    #[must_use]
    pub(crate) fn site_of_task(&self, task: MatchId) -> Option<&CanonicalId> {
        self.live
            .iter()
            .find(|(_, hosted)| hosted.task == task)
            .map(|(site, _)| site)
    }

    /// Every running field, by site.
    pub(crate) fn running(&self) -> impl Iterator<Item = (&CanonicalId, &Hosted)> {
        self.live.iter()
    }

    /// The last snapshot the world holds for `site`'s field.
    #[must_use]
    pub(crate) fn snapshot(&self, site: &CanonicalId) -> Option<&serde_json::Value> {
        self.snapshots.get(site)
    }

    /// Forget `site`'s snapshot (its field is over).
    pub(crate) fn forget_snapshot(&mut self, site: &CanonicalId) {
        self.snapshots.remove(site);
    }

    /// Whether a resume is held for `site` (or the boot is still to decide it).
    #[must_use]
    pub(crate) fn pending(&self, site: &CanonicalId) -> bool {
        self.held.contains_key(site) || self.boot_marks.contains_key(site)
    }
}

/// **Whether task `task` is a field's**, and so not a match's (the bridge's one branch).
#[must_use]
pub fn hosts(w: &World, task: MatchId) -> bool {
    w.module_state::<FieldBook>()
        .is_some_and(|book| book.site_of_task(task).is_some())
}

/// **The field open on the site `unit` stands in**, as its site, when a field holds it.
#[must_use]
pub fn field_of_unit(w: &World, unit: ObjId) -> Option<CanonicalId> {
    let mark = mush_sim::deployed::mark_of(w, unit)?;
    let field = mark.field.as_ref()?;
    CanonicalId::new(field.site.as_str()).ok()
}

/// **The live record of the field running on `site`** (doc 32 cut CF.6: what CF.7 and the
/// stories read to name a party), when one runs.
#[must_use]
pub fn live_on(w: &World, site: &CanonicalId) -> Option<FieldLive> {
    w.module_state::<FieldBook>()?
        .hosted(site)
        .map(|hosted| hosted.live.clone())
}

/// The `[tacsim.field]` the server runs under.
#[must_use]
pub(crate) fn config(w: &World) -> crate::config::FieldConfig {
    w.module_state::<crate::TacsimStore>()
        .map(|store| store.config().field)
        .unwrap_or_default()
}

/// One line to everyone on the site's ground or aboard a hull settled on its pad — a physical
/// fact anyone there observes, never who marked whom (Q-CF-7).
pub(crate) fn tell_site(ctx: &mut Ctx<'_>, site: &CanonicalId, text: &str) {
    let mut seen: BTreeSet<ObjId> = BTreeSet::new();
    if let Some(domain) = parties::site_domain(ctx.w, site) {
        for unit in parties::on_ground(ctx.w, domain) {
            if let Some((pilot, _)) = parties::seated(ctx.w, unit) {
                seen.insert(pilot);
            }
        }
    }
    // The operators of rigs out on the site, wherever they stand.
    if let Some(handles) = ctx
        .w
        .module_state::<crate::port::families::PortHandles>()
        .cloned()
    {
        for rig in crate::surface::act::world_rigs(ctx.w) {
            let key = crate::surface::excursion::RigKey(rig);
            if let Some(out) = ctx
                .action_state(&handles.families.excursions, &key)
                .ok()
                .flatten()
                .and_then(|record| record.excursion.as_ref())
                .filter(|out| &out.site == site)
            {
                seen.insert(out.operator);
            }
        }
    }
    for who in seen {
        ctx.out.sys_to_actor(who, "tacsim.field", text.to_owned());
    }
}

/// **The field host's registration**: its RAM book, `+field`, its tick and the clock observer.
///
/// # Errors
///
/// A registration the registrar refused.
pub fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.state(FieldBook::default());
    reg.state(admit::OwedCloses::default());
    reg.state(known::Sightings::default());
    reg.state(exit::ExitBook::default());
    reg.state(exit::OwedExits::default());
    reg.state(crew::CrewBook::default());
    reg.state(orders::Offered::default());
    // `+field` submits the host's authoritative action: the cockpit's command lane, then the
    // module's own default restored for whatever registers after.
    reg.classify_commands(mush_core::CommandCompatibility::Authoritative);
    reg.command(
        CommandSpec {
            name: FIELD_VERB.into(),
            switches: commands::SWITCHES.iter().map(|s| (*s).into()).collect(),
            shape: ArgShape::Raw,
            help: commands::HELP.into(),
            ..CommandSpec::default()
        },
        Arc::new(commands::command),
    )?;
    reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
    // Staff seed a fixture raider (doc 32 cut CF.6, Q-CF-44): a world write, post-image.
    reg.command(raiders::spec(), Arc::new(raiders::command))?;
    reg.tick(
        FIELD_TICK,
        TickCadence::Coarse(FIELD_TICK_PERIOD),
        Arc::new(admit::tick),
    )?;
    reg.subscribe(
        mush_core::module::WorldEventKind::WorldClockResumed,
        Arc::new(boot::on_resumed),
    );
    Ok(())
}

/// **The black box log of task `task`**, when it is a field's: `field.<site>.<ordinal>.<segment>`.
#[must_use]
pub fn log_of_task(w: &World, task: MatchId) -> Option<String> {
    let book = w.module_state::<FieldBook>()?;
    let site = book.site_of_task(task)?;
    book.hosted(site).map(|hosted| hosted.live.log_id())
}

/// **A field's snapshot** (the bridge's branch; Q-CF-34): the document beside the live record,
/// the live record's moment moved to it, and the RAM copy the void rule demotes from.
pub(crate) fn on_snapshot(ctx: &mut Ctx<'_>, task: MatchId, snapshot: Arc<crate::arena::Snapshot>) {
    let at_ms = snapshot.at_ms;
    let document = Arc::try_unwrap(snapshot).map_or_else(
        |shared| shared.document.clone(),
        |snapshot| snapshot.document,
    );
    let Some(book) = ctx.w.module_state_mut::<FieldBook>() else {
        return;
    };
    let Some(site) = book.site_of_task(task).cloned() else {
        return;
    };
    book.snapshots.insert(site.clone(), document.clone());
    let live = book.live.get_mut(&site).map(|hosted| {
        hosted.live.at_ms = at_ms;
        hosted.live.clone()
    });
    if let Some(registry) = ctx
        .w
        .module_state_mut::<crate::arena_bridge::ArenaRegistry>()
    {
        registry.note_snapshot(task, at_ms);
    }
    record::write_snapshot(ctx, &site, document);
    if let Some(live) = live {
        record::write_live(ctx, &live);
    }
}

/// **One pulse's frames of a field** (the bridge's branch; Q-CF-14): a fire order whose
/// shooter's sensors now hold a contact on the target's cell locks it and fires the shooter's
/// first loaded weapon, through the pilot's own seat; past [`FIRE_WINDOW_MS`] it lapses.
pub(crate) fn on_frames(
    ctx: &mut Ctx<'_>,
    task: MatchId,
    frames: &[(ObjId, crate::arena::step::Frame)],
    now_ms: i64,
) {
    // Doc 32 cut CF.4 (Q-CF-28): what each party's seated units hold on the ground.
    known::note(ctx, task, frames);
    // Doc 32 cut CF.5 (Q-CF-38): a control-section kill incapacitates its pilot at once.
    crew::on_frames(ctx, frames);
    let Some((site, pending)) = ctx.w.module_state::<FieldBook>().and_then(|book| {
        let site = book.site_of_task(task)?.clone();
        let pending = book.fire.get(&site)?.clone();
        Some((site, pending))
    }) else {
        return;
    };
    if now_ms > pending.until_ms {
        if let Some(book) = ctx.w.module_state_mut::<FieldBook>() {
            book.fire.remove(&site);
        }
        ctx.out.sys_to_actor(
            pending.pilot,
            "tacsim.field",
            "Your fire order lapsed: your sensors never held the target, and the shot never left the barrel.".to_owned(),
        );
        return;
    }
    let Some(frame) = frames
        .iter()
        .find(|(pilot, frame)| *pilot == pending.pilot && frame.unit == pending.shooter)
        .map(|(_, frame)| frame)
    else {
        return;
    };
    let near = |cell: Option<(u16, u16)>| {
        cell.is_some_and(|(x, y)| {
            x.abs_diff(pending.target_cell.0) <= 2 && y.abs_diff(pending.target_cell.1) <= 2
        })
    };
    let Some(contact) = frame.contacts.iter().find(|contact| near(contact.cell)) else {
        return;
    };
    let weapon = frame
        .weapons
        .iter()
        .find(|weapon| weapon.rounds > 0)
        .map(|weapon| weapon.id.clone());
    if let Some(book) = ctx.w.module_state_mut::<FieldBook>() {
        book.fire.remove(&site);
    }
    let lock = crate::arena::intents::IntentKind::Lock {
        contact: contact.id,
    };
    if crate::arena_bridge::forward_intent(ctx, pending.pilot, pending.shooter, lock).is_err() {
        return;
    }
    if let Some(weapon) = weapon {
        let fire = crate::arena::intents::IntentKind::Fire { weapon, slot: None };
        let _ = crate::arena_bridge::forward_intent(ctx, pending.pilot, pending.shooter, fire);
    }
}
