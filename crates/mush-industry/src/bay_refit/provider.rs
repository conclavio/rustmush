//! The bay's six actions: the supplier's offer, the requester's escrowed
//! acceptance and the exact scheduled refit transitions. Each prepares the
//! owners the kernel's `bay_refit` protocol grants it and writes exactly the
//! action's [`super::WRITE_SETS`] row.
use super::records::*;
use super::runtime::BayRuntime;
use super::{BayRefitError as Error, authority, config, owner};
use crate::workorder::{WorkOrderId, derive_work_order_id};
use mush_core::action::{ActionError, ActionErrorCode, ActionProvider, AuthoritativeActionOutput};
use mush_core::participant::ParticipantHandle;
use mush_core::{
    ActionInvocation, ActionTxn, CanonicalHash, CanonicalId, CommitToken, DefinitionRef,
    IdempotencyKey, IdempotencyKeyText, IdempotencyScope, ModError, ObjId, PerceivedRef,
    PrepareCtx, Registrar, RulesValue, ScopedIdempotencyKey,
};
use mush_economy::AccountId;
use mush_economy::bay_refit::{BayCancellationRequest, BayFundingRequest, BayInstallationRequest};
use mush_economy::registry_filing::{
    EconomyRequest, InitialFilingParticipant as EconomyParticipant,
};
use mush_inventory::bay_part::BayInstall;
use mush_inventory::{Lineage, Lot, LotId};
use mush_tactics::component_refit::{ComponentRefitParticipant, RefitRequest};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

