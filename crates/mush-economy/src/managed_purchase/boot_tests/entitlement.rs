//! Real public E4 setup, owner images, checkpoint/full-journal recovery. No
//! receipt completion setter or commercial action/grant fixture is introduced.
use super::*;
use crate::managed_entitlement::*;

struct ColdFixture {
    old: Fixture,
    purchase: EntitlementPurchase,
    parents: Vec<(ObjId, ObjId)>,
}
fn effect(protocol: EntitlementProtocol, transition: &str) -> TestResult<DefinitionRef> {
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
fn parent_objects(tw: &mut TestWorld) -> TestResult<Vec<(ObjId, ObjId)>> {
    (0..3)
        .map(|ordinal| {
            let owner = tw.world_mut().create(
                ObjKind::Player,
                &format!("Parent owner {ordinal}"),
                GOD,
                None,
            )?;
            let charter = tw
                .world_mut()
                .create(ObjKind::Thing, "Parent charter", GOD, None)?;
            Ok((owner, charter))
        })
        .collect()
}
impl ColdFixture {
    fn new() -> TestResult<Self> {
        let mut old = Fixture::new(true)?;
        let parents = parent_objects(&mut old.tw)?;
        run(&mut old.tw, "@account/system test.venue=cr");
        let venue =
            PrincipalId::System(mush_org::SystemPrincipalId::new("test.venue").ok_or("venue")?);
        let venue_account = store(&old.tw)?
            .snapshot()
            .accounts()
            .find(|a| a.owner == venue)
            .ok_or("venue wallet")?
            .id;
        let mut cuts = Vec::new();
        for (owner, charter) in &parents {
            run(&mut old.tw, &format!("@account/open #{}=cr", owner.0));
            run(
                &mut old.tw,
                &format!("@asset/title #{}=#{}", charter.0, owner.0),
            );
            let state = store(&old.tw)?;
            let recipient = state
                .snapshot()
                .accounts()
                .find(|a| a.owner == PrincipalId::Object(*owner))
                .ok_or("parent wallet")?;
            let title = state
                .snapshot()
                .asset(*charter)
                .ok_or("actual parent title")?;
            cuts.push(RoyaltyRecipientCut {
                charter: *charter,
                permille: 100,
                title: Some(RoyaltyTitleRecipient {
                    title_revision: title.revision,
                    recipient: Recipient {
                        account: recipient.id,
                        owner: recipient.owner.clone(),
                    },
                }),
            });
        }
        fold(&mut old.tw, &mut old.disk);
        let p = &old.purchase;
        let purchase = EntitlementPurchase {
            protocol: EntitlementProtocol::DerivedV1,
            quote: p.quote,
            quote_digest: p.quote_digest,
            product_digest: p.product_digest,
            buyer: p.buyer,
            seller: p.seller,
            charter: p.charter,
            title_revision: p.title_revision,
            payer: p.payer,
            instrument: p.instrument.clone(),
            price: p.price,
            registry: GrossRecipientCut {
                permille: 100,
                recipient: Some(Recipient {
                    account: AccountId(3),
                    owner: PrincipalId::System(
                        mush_org::SystemPrincipalId::new("roll.registry").ok_or("registry")?,
                    ),
                }),
            },
            venue: Some(GrossRecipientCut {
                permille: 50,
                recipient: Some(Recipient {
                    account: venue_account,
                    owner: venue,
                }),
            }),
            parents: cuts,
            remainder: Recipient {
                account: AccountId(2),
                owner: PrincipalId::Object(p.seller),
            },
            royalty_chain_digest: DefinitionDigest::from_bytes([31; 32]),
            grant_economics_digest: DefinitionDigest::from_bytes([32; 32]),
            quoted_at_utc: p.quoted_at_utc,
            expires_at_utc: p.expires_at_utc,
        };
        assert_eq!(purchase.payouts()?.len(), 6);
        Ok(Self {
            old,
            purchase,
            parents,
        })
    }
    fn fund(&mut self) -> TestResult<FundingOutput> {
        let action = effect(self.purchase.protocol, "fund")?;
        let prepared = prepare_funding_in(
            store(&self.old.tw)?,
            &FundingContext {
                world: self.old.tw.world(),
                actor: GOD,
                principal: &mush_core::PrincipalId::Object { id: GOD },
                action: &action,
                commit: CommitToken::from_bytes([3; 16]),
            },
            PurchaseSubject::try_from(self.purchase.clone())?,
        )?;
        let (out, patch, writes) = install::finish_images(
            store(&self.old.tw)?,
            self.old.tw.world().world_ic_now().clock,
            prepared,
        )?;
        patch
            .bind(
                self.old
                    .tw
                    .world_mut()
                    .module_state_mut::<EconomyStore>()
                    .ok_or("owner")?,
            )?
            .apply();
        persist(&mut self.old.disk, writes)?;
        Ok(out)
    }
    fn terminal(
        &mut self,
        funding: &FundingOutput,
        outcome: PurchaseOutcome,
    ) -> TestResult<TerminalOutput> {
        let action = effect(
            self.purchase.protocol,
            if outcome == PurchaseOutcome::Granted {
                "grant"
            } else {
                "refund"
            },
        )?;
        let prepared = terminal::prepare_in(
            store(&self.old.tw)?,
            &FundingContext {
                world: self.old.tw.world(),
                actor: GOD,
                principal: &mush_core::PrincipalId::Object { id: GOD },
                action: &action,
                commit: CommitToken::from_bytes([4; 16]),
            },
            SubjectTerminal::try_from(EntitlementTerminal {
                operation: funding.operation(),
                purchase: self.purchase.clone(),
                funding: funding.receipt().clone(),
            })?,
            outcome,
        )?;
        let (out, patch, writes) = install::finish_terminal_images(
            store(&self.old.tw)?,
            self.old.tw.world().world_ic_now().clock,
            prepared,
        )?;
        patch
            .bind(
                self.old
                    .tw
                    .world_mut()
                    .module_state_mut::<EconomyStore>()
                    .ok_or("owner")?,
            )?
            .apply();
        persist(&mut self.old.disk, writes)?;
        Ok(out)
    }
    fn restart(&self, disk: BootMap) -> TestResult<TestWorld> {
        let mut tw = world(true)?;
        assert_eq!(
            objects(&mut tw)?,
            (self.purchase.seller, self.purchase.charter)
        );
        assert_eq!(parent_objects(&mut tw)?, self.parents);
        tw.set_now(self.old.tw.world().now());
        tw.try_modules_loaded(disk)?;
        assert!(!store(&tw)?.is_frozen());
        Ok(tw)
    }
    fn check(
        &self,
        tw: &TestWorld,
        funding: &FundingOutput,
        terminal: Option<PurchaseOutcome>,
    ) -> TestResult {
        let state = store(tw)?;
        let history = state.purchase_receipt_audit().get(funding.operation())?;
        assert!(history.matches_entitlement(&self.purchase, funding.receipt()));
        assert_eq!(history.entitlement()?.as_ref(), Some(&self.purchase));
        assert_eq!(
            history.terminal().map(|t| t.granted()),
            terminal.map(|o| o == PurchaseOutcome::Granted)
        );
        history.verify_documents(
            state
                .snapshot()
                .contract(funding.receipt().contract())
                .ok_or("contract")?,
            state
                .snapshot()
                .hold(funding.receipt().hold())
                .ok_or("hold")?,
        )?;
        Ok(())
    }
}

#[test]
fn six_payee_history_survives_real_cold_checkpoint_then_grant_or_refund_and_full_replay()
-> TestResult {
    for outcome in [PurchaseOutcome::Granted, PurchaseOutcome::Refunded] {
        let mut f = ColdFixture::new()?;
        let funding = f.fund()?;
        let (covered, floor, _) = f.old.checkpoint()?;
        let narrowed = narrow(f.old.disk.clone(), floor)?;
        older_than_loaded(&narrowed, funding.receipt().journal_seq())?;
        let cold = f.restart(narrowed)?;
        assert_eq!(
            store(&cold)?.boot_replay().ok_or("boot")?.from_checkpoint,
            Some(covered)
        );
        assert!(
            store(&cold)?
                .transaction(funding.receipt().transaction())
                .is_none()
        );
        f.check(&cold, &funding, None)?;
        let full = f.restart(no_checkpoint(f.old.disk.clone())?)?;
        f.check(&full, &funding, None)?;
        assert_eq!(
            store(&cold)?.purchase_receipt_audit(),
            store(&full)?.purchase_receipt_audit()
        );
        f.old.tw = cold;
        fold(&mut f.old.tw, &mut f.old.disk);
        if outcome == PurchaseOutcome::Refunded {
            f.old.tw.set_now(f.purchase.expires_at_utc + 1);
        }
        let terminal = f.terminal(&funding, outcome)?;
        let (covered, floor, _) = f.old.checkpoint()?;
        let narrowed = narrow(f.old.disk.clone(), floor)?;
        older_than_loaded(&narrowed, terminal.journal_seq())?;
        let cold = f.restart(narrowed)?;
        let full = f.restart(no_checkpoint(f.old.disk.clone())?)?;
        assert_eq!(
            store(&cold)?.boot_replay().ok_or("boot")?.from_checkpoint,
            Some(covered)
        );
        f.check(&cold, &funding, Some(outcome))?;
        f.check(&full, &funding, Some(outcome))?;
        assert_eq!(
            store(&cold)?.purchase_receipt_audit(),
            store(&full)?.purchase_receipt_audit()
        );
        for payout in f.purchase.payouts()? {
            let a = store(&cold)?
                .snapshot()
                .account(payout.account)
                .ok_or("payee")?;
            assert_eq!(Some(a), store(&full)?.snapshot().account(payout.account));
            assert_eq!(
                a.posted_balance.minor(),
                if outcome == PurchaseOutcome::Granted {
                    payout.amount.minor()
                } else {
                    0
                }
            );
        }
        let payer = store(&full)?
            .snapshot()
            .account(f.purchase.payer)
            .ok_or("payer")?;
        assert_eq!(
            payer.posted_balance.minor(),
            10240
                - if outcome == PurchaseOutcome::Granted {
                    100
                } else {
                    0
                }
        );
        assert_eq!(payer.held_amount, crate::Amount::ZERO);
    }
    Ok(())
}

#[test]
fn new_namespace_missing_history_or_missing_contract_cannot_boot_as_legacy() -> TestResult {
    let mut f = ColdFixture::new()?;
    let funding = f.fund()?;
    let mut missing = f.old.disk.clone();
    missing
        .get_mut("economy")
        .ok_or("economy")?
        .kv
        .retain(|(key, _)| {
            key.as_str() != crate::api::contract_key(funding.receipt().contract()).as_str()
        });
    assert!(f.restart(missing).is_err());
    let (_, floor, head) = f.old.checkpoint()?;
    let mut missing = f.old.disk.clone();
    damage(&mut missing, &head, Damage::OmittedProof)?;
    let mut cold = world(true)?;
    objects(&mut cold)?;
    parent_objects(&mut cold)?;
    cold.set_now(f.old.tw.world().now());
    assert!(matches!(
        cold.try_modules_loaded(narrow(missing.clone(), floor)?),
        Err(ModError::RetryFullFetch { .. })
    ));
    let recovered = f.restart(missing)?;
    f.check(&recovered, &funding, None)?;
    Ok(())
}
