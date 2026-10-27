use super::*;
use mush_core::{Engine, GameModule, Registry, World};
use mush_economy::{AccountId, Amount, CatalogRef, InstrumentId};
use mush_market::{
    MarketId,
    mind_trade::{OriginalDuration, OriginalPrice, records::QuoteVenue},
};
use mush_org::{PrincipalId, SystemPrincipalId};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn issuer() -> Result<SystemPrincipalId, ModError> {
    SystemPrincipalId::new("roll.registry").ok_or_else(|| ModError::msg("valid issuer"))
}
fn layer() -> TacsimRuleset {
    TacsimRuleset::with_config(crate::TacsimConfig {
        enabled: true,
        ..crate::TacsimConfig::default()
    })
}
fn prepare(mode: CommerceMode) -> Result<MindCommerce, ModError> {
    MindCommerce::prepare(
        &layer(),
        mode,
        InitialFilingTerms::new(90 * 86_400, 3 * 86_400).map_err(error)?,
        InitialFilingPolicy::new(
            issuer()?,
            AccountId(1),
            Amount::from_minor(100),
            CatalogRef {
                ruleset: "roll".into(),
                code: "mind_charter".into(),
            },
        )
        .map_err(error)?,
    )
}
fn quotes() -> Result<OriginalQuotePolicy, ModError> {
    OriginalQuotePolicy::new(
        OriginalPrice::new(
            Amount::from_minor(100),
            InstrumentId::new("cr").ok_or_else(|| ModError::msg("currency"))?,
            issuer()?,
            AccountId(1),
            100,
        )?,
        QuoteVenue::new(
            MarketId::new("registry").ok_or_else(|| ModError::msg("venue"))?,
            PrincipalId::System(issuer()?),
            AccountId(1),
            0,
        )
        .map_err(error)?,
        OriginalDuration::new(true, 90 * 86_400, 3 * 86_400, 600)?,
    )
}
// Keep the real constructor's owned result out of the calling test frame.
#[inline(never)]
fn compose_boxed(
    world: World,
    registry: Registry,
    modules: Vec<Box<dyn GameModule>>,
) -> Result<Box<Engine>, ModError> {
    Engine::with_modules(world, registry, modules).map(Box::new)
}

fn engine(config: &MindCommerce) -> Result<Box<Engine>, ModError> {
    // The real E4 owner remains earlier than Roll and Tacsim. Their exact
    // references are validated by the final registry, not constructor order.
    let modules: Vec<Box<dyn GameModule>> = vec![
        Box::new(
            config.configure_economy(EconomyModule::new().with_title_issuers(vec![issuer()?]))?,
        ),
        Box::new(config.configure_roll(RollModule::with_writers([crate::NS]))),
        Box::new(config.configure_tacsim(layer())?),
    ];
    compose_boxed(World::new_minimal(), Registry::new(), modules)
}
fn has(engine: &Engine, owner: &str, kind: &str, id: &str) -> bool {
    engine.registry.definitions().is_some_and(|definitions| {
        definitions.iter().any(|entry| {
            let reference = entry.reference();
            reference.key().owner.as_str() == owner
                && reference.key().kind.as_str() == kind
                && reference.key().id.as_str() == id
        })
    })
}

#[test]
fn actual_registration_is_opt_in_and_initialize_has_no_rating_actions() -> TestResult {
    let ordinary = compose_boxed(
        World::new_minimal(),
        Registry::new(),
        vec![
            Box::new(EconomyModule::new()),
            Box::new(RollModule::with_writers([crate::NS])),
            Box::new(layer()),
        ],
    )?;
    check!(!has(&ordinary, "tacsim", "registry_manifest", "combat"));
    check!(!has(&ordinary, "roll", "action", "rating_admit"));
    check!(!has(&ordinary, "roll", "action", "initial_filing"));

    let initialize = prepare(CommerceMode::Initialize)?;
    let initialized = engine(&initialize)?;
    check!(has(&initialized, "tacsim", "registry_manifest", "combat"));
    check!(has(&initialized, "roll", "action", "initialize_commerce"));
    check!(!has(&initialized, "roll", "action", "rating_admit"));
    check!(!has(
        &initialized,
        "tacsim",
        "owner_participant",
        "registry_quota"
    ));

    let active = prepare(CommerceMode::Active)?;
    check_eq!(initialize.manifest(), active.manifest());
    let registered = engine(&active)?;
    check!(!has(&registered, "roll", "action", "initialize_commerce"));
    for id in [
        "rating_admit",
        "rating_grant",
        "rating_settle",
        "rating_cancel",
        "rating_recover",
    ] {
        check!(has(&registered, "roll", "action", id));
    }
    check!(has(
        &registered,
        "tacsim",
        "owner_participant",
        "registry_quota"
    ));
    check!(has(
        &registered,
        "tacsim",
        "owner_participant",
        "registry_artifacts"
    ));
    let actual = registered
        .registry
        .definitions()
        .ok_or("actual definitions")?
        .iter()
        .map(|entry| entry.reference())
        .find(|reference| *reference == active.manifest())
        .ok_or("actual manifest")?;
    check_eq!(actual, active.manifest());
    // This is registration evidence only: Active hydration without the real
    // acknowledged deployment must refuse, not manufacture commercial health.
    let mut uninitialized = registered;
    check!(
        uninitialized
            .modules_loaded(
                Default::default(),
                &mut mush_core::Outbox::new(),
                &mut mush_core::engine::Effects::new()
            )
            .is_err()
    );
    Ok(())
}

