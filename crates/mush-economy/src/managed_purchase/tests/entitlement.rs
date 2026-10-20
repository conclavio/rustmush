//! Actual pure E4 owner images; no Market/Roll grant or certification fixture.
use super::*;
use crate::managed_entitlement::*;
use mush_core::participant::BoundParticipant;

struct EntitlementFixture {
    old: Fixture,
    purchase: EntitlementPurchase,
}
fn action(
    protocol: EntitlementProtocol,
    transition: &str,
) -> Result<DefinitionRef, Box<dyn std::error::Error>> {
    Ok(DefinitionRegistryBuilder::new().register(
        OwnerId::new("market")?,
        DefinitionDescriptor::new(
            DefinitionKind::new("action")?,
            DefinitionId::new(protocol.action(transition).ok_or("transition")?)?,
            protocol.version(),
            json!({"format":protocol.version(),"protocol":protocol.name(),"transition":transition}),
        )?,
    )?)
}
impl EntitlementFixture {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let mut old = Fixture::new()?;
        let original = old.request.clone();
        let registry = original.payouts.last().ok_or("registry")?;
        let mut parents = Vec::new();
        for ordinal in 3..=6 {
            let owner =
                old.world
                    .create(ObjKind::Player, &format!("Recipient {ordinal}"), GOD, None)?;
            let mut account = old
                .state
                .snapshot
                .account(AccountId(1))
                .ok_or("account")?
                .clone();
            account.id = AccountId(ordinal);
            account.owner = PrincipalId::Object(owner);
            old.state.snapshot.put_account(account);
            if ordinal > 3 {
                let charter = old
                    .world
                    .create(ObjKind::Thing, "Parent charter", GOD, None)?;
                let mut title = old
                    .state
                    .snapshot
                    .asset(original.charter)
                    .ok_or("title")?
                    .clone();
                title.object = charter;
                title.title_owner = PrincipalId::Object(owner);
                old.state.snapshot.put_asset(title);
                parents.push(RoyaltyRecipientCut {
                    charter,
                    permille: 100,
                    title: Some(RoyaltyTitleRecipient {
                        title_revision: 1,
                        recipient: Recipient {
                            account: AccountId(ordinal),
                            owner: PrincipalId::Object(owner),
                        },
                    }),
                });
            }
        }
        let purchase = EntitlementPurchase {
            protocol: EntitlementProtocol::DerivedV1,
            quote: original.quote,
            quote_digest: original.quote_digest,
            product_digest: original.product_digest,
            buyer: original.buyer,
            seller: original.seller,
            charter: original.charter,
            title_revision: original.title_revision,
            payer: original.payer,
            instrument: original.instrument,
            price: original.price,
            registry: GrossRecipientCut {
                permille: 100,
                recipient: Some(Recipient {
                    account: registry.account,
                    owner: registry.owner.clone(),
                }),
            },
            venue: Some(GrossRecipientCut {
                permille: 50,
                recipient: Some(Recipient {
                    account: AccountId(3),
                    owner: old
                        .state
                        .snapshot
                        .account(AccountId(3))
                        .ok_or("venue")?
                        .owner
                        .clone(),
                }),
            }),
            parents,
            remainder: Recipient {
                account: AccountId(1),
                owner: PrincipalId::Object(original.seller),
            },
            royalty_chain_digest: DefinitionDigest::from_bytes([0x31; 32]),
            grant_economics_digest: DefinitionDigest::from_bytes([0x32; 32]),
            quoted_at_utc: original.quoted_at_utc,
            expires_at_utc: original.expires_at_utc,
        };
        Ok(Self { old, purchase })
    }
    fn prepare(&self) -> Result<FundingPreparation, Box<dyn std::error::Error>> {
        let action = action(self.purchase.protocol, "fund")?;
        Ok(prepare_funding_in(
            &self.old.state,
            &FundingContext {
                world: &self.old.world,
                actor: self.purchase.buyer,
                principal: &mush_core::PrincipalId::Object {
                    id: self.purchase.buyer,
                },
                action: &action,
                commit: CommitToken::from_bytes([3; 16]),
            },
            PurchaseSubject::try_from(self.purchase.clone())?,
        )?)
    }
    fn fund(&mut self) -> Result<FundingOutput, Box<dyn std::error::Error>> {
        let before = self.old.state.snapshot.clone();
        let prepared = self.prepare()?;
        let (output, patch, _) = install::finish_images(
            &self.old.state,
            self.old.world.world_ic_now().clock,
            prepared,
        )?;
        assert_eq!(self.old.state.snapshot, before);
        patch.bind(&mut self.old.state)?.apply();
        Ok(output)
    }
    fn terminal(
        &self,
        funding: &FundingOutput,
        outcome: PurchaseOutcome,
    ) -> Result<Box<terminal::TerminalPreparation>, Box<dyn std::error::Error>> {
        let action = action(
            self.purchase.protocol,
            if outcome == PurchaseOutcome::Granted {
                "grant"
            } else {
                "refund"
            },
        )?;
        Ok(terminal::prepare_in(
            &self.old.state,
            &FundingContext {
                world: &self.old.world,
                actor: self.purchase.buyer,
                principal: &mush_core::PrincipalId::Object {
                    id: self.purchase.buyer,
                },
                action: &action,
                commit: CommitToken::from_bytes([4; 16]),
            },
            SubjectTerminal::try_from(EntitlementTerminal {
                operation: funding.operation(),
                purchase: self.purchase.clone(),
                funding: funding.receipt().clone(),
            })?,
            outcome,
        )?)
    }
}

