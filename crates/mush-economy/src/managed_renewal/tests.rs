//! Actual isolated E4 owner images and document reconstruction. These tests do
//! not certify a commercial predecessor or activate a registered renewal owner.
use super::history::{OfferBook, OfferRecord};
use super::install::{OfferContext, OfferPatch};
use super::*;
use crate::managed_entitlement::{
    GrossRecipientCut, Recipient, RoyaltyRecipientCut, RoyaltyTitleRecipient,
};
use crate::record::{
    Account, AccountStatus, AssetLifecycle, CreditPolicy, EconomicAsset, InstrumentDef,
    InstrumentKind, InstrumentStatus, NegativePolicy,
};
use crate::{AccountId, Amount, EconomySnapshot, InstrumentId};
use mush_core::authoritative::ReplayWrite;
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::participant::BoundParticipant;
use mush_core::{GOD, ObjId, ObjKind, World};
use serde_json::json;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
type Prepared = (RenewalOfferOutput, OfferPatch, Vec<ReplayWrite>);
fn token(n: u64) -> CommitToken {
    let mut bytes = [0; 16];
    bytes[..8].copy_from_slice(&n.to_be_bytes());
    CommitToken::from_bytes(bytes)
}
fn action(name: &str, version: u32) -> TestResult<DefinitionRef> {
    Ok(DefinitionRegistryBuilder::new().register(
        OwnerId::new("market")?,
        DefinitionDescriptor::new(
            DefinitionKind::new("action")?,
            DefinitionId::new(name)?,
            version,
            json!({"format":version,"transition":name}),
        )?,
    )?)
}
struct Rig {
    world: World,
    state: EconomyStore,
    offer: RenewalOffer,
    principal: mush_core::PrincipalId,
    draft: DefinitionRef,
    close: DefinitionRef,
}
impl Rig {
    fn new() -> TestResult<Self> {
        let mut world = World::new_minimal();
        world.set_now(1000);
        let buyer = world.create(ObjKind::Player, "Renewal buyer", GOD, None)?;
        let seller = world.create(ObjKind::Player, "Renewal seller", GOD, None)?;
        let charter = world.create(ObjKind::Thing, "Renewal charter", GOD, None)?;
        let instrument = InstrumentId::new("cr").ok_or("instrument")?;
        let registry =
            PrincipalId::System(mush_org::SystemPrincipalId::new("roll.registry").ok_or("system")?);
        let mut state = EconomyStore::default();
        state.snapshot.put_instrument(InstrumentDef {
            id: instrument.clone(),
            ruleset: "test".into(),
            code: "cr".into(),
            kind: InstrumentKind::Currency,
            scale: 0,
            transferable: true,
            negative_policy: NegativePolicy::Forbid,
            status: InstrumentStatus::Active,
            display: None,
            revision: 1,
        });
        // Zero cash is deliberate: constructing an unaccepted offer must not
        // reserve, debit, or require credit to stand in for escrow.
        for (id, owner) in [
            (0, PrincipalId::Object(buyer)),
            (1, PrincipalId::Object(seller)),
            (2, registry.clone()),
        ] {
            state.snapshot.put_account(Account {
                id: AccountId(id),
                owner,
                instrument: instrument.clone(),
                scope: None,
                name: "renewal fixture".into(),
                status: AccountStatus::Active,
                posted_balance: Amount::ZERO,
                held_amount: Amount::ZERO,
                credit: CreditPolicy::forbid(),
                last_journal_seq: 0,
                last_hold_seq: 0,
                revision: 1,
                approval: None,
            });
        }
        state.snapshot.put_asset(EconomicAsset {
            object: charter,
            title_owner: PrincipalId::Object(seller),
            scope: None,
            assignment: None,
            maintenance_account: None,
            catalog: None,
            encumbrances: Vec::new(),
            acquisition: None,
            lifecycle: AssetLifecycle::Active,
            last_journal_seq: 0,
            revision: 1,
            stolen: None,
            frozen: false,
            frozen_reason: None,
        });
        let at = world.world_ic_now();
        let offer = RenewalOffer {
            format: 1,
            basis: RenewalBasisV1 {
                format: 1,
                licence: 7,
                predecessor_generation: 1,
                next_generation: 2,
                predecessor_operation: token(1),
                prior_quote: token(2),
                prior_quote_digest: DefinitionDigest::from_bytes([1; 32]),
                product_digest: DefinitionDigest::from_bytes([2; 32]),
                template_digest: DefinitionDigest::from_bytes([3; 32]),
                grant_economics_digest: DefinitionDigest::from_bytes([4; 32]),
                royalty_chain_digest: DefinitionDigest::from_bytes([5; 32]),
            },
            terms: RenewalOfferTermsV1 {
                format: 1,
                lineage: RenewalLineageV1::Original,
                buyer,
                seller,
                charter,
                title_revision: 1,
                payer: AccountId(0),
                instrument,
                price: Amount::from_minor(100),
                registry: GrossRecipientCut {
                    permille: 100,
                    recipient: Some(Recipient {
                        account: AccountId(2),
                        owner: registry,
                    }),
                },
                venue: None,
                parents: Vec::new(),
                remainder: Recipient {
                    account: AccountId(1),
                    owner: PrincipalId::Object(seller),
                },
                not_before: at.clone(),
                expires_at: mush_core::ClockStamp {
                    clock: at.clock,
                    at: at.at + 100,
                },
            },
        };
        Ok(Self {
            world,
            state,
            principal: mush_core::PrincipalId::Object { id: buyer },
            offer,
            draft: action(DRAFT_ACTION, 1)?,
            close: action(CLOSE_ACTION, 1)?,
        })
    }
    fn context(&self, closing: bool, commit: CommitToken) -> OfferContext<'_> {
        OfferContext {
            world: &self.world,
            actor: self.offer.terms.buyer,
            principal: &self.principal,
            action: if closing { &self.close } else { &self.draft },
            commit,
        }
    }
    fn prepare(&self) -> Result<Prepared, ParticipantError> {
        install::offer(
            &self.state,
            &self.context(false, token(3)),
            self.offer.clone(),
        )
    }
    fn publish(&mut self) -> TestResult<(RenewalOfferOutput, Vec<ReplayWrite>)> {
        let (out, patch, writes) = self.prepare()?;
        patch.recheck_in(&self.state, &self.context(false, token(3)))?;
        patch.bind(&mut self.state)?.apply();
        Ok((out, writes))
    }
    fn close(&self, reason: UnfundedCloseReason) -> Result<Prepared, ParticipantError> {
        install::close(&self.state, &self.context(true, token(4)), token(3), reason)
    }
    fn six_roles(&mut self) -> TestResult {
        self.offer.terms.lineage = RenewalLineageV1::Derived;
        for ordinal in 3..=6 {
            let owner = self.world.create(
                ObjKind::Player,
                &format!("Renewal recipient {ordinal}"),
                GOD,
                None,
            )?;
            let mut account = self
                .state
                .snapshot
                .account(AccountId(1))
                .ok_or("account")?
                .clone();
            account.id = AccountId(ordinal);
            account.owner = PrincipalId::Object(owner);
            self.state.snapshot.put_account(account);
            let recipient = Recipient {
                account: AccountId(ordinal),
                owner: PrincipalId::Object(owner),
            };
            if ordinal == 3 {
                self.offer.terms.venue = Some(GrossRecipientCut {
                    permille: 50,
                    recipient: Some(recipient),
                });
            } else {
                let charter =
                    self.world
                        .create(ObjKind::Thing, "Renewal parent charter", GOD, None)?;
                let mut title = self
                    .state
                    .snapshot
                    .asset(self.offer.terms.charter)
                    .ok_or("title")?
                    .clone();
                title.object = charter;
                title.title_owner = PrincipalId::Object(owner);
                self.state.snapshot.put_asset(title);
                self.offer.terms.parents.push(RoyaltyRecipientCut {
                    charter,
                    permille: 100,
                    title: Some(RoyaltyTitleRecipient {
                        title_revision: 1,
                        recipient,
                    }),
                });
            }
        }
        Ok(())
    }
}
fn documents(writes: &[ReplayWrite]) -> TestResult<Vec<(String, Vec<u8>)>> {
    let mut result = Vec::new();
    for write in writes {
        if let ReplayWrite::ModuleJsonSet {
            namespace,
            key,
            value,
        } = write
        {
            assert_eq!(namespace.as_str(), "economy");
            if key.as_str().starts_with(history::PREFIX) {
                result.push((key.as_str().to_owned(), serde_json::to_vec(value)?));
            }
        } else {
            return Err("unexpected write kind".into());
        }
    }
    Ok(result)
}
fn same_money(before: &EconomyStore, after: &EconomyStore) {
    assert_eq!(
        before.snapshot.accounts().collect::<Vec<_>>(),
        after.snapshot.accounts().collect::<Vec<_>>()
    );
    assert_eq!(
        before.snapshot.holds().collect::<Vec<_>>(),
        after.snapshot.holds().collect::<Vec<_>>()
    );
    assert_eq!(before.fold, after.fold);
    assert_eq!(before.next_transaction, after.next_transaction);
    assert_eq!(before.next_hold, after.next_hold);
    assert_eq!(before.next_account, after.next_account);
}

