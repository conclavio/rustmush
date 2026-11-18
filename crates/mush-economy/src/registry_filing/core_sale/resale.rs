//! **A governed core resold to a player** (doc 48 unit EX.2b; Q-48-47, and the core leg of
//! Q-48-52) **(expensive to reverse)**. The second closed plan beside CO.13's buy-back
//! ([`super`]): a seller's private offer names one buyer and a price, and the buyer's acceptance
//! is one union in which the buyer's money, both titles and the object move, or nothing does.
//!
//! * **Who buys** is the acceptance's own actor, a player, and only the one the offer names: the
//!   registered tacsim producer (`tacsim::hardware::sale::resale`) reads the standing offer and
//!   names its seller here; a third party's acceptance never reaches this leg.
//! * **The money**: the buyer's own credit account pays the whole price, **at acceptance** and
//!   only in the union that delivers the title and the core, so a buyer is never paid-up and
//!   undelivered and no hold is left to release (the escrow is the union). The price is split
//!   into the family's lines (Q-48-3's correction: a managed title pays its family's cuts): the
//!   seller's net, the venue's cut and the levy, at the composed rates.
//! * **What moves here**: the core's managed title to the buyer, and, when the offer sells a hull
//!   with the core fitted in her (Q-48-52's core leg), the hull's title too and her hold's vault,
//!   re-homed to the buyer, all in this one plan. The kernel's closed image
//!   (`mush_core::core_sale::resale`) owns the object: a stored core into the buyer's hands, a
//!   fitted core's fence.
//!
//! A closed plan whose guard (`ClosedCoreResalePlan`) admits exactly the before-title images, the
//! exact core `TitleBirth`, the composed lines and the one or two `TransferTitle`s to the buyer,
//! and nothing else. `managed_title::require_unmanaged_mutations` is untouched.
use super::*;
use crate::managed_entitlement::Recipient;
use crate::record::Vault;
use crate::{AssetRevision, VaultId};

mod install;
pub(crate) mod receipt;
pub use install::{BoundCoreResale, CoreResalePatch};
pub use receipt::{CoreResaleAuditSubject, CoreResaleReceiptAudit};

/// The resale action's id (tacsim-owned: the buyer's acceptance of a private offer).
pub const ACTION_ID: &str = "accept_core_resale";
/// **The venue's cut**, permille of the price. **Provisional**: Q-48-8's ruled sale cut (3%), a
/// constant here because EX.4's fees have not merged; the deployment may name another.
pub const VENUE_PERMILLE: u16 = 30;
/// **The levy**, permille of the price, to a system sink. **Provisional**: Q-48-8's ruled levy
/// (1%), a constant here because EX.4's fees have not merged.
pub const LEVY_PERMILLE: u16 = 10;

/// **What composition freezes** for the resale: the exact action, who takes the venue's cut and
/// the levy, and at what rates. None of it is caller input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoreResaleConfig {
    pub action: DefinitionRef,
    /// The venue's recipient (an organization or a system principal, never a person).
    pub venue: Recipient,
    /// The levy's sink (a system principal).
    pub levy: Recipient,
    pub venue_permille: u16,
    pub levy_permille: u16,
}
impl CoreResaleConfig {
    /// # Errors
    /// An action that is not the exact tacsim action of [`ACTION_ID`] at version 1, a person as
    /// the venue, a levy that is not a system sink, one account for both, or rates that leave
    /// the seller nothing.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.action.key().owner.as_str() != ACTION_OWNER
            || self.action.key().kind.as_str() != "action"
            || self.action.key().id.as_str() != ACTION_ID
            || self.action.version() != 1
        {
            return Err("core resale requires the exact tacsim action");
        }
        if matches!(self.venue.owner, PrincipalId::Object(_))
            || !matches!(self.levy.owner, PrincipalId::System(_))
            || self.venue.account == self.levy.account
        {
            return Err("core resale requires a venue and a system levy sink apart");
        }
        if u32::from(self.venue_permille) + u32::from(self.levy_permille) >= 1_000 {
            return Err("core resale cuts leave the seller nothing");
        }
        Ok(())
    }
    /// The venue's cut, the levy and the seller's net of `price` (minor units), or `None` when
    /// the price is not positive or leaves the seller nothing.
    #[must_use]
    pub fn split(&self, price: Amount) -> Option<(i64, i64, i64)> {
        let price = price.minor();
        if price <= 0 {
            return None;
        }
        let part = |permille: u16| price.checked_mul(i64::from(permille)).map(|m| m / 1_000);
        let venue = part(self.venue_permille)?;
        let levy = part(self.levy_permille)?;
        let net = price.checked_sub(venue)?.checked_sub(levy)?;
        (net > 0).then_some((venue, levy, net))
    }
}

