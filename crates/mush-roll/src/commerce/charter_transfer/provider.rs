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
const MAX_ACTIVE_OFFERS_PER_CHARTER: usize = 8;
const MAX_ACTIVE_OFFERS_PER_SELLER: usize = 32;
const MAX_OFFER_PRUNE_BATCH: usize = 32;

fn scan_and_prune_offers(
    txn: &mut ActionTxn<'_>,
    f: &CommerceFamilies,
    anchors: &[(CommitToken, crate::ReferenceKey)],
    charter: CharterId,
    ownership: &CharterOwnershipRef,
    seller: ObjId,
    now: i64,
) -> Result<Vec<(String, crate::ReferenceKey)>, PublicationError> {
    let mut pruned = Vec::new();
    let mut charter_active = 0usize;
    let mut seller_active = 0usize;
    for (offer_commit, root_key) in anchors {
        let offer_key = offer_commit.to_hex();
        let Some(existing) = txn
            .get(&f.charter_transfer_offers, &offer_key)
            .map_err(bad)?
            .cloned()
        else {
            continue;
        };
        // Accepted offers remain immutable history referenced by `charter_transfers`.
        let accepted = existing
            .ownership
            .generation
            .checked_add(1)
            .map(|next_generation| {
                txn.get(
                    &f.charter_transfers,
                    &CharterTransfer::key_for(existing.charter, next_generation),
                )
            })
            .transpose()
            .map_err(bad)?
            .flatten()
            .is_some_and(|tx| tx.offer == existing.operation);
        if accepted {
            continue;
        }
        let superseded = if existing.charter == charter {
            existing.ownership != *ownership
        } else {
            txn.get(&f.charter_ownership, &existing.charter.key())
                .map_err(bad)?
                .is_some_and(|head| head.current != existing.ownership)
        };
        if now >= existing.expires_at_utc || superseded {
            if pruned.len() < MAX_OFFER_PRUNE_BATCH {
                pruned.push((offer_key, root_key.clone()));
            }
            continue;
        }
        if existing.charter == charter {
            charter_active += 1;
        }
        if existing.seller == seller {
            seller_active += 1;
        }
    }
    if charter_active >= MAX_ACTIVE_OFFERS_PER_CHARTER
        || seller_active >= MAX_ACTIVE_OFFERS_PER_SELLER
    {
        return Err(PublicationError::Capacity);
    }
    Ok(pruned)
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
        let mut pruned_offers: Vec<(String, crate::ReferenceKey)> = Vec::new();
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
            let roll = world
                .module_state::<crate::RollStore>()
                .ok_or(PublicationError::Unavailable)?;
            let anchors = roll.charter_transfer_offer_anchors();
            pruned_offers = scan_and_prune_offers(
                txn,
                &self.f,
                &anchors,
                charter,
                &ownership,
                inv.actor(),
                world.now(),
            )?;
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
                pruned: pruned_offers.iter().map(|(_, root)| root.clone()).collect(),
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
            for (stale_key, _) in &pruned_offers {
                txn.remove(&self.f.charter_transfer_offers, stale_key)
                    .map_err(bad)?;
            }
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
                "A personal charter title sale preserves existing mind terms and paid licenses.",
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

#[cfg(test)]
mod high19_tests {
    use super::*;
    use crate::commerce::families::CommerceFamilies;
    use mush_core::{DefinitionDigest, Engine, GameModule, Registry, World};
    use std::sync::{Arc, Mutex};

    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    struct FamiliesMod(Arc<Mutex<Option<CommerceFamilies>>>);
    impl GameModule for FamiliesMod {
        fn name(&self) -> &'static str {
            crate::NS
        }
        fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
            let f = CommerceFamilies::register(reg)?;
            *self.0.lock().map_err(|_| ModError::msg("lock"))? = Some(f);
            Ok(())
        }
    }

    fn make_offer(
        seed: u8,
        charter: u32,
        seller: u32,
        expires_at_utc: i64,
    ) -> TestResult<CharterOffer> {
        let d = DefinitionDigest::from_bytes([seed; 32]);
        Ok(serde_json::from_value(json!({
            "format": 1,
            "operation": CommitToken::from_bytes([seed; 16]),
            "action": protocol::offer_action()?,
            "charter": charter.to_string(),
            "birth": CommitToken::from_bytes([200; 16]),
            "ownership": {"generation": 1, "operation": CommitToken::from_bytes([200; 16]), "digest": d},
            "seller": seller,
            "buyer": seller + 100,
            "seller_account": 10,
            "instrument": "cr",
            "price": 100,
            "title_revision": 1,
            "line": "1",
            "filed": "1",
            "revision": {
                "format": 1,
                "revision": {"key": {"character": seller, "mind": 1, "number": 1}, "digest": d},
                "source": d
            },
            "offered_at_ic": {"clock": {"kind": "world_ic", "generation": 1}, "at": 100},
            "offered_at_utc": 100,
            "expires_at_utc": expires_at_utc
        }))?)
    }

    #[test]
    fn high19_charter_transfer_prunes_expired_and_caps_active_offers() -> TestResult {
        let slot = Arc::new(Mutex::new(None));
        let mut engine = Engine::with_modules(
            World::new_minimal(),
            Registry::new(),
            vec![Box::new(FamiliesMod(slot.clone()))],
        )?;
        let f = slot.lock().map_err(|_| "lock")?.clone().ok_or("families")?;
        let charter = CharterId::new(7)?;
        let ownership = CharterOwnershipRef {
            generation: 1,
            operation: CommitToken::from_bytes([200; 16]),
            digest: DefinitionDigest::from_bytes([1; 32]),
        };
        let mut anchors = Vec::new();
        // Insert 3 expired offers + MAX_ACTIVE_OFFERS_PER_CHARTER live offers.
        {
            let mut txn = engine.action_transaction(&f.charter_transfer_offers)?;
            for i in 1..=3u8 {
                let mut o = make_offer(i, 7, 10, 500)?;
                o.ownership = ownership.clone();
                let root = history::offer_root(&o)?.anchor.key();
                anchors.push((o.operation, root));
                txn.insert(&f.charter_transfer_offers, &o.key(), o)?;
            }
            for i in 0..MAX_ACTIVE_OFFERS_PER_CHARTER {
                let seed = u8::try_from(i + 10)?;
                let mut o = make_offer(seed, 7, 10, 5_000)?;
                o.ownership = ownership.clone();
                let root = history::offer_root(&o)?.anchor.key();
                anchors.push((o.operation, root));
                txn.insert(&f.charter_transfer_offers, &o.key(), o)?;
            }
            let (_, install) = txn.prepare()?.into_parts();
            engine.install_prepared_action(install)?;
        }
        // At now = 1000: the 3 expired offers do not count, but 8 live offers hit
        // MAX_ACTIVE_OFFERS_PER_CHARTER -> Capacity error.
        {
            let mut txn = engine.action_transaction(&f.charter_transfer_offers)?;
            let err =
                scan_and_prune_offers(&mut txn, &f, &anchors, charter, &ownership, ObjId(10), 1000);
            check_eq!(err, Err(PublicationError::Capacity));
        }
        // Once time advances to 5000, all 11 unaccepted offers are expired and
        // returned for atomic pruning, allowing a fresh offer!
        {
            let mut txn = engine.action_transaction(&f.charter_transfer_offers)?;
            let pruned = scan_and_prune_offers(
                &mut txn,
                &f,
                &anchors,
                charter,
                &ownership,
                ObjId(10),
                5_000,
            )?;
            check_eq!(pruned.len(), 3 + MAX_ACTIVE_OFFERS_PER_CHARTER);
        }
        Ok(())
    }
}