#[test]
fn real_unaccepted_offer_and_both_closures_leave_all_money_untouched() -> TestResult {
    for reason in [UnfundedCloseReason::Declined, UnfundedCloseReason::Expired] {
        let mut f = Rig::new()?;
        f.six_roles()?;
        assert_eq!(f.offer.terms.payouts()?.len(), 6);
        let before = f.state.clone();
        let (out, writes) = f.publish()?;
        same_money(&before, &f.state);
        assert_eq!(f.state.next_contract, before.next_contract + 1);
        let offered = f
            .state
            .snapshot
            .contract(out.contract())
            .ok_or("contract")?;
        assert_eq!(offered.state, ContractState::Offered);
        assert!(
            offered.acceptances.is_empty()
                && offered.escrow.is_empty()
                && offered.pending.is_none()
        );
        assert_eq!(offered.consent.terms.split.len(), 6);
        assert_eq!(out.closed(), None);
        let rebuilt = OfferBook::from_documents(&f.state.snapshot, &documents(&writes)?)?;
        assert_eq!(
            rebuilt.get(out.operation()),
            f.state.renewal_offers.get(out.operation())
        );
        if reason == UnfundedCloseReason::Expired {
            assert!(f.close(reason).is_err());
            f.world.set_now(1100);
        }
        let (closed, patch, closed_writes) = f.close(reason)?;
        patch.recheck_in(&f.state, &f.context(true, token(4)))?;
        patch.bind(&mut f.state)?.apply();
        same_money(&before, &f.state);
        assert_eq!(f.state.next_contract, before.next_contract + 1);
        assert_eq!(closed.closed(), Some(reason));
        let record = f.state.renewal_offers.get(token(3)).ok_or("record")?;
        assert_eq!(record.contract, out.contract());
        assert_eq!(record.consent_hash, out.consent_hash());
        OfferBook::from_documents(&f.state.snapshot, &documents(&closed_writes)?)?;
        assert!(f.close(reason).is_err());
        let mut another = f.offer.clone();
        another.basis.prior_quote = token(20);
        assert!(
            install::offer(&f.state, &f.context(false, token(21)), another).is_err(),
            "closed predecessor remains claimed"
        );
    }
    Ok(())
}

