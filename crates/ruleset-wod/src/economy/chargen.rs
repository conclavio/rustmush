//! Chargen and approval (08d §5.4, owner answer 15).
//!
//! Two halves, and they are deliberately far apart in time:
//!
//! - **The Residence step** ([`vacancies`], [`choose`]) runs while the player is
//!   building the sheet. It lists **vacant** staff-built properties whose tier the
//!   derived support level can carry — *names and tiers only* — and records the pick.
//!   It is **optional** (§11 answer 2, binding).
//! - **The approval artifacts** ([`on_approved`]) run inside the `wod.chargen` payload
//!   handler, and create per `personal_mode`: nothing (`off`); a
//!   [`SpendingProfile`] from Resources (`abstract`); a `wod.usd` account plus the
//!   configured starting funds minted from `system:wod.chargen` (`exact`). Then, if
//!   housing is on and a residence was picked, the tenancy through
//!   [`mush_housing::create_tenancy`] — the same entry `@tenancy/create` uses.
//!
//! **Haven dots populate [`HavenQualities`] and nothing else** (D-E5-7): they never
//! select a property, never discount rent, and never become a tier.
//!
//! ## What the Residence step may disclose, and why
//!
//! Housing's read verbs answer `No such property.` to anybody who is not a tenant,
//! guest, landlord officer or staff — which is every character in chargen. So this
//! step does **not** go through `may_view_property`; it applies its own, narrower
//! rule: a property is offered only when it is `Vacant` **and** carries no live
//! tenancy, and only its **name and tier** are printed. Nothing about a `Let` or
//! `Withdrawn` property is disclosed at all — not its name, not its existence — so
//! occupancy is never inferable, which is the property the existence-hiding rule
//! exists to protect. A vacancy on offer is the one fact this step is *for*.

use mush_core::{Ctx, ObjId};
use mush_economy::{
    EconomicCause, EconomicVisibility, EconomyPlan, IdempotencyKey, RequestedLine,
};
use mush_housing::{HousingStore, PropertyId, PropertyStatus};
use mush_org::PrincipalId;

use crate::economy::boot;
use crate::economy::config::{FAUCET_CHARGEN, PersonalMode, WodEconomyConfig};
use crate::economy::record::{HavenQualities, SpendingProfile};
use crate::economy::store::{WodEconomyStore, put_profile, put_residence, seed_eval};

/// One offered vacancy: what the step is allowed to say about a property.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Vacancy {
    /// Which.
    pub property: PropertyId,
    /// Its name.
    pub name: String,
    /// Its tier.
    pub tier: u8,
}

/// The vacancies a character at support `level` could carry (§5.4).
///
/// The band is housing's own (`assess`: `level >= tier` is `Supported`), so the step
/// offers exactly what would assess as supported on the first period — never a home
/// the character would walk into already `Strained`.
#[must_use]
pub fn vacancies(ctx: &Ctx<'_>, level: u8) -> Vec<Vacancy> {
    let Some(store) = ctx.w.module_state::<HousingStore>() else { return Vec::new() };
    store
        .properties()
        .filter(|p| p.status == PropertyStatus::Vacant)
        .filter(|p| !store.has_live_tenancy(p.id))
        .filter(|p| p.tier.get() <= level)
        .map(|p| Vacancy { property: p.id, name: p.name.to_string(), tier: p.tier.get() })
        .collect()
}

/// Record (or clear) a chargen Residence pick.
///
/// # Errors
///
/// A message when the property is not on the offered list — the step refuses a
/// property id typed in from somewhere else with the same words it would use for one
/// that does not exist, so it is not an existence oracle either.
pub fn choose(
    ctx: &mut Ctx<'_>,
    who: ObjId,
    level: u8,
    property: Option<PropertyId>,
) -> Result<Option<Vacancy>, String> {
    let Some(want) = property else {
        put_residence(ctx, who, None);
        return Ok(None);
    };
    let Some(v) = vacancies(ctx, level).into_iter().find(|v| v.property == want) else {
        return Err("That is not one of the places on offer.".to_string());
    };
    put_residence(ctx, who, Some(want));
    Ok(Some(v))
}

