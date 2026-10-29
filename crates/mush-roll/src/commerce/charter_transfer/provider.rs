use super::*;
use crate::commerce::{
    families::CommerceFamilies,
    participant::{RootParticipant, RootRequest},
    records::{FiledRecord, InitialFilingOperation, Record},
};
use mush_core::action::{
    ActionError, ActionErrorCode, ActionInvocation, ActionProvider, AuthoritativeActionOutput,
    PerceivedRef, PrincipalId,
};
use mush_core::mechanics::RulesValue;
use mush_core::participant::ParticipantHandle;
use mush_core::{ActionTxn, PrepareCtx, Registrar};
use mush_economy::registry_filing::charter_transfer::RegistryCharterTransfer;
use mush_economy::registry_filing::{EconomyRequest, InitialFilingParticipant};
use mush_economy::{AccountId, AccountStatus, Amount};
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OfferWire {
    charter: String,
    buyer: String,
    seller_account: String,
    price_minor: String,
    valid_for_seconds: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AcceptWire {
    offer: CommitToken,
    payer: String,
}
fn decimal<T: std::str::FromStr + ToString>(s: &str) -> Result<T, PublicationError> {
    let n = s.parse::<T>().map_err(bad)?;
    if n.to_string() != s {
        return Err(PublicationError::Invalid);
    }
    Ok(n)
}
pub(super) struct Provider {
    accept: bool,
    action: DefinitionRef,
    outcome: DefinitionRef,
    retention: DefinitionRef,
    f: CommerceFamilies,
    roots: ParticipantHandle<RootParticipant>,
    economy: Option<ParticipantHandle<InitialFilingParticipant>>,
    readiness: Readiness,
    refusal: ActionError,
}
impl Provider {
    fn parse(&self, inv: &ActionInvocation) -> Result<(), PublicationError> {
        let action = if self.accept {
            protocol::accept_action()
        } else {
            protocol::offer_action()
        }
        .map_err(bad)?;
        if self.action != action
            || inv.principal() != &(PrincipalId::Object { id: inv.actor() })
            || inv.subject() != &PerceivedRef::SelfActor
            || !inv.targets().is_empty()
            || inv.idempotency().scope().owner != self.action.key().owner
        {
            return Err(PublicationError::Invalid);
        }
        if self.accept {
            let _: AcceptWire = serde_json::from_value(inv.parameters().clone()).map_err(bad)?;
        } else {
            let _: OfferWire = serde_json::from_value(inv.parameters().clone()).map_err(bad)?;
        }
        Ok(())
    }
    fn basis(
        &self,
        txn: &mut ActionTxn<'_>,
        world: &World,
        charter: CharterId,
        seller: ObjId,
    ) -> Result<(InitialFilingOperation, FiledRecord, CharterOwnershipRef), PublicationError> {
        self.readiness.check()?;
        let roll = world
            .module_state::<crate::RollStore>()
            .ok_or(PublicationError::Unavailable)?;
        roll.commerce_ready().map_err(bad)?;
        let c = txn
            .get(&self.f.charters, &charter.key())
            .map_err(bad)?
            .cloned()
            .ok_or(PublicationError::Missing)?;
        let b = txn
            .get(&self.f.operations, &c.operation.to_hex())
            .map_err(bad)?
            .cloned()
            .ok_or(PublicationError::Missing)?;
        let line = txn
            .get(&self.f.lines, &c.line.key())
            .map_err(bad)?
            .cloned()
            .ok_or(PublicationError::Missing)?;
        let filed = txn
            .get(&self.f.filed, &line.head.key())
            .map_err(bad)?
            .cloned()
            .ok_or(PublicationError::Missing)?;
        c.validate()?;
        b.validate()?;
        line.validate()?;
        filed.validate()?;
        if c.id != charter
            || b.charter != charter
            || c.operation != b.commit
            || c.line != b.line
            || line.id != c.line
            || line.charter != charter
            || filed.content.position().line != c.line
            || filed.content.position().charter != charter
            || filed.content.id() != line.head
            || b.economic.house.is_some()
        {
            return Err(PublicationError::Mismatch);
        }
        super::super::charter_pending::current::require_clear_in_txn(txn, world, &self.f, charter)?;
        let ownership = history::current_in_txn(txn, world, &self.f, &b, seller)?;
        roll.check_required_root(&RequiredRoot {
            anchor: crate::commerce::roots::RootAnchor::Filed(filed.content.id()),
            identity: filed.content.revision(),
        })
        .map_err(bad)?;
        Ok((b, filed, ownership))
    }
    #[allow(clippy::too_many_arguments)]
    fn fence(
        &self,
        inv: &ActionInvocation,
        ctx: &PrepareCtx<'_>,
        charter: CharterId,
        seller: ObjId,
        buyer: ObjId,
        payout: AccountId,
        deadline: i64,
        claims: bool,
    ) -> Result<TransferFence, PublicationError> {
        let world = ctx.world().map_err(bad)?;
        let e = world
            .module_state::<mush_economy::EconomyStore>()
            .ok_or(PublicationError::Unavailable)?;
        let title = e
            .snapshot()
            .asset(charter.object())
            .ok_or(PublicationError::Missing)?
            .clone();
        let payout = e
            .snapshot()
            .account(payout)
            .ok_or(PublicationError::Missing)?
            .clone();
        if payout.owner != mush_org::PrincipalId::Object(seller)
            || payout.scope.is_some()
            || payout.status != AccountStatus::Active
            || !title.encumbrances.is_empty()
        {
            return Err(PublicationError::Mismatch);
        }
        let claims = if claims {
            Some(
                world
                    .module_state::<crate::RollStore>()
                    .ok_or(PublicationError::Unavailable)?
                    .charter_transfer_claim_stamp(charter)
                    .map_err(bad)?,
            )
        } else {
            None
        };
        let v = TransferFence {
            action: self.action.clone(),
            commit: ctx.commit_token(),
            actor: inv.actor(),
            parameters: inv.parameters().clone(),
            charter,
            seller,
            buyer,
            title,
            payout,
            at_ic: world.world_ic_now(),
            at_utc: world.now(),
            deadline,
            claims,
            readiness: self.readiness.clone(),
        };
        v.check_world(world)?;
        Ok(v)
    }
    fn prepare(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, PublicationError> {
        self.parse(inv)?;
        let commit = ctx.commit_token();
        let (offer, ownership, fence, transfer) = if !self.accept {
            let w: OfferWire = serde_json::from_value(inv.parameters().clone()).map_err(bad)?;
            let charter = CharterId::new(decimal(&w.charter)?).map_err(bad)?;
            let buyer = ObjId(decimal(&w.buyer)?);
            let account = AccountId(decimal(&w.seller_account)?);
            let price = Amount::from_minor(decimal(&w.price_minor)?);
            let duration: i64 = decimal(&w.valid_for_seconds)?;
            if !(1..=MAX_OFFER_SECONDS).contains(&duration) || price.minor() <= 0 {
                return Err(PublicationError::Invalid);
            }
            let pair = ctx.distinct_personal_accounts(buyer).map_err(bad)?;
            if pair.requester() != inv.actor() || pair.other() != buyer {
                return Err(PublicationError::Mismatch);
            }
            let world = ctx.world().map_err(bad)?;
            let (birth, filed, ownership) = self.basis(txn, world, charter, inv.actor())?;
            if ownership.generation > 1 {
                // Current ownership was proved above against the native receipt.
                // Do not publish a new offer before that immutable acquisition;
                // the retained fence also refuses rollback after preparation.
                let predecessor = txn
                    .get(
                        &self.f.charter_transfers,
                        &CharterTransfer::key_for(charter, ownership.generation),
                    )
                    .map_err(bad)?
                    .ok_or(PublicationError::Missing)?;
                if predecessor.ownership()? != ownership
                    || world.now() < predecessor.economic.facts()?.posted_at_utc
                {
                    return Err(PublicationError::Mismatch);
                }
            }
            let deadline = world
                .now()
                .checked_add(duration)
                .ok_or(PublicationError::Capacity)?;
            let fence = self.fence(
                inv,
                ctx,
                charter,
                inv.actor(),
                buyer,
                account,
                deadline,
                false,
            )?;
            let offer = CharterOffer {
                format: 1,
                operation: commit,
                action: self.action.clone(),
                charter,
                birth: birth.commit,
                ownership: ownership.clone(),
                seller: inv.actor(),
                buyer,
                seller_account: account,
                instrument: fence.payout.instrument.clone(),
                price,
                title_revision: fence.title.revision,
                line: birth.line,
                filed: filed.content.id(),
                revision: filed.content.revision(),
                offered_at_ic: fence.at_ic.clone(),
                offered_at_utc: fence.at_utc,
                expires_at_utc: deadline,
            };
            offer.matches_birth(&birth)?;
            (offer, ownership, fence, None)
        } else {
            let w: AcceptWire = serde_json::from_value(inv.parameters().clone()).map_err(bad)?;
            let payer = AccountId(decimal(&w.payer)?);
            let offer = txn
                .get(&self.f.charter_transfer_offers, &w.offer.to_hex())
                .map_err(bad)?
                .cloned()
                .ok_or(PublicationError::Missing)?;
            offer.validate()?;
            if offer.buyer != inv.actor() {
                return Err(PublicationError::Mismatch);
            }
            let pair = ctx.distinct_personal_accounts(offer.seller).map_err(bad)?;
            if pair.requester() != inv.actor() || pair.other() != offer.seller {
                return Err(PublicationError::Mismatch);
            }
            let world = ctx.world().map_err(bad)?;
            let (birth, filed, ownership) = self.basis(txn, world, offer.charter, offer.seller)?;
            offer.matches_birth(&birth)?;
            if ownership != offer.ownership
                || filed.content.id() != offer.filed
                || filed.content.revision() != offer.revision
                || world.world_ic_now().clock != offer.offered_at_ic.clock
                || world.world_ic_now().at < offer.offered_at_ic.at
                || world.now() < offer.offered_at_utc
            {
                return Err(PublicationError::Mismatch);
            }
            let fence = self.fence(
                inv,
                ctx,
                offer.charter,
                offer.seller,
                offer.buyer,
                offer.seller_account,
                offer.expires_at_utc,
                true,
            )?;
            if fence.title.revision != offer.title_revision
                || fence.payout.instrument != offer.instrument
            {
                return Err(PublicationError::Mismatch);
            }
            let native = ctx
                .prepare_owner(
                    self.economy.as_ref().ok_or(PublicationError::Unavailable)?,
                    EconomyRequest::CharterTransfer(RegistryCharterTransfer {
                        offer: offer.operation,
                        offer_digest: offer.digest()?,
                        charter: offer.charter.object(),
                        title_revision: offer.title_revision,
                        seller: offer.seller,
                        buyer: offer.buyer,
                        payer,
                        seller_account: offer.seller_account,
                        price: offer.price,
                    }),
                )
                .map_err(bad)?
                .charter_transfer()
                .map_err(bad)?;
            if native.action() != &self.action || native.commit_token() != commit {
                return Err(PublicationError::Mismatch);
            }
            let row = CharterTransfer {
                format: 1,
                operation: commit,
                action: self.action.clone(),
                charter: offer.charter,
                generation: ownership
                    .generation
                    .checked_add(1)
                    .ok_or(PublicationError::Capacity)?,
                birth: offer.birth,
                predecessor: ownership.clone(),
                offer: offer.operation,
                offer_digest: offer.digest()?,
                accepted_at_ic: fence.at_ic.clone(),
                economic: native.subject().clone().into(),
            };
            row.matches_offer(&offer)?;
            (offer, ownership, fence, Some(row))
        };
        // Doc 17h cut M8d D4: a revoked principal neither offers nor accepts a charter.
        super::super::registry_inspection::principal::refuse_parties(
            ctx,
            txn,
            &self.f,
            &[offer.seller, offer.buyer],
        )?;
        let filed = RequiredRoot {
            anchor: crate::commerce::roots::RootAnchor::Filed(offer.filed),
            identity: offer.revision,
        };
        let mut old = vec![filed];
        let added = if let Some(row) = &transfer {
            old.push(history::offer_root(&offer)?);
            history::transfer_root(row, &offer)?
        } else {
            history::offer_root(&offer)?
        };
        ctx.prepare_owner(
            &self.roots,
            RootRequest::CharterTransfer(Box::new(TransferRoots {
                commit,
                old,
                added,
                fence,
            })),
        )
        .map_err(bad)?;
        if let Some(row) = transfer {
            let head = CharterOwnershipHead {
                format: 1,
                charter: row.charter,
                current: row.ownership()?,
            };
            txn.insert(&self.f.charter_transfers, &row.key(), row.clone())
                .map_err(bad)?;
            if ownership.generation == 1 {
                txn.insert(&self.f.charter_ownership, &head.key(), head.clone())
                    .map_err(bad)?;
            } else {
                txn.replace(&self.f.charter_ownership, &head.key(), head.clone())
                    .map_err(bad)?;
            }
            if txn
                .get(&self.f.charter_transfers, &row.key())
                .map_err(bad)?
                != Some(&row)
                || txn
                    .get(&self.f.charter_ownership, &head.key())
                    .map_err(bad)?
                    != Some(&head)
            {
                return Err(PublicationError::Mismatch);
            }
        } else {
            txn.insert(&self.f.charter_transfer_offers, &offer.key(), offer.clone())
                .map_err(bad)?;
            if txn
                .get(&self.f.charter_transfer_offers, &offer.key())
                .map_err(bad)?
                != Some(&offer)
            {
                return Err(PublicationError::Mismatch);
            }
        }
        Ok(AuthoritativeActionOutput::new(
            RulesValue::new(
                self.outcome.clone(),
                json!({"operation":commit,"offer":offer.operation,"charter":offer.charter}),
            )
            .map_err(bad)?,
        )
        .with_definitions(vec![
            self.action.clone(),
            self.outcome.clone(),
            self.retention.clone(),
        ])
        .with_eligible_witnesses(vec![inv.principal().clone()]))
    }
}
impl ActionProvider for Provider {
    fn validate_invocation(&self, inv: &ActionInvocation) -> Result<(), ActionError> {
        self.parse(inv).map_err(|_| self.refusal.clone())
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
    fn project_offer(
        &self,
        action: &DefinitionRef,
        q: &mush_core::LegalActionQuery,
        e: &mush_core::Engine,
    ) -> Result<Option<mush_core::ProviderOfferDraft>, ActionError> {
        if action != &self.action
            || self.readiness.check().is_err()
            || q.projected_subject() != &PerceivedRef::SelfActor
            || q.truth_subject() != &(mush_core::SubjectKey::Object { id: q.actor() })
            || e.world.get(q.actor()).is_none_or(|o| {
                o.kind != mush_core::ObjKind::Player || o.flags.contains(mush_core::Flags::GOING)
            })
            || e.world
                .module_state::<crate::RollStore>()
                .is_none_or(|r| r.commerce_ready().is_err())
        {
            return Ok(None);
        }
        let fail = || self.refusal.clone();
        let defs = e.registry.definitions().ok_or_else(fail)?;
        let mut guards = e
            .registry
            .server_guard_set_builder(mush_core::RevisionSet::default())
            .map_err(|_| fail())?;
        for d in [&self.action, &self.retention] {
            guards.add_definition(d.clone(), defs).map_err(|_| fail())?;
        }
        let fields: Vec<(&str, usize)> = if self.accept {
            vec![("offer", 64), ("payer", 20)]
        } else {
            vec![
                ("charter", 10),
                ("buyer", 10),
                ("seller_account", 20),
                ("price_minor", 20),
                ("valid_for_seconds", 5),
            ]
        };
        let parameters = fields
            .into_iter()
            .map(|(id, maximum_bytes)| {
                Ok(mush_core::ProjectedParameter {
                    id: mush_core::mechanics::CanonicalId::new(id).map_err(|_| fail())?,
                    required: true,
                    kind: mush_core::ProjectedParameterKind::Text { maximum_bytes },
                })
            })
            .collect::<Result<Vec<_>, ActionError>>()?;
        Ok(Some(mush_core::ProviderOfferDraft {
            fixed_parameters: json!({}),
            parameters: mush_core::ProjectedParameterSchema::new(parameters).map_err(|_| fail())?,
            targets: mush_core::ProjectedTargetSchema::None,
            required_channels: Vec::new(),
            expires: None,
            presentation: mush_core::ActionPresentation::new(
                if self.accept {
                    "roll-accept-charter-transfer"
                } else {
                    "roll-offer-charter-transfer"
                },
                if self.accept {
                    "Accept a charter transfer"
                } else {
                    "Offer a charter transfer"
                },
                "A personal charter title sale preserves existing mind terms and paid licences.",
            )
            .map_err(|_| fail())?,
            guards: guards.finish().map_err(|_| fail())?,
        }))
    }
}
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    f: CommerceFamilies,
    definitions: [DefinitionRef; 4],
) -> Result<Readiness, ModError> {
    if reg.module() != crate::NS
        || definitions[0] != protocol::offer_action()?
        || definitions[1] != protocol::accept_action()?
    {
        return Err(ModError::msg("charter transfer registration differs"));
    }
    let readiness = Readiness::default();
    for i in 0..2 {
        let (Some(action), Some(outcome)) = (definitions.get(i), definitions.get(i + 2)) else {
            return Err(ModError::msg("charter transfer registration differs"));
        };
        let action = action.clone();
        let roots = reg.request_owner_participant::<RootParticipant>(action.clone())?;
        let economy = if i == 1 {
            Some(reg.request_owner_participant::<InitialFilingParticipant>(action.clone())?)
        } else {
            None
        };
        reg.action_provider(
            action.clone(),
            Provider {
                accept: i == 1,
                action,
                outcome: outcome.clone(),
                retention: super::super::authored_retirement::retention_reference()?,
                f: f.clone(),
                roots,
                economy,
                readiness: readiness.clone(),
                refusal: ActionError::new(
                    ActionErrorCode::ProviderRejected,
                    false,
                    "The charter transfer is unavailable or stale.",
                )
                .map_err(|e| ModError::msg(e.to_string()))?,
            },
        )?;
    }
    Ok(readiness)
}