#[test]
fn unfunded_close_survives_issuer_and_account_loss_but_never_crossed_buyer_or_clock() -> TestResult
{
    let mut f = Rig::new()?;
    f.publish()?;
    let mut payer = f
        .state
        .snapshot
        .account(f.offer.terms.payer)
        .ok_or("payer")?
        .clone();
    payer.status = AccountStatus::Frozen;
    f.state.snapshot.put_account(payer);
    let mut title = f
        .state
        .snapshot
        .asset(f.offer.terms.charter)
        .ok_or("title")?
        .clone();
    title.frozen = true;
    title.revision += 1;
    f.state.snapshot.put_asset(title);
    f.world
        .get_mut(f.offer.terms.seller)
        .ok_or("seller")?
        .flags
        .insert(mush_core::Flags::GOING);
    let (out, patch, _) = f.close(UnfundedCloseReason::Declined)?;
    let wrong = mush_core::PrincipalId::Object {
        id: f.offer.terms.seller,
    };
    let mut context = f.context(true, token(4));
    context.actor = f.offer.terms.seller;
    context.principal = &wrong;
    assert!(patch.recheck_in(&f.state, &context).is_err());
    patch.recheck_in(&f.state, &f.context(true, token(4)))?;
    patch.bind(&mut f.state)?.apply();
    assert_eq!(out.closed(), Some(UnfundedCloseReason::Declined));
    Ok(())
}

