//! The world-attached halves of E5a's arrows (E5b, 08d §4).
//!
//! Every state change a command or the tick makes goes through one of these, so the
//! things that must happen *with* a transition — the `Obj.home` invariant, the
//! property's `Let` latch, the story job, the private notice — happen in one place
//! and cannot be half-applied by a second caller who forgot one.
//!
//! Three of them are worth reading before anything else:
//!
//! - [`activate`] and [`end`] are the two ends of **D-E5-11**. `Obj.home` follows the
//!   tenancy 1:1 because core's destroy dispersal honours no hook: it moves a
//!   destroyed room's contents to each object's home through a raw `move_to`, so the
//!   only way an evicted tenant does not land back inside the house they were evicted
//!   from is for the pointer to have been cleared when the tenancy ended.
//! - [`end`] **re-homes** after it unhomes (E5a fold, A5): clearing is per object, and
//!   an object can be a tenant of a second live tenancy elsewhere. The library's
//!   carve-out catches the common case; this catches the rest.
//! - [`close_window`] is S8's other half. A tenancy that recovers out of
//!   `EvictionPending` parks its window in `last_window`, and the job and escalation
//!   ids in it are a live ST ticket and a live staff page about a home that is now
//!   fine. Both are closed here, or a staffer is left holding a decision that has
//!   already made itself.

use std::collections::BTreeSet;

use mush_core::{Ctx, GOD, ObjId, World};
use mush_economy::{EscalationKind, EscalationSubject};

use crate::ids::TenancyId;
use crate::module::{
    HousingStore, STORY_QUEUE, live_rooms, put_property, put_tenancy, render_actor,
};
use crate::record::{Funding, Property, PropertyStatus, Tenancy, TenancyStatus, TenancyStatusKind};

/// How deep the ending sweep walks a property's contents — a lamp, in a bag, in a
/// coat, on a shelf (verify fold, H5).
///
/// The sweep used to look one level down, at `contents_of(room)` and no further, so
/// anything a tenant was *carrying* kept its `Obj.home` pointing into a house that
/// was no longer theirs — and core's destroy dispersal, which honours no hook, would
/// have posted it back through the front door.
const END_SWEEP_DEPTH: usize = 6;
/// The most objects one ending sweep will look at, in each of its two halves. It is
/// the same bound the economy puts on its own contents walks: a world-sized scan
/// belongs in a tick handler only when it cannot run away.
const END_SWEEP_MAX: usize = 4_096;

/// Stamp a brand-new tenancy's assessment cursor at the clock's **current** position
/// (08d §4's pull model).
///
/// Without it the first tick fire would find an unseeded cursor sitting at position
/// 0 while the campaign clock is at 900, and either assess nine hundred periods at
/// once or — with the tick's own seeding step — spend the first cadence advance
/// after creation doing nothing. Seeding at the moment of creation makes the rule
/// exactly what it reads as: **the next period the clock advances is the first one
/// this tenancy is assessed on.**
pub(crate) fn seed_assessment(ctx: &mut Ctx<'_>, t: &Tenancy) {
    let position = crate::module::cadence_now(ctx.w, t.cadence).unwrap_or(0);
    crate::module::put_assess(
        ctx,
        t.id,
        crate::module::AssessState {
            position,
            period: 0,
            unknown_periods: 0,
            seeded: true,
        },
    );
}