#[test]
fn different_configuration_missing_owner_and_double_attachment_refuse() -> TestResult {
    let prepared = prepare(CommerceMode::Active)?;
    let mut changed = layer().config().clone();
    changed.match_.ai_fill = !changed.match_.ai_fill;
    check!(
        prepared
            .configure_tacsim(TacsimRuleset::with_config(changed))
            .is_err()
    );
    let configured = prepared.configure_tacsim(layer())?;
    check!(prepared.configure_tacsim(configured).is_err());
    let mut changed_after_attachment = prepared.configure_tacsim(layer())?;
    changed_after_attachment.cfg.match_.ai_fill = !changed_after_attachment.cfg.match_.ai_fill;
    check!(
        compose_boxed(
            World::new_minimal(),
            Registry::new(),
            vec![
                Box::new(
                    prepared.configure_economy(
                        EconomyModule::new().with_title_issuers(vec![issuer()?]),
                    )?
                ),
                Box::new(prepared.configure_roll(RollModule::with_writers([crate::NS]))),
                Box::new(changed_after_attachment),
            ],
        )
        .is_err(),
        "actual Tacsim registration repeats the full configuration fence"
    );
    check!(
        compose_boxed(
            World::new_minimal(),
            Registry::new(),
            vec![
                Box::new(
                    prepared.configure_economy(
                        EconomyModule::new().with_title_issuers(vec![issuer()?])
                    )?
                ),
                Box::new(prepared.configure_roll(RollModule::with_writers([crate::NS]))),
                Box::new(layer()),
            ]
        )
        .is_err(),
        "ordinary Tacsim cannot satisfy configured Roll's actual manifest/participant dependencies"
    );
    Ok(())
}

#[test]
fn purchase_configuration_is_active_only_and_keeps_exact_shared_actions() -> TestResult {
    check!(
        prepare(CommerceMode::Initialize)?
            .with_original_purchases(quotes()?)
            .is_err()
    );
    let prepared = prepare(CommerceMode::Active)?.with_original_purchases(quotes()?)?;
    let actual = prepared
        .state
        .purchases
        .as_ref()
        .ok_or("prepared purchases")?
        .actions()?;
    check_eq!(prepared.state.purchase_actions.as_ref(), Some(&actual));
    let configured = prepared.configure_tacsim(layer())?;
    check!(configured.personal_filed_holdings);
    check_eq!(
        configured.original_holdings.as_ref(),
        Some(prepared.manifest())
    );
    // Real owner registrations and the real port presence gate; no venue,
    // account, licence or certificate is invented to prove composition.
    let actual_engine = compose_boxed(
        World::new_minimal(),
        Registry::new(),
        vec![
            Box::new(
                prepared
                    .configure_economy(EconomyModule::new().with_title_issuers(vec![issuer()?]))?,
            ),
            Box::new(prepared.configure_roll(RollModule::with_writers([crate::NS]))),
            Box::new(
                prepared.configure_market(
                    MarketModule::new()
                        .with_venue_gate(std::sync::Arc::new(crate::port::market::PortVenueGate)),
                ),
            ),
            Box::new(configured),
        ],
    )?;
    for action in &actual {
        check!(
            actual_engine
                .registry
                .definitions()
                .ok_or("definitions")?
                .iter()
                .any(|entry| entry.reference() == action)
        );
    }
    check!(prepared.with_original_purchases(quotes()?).is_err());
    Ok(())
}

#[test]
fn source_forms_are_an_explicit_host_protocol_with_a_distinct_paid_reader() -> TestResult {
    check!(
        prepare(CommerceMode::Initialize)?
            .with_original_source_purchases(quotes()?)
            .is_err()
    );
    let ordinary = prepare(CommerceMode::Active)?;
    check!(!ordinary.configure_tacsim(layer())?.personal_source_forms);
    for source_forms in [false, true] {
        let prepared = prepare(CommerceMode::Active)?;
        let prepared = if source_forms {
            prepared.with_original_source_purchases(quotes()?)?
        } else {
            prepared.with_original_purchases(quotes()?)?
        };
        let configured = prepared.configure_tacsim(layer())?;
        check_eq!(configured.personal_source_forms, source_forms);
        let actual =
            compose_boxed(
                World::new_minimal(),
                Registry::new(),
                vec![
                    Box::new(prepared.configure_economy(
                        EconomyModule::new().with_title_issuers(vec![issuer()?]),
                    )?),
                    Box::new(prepared.configure_roll(RollModule::with_writers([crate::NS]))),
                    Box::new(prepared.configure_market(
                        MarketModule::new().with_venue_gate(std::sync::Arc::new(
                            crate::port::market::PortVenueGate,
                        )),
                    )),
                    Box::new(configured),
                ],
            )?;
        check_eq!(
            has(&actual, "roll", "owner_read", "personal_paid_source"),
            source_forms
        );
        let definitions = actual.registry.definitions().ok_or("actual definitions")?;
        for id in [
            "purchase_original_mind",
            "settle_original_mind",
            "refund_original_mind",
        ] {
            let action = definitions
                .iter()
                .map(|entry| entry.reference())
                .find(|reference| {
                    reference.key().owner.as_str() == "market" && reference.key().id.as_str() == id
                })
                .ok_or("real action")?;
            check_eq!(action.version(), if source_forms { 2 } else { 1 });
        }
    }
    Ok(())
}

// Keep independent composition scenarios in separate stack frames. The
// real Engine still performs registration and final validation in each case.
#[inline(never)]
fn authored_fork_prerequisites_refuse() -> TestResult {
    check!(
        prepare(CommerceMode::Initialize)?
            .with_authored_forks()
            .is_err()
    );
    check!(
        prepare(CommerceMode::Active)?
            .with_authored_forks()
            .is_err()
    );
    check!(
        prepare(CommerceMode::Active)?
            .with_original_purchases(quotes()?)?
            .with_authored_forks()
            .is_err()
    );
    Ok(())
}

// Configuration errors still propagate separately from the registration
// result that the missing-reader case deliberately expects to refuse.
#[inline(never)]
fn authored_fork_engine(
    prepared: &MindCommerce,
    configured: TacsimRuleset,
) -> Result<Result<Box<Engine>, ModError>, ModError> {
    Ok(compose_boxed(
        World::new_minimal(),
        Registry::new(),
        vec![
            Box::new(
                prepared
                    .configure_economy(EconomyModule::new().with_title_issuers(vec![issuer()?]))?,
            ),
            Box::new(
                prepared.configure_market(
                    MarketModule::new()
                        .with_venue_gate(std::sync::Arc::new(crate::port::market::PortVenueGate)),
                ),
            ),
            Box::new(prepared.configure_roll(RollModule::with_writers([crate::NS]))),
            Box::new(configured),
        ],
    ))
}