#[test]
fn actual_before_images_and_identity_refuse_stale_bind_without_partial_install() -> TestResult {
    for mode in [
        "payer",
        "title",
        "instrument",
        "allocator",
        "book",
        "contract",
    ] {
        let mut f = Rig::new()?;
        let (_, patch, _) = f.prepare()?;
        match mode {
            "payer" => {
                let mut a = f
                    .state
                    .snapshot
                    .account(f.offer.terms.payer)
                    .ok_or("payer")?
                    .clone();
                a.revision += 1;
                f.state.snapshot.put_account(a);
            }
            "title" => {
                let mut a = f
                    .state
                    .snapshot
                    .asset(f.offer.terms.charter)
                    .ok_or("title")?
                    .clone();
                a.revision += 1;
                f.state.snapshot.put_asset(a);
            }
            "instrument" => {
                let mut i = f
                    .state
                    .snapshot
                    .instrument(&f.offer.terms.instrument)
                    .ok_or("instrument")?
                    .clone();
                i.revision += 1;
                f.state.snapshot.put_instrument(i);
            }
            "allocator" => f.state.next_contract += 1,
            "book" => {
                let (_, other, _) = f.prepare()?;
                other.bind(&mut f.state)?.apply();
            }
            "contract" => {
                let (out, _, _) = f.prepare()?;
                let row = OfferRecord {
                    format: 1,
                    attempt: None,
                    operation: token(3),
                    action: f.draft.clone(),
                    offer: f.offer.clone(),
                    contract: out.contract(),
                    consent_hash: out.consent_hash(),
                    created_at_ic: f.world.world_ic_now(),
                    created_at_utc: f.world.now(),
                    closure: None,
                };
                f.state.snapshot.put_contract(contract(&row)?);
            }
            _ => return Err("case".into()),
        }
        let before = f.state.snapshot.clone();
        let meta = f.state.meta_doc();
        let generation = f.state.renewal_offers.generation();
        assert!(patch.bind(&mut f.state).is_err(), "{mode}");
        assert_eq!(f.state.snapshot, before);
        assert_eq!(f.state.meta_doc(), meta);
        assert_eq!(f.state.renewal_offers.generation(), generation);
    }
    let mut f = Rig::new()?;
    let (_, patch, _) = f.prepare()?;
    assert!(
        patch
            .recheck_in(&f.state, &f.context(false, token(99)))
            .is_err()
    );
    assert!(
        patch
            .recheck_in(&f.state, &f.context(true, token(3)))
            .is_err()
    );
    f.world.set_now(999);
    assert!(
        patch
            .recheck_in(&f.state, &f.context(false, token(3)))
            .is_err()
    );
    f.world.set_now(1100);
    assert!(
        patch
            .recheck_in(&f.state, &f.context(false, token(3)))
            .is_err()
    );
    f.world.set_now(1000);
    let before = f.state.snapshot.clone();
    drop(patch);
    assert_eq!(f.state.snapshot, before);
    assert!(f.state.renewal_offers.get(token(3)).is_none());
    let (_, patch, _) = f.prepare()?;
    let mut timeline = f.world.world_timeline().clone();
    timeline.append(timeline.next_rate_epoch(f.world.now(), 2)?)?;
    f.world.set_world_timeline(timeline);
    assert!(
        patch
            .recheck_in(&f.state, &f.context(false, token(3)))
            .is_err()
    );
    Ok(())
}

#[test]
fn strict_deadline_generation_authority_and_real_contract_caps_are_preserved() -> TestResult {
    let f = Rig::new()?;
    for lifetime in [0, MAX_RENEWAL_OFFER_IC_SECONDS + 1] {
        let mut request = f.offer.clone();
        request.terms.expires_at.at = request.terms.not_before.at + lifetime;
        assert!(install::offer(&f.state, &f.context(false, token(3)), request).is_err());
    }
    let mut request = f.offer.clone();
    request.terms.expires_at.clock = mush_core::ClockRef::WorldIc { generation: 2 };
    assert!(install::offer(&f.state, &f.context(false, token(3)), request).is_err());
    let wrong = mush_core::PrincipalId::Object {
        id: f.offer.terms.seller,
    };
    let mut context = f.context(false, token(3));
    context.principal = &wrong;
    assert!(install::offer(&f.state, &context, f.offer.clone()).is_err());
    let mut full = Rig::new()?;
    for n in 0..crate::contract::MAX_OPEN_CONTRACTS_PER_ISSUER {
        let mut request = full.offer.clone();
        request.basis.licence = 100 + u64::try_from(n)?;
        let (_, patch, _) = install::offer(
            &full.state,
            &full.context(false, token(100 + u64::try_from(n)?)),
            request,
        )?;
        patch.bind(&mut full.state)?.apply();
    }
    assert!(full.prepare().is_err());
    assert_eq!(
        full.state.snapshot.contracts().count(),
        crate::contract::MAX_OPEN_CONTRACTS_PER_ISSUER
    );
    Ok(())
}