/// **Create a live abstract tenancy** (E5c, 08d §5.4's chargen approval).
///
/// `@tenancy/create` is the staff path and calls this; `ruleset-wod`'s chargen
/// approval is the other caller, because a Residence chosen at character creation
/// becomes somebody's home the moment staff approve the sheet and there is no
/// staffer typing a verb inside a payload handler. One creator, so the four
/// preconditions below — and the "the staff act records both consents" rule, and the
/// seeding, and the `Obj.home` invariant — cannot drift between them.
///
/// Abstract only: an **exact** tenancy stands on a lease contract whose activation is
/// the tenant's own `@contract/accept` (08d §4.1), which is not something a caller
/// can do on their behalf.
///
/// The tenancy goes straight to `Active`: in abstract mode the staff (or approval)
/// act that creates it is both consents recorded at once — 08d §4's command list has
/// no accept verb, and §5.4's approval "creates the tenancy (`Pending → Active`)".
///
/// # Errors
///
/// [`HousingError::NoSuchProperty`], [`HousingError::Withdrawn`],
/// [`HousingError::AlreadyLet`], [`HousingError::NoSuchCadence`],
/// [`HousingError::Exhausted`], [`HousingError::NotLoaded`], plus whatever
/// [`Tenancy::new`] and [`crate::accept`] refuse (the tenant caps).
pub fn create_tenancy(
    ctx: &mut Ctx<'_>,
    property: crate::ids::PropertyId,
    tenants: BTreeSet<ObjId>,
    cadence: mush_economy::CadenceId,
    terms: crate::record::AccessTerms,
) -> Result<TenancyId, crate::HousingError> {
    {
        let store = ctx
            .w
            .module_state::<HousingStore>()
            .ok_or(crate::HousingError::NotLoaded)?;
        let p = store
            .property(property)
            .ok_or(crate::HousingError::NoSuchProperty(property))?;
        if p.status == PropertyStatus::Withdrawn {
            return Err(crate::HousingError::Withdrawn(property));
        }
        if store.has_live_tenancy(property) {
            return Err(crate::HousingError::AlreadyLet(property));
        }
    }
    // A clock that does not resolve makes a home the tick freezes and pages about
    // forever (fold T6). Refuse it here instead.
    if !ctx
        .w
        .module_state::<mush_economy::EconomyStore>()
        .is_some_and(|e| e.snapshot().cadence(cadence).is_some())
    {
        return Err(crate::HousingError::NoSuchCadence);
    }
    let id = {
        let store = ctx
            .w
            .module_state_mut::<HousingStore>()
            .ok_or(crate::HousingError::NotLoaded)?;
        store
            .alloc_tenancy()
            .ok_or(crate::HousingError::Exhausted("tenancy-id"))?
    };
    let mut t = Tenancy::new(id, property, tenants.clone(), cadence)?;
    t.terms = terms;
    let now = crate::module::cadence_now(ctx.w, cadence).unwrap_or(0);
    let live = crate::accept(&t, true, &tenants, now)?;
    put_tenancy(ctx, live.clone());
    seed_assessment(ctx, &live);
    activate(ctx, &live);
    crate::module::persist_meta(ctx);
    Ok(id)
}

/// Point every tenant's `Obj.home` at the property's anchor, and latch the property
/// `Let` (08d §3.3, D-E5-11).
///
/// `homed` was computed by E5a's `accept` and is the record of exactly which pointers
/// housing set — never a copy of the roster, which is why a co-tenant who later leaves
/// still has theirs cleared at ending time.
pub(crate) fn activate(ctx: &mut Ctx<'_>, t: &Tenancy) {
    let Some(p) = ctx
        .w
        .module_state::<HousingStore>()
        .and_then(|s| s.property(t.property).cloned())
    else {
        return;
    };
    for who in &t.homed {
        if let Err(error) = ctx.w.set_home(*who, Some(p.anchor)) {
            // The invariant is best-effort per object and loud when it fails: a home
            // that could not be set is a dispersal that will go somewhere else, which
            // is the safe direction, but it is still a thing staff should know about.
            tracing::error!(
                tenancy = t.id.0, who = who.0, anchor = p.anchor.0, %error,
                "housing: could not point a tenant's home at the property anchor"
            );
        }
    }
    set_status(ctx, &p, PropertyStatus::Let);
}

