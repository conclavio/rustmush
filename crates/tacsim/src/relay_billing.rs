//! R8b composition: actual payer authority, institutional identity and economy
//! obligations. Government recognition uses existing checked port-map identities.
use mush_core::{Ctx, ObjId, World};
use mush_economy::{
    AccountId, EconomyStore, Obligation, ObligationId, ObligationStatus, PostOutcome,
};
use mush_propagation::{
    billing::{Subscription, Tariff, View},
    service::Principal,
};
use serde::Deserialize;
pub const OWNER: &str = "relay.billing";
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Seed {
    pub tariff: Tariff,
    pub governments: Vec<Government>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Government {
    pub port: String,
    pub institution: String,
    pub government: String,
}
fn seed() -> Result<Seed, String> {
    let value: serde_json::Value =
        serde_json::from_str(crate::relay_service::JSON).map_err(|e| e.to_string())?;
    let seed: Seed = serde_json::from_value(value["billing"].clone()).map_err(|e| e.to_string())?;
    seed.tariff.validate()?;
    if seed.governments.is_empty()
        || seed.governments.len() > 8
        || seed
            .governments
            .iter()
            .any(|g| g.government.is_empty() || g.government.len() > 64)
    {
        return Err("invalid recognized government authorities".into());
    }
    Ok(seed)
}
fn owner(p: &Principal) -> mush_org::PrincipalId {
    match p {
        Principal::Object(id) => mush_org::PrincipalId::Object(*id),
        Principal::Organization(id) => mush_org::PrincipalId::Organization(mush_org::OrgId(*id)),
    }
}
pub fn represents(w: &World, actor: ObjId, p: &Principal) -> bool {
    match p {
        Principal::Object(id) => {
            *id == actor
                && w.get(*id).is_some_and(|o| {
                    o.kind.is_actor() && !o.flags.contains(mush_core::Flags::GOING)
                })
        }
        Principal::Organization(org) => w
            .module_state::<mush_org::OrgStore>()
            .and_then(|s| s.state().org(mush_org::OrgId(*org)))
            .filter(|o| o.status == mush_org::OrgStatus::Active)
            .is_some_and(|_| mush_economy::acts_for(w, actor, &owner(p))),
    }
}
pub fn government(w: &World, org: u64) -> bool {
    let Ok(seed) = seed() else {
        return false;
    };
    let Some(store) = w.module_state::<crate::store::TacsimStore>() else {
        return false;
    };
    let Ok(lane) = mush_core::CanonicalId::new(crate::definitions::content::LANE_PACK) else {
        return false;
    };
    seed.governments.iter().any(|g| {
        let (Ok(port), Ok(institution)) = (
            mush_core::CanonicalId::new(&g.port),
            mush_core::CanonicalId::new(&g.institution),
        ) else {
            return false;
        };
        let crate::port::PortMapState::Present(map) = store.port_map(&lane, &port) else {
            return false;
        };
        map.orgs.get(&institution).is_some_and(|row| row.org == org)
    })
}
fn obligation<'a>(w: &'a World, s: &Subscription) -> Result<&'a Obligation, String> {
    let store = w
        .module_state::<EconomyStore>()
        .ok_or("Economy unavailable")?;
    let o = store
        .snapshot()
        .obligation(ObligationId(
            s.obligation.ok_or("Subscription is only a quote")?,
        ))
        .ok_or("Subscription obligation missing")?;
    let market = mush_propagation::commerce::current(w).ok_or("Exchange missing")?;
    let tariff = &mush_propagation::billing::current(w)
        .ok_or("Subscription tariff missing")?
        .tariff;
    let terms = o
        .service_terms
        .as_ref()
        .ok_or("Subscription service terms missing")?;
    if o.cause
        != (mush_economy::ObligationCause::Ruleset {
            namespace: OWNER.into(),
        })
        || o.debit_account != AccountId(s.payer)
        || o.credit_account != AccountId(market.payee)
        || o.amount.minor() != s.amount_minor
        || o.schedule.clock != mush_economy::EconomicClock::WorldIc
        || o.schedule.cadence != Some(tariff.period_secs)
        || o.grace.periods != 1
        || terms.until_ic != s.until_ic
        || terms.cured_misses > o.missed_total
        || o.minted_by.is_some()
    {
        return Err("Subscription obligation/terms mismatch".into());
    }
    Ok(o)
}
pub fn subscription(w: &World, s: &Subscription) -> Result<View, String> {
    let o = obligation(w, s)?;
    let terms = o
        .service_terms
        .as_ref()
        .ok_or("Service terms unavailable")?;
    let terminal = matches!(
        o.status,
        ObligationStatus::Cancelled | ObligationStatus::Satisfied
    ) || s.until_ic.is_some_and(|end| w.world_ic_now().at >= end);
    let inactive = match &s.principal {
        Principal::Organization(org) => w
            .module_state::<mush_org::OrgStore>()
            .and_then(|s| s.state().org(mush_org::OrgId(*org)))
            .is_none_or(|o| o.status != mush_org::OrgStatus::Active),
        _ => false,
    };
    Ok(View {
        paid: o.settled_total > 0 || terms.cured_misses > 0,
        missed: o.missed_total,
        cured: terms.cured_misses,
        paused: (!terminal && matches!(o.status, ObligationStatus::Suspended { .. })) || inactive,
        terminal,
    })
}
pub fn enroll(ctx: &mut Ctx<'_>, s: &Subscription, t: &Tariff) -> Result<u64, String> {
    if !represents(ctx.w, ctx.actor, &s.principal) {
        return Err("No represented principal".into());
    }
    if s.class == mush_propagation::service::Class::Sovereign
        && !matches!(&s.principal,Principal::Organization(org) if government(ctx.w,*org))
    {
        return Err("No government authority".into());
    }
    crate::relay_commerce::payer(ctx.w, ctx.actor, s.payer)?;
    let payee = mush_propagation::commerce::current(ctx.w)
        .ok_or("Exchange missing")?
        .payee;
    let now = ctx.w.game_now_unix();
    let spec = mush_economy::ObligationSpec {
        debit_account: AccountId(s.payer),
        credit_account: AccountId(payee),
        amount: mush_economy::Amount::from_minor(s.amount_minor),
        schedule: mush_economy::EconomicSchedule::new(
            mush_economy::EconomicClock::WorldIc,
            now,
            Some(t.period_secs),
            2,
        ),
        grace: mush_economy::GracePolicy::new(1),
        notify_on_success: false,
        cause: mush_economy::ObligationCause::Ruleset {
            namespace: OWNER.into(),
        },
    };
    mush_economy::create_service_obligation(ctx, spec, s.until_ic)
        .map(|id| id.0)
        .map_err(|e| format!("Enrollment refused: {e}"))
}
pub fn cancel(ctx: &mut Ctx<'_>, s: &Subscription) -> Result<(), String> {
    let o = obligation(ctx.w, s)?.clone();
    if matches!(
        o.status,
        ObligationStatus::Cancelled | ObligationStatus::Satisfied
    ) {
        return Ok(());
    }
    mush_economy::cancel_obligation(ctx, o.id, OWNER)
        .map_err(|e| format!("Cancellation refused: {e}"))
}
pub fn cure(ctx: &mut Ctx<'_>, s: &Subscription, missed: u32, cured: u32) -> Result<u64, String> {
    let id = obligation(ctx.w, s)?.id;
    match mush_economy::cure_service_obligation(ctx, id, OWNER, missed, cured)
        .map_err(|e| format!("Cure refused; obtain a current quote: {e}"))?
    {
        PostOutcome::Posted(tx) | PostOutcome::AlreadyPosted(tx) => Ok(tx.0),
        PostOutcome::NeedsApproval(id) => Err(format!(
            "Account approval #{id} required; approve and retry this cure quote."
        )),
    }
}
pub fn notice(ctx: &mut Ctx<'_>, p: &Principal, text: String) {
    let principal = owner(p);
    for actor in mush_economy::principal_holders(ctx.w, &principal) {
        mush_economy::deliver_notice_for(ctx, actor, &principal, text.clone());
    }
}
pub fn boot(ctx: &mut Ctx<'_>) -> Result<(), String> {
    if mush_propagation::commerce::current(ctx.w).is_none() {
        return Ok(());
    }
    let seed = seed()?;
    let book = mush_propagation::billing::current(ctx.w);
    let store = ctx
        .w
        .module_state::<EconomyStore>()
        .ok_or("Economy missing")?;
    let owned: Vec<_> = store
        .snapshot()
        .service_obligations(OWNER)
        .take(mush_propagation::billing::MAX_SUBSCRIPTIONS + 1)
        .collect();
    if owned.len() > mush_propagation::billing::MAX_SUBSCRIPTIONS {
        return Err("Relay subscription obligation bound exceeded".into());
    }
    for o in owned {
        if book.is_none_or(|b| {
            !b.subscriptions
                .values()
                .any(|s| s.obligation == Some(o.id.0))
        }) {
            return Err("Orphaned relay subscription obligation; restore committed records".into());
        }
    }
    if let Some(book) = book {
        for s in book
            .subscriptions
            .values()
            .filter(|s| s.obligation.is_some())
        {
            obligation(ctx.w, s)?;
        }
        for org in book.treaties.keys() {
            if !government(ctx.w, *org) {
                return Err("Government treaty identity drift".into());
            }
        }
    }
    mush_propagation::billing::install(ctx, seed.tariff)
}