#[inline(never)]
fn authored_fork_registration_matches_opt_in(forks: bool) -> TestResult {
    let prepared = prepare(CommerceMode::Active)?.with_original_source_purchases(quotes()?)?;
    let prepared = if forks {
        prepared.with_authored_forks()?
    } else {
        prepared
    };
    let configured = prepared.configure_tacsim(layer())?;
    check_eq!(
        configured.fork_material.as_ref(),
        forks.then(|| prepared.manifest())
    );
    let actual = authored_fork_engine(&prepared, configured)??;
    check_eq!(has(&actual, "roll", "action", "fork_paid_mind"), forks);
    check_eq!(
        has(&actual, "roll", "owner_read", "authored_draft_access"),
        forks
    );
    check_eq!(has(&actual, "roll", "outcome", "paid_mind_forked"), forks);
    // Real Engine freeze rejects duplicate declarations. The exact action
    // and outcome each enter once, then provider/grants reuse their refs.
    for (kind, id) in [
        ("action", "fork_paid_mind"),
        ("outcome", "paid_mind_forked"),
    ] {
        check_eq!(
            actual
                .registry
                .definitions()
                .ok_or("definitions")?
                .iter()
                .filter(|entry| {
                    let r = entry.reference();
                    r.key().owner.as_str() == "roll"
                        && r.key().kind.as_str() == kind
                        && r.key().id.as_str() == id
                })
                .count(),
            usize::from(forks)
        );
    }
    check_eq!(
        has(&actual, "roll", "owner_read", "authored_fork_parent"),
        forks
    );
    check_eq!(
        has(&actual, "tacsim", "owner_read", "authored_fork_material"),
        forks
    );
    if forks {
        check!(prepared.with_authored_forks().is_err());
    }
    Ok(())
}

#[inline(never)]
fn authored_fork_missing_source_reader_refuses() -> TestResult {
    let prepared = prepare(CommerceMode::Active)?
        .with_original_source_purchases(quotes()?)?
        .with_authored_forks()?;
    let mut detached = prepared.configure_tacsim(layer())?;
    detached.fork_material = None;
    check!(
        authored_fork_engine(&prepared, detached)?.is_err(),
        "a missing actual source service cannot be replaced by a structural descriptor"
    );
    Ok(())
}

#[test]
fn authored_forks_require_explicit_source_forms_and_register_exact_real_services() -> TestResult {
    authored_fork_prerequisites_refuse()?;
    for forks in [false, true] {
        authored_fork_registration_matches_opt_in(forks)?;
    }
    authored_fork_missing_source_reader_refuses()?;
    Ok(())
}

#[test]
fn marked_registry_is_explicit_saved_admission_with_unchanged_rating_protocol() -> TestResult {
    check!(
        prepare(CommerceMode::Initialize)?
            .with_marked_registry()
            .is_err()
    );
    check!(
        prepare(CommerceMode::Active)?
            .with_marked_registry()
            .is_err()
    );
    check!(
        prepare(CommerceMode::Active)?
            .with_original_source_purchases(quotes()?)?
            .with_marked_registry()
            .is_err()
    );
    let mut ordinary_refs = None;
    for marked in [false, true] {
        let prepared = prepare(CommerceMode::Active)?
            .with_original_source_purchases(quotes()?)?
            .with_authored_forks()?;
        let prepared = if marked {
            prepared.with_marked_registry()?
        } else {
            prepared
        };
        let configured = prepared.configure_tacsim(layer())?;
        check_eq!(
            configured.marked_registry.as_ref(),
            marked.then(|| prepared.manifest())
        );
        let actual =
            compose_boxed(
                World::new_minimal(),
                Registry::new(),
                vec![
                    Box::new(prepared.configure_economy(
                        EconomyModule::new().with_title_issuers(vec![issuer()?]),
                    )?),
                    Box::new(prepared.configure_market(
                        MarketModule::new().with_venue_gate(std::sync::Arc::new(
                            crate::port::market::PortVenueGate,
                        )),
                    )),
                    Box::new(prepared.configure_roll(RollModule::with_writers([crate::NS]))),
                    Box::new(configured),
                ],
            )?;
        check_eq!(
            has(&actual, "roll", "owner_read", "marked_rating_admission"),
            marked
        );
        let refs: Vec<_> = actual
            .registry
            .definitions()
            .ok_or("definitions")?
            .iter()
            .map(|entry| entry.reference())
            .filter(|r| {
                r.key().owner.as_str() == "roll"
                    && r.key().kind.as_str() == "action"
                    && r.key().id.as_str().starts_with("rating_")
            })
            .cloned()
            .collect();
        check_eq!(refs.len(), 5);
        if let Some(previous) = &ordinary_refs {
            check_eq!(
                &refs,
                previous,
                "existing protocol/history identities stay exact"
            );
        } else {
            ordinary_refs = Some(refs);
        }
        if marked {
            check!(prepared.with_marked_registry().is_err());
        }
    }
    Ok(())
}

#[path = "entitlement_tests.rs"]
mod entitlement_tests;

#[path = "ic_tests.rs"]
mod ic;

