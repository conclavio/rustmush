//! **Whose hull** (doc 23 cut S.4c, D-23-24; doc 44 MW.22): on what authority a character
//! opens a licensed mind's claim on a hull.
//!
//! One rule, read by the Roll's admission reader ([`super::reader`]) and by the surface
//! dispatch's refusal at `go` (`tacsim::surface::dispatch::untitled_refusal`), so the two
//! cannot disagree. The hull's title must be live, and then one of three holds, tried in
//! this order:
//!
//! 1. **Title.** The hull is titled to the character, under no organisation's scope (doc 23
//!    D-23-22, S.4b's rule, unchanged).
//! 2. **Lease.** The hull is titled to a lessor, the title carries that lessor's `Lease`
//!    claim, and the contract that placed it is Active, lease-shaped, issued by the lessor,
//!    pledges this hull as the lessor's collateral and names the character a party. That is
//!    the shape `mush_economy::offer_lease` builds: Occator's rig lease, the Trust's charter.
//! 3. **Office.** The hull is titled to an organisation and the character acts for it on the
//!    title's scope under a live `roll.holder.org` grant ([`org_holder`]), the grant an
//!    organisation's licence use already asks. It moves no money, so it is asked as a read.
//!
//! Staff and assignment never substitute for any of the three, as they never substitute for
//! title (`owner_holdings::check_title`).
//!
//! The authority is asked live and is not recorded on the claim row: the row keeps the
//! title's revision, as before. A lease that ends or a grant that is revoked while the claim
//! is open closes nothing here; the Close reads no authority.
use super::super::org_holder;
use mush_core::{Flags, ObjId, ObjKind, World};
use mush_economy::{
    AssetLifecycle, ContractId, ContractState, Deliverable, EconomicAsset, EconomicContract,
    EconomyStore, Encumbrance,
};
use mush_org::{OrgId, OrgScopeId, PrincipalId};
use serde::Serialize;
use std::any::TypeId;

/// **The arm that admitted the hull** (the module note's three).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "by", rename_all = "snake_case")]
pub enum HullAuthority {
    /// The character's own unscoped title.
    Title,
    /// A live lease from the hull's titled owner, naming the character a tenant.
    Lease {
        /// The lease contract.
        contract: ContractId,
        /// The lessor: the hull's titled owner.
        lessor: PrincipalId,
    },
    /// The character acts for the organisation the hull is titled to.
    Office {
        /// The organisation.
        org: OrgId,
        /// The title's scope, the scope the grant was asked on (the root scope when `None`).
        scope: Option<OrgScopeId>,
    },
}

/// **Why a character may not open a claim on a hull.**
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HullRefusal {
    /// The economy is not composed or is frozen, or the title is frozen, reported stolen or
    /// not Active, or the hull is not a live object: nobody flies a licensed mind on it now.
    Unavailable,
    /// The hull has no title.
    Untitled,
    /// The hull is another's, and the character holds neither a lease of it nor an office
    /// with its organisation.
    Anothers {
        /// The title's holder.
        owner: PrincipalId,
    },
}

/// **The hull's live title and the arm that admits `actor`** (the module note).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HullAdmission {
    /// The title's revision when it was read.
    pub revision: u64,
    /// The arm.
    pub authority: HullAuthority,
}

/// **Whether `actor` may open a claim on `hull`, and by which arm.**
///
/// # Errors
///
/// [`HullRefusal`]: the title cannot be flown under now, there is none, or it is another's
/// and `actor` holds no lease of the hull and no office with its organisation.
pub fn hull_authority(
    world: &World,
    hull: ObjId,
    actor: ObjId,
) -> Result<HullAdmission, HullRefusal> {
    if world
        .module_state_owner(TypeId::of::<EconomyStore>())
        .is_none_or(|owner| owner.as_str() != "economy")
    {
        return Err(HullRefusal::Unavailable);
    }
    let economy = world
        .module_state::<EconomyStore>()
        .ok_or(HullRefusal::Unavailable)?;
    let snapshot = economy.snapshot();
    let title = snapshot.asset(hull).ok_or(HullRefusal::Untitled)?;
    if economy.is_frozen()
        || world
            .get(hull)
            .is_none_or(|row| row.kind != ObjKind::Thing || row.flags.contains(Flags::GOING))
    {
        return Err(HullRefusal::Unavailable);
    }
    let authority = decide(
        title,
        hull,
        actor,
        |lessor, contract| {
            snapshot
                .contract(contract)
                .is_some_and(|lease| live_lease(lease, hull, lessor, actor))
        },
        |org, scope| {
            org_holder::scoped_actor_amount(world, org, scope, actor, org_holder::READ_AMOUNT)
                .is_ok()
        },
    )?;
    Ok(HullAdmission {
        revision: title.revision,
        authority,
    })
}