#[test]
fn actual_six_payee_capture_and_refund_install_the_exact_independent_fold() -> TestResult {
    for outcome in [PurchaseOutcome::Granted, PurchaseOutcome::Refunded] {
        let mut f = EntitlementFixture::new()?;
        assert_eq!(f.purchase.payouts()?.len(), 6);
        let funding = f.fund()?;
        assert_eq!(
            funding.marker().entitlement_protocol(),
            Some(EntitlementProtocol::DerivedV1)
        );
        assert_eq!(
            funding.marker().economic_digest(),
            Some(f.purchase.economic_digest()?)
        );
        assert_eq!(
            f.old
                .state
                .snapshot
                .account(f.purchase.payer)
                .ok_or("payer")?
                .held_amount,
            f.purchase.price
        );
        let prepared = f.terminal(&funding, outcome)?;
        assert_eq!(
            prepared.transaction.lines.len(),
            if outcome == PurchaseOutcome::Granted {
                7
            } else {
                0
            }
        );
        let mut expected = f.old.state.fold.clone();
        expected.fold_entry(prepared.transaction.journal_seq, &prepared.transaction);
        let tx = prepared.transaction.clone();
        let (_, patch, _) = install::finish_terminal_images(
            &f.old.state,
            f.old.world.world_ic_now().clock,
            prepared,
        )?;
        patch.bind(&mut f.old.state)?.apply();
        assert_eq!(f.old.state.fold, expected);
        let payer = f
            .old
            .state
            .snapshot
            .account(f.purchase.payer)
            .ok_or("payer")?;
        assert_eq!(payer.held_amount, crate::Amount::ZERO);
        assert_eq!(
            payer.posted_balance.minor(),
            if outcome == PurchaseOutcome::Granted {
                900
            } else {
                1000
            }
        );
        for payout in f.purchase.payouts()? {
            assert_eq!(
                f.old
                    .state
                    .snapshot
                    .account(payout.account)
                    .ok_or("payee")?
                    .posted_balance
                    .minor(),
                if outcome == PurchaseOutcome::Granted {
                    payout.amount.minor()
                } else {
                    0
                }
            );
        }
        let audit = &f.old.state.fold.purchase_receipts;
        let history = audit.get(funding.operation())?;
        assert!(history.matches_entitlement(&f.purchase, funding.receipt()));
        assert!(!history.matches_purchase(&f.old.request, funding.receipt()));
        assert!(history.terminal().is_some());
        assert!(audit.prepare(&tx).is_err());
        assert!(f.terminal(&funding, outcome).is_err());
    }
    Ok(())
}

#[test]
fn payer_royalty_is_a_real_credit_beside_full_gross_escrow_debit() -> TestResult {
    let mut f = EntitlementFixture::new()?;
    let parent = f.purchase.parents.first_mut().ok_or("parent")?;
    let mut title = f
        .old
        .state
        .snapshot
        .asset(parent.charter)
        .ok_or("parent title")?
        .clone();
    title.title_owner = PrincipalId::Object(f.purchase.buyer);
    f.old.state.snapshot.put_asset(title);
    parent.title.as_mut().ok_or("title")?.recipient = Recipient {
        account: f.purchase.payer,
        owner: PrincipalId::Object(f.purchase.buyer),
    };
    let funding = f.fund()?;
    let prepared = f.terminal(&funding, PurchaseOutcome::Granted)?;
    let mut payer_lines = prepared
        .transaction
        .lines
        .iter()
        .filter(|l| l.account == f.purchase.payer)
        .map(|l| l.delta.minor())
        .collect::<Vec<_>>();
    payer_lines.sort();
    assert_eq!(payer_lines, vec![-100, 10]);
    assert_eq!(
        prepared
            .transaction
            .hold_effects
            .first()
            .ok_or("capture")?
            .hold
            .amount
            .minor(),
        100
    );
    let actual = prepared.transaction.clone();
    let mut netted = actual.clone();
    netted
        .lines
        .retain(|l| l.account != f.purchase.payer || l.delta.is_negative());
    netted
        .lines
        .iter_mut()
        .find(|l| l.account == f.purchase.payer)
        .ok_or("debit")?
        .delta = crate::Amount::from_minor(-90);
    assert!(f.old.state.fold.purchase_receipts.prepare(&netted).is_err());
    let (_, patch, _) =
        install::finish_terminal_images(&f.old.state, f.old.world.world_ic_now().clock, prepared)?;
    patch.bind(&mut f.old.state)?.apply();
    assert_eq!(
        f.old
            .state
            .snapshot
            .account(f.purchase.payer)
            .ok_or("payer")?
            .posted_balance
            .minor(),
        910
    );
    let mut replay = PurchaseReceiptAudit::default();
    for tx in f.old.state.transactions.values() {
        replay.observe(tx);
    }
    replay.finish_replay()?;
    assert_eq!(replay, f.old.state.fold.purchase_receipts);
    Ok(())
}