/// **The hull a fitted core is sold with** (Q-48-52's core leg): her object, the title revision
/// the offer read and her hold's vault, re-homed to the buyer when the seller owns it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResaleHull {
    pub hull: ObjId,
    pub title_revision: u64,
    pub hold: Option<VaultId>,
}

/// The closed request the producer constructs from the standing offer: the core and the title
/// revision it read, the hull it is fitted in (if any), the seller the offer is from, both
/// accounts and the price. The buyer is the invocation's actor. Not a wire format.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegistryCoreResale {
    pub core: ObjId,
    pub title_revision: u64,
    pub hull: Option<ResaleHull>,
    pub seller: ObjId,
    pub seller_account: AccountId,
    pub buyer_account: AccountId,
    pub price: Amount,
}

/// The owner output the producer reads back: the action and the exact receipt.
pub struct CoreResaleOutput {
    action: DefinitionRef,
    /// Boxed: two post-title images, and the owner output enum holds every family's by value.
    subject: Box<CoreResaleAuditSubject>,
}
impl CoreResaleOutput {
    pub fn action(&self) -> &DefinitionRef {
        &self.action
    }
    pub fn subject(&self) -> &CoreResaleAuditSubject {
        &self.subject
    }
    pub const fn commit_token(&self) -> CommitToken {
        self.subject.commit
    }
}

fn require_action(
    config: &CoreResaleConfig,
    c: &OwnerPrepareContext<'_>,
) -> Result<(), ParticipantError> {
    let actor = c.invocation().actor();
    if config.validate().is_err()
        || c.action() != &config.action
        || c.invocation().principal() != &(mush_core::action::PrincipalId::Object { id: actor })
        || c.invocation().subject() != &mush_core::action::PerceivedRef::SelfActor
        || !c.invocation().targets().is_empty()
        || c.world().get(actor).is_none_or(|o| {
            o.kind != mush_core::ObjKind::Player || o.flags.contains(mush_core::Flags::GOING)
        })
    {
        return Err(refused());
    }
    Ok(())
}

/// **The closed op shape**: the core's title to the buyer, then the hull's when she goes with it,
/// each in no scope.
pub(crate) fn ops(core: ObjId, hull: Option<ObjId>, buyer: ObjId) -> Vec<AssetOp> {
    std::iter::once(core)
        .chain(hull)
        .map(|object| AssetOp::TransferTitle {
            object,
            to: PrincipalId::Object(buyer),
            to_scope: None,
        })
        .collect()
}
/// **The closed money lines**: the buyer pays the price; the seller is paid the net, the venue
/// its cut and the sink the levy (a zero cut has no line).
pub(crate) fn lines(config: &CoreResaleConfig, r: &RegistryCoreResale) -> Vec<RequestedLine> {
    let (venue, levy, net) = config.split(r.price).unwrap_or((0, 0, 0));
    let mut lines = vec![
        RequestedLine {
            account: r.buyer_account,
            delta: Amount::from_minor(-r.price.minor()),
            memo: "governed core resale".into(),
        },
        RequestedLine {
            account: r.seller_account,
            delta: Amount::from_minor(net),
            memo: "governed core resale proceeds".into(),
        },
    ];
    for (account, cut, memo) in [
        (
            config.venue.account,
            venue,
            "governed core resale venue cut",
        ),
        (config.levy.account, levy, "governed core resale levy"),
    ] {
        if cut > 0 {
            lines.push(RequestedLine {
                account,
                delta: Amount::from_minor(cut),
                memo: memo.into(),
            });
        }
    }
    lines
}

