#![cfg(test)]

//! Required expensive cross-owner gate. The certificate below comes only from
//! the complete actual Registry Machine and acknowledged registered actions.
//! No synthetic rating rows, source-readiness setter, or grant constructor is
//! used. Core installation models the receipt boundary; this is not a claim
//! about the server transport's Unknown/ACK lifecycle.

use super::*;
#[path = "original_purchase_driver_tests.rs"]
mod driver;
use mush_core::{Obj, ObjKind};
use mush_economy::{
    Account as Wallet, AccountId as WalletId, AccountStatus, CreditPolicy, EconomyStore,
    InstrumentDef, InstrumentId, InstrumentKind, InstrumentStatus, NegativePolicy,
};
use mush_market::mind_trade::{
    OriginalDuration, OriginalPrice, OriginalPurchaseConfig, OriginalQuoteConfig,
    OriginalQuotePolicy, records::QuoteVenue,
};
use mush_market::{MarketId, MarketModule, Presence, VenueGate, VenueRefusal};
use mush_org::{PrincipalId as EconomicOwner, SystemPrincipalId};
use mush_roll::commerce::{
    paid::{PaidTermRecord, PersonalPaidLicence},
    purchase::PurchaseParticipants,
    purchase_writer::OriginalPurchaseWriter,
    rating::{RatingReceipt, RatingRef},
};

#[path = "charter_transfer_fences.rs"]
mod charter_transfer_fences;
#[path = "charter_transfer_fixture.rs"]
mod charter_transfer_fixture;
#[path = "charter_transfer_integration_tests.rs"]
mod charter_transfers;
#[path = "house_filing_integration_tests.rs"]
mod house_filing;
#[path = "ic_integration_tests.rs"]
mod ic_tests;
#[path = "personal_paid_checkout_tests.rs"]
mod paid_checkout;
#[path = "personal_paid_holding_tests.rs"]
mod paid_holdings;
#[path = "original_source_form_integration_tests.rs"]
mod source_forms;

const BUYER: ObjId = ObjId(2);
const ROOM: ObjId = ObjId(0);
const TERM_SECONDS: i64 = 90 * 86_400;
const GRACE_SECONDS: i64 = 3 * 86_400;
const QUOTE_SECONDS: i64 = 600;

fn token(number: u128) -> CommitToken {
    CommitToken::from_bytes(number.to_be_bytes())
}
fn request(
    number: u128,
    actor: ObjId,
    owner: &str,
    parameters: Value,
) -> TestResult<ActionInvocation> {
    let request = RequestUid::from_bytes(number.to_be_bytes());
    let principal = PrincipalId::Object { id: actor };
    Ok(ActionInvocation::new(ActionInvocationDraft {
        request,
        principal: principal.clone(),
        actor,
        offered_action: ActionOfferId(CanonicalHash::from_bytes([31; 32])),
        subject: PerceivedRef::SelfActor,
        targets: vec![],
        parameters,
        idempotency: ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal,
                owner: OwnerId::new(owner)?,
                namespace: CanonicalId::new("original_purchase_fixture")?,
            },
            1,
            IdempotencyKey::Request(request),
        ),
    })?)
}

/// The ordinary fixture tests one quantum; this gate runs every configured
/// case at its actual per-case quota. Operation IDs have full u128 space, not
/// the parent's deliberately small one-quantum fixture counter.
fn complete_registry(f: &mut Fixture) -> TestResult<RatingRef> {
    let (binding, mut ready) = f.admitted()?;
    let executor = Executor::with_wake(&BulkConfig::default(), None)?;
    let deadline = Instant::now() + Duration::from_secs(7_200);
    let mut next = 1_u128 << 64;
    let mut total_actual = 0_u64;
    let mut quanta = 0_u64;
    loop {
        if Instant::now() >= deadline {
            return Err(format!(
                "full Registry hang guard; {quanta} quanta, {total_actual} unit-ms"
            )
            .into());
        }
        let ticket = f
            .artifacts_mut()?
            .registry
            .grant(binding, ready.next_grant_ms())?;
        let grant = token(next);
        let invocation = request(
            next,
            GOD,
            "roll",
            json!({"ticket":ticket.get(),"job":binding.job}),
        )?;
        next = next.checked_add(1).ok_or("operation counter exhausted")?;
        let action = f.action(Transition::Grant)?;
        let (prepared, _) = f.engine.prepare_registered_composed_authoritative_action(
            &action,
            &invocation,
            grant,
        )?;
        registry_ack(f, prepared, ticket, binding, grant, Transition::Grant)?;
        let lease = f
            .engine
            .world
            .module_state_mut::<BulkStore>()
            .ok_or("quota owner")?
            .take_registry_lease(grant)
            .ok_or("acknowledged lease")?;
        let allowance = lease
            .maximum_sim_ms()
            .checked_mul(lease.units())
            .ok_or("allowance overflow")?;
        executor.submit(lease, ready).map_err(|(e, _, _)| e)?;
        let mut done = loop {
            if let Some(done) = executor.poll() {
                break done;
            }
            if Instant::now() >= deadline {
                return Err(
                    format!("actual Registry worker timed out after {quanta} quanta").into(),
                );
            }
            std::thread::sleep(Duration::from_millis(2));
        };
        check!(done.receipt.actual_unit_ms <= allowance);
        total_actual = total_actual
            .checked_add(done.receipt.actual_unit_ms)
            .ok_or("usage overflow")?;
        let continuation = done.ready.take();
        let settlement = done
            .registry_settlement
            .take()
            .ok_or("actual measured settlement")??;
        let ticket = f
            .artifacts_mut()?
            .registry
            .insert_settlement(settlement)
            .map_err(|(e, _)| e)?;
        let settle = token(next);
        let invocation = request(
            next,
            GOD,
            "roll",
            json!({"ticket":ticket.get(),"job":binding.job}),
        )?;
        next = next.checked_add(1).ok_or("operation counter exhausted")?;
        let before = f.run(binding)?;
        let action = f.action(Transition::Settle)?;
        let (prepared, _) = f.engine.prepare_registered_composed_authoritative_action(
            &action,
            &invocation,
            settle,
        )?;
        check_eq!(
            f.run(binding)?,
            before,
            "worker result does not publish before acknowledgement"
        );
        registry_ack(f, prepared, ticket, binding, settle, Transition::Settle)?;
        quanta += 1;
        let run = f.run(binding)?;
        check_eq!(run.charged_unit_ms, total_actual);
        check!(run.flight.is_none());
        if run.status == Status::Complete {
            // Registry retains its actual Machine through the acknowledged
            // terminal settlement, just as the production Settle phase does.
            let retained = continuation.ok_or("terminal Registry machine")?;
            check_eq!(retained.account(), binding.account);
            check_eq!(retained.run(), binding.run);
            check!(run.result.is_some());
            drop(retained); // Only after registry_ack installed the terminal image.
            break;
        }
        if run.status.terminal() {
            let job: RatingJob = f.retained("rating_jobs")?;
            eprintln!(
                "M7_ACTUAL_PURCHASE_REGISTRY_FAILURE quanta={quanta} unit_ms={total_actual} status={:?} cursor={:?} next_sequence={} result_present={} typed_job={:?}",
                run.status,
                run.registry,
                run.next_sequence,
                run.result.is_some(),
                job.state(),
            );
            return Err(format!("Registry ended without certification: {:?}", run.status).into());
        }
        ready = continuation.ok_or("nonterminal continuation")?;
    }
    let receipt: RatingReceipt = f.retained("rating_receipts")?;
    let reference = receipt.reference()?;
    check_eq!(reference.job(), binding.job);
    let job: RatingJob = f.retained("rating_jobs")?;
    check!(matches!(job.state(), RatingJobState::Completed { .. }));
    check!(
        f.disk
            .keys()
            .all(|(_, k)| !k.starts_with("action-state/capacity/v1/"))
    );
    f.restart_terminal(binding, Status::Complete)?;
    eprintln!(
        "M7_ACTUAL_PURCHASE_CERTIFICATE quanta={quanta} unit_ms={total_actual} reference={reference:?}"
    );
    Ok(reference)
}