/// Clear `Obj.home` for everything this tenancy homed, re-home anybody still on
/// another live lease, unlatch the property (08d §3.3, D-E5-11 + fold A5) — and, for
/// an exact tenancy, **end the lease it stands on** (look-back fold, rent-after-end).
///
/// The lease first, and before the property lookup: a tenancy whose property record
/// is gone (a quarantined orphan being resolved) has no homes this can clear, but it
/// still has a rent obligation, and that is the one thing an ending must never leave
/// running.
pub(crate) fn end(ctx: &mut Ctx<'_>, t: &Tenancy) {
    let ending = end_lease(ctx, t);
    report_lease_ending(ctx, t, &ending);
    let Some(p) = ctx
        .w
        .module_state::<HousingStore>()
        .and_then(|s| s.property(t.property).cloned())
    else {
        return;
    };
    let rooms = live_rooms(ctx.w, &p);
    let resolve_home = |o: ObjId| ctx.w.get(o).and_then(|x| x.home);
    // The tenancy's own roster and guest list…
    let mut clear: BTreeSet<ObjId> = crate::to_unhome_on_end(t, resolve_home, p.anchor, &rooms);
    // …plus everything homed inside it, which is how a departing tenant's belongings
    // leave with them instead of staying as somebody else's furniture. Two halves,
    // because "homed here" and "standing here" are different questions (H5):
    //
    // - the property's contents walked **transitively**, so a lamp in a pocket in the
    //   spare room is found;
    // - a bounded pass over everything whose `Obj.home` resolves inside, **wherever
    //   it is standing**, because an item its owner carried out of the house is still
    //   pointed at the house.
    let mut candidates: Vec<ObjId> = contents_deep(ctx.w, &rooms, p.anchor);
    candidates.extend(homed_inside_anywhere(ctx.w, p.anchor, &rooms));
    clear.extend(crate::homed_inside(
        candidates,
        resolve_home,
        p.anchor,
        &rooms,
    ));
    for who in &clear {
        if let Err(error) = ctx.w.set_home(*who, None) {
            tracing::warn!(tenancy = t.id.0, who = who.0, %error, "housing: could not clear a home");
        }
    }
    // A5 — **re-home whoever is still on another live lease.** Clearing is per
    // object, and somebody homed here who also rents elsewhere would otherwise be
    // left homed on nothing: the invariant is "`Obj.home` follows *a* live tenancy",
    // not "the one that just ended".
    for who in clear {
        let elsewhere = {
            let Some(store) = ctx.w.module_state::<HousingStore>() else {
                break;
            };
            store
                .live_tenancies_of_tenant(who)
                .find(|other| other.id != t.id)
                .and_then(|other| store.property(other.property).map(|op| op.anchor))
        };
        if let Some(anchor) = elsewhere
            && let Err(error) = ctx.w.set_home(who, Some(anchor))
        {
            tracing::warn!(who = who.0, anchor = anchor.0, %error, "housing: could not re-home");
        }
    }
    // The latch follows the tenancies: `Withdrawn` is a decision and is never
    // re-derived away here.
    let want = {
        let Some(store) = ctx.w.module_state::<HousingStore>() else {
            return;
        };
        if store.has_live_tenancy(p.id) {
            PropertyStatus::Let
        } else {
            PropertyStatus::Vacant
        }
    };
    set_status(ctx, &p, want);
}

