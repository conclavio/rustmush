//! Actual isolated E4 owner images and document reconstruction. These tests do
//! not certify a commercial predecessor or activate a registered renewal owner.
//!
//! Every scenario here is one body, generic over the issuer, and runs once for each:
//! [`instantiate!`] names it as a `#[test]` in the family's own test module
//! (`managed_renewal::tests`, `managed_house_renewal::tests`), so the personal family's tests
//! are where they always were and the House family runs the same list. What a scenario needs
//! from an issuer that the owner itself does not (a seeded world, the widest offer, a write to
//! one of the offer's fields) is [`Rig`]. A body left out of `instantiate!` is dead code.
use super::history::{OfferBook, OfferRecord};
use super::install::{OfferContext, OfferPatch};
use super::*;
use crate::contract::managed::ManagedConsent;
use crate::managed_entitlement::{GrossRecipientCut, Recipient};
use crate::record::{AccountStatus, CreditPolicy, NegativePolicy};
use crate::{AccountId, Amount, EconomySnapshot, Holder, InstrumentId};
use mush_core::authoritative::ReplayWrite;
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::participant::BoundParticipant;
use mush_core::{GOD, World};
use mush_org::PrincipalId;
use serde_json::json;

pub(crate) type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
pub(crate) type Prepared<I> = (RenewalOfferOutput<I>, Box<OfferPatch<I>>, Vec<ReplayWrite>);
pub(crate) fn token(n: u64) -> CommitToken {
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

/// The fields both issuers' terms have, borrowed for a write. The owner reads an offer
/// through [`Terms`]; only a test rewrites one.
pub(crate) struct TermsMut<'a> {
    pub(crate) buyer: &'a mut Holder,
    pub(crate) charter: &'a mut ObjId,
    pub(crate) title_revision: &'a mut u64,
    pub(crate) payer: &'a mut AccountId,
    pub(crate) instrument: &'a mut InstrumentId,
    pub(crate) price: &'a mut Amount,
    pub(crate) registry: &'a mut GrossRecipientCut,
    pub(crate) venue: &'a mut Option<GrossRecipientCut>,
    pub(crate) remainder: &'a mut Recipient,
    pub(crate) not_before: &'a mut ClockStamp,
    pub(crate) expires_at: &'a mut ClockStamp,
}
/// The fields both issuers' bases have, borrowed for a write.
pub(crate) struct BasisMut<'a> {
    pub(crate) licence: &'a mut u64,
    pub(crate) predecessor_generation: &'a mut u64,
    pub(crate) next_generation: &'a mut u64,
    pub(crate) prior_quote: &'a mut CommitToken,
    pub(crate) prior_quote_digest: &'a mut DefinitionDigest,
    pub(crate) product_digest: &'a mut DefinitionDigest,
    pub(crate) grant_economics_digest: &'a mut DefinitionDigest,
}
/// Both families' records name these fields alike, so each rig borrows them by one text.
macro_rules! views {
    () => {
        fn terms(offer: &mut Self::Offer) -> $crate::renewal_twin::tests::TermsMut<'_> {
            let t = &mut offer.terms;
            $crate::renewal_twin::tests::TermsMut {
                buyer: &mut t.buyer,
                charter: &mut t.charter,
                title_revision: &mut t.title_revision,
                payer: &mut t.payer,
                instrument: &mut t.instrument,
                price: &mut t.price,
                registry: &mut t.registry,
                venue: &mut t.venue,
                remainder: &mut t.remainder,
                not_before: &mut t.not_before,
                expires_at: &mut t.expires_at,
            }
        }
        fn basis(offer: &mut Self::Offer) -> $crate::renewal_twin::tests::BasisMut<'_> {
            let b = &mut offer.basis;
            $crate::renewal_twin::tests::BasisMut {
                licence: &mut b.licence,
                predecessor_generation: &mut b.predecessor_generation,
                next_generation: &mut b.next_generation,
                prior_quote: &mut b.prior_quote,
                prior_quote_digest: &mut b.prior_quote_digest,
                product_digest: &mut b.product_digest,
                grant_economics_digest: &mut b.grant_economics_digest,
            }
        }
        fn decode(bytes: &[u8]) -> Result<Self::Offer, mush_core::participant::ParticipantError> {
            <Self::Offer>::decode(bytes)
        }
    };
}
pub(crate) use views;