fn registry_ack(
    f: &mut Fixture,
    prepared: PreparedComposedTxn,
    ticket: TicketId,
    binding: Binding,
    operation: CommitToken,
    transition: Transition,
) -> TestResult {
    install(&mut f.engine, &mut f.disk, prepared)?;
    let ack = f
        .artifacts_mut()?
        .registry
        .take_acknowledged()
        .ok_or("actual owner Ack")?;
    check_eq!(
        (ack.ticket, ack.job, ack.operation, ack.transition),
        (ticket, binding.job, operation, transition)
    );
    check!(f.artifacts()?.registry.get(ticket).is_err());
    Ok(())
}

fn issuer() -> TestResult<SystemPrincipalId> {
    SystemPrincipalId::new("roll.registry").ok_or_else(|| "Registry issuer".into())
}
fn filing_policy() -> TestResult<mush_economy::registry_filing::InitialFilingPolicy> {
    Ok(mush_economy::registry_filing::InitialFilingPolicy::new(
        issuer()?,
        WalletId(1),
        Amount::from_minor(100),
        CatalogRef {
            ruleset: "roll".into(),
            code: "mind_charter".into(),
        },
    )?)
}
fn reference(owner: &str, kind: &str, id: &str, payload: Value) -> TestResult<DefinitionRef> {
    Ok(DefinitionRegistryBuilder::new().register(
        OwnerId::new(owner)?,
        DefinitionDescriptor::new(
            DefinitionKind::new(kind)?,
            DefinitionId::new(id)?,
            1,
            payload,
        )?,
    )?)
}
#[derive(Clone)]
struct PurchaseConfiguration {
    paid_resales: Option<mush_market::mind_paid_resale::PaidResaleConfig>,
    quotes: OriginalQuoteConfig,
    purchases: OriginalPurchaseConfig,
    participants: PurchaseParticipants,
    writer: OriginalPurchaseWriter,
    dependent: Option<DefinitionRef>,
    compatible_update: Option<DefinitionRef>,
    derived_update: Option<DefinitionRef>,
    charter_transfer: Option<DefinitionRef>,
    entitlements: Option<mush_market::mind_entitlement::EntitlementConfig>,
    renewals: Option<mush_market::mind_renewal::RenewalConfig>,
    minds_facts: Option<mush_roll::commerce::facts::MindsFactProvider>,
}
fn resale_configuration(
    participants: PurchaseParticipants,
) -> Result<mush_market::mind_paid_resale::PaidResaleConfig, ModError> {
    mush_market::mind_paid_resale::PaidResaleConfig::new(
        issuer().map_err(module_error)?,
        GOD,
        600,
        participants,
        mush_roll::commerce::paid_resale::writer::PaidResaleWriter::new(),
        None,
    )
}
fn configuration(
    manifest: DefinitionRef,
    source_forms: bool,
) -> TestResult<Box<PurchaseConfiguration>> {
    let quotes = OriginalQuoteConfig::new(
        OriginalQuotePolicy::new(
            OriginalPrice::new(
                Amount::from_minor(100),
                InstrumentId::new("cr").ok_or("currency")?,
                issuer()?,
                WalletId(1),
                100,
            )?,
            QuoteVenue::new(
                MarketId::new("fixture").ok_or("venue")?,
                EconomicOwner::System(issuer()?),
                WalletId(1),
                0,
            )?,
            OriginalDuration::new(true, TERM_SECONDS, GRACE_SECONDS, QUOTE_SECONDS)?,
        )?,
        manifest,
    )?;
    let quotes = if source_forms {
        quotes.with_source_forms()
    } else {
        quotes
    };
    let retention = DefinitionRegistryBuilder::new().register(
        OwnerId::new("roll")?,
        mush_roll::commerce::rating::registration::commercial_roots_descriptor()?,
    )?;
    let participants = PurchaseParticipants::new(
        reference(
            "economy",
            "owner_participant",
            "initial_filing",
            filing_policy()?.definition_payload(),
        )?,
        retention,
    )?;
    let writer = OriginalPurchaseWriter::new();
    let purchases =
        OriginalPurchaseConfig::new(quotes.clone(), participants.clone(), writer.clone())?;
    Ok(Box::new(PurchaseConfiguration {
        paid_resales: None,
        quotes,
        purchases,
        participants,
        writer,
        dependent: None,
        compatible_update: None,
        derived_update: None,
        charter_transfer: None,
        entitlements: None,
        renewals: None,
        minds_facts: None,
    }))
}
fn entitlement_configuration(
    base: &PurchaseConfiguration,
    manifest: DefinitionRef,
) -> TestResult<mush_market::mind_entitlement::EntitlementConfig> {
    entitlement_configuration_with_window(base, manifest, TERM_SECONDS, GRACE_SECONDS)
}
fn entitlement_configuration_with_window(
    base: &PurchaseConfiguration,
    manifest: DefinitionRef,
    term_seconds: i64,
    grace_seconds: i64,
) -> TestResult<mush_market::mind_entitlement::EntitlementConfig> {
    use mush_economy::managed_entitlement::{GrossRecipientCut, Recipient};
    use mush_market::mind_entitlement::{EntitlementConfig, EntitlementPolicy};
    Ok(EntitlementConfig::new(
        EntitlementPolicy {
            price: Amount::from_minor(100),
            instrument: InstrumentId::new("cr").ok_or("currency")?,
            registry: GrossRecipientCut {
                permille: 100,
                recipient: Some(Recipient {
                    account: WalletId(1),
                    owner: EconomicOwner::System(issuer()?),
                }),
            },
            venue_name: MarketId::new("fixture").ok_or("venue")?,
            venue: GrossRecipientCut {
                permille: 0,
                recipient: None,
            },
            transferable: true,
            term_seconds,
            grace_seconds,
            quote_seconds: QUOTE_SECONDS,
            renewal_price: Amount::from_minor(73),
            renewal_term_seconds: TERM_SECONDS,
            renewal_grace_seconds: GRACE_SECONDS,
        },
        manifest,
        base.participants.clone(),
        mush_roll::commerce::entitlement::writer::EntitlementPurchaseWriter::new(),
    )?)
}
fn renewal_configuration(
    base: &PurchaseConfiguration,
    manifest: DefinitionRef,
) -> TestResult<mush_market::mind_renewal::RenewalConfig> {
    use mush_economy::managed_entitlement::{GrossRecipientCut, Recipient};
    use mush_market::mind_renewal::{RenewalConfig, RenewalPolicy};
    Ok(RenewalConfig::new(
        manifest,
        base.participants.clone(),
        mush_roll::commerce::renewal::writer::RenewalWriter::new(),
        RenewalPolicy {
            registry: GrossRecipientCut {
                permille: 100,
                recipient: Some(Recipient {
                    account: WalletId(1),
                    owner: EconomicOwner::System(issuer()?),
                }),
            },
            venue: None,
            offer_ic_seconds: QUOTE_SECONDS,
        },
    )?)
}
type PurchaseShared = Arc<Mutex<Option<Box<PurchaseConfiguration>>>>;

