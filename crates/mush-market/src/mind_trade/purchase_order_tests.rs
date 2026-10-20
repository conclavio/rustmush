//! Real composition only: the manifest is this module's strict registration
//! fixture, not a certificate. No boot data, money, licence, or readiness is
//! manufactured. Actual owner modules must resolve the same opaque writer cell
//! before Engine composition returns, independent of roster order.

use super::*;
use mush_core::{Engine, GameModule, Registry, World};
use mush_economy::registry_filing::InitialFilingPolicy;
use mush_roll::commerce::filing::InitialFilingTerms;
use mush_roll::commerce::module::{CommerceConfig, CommerceMode};
use mush_roll::commerce::purchase::PurchaseParticipants;
use mush_roll::commerce::purchase_writer::OriginalPurchaseWriter;
use mush_roll::commerce::rating::RatingCompatibility;
use std::sync::Mutex;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

#[derive(Clone, Copy)]
enum Wiring {
    Shared,
    Missing,
    Split,
    MismatchedParticipants,
}

struct Registered {
    module: Box<dyn GameModule>,
    calls: Arc<Mutex<Vec<&'static str>>>,
}
impl GameModule for Registered {
    fn name(&self) -> &'static str {
        self.module.name()
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        self.module.register(reg)?;
        self.calls
            .lock()
            .map_err(|_| ModError::msg("registration trace poisoned"))?
            .push(self.module.name());
        Ok(())
    }
}

struct Attempt {
    result: Result<Box<Engine>, ModError>,
    purchase: OriginalPurchaseConfig,
    calls: Arc<Mutex<Vec<&'static str>>>,
}

fn policy(fee: i64) -> TestResult<InitialFilingPolicy> {
    Ok(InitialFilingPolicy::new(
        SystemPrincipalId::new("roll.registry").ok_or("issuer")?,
        AccountId(11),
        Amount::from_minor(fee),
        mush_economy::CatalogRef {
            ruleset: "roll".into(),
            code: "mind_charter".into(),
        },
    )?)
}

fn participants(fee: i64) -> TestResult<PurchaseParticipants> {
    let economy = DefinitionRegistryBuilder::new().register(
        OwnerId::new("economy")?,
        descriptor(
            "owner_participant",
            "initial_filing",
            policy(fee)?.definition_payload(),
        )?,
    )?;
    let retention = DefinitionRegistryBuilder::new().register(
        OwnerId::new("roll")?,
        mush_roll::commerce::rating::registration::commercial_roots_descriptor()?,
    )?;
    Ok(PurchaseParticipants::new(economy, retention)?)
}

#[inline(never)]
fn attempt(market_first: bool, wiring: Wiring) -> TestResult<Attempt> {
    let writer = OriginalPurchaseWriter::new();
    let expected_participants = participants(100)?;
    let purchase =
        OriginalPurchaseConfig::new(config(), expected_participants.clone(), writer.clone())?;
    let mut roll = CommerceConfig::new(
        CommerceMode::Active,
        InitialFilingTerms::new(90 * 86_400, 3 * 86_400)?,
        manifest(),
        RatingCompatibility::combat_v1(manifest().digest()),
    )?
    .with_original_market_quotes();
    if !matches!(wiring, Wiring::Missing) {
        let roll_writer = if matches!(wiring, Wiring::Split) {
            OriginalPurchaseWriter::new()
        } else {
            writer
        };
        let roll_participants = if matches!(wiring, Wiring::MismatchedParticipants) {
            // Exact structural refs are lawful but differ from actual E4's
            // configured fee. The cell comparison must catch this before the
            // later participant grant resolver sees these crossed actions.
            participants(101)?
        } else {
            expected_participants
        };
        let roll_actions =
            OriginalPurchaseConfig::new(config(), roll_participants.clone(), roll_writer.clone())?
                .actions()?;
        roll = roll.with_original_purchases(roll_writer, roll_participants, roll_actions)?;
    }
    let economy = mush_economy::EconomyModule::new()
        .with_title_issuers(vec![
            SystemPrincipalId::new("roll.registry").ok_or("issuer")?,
        ])
        .with_initial_filing(policy(100)?, roll.initial_filing_action()?)
        .with_original_mind_purchases(purchase.actions()?);
    let roll = mush_roll::RollModule::with_writers(["tacsim"]).with_commerce(roll);
    let market = crate::MarketModule::new()
        .with_venue_gate(Arc::new(Gate))
        .with_original_mind_purchases(purchase.clone());
    let mut modules: Vec<Box<dyn GameModule>> = vec![Box::new(Manifest), Box::new(economy)];
    if market_first {
        modules.extend([Box::new(market) as Box<dyn GameModule>, Box::new(roll)]);
    } else {
        modules.extend([Box::new(roll) as Box<dyn GameModule>, Box::new(market)]);
    }
    let calls = Arc::new(Mutex::new(Vec::new()));
    let modules = modules
        .into_iter()
        .map(|module| {
            Box::new(Registered {
                module,
                calls: calls.clone(),
            }) as Box<dyn GameModule>
        })
        .collect();
    Ok(Attempt {
        result: Engine::with_modules(World::new_minimal(), Registry::new(), modules).map(Box::new),
        purchase,
        calls,
    })
}