/// **The rule, pure** (the module note): `leased(lessor, contract)` answers whether that
/// contract is a live lease of the hull from that lessor to the actor ([`live_lease`]);
/// `office(org, scope)` whether the actor acts for the organisation on that scope.
fn decide(
    title: &EconomicAsset,
    hull: ObjId,
    actor: ObjId,
    leased: impl Fn(&PrincipalId, ContractId) -> bool,
    office: impl Fn(OrgId, Option<OrgScopeId>) -> bool,
) -> Result<HullAuthority, HullRefusal> {
    if actor == ObjId(0)
        || title.object != hull
        || title.frozen
        || title.stolen.is_some()
        || title.lifecycle != AssetLifecycle::Active
        || title.revision == 0
    {
        return Err(HullRefusal::Unavailable);
    }
    if title.title_owner == PrincipalId::Object(actor) && title.scope.is_none() {
        return Ok(HullAuthority::Title);
    }
    let lease = title.encumbrances.iter().find_map(|claim| match claim {
        Encumbrance::Lease { lessor, contract }
            if *lessor == title.title_owner && leased(lessor, *contract) =>
        {
            Some(*contract)
        }
        _ => None,
    });
    if let Some(contract) = lease {
        return Ok(HullAuthority::Lease {
            contract,
            lessor: title.title_owner.clone(),
        });
    }
    if let PrincipalId::Organization(org) = title.title_owner
        && office(org, title.scope)
    {
        return Ok(HullAuthority::Office {
            org,
            scope: title.scope,
        });
    }
    Err(HullRefusal::Anothers {
        owner: title.title_owner.clone(),
    })
}