#[test]
fn registry_recall_configuration_is_explicit_stable_and_active_only() -> TestResult {
    use mush_roll::commerce::registry_recall::{REGISTRY_RECALL_CAPABILITY, RegistryRecallPolicy};
    let policy = RegistryRecallPolicy::new(
        issuer()?,
        mush_org::OrgId(4),
        mush_org::OrgScopeId(5),
        vec![mush_core::ObjId(6)],
    )?;
    check!(
        prepare(CommerceMode::Initialize)?
            .with_registry_recall(policy.clone())
            .is_err()
    );
    let ordinary = prepare(CommerceMode::Active)?;
    let plain = engine(&ordinary)?;
    check!(!has(&plain, "roll", "action", "registry_revision_recall"));
    let configured = ordinary.with_registry_recall(policy.clone())?;
    let actual = engine(&configured)?;
    let definitions = actual.registry.definitions().ok_or("real definitions")?;
    check!(definitions.get_exact(&policy.action()?).is_ok());
    check!(
        crate::composition::org_capability_vocabulary().contains(
            &mush_org::CapabilityId::new(REGISTRY_RECALL_CAPABILITY).ok_or("capability")?
        )
    );
    // Registration alone does not create an Org, scope, actor, grant or healthy
    // commerce world. The genuine child supplies those through real producers.
    check!(actual.world.module_state::<mush_org::OrgStore>().is_none());
    check!(configured.with_registry_recall(policy).is_err());
    Ok(())
}

#[test]
fn registry_revocation_configuration_is_explicit_stable_and_active_only() -> TestResult {
    use mush_roll::commerce::registry_revocation::{
        REGISTRY_REVOCATION_CAPABILITY, RegistryRevocationPolicy,
    };
    let policy = RegistryRevocationPolicy::new(
        issuer()?,
        mush_org::OrgId(4),
        mush_org::OrgScopeId(5),
        vec![mush_core::ObjId(6)],
    )?;
    check!(
        prepare(CommerceMode::Initialize)?
            .with_registry_revocation(policy.clone())
            .is_err()
    );
    let ordinary = prepare(CommerceMode::Active)?;
    let plain = engine(&ordinary)?;
    check!(!has(&plain, "roll", "action", "registry_term_revocation"));
    let configured = ordinary.with_registry_revocation(policy.clone())?;
    let actual = engine(&configured)?;
    let definitions = actual.registry.definitions().ok_or("real definitions")?;
    check!(definitions.get_exact(&policy.action()?).is_ok());
    check!(crate::composition::org_capability_vocabulary().contains(
        &mush_org::CapabilityId::new(REGISTRY_REVOCATION_CAPABILITY).ok_or("capability")?
    ));
    // Registration alone does not create an Org, scope, actor, grant or healthy
    // commerce world. The genuine child supplies those through real producers.
    check!(actual.world.module_state::<mush_org::OrgStore>().is_none());
    check!(configured.with_registry_revocation(policy).is_err());
    Ok(())
}
#[test]
fn minds_facts_are_explicit_active_and_register_exact_service_and_action() -> TestResult {
    check!(
        prepare(CommerceMode::Initialize)?
            .with_minds_facts()
            .is_err()
    );
    let ordinary = prepare(CommerceMode::Active)?;
    let engine_without = engine(&ordinary)?;
    check!(!has(
        &engine_without,
        "roll",
        "action",
        "record_mind_term_lapse"
    ));
    check!(!has(
        &engine_without,
        "roll",
        "owner_read",
        "minds_contract_facts"
    ));
    let selected = prepare(CommerceMode::Active)?.with_minds_facts()?;
    let actual = engine(&selected)?;
    check!(has(&actual, "roll", "action", "record_mind_term_lapse"));
    check!(has(&actual, "roll", "owner_read", "minds_contract_facts"));
    let count = actual
        .registry
        .definitions()
        .ok_or("definitions")?
        .iter()
        .filter(|d| {
            d.reference().key().owner.as_str() == "roll"
                && d.reference().key().kind.as_str() == "action"
                && d.reference().key().id.as_str() == "record_mind_term_lapse"
        })
        .count();
    check_eq!(
        count,
        1,
        "the actual action is declared once and shared with its participant"
    );
    check!(selected.with_minds_facts().is_err());
    // Merely registering a fact provider cannot fabricate commercial readiness.
    let provider = mush_roll::commerce::facts::MindsFactProvider::new();
    use mush_economy::FactProvider;
    check!(
        provider
            .resolve(&actual.world, "filed", "filed/0000000000000001", Some(1))
            .is_none()
    );
    Ok(())
}

#[test]
fn reconformance_requires_active_opt_in_and_exact_real_services() -> TestResult {
    check!(
        prepare(CommerceMode::Initialize)?
            .with_registry_reconformance()
            .is_err()
    );
    let absent = engine(&prepare(CommerceMode::Active)?)?;
    check!(!has(&absent, "roll", "action", "reconformance_admit"));
    check!(!has(&absent, "roll", "action", "recall_nonconforming_mind"));
    let configured = prepare(CommerceMode::Active)?.with_registry_reconformance()?;
    check!(
        prepare(CommerceMode::Active)?
            .with_registry_reconformance()?
            .with_registry_reconformance()
            .is_err()
    );
    let active = engine(&configured)?;
    for id in [
        "reconformance_admit",
        "reconformance_grant",
        "reconformance_settle",
        "reconformance_cancel",
        "reconformance_recover",
        "recall_nonconforming_mind",
    ] {
        check!(has(&active, "roll", "action", id));
    }
    check!(has(&active, "roll", "owner_read", "registry_reconformance"));
    check!(has(&active, "roll", "action", "rating_admit"));
    for id in ["registry_quota", "registry_artifacts"] {
        let definitions = active.registry.definitions().ok_or("actual definitions")?;
        check_eq!(
            definitions
                .iter()
                .filter(|d| d.reference().key().owner.as_str() == "tacsim"
                    && d.reference().key().kind.as_str() == "owner_participant"
                    && d.reference().key().id.as_str() == id)
                .count(),
            1
        );
    }
    Ok(())
}