/// A sellable title: the seller's alone, live, unclaimed and at the revision the offer read.
fn sellable(asset: &EconomicAsset, object: ObjId, revision: u64, seller: ObjId) -> bool {
    asset.object == object
        && asset.revision == revision
        && asset.title_owner == PrincipalId::Object(seller)
        && asset.scope.is_none()
        && !asset.frozen
        && asset.stolen.is_none()
        && asset.lifecycle == AssetLifecycle::Active
        && asset.encumbrances.is_empty()
}

/// The before-images, after every precondition held: a live managed core titled to the seller
/// alone (and, for a hull leg, a live unmanaged hull titled to the seller alone, her hold the
/// seller's), the buyer's and the seller's own active accounts and the configured recipients',
/// all in one active instrument.
#[allow(clippy::type_complexity)]
fn titles<'a>(
    config: &CoreResaleConfig,
    policy: &InitialFilingPolicy,
    state: &'a EconomyStore,
    world: &World,
    buyer: ObjId,
    r: &RegistryCoreResale,
) -> Result<
    (
        &'a EconomicAsset,
        Option<(&'a EconomicAsset, Option<&'a Vault>)>,
    ),
    ParticipantError,
> {
    let live = |id: ObjId| {
        world
            .get(id)
            .is_some_and(|o| !o.flags.contains(mush_core::Flags::GOING))
    };
    if state.frozen
        || r.title_revision == 0
        || r.title_revision == u64::MAX
        || config.split(r.price).is_none()
        || r.seller == buyer
        || r.seller.0 == 0
        || r.core == r.seller
        || r.core == buyer
        || !state.is_title_issuer(&policy.issuer)
        || state.snapshot.managed_titles.core(r.core).is_none()
        || !live(r.core)
    {
        return Err(refused());
    }
    let core = state.snapshot.asset(r.core).ok_or_else(refused)?;
    if !sellable(core, r.core, r.title_revision, r.seller)
        || state.fold.seizure_receipts.held(r.core).is_some()
    {
        return Err(refused());
    }
    let hull = match &r.hull {
        None => None,
        Some(leg) => {
            if leg.hull == r.core
                || leg.hull == r.seller
                || leg.hull == buyer
                || leg.title_revision == 0
                || leg.title_revision == u64::MAX
                || !live(leg.hull)
                || state.snapshot.managed_titles.contains(leg.hull)
            {
                return Err(refused());
            }
            let title = state.snapshot.asset(leg.hull).ok_or_else(refused)?;
            if !sellable(title, leg.hull, leg.title_revision, r.seller) {
                return Err(refused());
            }
            let hold = match leg.hold {
                None => None,
                Some(id) => {
                    let vault = state.snapshot.vault(id).ok_or_else(refused)?;
                    if vault.owner != PrincipalId::Object(r.seller) || vault.scope.is_some() {
                        return Err(refused());
                    }
                    Some(vault)
                }
            };
            Some((title, hold))
        }
    };
    let accounts = [
        (r.buyer_account, PrincipalId::Object(buyer)),
        (r.seller_account, PrincipalId::Object(r.seller)),
        (config.venue.account, config.venue.owner.clone()),
        (config.levy.account, config.levy.owner.clone()),
    ];
    let distinct = accounts
        .iter()
        .map(|(id, _)| *id)
        .collect::<std::collections::BTreeSet<_>>()
        .len()
        == accounts.len();
    if !distinct {
        return Err(refused());
    }
    let mut instrument = None;
    for (id, owner) in accounts {
        let account = state.snapshot.account(id).ok_or_else(refused)?;
        let personal = matches!(owner, PrincipalId::Object(_));
        if account.owner != owner
            || (personal && account.scope.is_some())
            || account.status != AccountStatus::Active
            || instrument.is_some_and(|i| i != &account.instrument)
        {
            return Err(refused());
        }
        instrument = Some(&account.instrument);
    }
    if instrument
        .and_then(|i| state.snapshot.instrument(i))
        .is_none_or(|i| i.status != InstrumentStatus::Active)
    {
        return Err(refused());
    }
    Ok((core, hull))
}