#[test]
fn actual_proof_and_contract_reconstruction_reject_each_missing_crossed_or_funded_shape()
-> TestResult {
    let mut f = Rig::new()?;
    let (out, writes) = f.publish()?;
    let docs = documents(&writes)?;
    assert!(OfferBook::from_documents(&f.state.snapshot, &[]).is_err());
    assert!(OfferBook::from_documents(&EconomySnapshot::new(), &docs).is_err());
    let offered = f
        .state
        .snapshot
        .contract(out.contract())
        .ok_or("contract")?
        .clone();
    for mode in ["id", "consent", "state", "acceptance", "escrow", "pending"] {
        let mut snapshot = f.state.snapshot.clone();
        let mut changed = offered.clone();
        match mode {
            "id" => changed.id = ContractId(99),
            "consent" => changed.consent_hash[0] ^= 1,
            "state" => changed.state = ContractState::Draft,
            "acceptance" => changed.acceptances.push(crate::contract::Acceptance {
                party: PrincipalId::Object(f.offer.terms.buyer),
                at: f.world.now(),
                consent_hash: changed.consent_hash,
            }),
            "escrow" => changed.escrow.push(crate::HoldId(1)),
            "pending" => {
                changed.pending = Some(crate::contract::PendingTerminal {
                    kind: crate::contract::PendingKind::Cancel,
                    attempts: 1,
                    since: f.world.now(),
                })
            }
            _ => return Err("case".into()),
        }
        snapshot.put_contract(changed);
        assert!(
            OfferBook::from_documents(&snapshot, &docs).is_err(),
            "{mode}"
        );
    }
    let mut wrong = docs.clone();
    wrong.first_mut().ok_or("doc")?.0 = history::key(token(99));
    assert!(OfferBook::from_documents(&f.state.snapshot, &wrong).is_err());
    let mut snapshot = f.state.snapshot.clone();
    snapshot.put_hold(crate::record::Hold {
        id: crate::HoldId(1),
        account: f.offer.terms.payer,
        amount: Amount::from_minor(1),
        beneficiary: None,
        purpose: crate::record::HoldPurpose::Escrow {
            contract: out.contract(),
        },
        placed_by: PrincipalId::Object(f.offer.terms.buyer),
        holder: crate::record::IdempotencyKey::new("test", "unexpected"),
        expires: None,
        status: crate::record::HoldStatus::Released,
        last_journal_seq: 1,
        revision: 1,
    });
    assert!(
        OfferBook::from_documents(&snapshot, &docs).is_err(),
        "even a released hold disproves never-funded F0"
    );
    Ok(())
}