fn house_policy() -> Result<mush_market::mind_house::HousePolicy, ModError> {
    use mush_economy::managed_entitlement::{GrossRecipientCut, Recipient};
    Ok(mush_market::mind_house::HousePolicy {
        price: Amount::from_minor(400),
        instrument: InstrumentId::new("cr").ok_or_else(|| ModError::msg("currency"))?,
        registry: GrossRecipientCut {
            permille: 100,
            recipient: Some(Recipient {
                account: AccountId(1),
                owner: PrincipalId::System(issuer()?),
            }),
        },
        venue_name: MarketId::new("registry").ok_or_else(|| ModError::msg("venue"))?,
        venue: GrossRecipientCut {
            permille: 0,
            recipient: None,
        },
        transferable: false,
        term_seconds: 90 * 86400,
        grace_seconds: 3 * 86400,
        quote_seconds: 600,
        renewal_price: Amount::from_minor(300),
        renewal_term_seconds: 60 * 86400,
        renewal_grace_seconds: 86400,
    })
}
fn house_engine(c: &MindCommerce, market: bool) -> Result<Box<Engine>, ModError> {
    let mut modules: Vec<Box<dyn GameModule>> = vec![
        Box::new(mush_org::OrgModule::new()),
        Box::new(c.configure_economy(EconomyModule::new().with_title_issuers(vec![issuer()?]))?),
        Box::new(c.configure_roll(RollModule::with_writers([crate::NS]))),
        Box::new(c.configure_tacsim(layer())?),
    ];
    if market {
        modules.insert(
            1,
            Box::new(
                c.configure_market(
                    MarketModule::new()
                        .with_venue_gate(std::sync::Arc::new(crate::port::market::PortVenueGate)),
                ),
            ),
        );
    }
    compose_boxed(World::new_minimal(), Registry::new(), modules)
}
#[test]
fn house_purchase_requires_explicit_active_house_and_all_actual_owners() -> TestResult {
    check!(
        prepare(CommerceMode::Active)?
            .with_house_minds(house_policy()?)
            .is_err()
    );
    check!(
        prepare(CommerceMode::Initialize)?
            .with_house_minds(house_policy()?)
            .is_err()
    );
    let ordinary = prepare(CommerceMode::Active)?.with_house_filings()?;
    check!(!has(
        house_engine(&ordinary, true)?.as_ref(),
        "market",
        "action",
        "purchase_house_mind"
    ));
    // Actual writer handles are installed once, including when a later owner
    // composition check refuses. Each independent boot needs a fresh handle.
    let missing_provider = prepare(CommerceMode::Active)?
        .with_house_filings()?
        .with_house_minds(house_policy()?)?;
    check!(
        house_engine(&missing_provider, false).is_err(),
        "actual provider is mandatory"
    );
    let selected = ordinary.with_house_minds(house_policy()?)?;
    let actual = house_engine(&selected, true)?;
    for id in [
        "list_house_mind",
        "quote_house_mind",
        "purchase_house_mind",
        "settle_house_mind",
        "refund_house_mind",
    ] {
        check!(has(&actual, "market", "action", id));
    }
    check!(has(&actual, "roll", "owner_read", "personal_house_holding"));
    check!(!has(
        &actual,
        "roll",
        "owner_read",
        "personal_entitlement_holding"
    ));
    check!(selected.with_house_minds(house_policy()?).is_err());
    Ok(())
}
#[test]
fn house_and_historical_personal_purchases_register_without_replacing_protocols() -> TestResult {
    let configured = prepare(CommerceMode::Active)?
        .with_original_purchases(quotes()?)?
        .with_house_filings()?
        .with_house_minds(house_policy()?)?;
    let actual = house_engine(&configured, true)?;
    check!(has(&actual, "market", "action", "purchase_original_mind"));
    check!(has(&actual, "market", "action", "purchase_house_mind"));
    let definitions = actual.registry.definitions().ok_or("definitions")?;
    let old = configured
        .state
        .purchases
        .as_ref()
        .ok_or("original config")?;
    let house = configured
        .state
        .house_purchases
        .as_ref()
        .ok_or("house config")?;
    check_eq!(old.participants(), house.participants());
    for reference in old
        .actions()?
        .into_iter()
        .chain(house.purchase_actions(mush_roll::commerce::house_purchase::HouseProtocol::HouseV1)?)
    {
        check!(definitions.get_exact(&reference).is_ok());
    }
    check_eq!(
        definitions
            .iter()
            .filter(|d| d.reference() == house.participants().economy())
            .count(),
        1
    );
    Ok(())
}

#[path = "pending_tests.rs"]
mod pending;

