use super::*;
use crate::record::{
    Account, CreditPolicy, EconomicAsset, InstrumentDef, InstrumentKind, NegativePolicy,
};
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::{GOD, World};
use serde_json::json;

type TestResult = Result<(), Box<dyn std::error::Error>>;

// Receipt-accounting tests consume actual engine-generated transactions. They
// do not use their expanded table as a substitute for full boot evidence.
pub(super) fn receipt_capacity_fixture() -> Result<
    (
        PurchaseReceiptAudit,
        EconomicTransaction,
        [EconomicTransaction; 2],
    ),
    Box<dyn std::error::Error>,
> {
    let mut fixture = Fixture::new()?;
    let funding = fixture.fund()?;
    let next = prepare_funding_in(
        &fixture.state,
        &FundingContext {
            world: &fixture.world,
            actor: fixture.request.buyer,
            principal: &mush_core::PrincipalId::Object {
                id: fixture.request.buyer,
            },
            action: &fixture.action,
            commit: CommitToken::from_bytes([5; 16]),
        },
        fixture.request.clone(),
    )?;
    let grant = fixture.terminal(&funding, PurchaseOutcome::Granted)?;
    let refund = fixture.terminal(&funding, PurchaseOutcome::Refunded)?;
    Ok((
        fixture.state.fold.purchase_receipts.clone(),
        next.transaction,
        [grant.transaction, refund.transaction],
    ))
}