struct ResaleAuth {
    issuer: PrincipalId,
    owners: [PrincipalId; 4],
    seller: ObjId,
    buyer: ObjId,
    lines: Vec<RequestedLine>,
    ops: Vec<AssetOp>,
    revisions: Vec<AssetRevision>,
}
impl AuthEval for ResaleAuth {
    fn authorize(&self, actor: &PrincipalId, account: &Account, delta: Amount) -> bool {
        actor == &self.issuer
            && self
                .lines
                .iter()
                .any(|line| line.account == account.id && line.delta == delta)
            && self.owners.contains(&account.owner)
    }
    fn authorize_asset(
        &self,
        actor: &PrincipalId,
        asset: Option<&EconomicAsset>,
        op: &AssetOp,
    ) -> bool {
        actor == &self.issuer
            && self.ops.contains(op)
            && asset.is_some_and(|a| {
                a.object == op.object()
                    && a.title_owner == PrincipalId::Object(self.seller)
                    && self
                        .revisions
                        .iter()
                        .any(|r| r.object == a.object && r.revision == a.revision)
            })
    }
    fn authorize_title_receipt(
        &self,
        actor: &PrincipalId,
        to: &PrincipalId,
        scope: Option<OrgScopeId>,
    ) -> bool {
        actor == &self.issuer && to == &PrincipalId::Object(self.buyer) && scope.is_none()
    }
}

fn revisions(r: &RegistryCoreResale) -> Vec<AssetRevision> {
    std::iter::once(AssetRevision {
        object: r.core,
        revision: r.title_revision,
    })
    .chain(r.hull.as_ref().map(|leg| AssetRevision {
        object: leg.hull,
        revision: leg.title_revision,
    }))
    .collect()
}

fn plan(
    config: &CoreResaleConfig,
    policy: &InitialFilingPolicy,
    (buyer, commit, at_ic): (ObjId, CommitToken, i64),
    r: &RegistryCoreResale,
) -> EconomyPlan {
    let mut plan = EconomyPlan::new(
        IdempotencyKey::new(receipt::NAMESPACE, commit.to_hex()),
        PrincipalId::System(policy.issuer.clone()),
        EconomicCause::Ruleset {
            namespace: receipt::NAMESPACE.into(),
            correlation: receipt::correlation((commit, buyer, at_ic), config, r).into(),
        },
        lines(config, r),
    );
    plan.description = "Resale of a governed core with its title".into();
    plan.asset_ops = ops(r.core, r.hull.as_ref().map(|leg| leg.hull), buyer);
    plan.expected_asset_revisions = revisions(r);
    plan
}