struct PurchaseRoll {
    paid_resales: bool,
    minds_facts: Option<mush_roll::commerce::facts::MindsFactProvider>,
    source_forms: bool,
    authored_forks: bool,
    marked_registry: bool,
    dependent_owners: bool,
    entitlements: bool,
    renewals: bool,
    renewal_service: bool,
    owner_qualification: bool,
    owner_attestations: bool,
    registry_recall: Option<mush_roll::commerce::registry_recall::RegistryRecallPolicy>,
    registry_revocation: Option<mush_roll::commerce::registry_revocation::RegistryRevocationPolicy>,
    qualification_short_parent: bool,
    author_withdrawal: bool,
    compatible_updates: bool,
    derived_updates: bool,
    shared: Shared,
    purchase: PurchaseShared,
    inner: Option<RollModule>,
}
impl PurchaseRoll {
    #[inline(never)]
    fn configure_renewals(
        &self,
        config: CommerceConfig,
        purchase: &mut PurchaseConfiguration,
        manifest: &DefinitionRef,
    ) -> Result<CommerceConfig, ModError> {
        let config = if self.renewals {
            let renewal =
                renewal_configuration(purchase, manifest.clone()).map_err(module_error)?;
            let renewal = if self.renewal_service {
                renewal.with_service()?
            } else {
                renewal
            };
            let config = config.with_mind_renewals(
                renewal.writer(),
                purchase.participants.clone(),
                renewal.actions()?,
            )?;
            let config = if let Some(service) = renewal.service_action()? {
                config.with_mind_renewal_service(service)?
            } else {
                config
            };
            purchase.renewals = Some(renewal);
            config
        } else {
            config
        };
        Ok(config)
    }
}

impl GameModule for PurchaseRoll {
    fn name(&self) -> &'static str {
        "roll"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let mut shared = self.shared.lock().map_err(module_error)?;
        let manifest = shared
            .manifest
            .clone()
            .ok_or_else(|| ModError::msg("actual Registry manifest"))?;
        let mut purchase =
            configuration(manifest.clone(), self.source_forms).map_err(module_error)?;
        let config = CommerceConfig::new(
            CommerceMode::Active,
            InitialFilingTerms::new(TERM_SECONDS, GRACE_SECONDS).map_err(module_error)?,
            manifest.clone(),
            shared
                .compatibility
                .clone()
                .ok_or_else(|| ModError::msg("actual compatibility"))?,
        )?
        .with_original_purchases(
            purchase.writer.clone(),
            purchase.participants.clone(),
            purchase.purchases.actions()?,
        )?;
        let config = if shared.reconformance {
            config.with_registry_reconformance()?
        } else {
            config
        };
        let config = if self.authored_forks {
            config.with_authored_forks()?
        } else {
            config
        };
        let config = if self.marked_registry {
            config.with_marked_registry()?
        } else {
            config
        };
        let config = if self.dependent_owners {
            let config = config.with_dependent_owner_filings()?;
            purchase.dependent = Some(config.dependent_filing_action()?);
            config
        } else {
            config
        };
        let config = if self.entitlements {
            let entitlement = if self.qualification_short_parent {
                // A lawful explicit fixture offer policy; existing immutable terms
                // remain unchanged when the next composition uses normal durations.
                entitlement_configuration_with_window(&purchase, manifest.clone(), 60, 0)
            } else {
                entitlement_configuration(&purchase, manifest.clone())
            }
            .map_err(module_error)?;
            let config = config.with_mind_entitlements(
                entitlement.writer(),
                purchase.participants.clone(),
                entitlement.owner_actions(
                    mush_economy::managed_entitlement::EntitlementProtocol::OriginalV3,
                )?,
                entitlement.owner_actions(
                    mush_economy::managed_entitlement::EntitlementProtocol::DerivedV1,
                )?,
            )?;
            purchase.entitlements = Some(entitlement);
            config
        } else {
            config
        };
        let config = self.configure_renewals(config, &mut purchase, &manifest)?;
        let config = if self.author_withdrawal {
            config.with_author_withdrawal()?
        } else {
            config
        };
        let config = if self.owner_qualification {
            config.with_owner_parent_qualification()?
        } else {
            config
        };
        let config = if self.owner_attestations {
            config.with_owner_attestations()?
        } else {
            config
        };
        let config = if self.compatible_updates {
            let config = config.with_compatible_updates()?;
            purchase.compatible_update = Some(config.compatible_update_action()?);
            config
        } else {
            config
        };
        let config = if self.derived_updates {
            let config = config.with_derived_updates()?;
            purchase.derived_update = Some(config.derived_update_action()?);
            config
        } else {
            config
        };
        let config = if let Some(policy) = &self.registry_recall {
            config.with_registry_recall(policy.clone())?
        } else {
            config
        };
        let config = if let Some(policy) = &self.registry_revocation {
            config.with_registry_revocation(policy.clone())?
        } else {
            config
        };
        let config = if let Some(provider) = &self.minds_facts {
            purchase.minds_facts = Some(provider.clone());
            config.with_minds_facts(provider.clone())?
        } else {
            config
        };
        let config = if self.paid_resales {
            let resale = resale_configuration(purchase.participants.clone())?;
            let config = config.with_paid_mind_resale(
                resale.writer(),
                purchase.participants.clone(),
                GOD,
            )?;
            purchase.paid_resales = Some(resale);
            config
        } else {
            config
        };
        // Every genuine story that changes a charter title uses the actual
        // closed seller/buyer protocol. Ordinary production composition remains
        // explicit; this is the Active commerce fixture's declared option.
        let charter = mush_roll::commerce::charter_transfer::CharterTransferConfig::new();
        let [_, accept] = charter.actions()?;
        let config = config.with_charter_transfers(charter)?;
        purchase.charter_transfer = Some(accept);
        shared.filing = Some(config.initial_filing_action()?);
        let module =
            Resources::roll_owners(RollModule::with_writers(["tacsim"]).with_commerce(config));
        let mut module = if self.authored_forks {
            module.with_authored_fork_source::<crate::minds::workshop::fork_material::ForkMaterialReader>()
        } else {
            module
        };
        module.register(reg)?;
        *self.purchase.lock().map_err(module_error)? = Some(purchase);
        self.inner = Some(module);
        Ok(())
    }
    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        data: ModuleBootData,
    ) -> Result<(), ModError> {
        self.inner
            .as_mut()
            .ok_or_else(|| ModError::msg("Roll registration"))?
            .on_world_loaded(ctx, data)
    }
    fn finalize_owner_composition(&mut self, ctx: &mut ModuleCtx<'_>) -> Result<(), ModError> {
        self.inner
            .as_mut()
            .ok_or_else(|| ModError::msg("Roll registration"))?
            .finalize_owner_composition(ctx)
    }
}
struct PurchaseEconomy {
    shared: Shared,
    purchase: PurchaseShared,
    inner: Option<EconomyModule>,
}
impl GameModule for PurchaseEconomy {
    fn name(&self) -> &'static str {
        "economy"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let action = self
            .shared
            .lock()
            .map_err(module_error)?
            .filing
            .clone()
            .ok_or_else(|| ModError::msg("actual filing action"))?;
        let purchase = self
            .purchase
            .lock()
            .map_err(module_error)?
            .clone()
            .ok_or_else(|| ModError::msg("purchase composition"))?;
        let mut module = EconomyModule::new()
            .with_title_issuers(vec![issuer().map_err(module_error)?])
            .with_initial_filing(filing_policy().map_err(module_error)?, action)
            .with_original_mind_purchases(purchase.purchases.actions()?);
        if let Some(resale) = purchase.paid_resales {
            module = module.with_paid_mind_resale(resale.economy()?);
        }
        if let Some(provider) = purchase.minds_facts {
            module = module.with_fact_provider(Arc::new(provider));
        }
        if let Some(action) = purchase.charter_transfer {
            module = module.with_mind_charter_transfers(action);
        }
        if let Some(action) = purchase.compatible_update {
            module = module.with_compatible_mind_updates(action);
        }
        if let Some(action) = purchase.derived_update {
            module = module.with_derived_mind_updates(action);
        }
        if let Some(action) = purchase.dependent {
            module = module.with_dependent_initial_filing(action);
        }
        if let Some(entitlement) = purchase.entitlements {
            module = module.with_mind_entitlement_purchases(
                entitlement.purchase_actions(
                    mush_economy::managed_entitlement::EntitlementProtocol::OriginalV3,
                )?,
                entitlement.purchase_actions(
                    mush_economy::managed_entitlement::EntitlementProtocol::DerivedV1,
                )?,
            );
        }
        if let Some(renewal) = purchase.renewals {
            module = module.with_mind_renewals(
                renewal.manifest().clone(),
                [
                    renewal.participants().economy().clone(),
                    renewal.participants().retention().clone(),
                ],
                renewal.actions()?,
            );
            if let Some(service) = renewal.service_action()? {
                module = module.with_mind_renewal_service(service)?;
            }
        }
        module.register(reg)?;
        self.inner = Some(module);
        Ok(())
    }
    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        data: ModuleBootData,
    ) -> Result<(), ModError> {
        self.inner
            .as_mut()
            .ok_or_else(|| ModError::msg("Economy registration"))?
            .on_world_loaded(ctx, data)
    }
}

