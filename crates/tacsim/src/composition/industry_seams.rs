//! **Doc 20 cut SM.6's industry seams, composed at the root** (§3.4, D-20-5; the
//! `industry_recipes::TacsimRecipeAuthority` shape): the traits are declared in
//! `mush-industry` and implemented here, so industry never imports tacsim.
//!
//! - `SiteOpener` — `mush_industry::SystemOpener`: a lane site's clerk is admitted to
//!   `DepositOpen` for exactly the ONE server-authored opening it makes next, in plan order
//!   (`crate::surface::opening::opening_plan`). The clerk's identity in the port map's
//!   `SiteRow` is its whole authority — there is no opener grant (D13);
//! - `SurfaceDraws` — `mush_industry::ExternalDraws`: what the surface provider has drawn
//!   from a deposit is its `tacsim.pockets` cursor's `Σ drawn_t`, read through industry's own
//!   module context (tacsim delegates its families to industry on this roster, D1).
//! - `SurfaceTenure` — `mush_industry::TenureAuthority` (doc 20 cut SM.10): the claim acts — a
//!   player's stake, renewal, release and contest re-planned from their order at the offer, and
//!   the site clerk's lapses and seeds — and tacsim's own block record staged in the same bundle.

use mush_core::{ActionTxn, ClockStamp, Engine, ModuleCtx, ObjId};
use mush_industry::{
    CurvePoint, ExternalDraws, IndustryHandleCell, OreDeposit, SystemOpener, SystemOpening,
    TenureAct, TenureAuthority, TenureOutcome,
};
use smol_str::SmolStr;

use crate::port::site::{PlanRead, clerk_site, org_principal, read_plan};
use crate::surface::opening::{
    ORE_DEPOSIT_ACCESSIBILITY, ORE_DEPOSIT_GRADE, SURFACE_BAND, SURFACE_PROVIDER,
};

/// **The site clerk's system opener** (doc 20 §3.4; the SM.6 brief's 3.11).
pub struct SiteOpener {
    industry: IndustryHandleCell,
}

impl SiteOpener {
    /// The opener over the composition root's industry cell.
    #[must_use]
    pub fn new(industry: IndustryHandleCell) -> SiteOpener {
        SiteOpener { industry }
    }
}

impl SystemOpener for SiteOpener {
    /// In order: the latched lane site whose `SiteRow.clerk` is `actor` (live), else `None`;
    /// its context and its **cached** ground (never a build: a cold or dark ground is
    /// `None`); its opening plan; the **first** row, in plan order, whose deposit is absent;
    /// and that row as a `SystemOpening` — accessibility 100, the one-point curve, both bands
    /// `surface`, grade 0, the block vault as endpoint, the gross as reserve, `external`
    /// `tacsim.surface`, and the site's issuer org as its jurisdiction (lead ruling 7).
    ///
    /// **An issuer that does not resolve to a lane org row opens nothing** (fail closed): a
    /// deposit is never re-opened (its id is its receipt's), so one opened without its
    /// jurisdiction would stay out of the Registry's contest row for good. The clerk's step
    /// reads the same resolution, records the site `Failed` and pages once.
    fn next_opening(&self, engine: &Engine, actor: ObjId) -> Option<SystemOpening> {
        let w = &engine.world;
        if crate::port_tick::gone(w, actor) {
            return None;
        }
        let deposits = self.industry.get()?.deposits.clone();
        let (map, site) = clerk_site(w, actor)?;
        let jurisdiction = org_principal(w, &site.issuer)?;
        let PlanRead::Plan(plan) = read_plan(w, &map, &site) else {
            return None;
        };
        let state = engine.action_state();
        let row = plan
            .into_iter()
            .find(|row| matches!(state.get(&deposits, &row.deposit), Ok(None)))?;
        Some(SystemOpening {
            receipt: SmolStr::new(&row.receipt),
            place: row.place,
            commodity: row.good,
            grade: ORE_DEPOSIT_GRADE,
            accessibility: ORE_DEPOSIT_ACCESSIBILITY,
            grade_class: SURFACE_BAND.to_owned(),
            reserve_class: SURFACE_BAND.to_owned(),
            curve: vec![CurvePoint {
                cumulative: 0,
                per_cycle: 1,
            }],
            endpoint: mush_inventory::EndpointRef(row.vault),
            reserve: row.gross_t,
            external: SmolStr::new_static(SURFACE_PROVIDER),
            jurisdiction: Some(jurisdiction),
        })
    }
}