#[test]
fn new_wire_is_strict_and_old_managed_consent_bytes_remain_identical() -> TestResult {
    let f = Rig::new()?;
    let mut missing = serde_json::to_value(&f.offer)?;
    missing
        .get_mut("terms")
        .and_then(|v| v.as_object_mut())
        .ok_or("terms")?
        .remove("venue");
    assert!(serde_json::from_value::<RenewalOffer>(missing).is_err());
    let mut zero = f.offer.clone();
    zero.terms.registry = GrossRecipientCut {
        permille: 0,
        recipient: None,
    };
    let mut missing = serde_json::to_value(&zero)?;
    missing
        .get_mut("terms")
        .and_then(|v| v.get_mut("registry"))
        .and_then(|v| v.as_object_mut())
        .ok_or("registry")?
        .remove("recipient");
    assert!(serde_json::from_value::<RenewalOffer>(missing).is_err());
    let mut marked = zero.clone();
    marked.terms.lineage = RenewalLineageV1::Derived;
    marked.terms.parents.push(RoyaltyRecipientCut {
        charter: ObjId(900),
        permille: 0,
        title: None,
    });
    let mut missing = serde_json::to_value(&marked)?;
    missing
        .get_mut("terms")
        .and_then(|v| v.get_mut("parents"))
        .and_then(|v| v.as_array_mut())
        .and_then(|v| v.first_mut())
        .and_then(|v| v.as_object_mut())
        .ok_or("parent")?
        .remove("title");
    assert!(serde_json::from_value::<RenewalOffer>(missing).is_err());
    assert_eq!(
        RenewalOffer::decode(&serde_json::to_vec(&f.offer)?)?,
        f.offer
    );
    assert!(RenewalOffer::decode(&vec![b' '; MAX_RENEWAL_OFFER_BYTES + 1]).is_err());
    let mut too_many = serde_json::to_value(&marked)?;
    let rows = too_many
        .get_mut("terms")
        .and_then(|v| v.get_mut("parents"))
        .and_then(|v| v.as_array_mut())
        .ok_or("parents")?;
    let first = rows.first().cloned().ok_or("parent")?;
    rows.resize(4, first);
    assert!(RenewalOffer::decode(&serde_json::to_vec(&too_many)?).is_err());
    let raw = serde_json::to_string(&f.offer)?;
    assert!(
        serde_json::from_str::<RenewalOffer>(&raw.replacen(
            "\"format\":1",
            "\"format\":1,\"format\":1",
            1
        ))
        .is_err()
    );
    let mut unknown = serde_json::to_value(&f.offer)?;
    unknown
        .as_object_mut()
        .ok_or("offer")?
        .insert("actor".into(), json!(9));
    assert!(serde_json::from_value::<RenewalOffer>(unknown).is_err());
    #[derive(Serialize)]
    #[serde(tag = "kind", rename_all = "snake_case")]
    enum OldWire {
        OriginalPersonal {
            operation: CommitToken,
            effect: DefinitionRef,
            quote: CommitToken,
            quote_digest: DefinitionDigest,
            product_digest: DefinitionDigest,
        },
        MindEntitlement {
            operation: CommitToken,
            effect: DefinitionRef,
            quote: CommitToken,
            quote_digest: DefinitionDigest,
            product_digest: DefinitionDigest,
            protocol: crate::managed_entitlement::EntitlementProtocol,
            economic_digest: DefinitionDigest,
        },
    }
    for version in [1, 2] {
        let effect = action("purchase_original_mind", version)?;
        let old = OldWire::OriginalPersonal {
            operation: token(3),
            effect: effect.clone(),
            quote: token(2),
            quote_digest: f.offer.basis.prior_quote_digest,
            product_digest: f.offer.basis.product_digest,
        };
        let actual = ManagedConsent::original_purchase(
            token(3),
            effect,
            token(2),
            f.offer.basis.prior_quote_digest,
            f.offer.basis.product_digest,
        )?;
        assert_eq!(serde_json::to_vec(&old)?, serde_json::to_vec(&actual)?);
    }
    for protocol in [
        crate::managed_entitlement::EntitlementProtocol::OriginalV3,
        crate::managed_entitlement::EntitlementProtocol::DerivedV1,
    ] {
        let effect = action(protocol.action("fund").ok_or("action")?, protocol.version())?;
        let old = OldWire::MindEntitlement {
            operation: token(3),
            effect: effect.clone(),
            quote: token(2),
            quote_digest: f.offer.basis.prior_quote_digest,
            product_digest: f.offer.basis.product_digest,
            protocol,
            economic_digest: f.offer.basis.grant_economics_digest,
        };
        let actual = ManagedConsent::entitlement_purchase(
            token(3),
            effect,
            token(2),
            f.offer.basis.prior_quote_digest,
            f.offer.basis.product_digest,
            protocol,
            f.offer.basis.grant_economics_digest,
        )?;
        assert_eq!(serde_json::to_vec(&old)?, serde_json::to_vec(&actual)?);
    }
    let marker =
        ManagedConsent::renewal_offer(RenewalMarkerV1::new(token(3), f.draft.clone(), &f.offer)?)?;
    assert!(marker.renewal_offer_marker().is_some());
    assert_eq!(marker.entitlement_protocol(), None);
    assert_eq!(marker.economic_digest(), None);
    assert_eq!(
        serde_json::from_slice::<ManagedConsent>(&serde_json::to_vec(&marker)?)?,
        marker
    );
    Ok(())
}