struct Fixture {
    world: World,
    state: EconomyStore,
    action: DefinitionRef,
    request: OriginalPurchase,
}
impl Fixture {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let mut world = World::new_minimal();
        world.set_now(1000);
        let buyer = world.create(ObjKind::Player, "Buyer", GOD, None)?;
        let seller = world.create(ObjKind::Player, "Seller", GOD, None)?;
        let charter = world.create(ObjKind::Thing, "Charter", GOD, None)?;
        let instrument = InstrumentId::new("cr").ok_or("instrument")?;
        let registry = mush_org::SystemPrincipalId::new("roll.registry").ok_or("registry")?;
        let mut state = EconomyStore::default();
        // Unit-only empty-history fixture. Boot tests use actual replay hooks.
        state.fold.purchase_receipts.finish_replay()?;
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
        for (id, owner, posted) in [
            (0, PrincipalId::Object(buyer), 1000),
            (1, PrincipalId::Object(seller), 0),
            (2, PrincipalId::System(registry.clone()), 0),
        ] {
            state.snapshot.put_account(Account {
                id: AccountId(id),
                owner,
                instrument: instrument.clone(),
                scope: None,
                name: "purchase fixture".into(),
                status: AccountStatus::Active,
                posted_balance: crate::Amount::from_minor(posted),
                held_amount: crate::Amount::ZERO,
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
        let action = DefinitionRegistryBuilder::new().register(
            OwnerId::new("market")?,
            DefinitionDescriptor::new(
                DefinitionKind::new("action")?,
                DefinitionId::new("purchase_original_mind")?,
                1,
                json!({"format":1,"protocol":"original_mind_purchase_v1","transition":"fund"}),
            )?,
        )?;
        let request = OriginalPurchase {
            quote: CommitToken::from_bytes([2; 16]),
            quote_digest: DefinitionDigest::for_canonical_value("test.quote", &json!({"quote":2}))?,
            product_digest: DefinitionDigest::for_canonical_value(
                "test.product",
                &json!({"product":1}),
            )?,
            buyer,
            seller,
            charter,
            title_revision: 1,
            payer: AccountId(0),
            instrument,
            price: crate::Amount::from_minor(100),
            payouts: vec![
                PurchasePayout {
                    account: AccountId(1),
                    owner: PrincipalId::Object(seller),
                    amount: crate::Amount::from_minor(90),
                },
                PurchasePayout {
                    account: AccountId(2),
                    owner: PrincipalId::System(registry),
                    amount: crate::Amount::from_minor(10),
                },
            ],
            quoted_at_utc: 999,
            expires_at_utc: 1100,
        };
        Ok(Self {
            world,
            state,
            action,
            request,
        })
    }
    fn prepare(&self) -> Result<FundingPreparation, ParticipantError> {
        prepare_funding_in(
            &self.state,
            &FundingContext {
                world: &self.world,
                actor: self.request.buyer,
                principal: &mush_core::PrincipalId::Object {
                    id: self.request.buyer,
                },
                action: &self.action,
                commit: CommitToken::from_bytes([3; 16]),
            },
            self.request.clone(),
        )
    }
    fn fund(&mut self) -> Result<FundingOutput, Box<dyn std::error::Error>> {
        use mush_core::participant::BoundParticipant;
        let (output, patch, _) = install::finish_images(
            &self.state,
            self.world.world_ic_now().clock,
            self.prepare()?,
        )?;
        patch.bind(&mut self.state)?.apply();
        Ok(output)
    }
    fn terminal(
        &self,
        funding: &FundingOutput,
        outcome: PurchaseOutcome,
    ) -> Result<Box<terminal::TerminalPreparation>, Box<dyn std::error::Error>> {
        self.terminal_as(funding, outcome, self.request.buyer)
    }
    fn terminal_as(
        &self,
        funding: &FundingOutput,
        outcome: PurchaseOutcome,
        actor: ObjId,
    ) -> Result<Box<terminal::TerminalPreparation>, Box<dyn std::error::Error>> {
        let (name, transition) = match outcome {
            PurchaseOutcome::Granted => ("settle_original_mind", "grant"),
            PurchaseOutcome::Refunded => ("refund_original_mind", "refund"),
        };
        let action = DefinitionRegistryBuilder::new().register(
            OwnerId::new("market")?,
            DefinitionDescriptor::new(
                DefinitionKind::new("action")?,
                DefinitionId::new(name)?,
                self.action.version(),
                json!({"format":self.action.version(),
                    "protocol":format!("original_mind_purchase_v{}", self.action.version()),
                    "transition":transition}),
            )?,
        )?;
        Ok(terminal::prepare_in(
            &self.state,
            &FundingContext {
                world: &self.world,
                actor,
                principal: &mush_core::PrincipalId::Object { id: actor },
                action: &action,
                commit: CommitToken::from_bytes([4; 16]),
            },
            PurchaseTerminal {
                operation: funding.operation(),
                purchase: self.request.clone(),
                funding: funding.receipt().clone(),
            },
            outcome,
        )?)
    }
}

#[test]
fn terminal_grant_uses_actual_escrow_and_legacy_validation_cannot_post_its_plan() -> TestResult {
    let mut fixture = Fixture::new()?;
    let funding = fixture.fund()?;
    let before = fixture.state.snapshot.clone();
    let prepared = fixture.terminal(&funding, PurchaseOutcome::Granted)?;
    assert_eq!(fixture.state.snapshot, before);
    assert_eq!(prepared.output.operation(), funding.operation());
    assert_eq!(prepared.output.contract(), funding.receipt().contract());
    assert!(prepared.output.issued_at_ic().is_some());
    assert_eq!(
        prepared
            .transaction
            .hold_effects
            .first()
            .ok_or("effect")?
            .kind,
        crate::record::HoldEffectKind::Captured
    );
    assert_eq!(prepared.transaction.lines.len(), 3);
    let gate = fixture.state.key_gate();
    let auth = WorldAuth { w: &fixture.world };
    let env = ValidateEnv {
        auth: &auth,
        custody: &NoCustody,
        facts: &NoFacts,
        now: fixture.world.now(),
        ic_now: fixture.world.game_now_unix(),
        next_hold_id: fixture.state.next_hold,
        spent: BTreeMap::new(),
        keys: &gate,
    };
    assert!(matches!(
        crate::engine::validate(&prepared.plan, &fixture.state.snapshot, &env),
        PlanResult::Refused(crate::EconomyError::ManagedContract)
    ));
    let mut after = before;
    prepared.delta.apply_to(&mut after);
    assert_eq!(
        after
            .account(AccountId(0))
            .ok_or("buyer")?
            .posted_balance
            .minor(),
        900
    );
    assert!(
        after
            .account(AccountId(0))
            .ok_or("buyer")?
            .held_amount
            .is_zero()
    );
    assert_eq!(
        after
            .account(AccountId(1))
            .ok_or("seller")?
            .posted_balance
            .minor(),
        90
    );
    assert_eq!(
        after
            .account(AccountId(2))
            .ok_or("registry")?
            .posted_balance
            .minor(),
        10
    );
    Ok(())
}

#[test]
fn expired_or_lost_seller_title_refuses_grant_but_exact_refund_returns_only_buyers_hold()
-> TestResult {
    let mut fixture = Fixture::new()?;
    let funding = fixture.fund()?;
    fixture.world.set_now(fixture.request.expires_at_utc + 1);
    let mut title = fixture
        .state
        .snapshot
        .asset(fixture.request.charter)
        .cloned()
        .ok_or("title")?;
    title.frozen = true;
    fixture.state.snapshot.put_asset(title);
    assert!(
        fixture
            .terminal(&funding, PurchaseOutcome::Granted)
            .is_err()
    );
    let before = fixture.state.snapshot.clone();
    let prepared = fixture.terminal(&funding, PurchaseOutcome::Refunded)?;
    assert_eq!(fixture.state.snapshot, before);
    assert!(prepared.output.issued_at_ic().is_none());
    assert!(prepared.transaction.lines.is_empty());
    assert_eq!(
        prepared
            .transaction
            .hold_effects
            .first()
            .ok_or("effect")?
            .kind,
        crate::record::HoldEffectKind::Released
    );
    let mut after = before;
    prepared.delta.apply_to(&mut after);
    assert_eq!(
        after
            .account(AccountId(0))
            .ok_or("buyer")?
            .posted_balance
            .minor(),
        1000
    );
    assert!(
        after
            .account(AccountId(0))
            .ok_or("buyer")?
            .held_amount
            .is_zero()
    );
    assert!(
        after
            .account(AccountId(1))
            .ok_or("seller")?
            .posted_balance
            .is_zero()
    );
    assert!(
        after
            .account(AccountId(2))
            .ok_or("registry")?
            .posted_balance
            .is_zero()
    );
    Ok(())
}

#[test]
fn terminal_refuses_crossed_receipts_and_missing_actual_funding_history() -> TestResult {
    for mutation in 0..4 {
        let mut fixture = Fixture::new()?;
        let mut funding = fixture.fund()?;
        match mutation {
            0 => funding.receipt.transaction = TransactionId(99),
            1 => funding.receipt.journal_seq += 1,
            2 => funding.operation = CommitToken::from_bytes([99; 16]),
            _ => {
                fixture.state.fold.purchase_receipts = PurchaseReceiptAudit::default();
            }
        }
        let before = fixture.state.snapshot.clone();
        assert!(
            fixture
                .terminal(&funding, PurchaseOutcome::Granted)
                .is_err()
        );
        assert!(
            fixture
                .terminal(&funding, PurchaseOutcome::Refunded)
                .is_err()
        );
        assert_eq!(fixture.state.snapshot, before);
    }
    Ok(())
}

#[test]
fn terminal_images_install_exact_fold_and_cannot_apply_twice_or_after_hold_drift() -> TestResult {
    use mush_core::participant::BoundParticipant;
    for outcome in [PurchaseOutcome::Granted, PurchaseOutcome::Refunded] {
        let mut fixture = Fixture::new()?;
        let funding = fixture.fund()?;
        let prepared = fixture.terminal(&funding, outcome)?;
        let transaction = prepared.transaction.clone();
        let mut expected_fold = fixture.state.fold.clone();
        expected_fold.fold_entry(transaction.journal_seq, &transaction);
        let (_, patch, writes) = install::finish_terminal_images(
            &fixture.state,
            fixture.world.world_ic_now().clock,
            prepared,
        )?;
        assert_eq!(
            writes.len(),
            if outcome == PurchaseOutcome::Granted {
                8
            } else {
                6
            }
        );
        drop(patch.bind(&mut fixture.state)?);
        assert!(
            fixture
                .state
                .snapshot
                .hold(funding.receipt().hold())
                .ok_or("hold")?
                .status
                .is_held()
        );
        let (_, patch, _) = install::finish_terminal_images(
            &fixture.state,
            fixture.world.world_ic_now().clock,
            fixture.terminal(&funding, outcome)?,
        )?;
        patch.bind(&mut fixture.state)?.apply();
        assert_eq!(fixture.state.fold, expected_fold);
        assert_eq!(
            fixture.state.transactions.get(&transaction.id),
            Some(&transaction)
        );
        assert!(
            !fixture
                .state
                .snapshot
                .hold(funding.receipt().hold())
                .ok_or("hold")?
                .status
                .is_held()
        );
        assert!(fixture.terminal(&funding, outcome).is_err());

        let mut changed = Fixture::new()?;
        let funding = changed.fund()?;
        let (_, patch, _) = install::finish_terminal_images(
            &changed.state,
            changed.world.world_ic_now().clock,
            changed.terminal(&funding, outcome)?,
        )?;
        let mut hold = changed
            .state
            .snapshot
            .hold(funding.receipt().hold())
            .cloned()
            .ok_or("hold")?;
        hold.revision += 1;
        changed.state.snapshot.put_hold(hold);
        let before = changed.state.snapshot.clone();
        assert!(patch.bind(&mut changed.state).is_err());
        assert_eq!(changed.state.snapshot, before);
        assert_eq!(changed.state.journal.next_seq(JOURNAL_LOG), 1);
    }
    Ok(())
}

#[test]
fn funding_images_bind_inertly_then_install_the_exact_prebuilt_journal_and_fold() -> TestResult {
    use mush_core::authoritative::ReplayWrite;
    use mush_core::participant::BoundParticipant;
    let mut fixture = Fixture::new()?;
    let before = fixture.state.snapshot.clone();
    let before_meta = fixture.state.meta_doc();
    let prepared = fixture.prepare()?;
    let transaction = prepared.transaction.clone();
    let mut expected_fold = fixture.state.fold.clone();
    expected_fold.fold_entry(transaction.journal_seq, &transaction);
    let (output, patch, writes) =
        install::finish_images(&fixture.state, fixture.world.world_ic_now().clock, prepared)?;
    assert_eq!(fixture.state.snapshot, before);
    assert_eq!(fixture.state.meta_doc(), before_meta);
    // The actual durable union has one journal and all five exact projection
    // images. No later ordinary post or contract write is needed.
    assert_eq!(writes.len(), 6);
    assert_eq!(
        writes
            .iter()
            .filter(|write| matches!(write, ReplayWrite::LedgerChunkSet { .. }))
            .count(),
        1
    );
    let contract_key = crate::api::contract_key(output.receipt().contract());
    let expected_contract = writes
        .iter()
        .find_map(|write| match write {
            ReplayWrite::ModuleJsonSet { key, value, .. } if key.as_str() == contract_key => {
                Some(value.clone())
            }
            _ => None,
        })
        .ok_or("managed contract image")?;
    patch.bind(&mut fixture.state)?.apply();
    assert_eq!(fixture.state.fold, expected_fold);
    assert_eq!(
        fixture.state.transactions.get(&transaction.id),
        Some(&transaction)
    );
    let actual_contract = fixture
        .state
        .snapshot
        .contract(output.receipt().contract())
        .ok_or("contract")?;
    assert_eq!(serde_json::to_value(actual_contract)?, expected_contract);
    assert_eq!(
        actual_contract.consent.managed.as_ref(),
        Some(output.marker())
    );
    let payer = fixture
        .state
        .snapshot
        .account(fixture.request.payer)
        .ok_or("payer")?;
    assert_eq!(
        payer.posted_balance,
        before
            .account(fixture.request.payer)
            .ok_or("old payer")?
            .posted_balance
    );
    assert_eq!(payer.held_amount, fixture.request.price);
    assert_eq!(fixture.state.next_transaction, transaction.id.0 + 1);
    assert_eq!(fixture.state.next_contract, actual_contract.id.0 + 1);
    assert_eq!(fixture.state.next_hold, output.receipt().hold().0 + 1);
    assert!(
        fixture
            .state
            .snapshot
            .account(AccountId(1))
            .is_some_and(|payee| payee.posted_balance.is_zero())
    );
    Ok(())
}

#[test]
fn dropping_funding_bind_leaves_every_economic_projection_unchanged() -> TestResult {
    let mut fixture = Fixture::new()?;
    let before = fixture.state.snapshot.clone();
    let meta = fixture.state.meta_doc();
    let fold = fixture.state.fold.clone();
    let (_, patch, _) = install::finish_images(
        &fixture.state,
        fixture.world.world_ic_now().clock,
        fixture.prepare()?,
    )?;
    drop(patch.bind(&mut fixture.state)?);
    assert_eq!(fixture.state.snapshot, before);
    assert_eq!(fixture.state.meta_doc(), meta);
    assert_eq!(fixture.state.fold, fold);
    assert!(fixture.state.transactions.is_empty());
    assert_eq!(fixture.state.journal.next_seq(JOURNAL_LOG), 0);
    assert_eq!(fixture.state.posts_since_boot, 0);
    Ok(())
}

#[test]
fn funding_bind_rejects_doc_only_authority_and_allocator_fold_drift_before_mutation() -> TestResult
{
    for mutation in 0..5 {
        let mut fixture = Fixture::new()?;
        let (_, patch, _) = install::finish_images(
            &fixture.state,
            fixture.world.world_ic_now().clock,
            fixture.prepare()?,
        )?;
        match mutation {
            0 => {
                let mut title = fixture
                    .state
                    .snapshot
                    .asset(fixture.request.charter)
                    .cloned()
                    .ok_or("title")?;
                title.frozen = true;
                fixture.state.snapshot.put_asset(title);
            }
            1 => {
                let mut account = fixture
                    .state
                    .snapshot
                    .account(AccountId(1))
                    .cloned()
                    .ok_or("payee")?;
                account.status = AccountStatus::Closed;
                fixture.state.snapshot.put_account(account);
            }
            2 => fixture.state.next_hold += 1,
            3 => {
                fixture.state.fold.touches.insert(fixture.request.payer, 42);
            }
            _ => {
                fixture
                    .state
                    .fold
                    .running_held
                    .insert(fixture.request.payer, 42);
            }
        }
        let before = fixture.state.snapshot.clone();
        let meta = fixture.state.meta_doc();
        let fold = fixture.state.fold.clone();
        assert!(
            patch.bind(&mut fixture.state).is_err(),
            "mutation {mutation}"
        );
        assert_eq!(fixture.state.snapshot, before);
        assert_eq!(fixture.state.meta_doc(), meta);
        assert_eq!(fixture.state.fold, fold);
        assert_eq!(fixture.state.journal.next_seq(JOURNAL_LOG), 0);
        assert!(fixture.state.transactions.is_empty());
    }
    Ok(())
}

#[test]
fn funding_uses_actual_e4_hold_and_retains_an_inert_complete_consent() -> TestResult {
    let fixture = Fixture::new()?;
    let before = fixture.state.snapshot.clone();
    let counters = (
        fixture.state.next_contract,
        fixture.state.next_hold,
        fixture.state.next_transaction,
    );
    let prepared = fixture.prepare()?;
    // Reserving the closed namespace also protects future-contract funding:
    // no actual managed contract exists yet for the legacy reference fence.
    let gate = fixture.state.key_gate();
    let auth = WorldAuth { w: &fixture.world };
    let env = ValidateEnv {
        auth: &auth,
        custody: &NoCustody,
        facts: &NoFacts,
        now: fixture.world.now(),
        ic_now: fixture.world.game_now_unix(),
        next_hold_id: fixture.state.next_hold,
        spent: BTreeMap::new(),
        keys: &gate,
    };
    assert!(matches!(
        crate::engine::validate(&prepared.plan, &fixture.state.snapshot, &env),
        PlanResult::Refused(crate::EconomyError::ManagedContract)
    ));
    assert_eq!(fixture.state.snapshot, before);
    assert_eq!(
        (
            fixture.state.next_contract,
            fixture.state.next_hold,
            fixture.state.next_transaction
        ),
        counters
    );
    assert_eq!(prepared.contract.escrow, vec![HoldId(counters.1)]);
    assert!(prepared.contract.consent_intact());
    assert!(prepared.contract.fully_accepted());
    assert_eq!(
        prepared.contract.acceptances.len(),
        1,
        "no forged seller acceptance"
    );
    assert_eq!(
        prepared
            .contract
            .acceptances
            .first()
            .ok_or("acceptance")?
            .party,
        PrincipalId::Object(fixture.request.buyer)
    );
    assert!(
        prepared.contract.consent.terms.deliverables.is_empty(),
        "licence is a paired typed effect, not a fabricated E4 object"
    );
    let mut landed = before.clone();
    prepared.delta.clone().apply_to(&mut landed);
    landed.put_contract(prepared.contract.clone());
    assert!(crate::contract::qualifying_escrow(&prepared.contract, &landed).complete);
    let payer = landed.account(AccountId(0)).ok_or("payer")?;
    assert_eq!(payer.posted_balance, crate::Amount::from_minor(1000));
    assert_eq!(payer.held_amount, crate::Amount::from_minor(100));
    assert_eq!(
        landed.account(AccountId(1)).ok_or("seller")?.posted_balance,
        crate::Amount::ZERO
    );
    assert_eq!(
        prepared.output.receipt().transaction(),
        prepared.transaction.id
    );
    assert_eq!(
        prepared.output.receipt().journal_seq(),
        prepared.transaction.journal_seq
    );
    assert_eq!(
        prepared.output.receipt().consent_hash(),
        &prepared.contract.consent_hash
    );
    assert_eq!(prepared.output.marker().quote(), fixture.request.quote);
    assert_eq!(
        prepared.output.marker().product_digest(),
        fixture.request.product_digest
    );
    let mut cancelled = prepared.contract.clone();
    cancelled.state = ContractState::Cancelled;
    assert!(
        crate::contract::managed::require_legacy_plan(
            &crate::contract::refund_plan(&cancelled, &landed)?,
            &landed,
        )
        .is_err(),
        "prepared managed money cannot fall back to a money-only refund"
    );
    Ok(())
}

#[test]
fn actual_current_title_wallet_and_quote_boundaries_refuse_before_any_mutation() -> TestResult {
    let mut fixture = Fixture::new()?;
    let before = fixture.state.snapshot.clone();
    fixture.world.set_now(fixture.request.expires_at_utc);
    assert!(fixture.prepare().is_err());
    fixture.world.set_now(fixture.request.quoted_at_utc - 1);
    assert!(fixture.prepare().is_err());
    fixture.world.set_now(1000);
    fixture.request.title_revision += 1;
    assert!(fixture.prepare().is_err());
    fixture.request.title_revision -= 1;
    let mut title = fixture
        .state
        .snapshot
        .asset(fixture.request.charter)
        .ok_or("title")?
        .clone();
    title.frozen = true;
    fixture.state.snapshot.put_asset(title.clone());
    assert!(
        fixture.prepare().is_err(),
        "doc-only frozen status is authority even without a journal revision"
    );
    title.frozen = false;
    fixture.state.snapshot.put_asset(title);
    let mut payer = fixture
        .state
        .snapshot
        .account(fixture.request.payer)
        .ok_or("payer")?
        .clone();
    payer.owner = PrincipalId::Object(fixture.request.seller);
    fixture.state.snapshot.put_account(payer);
    assert!(fixture.prepare().is_err());
    assert_eq!(fixture.state.next_contract, 0);
    assert_eq!(fixture.state.next_hold, 0);
    fixture.state.snapshot = before;
    assert!(fixture.prepare().is_ok());
    Ok(())
}

#[test]
fn quote_payouts_are_exact_and_actual_payee_identity_is_rechecked() -> TestResult {
    let mut fixture = Fixture::new()?;
    let original = fixture.request.clone();
    fixture.request.payouts.first_mut().ok_or("payout")?.amount = crate::Amount::from_minor(91);
    assert!(fixture.prepare().is_err());
    fixture.request = original.clone();
    fixture.request.payouts.reverse();
    assert!(
        fixture.prepare().is_err(),
        "canonical ordering is not silently normalized"
    );
    fixture.request = original;
    let mut recipient = fixture
        .state
        .snapshot
        .account(AccountId(2))
        .ok_or("recipient")?
        .clone();
    recipient.owner = PrincipalId::Object(fixture.request.buyer);
    fixture.state.snapshot.put_account(recipient);
    assert!(
        fixture.prepare().is_err(),
        "quote owner cannot substitute for current E4 identity"
    );
    Ok(())
}

#[test]
fn funding_receipt_clock_is_strict_and_journal_sequence_zero_is_valid() -> TestResult {
    let fixture = Fixture::new()?;
    let prepared = fixture.prepare()?;
    let original = serde_json::to_value(prepared.output.receipt())?;
    let mut value = original.clone();
    value["journal_seq"] = json!(0);
    let decoded: FundingReceipt = serde_json::from_value(value.clone())?;
    assert_eq!(decoded.journal_seq(), 0);
    value["at_ic"]["clock"] = json!({"kind":"utc"});
    assert!(serde_json::from_value::<FundingReceipt>(value.clone()).is_err());
    value["at_ic"]["clock"] = json!({"kind":"world_ic","generation":0});
    assert!(serde_json::from_value::<FundingReceipt>(value).is_err());
    let mut extra = original;
    extra["validated"] = json!(true);
    assert!(serde_json::from_value::<FundingReceipt>(extra).is_err());
    Ok(())
}

#[test]
fn terminal_proof_survives_hot_transaction_eviction() -> TestResult {
    let mut fixture = Fixture::new()?;
    let funding = fixture.fund()?;
    fixture.state.transactions.clear();
    let before = fixture.state.snapshot.clone();
    assert!(fixture.terminal(&funding, PurchaseOutcome::Granted).is_ok());
    assert!(
        fixture
            .terminal(&funding, PurchaseOutcome::Refunded)
            .is_ok()
    );
    assert_eq!(fixture.state.snapshot, before);
    Ok(())
}

fn purchase_idem_rows(state: &EconomyStore) -> Vec<crate::checkpoint::IdemRow> {
    state
        .transactions
        .values()
        .filter(|tx| tx.idempotency.namespace.as_str() == NAMESPACE)
        .map(|tx| crate::checkpoint::IdemRow {
            ns: tx.idempotency.namespace.clone(),
            key: tx.idempotency.key.clone(),
            plan_hash: tx
                .plan_hash
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
            tx: tx.id,
            posted_at: tx.posted_at,
            seq: tx.journal_seq,
        })
        .collect()
}
#[test]
fn checkpoint_proof_binds_every_actual_key_and_reconstructs_economic_plan() -> TestResult {
    use mush_core::participant::BoundParticipant;
    for outcome in [PurchaseOutcome::Granted, PurchaseOutcome::Refunded] {
        let mut f = Fixture::new()?;
        let funding = f.fund()?;
        let rows = purchase_idem_rows(&f.state);
        assert!(f.state.fold.purchase_receipts.checkpoint_complete(&rows));
        let (_, patch, _) = install::finish_terminal_images(
            &f.state,
            f.world.world_ic_now().clock,
            f.terminal(&funding, outcome)?,
        )?;
        patch.bind(&mut f.state)?.apply();
        let rows = purchase_idem_rows(&f.state);
        assert_eq!(rows.len(), 2);
        let audit = &f.state.fold.purchase_receipts;
        assert!(audit.checkpoint_complete(&rows));
        let encoded = serde_json::to_value(audit)?;
        for alteration in 0..4 {
            let mut altered = encoded.clone();
            let histories = altered
                .get_mut("histories")
                .and_then(serde_json::Value::as_object_mut)
                .ok_or("histories")?;
            if alteration == 0 {
                histories.clear();
            } else {
                let row = histories.values_mut().next().ok_or("history")?;
                match alteration {
                    1 => {
                        row.get_mut("purchase")
                            .and_then(serde_json::Value::as_object_mut)
                            .ok_or("purchase")?
                            .insert("title_revision".into(), json!(99));
                    }
                    2 => {
                        row.get_mut("terminal")
                            .and_then(serde_json::Value::as_object_mut)
                            .ok_or("terminal")?
                            .insert(
                                "commit".into(),
                                serde_json::to_value(CommitToken::from_bytes([88; 16]))?,
                            );
                    }
                    _ => {
                        row.get_mut("funding")
                            .and_then(serde_json::Value::as_object_mut)
                            .ok_or("funding")?
                            .insert("transaction".into(), json!(999));
                    }
                }
            }
            let altered: PurchaseReceiptAudit = serde_json::from_value(altered)?;
            assert!(
                !altered.checkpoint_complete(&rows),
                "alteration {alteration}"
            );
            assert_eq!(
                altered.require_complete(),
                Err(PurchaseAuditError::Incomplete)
            );
        }
        let mut omitted = rows.clone();
        omitted.pop();
        assert!(!audit.checkpoint_complete(&omitted));
        let mut extra = rows.clone();
        let mut row = rows.first().ok_or("row")?.clone();
        row.key = "not-an-operation/fund".into();
        extra.push(row);
        assert!(!audit.checkpoint_complete(&extra));
        let mut crossed = rows.clone();
        crossed.first_mut().ok_or("row")?.posted_at += 1;
        assert!(!audit.checkpoint_complete(&crossed));
    }
    Ok(())
}
#[test]
fn closed_history_replay_rejects_malformed_cause_and_duplicate_terminal() -> TestResult {
    use mush_core::participant::BoundParticipant;
    let mut f = Fixture::new()?;
    let funding = f.fund()?;
    let (_, patch, _) = install::finish_terminal_images(
        &f.state,
        f.world.world_ic_now().clock,
        f.terminal(&funding, PurchaseOutcome::Granted)?,
    )?;
    patch.bind(&mut f.state)?.apply();
    let transactions: Vec<_> = f.state.transactions.values().cloned().collect();
    let mut replay = PurchaseReceiptAudit::default();
    for tx in &transactions {
        replay.observe(tx);
    }
    replay.finish_replay()?;
    assert_eq!(replay, f.state.fold.purchase_receipts);
    let last = transactions.last().ok_or("terminal")?;
    replay.observe(last);
    assert_eq!(replay.finish_replay(), Err(PurchaseAuditError::Refused));
    let mut malformed = transactions.first().ok_or("funding")?.clone();
    malformed.cause = EconomicCause::Ruleset {
        namespace: NAMESPACE.into(),
        correlation: "{not-json".into(),
    };
    let mut invalid = PurchaseReceiptAudit::default();
    invalid.observe(&malformed);
    assert_eq!(invalid.finish_replay(), Err(PurchaseAuditError::Refused));
    Ok(())
}

#[test]
fn economy_owner_allows_god_to_refund_but_never_grant_for_another_buyer() -> TestResult {
    let mut f = Fixture::new()?;
    let funding = f.fund()?;
    let before = f.state.snapshot.clone();
    assert!(
        f.terminal_as(&funding, PurchaseOutcome::Granted, GOD)
            .is_err()
    );
    let refund = f.terminal_as(&funding, PurchaseOutcome::Refunded, GOD)?;
    assert_eq!(refund.output.outcome(), PurchaseOutcome::Refunded);
    assert!(refund.transaction.lines.is_empty());
    assert_eq!(
        refund.transaction.actor,
        PrincipalId::Object(f.request.buyer)
    );
    assert_eq!(f.state.snapshot, before);
    Ok(())
}

#[test]
fn versioned_original_consent_funding_and_terminal_history_preserve_closed_protocol() -> TestResult
{
    use mush_core::participant::BoundParticipant;
    for version in [1, 2] {
        for outcome in [PurchaseOutcome::Granted, PurchaseOutcome::Refunded] {
            let mut fixture = Fixture::new()?;
            fixture.action = DefinitionRegistryBuilder::new().register(
                OwnerId::new("market")?,
                DefinitionDescriptor::new(
                    DefinitionKind::new("action")?,
                    DefinitionId::new("purchase_original_mind")?,
                    version,
                    json!({"format":version,
                        "protocol":format!("original_mind_purchase_v{version}"),
                        "transition":"fund"}),
                )?,
            )?;
            // Real E4 preparation and owner-image install, not a constructed
            // consent used as permission. Full paired Registry boot is separate.
            let funding = fixture.fund()?;
            let marker = funding.marker();
            assert_eq!(marker.effect(), &fixture.action);
            let encoded = serde_json::to_vec(marker)?;
            let decoded: ManagedConsent = serde_json::from_slice(&encoded)?;
            assert_eq!(&decoded, marker);
            let mut unsupported = serde_json::to_value(marker)?;
            *unsupported
                .pointer_mut("/effect/version")
                .ok_or("effect version")? = json!(3);
            assert!(serde_json::from_value::<ManagedConsent>(unsupported).is_err());
            let actual = fixture
                .state
                .snapshot
                .contract(funding.receipt().contract())
                .ok_or("contract")?;
            assert_eq!(actual.consent.managed.as_ref(), Some(marker));
            let cold_document: EconomicContract =
                serde_json::from_slice(&serde_json::to_vec(actual)?)?;
            assert_eq!(&cold_document, actual);
            assert!(!crate::contract::managed::legacy_contract(
                actual,
                &fixture.state.snapshot
            ));
            let prepared = fixture.terminal(&funding, outcome)?;
            let mut expected_fold = fixture.state.fold.clone();
            expected_fold.fold_entry(prepared.transaction.journal_seq, &prepared.transaction);
            let (_, patch, _) = install::finish_terminal_images(
                &fixture.state,
                fixture.world.world_ic_now().clock,
                prepared,
            )?;
            patch.bind(&mut fixture.state)?.apply();
            assert_eq!(fixture.state.fold, expected_fold);
            let payer = fixture
                .state
                .snapshot
                .account(fixture.request.payer)
                .ok_or("payer")?;
            assert_eq!(payer.held_amount, crate::Amount::ZERO);
            assert_eq!(
                payer.posted_balance,
                crate::Amount::from_minor(if outcome == PurchaseOutcome::Granted {
                    900
                } else {
                    1000
                })
            );
            let history = fixture
                .state
                .fold
                .purchase_receipts
                .get(funding.operation())?;
            assert_eq!(history.effect(), &fixture.action);
            assert_eq!(
                history.terminal().ok_or("terminal history")?.granted(),
                outcome == PurchaseOutcome::Granted
            );
            assert!(fixture.terminal(&funding, outcome).is_err());
        }
    }
    Ok(())
}

pub(super) mod entitlement;