fn validate(
    config: &CoreResaleConfig,
    policy: &InitialFilingPolicy,
    state: &EconomyStore,
    world: &World,
    buyer: ObjId,
    r: &RegistryCoreResale,
    plan: &EconomyPlan,
) -> Result<Prepared, ParticipantError> {
    let (core, hull) = titles(config, policy, state, world, buyer, r)?;
    let auth = ResaleAuth {
        issuer: PrincipalId::System(policy.issuer.clone()),
        owners: [
            PrincipalId::Object(buyer),
            PrincipalId::Object(r.seller),
            config.venue.owner.clone(),
            config.levy.owner.clone(),
        ],
        seller: r.seller,
        buyer,
        lines: lines(config, r),
        ops: ops(r.core, r.hull.as_ref().map(|leg| leg.hull), buyer),
        revisions: revisions(r),
    };
    let mut spent = BTreeMap::new();
    if let Some(window) = state
        .snapshot
        .account(r.buyer_account)
        .and_then(|a| a.approval.as_ref())
        .and_then(|a| a.window.as_ref())
    {
        spent.insert(
            r.buyer_account,
            state.spent_in_window(r.buyer_account, world.now(), window.period_secs),
        );
    }
    let gate = state.key_gate();
    let env = ValidateEnv {
        auth: &auth,
        custody: &NoCustody,
        facts: &NoFacts,
        now: world.now(),
        ic_now: world.game_now_unix(),
        next_hold_id: state.next_hold,
        spent,
        keys: &gate,
    };
    let proof = ClosedCoreResalePlan {
        plan: plan.clone(),
        before_title: core.clone(),
        before_hull: hull.map(|(title, _)| title.clone()),
        birth: state
            .snapshot
            .managed_titles
            .core(r.core)
            .cloned()
            .ok_or_else(refused)?,
        buyer,
        lines: lines(config, r),
    };
    match crate::engine::validate_core_resale(&proof, &state.snapshot, &env) {
        PlanResult::Post(prepared) => Ok(*prepared),
        PlanResult::NeedsApproval(_) | PlanResult::AlreadyPosted(_) | PlanResult::Refused(_) => {
            Err(refused())
        }
    }
}

pub(in crate::registry_filing) use install::{bind, prepare, recheck};

impl EconomyStore {
    /// **The resale receipt of `commit`** (doc 48 unit EX.2b): the governed-core resale that
    /// commit made, once the audit is complete this boot.
    ///
    /// # Errors
    /// While the store is frozen, or the resale audit is not complete this boot.
    pub fn core_resale_receipt(
        &self,
        commit: CommitToken,
    ) -> Result<Option<&CoreResaleAuditSubject>, ReceiptAuditError> {
        if self.frozen {
            return Err(ReceiptAuditError::Incomplete);
        }
        self.fold.core_resale_receipts.get(commit)
    }
    /// **Every governed-core resale**, in commit order (doc 48 unit EX.2b).
    ///
    /// # Errors
    /// As [`Self::core_resale_receipt`].
    pub fn core_resale_receipts(
        &self,
    ) -> Result<impl ExactSizeIterator<Item = &CoreResaleAuditSubject>, ReceiptAuditError> {
        if self.frozen {
            return Err(ReceiptAuditError::Incomplete);
        }
        self.fold.core_resale_receipts.receipts()
    }
}

/// **The private closed plan proof**: exact plan equality, the exact before-title images (the
/// core's, and the hull's when she goes with it), the exact core `TitleBirth`, exactly the
/// composed lines and exactly the transfers to the buyer. No hold or custody op rides along, and
/// the hull is no managed title. Only this module constructs it.
pub(crate) struct ClosedCoreResalePlan {
    plan: EconomyPlan,
    before_title: EconomicAsset,
    before_hull: Option<EconomicAsset>,
    birth: crate::managed_title::TitleBirth,
    buyer: ObjId,
    lines: Vec<RequestedLine>,
}
impl ClosedCoreResalePlan {
    pub(crate) fn plan(&self) -> &EconomyPlan {
        &self.plan
    }
    pub(crate) fn check(
        &self,
        plan: &EconomyPlan,
        snapshot: &EconomySnapshot,
    ) -> Result<(), crate::EconomyError> {
        let object = self.before_title.object;
        let hull = self.before_hull.as_ref().map(|h| h.object);
        if plan != &self.plan
            || plan.idempotency.namespace != receipt::NAMESPACE
            || !matches!(&plan.cause,EconomicCause::Ruleset{namespace,..} if namespace==receipt::NAMESPACE)
            || snapshot.asset(object) != Some(&self.before_title)
            || snapshot.managed_titles.core(object) != Some(&self.birth)
            || hull.is_some_and(|h| {
                snapshot.asset(h) != self.before_hull.as_ref()
                    || snapshot.managed_titles.contains(h)
            })
            || plan.asset_ops != ops(object, hull, self.buyer)
            || plan.requested_lines != self.lines
            || !plan.hold_ops.is_empty()
            || !plan.custody_ops.is_empty()
        {
            return Err(crate::EconomyError::ManagedContract);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