#[test]
fn complete_near_limit_record_and_contract_reserve_both_closures_before_custody() -> TestResult {
    let mut f = Rig::new()?;
    f.six_roles()?;
    f.offer.basis.licence = u64::MAX;
    f.offer.basis.predecessor_generation = u64::MAX - 1;
    f.offer.basis.next_generation = u64::MAX;
    f.offer.terms.price = Amount::from_minor(i64::MAX);
    // Largest numeric widths and six roles, plus a variable canonical
    // instrument that approaches the finite serialized admission boundary.
    f.offer.terms.buyer = ObjId(u32::MAX);
    f.offer.terms.seller = ObjId(u32::MAX - 1);
    f.offer.terms.charter = ObjId(u32::MAX - 2);
    f.offer.terms.title_revision = u64::MAX;
    f.offer.terms.payer = AccountId(u64::MAX);
    f.offer.terms.remainder.account = AccountId(u64::MAX - 1);
    f.offer.terms.remainder.owner = PrincipalId::Object(f.offer.terms.seller);
    f.offer
        .terms
        .registry
        .recipient
        .as_mut()
        .ok_or("registry")?
        .account = AccountId(u64::MAX - 2);
    f.offer
        .terms
        .venue
        .as_mut()
        .and_then(|v| v.recipient.as_mut())
        .ok_or("venue")?
        .account = AccountId(u64::MAX - 3);
    for (n, cut) in f.offer.terms.parents.iter_mut().enumerate() {
        cut.charter = ObjId(u32::MAX - 3 - u32::try_from(n)?);
        let title = cut.title.as_mut().ok_or("title")?;
        title.title_revision = u64::MAX;
        title.recipient.account = AccountId(u64::MAX - 4 - u64::try_from(n)?);
        title.recipient.owner = PrincipalId::Object(ObjId(u32::MAX - 10 - u32::try_from(n)?));
    }
    f.offer.terms.not_before = ClockStamp {
        clock: mush_core::ClockRef::WorldIc {
            generation: u64::MAX,
        },
        at: i64::MAX - MAX_RENEWAL_OFFER_IC_SECONDS,
    };
    f.offer.terms.expires_at = ClockStamp {
        clock: f.offer.terms.not_before.clock.clone(),
        at: i64::MAX,
    };
    let mut row = OfferRecord {
        format: 1,
        attempt: None,
        operation: token(3),
        action: f.draft.clone(),
        offer: f.offer.clone(),
        contract: ContractId(u64::MAX),
        consent_hash: [0; 32],
        created_at_ic: f.offer.terms.not_before.clone(),
        created_at_utc: i64::MAX,
        closure: None,
    };
    row.consent_hash = contract(&row)?.consent_hash;
    let base = serde_json::to_vec(&row)?.len();
    let remaining = (history::MAX_RECORD_BYTES - history::FUTURE_RESERVE_BYTES)
        .checked_sub(base)
        .ok_or("shape exceeds reserved cap")?;
    let instrument = format!(
        "cr{}",
        "a".repeat(remaining.checked_sub(128).ok_or("shape headroom")?)
    );
    row.offer.terms.instrument = InstrumentId::new(instrument).ok_or("instrument")?;
    row.consent_hash = contract(&row)?.consent_hash;
    row.validate()?;
    let admitted_bytes = serde_json::to_vec(&row)?.len();
    assert!(admitted_bytes + history::FUTURE_RESERVE_BYTES <= history::MAX_RECORD_BYTES);
    assert!(admitted_bytes + history::FUTURE_RESERVE_BYTES + 256 >= history::MAX_RECORD_BYTES);
    let open_contract = contract(&row)?;
    assert!(
        serde_json::to_vec(&open_contract)?.len() + history::FUTURE_RESERVE_BYTES
            <= history::MAX_CONTRACT_BYTES
    );
    let offered_writes = vec![
        crate::registry_filing::json_write(history::key(row.operation).into(), &row)?,
        crate::registry_filing::json_write(crate::api::contract_key(row.contract), &open_contract)?,
        crate::registry_filing::json_write(crate::api::META_KEY.into(), &f.state.meta_doc())?,
    ];
    let envelope = serde_json::to_vec(&offered_writes)?;
    assert!(envelope.len() <= 32768);
    assert_eq!(
        serde_json::from_slice::<Vec<ReplayWrite>>(&envelope)?,
        offered_writes
    );
    let mut too_large = row.clone();
    too_large.offer.terms.instrument =
        InstrumentId::new(format!("{}{}", row.offer.terms.instrument, "a".repeat(512)))
            .ok_or("instrument")?;
    too_large.consent_hash = contract(&too_large)?.consent_hash;
    assert!(too_large.validate().is_err());
    for reason in [UnfundedCloseReason::Declined, UnfundedCloseReason::Expired] {
        let mut closed = row.clone();
        closed.closure = Some(history::Closure {
            commit: token(4),
            action: f.close.clone(),
            at_ic: row.offer.terms.expires_at.clone(),
            at_utc: i64::MAX,
            reason,
        });
        closed.validate()?;
        let final_contract = contract(&closed)?;
        let closed_bytes = serde_json::to_vec(&closed)?.len();
        assert!(closed_bytes <= history::MAX_RECORD_BYTES);
        assert!(closed_bytes.saturating_sub(admitted_bytes) <= history::FUTURE_RESERVE_BYTES);
        assert!(serde_json::to_vec(&final_contract)?.len() <= history::MAX_CONTRACT_BYTES);
        let writes = vec![
            crate::registry_filing::json_write(history::key(closed.operation).into(), &closed)?,
            crate::registry_filing::json_write(
                crate::api::contract_key(closed.contract),
                &final_contract,
            )?,
        ];
        assert!(serde_json::to_vec(&writes)?.len() <= 32768);
        assert_eq!(final_contract.consent, open_contract.consent);
    }
    // The reserve is a bound, not proof that the future F1 funded schema exists.
    Ok(())
}