/// What an issuer's rig seeds: a world at 1000 with a buyer, the issuer's side and a
/// charter, three zero-balance accounts (buyer 0, the issuer's 1, the registry 2), and an
/// offer of 100 over a window of 100 IC seconds.
pub(crate) struct Seed<I: Rig> {
    pub(crate) world: World,
    pub(crate) state: EconomyStore,
    pub(crate) offer: I::Offer,
}

/// The test side of an issuer: what a scenario needs that the owner does not.
pub(crate) trait Rig: Issuer {
    /// How many accounts a grant of the widened offer pays: registry, venue and remainder,
    /// and for the personal issuer three royalty parents.
    const ROLES: usize;
    fn seed() -> TestResult<Seed<Self>>;
    /// Every payee role the issuer has.
    fn widen(f: &mut Fixture<Self>) -> TestResult;
    fn terms(offer: &mut Self::Offer) -> TermsMut<'_>;
    fn basis(offer: &mut Self::Offer) -> BasisMut<'_>;
    fn decode(bytes: &[u8]) -> Result<Self::Offer, ParticipantError>;
    fn book_mut(state: &mut EconomyStore) -> &mut OfferBook<Self>;
    /// A character that is not the buyer and has no part in the offer's consent.
    fn stranger(f: &mut Fixture<Self>) -> TestResult<ObjId>;
    /// The issuer's side stops being current: the seller leaves, or the organisation's
    /// charter does. A sale is refused after it; a close and a refund are not.
    fn lapse(f: &mut Fixture<Self>) -> TestResult;
    /// The issuer's own fields of a widened offer at their widest encoding.
    fn widest(offer: &mut Self::Offer) -> TestResult;
    /// What the issuer's own part of the wire refuses: a royalty parent that leaves its
    /// title out and a fourth parent, or an offer without its organisation or its scope.
    fn own_wire_is_strict(offer: &Self::Offer) -> TestResult;
}

