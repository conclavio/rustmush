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
            buyer: p.buyer.into(),
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
        check_eq!(purchase.payouts()?.len(), 6);
        Ok(Self {
            old,
            purchase,
            parents,
        })
    }
    fn fund(&mut self) -> TestResult<FundingOutput> {
        let prepared = self.prepare_fund(GOD)?;
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
    /// The Fund prepared as `actor` (its own principal), installing nothing.
    fn prepare_fund(&self, actor: ObjId) -> TestResult<FundingPreparation> {
        let action = effect(self.purchase.protocol, "fund")?;
        Ok(prepare_funding_in(
            store(&self.old.tw)?,
            &FundingContext {
                world: self.old.tw.world(),
                actor,
                principal: &mush_core::PrincipalId::Object { id: actor },
                action: &action,
                commit: CommitToken::from_bytes([3; 16]),
            },
            PurchaseSubject::try_from(self.purchase.clone())?,
        )?)
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
    fn restart(&self, disk: BootMap) -> TestResult<Box<TestWorld>> {
        let mut tw = world(true)?;
        check_eq!(
            objects(&mut tw)?,
            (self.purchase.seller, self.purchase.charter)
        );
        check_eq!(parent_objects(&mut tw)?, self.parents);
        tw.set_now(self.old.tw.world().now());
        tw.try_modules_loaded(disk)?;
        check!(!store(&tw)?.is_frozen());
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
        check!(history.matches_entitlement(&self.purchase, funding.receipt()));
        check_eq!(history.entitlement()?.as_ref(), Some(&self.purchase));
        check_eq!(
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
        check_eq!(
            store(&cold)?.boot_replay().ok_or("boot")?.from_checkpoint,
            Some(covered)
        );
        check!(
            store(&cold)?
                .transaction(funding.receipt().transaction())
                .is_none()
        );
        f.check(&cold, &funding, None)?;
        let full = f.restart(no_checkpoint(f.old.disk.clone())?)?;
        f.check(&full, &funding, None)?;
        check_eq!(
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
        check_eq!(
            store(&cold)?.boot_replay().ok_or("boot")?.from_checkpoint,
            Some(covered)
        );
        f.check(&cold, &funding, Some(outcome))?;
        f.check(&full, &funding, Some(outcome))?;
        check_eq!(
            store(&cold)?.purchase_receipt_audit(),
            store(&full)?.purchase_receipt_audit()
        );
        for payout in f.purchase.payouts()? {
            let a = store(&cold)?
                .snapshot()
                .account(payout.account)
                .ok_or("payee")?;
            check_eq!(Some(a), store(&full)?.snapshot().account(payout.account));
            check_eq!(
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
        check_eq!(
            payer.posted_balance.minor(),
            10240
                - if outcome == PurchaseOutcome::Granted {
                    100
                } else {
                    0
                }
        );
        check_eq!(payer.held_amount, crate::Amount::ZERO);
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
    check!(crate::test_checks::refused(f.restart(missing))?);
    let (_, floor, head) = f.old.checkpoint()?;
    let mut missing = f.old.disk.clone();
    damage(&mut missing, &head, Damage::OmittedProof)?;
    let mut cold = world(true)?;
    objects(&mut cold)?;
    parent_objects(&mut cold)?;
    cold.set_now(f.old.tw.world().now());
    check!(matches!(
        cold.try_modules_loaded(narrow(missing.clone(), floor)?),
        Err(ModError::RetryFullFetch { .. })
    ));
    let recovered = f.restart(missing)?;
    f.check(&recovered, &funding, None)?;
    Ok(())
}

/// Doc 17h §3 M8f (fix round 1 H1, round 2): the Fund subject funds only as
/// the character that quoted. GOD's own purchase (`Holder::Character(GOD)`)
/// is refused to another character, its principal its own, and the same
/// purchase then prepares as GOD: the actor alone was refused. An
/// organisation's purchase is the same check on its `acting` character
/// (`Holder::acting_character`): one quoted by GOD for an organisation is
/// refused to the other character too.
#[test]
fn fund_is_refused_to_any_actor_but_the_quoting_character() -> TestResult {
    let mut f = ColdFixture::new()?;
    let other = f
        .old
        .tw
        .world_mut()
        .create(ObjKind::Player, "Another member", GOD, None)?;
    check!(f.prepare_fund(other).is_err(), "another character's Fund");
    let personal = f.purchase.clone();
    f.purchase.buyer = crate::Holder::Organization {
        org: mush_org::OrgId(1),
        acting: GOD,
    };
    check!(f.prepare_fund(other).is_err(), "another member's Fund");
    f.purchase = personal;
    check!(f.prepare_fund(GOD).is_ok(), "the quoting character's Fund");
    Ok(())
}