/// The chargen approval artifacts, per mode (§5.4). Runs inside the `wod.chargen`
/// payload handler, after the sheet is stamped Approved.
///
/// Returns the lines to show the approving staffer. It never fails the approval: a
/// character whose account could not be opened is approved with a note, because a
/// sheet that has been signed off is not un-signed by an economic hiccup.
pub fn on_approved(ctx: &mut Ctx<'_>, cfg: &WodEconomyConfig, who: ObjId) -> Vec<String> {
    let mut out = Vec::new();
    if !cfg.personal_mode.is_on() {
        // `off` creates nothing. The Residence pick, if any, is dropped rather than
        // left to become somebody's home in a deployment that has no housing layer.
        put_residence(ctx, who, None);
        return out;
    }
    let resources = crate::economy::sheet_resources(ctx.w, who).unwrap_or(0);
    let haven = crate::economy::sheet_haven(ctx.w, who);

    let mut profile = ctx
        .w
        .module_state::<WodEconomyStore>()
        .and_then(|s| s.profile(who).cloned())
        .unwrap_or_else(|| SpendingProfile::new(who, resources, cfg.default_engagement));
    profile.resources = resources;
    profile.level = resources;
    // D-E5-7: Haven is qualities. It is recorded here and read by nothing.
    profile.haven = haven.map(HavenQualities::from_dots);
    profile.note(0, "profile created at chargen approval");
    profile.revision = profile.revision.saturating_add(1);
    put_profile(ctx, profile.clone());
    seed_eval(ctx, who);
    out.push(format!("WoD economy: support profile created (level {}).", profile.level));

    if cfg.personal_mode == PersonalMode::Exact {
        match open_wallet(ctx, cfg, who) {
            Ok(Some(line)) => out.push(line),
            Ok(None) => {}
            Err(e) => out.push(format!("WoD economy: {e}")),
        }
    }
    if cfg.housing {
        // The level the **approved** sheet carries, which is what M7 re-checks the
        // Residence pick against.
        match settle_residence(ctx, who, profile.level) {
            Ok(Some(line)) => out.push(line),
            Ok(None) => {}
            Err(e) => out.push(format!("WoD housing: {e}")),
        }
    } else {
        put_residence(ctx, who, None);
    }
    out
}

/// Exact mode: a personal `wod.usd` account and the configured starting funds, minted
/// from `system:wod.chargen` through an ordinary attributed post (E4's only route).
fn open_wallet(
    ctx: &mut Ctx<'_>,
    cfg: &WodEconomyConfig,
    who: ObjId,
) -> Result<Option<String>, String> {
    let Some(instrument) =
        ctx.w.module_state::<WodEconomyStore>().and_then(|s| s.instrument().cloned())
    else {
        return Err("wod.usd is not registered; no account was opened.".to_string());
    };
    let Some(account) = boot::open_personal_account(ctx, who, &instrument) else {
        return Err("no account could be opened (the allocator is exhausted).".to_string());
    };
    if let Some(mut p) = ctx.w.module_state::<WodEconomyStore>().and_then(|s| s.profile(who).cloned())
    {
        p.account = Some(account);
        p.revision = p.revision.saturating_add(1);
        put_profile(ctx, p);
    }
    if cfg.starting_funds_major == 0 {
        return Ok(Some(format!("WoD economy: account #{account} opened (zero balance).")));
    }
    let Some(amount) = boot::major_to_amount(ctx, &instrument, cfg.starting_funds_major) else {
        return Err(format!(
            "account #{account} opened, but starting_funds_major = {} does not fit wod.usd.",
            cfg.starting_funds_major
        ));
    };
    let Some(debit) = boot::system_account(ctx, FAUCET_CHARGEN, &instrument) else {
        return Err(format!("account #{account} opened; system:wod.chargen has no account."));
    };
    let Some(negative) = amount.checked_neg() else {
        return Err(format!("account #{account} opened; the starting amount does not negate."));
    };
    let mut plan = EconomyPlan::new(
        // Deterministic, so a re-run of the approval handler cannot mint a second
        // stake: a second post under the same key resolves to `AlreadyPosted`.
        IdempotencyKey::new(
            smol_str::SmolStr::new_static("wod.chargen"),
            smol_str::SmolStr::new(who.0.to_string()),
        ),
        // **The approving staffer's name is on it**, not the faucet's: nobody acts
        // for a `System` principal, staff included (E4b1's A-lens rule), so a plan
        // whose actor is `system:wod.chargen` is refused at validation. This is the
        // same attribution `@econ/mint` carries — a seed somebody signed for.
        PrincipalId::Object(ctx.actor),
        EconomicCause::Ruleset {
            namespace: smol_str::SmolStr::new_static("wod"),
            correlation: smol_str::SmolStr::new_static("chargen.starting-funds"),
        },
        vec![
            RequestedLine {
                account: debit,
                delta: negative,
                memo: smol_str::SmolStr::new_static("starting funds"),
            },
            RequestedLine {
                account,
                delta: amount,
                memo: smol_str::SmolStr::new_static("starting funds"),
            },
        ],
    );
    plan.description = "Starting funds at character approval.".to_string();
    plan.visibility = EconomicVisibility::Parties;
    match mush_economy::post(ctx, &plan) {
        Ok(_) => Ok(Some(format!(
            "WoD economy: account #{account} opened and seeded with {} from system:wod.chargen.",
            amount.to_decimal_string(crate::economy::config::WOD_USD_SCALE)
        ))),
        Err(e) => Err(format!("account #{account} opened, but the starting funds refused: {e}")),
    }
}