fn house_renewal_policy() -> Result<mush_market::mind_house_renewal::RenewalPolicy, ModError> {
    let house = house_policy()?;
    Ok(mush_market::mind_house_renewal::RenewalPolicy {
        registry: house.registry,
        venue: None,
        offer_ic_seconds: 600,
    })
}
#[test]
fn house_renewal_requires_explicit_active_purchase_and_all_three_owners() -> TestResult {
    check!(
        prepare(CommerceMode::Initialize)?
            .with_house_mind_renewals(house_renewal_policy()?)
            .is_err()
    );
    check!(
        prepare(CommerceMode::Active)?
            .with_house_mind_renewals(house_renewal_policy()?)
            .is_err()
    );
    let ordinary = prepare(CommerceMode::Active)?
        .with_house_filings()?
        .with_house_minds(house_policy()?)?;
    check!(!has(
        house_engine(&ordinary, true)?.as_ref(),
        "market",
        "action",
        "draft_house_mind_renewal"
    ));
    // The ordinary boot already installed its House purchase writer. Negative
    // and positive renewal boots must each compose fresh actual owner handles.
    let configured = || -> Result<MindCommerce, ModError> {
        prepare(CommerceMode::Active)?
            .with_house_filings()?
            .with_house_minds(house_policy()?)?
            .with_house_mind_renewals(house_renewal_policy()?)
    };
    check!(house_engine(&configured()?, false).is_err());
    let selected = configured()?;
    let actual = house_engine(&selected, true)?;
    for id in [
        "draft_house_mind_renewal",
        "fund_house_mind_renewal",
        "settle_house_mind_renewal",
        "refund_house_mind_renewal",
        "close_house_mind_renewal",
    ] {
        check!(has(&actual, "market", "action", id));
    }
    check!(!has(
        &actual,
        "market",
        "action",
        "close_house_mind_renewal_service"
    ));
    check!(has(&actual, "roll", "owner_read", "house_mind_renewal"));
    check!(
        selected
            .with_house_mind_renewals(house_renewal_policy()?)
            .is_err()
    );
    Ok(())
}
#[test]
fn house_renewal_service_is_optional_and_retains_exact_purchase_refs() -> TestResult {
    check!(
        prepare(CommerceMode::Active)?
            .with_house_mind_renewal_service()
            .is_err()
    );
    let selected = prepare(CommerceMode::Active)?
        .with_house_filings()?
        .with_house_minds(house_policy()?)?
        .with_house_mind_renewals(house_renewal_policy()?)?
        .with_house_mind_renewal_service()?;
    let actual = house_engine(&selected, true)?;
    let definitions = actual.registry.definitions().ok_or("definitions")?;
    let house = selected.state.house_purchases.as_ref().ok_or("house")?;
    let renewal = selected.state.house_renewals.as_ref().ok_or("renewal")?;
    check_eq!(house.participants(), renewal.participants());
    for reference in renewal
        .actions()?
        .into_iter()
        .chain(house.purchase_actions(mush_roll::commerce::house_purchase::HouseProtocol::HouseV1)?)
    {
        check!(definitions.get_exact(&reference).is_ok());
    }
    check!(has(
        &actual,
        "market",
        "action",
        "close_house_mind_renewal_service"
    ));
    check!(!has(&actual, "market", "action", "draft_mind_renewal"));
    // A missing paired native configuration must return its original error,
    // including at host composition, rather than panic on a private invariant.
    let mut missing = prepare(CommerceMode::Active)?;
    missing.state.house_renewal_service = selected.state.house_renewal_service.clone();
    check!(missing.configure_economy(EconomyModule::new()).is_err());
    check!(selected.with_house_mind_renewal_service().is_err());
    Ok(())
}

#[path = "paid_resale_tests.rs"]
mod paid_resale;

#[path = "house_two_seat_tests.rs"]
mod house_two_seat;

#[test]
fn derived_updates_require_real_dependent_services_and_the_distinct_fee_action() -> TestResult {
    check!(
        prepare(CommerceMode::Initialize)?
            .with_derived_updates()
            .is_err()
    );
    check!(
        prepare(CommerceMode::Active)?
            .with_derived_updates()
            .is_err()
    );
    let configured = || -> Result<MindCommerce, ModError> {
        prepare(CommerceMode::Active)?
            .with_original_source_purchases(quotes()?)?
            .with_authored_forks()?
            .with_marked_registry()?
            .with_dependent_owner_filings()?
            .with_derived_updates()
    };
    check!(configured()?.with_derived_updates().is_err());
    for fee_owner in [false, true] {
        let prepared = configured()?;
        let action = prepared.state.roll.derived_update_action()?;
        let mut modules: Vec<Box<dyn GameModule>> = vec![
            Box::new(prepared.configure_roll(RollModule::with_writers([crate::NS]))),
            Box::new(
                prepared.configure_market(
                    MarketModule::new()
                        .with_venue_gate(std::sync::Arc::new(crate::port::market::PortVenueGate)),
                ),
            ),
            Box::new(prepared.configure_tacsim(layer())?),
        ];
        if fee_owner {
            modules.push(Box::new(prepared.configure_economy(
                EconomyModule::new().with_title_issuers(vec![issuer()?]),
            )?));
        }
        let result = compose_boxed(World::new_minimal(), Registry::new(), modules);
        if !fee_owner {
            check!(result.is_err());
            continue;
        }
        let actual = result?;
        check!(
            actual
                .registry
                .definitions()
                .ok_or("definitions")?
                .get_exact(&action)
                .is_ok()
        );
        check!(has(&actual, "roll", "action", "update_derived_mind"));
        check!(!has(&actual, "roll", "action", "update_original_mind"));
        check!(has(&actual, "roll", "owner_read", "authored_fork_parent"));
    }
    Ok(())
}

// 17g C1a: the four player-market owners the builder could not select, and
// the order the full player-market chain follows (plan §2.3). Every boot
// composes fresh owner handles: actual writers are installed once.
fn renewal_policy() -> Result<mush_market::mind_renewal::RenewalPolicy, ModError> {
    use mush_economy::managed_entitlement::{GrossRecipientCut, Recipient};
    Ok(mush_market::mind_renewal::RenewalPolicy {
        registry: GrossRecipientCut {
            permille: 50,
            recipient: Some(Recipient {
                account: AccountId(1),
                owner: PrincipalId::System(issuer()?),
            }),
        },
        venue: None,
        offer_ic_seconds: 259_200,
    })
}
fn entitled() -> Result<MindCommerce, ModError> {
    entitlement_tests::prepared()?.with_mind_entitlements(entitlement_tests::policy()?)
}
fn registered(engine: &Engine, reference: &DefinitionRef) -> bool {
    engine
        .registry
        .definitions()
        .is_some_and(|definitions| definitions.get_exact(reference).is_ok())
}

#[test]
fn personal_renewals_are_explicit_active_entitlement_bound_and_once() -> TestResult {
    check!(
        prepare(CommerceMode::Initialize)?
            .with_mind_renewals(renewal_policy()?)
            .is_err()
    );
    check!(
        prepare(CommerceMode::Active)?
            .with_mind_renewals(renewal_policy()?)
            .is_err()
    );
    // The dependent-owner chain without its entitlement composition.
    check!(
        entitlement_tests::prepared()?
            .with_mind_renewals(renewal_policy()?)
            .is_err()
    );
    let selected = entitled()?.with_mind_renewals(renewal_policy()?)?;
    let actions = selected
        .state
        .renewals
        .as_ref()
        .ok_or("personal renewal configuration")?
        .actions()?;
    check_eq!(selected.state.renewal_actions.as_ref(), Some(&actions));
    let actual = house_engine(&selected, true)?;
    for action in &actions {
        check!(registered(&actual, action), "personal renewal action");
    }
    check!(has(&actual, "market", "action", "draft_mind_renewal"));
    check!(!has(
        &actual,
        "market",
        "action",
        "draft_house_mind_renewal"
    ));
    let without = house_engine(&entitled()?, true)?;
    for action in &actions {
        check!(!registered(&without, action), "unselected renewal action");
    }
    check!(
        selected.with_mind_renewals(renewal_policy()?).is_err(),
        "personal renewals are configured once"
    );
    Ok(())
}