/// **The surface provider's draws** (doc 20 §3.4, D11): a `tacsim.surface` deposit's
/// `tacsim.pockets` cursor total — absent is 0 (nothing drawn yet), a cursor of another
/// deposit or an unreadable one is `None` (industry then quarantines the block, fail closed).
pub struct SurfaceDraws;

impl ExternalDraws for SurfaceDraws {
    fn drawn(&self, ctx: &ModuleCtx<'_>, deposit: &OreDeposit) -> Option<u64> {
        if deposit.external().map(SmolStr::as_str) != Some(SURFACE_PROVIDER) {
            return None;
        }
        let pockets = ctx
            .w
            .module_state::<crate::port::families::PortHandles>()?
            .families
            .pockets
            .clone();
        match ctx.action_state(&pockets, &deposit.id()) {
            Ok(None) => Some(0),
            Ok(Some(cursor)) if cursor.deposit == deposit.id() => cursor.drawn_total(),
            Ok(Some(_)) => {
                tracing::error!(
                    deposit = %deposit.id(),
                    "tacsim: a pocket cursor names another deposit; its block is closed"
                );
                None
            }
            Err(error) => {
                tracing::error!(
                    deposit = %deposit.id(),
                    %error,
                    "tacsim: a pocket cursor does not read; its block is closed"
                );
                None
            }
        }
    }
}

/// **The composed tenure authority** (doc 20 §9, D-20-12; cut SM.10, D1/D2): the ONE
/// server-authored `mush_industry::TenureAct` an actor may make now — a site clerk's next system
/// act (`crate::port::site::next_system_act`: an unpaid lapse, a due lapse, a seed), or a
/// player's claim order re-planned at the offer (`crate::port::claims::order_tenure`) — and
/// tacsim's own block record staged in industry's bundle (`crate::port::claims::stage`): the one
/// place industry's bundle carries a tacsim write, made by tacsim's code on tacsim's family.
///
/// It holds the two tacsim family handles its staging writes through, filled from the port
/// handles at the first offer (an offer is always projected before its prepare, in the same
/// process), so the composition root threads no second cell; a stage with them unfilled refuses
/// (fail closed).
#[derive(Default)]
pub struct SurfaceTenure {
    handles: std::sync::OnceLock<crate::port::claims::ClaimHandles>,
}

impl SurfaceTenure {
    /// The authority, unfilled.
    #[must_use]
    pub fn new() -> SurfaceTenure {
        SurfaceTenure::default()
    }
}

impl TenureAuthority for SurfaceTenure {
    fn next_tenure(&self, engine: &Engine, actor: ObjId) -> Option<TenureAct> {
        let w = &engine.world;
        if self.handles.get().is_none()
            && let Some(handles) = w.module_state::<crate::port::families::PortHandles>()
        {
            let _ = self.handles.set(crate::port::claims::ClaimHandles {
                claims: handles.families.claims.clone(),
                pockets: handles.families.pockets.clone(),
            });
        }
        if crate::port_tick::gone(w, actor) {
            return None;
        }
        if clerk_site(w, actor).is_some() {
            let reads = crate::port::site_map::EngineReads(engine);
            return crate::port::site::next_system_act(w, &reads, actor, w.world_ic_now().at);
        }
        crate::port::claims::order_tenure(engine, actor)
    }

    fn stage(
        &self,
        txn: &mut ActionTxn<'_>,
        act: &TenureAct,
        outcome: &TenureOutcome,
        now: &ClockStamp,
    ) -> Result<(), String> {
        let Some(handles) = self.handles.get() else {
            tracing::error!(
                "tacsim: a tenure act reached its stage before any offer filled the handles"
            );
            return Err(
                "The Registry's books cannot be read right now; nothing changes.".to_owned(),
            );
        };
        crate::port::claims::stage(txn, handles, act, outcome, now)
    }
}