/// Turn a recorded Residence pick into a live tenancy (§5.4's "approval creates the
/// tenancy through housing").
///
/// **The pick is re-checked against the approved sheet** (E5c fold M7).
/// [`vacancies`] filters `tier <= level` from the sheet *as it stood at pick time*,
/// so a player could pick a tier-5 home at Resources 5, move the dots to Contacts
/// before `+chargen/submit`, and be handed a home their approved sheet cannot carry —
/// Active → Grace → Delinquent → EvictionPending inside their first week, which is
/// exactly what this step is documented never to do. When the pick no longer fits it
/// is **dropped and reported to the approver** with the property's tier and the
/// character's level: deviation 11's shape, report and never un-sign.
fn settle_residence(ctx: &mut Ctx<'_>, who: ObjId, level: u8) -> Result<Option<String>, String> {
    let Some(property) = ctx.w.module_state::<WodEconomyStore>().and_then(|s| s.residence(who))
    else {
        return Ok(None);
    };
    let tier = ctx
        .w
        .module_state::<HousingStore>()
        .and_then(|s| s.property(property))
        .map_or_else(|| "?".to_string(), |p| p.tier.get().to_string());
    if !vacancies(ctx, level).iter().any(|v| v.property == property) {
        // Consumed either way — a pick that cannot be honoured is a staff question
        // now, not a surprise home three approvals later.
        put_residence(ctx, who, None);
        return Err(format!(
            "the Residence pick (property #{property}, tier {tier}) is not one this approved \
             sheet can carry (support level {level}), or it is no longer vacant; it was dropped \
             and no tenancy was created."
        ));
    }
    let Some(cadence) = ctx.w.module_state::<WodEconomyStore>().and_then(WodEconomyStore::cadence)
    else {
        return Err("no campaign clock resolves; no tenancy was created.".to_string());
    };
    let tenants: std::collections::BTreeSet<ObjId> = [who].into_iter().collect();
    let result = mush_housing::create_tenancy(
        ctx,
        property,
        tenants,
        cadence,
        mush_housing::AccessTerms::default(),
    );
    // The pick is consumed either way: a residence that could not be created is a
    // staff question now, not a surprise home three approvals later.
    put_residence(ctx, who, None);
    match result {
        Ok(id) => Ok(Some(format!(
            "WoD housing: tenancy #{id} is active on property #{property} (tier {tier}, support \
             level {level})."
        ))),
        Err(e) => Err(format!("the chosen residence could not be let: {e}")),
    }
}