/// Everything inside a property's rooms, transitively, to [`END_SWEEP_DEPTH`] and at
/// most [`END_SWEEP_MAX`] objects (H5).
///
/// Breadth-first with both a depth and a total cap, so a container cycle a corrupt
/// world could hold cannot make an ending loop forever — this runs on the command and
/// the tick path, where a hang is the world task.
fn contents_deep(w: &World, rooms: &BTreeSet<ObjId>, anchor: ObjId) -> Vec<ObjId> {
    let mut out: Vec<ObjId> = Vec::new();
    let mut frontier: Vec<ObjId> = rooms
        .iter()
        .copied()
        .chain(std::iter::once(anchor))
        .collect();
    for _ in 0..END_SWEEP_DEPTH {
        let mut next: Vec<ObjId> = Vec::new();
        for r in frontier.drain(..) {
            for c in w.contents_of(r) {
                if out.len() >= END_SWEEP_MAX {
                    tracing::warn!(anchor = anchor.0, "housing: the ending sweep hit its cap");
                    return out;
                }
                out.push(*c);
                next.push(*c);
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }
    out
}

/// Every object whose `Obj.home` resolves inside the property, wherever it is
/// standing (H5) — bounded by [`END_SWEEP_MAX`].
fn homed_inside_anywhere(w: &World, anchor: ObjId, rooms: &BTreeSet<ObjId>) -> Vec<ObjId> {
    w.iter()
        .filter(|o| matches!(o.home, Some(h) if h == anchor || rooms.contains(&h)))
        .map(|o| o.id)
        .take(END_SWEEP_MAX)
        .collect()
}

/// What ending an exact tenancy's lease came to (look-back fold, rent-after-end).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LeaseEnding {
    /// Abstract funding, or a lease that was already terminal: nothing to do.
    Nothing,
    /// The economy ended it in this step: the pledge is released and the rent is
    /// `Cancelled`.
    Ended,
    /// The economy's unwind was refused; it keeps the transition pending, the sweep
    /// retries it, and it has paged with the commands that finish it.
    Retrying,
    /// The property record is gone, so the lease could not be keyed.
    NoProperty,
    /// The economy refused the seam outright (no such contract, a key it does not
    /// own, a frozen economy). Nothing was written on its side.
    Refused(String),
}

/// **End the lease an exact tenancy stands on**, through the economy's own seam
/// ([`mush_economy::end_lease`]; look-back fold, rent-after-end).
///
/// Housing moves no money here and writes no economy record: it names the lease by
/// the two facts it offered it under — the property's landlord as lessor and its
/// anchor as the premises ([`mush_economy::LeaseKey`]) — and the economy does the
/// unwind under its own terminal-transition rule (the refund plan posts under the
/// contract's deterministic key first, then the state moves), cancelling the rent
/// obligation as part of that same step. A pending lease (the tenant never accepted)
/// is ended the same way, so a later acceptance cannot activate a lease whose tenancy
/// is gone.
///
/// Every outcome is returned rather than acted on, because the two callers want
/// different things of it: [`end`] logs and pages only when it could not be done,
/// boot's pass pages about every one it finds.
pub(crate) fn end_lease(ctx: &mut Ctx<'_>, t: &Tenancy) -> LeaseEnding {
    let Funding::Exact { contract, .. } = &t.funding else {
        return LeaseEnding::Nothing;
    };
    let contract = *contract;
    let Some((lessor, premises)) = ctx.w.module_state::<HousingStore>().and_then(|s| {
        s.property(t.property)
            .map(|p| (p.landlord.clone(), p.anchor))
    }) else {
        return LeaseEnding::NoProperty;
    };
    let key = mush_economy::LeaseKey { lessor, premises };
    match mush_economy::end_lease(ctx, contract, &key) {
        Ok(mush_economy::LeaseEnd::Ended) => LeaseEnding::Ended,
        Ok(mush_economy::LeaseEnd::AlreadyEnded) => LeaseEnding::Nothing,
        Ok(mush_economy::LeaseEnd::Retrying) => LeaseEnding::Retrying,
        Err(e) => LeaseEnding::Refused(e.to_string()),
    }
}

/// Log a lease ending, and page when housing could not have it ended: a live lease
/// behind an ended tenancy is a standing charge against somebody who no longer has
/// the home, and the only remaining route out of it is a staff act, so the page
/// carries that verb (`@contract/cancel`, E4b2-D21).
///
/// `Retrying` is **not** paged here: the economy already has, with the commands that
/// finish its own pending transition, and two pages about one contract is the thing
/// 08a decision 25 exists to prevent.
fn report_lease_ending(ctx: &mut Ctx<'_>, t: &Tenancy, ending: &LeaseEnding) {
    let Funding::Exact { contract, .. } = &t.funding else {
        return;
    };
    let contract = *contract;
    match ending {
        LeaseEnding::Nothing => {}
        LeaseEnding::Ended => {
            tracing::info!(
                tenancy = t.id.0,
                contract = contract.0,
                "housing: the tenancy's lease is ended; its rent is cancelled"
            );
        }
        LeaseEnding::Retrying => {
            tracing::warn!(
                tenancy = t.id.0,
                contract = contract.0,
                "housing: the tenancy's lease could not be unwound yet; the economy is retrying and has paged"
            );
        }
        LeaseEnding::NoProperty | LeaseEnding::Refused(_) => {
            let why = match ending {
                LeaseEnding::Refused(e) => e.clone(),
                _ => "its property record is gone, so the lease cannot be keyed".to_string(),
            };
            tracing::error!(
                tenancy = t.id.0,
                contract = contract.0,
                %why,
                "housing: an ended tenancy's lease is still live and housing could not end it; paging"
            );
            mush_economy::open_escalation(
                ctx,
                EscalationKind::BootReconciliation,
                EscalationSubject::HousingTenancy(t.id.0),
                format!(
                    "tenancy #{} has ended but lease contract #{contract} behind it is still live \
                     and housing could not end it ({why}). Its rent obligation keeps falling due \
                     until the lease ends",
                    t.id
                ),
                vec![
                    format!("@tenancy/show {}", t.id),
                    format!("@contract/show #{contract}"),
                    format!("@contract/cancel #{contract}"),
                ],
            );
        }
    }
}

/// Move a property's lifecycle latch, writing nothing when it already says that.
pub(crate) fn set_status(ctx: &mut Ctx<'_>, p: &Property, want: PropertyStatus) {
    if p.status == want || p.status == PropertyStatus::Withdrawn {
        return;
    }
    let mut next = p.clone();
    next.status = want;
    next.revision = next.revision.saturating_add(1);
    put_property(ctx, next);
}

// ---------------------------------------------------------------------------
// The ruleset's entry (D-E5-5)
// ---------------------------------------------------------------------------

/// **The one way a support plan reaches a tenancy** (08d §3.1, D-E5-5).
///
/// `ruleset-wod`'s period evaluation calls this — in code, from its own tick — and
/// **no player command does**: housing never reads a sheet, and the ruleset never
/// writes a tenancy. It is the whole of the interface between them, which is why it
/// takes an opaque [`crate::SupportLevel`] and a list of attribution labels rather
/// than anything that could carry a dot.
///
/// The two stamps come from housing, not the caller, and that matters: `asserted_at`
/// is the tenancy's own cadence position (what `@tenancy/show` prints) and
/// `asserted_period` is the module's monotonic period ordinal (what
/// [`crate::assess_at`] ages a plan against). A ruleset that supplied either could
/// make a stale plan look fresh, which is exactly what
/// [`crate::PLAN_MAX_AGE_PERIODS`] exists to prevent.
///
/// A restatement that changes nothing writes nothing at all (E5a fold, S3).
///
/// # Errors
///
/// [`crate::HousingError`] for a plan naming somebody who is not a tenant, an
/// **exact** tenancy (whose assessment is its obligation's), an ended tenancy, or a
/// source list past the cap. [`crate::HousingError::State`] when the tenancy does not
/// exist.
pub fn submit_support_plan(
    ctx: &mut Ctx<'_>,
    tenancy: TenancyId,
    resident: ObjId,
    level: crate::SupportLevel,
    sources: Vec<crate::SupportSource>,
) -> Result<(), crate::HousingError> {
    let Some(t) = tenancy_of(ctx, tenancy) else {
        return Err(crate::HousingError::State(tenancy));
    };
    let now = crate::module::cadence_now(ctx.w, t.cadence).unwrap_or(0);
    let period = ctx
        .w
        .module_state::<HousingStore>()
        .map_or(0, |s| s.assess_state(tenancy).period);
    let plan = crate::SupportPlan::new(resident, level, sources, now, period)?;
    let next = crate::submit_plan(&t, plan)?;
    if next.revision != t.revision {
        put_tenancy(ctx, next);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// The story window
// ---------------------------------------------------------------------------

/// Open the `wod-story` job a new [`TenancyStatus::EvictionPending`] deserves, and
/// record it on the window (08d §4).
///
/// The **opener is explicit** (`GOD`), never the tick's incidental actor: the opener
/// is who gets the job's status notices, and that must not be the tenant — this is an
/// ST's ticket about their home, and the tenant has their own private notice. The
/// queue's per-queue visibility is what decides who reads it.
///
/// The job title and body carry the tenancy summary **and the three commands that
/// answer it, arguments filled in** — 08a decision 25's rule, applied to the story
/// half: the ST is going to want to know what they can do, so say it.
pub(crate) fn open_story_job(ctx: &mut Ctx<'_>, t: &Tenancy) -> Option<u64> {
    let TenancyStatus::EvictionPending { window } = &t.status else {
        return None;
    };
    if window.job.is_some() {
        return window.job;
    }
    let id = t.id;
    let summary = summary(ctx, t);
    let spec = mush_jobs::OpenJob {
        queue: STORY_QUEUE.into(),
        title: format!("Tenancy #{id}: support has lapsed"),
        opener: GOD,
        text: Some(format!(
            "{summary}\n\nAutomatic processing stops here: nothing ends this home on its own.\n\
             The window is bounded in notices, not in time — the tenant keeps their key and \
             keeps walking in.\n\n\
             @tenancy/resolve {id}=extend   leave it open and reset the notice budget\n\
             @tenancy/resolve {id}=scene    start a scene on it, and decide after\n\
             @tenancy/resolve {id}=end      end it (their home anchor is cleared)\n\
             @tenancy/resolve {id}=waive    end it as forgiven, with no fault recorded\n\
             @tenancy/show {id}"
        )),
        payload: None,
        tags: vec!["housing".into()],
    };
    match mush_jobs::open_job(ctx, spec) {
        Ok(job) => Some(job),
        Err(error) => {
            tracing::error!(tenancy = id.0, %error, "housing: could not open the story job");
            None
        }
    }
}

/// Close the job and acknowledge the page a **recovered** tenancy left behind (E5a
/// fold, S8).
///
/// **Who caused it is recorded** (verify fold, T4). Recovery does not only arrive
/// from the tick: a landlord who is the rent obligation's creditor can
/// `@obligation/reinstate` it, and the observer mirrors that straight onto the ladder
/// — a legitimate recovery, and one this module deliberately still allows, because it
/// is somebody *un*-ending a home rather than ending one. The escalation it closes
/// was credited to `GOD` regardless, so `@econ/alerts` said a page about a private
/// landlord's tenant had been handled by the game itself. `ctx.actor` is the tick's
/// `GOD` on the automatic path and the real person on every other one, which is
/// exactly the distinction the alert list needs to show.
pub(crate) fn close_window(ctx: &mut Ctx<'_>, t: &Tenancy) {
    let Some(window) = t.last_window else { return };
    let by = ctx.actor;
    let who = render_actor(ctx.w, by);
    if let Some(job) = window.job {
        // The plain-job seam (look-back fold, D-LBK-6): the queue is named, a
        // payload would be refused, and a closed job is a no-op.
        match mush_jobs::resolve_plain_job(
            ctx,
            job,
            STORY_QUEUE,
            GOD,
            &format!(
                "Recovered: tenancy #{} is supported again and back to Active (recovery caused \
                 by {who}). No decision is needed.",
                t.id
            ),
        ) {
            Ok(()) => tracing::info!(
                tenancy = t.id.0,
                job,
                "housing: story job resolved (recovered)"
            ),
            // Already closed, or the jobs module is not loaded: either way there is
            // no ST left holding a stale decision, which is the property that matters.
            Err(error) => {
                tracing::debug!(tenancy = t.id.0, job, %error, "housing: story job not closed")
            }
        }
    }
    if let Some(escalation) = window.escalation {
        mush_economy::acknowledge_escalation(ctx, escalation, by);
    }
}

/// Close the ST's ticket and acknowledge the page when a **person** decides the
/// window instead (`@tenancy/resolve end|waive`, or the tenant's own `+home/leave`).
///
/// The recovery path is [`close_window`], which reads `last_window`; this one reads
/// the window that is still open on the tenancy being resolved, because a resolution
/// does not park it anywhere — the tenancy goes straight to `Ended`.
pub(crate) fn close_pending_job(ctx: &mut Ctx<'_>, t: &Tenancy, waived: bool) {
    let TenancyStatus::EvictionPending { window } = &t.status else {
        return;
    };
    let (job, escalation) = (window.job, window.escalation);
    // T4: the person who decided it, not `GOD` — this path is always somebody's
    // explicit act (`@tenancy/resolve`, or the tenant's own `+home/leave`).
    let by = ctx.actor;
    let who = render_actor(ctx.w, by);
    if let Some(job) = job {
        let note = format!(
            "Decided: tenancy #{} was {} by {who}. Nothing further is pending.",
            t.id,
            if waived { "waived" } else { "ended" }
        );
        if let Err(error) = mush_jobs::resolve_plain_job(ctx, job, STORY_QUEUE, GOD, &note) {
            tracing::debug!(tenancy = t.id.0, job, %error, "housing: story job not closed");
        }
    }
    if let Some(escalation) = escalation {
        mush_economy::acknowledge_escalation(ctx, escalation, by);
    }
}

/// A one-line description of a tenancy for a job, an alert or a staff render.
pub(crate) fn summary(ctx: &Ctx<'_>, t: &Tenancy) -> String {
    let name = ctx
        .w
        .module_state::<HousingStore>()
        .and_then(|s| s.property(t.property).map(|p| p.name.to_string()))
        .unwrap_or_else(|| format!("property #{}", t.property));
    let who: Vec<String> = t
        .tenants
        .iter()
        .map(|o| {
            ctx.w
                .get(*o)
                .map_or_else(|| format!("#{}", o.0), |x| x.name.clone())
        })
        .collect();
    format!(
        "Tenancy #{} — {name} (property #{}), {} · {} · funding {}",
        t.id,
        t.property,
        who.join(", "),
        t.status.kind().label(),
        match &t.funding {
            crate::record::Funding::Abstract { plans } => {
                format!("abstract ({} plan(s))", plans.len())
            }
            crate::record::Funding::Exact {
                contract,
                obligation,
            } => {
                format!("exact (contract #{contract}, obligation #{obligation})")
            }
        }
    )
}

/// Open the level-2 page a story window asks for once its notices are spent (08d
/// §4/§3.2, decision 25's ladder).
pub(crate) fn page_window(ctx: &mut Ctx<'_>, t: &Tenancy) -> Option<mush_economy::EscalationId> {
    let id = t.id;
    let summary = summary(ctx, t);
    mush_economy::open_escalation(
        ctx,
        EscalationKind::StoryWindow,
        EscalationSubject::HousingTenancy(id.0),
        format!(
            "{summary}. The story window has spent its notices and nobody has decided. Nothing \
             happens next — a home never ends on its own — so this needs a person"
        ),
        vec![
            format!("@tenancy/resolve {id}=extend"),
            format!("@tenancy/resolve {id}=end"),
            format!("@tenancy/resolve {id}=waive"),
            format!("@tenancy/show {id}"),
        ],
    )
}

/// A tenancy whose **campaign clock has gone away** is frozen, and used to be frozen
/// in total silence (verify fold, T6).
///
/// "An unresolvable clock never makes anything happen" is the right rule and the
/// wrong output: every housing arrow takes a position on the tenancy's own cadence,
/// so a deleted or renumbered clock stops the ladder, stops the notices and stops the
/// story window — indefinitely, with nothing anywhere saying why the home in trouble
/// went quiet. The economy's ladder dedupes by subject, so asking on every fire costs
/// one record however many times it is asked.
pub(crate) fn page_missing_cadence(ctx: &mut Ctx<'_>, t: &Tenancy) {
    let (id, cadence) = (t.id, t.cadence);
    mush_economy::open_escalation(
        ctx,
        EscalationKind::BootReconciliation,
        EscalationSubject::HousingTenancy(id.0),
        format!(
            "tenancy #{id} is assessed on cadence #{cadence}, which has no record. It is FROZEN: \
             nothing is assessed, nothing is notified and nobody is evicted, because a clock \
             nobody can read must never move a home. Restore the clock, or point the tenancy at \
             a live one"
        ),
        vec![
            "@econ/cadence list".to_string(),
            format!("@tenancy/show {id}"),
        ],
    );
}

// ---------------------------------------------------------------------------
// Notices
// ---------------------------------------------------------------------------

/// Tell every tenant of a tenancy the same thing, once each.
pub(crate) fn notify_tenants(ctx: &mut Ctx<'_>, t: &Tenancy, msg: &str) {
    for who in t.tenants.iter().copied().collect::<Vec<ObjId>>() {
        crate::notice::notify(ctx, who, msg.to_string());
    }
}

/// **One private notice per transition, and none at all on a supported period**
/// (08d §4, §14.3's exception-driven output).
///
/// A supported tenancy is the overwhelmingly common case and it says nothing,
/// forever: that is what "routine life posts nothing" means. A recovery is silent
/// too (§3.2) — the tenant already knows they paid — with one exception, the
/// recovery **out of an open story window**, because that one closes an ST ticket
/// and a staff page and the tenant was told when it opened.
pub(crate) fn announce(ctx: &mut Ctx<'_>, before: TenancyStatusKind, t: &Tenancy) {
    let after = t.status.kind();
    if before == after {
        return;
    }
    let id = t.id;
    let msg = match after {
        TenancyStatusKind::Grace => Some(format!(
            "HOME: your support for tenancy #{id} did not cover this period. Nothing has changed \
             about your home — you still live there — but a second missed period puts it behind."
        )),
        TenancyStatusKind::Delinquent => Some(format!(
            "HOME: tenancy #{id} is behind. You keep your key and your home; this is a story \
             problem, not a locked door. +home shows where it stands."
        )),
        TenancyStatusKind::EvictionPending => Some(format!(
            "HOME: tenancy #{id} has reached the end of what happens automatically. Staff have \
             been asked to look at it. **Nothing else happens on its own** — you still live \
             there, and you can still get in."
        )),
        TenancyStatusKind::Active if before == TenancyStatusKind::EvictionPending => Some(format!(
            "HOME: tenancy #{id} is supported again; the open question about it is closed."
        )),
        _ => None,
    };
    if let Some(msg) = msg {
        notify_tenants(ctx, t, &msg);
    }
    // The landlord hears about delinquency once (§3.2's "landlord notice") — an
    // object landlord only: an org or system principal has no inbox, which is E4c's
    // observer seam's problem and not this cut's.
    if after == TenancyStatusKind::Delinquent {
        let landlord = ctx
            .w
            .module_state::<HousingStore>()
            .and_then(|s| s.property(t.property).map(|p| p.landlord.clone()));
        if let Some(mush_org::PrincipalId::Object(who)) = landlord {
            crate::notice::notify(
                ctx,
                who,
                format!("HOME: tenancy #{id} on your property is behind."),
            );
        }
    }
}

/// Write a tenancy through, announcing the transition it made.
pub(crate) fn commit(ctx: &mut Ctx<'_>, before: TenancyStatusKind, next: Tenancy) {
    let after = next.status.kind();
    put_tenancy(ctx, next.clone());
    announce(ctx, before, &next);
    match (before, after) {
        // Recovery out of the window: close the ST job and the staff page S8 parked.
        (TenancyStatusKind::EvictionPending, TenancyStatusKind::Active) => close_window(ctx, &next),
        (_, TenancyStatusKind::Ended) => end(ctx, &next),
        _ => {}
    }
}

/// The ids a caller may hand [`commit`] without holding a borrow open.
#[must_use]
pub(crate) fn tenancy_of(ctx: &Ctx<'_>, id: TenancyId) -> Option<Tenancy> {
    ctx.w.module_state::<HousingStore>()?.tenancy(id).cloned()
}