#[derive(Debug)]
struct Venue;
impl VenueGate for Venue {
    fn venue_of_room(&self, _: &World, room: ObjId) -> Option<MarketId> {
        (room == ROOM).then(|| MarketId::new("fixture")).flatten()
    }
    fn endpoint_for(
        &self,
        _: &World,
        _: ObjId,
        _: &MarketId,
        _: Presence,
    ) -> Option<mush_inventory::EndpointRef> {
        None
    }
    fn admit(
        &self,
        w: &World,
        actor: ObjId,
        venue: &MarketId,
        _: Option<mush_inventory::EndpointRef>,
        _: Presence,
    ) -> Result<(), VenueRefusal> {
        if venue.as_str() == "fixture" && w.containing_room(actor) == Some(ROOM) {
            Ok(())
        } else {
            Err(VenueRefusal {
                line: "The fixture venue requires actual room presence.".into(),
            })
        }
    }
    fn governed(&self, _: &World, _: mush_inventory::EndpointRef) -> bool {
        false
    }
    fn port_seller(&self, _: &World, _: &MarketId, _: &EconomicOwner) -> bool {
        false
    }
}
struct PurchaseMarket {
    purchase: PurchaseShared,
    inner: Option<MarketModule>,
}
impl GameModule for PurchaseMarket {
    fn name(&self) -> &'static str {
        "market"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let config = self
            .purchase
            .lock()
            .map_err(module_error)?
            .clone()
            .ok_or_else(|| ModError::msg("purchase composition"))?;
        let mut module = MarketModule::new()
            .with_venue_gate(Arc::new(Venue))
            .with_original_mind_purchases(config.purchases);
        if let Some(resale) = config.paid_resales {
            module = module.with_paid_mind_resale(resale);
        }
        if let Some(entitlement) = config.entitlements {
            module = module.with_mind_entitlements(entitlement);
        }
        if let Some(renewal) = config.renewals {
            module = module.with_mind_renewals(renewal);
        }
        module.register(reg)?;
        self.inner = Some(module);
        Ok(())
    }
    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        data: ModuleBootData,
    ) -> Result<(), ModError> {
        self.inner
            .as_mut()
            .ok_or_else(|| ModError::msg("Market registration"))?
            .on_world_loaded(ctx, data)
    }
    fn finalize_owner_composition(&mut self, ctx: &mut ModuleCtx<'_>) -> Result<(), ModError> {
        self.inner
            .as_mut()
            .ok_or_else(|| ModError::msg("Market registration"))?
            .finalize_owner_composition(ctx)
    }
}

#[derive(Clone)]
struct Durable {
    disk: Disk,
    objects: Vec<Obj>,
    next_id: u32,
    now: i64,
}
impl Durable {
    fn capture(engine: &Engine, disk: &Disk) -> Self {
        Self {
            disk: disk.clone(),
            objects: engine.world.iter().cloned().collect(),
            next_id: engine.world.next_id(),
            now: engine.world.now(),
        }
    }
    fn world(&self) -> TestResult<World> {
        let mut world = World::from_objects(self.objects.clone(), self.next_id)?;
        world.set_now(self.now);
        Ok(world)
    }
}

fn seed_wallets(disk: &mut Disk) -> TestResult {
    // Legitimate zero-balance personal accounts with bounded credit. There is
    // no fabricated mint/history; the real filing and purchase debit them.
    let instrument = InstrumentDef {
        id: InstrumentId::new("cr").ok_or("currency")?,
        ruleset: "fixture".into(),
        code: "cr".into(),
        kind: InstrumentKind::Currency,
        scale: 0,
        transferable: true,
        negative_policy: NegativePolicy::Allow,
        status: InstrumentStatus::Active,
        display: None,
        revision: 1,
    };
    disk.insert(
        ("economy".into(), "instrument:cr".into()),
        json!(instrument),
    );
    for (id, owner) in [
        (0, EconomicOwner::Object(GOD)),
        (1, EconomicOwner::System(issuer()?)),
        (2, EconomicOwner::Object(BUYER)),
    ] {
        let account = Wallet {
            id: WalletId(id),
            owner,
            instrument: instrument.id.clone(),
            scope: None,
            name: format!("purchase fixture {id}").into(),
            status: AccountStatus::Active,
            posted_balance: Amount::ZERO,
            held_amount: Amount::ZERO,
            credit: if id == 1 {
                CreditPolicy::forbid()
            } else {
                CreditPolicy {
                    allow_negative: true,
                    floor: Some(Amount::from_minor(-1000)),
                }
            },
            last_journal_seq: 0,
            revision: 1,
            approval: None,
            last_hold_seq: 0,
        };
        disk.insert(("economy".into(), format!("account:{id}")), json!(account));
    }
    Ok(())
}