/// **Whether `lease` is a live lease of `hull` from `lessor` to `actor`**: Active (an offer
/// not yet accepted, a disputed lease and an ended one are not), lease-shaped
/// (`mush_economy::is_lease_shaped`), issued by the lessor, pledging the hull as the lessor's
/// collateral, with the actor a party who is not the lessor.
fn live_lease(lease: &EconomicContract, hull: ObjId, lessor: &PrincipalId, actor: ObjId) -> bool {
    let tenant = PrincipalId::Object(actor);
    lease.state == ContractState::Active
        && lease.issuer == *lessor
        && tenant != *lessor
        && lease.consent.parties.contains(&tenant)
        && mush_economy::is_lease_shaped(lease)
        && lease.consent.terms.deliverables.iter().any(|deliverable| {
            matches!(
                deliverable,
                Deliverable::Collateral { object, from } if *object == hull && from == lessor
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const HULL: ObjId = ObjId(21);
    const PILOT: ObjId = ObjId(7);
    const HOUSE: OrgId = OrgId(3);

    fn title(owner: PrincipalId, scope: Option<OrgScopeId>) -> EconomicAsset {
        EconomicAsset {
            object: HULL,
            title_owner: owner,
            scope,
            assignment: None,
            maintenance_account: None,
            catalog: None,
            encumbrances: vec![],
            acquisition: None,
            lifecycle: AssetLifecycle::Active,
            last_journal_seq: 1,
            revision: 4,
            stolen: None,
            frozen: false,
            frozen_reason: None,
        }
    }
    fn no_lease(_: &PrincipalId, _: ContractId) -> bool {
        false
    }
    fn no_office(_: OrgId, _: Option<OrgScopeId>) -> bool {
        false
    }
    fn house() -> PrincipalId {
        PrincipalId::Organization(HOUSE)
    }

    /// The first arm is S.4b's rule: the pilot's own unscoped title, and nothing is asked of
    /// a lease or an office.
    #[test]
    fn s4c_a_personal_unscoped_title_admits_and_asks_nothing_else() {
        let asked = |_: &PrincipalId, _: ContractId| -> bool { panic!("a lease was asked") };
        let office = |_: OrgId, _: Option<OrgScopeId>| -> bool { panic!("an office was asked") };
        assert_eq!(
            decide(
                &title(PrincipalId::Object(PILOT), None),
                HULL,
                PILOT,
                asked,
                office
            ),
            Ok(HullAuthority::Title)
        );
    }

    /// A pilot with neither a title, a lease nor an office is refused, and the refusal names
    /// the title's holder: another character's hull, a lessor's with no lease, an
    /// organisation's with no grant.
    #[test]
    fn s4c_no_title_no_lease_and_no_office_is_refused_naming_the_holder() {
        for owner in [PrincipalId::Object(ObjId(11)), house()] {
            assert_eq!(
                decide(
                    &title(owner.clone(), None),
                    HULL,
                    PILOT,
                    no_lease,
                    no_office
                ),
                Err(HullRefusal::Anothers { owner })
            );
        }
    }

    /// The lease arm reads the title's own `Lease` claim: the lessor must be the title's
    /// holder, and the contract must answer as a live lease to the pilot. A claim by anyone
    /// else, a plain collateral pledge or a lien is not a lease.
    #[test]
    fn s4c_a_lease_admits_only_by_the_titled_lessors_own_live_claim() {
        let scope = Some(OrgScopeId(9));
        let lease = ContractId(40);
        let live =
            |lessor: &PrincipalId, contract: ContractId| *lessor == house() && contract == lease;
        let mut leased = title(house(), scope);
        leased.encumbrances = vec![
            Encumbrance::Collateral {
                contract: ContractId(39),
            },
            Encumbrance::Lease {
                lessor: house(),
                contract: lease,
            },
        ];
        assert_eq!(
            decide(&leased, HULL, PILOT, live, no_office),
            Ok(HullAuthority::Lease {
                contract: lease,
                lessor: house()
            })
        );
        // The contract does not answer (ended, disputed, another tenant's): no lease.
        assert_eq!(
            decide(&leased, HULL, PILOT, no_lease, no_office),
            Err(HullRefusal::Anothers { owner: house() })
        );
        // A `Lease` claim naming a lessor who does not hold the title is not this hull's lease.
        let mut crossed = title(house(), scope);
        crossed.encumbrances = vec![Encumbrance::Lease {
            lessor: PrincipalId::Organization(OrgId(8)),
            contract: lease,
        }];
        let any = |_: &PrincipalId, _: ContractId| true;
        assert_eq!(
            decide(&crossed, HULL, PILOT, any, no_office),
            Err(HullRefusal::Anothers { owner: house() })
        );
        // A pledge that is not a lease never admits, whatever the contract says.
        let mut pledged = title(house(), scope);
        pledged.encumbrances = vec![Encumbrance::Collateral { contract: lease }];
        assert_eq!(
            decide(&pledged, HULL, PILOT, any, no_office),
            Err(HullRefusal::Anothers { owner: house() })
        );
    }

    /// The office arm asks the grant on the title's own scope, for an organisation's title
    /// only: a character's hull has no office to hold.
    #[test]
    fn s4c_an_office_admits_on_the_titles_scope_for_an_organisations_hull_only() {
        let scope = Some(OrgScopeId(9));
        let granted = |org: OrgId, asked: Option<OrgScopeId>| org == HOUSE && asked == scope;
        assert_eq!(
            decide(&title(house(), scope), HULL, PILOT, no_lease, granted),
            Ok(HullAuthority::Office { org: HOUSE, scope })
        );
        // The same grant does not reach a title on another scope of the organisation.
        assert_eq!(
            decide(
                &title(house(), Some(OrgScopeId(10))),
                HULL,
                PILOT,
                no_lease,
                granted
            ),
            Err(HullRefusal::Anothers { owner: house() })
        );
        let office = |_: OrgId, _: Option<OrgScopeId>| -> bool { panic!("an office was asked") };
        assert_eq!(
            decide(
                &title(PrincipalId::Object(ObjId(11)), None),
                HULL,
                PILOT,
                no_lease,
                office
            ),
            Err(HullRefusal::Anothers {
                owner: PrincipalId::Object(ObjId(11))
            })
        );
    }

    /// A title nobody flies under now is refused before any arm is tried, the pilot's own
    /// included: frozen, reported stolen, not Active, unrevisioned, or another object's.
    #[test]
    fn s4c_a_title_that_is_not_live_admits_nobody() {
        let any_lease = |_: &PrincipalId, _: ContractId| true;
        let any_office = |_: OrgId, _: Option<OrgScopeId>| true;
        let own = || title(PrincipalId::Object(PILOT), None);
        let mut frozen = own();
        frozen.frozen = true;
        let mut retired = own();
        retired.lifecycle = AssetLifecycle::WrittenOff;
        let mut unrevisioned = own();
        unrevisioned.revision = 0;
        let mut other = own();
        other.object = ObjId(22);
        for bad in [frozen, retired, unrevisioned, other] {
            assert_eq!(
                decide(&bad, HULL, PILOT, any_lease, any_office),
                Err(HullRefusal::Unavailable),
                "{bad:?}"
            );
        }
        assert_eq!(
            decide(&own(), HULL, ObjId(0), any_lease, any_office),
            Err(HullRefusal::Unavailable)
        );
    }

    /// A personal title under an organisation's scope is not the pilot's own (economy
    /// invariant 18 forbids it; the rule does not rely on that).
    #[test]
    fn s4c_a_scoped_title_is_never_a_personal_one() {
        assert_eq!(
            decide(
                &title(PrincipalId::Object(PILOT), Some(OrgScopeId(9))),
                HULL,
                PILOT,
                no_lease,
                no_office
            ),
            Err(HullRefusal::Anothers {
                owner: PrincipalId::Object(PILOT)
            })
        );
    }
}