#[test]
fn role_floors_precede_merge_and_zero_royalties_remain_explicit() -> TestResult {
    let mut f = EntitlementFixture::new()?;
    f.purchase.venue = Some(GrossRecipientCut {
        permille: 100,
        recipient: f.purchase.registry.recipient.clone(),
    });
    f.purchase.price = crate::Amount::from_minor(9);
    assert_eq!(
        f.purchase.payouts()?,
        vec![PurchasePayout {
            account: AccountId(1),
            owner: PrincipalId::Object(f.purchase.seller),
            amount: crate::Amount::from_minor(9)
        }]
    );
    f.purchase.price = crate::Amount::from_minor(101);
    let payouts = f.purchase.payouts()?;
    assert_eq!(payouts.len(), 5);
    assert_eq!(
        payouts
            .iter()
            .find(|p| p.account == AccountId(2))
            .ok_or("merged")?
            .amount
            .minor(),
        20
    );
    assert_eq!(
        payouts
            .iter()
            .find(|p| p.account == AccountId(1))
            .ok_or("remainder")?
            .amount
            .minor(),
        51
    );
    let funding = f.fund()?;
    assert_eq!(
        f.terminal(&funding, PurchaseOutcome::Granted)?
            .transaction
            .lines
            .len(),
        6
    );
    let mut zero = f.purchase.clone();
    for parent in &mut zero.parents {
        parent.permille = 0;
        parent.title = None;
    }
    assert!(zero.payouts().is_ok());
    assert_ne!(zero.economic_digest()?, f.purchase.economic_digest()?);
    Ok(())
}

#[test]
fn parent_rates_total_rates_depth_identity_and_legacy_three_payee_ceiling_refuse() -> TestResult {
    let f = EntitlementFixture::new()?;
    let original = f.purchase.clone();
    for mode in 0..7 {
        let mut p = original.clone();
        match mode {
            0 => p.parents.first_mut().ok_or("parent")?.permille = 101,
            1 => p.registry.permille = 1001,
            2 => p.registry.permille = 800,
            3 => p.parents.push(p.parents.first().ok_or("parent")?.clone()),
            4 => p.parents.first_mut().ok_or("parent")?.charter = p.charter,
            5 => p.protocol = EntitlementProtocol::OriginalV3,
            _ => p.parents.first_mut().ok_or("parent")?.title = None,
        }
        assert!(p.payouts().is_err(), "mode {mode}");
    }
    let mut old = f.old.request.clone();
    old.payouts = f.purchase.payouts()?;
    assert!(
        old.validate_shape().is_err(),
        "new recipe cannot widen original maximum"
    );
    Ok(())
}

#[test]
fn actual_parent_title_drift_blocks_grant_and_bind_but_not_frozen_refund() -> TestResult {
    let mut f = EntitlementFixture::new()?;
    let funding = f.fund()?;
    let granted = f.terminal(&funding, PurchaseOutcome::Granted)?;
    let (_, patch, _) =
        install::finish_terminal_images(&f.old.state, f.old.world.world_ic_now().clock, granted)?;
    let parent = f.purchase.parents.first().ok_or("parent")?;
    let mut title = f
        .old
        .state
        .snapshot
        .asset(parent.charter)
        .ok_or("title")?
        .clone();
    title.revision += 1;
    title.frozen = true;
    f.old.state.snapshot.put_asset(title);
    let before = f.old.state.snapshot.clone();
    assert!(patch.bind(&mut f.old.state).is_err());
    assert_eq!(f.old.state.snapshot, before);
    assert!(f.terminal(&funding, PurchaseOutcome::Granted).is_err());
    f.old.world.set_now(f.purchase.expires_at_utc + 1);
    let refund = f.terminal(&funding, PurchaseOutcome::Refunded)?;
    assert!(refund.transaction.lines.is_empty());
    let (_, patch, _) =
        install::finish_terminal_images(&f.old.state, f.old.world.world_ic_now().clock, refund)?;
    patch.bind(&mut f.old.state)?.apply();
    assert_eq!(
        f.old
            .state
            .snapshot
            .account(f.purchase.payer)
            .ok_or("payer")?
            .held_amount,
        crate::Amount::ZERO
    );
    Ok(())
}