struct PurchaseFixture {
    source_forms: bool,
    resources: Resources,
    accounts: Arc<Accounts>,
    // Reboot branches retain the owner on the heap, not repeated Engine stack images.
    engine: Box<Engine>,
    disk: Disk,
    config: Box<PurchaseConfiguration>,
    filing: DefinitionRef,
    factory: Arc<Factory>,
}
impl PurchaseFixture {
    fn boot(resources: Resources, accounts: Arc<Accounts>, saved: &Durable) -> TestResult<Self> {
        Self::boot_protocol(resources, accounts, saved, false)
    }
    fn boot_protocol(
        resources: Resources,
        accounts: Arc<Accounts>,
        saved: &Durable,
        source_forms: bool,
    ) -> TestResult<Self> {
        Self::boot_options(resources, accounts, saved, source_forms, false)
    }
    // Keep actual Engine construction out of every caller's retained fixture frame.
    #[inline(never)]
    fn boot_options(
        resources: Resources,
        accounts: Arc<Accounts>,
        saved: &Durable,
        source_forms: bool,
        compatible_updates: bool,
    ) -> TestResult<Self> {
        let shared: Shared = Arc::new(Mutex::new(Registered::default()));
        let purchase: PurchaseShared = Arc::new(Mutex::new(None));
        let mut engine = Box::new(Engine::with_modules(
            saved.world()?,
            Registry::new(),
            vec![
                Box::new(paid_holdings::TacsimReader(
                    Tacsim {
                        resources: resources.clone(),
                        active: true,
                        shared: shared.clone(),
                        accounts: accounts.clone(),
                        real: None,
                    },
                    source_forms,
                )),
                Box::new(PurchaseRoll {
                    paid_resales: false,
                    minds_facts: None,
                    source_forms,
                    authored_forks: false,
                    marked_registry: false,
                    dependent_owners: false,
                    entitlements: false,
                    renewals: false,
                    renewal_service: false,
                    owner_qualification: false,
                    owner_attestations: false,
                    registry_recall: None,
                    registry_revocation: None,
                    qualification_short_parent: false,
                    author_withdrawal: false,
                    compatible_updates,
                    derived_updates: false,
                    shared: shared.clone(),
                    purchase: purchase.clone(),
                    inner: None,
                }),
                Box::new(PurchaseEconomy {
                    shared: shared.clone(),
                    purchase: purchase.clone(),
                    inner: None,
                }),
                Box::new(PurchaseMarket {
                    purchase: purchase.clone(),
                    inner: None,
                }),
            ],
        )?);
        engine.set_account_identity(accounts.clone());
        let mut disk = saved.disk.clone();
        load(&mut engine, &disk)?;
        module_ops(&mut disk, &engine.take_module_ops())?;
        let filing = shared
            .lock()
            .map_err(module_error)?
            .filing
            .clone()
            .ok_or("filing registration")?;
        let factory = shared
            .lock()
            .map_err(module_error)?
            .factory
            .clone()
            .ok_or("actual Registry factory")?;
        let config = purchase
            .lock()
            .map_err(module_error)?
            .clone()
            .ok_or("purchase configuration")?;
        Ok(Self {
            source_forms,
            resources,
            accounts,
            engine,
            disk,
            config,
            filing,
            factory,
        })
    }
    fn fund_buyer_cash(&mut self) -> TestResult {
        // E4 escrow must be backed by cash: a credit line cannot fund a hold.
        // Use the real operator commands and retain their actual journal writes.
        let conn = 95;
        self.engine.step(
            mush_core::Inbound::ConnUp {
                conn,
                actor: GOD,
                caps_structured: false,
                from_linkdead: false,
            },
            &mut Outbox::new(),
            &mut mush_core::Effects::new(),
        );
        self.setup_command(conn, "@account/system purchase.fixture=cr")?;
        let owner = EconomicOwner::System(
            SystemPrincipalId::new("purchase.fixture").ok_or("funding system owner")?,
        );
        let faucet = self
            .economy()?
            .snapshot()
            .accounts()
            .find(|account| account.owner == owner)
            .ok_or("actual purchase funding account")?
            .id;
        self.setup_command(conn, &format!("@econ/mint {faucet}=2 1000"))?;
        check_eq!(
            self.economy()?
                .snapshot()
                .account(faucet)
                .ok_or("faucet")?
                .posted_balance,
            Amount::from_minor(-1000)
        );
        check_eq!(
            self.balances()?[2],
            (Amount::from_minor(1000), Amount::ZERO)
        );
        // Cold boot must replay/audit the actual balanced grant before filing.
        let saved = self.snapshot();
        check_eq!(self.reboot(&saved)?.balances()?, self.balances()?);
        Ok(())
    }
    fn setup_command(&mut self, conn: u64, line: &str) -> TestResult {
        let mut out = Outbox::new();
        self.engine.step(
            mush_core::Inbound::Cmd {
                conn,
                actor: GOD,
                line: line.into(),
            },
            &mut out,
            &mut mush_core::Effects::new(),
        );
        if out.events().iter().any(|(_, event)| {
            matches!(event, mush_proto::OutputEvent::Sys(text)
                if text.spans().iter().any(|span| span.tag.as_deref() == Some("error")))
        }) {
            return Err("actual purchase funding setup refused".into());
        }
        module_ops(&mut self.disk, &self.engine.take_module_ops())?;
        Ok(())
    }
    fn transfer_charter(&mut self, charter: ObjId, buyer: ObjId, n: u128) -> TestResult {
        let disk = &mut self.disk;
        charter_transfer_fixture::sale(&mut self.engine, charter, buyer, n, |writes| {
            retain_writes(disk, writes)
        })
    }
    fn snapshot(&self) -> Durable {
        Durable::capture(&self.engine, &self.disk)
    }
    fn reboot(&self, saved: &Durable) -> TestResult<Self> {
        Self::boot_protocol(
            self.resources.clone(),
            self.accounts.clone(),
            saved,
            self.source_forms,
        )
    }
    fn economy(&self) -> TestResult<&EconomyStore> {
        self.engine
            .world
            .module_state::<EconomyStore>()
            .ok_or_else(|| "Economy owner".into())
    }
    fn balances(&self) -> TestResult<Vec<(Amount, Amount)>> {
        [0, 1, 2]
            .into_iter()
            .map(|id| {
                let a = self
                    .economy()?
                    .snapshot()
                    .account(WalletId(id))
                    .ok_or("actual wallet")?;
                Ok((a.posted_balance, a.held_amount))
            })
            .collect()
    }
    fn roots(&self) -> TestResult<BTreeMap<String, Value>> {
        let store = self
            .engine
            .world
            .module_state::<RollStore>()
            .ok_or("Roll owner")?;
        let keys = store.reference_keys("roll", ReferenceKind::Commercial);
        let mut values = BTreeMap::new();
        for key in keys {
            let disk_key = format!(
                "reference:{}:{}:{}",
                key.owner,
                key.kind.as_str(),
                key.record
            );
            let value = self
                .disk
                .get(&("roll".into(), disk_key))
                .ok_or("persisted root")?;
            check_eq!(value.get("key"), Some(&json!(key)));
            values.insert(key.record, value.clone());
        }
        Ok(values)
    }
    fn prepare(
        &mut self,
        action: &DefinitionRef,
        actor: ObjId,
        number: u128,
        params: Value,
    ) -> TestResult<PreparedComposedTxn> {
        Ok(self
            .engine
            .prepare_registered_composed_authoritative_action(
                action,
                &request(number, actor, action.key().owner.as_str(), params)?,
                token(number),
            )?
            .0)
    }
    fn install(&mut self, prepared: PreparedComposedTxn) -> TestResult<Vec<ReplayWrite>> {
        let (writes, installation) = prepared.into_parts();
        // Durability is represented by the exact returned union. The retained
        // copy also permits a crash-before-RAM branch below.
        retain_writes(&mut self.disk, &writes)?;
        self.engine.install_composed_action(installation)?;
        Ok(writes)
    }
    fn perform(
        &mut self,
        action: &DefinitionRef,
        actor: ObjId,
        number: u128,
        params: Value,
    ) -> TestResult<Vec<ReplayWrite>> {
        let prepared = self.prepare(action, actor, number, params)?;
        self.install(prepared)
    }
    fn rows(&self, owner: &str, family: &str) -> Vec<Value> {
        rows(&self.disk, owner, family)
    }
}
fn rows(disk: &Disk, owner: &str, family: &str) -> Vec<Value> {
    disk.iter()
        .filter(|((ns, key), v)| {
            ns == owner
                && mush_core::action_state::is_action_state_persistence_key(key)
                && v.get("family")
                    .and_then(|f| f.get("id"))
                    .and_then(Value::as_str)
                    == Some(family)
        })
        .filter_map(|(_, v)| v.get("value").cloned())
        .collect()
}
#[track_caller]
fn one(values: Vec<Value>) -> TestResult<Value> {
    let at = std::panic::Location::caller();
    let [value] = values.as_slice() else {
        return Err(format!("expected one row, got {} (at {at})", values.len()).into());
    };
    Ok(value.clone())
}
fn retain_writes(disk: &mut Disk, writes: &[ReplayWrite]) -> TestResult {
    for write in writes {
        match write {
            ReplayWrite::ModuleJsonSet {
                namespace,
                key,
                value,
            } => {
                disk.insert((namespace.to_string(), key.to_string()), value.clone());
            }
            ReplayWrite::ModuleDelete { namespace, key } => {
                disk.remove(&(namespace.to_string(), key.to_string()));
            }
            ReplayWrite::LedgerChunkSet { write } => {
                disk.insert(
                    (write.namespace().to_string(), write.key().to_string()),
                    serde_json::to_value(write.chunk())?,
                );
            }
            ReplayWrite::ObjectSet { .. } | ReplayWrite::ObjectAllocatorSet { .. } => {}
            ReplayWrite::ObjectDelete { .. } => {
                return Err("purchase never deletes an object".into());
            }
        }
    }
    Ok(())
}