pub(crate) struct Fixture<I: Rig> {
    pub(crate) world: World,
    pub(crate) state: EconomyStore,
    pub(crate) offer: I::Offer,
    pub(crate) principal: mush_core::PrincipalId,
    pub(crate) draft: DefinitionRef,
    pub(crate) close: DefinitionRef,
}
impl<I: Rig> Fixture<I> {
    #[inline(never)]
    pub(crate) fn new() -> TestResult<Box<Self>> {
        let Seed {
            world,
            state,
            offer,
        } = I::seed()?;
        Ok(Box::new(Self {
            world,
            state,
            principal: mush_core::PrincipalId::Object {
                id: offer.terms().buyer().acting_character(),
            },
            offer,
            draft: action(I::DRAFT_ACTION, 1)?,
            close: action(I::CLOSE_ACTION, 1)?,
        }))
    }
    fn context(&self, closing: bool, commit: CommitToken) -> OfferContext<'_> {
        OfferContext {
            world: &self.world,
            actor: self.offer.terms().buyer().acting_character(),
            principal: &self.principal,
            action: if closing { &self.close } else { &self.draft },
            commit,
        }
    }
    pub(crate) fn prepare(&self) -> Result<Prepared<I>, ParticipantError> {
        install::offer::<I>(
            &self.state,
            &self.context(false, token(3)),
            self.offer.clone(),
        )
    }
    pub(crate) fn publish(&mut self) -> TestResult<(RenewalOfferOutput<I>, Vec<ReplayWrite>)> {
        let (out, patch, writes) = self.prepare()?;
        patch.recheck_in(&self.state, &self.context(false, token(3)))?;
        patch.bind(&mut self.state)?.apply();
        Ok((out, writes))
    }
    pub(crate) fn close(
        &self,
        reason: UnfundedCloseReason,
    ) -> Result<Prepared<I>, ParticipantError> {
        install::close::<I>(&self.state, &self.context(true, token(4)), token(3), reason)
    }
}
fn documents<I: Rig>(writes: &[ReplayWrite]) -> TestResult<Vec<(String, Vec<u8>)>> {
    let mut result = Vec::new();
    for write in writes {
        if let ReplayWrite::ModuleJsonSet {
            namespace,
            key,
            value,
        } = write
        {
            check_eq!(namespace.as_str(), "economy");
            if key.as_str().starts_with(I::OFFER_PREFIX) {
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

pub(crate) fn real_unaccepted_offer_and_both_closures_leave_all_money_untouched<I: Rig>()
-> TestResult {
    for reason in [UnfundedCloseReason::Declined, UnfundedCloseReason::Expired] {
        let mut f = Fixture::<I>::new()?;
        I::widen(&mut f)?;
        check_eq!(f.offer.terms().payouts()?.len(), I::ROLES);
        let before = f.state.clone();
        let (out, writes) = f.publish()?;
        same_money(&before, &f.state);
        check_eq!(f.state.next_contract, before.next_contract + 1);
        let offered = f
            .state
            .snapshot
            .contract(out.contract())
            .ok_or("contract")?;
        check_eq!(offered.state, ContractState::Offered);
        check!(
            offered.acceptances.is_empty()
                && offered.escrow.is_empty()
                && offered.pending.is_none()
        );
        check_eq!(offered.consent.terms.split.len(), I::ROLES);
        check_eq!(out.closed(), None);
        let rebuilt = OfferBook::<I>::from_documents(&f.state.snapshot, &documents::<I>(&writes)?)?;
        check_eq!(
            rebuilt.get(out.operation()),
            I::book(&f.state).get(out.operation())
        );
        if reason == UnfundedCloseReason::Expired {
            check!(f.close(reason).is_err());
            f.world.set_now(1100);
        }
        let (closed, patch, closed_writes) = f.close(reason)?;
        patch.recheck_in(&f.state, &f.context(true, token(4)))?;
        patch.bind(&mut f.state)?.apply();
        same_money(&before, &f.state);
        check_eq!(f.state.next_contract, before.next_contract + 1);
        check_eq!(closed.closed(), Some(reason));
        let record = I::book(&f.state).get(token(3)).ok_or("record")?;
        check_eq!(record.contract, out.contract());
        check_eq!(record.consent_hash, out.consent_hash());
        OfferBook::<I>::from_documents(&f.state.snapshot, &documents::<I>(&closed_writes)?)?;
        check!(f.close(reason).is_err());
        let mut another = f.offer.clone();
        *I::basis(&mut another).prior_quote = token(20);
        check!(
            install::offer::<I>(&f.state, &f.context(false, token(21)), another).is_err(),
            "closed predecessor remains claimed"
        );
    }
    Ok(())
}

pub(crate) fn unfunded_close_survives_issuer_and_account_loss_but_never_crossed_buyer_or_clock<
    I: Rig,
>() -> TestResult {
    let mut f = Fixture::<I>::new()?;
    f.publish()?;
    let mut payer = f
        .state
        .snapshot
        .account(f.offer.terms().payer())
        .ok_or("payer")?
        .clone();
    payer.status = AccountStatus::Frozen;
    f.state.snapshot.put_account(payer);
    let mut title = f
        .state
        .snapshot
        .asset(f.offer.terms().charter())
        .ok_or("title")?
        .clone();
    title.frozen = true;
    title.revision += 1;
    f.state.snapshot.put_asset(title);
    I::lapse(&mut f)?;
    let stranger = I::stranger(&mut f)?;
    let (out, patch, _) = f.close(UnfundedCloseReason::Declined)?;
    let wrong = mush_core::PrincipalId::Object { id: stranger };
    let mut context = f.context(true, token(4));
    context.actor = stranger;
    context.principal = &wrong;
    check!(patch.recheck_in(&f.state, &context).is_err());
    patch.recheck_in(&f.state, &f.context(true, token(4)))?;
    patch.bind(&mut f.state)?.apply();
    check_eq!(out.closed(), Some(UnfundedCloseReason::Declined));
    Ok(())
}

pub(crate) fn actual_before_images_and_identity_refuse_stale_bind_without_partial_install<
    I: Rig,
>() -> TestResult {
    for mode in [
        "payer",
        "title",
        "instrument",
        "allocator",
        "book",
        "contract",
    ] {
        let mut f = Fixture::<I>::new()?;
        let (_, patch, _) = f.prepare()?;
        match mode {
            "payer" => {
                let mut a = f
                    .state
                    .snapshot
                    .account(f.offer.terms().payer())
                    .ok_or("payer")?
                    .clone();
                a.revision += 1;
                f.state.snapshot.put_account(a);
            }
            "title" => {
                let mut a = f
                    .state
                    .snapshot
                    .asset(f.offer.terms().charter())
                    .ok_or("title")?
                    .clone();
                a.revision += 1;
                f.state.snapshot.put_asset(a);
            }
            "instrument" => {
                let mut i = f
                    .state
                    .snapshot
                    .instrument(f.offer.terms().instrument())
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
                let row = OfferRecord::<I> {
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
        let generation = I::book(&f.state).generation();
        check!(patch.bind(&mut f.state).is_err(), "{mode}");
        check_eq!(f.state.snapshot, before);
        check_eq!(f.state.meta_doc(), meta);
        check_eq!(I::book(&f.state).generation(), generation);
    }
    let mut f = Fixture::<I>::new()?;
    let (_, patch, _) = f.prepare()?;
    check!(
        patch
            .recheck_in(&f.state, &f.context(false, token(99)))
            .is_err()
    );
    check!(
        patch
            .recheck_in(&f.state, &f.context(true, token(3)))
            .is_err()
    );
    f.world.set_now(999);
    check!(
        patch
            .recheck_in(&f.state, &f.context(false, token(3)))
            .is_err()
    );
    f.world.set_now(1100);
    check!(
        patch
            .recheck_in(&f.state, &f.context(false, token(3)))
            .is_err()
    );
    f.world.set_now(1000);
    let before = f.state.snapshot.clone();
    drop(patch);
    check_eq!(f.state.snapshot, before);
    check!(I::book(&f.state).get(token(3)).is_none());
    let (_, patch, _) = f.prepare()?;
    let mut timeline = f.world.world_timeline().clone();
    timeline.append(timeline.next_rate_epoch(f.world.now(), 2)?)?;
    f.world.set_world_timeline(timeline);
    check!(
        patch
            .recheck_in(&f.state, &f.context(false, token(3)))
            .is_err()
    );
    Ok(())
}

pub(crate) fn strict_deadline_generation_authority_and_real_contract_caps_are_preserved<I: Rig>()
-> TestResult {
    let mut f = Fixture::<I>::new()?;
    let stranger = I::stranger(&mut f)?;
    for lifetime in [0, MAX_RENEWAL_OFFER_IC_SECONDS + 1] {
        let mut request = f.offer.clone();
        I::terms(&mut request).expires_at.at = request.terms().not_before().at + lifetime;
        check!(install::offer::<I>(&f.state, &f.context(false, token(3)), request).is_err());
    }
    let mut request = f.offer.clone();
    I::terms(&mut request).expires_at.clock = mush_core::ClockRef::WorldIc { generation: 2 };
    check!(install::offer::<I>(&f.state, &f.context(false, token(3)), request).is_err());
    let wrong = mush_core::PrincipalId::Object { id: stranger };
    let mut context = f.context(false, token(3));
    context.principal = &wrong;
    check!(install::offer::<I>(&f.state, &context, f.offer.clone()).is_err());
    let mut full = Fixture::<I>::new()?;
    for n in 0..crate::contract::MAX_OPEN_CONTRACTS_PER_ISSUER {
        let mut request = full.offer.clone();
        *I::basis(&mut request).licence = 100 + u64::try_from(n)?;
        let (_, patch, _) = install::offer::<I>(
            &full.state,
            &full.context(false, token(100 + u64::try_from(n)?)),
            request,
        )?;
        patch.bind(&mut full.state)?.apply();
    }
    check!(full.prepare().is_err());
    check_eq!(
        full.state.snapshot.contracts().count(),
        crate::contract::MAX_OPEN_CONTRACTS_PER_ISSUER
    );
    Ok(())
}

pub(crate) fn actual_proof_and_contract_reconstruction_reject_each_missing_crossed_or_funded_shape<
    I: Rig,
>() -> TestResult {
    let mut f = Fixture::<I>::new()?;
    let (out, writes) = f.publish()?;
    let docs = documents::<I>(&writes)?;
    check!(OfferBook::<I>::from_documents(&f.state.snapshot, &[]).is_err());
    check!(OfferBook::<I>::from_documents(&EconomySnapshot::new(), &docs).is_err());
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
                party: PrincipalId::Object(f.offer.terms().buyer().acting_character()),
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
        check!(
            OfferBook::<I>::from_documents(&snapshot, &docs).is_err(),
            "{mode}"
        );
    }
    let mut wrong = docs.clone();
    wrong.first_mut().ok_or("doc")?.0 = history::key::<I>(token(99));
    check!(OfferBook::<I>::from_documents(&f.state.snapshot, &wrong).is_err());
    let mut snapshot = f.state.snapshot.clone();
    snapshot.put_hold(crate::record::Hold {
        id: crate::HoldId(1),
        account: f.offer.terms().payer(),
        amount: Amount::from_minor(1),
        beneficiary: None,
        purpose: crate::record::HoldPurpose::Escrow {
            contract: out.contract(),
        },
        placed_by: PrincipalId::Object(f.offer.terms().buyer().acting_character()),
        holder: crate::record::IdempotencyKey::new("test", "unexpected"),
        expires: None,
        status: crate::record::HoldStatus::Released,
        last_journal_seq: 1,
        revision: 1,
    });
    check!(
        OfferBook::<I>::from_documents(&snapshot, &docs).is_err(),
        "even a released hold disproves never-funded F0"
    );
    Ok(())
}

pub(crate) fn new_wire_is_strict_and_old_managed_consent_bytes_remain_identical<I: Rig>()
-> TestResult {
    let f = Fixture::<I>::new()?;
    let mut missing = serde_json::to_value(&f.offer)?;
    missing
        .get_mut("terms")
        .and_then(|v| v.as_object_mut())
        .ok_or("terms")?
        .remove("venue");
    check!(serde_json::from_value::<I::Offer>(missing).is_err());
    let mut zero = f.offer.clone();
    *I::terms(&mut zero).registry = GrossRecipientCut {
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
    check!(serde_json::from_value::<I::Offer>(missing).is_err());
    I::own_wire_is_strict(&zero)?;
    check_eq!(I::decode(&serde_json::to_vec(&f.offer)?)?, f.offer);
    check!(I::decode(&vec![b' '; MAX_RENEWAL_OFFER_BYTES + 1]).is_err());
    let raw = serde_json::to_string(&f.offer)?;
    check!(
        serde_json::from_str::<I::Offer>(&raw.replacen(
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
    check!(serde_json::from_value::<I::Offer>(unknown).is_err());
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
    let mut scratch = f.offer.clone();
    let basis = I::basis(&mut scratch);
    for version in [1, 2] {
        let effect = action("purchase_original_mind", version)?;
        let old = OldWire::OriginalPersonal {
            operation: token(3),
            effect: effect.clone(),
            quote: token(2),
            quote_digest: *basis.prior_quote_digest,
            product_digest: *basis.product_digest,
        };
        let actual = ManagedConsent::original_purchase(
            token(3),
            effect,
            token(2),
            *basis.prior_quote_digest,
            *basis.product_digest,
        )?;
        check_eq!(serde_json::to_vec(&old)?, serde_json::to_vec(&actual)?);
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
            quote_digest: *basis.prior_quote_digest,
            product_digest: *basis.product_digest,
            protocol,
            economic_digest: *basis.grant_economics_digest,
        };
        let actual = ManagedConsent::entitlement_purchase(
            token(3),
            effect,
            token(2),
            *basis.prior_quote_digest,
            *basis.product_digest,
            protocol,
            *basis.grant_economics_digest,
        )?;
        check_eq!(serde_json::to_vec(&old)?, serde_json::to_vec(&actual)?);
    }
    let marker = I::consent(RenewalMarkerV1::<I>::new(
        token(3),
        f.draft.clone(),
        &f.offer,
    )?)?;
    check!(I::marker(&marker).is_some());
    check_eq!(marker.entitlement_protocol(), None);
    check_eq!(marker.economic_digest(), None);
    check_eq!(
        serde_json::from_slice::<ManagedConsent>(&serde_json::to_vec(&marker)?)?,
        marker
    );
    Ok(())
}

pub(crate) fn complete_near_limit_record_and_contract_reserve_both_closures_before_custody<
    I: Rig,
>() -> TestResult {
    let mut f = Fixture::<I>::new()?;
    I::widen(&mut f)?;
    *I::basis(&mut f.offer).licence = u64::MAX;
    *I::basis(&mut f.offer).predecessor_generation = u64::MAX - 1;
    *I::basis(&mut f.offer).next_generation = u64::MAX;
    *I::terms(&mut f.offer).price = Amount::from_minor(i64::MAX);
    // Largest numeric widths and six roles, plus a variable canonical
    // instrument that approaches the finite serialized admission boundary.
    *I::terms(&mut f.offer).buyer = ObjId(u32::MAX).into();
    *I::terms(&mut f.offer).charter = ObjId(u32::MAX - 2);
    *I::terms(&mut f.offer).title_revision = u64::MAX;
    *I::terms(&mut f.offer).payer = AccountId(u64::MAX);
    I::terms(&mut f.offer).remainder.account = AccountId(u64::MAX - 1);
    I::terms(&mut f.offer)
        .registry
        .recipient
        .as_mut()
        .ok_or("registry")?
        .account = AccountId(u64::MAX - 2);
    I::terms(&mut f.offer)
        .venue
        .as_mut()
        .and_then(|v| v.recipient.as_mut())
        .ok_or("venue")?
        .account = AccountId(u64::MAX - 3);
    I::widest(&mut f.offer)?;
    *I::terms(&mut f.offer).not_before = ClockStamp {
        clock: mush_core::ClockRef::WorldIc {
            generation: u64::MAX,
        },
        at: i64::MAX - MAX_RENEWAL_OFFER_IC_SECONDS,
    };
    *I::terms(&mut f.offer).expires_at = ClockStamp {
        clock: f.offer.terms().not_before().clock.clone(),
        at: i64::MAX,
    };
    let mut row = OfferRecord::<I> {
        format: 1,
        attempt: None,
        operation: token(3),
        action: f.draft.clone(),
        offer: f.offer.clone(),
        contract: ContractId(u64::MAX),
        consent_hash: [0; 32],
        created_at_ic: f.offer.terms().not_before().clone(),
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
    *I::terms(&mut row.offer).instrument = InstrumentId::new(instrument).ok_or("instrument")?;
    row.consent_hash = contract(&row)?.consent_hash;
    row.validate()?;
    let admitted_bytes = serde_json::to_vec(&row)?.len();
    check!(admitted_bytes + history::FUTURE_RESERVE_BYTES <= history::MAX_RECORD_BYTES);
    check!(admitted_bytes + history::FUTURE_RESERVE_BYTES + 256 >= history::MAX_RECORD_BYTES);
    let open_contract = contract(&row)?;
    check!(
        serde_json::to_vec(&open_contract)?.len() + history::FUTURE_RESERVE_BYTES
            <= history::MAX_CONTRACT_BYTES
    );
    let offered_writes = vec![
        crate::registry_filing::json_write(history::key::<I>(row.operation).into(), &row)?,
        crate::registry_filing::json_write(crate::api::contract_key(row.contract), &open_contract)?,
        crate::registry_filing::json_write(crate::api::META_KEY.into(), &f.state.meta_doc())?,
    ];
    let envelope = serde_json::to_vec(&offered_writes)?;
    check!(envelope.len() <= 32768);
    check_eq!(
        serde_json::from_slice::<Vec<ReplayWrite>>(&envelope)?,
        offered_writes
    );
    let mut too_large = row.clone();
    *I::terms(&mut too_large.offer).instrument = InstrumentId::new(format!(
        "{}{}",
        row.offer.terms().instrument(),
        "a".repeat(512)
    ))
    .ok_or("instrument")?;
    too_large.consent_hash = contract(&too_large)?.consent_hash;
    check!(too_large.validate().is_err());
    for reason in [UnfundedCloseReason::Declined, UnfundedCloseReason::Expired] {
        let mut closed = row.clone();
        closed.closure = Some(history::Closure {
            commit: token(4),
            action: f.close.clone(),
            at_ic: row.offer.terms().expires_at().clone(),
            at_utc: i64::MAX,
            reason,
        });
        closed.validate()?;
        let final_contract = contract(&closed)?;
        let closed_bytes = serde_json::to_vec(&closed)?.len();
        check!(closed_bytes <= history::MAX_RECORD_BYTES);
        check!(closed_bytes.saturating_sub(admitted_bytes) <= history::FUTURE_RESERVE_BYTES);
        check!(serde_json::to_vec(&final_contract)?.len() <= history::MAX_CONTRACT_BYTES);
        let writes = vec![
            crate::registry_filing::json_write(
                history::key::<I>(closed.operation).into(),
                &closed,
            )?,
            crate::registry_filing::json_write(
                crate::api::contract_key(closed.contract),
                &final_contract,
            )?,
        ];
        check!(serde_json::to_vec(&writes)?.len() <= 32768);
        check_eq!(final_contract.consent, open_contract.consent);
    }
    // The reserve is a bound, not proof that the future F1 funded schema exists.
    Ok(())
}

pub(crate) fn permanent_offer_table_counts_closed_predecessors_at_the_actual_limit<I: Rig>()
-> TestResult {
    let mut f = Fixture::<I>::new()?;
    let (_, writes) = f.publish()?;
    let original = I::book(&f.state).get(token(3)).ok_or("record")?.clone();
    check_eq!(documents::<I>(&writes)?.len(), 1);
    let mut book = OfferBook::<I>::default();
    for n in 0..history::MAX_RECORDS {
        let serial = 1000 + u64::try_from(n)?;
        let mut row = original.clone();
        row.operation = token(serial);
        row.contract = ContractId(serial);
        *I::basis(&mut row.offer).licence = serial;
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
    check_eq!(book.generation(), u64::try_from(history::MAX_RECORDS)?);
    let mut next = original;
    next.operation = token(900_000);
    next.contract = ContractId(900_000);
    *I::basis(&mut next.offer).licence = 900_000;
    next.consent_hash = contract(&next)?.consent_hash;
    check!(book.admits(&next).is_err());
    check!(book.get(next.operation).is_none());
    Ok(())
}

pub(crate) fn actual_legacy_plan_and_boot_continue_refusing_the_new_managed_kind<I: Rig>()
-> TestResult {
    let mut f = Fixture::<I>::new()?;
    let (out, _) = f.publish()?;
    let contract = f
        .state
        .snapshot
        .contract(out.contract())
        .ok_or("contract")?
        .clone();
    let mut stripped = contract.clone();
    stripped.consent.managed = None;
    check!(!crate::contract::managed::legacy_contract(
        &stripped,
        &f.state.snapshot
    ));
    let mut plan = crate::EconomyPlan::new(
        crate::record::IdempotencyKey::new("unrelated", "fresh"),
        PrincipalId::Object(f.offer.terms().buyer().acting_character()),
        crate::record::EconomicCause::Command { serial: 1 },
        Vec::new(),
    );
    plan.hold_ops.push(crate::plan::HoldOp::Place {
        account: f.offer.terms().payer(),
        amount: Amount::from_minor(1),
        beneficiary: None,
        purpose: crate::record::HoldPurpose::Escrow {
            contract: contract.id,
        },
        expires: None,
    });
    check!(matches!(
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
    check!(
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
    check!(
        message.contains("complete entitlement recovery"),
        "unexpected renewal boot layer: {}",
        message.chars().take(240).collect::<String>()
    );
    check!(world.take_module_ops().is_empty());
    Ok(())
}

pub(crate) mod ct0_walk;
pub(crate) mod funded;
pub(crate) mod output;

/// A census of the crate's source, for a later edit that widens the seal instead of going
/// round it: exactly two `impl Sealed for`, both in `issuer.rs`; exactly two `impl Issuer
/// for`, one in each family's file; one `mod sealed {` with no `pub`; and no `dyn` beside
/// the trait's name. It is not the proof. The proof is the compiler's refusal of a third
/// implementation (the `compile_fail` example on [`Issuer`], and doc 46's cut note).
#[test]
fn the_issuer_seal_admits_exactly_two() -> TestResult {
    fn sources(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) -> TestResult {
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            if path.is_dir() {
                sources(&path, out)?;
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
        Ok(())
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    sources(&root, &mut files)?;
    files.sort();
    let dynamic = ["dyn ", "Issuer"].concat();
    let seal = ["mod ", "sealed"].concat();
    let (mut sealed, mut issuers, mut seals) = (Vec::new(), Vec::new(), Vec::new());
    for path in &files {
        let name = path
            .strip_prefix(&root)?
            .to_string_lossy()
            .replace('\\', "/");
        for line in std::fs::read_to_string(path)?.lines() {
            let line = line.trim();
            if line.starts_with("impl Sealed for ") {
                sealed.push(name.clone());
            }
            if line.starts_with("impl Issuer for ") {
                issuers.push(name.clone());
            }
            if line.contains(&seal) && !line.starts_with("//") {
                seals.push(line.to_owned());
            }
            check!(!line.contains(&dynamic), "{name}: {line}");
        }
    }
    check_eq!(sealed, ["renewal_twin/issuer.rs", "renewal_twin/issuer.rs"]);
    check_eq!(issuers, ["managed_house_renewal.rs", "managed_renewal.rs"]);
    check_eq!(seals, [[seal.as_str(), " {"].concat()]);
    Ok(())
}

/// One scenario as a `#[test]` of the family's module: `$prefix` is the path from this
/// module to the body, in parentheses (`()`, `(funded::)`, `(funded::reoffer::)`).
macro_rules! scenario {
    ($issuer:ty, ($($prefix:tt)*), $name:ident) => {
        #[test]
        fn $name() -> $crate::renewal_twin::tests::TestResult {
            $crate::renewal_twin::tests::$($prefix)*$name::<$issuer>()
        }
    };
}
pub(crate) use scenario;
macro_rules! scenarios {
    ($issuer:ty, $prefix:tt, [$($name:ident),* $(,)?]) => {
        $( $crate::renewal_twin::tests::scenario!($issuer, $prefix, $name); )*
    };
}
pub(crate) use scenarios;

/// Every scenario of this module tree, named as a `#[test]` for one issuer. Expanded in each
/// family's `tests` module, it gives the family the module paths the personal family's tests
/// always had: `tests::`, `tests::funded::`, `tests::funded::bridge_tests::`,
/// `tests::funded::reoffer::`, `tests::funded::service::` and `tests::output::`.
macro_rules! instantiate {
    ($issuer:ty) => {
        $crate::renewal_twin::tests::scenarios!($issuer, (), [
            real_unaccepted_offer_and_both_closures_leave_all_money_untouched,
            unfunded_close_survives_issuer_and_account_loss_but_never_crossed_buyer_or_clock,
            actual_before_images_and_identity_refuse_stale_bind_without_partial_install,
            strict_deadline_generation_authority_and_real_contract_caps_are_preserved,
            actual_proof_and_contract_reconstruction_reject_each_missing_crossed_or_funded_shape,
            new_wire_is_strict_and_old_managed_consent_bytes_remain_identical,
            complete_near_limit_record_and_contract_reserve_both_closures_before_custody,
            permanent_offer_table_counts_closed_predecessors_at_the_actual_limit,
            actual_legacy_plan_and_boot_continue_refusing_the_new_managed_kind,
        ]);
        mod funded {
            $crate::renewal_twin::tests::scenarios!($issuer, (funded::), [
                renewal_funding_keeps_exact_contract_consent_and_accepts_one_real_cash_hold,
                renewal_six_payee_capture_and_refund_have_exact_real_journal_provenance,
                renewal_exhausted_money_allocators_refuse_before_any_image_install,
                renewal_refund_ignores_seller_lapse_and_old_clock_but_grant_does_not,
                renewal_history_cannot_be_minted_by_mutable_contract_or_decoded_checkpoint,
                renewal_checkpoint_join_requires_both_actual_money_rows_even_after_hot_eviction,
                renewal_replay_refuses_missing_fund_duplicate_and_changed_real_hold,
                renewal_borrowed_audit_requires_combined_replay_and_exact_current_projection,
                renewal_new_namespace_legacy_refuses_before_exact_idempotency,
                maximum_primitive_receipts_fit_their_real_strict_codecs,
                renewal_live_contract_account_and_book_fences_refuse_stale_bind,
                renewal_receipt_strict_nested_decode_and_complete_terminal_margin,
                largest_actual_offer_keeps_both_complete_terminal_images_with_unchanged_f0_caps,
                renewal_zero_cash_credit_changed_owner_and_approval_rule_never_create_escrow,
                renewal_terminal_holds_survive_actual_sweep_and_complete_cold_reconstruction,
            ]);
            mod bridge_tests {
                $crate::renewal_twin::tests::scenarios!($issuer, (funded::bridge_tests::), [
                    actual_default_boot_proves_absence_without_new_serialized_history,
                    disabled_actual_boot_refuses_retained_offer_before_any_healing_write,
                    complete_boot_helper_joins_all_five_actual_owner_states,
                    boot_helper_rejects_independent_missing_crossed_and_extra_images,
                    disabled_inventory_refuses_every_actual_evidence_source,
                    renewal_checkpoint_coverage_cannot_disappear_in_disabled_mode,
                ]);
            }
            mod reoffer {
                $crate::renewal_twin::tests::scenarios!($issuer, (funded::reoffer::), [
                    actual_unfunded_closures_allow_only_explicit_fresh_latest_buyer_retry,
                    actual_refund_then_retry_grant_retains_both_journal_histories_and_old_closed_proof,
                    retry_refuses_crossed_principal_basis_clock_and_stale_terminal_before_custody,
                    legacy_wire_is_exact_and_retry_shape_forbids_missing_null_or_automatic_attempts,
                    complete_recovery_rejects_missing_crossed_branched_and_cyclic_retry_links,
                    newer_automatic_predecessor_prevents_returning_to_old_retry_head,
                    complete_retry_closure_fits_a_full_legacy_offer_without_raising_any_cap,
                ]);
            }
            mod service {
                $crate::renewal_twin::tests::scenarios!($issuer, (funded::service::), [
                    service_both_causes_preserve_origin_bytes_and_all_money_through_cold_replay,
                    service_cleans_unavailable_buyer_but_never_borrows_buyer_or_funded_authority,
                    service_clocks_and_prepared_owner_fences_refuse_wrong_or_changed_evidence,
                    service_history_rejects_missing_orphan_duplicate_crossed_and_funded_proofs,
                    service_closed_offer_allows_only_fresh_explicit_retry_and_retains_old_proof,
                    service_complete_boundary_images_do_not_expand_existing_offer_records,
                ]);
            }
        }
        mod output {
            $crate::renewal_twin::tests::scenarios!($issuer, (output::), [
                offer_and_close_outputs_bind_exact_material_and_actual_preparation,
            ]);
        }
    };
}
pub(crate) use instantiate;