#[test]
fn actual_paid_writer_composes_in_both_owner_orders_without_business_state() -> TestResult {
    let mut fingerprint = None;
    for market_first in [false, true] {
        let Attempt {
            result,
            purchase,
            calls,
        } = attempt(market_first, Wiring::Shared)?;
        let mut engine = result?;
        let expected = if market_first {
            vec!["tacsim", "economy", "market", "roll"]
        } else {
            vec!["tacsim", "economy", "roll", "market"]
        };
        assert_eq!(*calls.lock().map_err(|_| "trace poisoned")?, expected);
        let definitions = engine.registry.definitions().ok_or("definitions")?;
        for action in purchase.actions()? {
            definitions.get_exact(&action)?;
            engine
                .registry
                .action_providers()
                .ok_or("providers")?
                .get_exact(&action)?;
        }
        if let Some(prior) = fingerprint {
            assert_eq!(definitions.fingerprint(), prior);
        } else {
            fingerprint = Some(definitions.fingerprint());
        }
        assert!(engine.action_state().is_empty());
        assert!(!engine.action_state().has_prepared());
        // Composition is not commercial boot, payment, or licence authority.
        // Every actual registered phase still refuses against untouched state.
        for (n, action) in purchase.actions()?.iter().enumerate() {
            let request = if n == 0 {
                json!({"quote":token(40)})
            } else {
                json!({"operation":token(40)})
            };
            assert!(
                engine
                    .prepare_registered_composed_authoritative_action(
                        action,
                        &invocation(request),
                        token(u8::try_from(41 + n)?)
                    )
                    .is_err()
            );
            assert!(engine.action_state().is_empty());
            assert!(!engine.action_state().has_prepared());
            assert!(!engine.action_state().reservation_lost());
        }
    }
    Ok(())
}

fn refused_after_all_registration(wiring: Wiring, expected: &str) -> TestResult {
    for market_first in [false, true] {
        let Attempt { result, calls, .. } = attempt(market_first, wiring)?;
        let error = result
            .err()
            .ok_or("invalid composition unexpectedly succeeded")?;
        assert!(error.to_string().contains("definition registry"), "{error}");
        assert!(error.to_string().contains(expected), "{error}");
        assert_eq!(calls.lock().map_err(|_| "trace poisoned")?.len(), 4);
    }
    Ok(())
}

#[test]
fn actual_paid_writer_missing_install_refuses_at_final_definition_freeze() -> TestResult {
    refused_after_all_registration(Wiring::Missing, "Roll paid writer is not registered")
}

#[test]
fn actual_paid_writer_split_cells_refuse_at_final_definition_freeze() -> TestResult {
    refused_after_all_registration(Wiring::Split, "Roll paid writer is not registered")
}

#[test]
fn actual_paid_writer_crossed_participants_refuse_before_hydration_in_both_orders() -> TestResult {
    for market_first in [false, true] {
        let Attempt { result, calls, .. } = attempt(market_first, Wiring::MismatchedParticipants)?;
        let error = result
            .err()
            .ok_or("crossed composition unexpectedly succeeded")?;
        assert!(
            error
                .to_string()
                .contains("paid writer composition differs"),
            "{error}"
        );
        let names = calls.lock().map_err(|_| "trace poisoned")?;
        if market_first {
            assert!(error.to_string().contains("definition registry"), "{error}");
            assert_eq!(
                names.len(),
                4,
                "deferred cell checked after all registrations"
            );
        } else {
            assert_eq!(*names, vec!["tacsim", "economy", "roll"]);
        }
    }
    Ok(())
}