fn remove_family(saved: &mut Durable, owner: &str, family: &str) -> usize {
    let before = saved.disk.len();
    saved.disk.retain(|(ns, key), v| {
        !(ns == owner
            && mush_core::action_state::is_action_state_persistence_key(key)
            && v.get("family")
                .and_then(|f| f.get("id"))
                .and_then(Value::as_str)
                == Some(family))
    });
    before - saved.disk.len()
}
fn change_one_family(
    saved: &mut Durable,
    owner: &str,
    family: &str,
    edit: impl FnOnce(&mut Value) -> TestResult,
) -> TestResult {
    let mut rows = saved.disk.iter_mut().filter(|((ns, key), v)| {
        ns == owner
            && mush_core::action_state::is_action_state_persistence_key(key)
            && v.get("family")
                .and_then(|f| f.get("id"))
                .and_then(Value::as_str)
                == Some(family)
    });
    let (_, value) = rows.next().ok_or("corruption target")?;
    check!(rows.next().is_none());
    edit(value.get_mut("value").ok_or("typed value")?)
}

#[test]
#[ignore = "required serialized M7 gate: completes the unmodified real Registry schedule before purchase/recovery branches"]
fn genuine_registry_original_purchase_grant_refund_and_paired_recovery() -> TestResult {
    let mut registry = Fixture::new()?;
    let rating = complete_registry(&mut registry)?;
    eprintln!("M7_STORY phase=house event=begin");
    house_filing::exercise(&registry, &rating)?;
    eprintln!("M7_STORY phase=house event=complete");
    let mut world = World::new_minimal();
    check_eq!(
        world.create(ObjKind::Player, "Actual buyer", GOD, Some(ROOM))?,
        BUYER
    );
    world.set_now(1000);
    {
        let mut accounts = registry.accounts.values.lock().map_err(module_error)?;
        registry.accounts.epoch.invalidate();
        accounts.insert(BUYER, AccountId::mint("actual-purchase-buyer", 2)?);
    }
    seed_wallets(&mut registry.disk)?;
    let initial = Durable {
        disk: registry.disk.clone(),
        objects: world.iter().cloned().collect(),
        next_id: world.next_id(),
        now: world.now(),
    };
    let mut f = PurchaseFixture::boot(
        registry.resources.clone(),
        registry.accounts.clone(),
        &initial,
    )?;
    eprintln!("M7_STORY phase=cash event=begin");
    f.fund_buyer_cash()?;
    eprintln!("M7_STORY phase=cash event=complete");
    let reserve = reference(
        "roll",
        "action",
        mush_core::reserved_shell::RESERVE_ACTION,
        json!({"format":1,"purpose":"roll.initial_filing","object":"inert"}),
    )?;
    let shell = ObjId(f.engine.world.next_id());
    f.perform(&reserve, GOD, 1_000_000, json!({}))?;
    check_eq!(
        f.engine
            .world
            .get(shell)
            .ok_or("acknowledged inert shell")?
            .owner,
        GOD
    );
    let revision = registry.revision.reference();
    let filing = f.filing.clone();
    eprintln!("M7_STORY phase=personal_filing event=begin");
    let writes = f.perform(&filing, GOD, 1_000_001, json!({
        "shell_request":RequestUid::from_bytes(1_000_000_u128.to_be_bytes()),
        "mind":revision.key.mind,"revision":revision.key.number,"revision_digest":revision.digest,
        "rating_job":rating.job(),"rating_id":rating.id(),"rating_digest":rating.receipt_digest(),
        "payer":0,"charter_name":"Measured original",
    }))?;
    check!(
        writes
            .iter()
            .any(|w| matches!(w, ReplayWrite::LedgerChunkSet { .. }))
    );
    check_eq!(
        f.economy()?
            .snapshot()
            .asset(shell)
            .ok_or("real E4 title")?
            .title_owner,
        EconomicOwner::Object(GOD)
    );
    check_eq!(
        f.balances()?,
        vec![
            (Amount::from_minor(-100), Amount::ZERO),
            (Amount::from_minor(100), Amount::ZERO),
            (Amount::from_minor(1000), Amount::ZERO)
        ]
    );
    eprintln!("M7_STORY phase=personal_filing event=complete");
    eprintln!("M7_STORY phase=source_forms event=begin");
    source_forms::exercise(&f, &f.snapshot())?;
    eprintln!("M7_STORY phase=source_forms event=complete");
    let listing_action = f.config.quotes.listing_action()?;
    f.perform(
        &listing_action,
        GOD,
        1_000_002,
        json!({"owner_licence":"1"}),
    )?;
    let quote_action = f.config.quotes.quote_action()?;
    f.perform(
        &quote_action,
        BUYER,
        1_000_003,
        json!({"listing":token(1_000_002)}),
    )?;
    let quoted = f.snapshot();
    let [fund, grant, refund] = f.config.purchases.actions()?;

    // A stale venue refuses before custody, with the quote, allocator, balances
    // and roots unchanged. It does not create a partial funded operation.
    let mut stale = f.reboot(&quoted)?;
    let outside = stale
        .engine
        .world
        .create(ObjKind::Room, "Outside venue", GOD, None)?;
    stale.engine.world.move_to(BUYER, outside)?;
    let before_money = stale.balances()?;
    let before_roots = stale.roots()?;
    let before_revision = stale.engine.action_state().revision();
    check!(
        stale
            .prepare(&fund, BUYER, 1_000_004, json!({"quote":token(1_000_003)}))
            .is_err()
    );
    check_eq!(stale.engine.action_state().revision(), before_revision);
    check_eq!(stale.balances()?, before_money);
    check_eq!(stale.roots()?, before_roots);
    check_eq!(stale.disk, quoted.disk);
    check!(stale.rows("market", "market_mind_purchase").is_empty());

    let before_money = f.balances()?;
    let before_roots = f.roots()?;
    let before_revision = f.engine.action_state().revision();
    let operation = token(1_000_004);
    eprintln!("M7_STORY phase=v1_fund event=begin");
    let prepared = f.prepare(&fund, BUYER, 1_000_004, json!({"quote":token(1_000_003)}))?;
    check_eq!(f.balances()?, before_money);
    check_eq!(f.roots()?, before_roots);
    check_eq!(f.engine.action_state().revision(), before_revision);
    let funding_writes = f.install(prepared)?;
    check!(
        funding_writes
            .iter()
            .any(|w| matches!(w, ReplayWrite::LedgerChunkSet { .. }))
    );
    check_eq!(
        f.balances()?,
        vec![
            (Amount::from_minor(-100), Amount::ZERO),
            (Amount::from_minor(100), Amount::ZERO),
            (Amount::from_minor(1000), Amount::from_minor(100))
        ]
    );
    check!(f.rows("market", "market_mind_original_quote").is_empty());
    check!(f.rows("roll", "roll_mind_personal_paid_licence").is_empty());
    check!(f.rows("roll", "roll_mind_paid_term").is_empty());
    check!(f.rows("market", "market_mind_purchase_receipt").is_empty());
    let purchase = one(f.rows("market", "market_mind_purchase"))?;
    let allocation = one(f.rows("roll", "roll_mind_paid_purchase"))?;
    check_eq!(purchase.get("operation"), Some(&json!(operation)));
    check_eq!(
        purchase.get("licence"),
        Some(&json!("2")),
        "shared allocator follows owner licence 1"
    );
    for field in [
        "operation",
        "participants",
        "licence",
        "term",
        "funding",
        "phase",
        "updated_by",
    ] {
        check_eq!(purchase.get(field), allocation.get(field), "paired {field}");
    }
    let roots = f.roots()?;
    check_eq!(roots.len(), before_roots.len() + 2);
    check!(roots.contains_key(&format!("v1/paid_operation/{operation}")));
    check!(roots.contains_key("v1/licence/0000000000000002/term/0000000000000001"));
    check_eq!(
        f.disk
            .keys()
            .filter(|(_, k)| k.starts_with("action-state/capacity/v1/"))
            .count(),
        1
    );
    eprintln!("M7_STORY phase=v1_fund event=complete");
    let funded = f.snapshot();
    eprintln!("M7_STORY phase=driver event=begin");
    driver::exercise(&f, &funded, operation)?;
    eprintln!("M7_STORY phase=driver event=complete");
    let mut funding_crash = quoted.clone();
    retain_writes(&mut funding_crash.disk, &funding_writes)?;
    check_eq!(funding_crash.disk, funded.disk);
    let funded_reboot = f.reboot(&funded)?;
    check_eq!(funded_reboot.balances()?, f.balances()?);
    check_eq!(funded_reboot.roots()?, roots);
    check!(
        funded_reboot
            .economy()?
            .purchase_receipt_audit()
            .get(operation)?
            .terminal()
            .is_none()
    );
    let funding_recovered = f.reboot(&funding_crash)?;
    check_eq!(funding_recovered.balances()?, f.balances()?);
    check_eq!(funding_recovered.roots()?, roots);
    let funded_money = f.balances()?;
    check!(
        f.prepare(&fund, BUYER, 1_000_011, json!({"quote":token(1_000_003)}))
            .is_err()
    );
    check!(
        f.prepare(&grant, GOD, 1_000_012, json!({"operation":operation}))
            .is_err(),
        "staff cannot grant another buyer's funded operation"
    );
    check_eq!(f.balances()?, funded_money);
    check_eq!(f.disk, funded.disk);

    // The receipt may be durable before its RAM images are installed. Replay
    // reconstructs exactly that durable union without a live prepare callback.
    let mut granted = f.reboot(&funded)?;
    granted.engine.world.set_now(1017);
    eprintln!("M7_STORY phase=v1_grant event=begin");
    let prepared = granted.prepare(&grant, BUYER, 1_000_005, json!({"operation":operation}))?;
    check_eq!(granted.balances()?, f.balances()?);
    check!(
        granted
            .rows("roll", "roll_mind_personal_paid_licence")
            .is_empty()
    );
    let terminal_writes = granted.install(prepared)?;
    check_eq!(
        granted.balances()?,
        vec![
            (Amount::from_minor(-10), Amount::ZERO),
            (Amount::from_minor(110), Amount::ZERO),
            (Amount::from_minor(900), Amount::ZERO)
        ]
    );
    check_eq!(
        granted.roots()?,
        roots,
        "terminal republish retains the exact funded source roots"
    );
    check!(
        granted
            .disk
            .keys()
            .all(|(_, k)| !k.starts_with("action-state/capacity/v1/"))
    );
    let licence: PersonalPaidLicence =
        serde_json::from_value(one(granted.rows("roll", "roll_mind_personal_paid_licence"))?)?;
    let term: PaidTermRecord =
        serde_json::from_value(one(granted.rows("roll", "roll_mind_paid_term"))?)?;
    term.matches_current(&licence)?;
    check_eq!(licence.id().get(), 2);
    check_eq!(licence.holder(), BUYER);
    check_eq!(licence.charter().object(), shell);
    check_eq!(licence.operation(), operation);
    check_eq!(term.operation(), operation);
    check_eq!(term.window().issued_at().get(), 1017);
    check_eq!(term.window().eligibility_start().get(), 1017);
    check_eq!(term.window().term_end().get(), 1017 + TERM_SECONDS);
    check_eq!(
        term.window().grace_end().get(),
        1017 + TERM_SECONDS + GRACE_SECONDS
    );
    let receipt = one(granted.rows("market", "market_mind_purchase_receipt"))?;
    check_eq!(receipt.get("outcome"), Some(&json!({"kind":"granted"})));
    check_eq!(receipt.get("commit"), Some(&json!(token(1_000_005))));
    let history = granted.economy()?.purchase_receipt_audit().get(operation)?;
    let terminal = history.terminal().ok_or("real paid terminal history")?;
    check!(terminal.granted());
    check_eq!(
        receipt.get("transaction"),
        Some(&json!(terminal.transaction()))
    );
    check_eq!(
        receipt.get("journal_seq"),
        Some(&json!(terminal.journal_seq()))
    );
    check_eq!(
        receipt.get("issued_at_ic"),
        Some(&json!(terminal.issued_at_ic()))
    );
    eprintln!("M7_STORY phase=v1_grant event=complete");
    let committed = granted.snapshot();
    ic_tests::exercise(&granted, &committed)?;
    charter_transfers::exercise(&granted, &committed)?;
    charter_transfer_fences::exercise(&granted, &committed)?;
    let mut lost = f.reboot(&funded)?;
    lost.engine.world.set_now(1017);
    let prepared = lost.prepare(&grant, BUYER, 1_000_005, json!({"operation":operation}))?;
    let (durable_writes, uninstalled) = prepared.into_parts();
    check_eq!(durable_writes, terminal_writes);
    let mut crash_before_install = funded.clone();
    crash_before_install.now = 1017;
    retain_writes(&mut crash_before_install.disk, &durable_writes)?;
    check_eq!(lost.balances()?, f.balances()?);
    check!(
        lost.rows("roll", "roll_mind_personal_paid_licence")
            .is_empty()
    );
    drop(uninstalled); // The acknowledged images never reached these RAM owners.
    check_eq!(crash_before_install.disk, committed.disk);
    let recovered = granted.reboot(&crash_before_install)?;
    check_eq!(recovered.balances()?, granted.balances()?);
    check_eq!(recovered.roots()?, roots);
    check_eq!(
        recovered.rows("roll", "roll_mind_paid_term"),
        granted.rows("roll", "roll_mind_paid_term")
    );
    let money = granted.balances()?;
    let disk = granted.disk.clone();
    check!(
        granted
            .prepare(&grant, BUYER, 1_000_006, json!({"operation":operation}))
            .is_err()
    );
    check!(
        granted
            .prepare(&refund, GOD, 1_000_007, json!({"operation":operation}))
            .is_err()
    );
    check_eq!(granted.balances()?, money);
    check_eq!(granted.disk, disk);

    // An existing funded escrow remains refundable after both the quote and
    // seller's owner term expire and after the buyer leaves the venue. It
    // creates no paid grant; permanent allocator/history/source roots remain.
    let mut refunded = f.reboot(&funded)?;
    refunded
        .engine
        .world
        .set_now(1000 + TERM_SECONDS + GRACE_SECONDS + 1);
    let outside = refunded
        .engine
        .world
        .create(ObjKind::Room, "Refund away", GOD, None)?;
    refunded.engine.world.move_to(BUYER, outside)?;
    check!(
        refunded
            .prepare(&grant, BUYER, 1_000_008, json!({"operation":operation}))
            .is_err()
    );
    refunded.perform(&refund, GOD, 1_000_009, json!({"operation":operation}))?;
    check_eq!(refunded.balances()?, before_money);
    check_eq!(refunded.roots()?, roots);
    check!(
        refunded
            .rows("roll", "roll_mind_personal_paid_licence")
            .is_empty()
    );
    check!(refunded.rows("roll", "roll_mind_paid_term").is_empty());
    check!(
        refunded
            .disk
            .keys()
            .all(|(_, k)| !k.starts_with("action-state/capacity/v1/"))
    );
    let refund_receipt = one(refunded.rows("market", "market_mind_purchase_receipt"))?;
    check_eq!(
        refund_receipt.get("outcome"),
        Some(&json!({"kind":"refunded"}))
    );
    check_eq!(refund_receipt.get("issued_at_ic"), Some(&Value::Null));
    paid_holdings::exercise(&granted, &funded, &committed, &refunded.snapshot())?;
    eprintln!("M7_STORY phase=paid_checkout event=begin");
    paid_checkout::exercise(&granted, &committed)?;
    eprintln!("M7_STORY phase=paid_checkout event=complete");
    source_forms::historical_v1(&granted, &funded, &committed)?;
    eprintln!("M7_STORY phase=compatible_update event=begin");
    paid_checkout::exercise_updates(&granted, &committed)?;
    paid_checkout::exercise_reconformance(&granted, &committed)?;
    eprintln!("M7_STORY phase=compatible_update event=complete");
    let refund_boot = refunded.reboot(&refunded.snapshot())?;
    check_eq!(refund_boot.balances()?, before_money);
    check_eq!(refund_boot.roots()?, roots);
    check!(
        refunded
            .prepare(&refund, GOD, 1_000_010, json!({"operation":operation}))
            .is_err()
    );

    // Reverse joins must catch all-missing owner rows, not only mismatched
    // rows reached from a surviving forward pointer. Every test starts from a
    // known healthy terminal union with no remaining core reservation.
    for (owner, family) in [
        ("market", "market_mind_purchase"),
        ("market", "market_mind_purchase_receipt"),
        ("roll", "roll_mind_paid_purchase"),
        ("roll", "roll_mind_personal_paid_licence"),
        ("roll", "roll_mind_paid_term"),
    ] {
        let mut corrupt = committed.clone();
        check_eq!(remove_family(&mut corrupt, owner, family), 1);
        check!(
            granted.reboot(&corrupt).is_err(),
            "missing {owner}:{family} was admitted"
        );
    }
    let mut roots_missing = committed.clone();
    let before = roots_missing.disk.len();
    roots_missing.disk.retain(|(owner, key), _| {
        !(owner == "roll" && key.starts_with("reference:roll:commercial:"))
    });
    check_eq!(before - roots_missing.disk.len(), roots.len());
    check!(
        granted.reboot(&roots_missing).is_err(),
        "all commercial roots absent was admitted"
    );
    let mut term_changed = committed.clone();
    change_one_family(&mut term_changed, "roll", "roll_mind_paid_term", |row| {
        let window = row
            .get_mut("window")
            .and_then(Value::as_object_mut)
            .ok_or("term window")?;
        *window.get_mut("term_end").ok_or("term end")? = json!(1017 + TERM_SECONDS - 1);
        *window.get_mut("grace_end").ok_or("grace end")? =
            json!(1017 + TERM_SECONDS + GRACE_SECONDS - 1);
        Ok(())
    })?;
    let changed_term: PaidTermRecord = serde_json::from_value(one(rows(
        &term_changed.disk,
        "roll",
        "roll_mind_paid_term",
    ))?)?;
    check_eq!(
        changed_term.window().term_end().get(),
        term.window().term_end().get() - 1
    );
    check!(
        granted.reboot(&term_changed).is_err(),
        "locally valid shortened paid term escaped frozen duration join"
    );
    eprintln!("M7_STORY phase=v1_story event=complete");
    eprintln!(
        "M7_ACTUAL_PURCHASE_PASSED registry={rating:?} funded={operation} grant={} refund={}",
        token(1_000_005),
        token(1_000_009)
    );
    Ok(())
}