#[test]
fn personal_renewal_service_requires_renewals_and_registers_its_close() -> TestResult {
    check!(
        prepare(CommerceMode::Active)?
            .with_mind_renewal_service()
            .is_err()
    );
    check!(entitled()?.with_mind_renewal_service().is_err());
    let renewals_only = entitled()?.with_mind_renewals(renewal_policy()?)?;
    let selected = entitled()?
        .with_mind_renewals(renewal_policy()?)?
        .with_mind_renewal_service()?;
    let action = selected
        .state
        .renewals
        .as_ref()
        .ok_or("personal renewal configuration")?
        .service_action()?
        .ok_or("personal renewal service action")?;
    check_eq!(selected.state.renewal_service.as_ref(), Some(&action));
    let actual = house_engine(&selected, true)?;
    check!(
        registered(&actual, &action),
        "the service close is registered"
    );
    let config = selected.state.renewals.as_ref().ok_or("renewals")?;
    for reference in config.actions()? {
        check!(registered(&actual, &reference));
    }
    check!(!registered(
        house_engine(&renewals_only, true)?.as_ref(),
        &action
    ));
    // A missing paired native configuration returns its own error at host
    // composition rather than panicking on a private invariant.
    let mut missing = prepare(CommerceMode::Active)?;
    missing.state.renewal_service = selected.state.renewal_service.clone();
    check!(missing.configure_economy(EconomyModule::new()).is_err());
    check!(
        selected.with_mind_renewal_service().is_err(),
        "the service is configured once"
    );
    Ok(())
}

#[test]
fn withdrawal_attestation_and_requalification_are_explicit_active_once() -> TestResult {
    // The registered ids, as the Roll owners declare them: `withdrawal.rs`,
    // `owner_attestation/protocol.rs` and `owner_qualification/protocol.rs`.
    const IDS: [&str; 3] = [
        "withdraw_authored_mind",
        "attest_owner_mind",
        "replace_owner_parent_qualification",
    ];
    type Select = fn(MindCommerce) -> Result<MindCommerce, ModError>;
    let selectors: [Select; 3] = [
        MindCommerce::with_author_withdrawal,
        MindCommerce::with_owner_attestations,
        MindCommerce::with_owner_parent_qualification,
    ];
    let plain = house_engine(&entitlement_tests::prepared()?, true)?;
    for id in IDS {
        check!(!has(&plain, "roll", "action", id), "unselected {id}");
    }
    for (index, select) in selectors.iter().enumerate() {
        check!(select(prepare(CommerceMode::Initialize)?).is_err());
        let selected = select(entitlement_tests::prepared()?)?;
        let actual = house_engine(&selected, true)?;
        for (other, id) in IDS.iter().enumerate() {
            check_eq!(
                has(&actual, "roll", "action", id),
                other == index,
                "only the selected owner registers"
            );
        }
        check!(select(selected).is_err(), "configured once");
    }
    Ok(())
}

#[test]
fn requalification_requires_dependent_owner_filing() -> TestResult {
    let before = prepare(CommerceMode::Active)?
        .with_original_source_purchases(quotes()?)?
        .with_authored_forks()?
        .with_marked_registry()?;
    check!(before.with_owner_parent_qualification().is_err());
    entitlement_tests::prepared()?.with_owner_parent_qualification()?;
    Ok(())
}

fn revocation_policy()
-> Result<mush_roll::commerce::registry_revocation::RegistryRevocationPolicy, ModError> {
    mush_roll::commerce::registry_revocation::RegistryRevocationPolicy::new(
        issuer()?,
        mush_org::OrgId(4),
        mush_org::OrgScopeId(5),
        vec![mush_core::ObjId(6)],
    )
}
fn recall_policy() -> Result<mush_roll::commerce::registry_recall::RegistryRecallPolicy, ModError> {
    mush_roll::commerce::registry_recall::RegistryRecallPolicy::new(
        issuer()?,
        mush_org::OrgId(4),
        mush_org::OrgScopeId(5),
        vec![mush_core::ObjId(6)],
    )
}
fn pending_policy() -> Result<mush_roll::commerce::PendingCharterTransferConfig, ModError> {
    mush_roll::commerce::PendingCharterTransferConfig::new(mush_core::ObjId(7), 259_200)
}
/// The player-market chain in C1c's builder order, without the House part.
/// `revocation_first` false moves revocation after pending charters and paid
/// resale: a convention, not a guard (plan §2.3).
fn player_market(revocation_first: bool) -> Result<MindCommerce, ModError> {
    let c = entitled()?
        .with_compatible_updates()?
        .with_derived_updates()?
        .with_charter_transfers()?;
    let c = if revocation_first {
        c.with_registry_revocation(revocation_policy()?)?
            .with_pending_charter_transfers(pending_policy()?)?
            .with_paid_mind_resales(mush_core::ObjId(7), 86_400)?
    } else {
        c.with_pending_charter_transfers(pending_policy()?)?
            .with_paid_mind_resales(mush_core::ObjId(7), 86_400)?
            .with_registry_revocation(revocation_policy()?)?
    };
    c.with_registry_recall(recall_policy()?)?
        .with_mind_renewals(renewal_policy()?)?
        .with_mind_renewal_service()?
        .with_author_withdrawal()?
        .with_owner_attestations()?
        .with_owner_parent_qualification()
}
fn definitions_of(engine: &Engine) -> Result<Vec<DefinitionRef>, Box<dyn std::error::Error>> {
    let mut all: Vec<_> = engine
        .registry
        .definitions()
        .ok_or("definitions")?
        .iter()
        .map(|entry| entry.reference().clone())
        .collect();
    all.sort();
    Ok(all)
}

