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
fn engine(config: &MindCommerce) -> Result<Engine, ModError> {
    // The real E4 owner remains earlier than Roll and Tacsim. Their exact
    // references are validated by the final registry, not constructor order.
    let modules: Vec<Box<dyn GameModule>> = vec![
        Box::new(
            config.configure_economy(EconomyModule::new().with_title_issuers(vec![issuer()?])),
        ),
        Box::new(config.configure_roll(RollModule::with_writers([crate::NS]))),
        Box::new(config.configure_tacsim(layer())?),
    ];
    Engine::with_modules(World::new_minimal(), Registry::new(), modules)
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
    let ordinary = Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        vec![
            Box::new(EconomyModule::new()),
            Box::new(RollModule::with_writers([crate::NS])),
            Box::new(layer()),
        ],
    )?;
    assert!(!has(&ordinary, "tacsim", "registry_manifest", "combat"));
    assert!(!has(&ordinary, "roll", "action", "rating_admit"));
    assert!(!has(&ordinary, "roll", "action", "initial_filing"));

    let initialize = prepare(CommerceMode::Initialize)?;
    let initialized = engine(&initialize)?;
    assert!(has(&initialized, "tacsim", "registry_manifest", "combat"));
    assert!(has(&initialized, "roll", "action", "initialize_commerce"));
    assert!(!has(&initialized, "roll", "action", "rating_admit"));
    assert!(!has(
        &initialized,
        "tacsim",
        "owner_participant",
        "registry_quota"
    ));

    let active = prepare(CommerceMode::Active)?;
    assert_eq!(initialize.manifest(), active.manifest());
    let registered = engine(&active)?;
    assert!(!has(&registered, "roll", "action", "initialize_commerce"));
    for id in [
        "rating_admit",
        "rating_grant",
        "rating_settle",
        "rating_cancel",
        "rating_recover",
    ] {
        assert!(has(&registered, "roll", "action", id));
    }
    assert!(has(
        &registered,
        "tacsim",
        "owner_participant",
        "registry_quota"
    ));
    assert!(has(
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
    assert_eq!(actual, active.manifest());
    // This is registration evidence only: Active hydration without the real
    // acknowledged deployment must refuse, not manufacture commercial health.
    let mut uninitialized = registered;
    assert!(
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
    assert!(
        prepared
            .configure_tacsim(TacsimRuleset::with_config(changed))
            .is_err()
    );
    let configured = prepared.configure_tacsim(layer())?;
    assert!(prepared.configure_tacsim(configured).is_err());
    let mut changed_after_attachment = prepared.configure_tacsim(layer())?;
    changed_after_attachment.cfg.match_.ai_fill = !changed_after_attachment.cfg.match_.ai_fill;
    assert!(
        Engine::with_modules(
            World::new_minimal(),
            Registry::new(),
            vec![
                Box::new(
                    prepared.configure_economy(
                        EconomyModule::new().with_title_issuers(vec![issuer()?]),
                    )
                ),
                Box::new(prepared.configure_roll(RollModule::with_writers([crate::NS]))),
                Box::new(changed_after_attachment),
            ],
        )
        .is_err(),
        "actual Tacsim registration repeats the full configuration fence"
    );
    assert!(
        Engine::with_modules(
            World::new_minimal(),
            Registry::new(),
            vec![
                Box::new(
                    prepared.configure_economy(
                        EconomyModule::new().with_title_issuers(vec![issuer()?])
                    )
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
    assert!(
        prepare(CommerceMode::Initialize)?
            .with_original_purchases(quotes()?)
            .is_err()
    );
    let prepared = prepare(CommerceMode::Active)?.with_original_purchases(quotes()?)?;
    let actual = prepared
        .purchases
        .as_ref()
        .ok_or("prepared purchases")?
        .actions()?;
    assert_eq!(prepared.purchase_actions.as_ref(), Some(&actual));
    let configured = prepared.configure_tacsim(layer())?;
    assert!(configured.personal_filed_holdings);
    assert_eq!(
        configured.original_holdings.as_ref(),
        Some(prepared.manifest())
    );
    // Real owner registrations and the real port presence gate; no venue,
    // account, licence or certificate is invented to prove composition.
    let actual_engine = Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        vec![
            Box::new(
                prepared
                    .configure_economy(EconomyModule::new().with_title_issuers(vec![issuer()?])),
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
        assert!(
            actual_engine
                .registry
                .definitions()
                .ok_or("definitions")?
                .iter()
                .any(|entry| entry.reference() == action)
        );
    }
    assert!(prepared.with_original_purchases(quotes()?).is_err());
    Ok(())
}

#[test]
fn source_forms_are_an_explicit_host_protocol_with_a_distinct_paid_reader() -> TestResult {
    assert!(
        prepare(CommerceMode::Initialize)?
            .with_original_source_purchases(quotes()?)
            .is_err()
    );
    let ordinary = prepare(CommerceMode::Active)?;
    assert!(!ordinary.configure_tacsim(layer())?.personal_source_forms);
    for source_forms in [false, true] {
        let prepared = prepare(CommerceMode::Active)?;
        let prepared = if source_forms {
            prepared.with_original_source_purchases(quotes()?)?
        } else {
            prepared.with_original_purchases(quotes()?)?
        };
        let configured = prepared.configure_tacsim(layer())?;
        assert_eq!(configured.personal_source_forms, source_forms);
        let actual =
            Engine::with_modules(
                World::new_minimal(),
                Registry::new(),
                vec![
                    Box::new(prepared.configure_economy(
                        EconomyModule::new().with_title_issuers(vec![issuer()?]),
                    )),
                    Box::new(prepared.configure_roll(RollModule::with_writers([crate::NS]))),
                    Box::new(prepared.configure_market(
                        MarketModule::new().with_venue_gate(std::sync::Arc::new(
                            crate::port::market::PortVenueGate,
                        )),
                    )),
                    Box::new(configured),
                ],
            )?;
        assert_eq!(
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
            assert_eq!(action.version(), if source_forms { 2 } else { 1 });
        }
    }
    Ok(())
}

#[test]
fn authored_forks_require_explicit_source_forms_and_register_exact_real_services() -> TestResult {
    assert!(
        prepare(CommerceMode::Initialize)?
            .with_authored_forks()
            .is_err()
    );
    assert!(
        prepare(CommerceMode::Active)?
            .with_authored_forks()
            .is_err()
    );
    assert!(
        prepare(CommerceMode::Active)?
            .with_original_purchases(quotes()?)?
            .with_authored_forks()
            .is_err()
    );
    for forks in [false, true] {
        let prepared = prepare(CommerceMode::Active)?.with_original_source_purchases(quotes()?)?;
        let prepared = if forks {
            prepared.with_authored_forks()?
        } else {
            prepared
        };
        let configured = prepared.configure_tacsim(layer())?;
        assert_eq!(
            configured.fork_material.as_ref(),
            forks.then(|| prepared.manifest())
        );
        let actual =
            Engine::with_modules(
                World::new_minimal(),
                Registry::new(),
                vec![
                    Box::new(prepared.configure_economy(
                        EconomyModule::new().with_title_issuers(vec![issuer()?]),
                    )),
                    Box::new(prepared.configure_market(
                        MarketModule::new().with_venue_gate(std::sync::Arc::new(
                            crate::port::market::PortVenueGate,
                        )),
                    )),
                    Box::new(prepared.configure_roll(RollModule::with_writers([crate::NS]))),
                    Box::new(configured),
                ],
            )?;
        assert_eq!(has(&actual, "roll", "action", "fork_paid_mind"), forks);
        assert_eq!(
            has(&actual, "roll", "owner_read", "authored_draft_access"),
            forks
        );
        assert_eq!(has(&actual, "roll", "outcome", "paid_mind_forked"), forks);
        // Real Engine freeze rejects duplicate declarations. The exact action
        // and outcome each enter once, then provider/grants reuse their refs.
        for (kind, id) in [
            ("action", "fork_paid_mind"),
            ("outcome", "paid_mind_forked"),
        ] {
            assert_eq!(
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
        assert_eq!(
            has(&actual, "roll", "owner_read", "authored_fork_parent"),
            forks
        );
        assert_eq!(
            has(&actual, "tacsim", "owner_read", "authored_fork_material"),
            forks
        );
        if forks {
            assert!(prepared.with_authored_forks().is_err());
        }
    }
    let prepared = prepare(CommerceMode::Active)?
        .with_original_source_purchases(quotes()?)?
        .with_authored_forks()?;
    let mut detached = prepared.configure_tacsim(layer())?;
    detached.fork_material = None;
    assert!(
        Engine::with_modules(
            World::new_minimal(),
            Registry::new(),
            vec![
                Box::new(
                    prepared.configure_economy(
                        EconomyModule::new().with_title_issuers(vec![issuer()?])
                    )
                ),
                Box::new(
                    prepared.configure_market(
                        MarketModule::new().with_venue_gate(std::sync::Arc::new(
                            crate::port::market::PortVenueGate
                        ))
                    )
                ),
                Box::new(prepared.configure_roll(RollModule::with_writers([crate::NS]))),
                Box::new(detached),
            ]
        )
        .is_err(),
        "a missing actual source service cannot be replaced by a structural descriptor"
    );
    Ok(())
}

#[test]
fn marked_registry_is_explicit_saved_admission_with_unchanged_rating_protocol() -> TestResult {
    assert!(
        prepare(CommerceMode::Initialize)?
            .with_marked_registry()
            .is_err()
    );
    assert!(
        prepare(CommerceMode::Active)?
            .with_marked_registry()
            .is_err()
    );
    assert!(
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
        assert_eq!(
            configured.marked_registry.as_ref(),
            marked.then(|| prepared.manifest())
        );
        let actual =
            Engine::with_modules(
                World::new_minimal(),
                Registry::new(),
                vec![
                    Box::new(prepared.configure_economy(
                        EconomyModule::new().with_title_issuers(vec![issuer()?]),
                    )),
                    Box::new(prepared.configure_market(
                        MarketModule::new().with_venue_gate(std::sync::Arc::new(
                            crate::port::market::PortVenueGate,
                        )),
                    )),
                    Box::new(prepared.configure_roll(RollModule::with_writers([crate::NS]))),
                    Box::new(configured),
                ],
            )?;
        assert_eq!(
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
        assert_eq!(refs.len(), 5);
        if let Some(previous) = &ordinary_refs {
            assert_eq!(
                &refs, previous,
                "existing protocol/history identities stay exact"
            );
        } else {
            ordinary_refs = Some(refs);
        }
        if marked {
            assert!(prepared.with_marked_registry().is_err());
        }
    }
    Ok(())
}

#[path = "entitlement_tests.rs"]
mod entitlement_tests;