fn bad<T>(_: T) -> Error {
    Error::Integrity
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OfferSelector {
    requester: ObjId,
    hull: ObjId,
    facility: ObjId,
    /// The target unit's definition id (`tacsim.unit.line_tank_laser`).
    target: String,
    /// The lot the part comes from, when the refit fits one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    part: Option<LotId>,
    fee: i64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AcceptSelector {
    offer: CommitToken,
    payer: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OrderSelector {
    order: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AdvanceSelector {
    pub(super) order: WorkOrderId,
    pub(super) observed: mush_core::ClockStamp,
    pub(super) restart: bool,
}
fn decode_selector<T: Serialize + serde::de::DeserializeOwned>(v: &Value) -> Result<T, Error> {
    mush_core::canonical_json_bytes_with_limit(v, 2048).map_err(bad)?;
    let s: T = serde_json::from_value(v.clone()).map_err(bad)?;
    if serde_json::to_value(&s).map_err(bad)? != *v {
        return Err(Error::Invalid);
    }
    Ok(s)
}
fn order_id(s: &str) -> Result<WorkOrderId, Error> {
    let id = WorkOrderId::parse(s).ok_or(Error::Invalid)?;
    if id.to_string() != s {
        return Err(Error::Invalid);
    }
    Ok(id)
}
fn payer(s: &str) -> Result<AccountId, Error> {
    let id = s.parse::<u64>().map_err(bad)?;
    if id.to_string() != s {
        return Err(Error::Invalid);
    }
    Ok(AccountId(id))
}
fn validate(inv: &ActionInvocation, index: usize) -> Result<(), Error> {
    if (index != 3 && inv.principal() != &(mush_core::PrincipalId::Object { id: inv.actor() }))
        || inv.subject() != &PerceivedRef::SelfActor
        || !inv.targets().is_empty()
        || inv.idempotency().scope().owner.as_str() != crate::NS
    {
        return Err(Error::Invalid);
    }
    match index {
        0 if inv.parameters() == &json!({}) => {}
        1 => {
            decode_selector::<OfferSelector>(inv.parameters())?;
        }
        2 => {
            payer(&decode_selector::<AcceptSelector>(inv.parameters())?.payer)?;
        }
        3 => {
            decode_selector::<AdvanceSelector>(inv.parameters())?;
        }
        4 | 5 => {
            order_id(&decode_selector::<OrderSelector>(inv.parameters())?.order)?;
        }
        _ => return Err(Error::Invalid),
    }
    Ok(())
}
/// The slot's holder, exactly as the kernel's `bay_refit` row derives it:
/// the bay work order in its own key domain.
pub(crate) fn holder(order: WorkOrderId) -> Result<ScopedIdempotencyKey, Error> {
    let owner = crate::record::owner();
    Ok(ScopedIdempotencyKey::new(
        IdempotencyScope {
            principal: mush_core::PrincipalId::Scoped {
                owner: owner.clone(),
                class: CanonicalId::new("workorder").map_err(bad)?,
                id: IdempotencyKeyText::parse_canonical(format!("bay{:016x}", order.0))
                    .map_err(bad)?,
            },
            owner,
            namespace: CanonicalId::new("industry").map_err(bad)?,
        },
        0,
        IdempotencyKey::Deterministic(
            CanonicalHash::for_canonical_value(
                "industry.bay_holder.v1",
                1,
                &json!({"order":order.0}),
                1024,
            )
            .map_err(bad)?,
        ),
    ))
}
/// The lot the part taken out becomes: derived from the offer, so the
/// kernel can reserve its key at Accept.
fn returned_lot(offer: CommitToken) -> Result<LotId, Error> {
    let h = CanonicalHash::for_canonical_value(
        "industry.bay_refit.returned_lot.v1",
        1,
        &json!({"offer":offer}),
        1024,
    )
    .map_err(bad)?;
    let mut bytes = [0_u8; 8];
    bytes.copy_from_slice(&h.as_bytes()[..8]);
    Ok(LotId(u64::from_be_bytes(bytes)))
}
type Tactics = ComponentRefitParticipant<super::config::BayTacticsPolicy>;
pub(crate) struct Provider {
    runtime: Arc<BayRuntime>,
    index: usize,
    owner: Option<ParticipantHandle<owner::BayOwner>>,
    economy: Option<ParticipantHandle<EconomyParticipant>>,
    tactics: Option<ParticipantHandle<Tactics>>,
    refusal: ActionError,
}
type Prepared = (&'static str, Option<CommitToken>, Option<WorkOrderId>);
impl Provider {
    fn owner(
        &self,
        ctx: &mut PrepareCtx<'_>,
        request: owner::Request,
    ) -> Result<Box<owner::Output>, Error> {
        ctx.prepare_owner(self.owner.as_ref().ok_or(Error::Unavailable)?, request)
            .map_err(bad)
    }
    fn economy(
        &self,
        ctx: &mut PrepareCtx<'_>,
        request: EconomyRequest,
    ) -> Result<mush_economy::registry_filing::EconomyOutput, Error> {
        ctx.prepare_owner(self.economy.as_ref().ok_or(Error::Unavailable)?, request)
            .map_err(bad)
    }
    fn before(
        &self,
        txn: &mut ActionTxn<'_>,
        id: WorkOrderId,
        actor: ObjId,
    ) -> Result<BayOrder, Error> {
        let row = txn
            .get(&self.runtime.families.orders, &id)
            .map_err(bad)?
            .cloned()
            .ok_or(Error::Unavailable)?;
        row.validate()?;
        if row.intent.offer.requester != actor || row.terminal.is_some() {
            return Err(Error::Unavailable);
        }
        Ok(row)
    }
    #[inline(never)]
    fn initialize(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<Prepared, Error> {
        let f = &self.runtime.families;
        if inv.actor() != mush_core::GOD
            || txn.get(&f.deployment, &()).map_err(bad)?.is_some()
            || !txn.family_is_empty(&f.offers).map_err(bad)?
            || !txn.family_is_empty(&f.orders).map_err(bad)?
        {
            return Err(Error::Unavailable);
        }
        let out = self.owner(ctx, owner::Request::Initialize)?;
        txn.insert(&f.deployment, &(), out.index).map_err(bad)?;
        Ok(("initialized", None, None))
    }
    #[inline(never)]
    fn offer(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
        commit: CommitToken,
        action: &DefinitionRef,
    ) -> Result<Prepared, Error> {
        let r = &self.runtime;
        let c = &r.config;
        let f = &r.families;
        let s: OfferSelector = decode_selector(inv.parameters())?;
        let index = txn
            .get(&f.deployment, &())
            .map_err(bad)?
            .cloned()
            .ok_or(Error::Unavailable)?;
        index.validate()?;
        if index.offers >= MAX_ORDERS || txn.get(&f.offers, &commit).map_err(bad)?.is_some() {
            return Err(Error::Capacity);
        }
        let world = ctx.world().map_err(bad)?;
        let live = authority::current(world, c, s.requester, s.hull, s.facility)?;
        let registry = c.registry()?;
        let target = c
            .roots
            .iter()
            .find(|r| r.key().id.as_str() == s.target)
            .cloned()
            .ok_or(Error::Unavailable)?;
        let unit =
            mush_tactics::component_replacement::current_refit_unit(world, s.requester, s.hull)
                .map_err(|_| Error::Unavailable)?;
        let (remove, install) = config::moves(registry, unit, &target)?;
        let stores = *c.stores.get(&s.facility).ok_or(Error::Unavailable)?;
        // The part fitted is a one-unit lot of its commodity in the bay's
        // stores; the part taken out must be a stocked commodity too.
        if s.part.is_some() != install.is_some() {
            return Err(Error::Invalid);
        }
        if let (Some(lot), Some(component)) = (s.part, &install) {
            let commodity = c
                .parts
                .commodity(&config::definition_of(registry, &target, component)?)
                .ok_or(Error::Unavailable)?;
            let row = txn
                .get(&r.industry.inventory.lots, &lot)
                .map_err(bad)?
                .cloned()
                .ok_or(Error::Unavailable)?;
            if row.endpoint() != stores || row.quantity() != 1 || row.commodity() != &commodity {
                return Err(Error::Unavailable);
            }
        }
        if let Some(component) = &remove {
            c.parts
                .commodity(&config::definition_of(registry, &live.before, component)?)
                .ok_or(Error::Unavailable)?;
        }
        let row = BayOffer {
            format: 1,
            sequence: index.offers.checked_add(1).ok_or(Error::Capacity)?,
            operation: commit,
            action: action.clone(),
            operator: inv.actor(),
            requester: s.requester,
            policy: c.policy.clone(),
            facility: s.facility,
            stores,
            hull: s.hull,
            unit_revision: live.unit_revision,
            before: live.before.clone(),
            target,
            returned: remove.as_ref().map(|_| returned_lot(commit)).transpose()?,
            remove,
            install,
            part: s.part,
            recipe: BayRecipe {
                work_units: c.work_units,
            },
            fee_minor: s.fee,
            observed_ic: live.at.clone(),
            observed_utc: live.utc,
            expires_utc: live
                .utc
                .checked_add(super::OFFER_SECONDS)
                .ok_or(Error::Capacity)?,
        };
        row.validate()?;
        let out = self.owner(ctx, owner::Request::Offer(Box::new(row.clone())))?;
        if out.index != index.offer(&row)? || out.order.is_some() {
            return Err(Error::Integrity);
        }
        txn.insert(&f.offers, &commit, row).map_err(bad)?;
        txn.replace(&f.deployment, &(), out.index).map_err(bad)?;
        Ok(("offered", Some(commit), None))
    }
    #[inline(never)]
    fn accept(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
        commit: CommitToken,
        action: &DefinitionRef,
    ) -> Result<Prepared, Error> {
        let r = &self.runtime;
        let c = &r.config;
        let f = &r.families;
        let s: AcceptSelector = decode_selector(inv.parameters())?;
        let offer = txn
            .get(&f.offers, &s.offer)
            .map_err(bad)?
            .cloned()
            .ok_or(Error::Unavailable)?;
        if offer.requester != inv.actor() {
            return Err(Error::Unavailable);
        }
        let index = txn
            .get(&f.deployment, &())
            .map_err(bad)?
            .cloned()
            .ok_or(Error::Unavailable)?;
        index.validate()?;
        let id = derive_work_order_id(inv.request(), 0).map_err(bad)?;
        if index.accepted >= MAX_ORDERS
            || txn.get(&f.orders, &id).map_err(bad)?.is_some()
            || txn
                .get(&r.industry.work_orders, &id)
                .map_err(bad)?
                .is_some()
        {
            return Err(Error::Capacity);
        }
        let at = ctx.world().map_err(bad)?.world_ic_now();
        let intent = BayIntent {
            order: id,
            offer,
            payer: payer(&s.payer)?,
            accepted_at: at.clone(),
            expires_at: mush_inventory::reservation_deadline(&at).map_err(bad)?,
        };
        authority::accept(ctx.world().map_err(bad)?, c, &intent)?;
        let record = ctx
            .world()
            .map_err(bad)?
            .module_state::<crate::IndustryStore>()
            .and_then(|st| st.facility(intent.offer.facility).cloned())
            .ok_or(Error::Unavailable)?;
        let capacity = r
            .industry
            .reserve_slot(
                txn,
                intent.offer.facility,
                record.maximum_slots(),
                &holder(id)?,
                &at,
            )
            .map_err(bad)?;
        let payment = self
            .economy(
                ctx,
                EconomyRequest::BayFunding(BayFundingRequest {
                    subject: intent.subject(&c.payment)?,
                }),
            )?
            .bay_funding()
            .map_err(bad)?;
        let row = BayOrder {
            format: 1,
            sequence: index.accepted.checked_add(1).ok_or(Error::Capacity)?,
            accepted: commit,
            action: action.clone(),
            progress: super::progress::BayProgress::new(id, at, intent.offer.recipe.work_units)?,
            intent,
            funding: payment.receipt().clone(),
            capacity,
            terminal: None,
            installation: None,
        };
        row.validate()?;
        super::runtime::terminal_room(ctx.world().map_err(bad)?, r, &row)?;
        let out = self.owner(
            ctx,
            owner::Request::Accept {
                order: Box::new(row.clone()),
                payment: Box::new(payment),
            },
        )?;
        if out.index != index.accept(&row)? || out.order.as_deref() != Some(&row) {
            return Err(Error::Integrity);
        }
        f.accept_heads(txn, &row)?;
        f.publish_schedule(txn, &row)?;
        if let Some(lot) = row.intent.offer.part {
            r.parts()?
                .reserve(txn, id.0, lot, row.intent.offer.stores, commit)
                .map_err(bad)?;
        }
        txn.insert(&f.orders, &id, row).map_err(bad)?;
        txn.replace(&f.deployment, &(), out.index).map_err(bad)?;
        ctx.reserve_bay_refit_completion(txn, id.0).map_err(bad)?;
        Ok(("accepted", Some(s.offer), Some(id)))
    }
    #[inline(never)]
    fn advance(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<Prepared, Error> {
        let f = &self.runtime.families;
        let s: AdvanceSelector = decode_selector(inv.parameters())?;
        let before = self.before(txn, s.order, inv.actor())?;
        if s.observed != ctx.world().map_err(bad)?.world_ic_now() {
            return Err(Error::Unavailable);
        }
        let cursor = before.progress.cursor();
        let mush_core::ScheduleEvaluation::Advance(advance) = cursor
            .plan_due(
                cursor.revision(),
                &s.observed,
                if s.restart {
                    mush_core::ScheduleDriveCause::Restart
                } else {
                    mush_core::ScheduleDriveCause::Live
                },
            )
            .map_err(bad)?
        else {
            return Err(Error::Unavailable);
        };
        let out = self.owner(
            ctx,
            owner::Request::Advance {
                before: Box::new(before),
                advance,
            },
        )?;
        let row = *out.order.ok_or(Error::Integrity)?;
        // The schedule source owns the cursor advance; the owner's reproduced
        // progress must equal its already seeded typed postimage.
        if txn
            .get(&f.schedules, &super::schedule_id(s.order).map_err(bad)?)
            .map_err(bad)?
            != Some(row.progress.cursor())
        {
            return Err(Error::Integrity);
        }
        txn.replace(&f.orders, &s.order, row).map_err(bad)?;
        ctx.advance_bay_refit_completion(txn, s.order.0)
            .map_err(bad)?;
        Ok(("advanced", None, Some(s.order)))
    }
    #[inline(never)]
    fn install(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<Prepared, Error> {
        let r = &self.runtime;
        let c = &r.config;
        let f = &r.families;
        let id = order_id(&decode_selector::<OrderSelector>(inv.parameters())?.order)?;
        let before = self.before(txn, id, inv.actor())?;
        if !before.progress.production().is_complete() {
            return Err(Error::Unavailable);
        }
        let o = before.intent.offer.clone();
        let tactics = ctx
            .prepare_owner(
                self.tactics.as_ref().ok_or(Error::Unavailable)?,
                RefitRequest::new(o.hull, o.unit_revision, id),
            )
            .map_err(bad)?;
        let payment = self
            .economy(
                ctx,
                EconomyRequest::BayInstallation(BayInstallationRequest {
                    acceptance: before.accepted,
                }),
            )?
            .bay_terminal()
            .map_err(bad)?;
        let t = payment.receipt();
        let j = tactics.journal();
        if payment.subject() != &before.intent.subject(&c.payment)?
            || tactics.commit() != t.commit()
            || tactics.action() != t.action()
            || tactics.actor() != o.requester
            || j.now != t.posted_at_utc()
        {
            return Err(Error::Integrity);
        }
        let row = BayOrder {
            terminal: Some(t.clone()),
            installation: Some(BayInstallation {
                refit: RefitEvidence::capture(j),
                participants: r.participants.clone(),
                at_utc: j.now,
            }),
            ..before
        };
        row.validate()?;
        let returned = match (&row.intent.offer.returned, &j.removed) {
            (Some(lot), Some((_, removed))) => {
                let registry = c.registry()?;
                let component = row.intent.offer.remove.as_ref().ok_or(Error::Integrity)?;
                let definition = config::definition_of(registry, &o.before, component)?;
                let commodity = c.parts.commodity(&definition).ok_or(Error::Unavailable)?;
                let grade = c.parts.grade(removed).ok_or(Error::Unavailable)?;
                Some(
                    Lot::new(
                        *lot,
                        row.intent.offer.stores,
                        commodity,
                        1,
                        grade,
                        Lineage::Origin {
                            source: mush_economy::ExternalRef {
                                provider: "industry".into(),
                                kind: "bay_refit".into(),
                                id: id.to_string().into(),
                                revision: Some(1),
                            },
                        },
                        0,
                    )
                    .map_err(bad)?,
                )
            }
            (None, None) => None,
            _ => return Err(Error::Integrity),
        };
        r.industry
            .release_slot(txn, row.intent.offer.facility, row.capacity, 1)
            .map_err(bad)?;
        f.terminal_heads(txn, &row)?;
        r.parts()?
            .install(
                txn,
                BayInstall {
                    order: id.0,
                    commit: t.commit(),
                    stores: row.intent.offer.stores,
                    consume: row.intent.offer.part.is_some(),
                    returned,
                },
            )
            .map_err(bad)?;
        txn.replace(&f.orders, &id, row).map_err(bad)?;
        ctx.advance_bay_refit_completion(txn, id.0).map_err(bad)?;
        Ok(("installed", None, Some(id)))
    }
    #[inline(never)]
    fn cancel(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<Prepared, Error> {
        let r = &self.runtime;
        let f = &r.families;
        let id = order_id(&decode_selector::<OrderSelector>(inv.parameters())?.order)?;
        let before = self.before(txn, id, inv.actor())?;
        let payment = self
            .economy(
                ctx,
                EconomyRequest::BayCancellation(BayCancellationRequest {
                    acceptance: before.accepted,
                }),
            )?
            .bay_terminal()
            .map_err(bad)?;
        let commit = payment.receipt().commit();
        let out = self.owner(
            ctx,
            owner::Request::Cancel {
                before: Box::new(before),
                payment: Box::new(payment),
            },
        )?;
        let row = *out.order.ok_or(Error::Integrity)?;
        r.industry
            .release_slot(txn, row.intent.offer.facility, row.capacity, 1)
            .map_err(bad)?;
        f.terminal_heads(txn, &row)?;
        if row.intent.offer.part.is_some() {
            r.parts()?.release(txn, id.0, commit).map_err(bad)?;
        }
        txn.replace(&f.orders, &id, row).map_err(bad)?;
        ctx.advance_bay_refit_completion(txn, id.0).map_err(bad)?;
        Ok(("cancelled", None, Some(id)))
    }
    fn prepare(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, Error> {
        validate(inv, self.index)?;
        let r = &self.runtime;
        let commit = ctx.commit_token();
        let action = r.actions.get(self.index).ok_or(Error::Integrity)?;
        let (state, offer, order) = match self.index {
            0 => self.initialize(inv, txn, ctx)?,
            1 => self.offer(inv, txn, ctx, commit, action)?,
            2 => self.accept(inv, txn, ctx, commit, action)?,
            3 => self.advance(inv, txn, ctx)?,
            4 => self.install(inv, txn, ctx)?,
            5 => self.cancel(inv, txn, ctx)?,
            _ => return Err(Error::Invalid),
        };
        Ok(AuthoritativeActionOutput::new(
            RulesValue::new(
                r.outcome.clone(),
                json!({"operation":commit,"offer":offer,"order":order,"state":state}),
            )
            .map_err(bad)?,
        )
        .with_definitions(vec![action.clone(), r.outcome.clone()])
        .with_eligible_witnesses(vec![inv.principal().clone()]))
    }
}
impl ActionProvider for Provider {
    fn validate_invocation(&self, inv: &ActionInvocation) -> Result<(), ActionError> {
        validate(inv, self.index).map_err(|_| self.refusal.clone())
    }
    fn prepare_authoritative_with_context(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        self.prepare(inv, txn, ctx)
            .map_err(|_| self.refusal.clone())
    }
}
/// Register the six providers, each with exactly the owners the kernel's
/// protocol grants its action.
pub(crate) fn register(reg: &mut Registrar<'_>, runtime: Arc<BayRuntime>) -> Result<(), ModError> {
    for (index, action) in runtime.actions.iter().cloned().enumerate() {
        let owner = if index == 4 {
            None
        } else {
            Some(reg.request_owner_participant::<owner::BayOwner>(action.clone())?)
        };
        let economy = if matches!(index, 2 | 4 | 5) {
            Some(reg.request_owner_participant::<EconomyParticipant>(action.clone())?)
        } else {
            None
        };
        let tactics = if index == 4 {
            Some(reg.request_owner_participant::<Tactics>(action.clone())?)
        } else {
            None
        };
        reg.action_provider(
            action,
            Provider {
                runtime: runtime.clone(),
                index,
                owner,
                economy,
                tactics,
                refusal: ActionError::new(
                    ActionErrorCode::ProviderRejected,
                    false,
                    "The bay refit is unavailable or changed.",
                )
                .map_err(|e| ModError::msg(e.to_string()))?,
            },
        )?;
    }
    Ok(())
}