#[test]
fn permanent_offer_table_counts_closed_predecessors_at_the_actual_limit() -> TestResult {
    let mut f = Rig::new()?;
    let (_, writes) = f.publish()?;
    let original = f
        .state
        .renewal_offers
        .get(token(3))
        .ok_or("record")?
        .clone();
    assert_eq!(documents(&writes)?.len(), 1);
    let mut book = OfferBook::default();
    for n in 0..history::MAX_RECORDS {
        let serial = 1000 + u64::try_from(n)?;
        let mut row = original.clone();
        row.operation = token(serial);
        row.contract = ContractId(serial);
        row.offer.basis.licence = serial;
        row.closure = Some(history::Closure {
            commit: token(serial + 100_000),
            action: f.close.clone(),
            at_ic: row.created_at_ic.clone(),
            at_utc: row.created_at_utc,
            reason: UnfundedCloseReason::Declined,
        });
        row.consent_hash = contract(&row)?.consent_hash;
        book.admits(&row)?;
        let generation = book.next_generation()?;
        book.install(row, generation);
    }
    assert_eq!(book.generation(), u64::try_from(history::MAX_RECORDS)?);
    let mut next = original;
    next.operation = token(900_000);
    next.contract = ContractId(900_000);
    next.offer.basis.licence = 900_000;
    next.consent_hash = contract(&next)?.consent_hash;
    assert!(book.admits(&next).is_err());
    assert!(book.get(next.operation).is_none());
    Ok(())
}

#[test]
fn actual_legacy_plan_and_boot_continue_refusing_the_new_managed_kind() -> TestResult {
    let mut f = Rig::new()?;
    let (out, _) = f.publish()?;
    let contract = f
        .state
        .snapshot
        .contract(out.contract())
        .ok_or("contract")?
        .clone();
    let mut stripped = contract.clone();
    stripped.consent.managed = None;
    assert!(!crate::contract::managed::legacy_contract(
        &stripped,
        &f.state.snapshot
    ));
    let mut plan = crate::EconomyPlan::new(
        crate::record::IdempotencyKey::new("unrelated", "fresh"),
        PrincipalId::Object(f.offer.terms.buyer),
        crate::record::EconomicCause::Command { serial: 1 },
        Vec::new(),
    );
    plan.hold_ops.push(crate::plan::HoldOp::Place {
        account: f.offer.terms.payer,
        amount: Amount::from_minor(1),
        beneficiary: None,
        purpose: crate::record::HoldPurpose::Escrow {
            contract: contract.id,
        },
        expires: None,
    });
    assert!(matches!(
        crate::contract::managed::require_legacy_plan(&plan, &f.state.snapshot),
        Err(crate::EconomyError::ManagedContract)
    ));
    let capabilities = crate::CAPABILITIES
        .iter()
        .copied()
        .map(mush_org::CapabilityId::new)
        .collect::<Option<std::collections::BTreeSet<_>>>()
        .ok_or("capabilities")?;
    let mut world = mush_testkit::TestWorld::with_modules(vec![
        Box::new(mush_org::OrgModule::new().with_capabilities(capabilities)),
        Box::new(crate::EconomyModule::with_custody(Vec::new())),
    ]);
    // Discard only the fixture's initial empty-boot writes. The assertion below
    // measures this refused recovery attempt, as the existing managed boot test does.
    let _ = world.take_module_ops();
    let document = serde_json::to_value(&contract)?;
    let decoded =
        serde_json::from_value::<EconomicContract>(document.clone()).map_err(|error| {
            format!(
                "renewal boot fixture contract Value decode: {}",
                error.to_string().chars().take(240).collect::<String>()
            )
        })?;
    assert!(
        decoded == contract,
        "renewal boot fixture contract Value roundtrip"
    );
    let boot = std::collections::HashMap::from([(
        smol_str::SmolStr::new("economy"),
        mush_core::ModuleBootData {
            kv: vec![(crate::api::contract_key(contract.id), document)],
            ..Default::default()
        },
    )]);
    let error = world
        .try_modules_loaded(boot)
        .err()
        .ok_or("new managed contract unexpectedly booted")?;
    let message = error.to_string();
    assert!(
        message.contains("complete entitlement recovery"),
        "unexpected renewal boot layer: {}",
        message.chars().take(240).collect::<String>()
    );
    assert!(world.take_module_ops().is_empty());
    Ok(())
}

#[path = "tests/funded.rs"]
mod funded;

#[path = "tests/output.rs"]
mod output;