#[test]
fn rounded_zero_roles_still_require_and_capture_exact_recipient_accounts() -> TestResult {
    let mut f = EntitlementFixture::new()?;
    f.purchase.price = crate::Amount::from_minor(1);
    assert_eq!(f.purchase.payouts()?.len(), 1);
    let prepared = f.prepare()?;
    let (_, patch, _) =
        install::finish_images(&f.old.state, f.old.world.world_ic_now().clock, prepared)?;
    let id = f
        .purchase
        .parents
        .first()
        .and_then(|p| p.title.as_ref())
        .ok_or("parent")?
        .recipient
        .account;
    let mut account = f
        .old
        .state
        .snapshot
        .account(id)
        .ok_or("parent account")?
        .clone();
    account.revision += 1;
    account.owner = PrincipalId::Object(f.purchase.seller);
    f.old.state.snapshot.put_account(account);
    assert!(patch.bind(&mut f.old.state).is_err());
    assert!(f.prepare().is_err());
    let mut crossed = f.purchase.clone();
    crossed.venue.as_mut().ok_or("venue")?.recipient = Some(Recipient {
        account: crossed
            .registry
            .recipient
            .as_ref()
            .ok_or("registry")?
            .account,
        owner: PrincipalId::Object(crossed.seller),
    });
    assert!(
        crossed.payouts().is_err(),
        "rounding cannot hide crossed ownership"
    );
    Ok(())
}

pub(in crate::managed_purchase) fn history_fixture() -> Result<
    (
        PurchaseReceiptAudit,
        EconomicTransaction,
        [EconomicTransaction; 2],
    ),
    Box<dyn std::error::Error>,
> {
    let mut f = EntitlementFixture::new()?;
    let funding = f.fund()?;
    let tx = f
        .old
        .state
        .transactions
        .get(&funding.receipt().transaction())
        .ok_or("funding")?
        .clone();
    Ok((
        f.old.state.fold.purchase_receipts.clone(),
        tx,
        [
            f.terminal(&funding, PurchaseOutcome::Granted)?.transaction,
            f.terminal(&funding, PurchaseOutcome::Refunded)?.transaction,
        ],
    ))
}

#[test]
fn exact_new_protocol_markers_refuse_legacy_authority_and_crossed_subjects() -> TestResult {
    for protocol in [
        EntitlementProtocol::OriginalV3,
        EntitlementProtocol::DerivedV1,
    ] {
        let mut f = EntitlementFixture::new()?;
        f.purchase.protocol = protocol;
        if protocol == EntitlementProtocol::OriginalV3 {
            f.purchase.parents.clear();
        }
        let prepared = f.prepare()?;
        let wire = serde_json::to_value(prepared.output.marker())?;
        assert_eq!(wire.get("kind"), Some(&json!("mind_entitlement")));
        let decoded: ManagedConsent = serde_json::from_value(wire.clone())?;
        assert_eq!(&decoded, prepared.output.marker());
        assert!(
            crate::contract::managed::require_legacy_plan(&prepared.plan, &f.old.state.snapshot)
                .is_err()
        );
        let mut unknown = wire.clone();
        unknown
            .as_object_mut()
            .ok_or("marker object")?
            .insert("authority".into(), json!(true));
        assert!(serde_json::from_value::<ManagedConsent>(unknown).is_err());
        let mut foreign = wire.clone();
        *foreign
            .pointer_mut("/effect/version")
            .ok_or("effect version")? = json!(99);
        assert!(serde_json::from_value::<ManagedConsent>(foreign).is_err());
        let encoded = serde_json::to_string(&wire)?;
        let duplicate = encoded.replacen("{", "{\"kind\":\"mind_entitlement\",", 1);
        assert!(serde_json::from_str::<ManagedConsent>(&duplicate).is_err());
        let funding = f.fund()?;
        f.purchase.grant_economics_digest = DefinitionDigest::from_bytes([99; 32]);
        assert!(f.terminal(&funding, PurchaseOutcome::Granted).is_err());
        assert!(
            f.terminal(&funding, PurchaseOutcome::Refunded).is_err(),
            "refund still requires the frozen subject"
        );
    }
    Ok(())
}