#[test]
fn full_player_market_chain_registers_every_owner_in_builder_order() -> TestResult {
    let chain = player_market(true)?;
    let actual = house_engine(&chain, true)?;
    let mut expected: Vec<DefinitionRef> = Vec::new();
    expected.extend(chain.state.purchase_actions.clone().ok_or("purchases")?);
    for set in chain
        .state
        .entitlement_actions
        .clone()
        .ok_or("entitlements")?
    {
        expected.extend(set);
    }
    expected.extend(chain.state.dependent_filing_action.clone());
    expected.extend(chain.state.compatible_update_action.clone());
    expected.extend(chain.state.derived_update_action.clone());
    expected.extend(chain.state.charter_transfer_action.clone());
    expected.extend(chain.state.registry_revocation_action.clone());
    expected.push(recall_policy()?.action()?);
    expected.extend(pending_policy()?.actions()?);
    expected.extend(mush_roll::commerce::paid_resale::protocol::actions()?);
    expected.extend(chain.state.renewal_actions.clone().ok_or("renewals")?);
    expected.extend(chain.state.renewal_service.clone());
    // purchases 3, entitlements 6, dependent filing, compatible and derived
    // updates, charter accept, revocation, recall, pending 4, resale 5,
    // renewals 5 and their service.
    check_eq!(
        expected.len(),
        3 + 6 + 1 + 1 + 1 + 1 + 1 + 1 + 4 + 5 + 5 + 1
    );
    for reference in &expected {
        check!(registered(&actual, reference), "{reference:?}");
    }
    for id in [
        "fork_paid_mind",
        "withdraw_authored_mind",
        "attest_owner_mind",
        "replace_owner_parent_qualification",
    ] {
        check!(has(&actual, "roll", "action", id), "{id}");
    }
    // Real guards: swapping either pair refuses.
    check!(
        entitlement_tests::prepared()?
            .with_mind_renewals(renewal_policy()?)
            .is_err(),
        "personal renewals before entitlements"
    );
    check!(
        entitled()?
            .with_pending_charter_transfers(pending_policy()?)
            .is_err(),
        "pending charters before charter transfers"
    );
    // A convention, not a guard: revocation after pending charters and paid
    // resale composes, and registers exactly the same definitions.
    let late = player_market(false)?;
    let reordered = house_engine(&late, true)?;
    check_eq!(definitions_of(&reordered)?, definitions_of(&actual)?);
    Ok(())
}

/// 17g C2a test 3 (C2 plan §2.4): the line-mode command slot is composed
/// exactly with the player market. The full chain, Active with entitlements,
/// registers it; Initialize, an Active House-only chain (no entitlements, the
/// format-1 launch's shape) and a layer with no commerce do not. No chain
/// registers an owner read of the commands' own: their Roll kinds are folded
/// into `original_owner_holding` (§2.4 fallback 1, because the format-2
/// Active launch had no grant to spare), whose current version is 2
/// wherever commerce is Active. The full chain's owner-read service and grant
/// counts are printed against the caps of 32 each.
#[test]
fn the_command_service_is_composed_only_with_the_player_market() -> TestResult {
    use crate::minds::market_commands::MindCommandService;
    let caps = (
        mush_core::read_service::MAX_OWNER_READ_SERVICES,
        mush_core::read_service::MAX_OWNER_READ_GRANTS,
    );
    let composed = |engine: &Engine| {
        (
            engine.world.module_state::<MindCommandService>().is_some(),
            has(engine, "roll", "owner_read", "mind_commands"),
        )
    };
    let holdings_version = |engine: &Engine| {
        engine.registry.definitions().and_then(|definitions| {
            definitions.iter().find_map(|entry| {
                let reference = entry.reference();
                (reference.key().owner.as_str() == "roll"
                    && reference.key().kind.as_str() == "owner_read"
                    && reference.key().id.as_str() == "original_owner_holding")
                    .then(|| reference.version())
            })
        })
    };
    let chain = player_market(true)?;
    check!(chain.configure_tacsim(layer())?.mind_commands.is_some());
    let full = house_engine(&chain, true)?;
    let counts = full.registry.owner_read_counts().ok_or("frozen reads")?;
    eprintln!(
        "MIND_COMMANDS_OWNER_READS services={} grants={} caps={caps:?}",
        counts.0, counts.1
    );
    check_eq!(
        composed(&full),
        (true, false),
        "the full player market composes the slot and no read of its own; owner reads (services, grants) = {counts:?} of {caps:?}"
    );
    check_eq!(holdings_version(&full), Some(2), "the folded holdings read");
    check!(counts.0 <= caps.0 && counts.1 <= caps.1, "{counts:?}");
    // Initialize composes no market.
    let initialize = prepare(CommerceMode::Initialize)?;
    check!(
        initialize
            .configure_tacsim(layer())?
            .mind_commands
            .is_none()
    );
    check_eq!(
        composed(&*engine(&initialize)?),
        (false, false),
        "Initialize"
    );
    // Active House-only: the House chain without entitlements.
    let house = prepare(CommerceMode::Active)?
        .with_house_filings()?
        .with_house_minds(house_policy()?)?;
    check!(house.configure_tacsim(layer())?.mind_commands.is_none());
    check_eq!(
        composed(&*house_engine(&house, true)?),
        (false, false),
        "an Active House-only chain"
    );
    // Active with the dependent-owner chain but no entitlements yet.
    let dependent = entitlement_tests::prepared()?;
    check!(dependent.configure_tacsim(layer())?.mind_commands.is_none());
    // No commerce at all.
    check!(layer().mind_commands.is_none());
    let plain = compose_boxed(
        World::new_minimal(),
        Registry::new(),
        vec![Box::new(layer())],
    )?;
    check_eq!(composed(&plain), (false, false), "a layer with no commerce");
    Ok(())
}
