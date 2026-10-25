#![cfg(test)]

//! Structural codecs and registered typed envelopes only. Actual authority,
//! native receipt/replay and seller/buyer custody are tested by the genuine child.
use super::*;
use crate::commerce::{families::CommerceFamilies, records::Record};
use mush_core::{DefinitionDigest, Engine, GameModule, Registrar, Registry, StateFamilyHandle};
use mush_economy::{AccountId, Amount, InstrumentId, TransactionId};
use serde_json::json;
use std::sync::{Arc, Mutex};
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn offer() -> TestResult<CharterOffer> {
    let d = DefinitionDigest::from_bytes([255; 32]);
    Ok(serde_json::from_value(json!({
        "format":1,"operation":CommitToken::from_bytes([254;16]),"action":protocol::offer_action()?,
        "charter":u32::MAX.to_string(),"birth":CommitToken::from_bytes([253;16]),
        "ownership":{"generation":65536,"operation":CommitToken::from_bytes([252;16]),"digest":d},
        "seller":u32::MAX,"buyer":u32::MAX-1,"seller_account":u64::MAX,"instrument":"cr",
        "price":i64::MAX,"title_revision":u64::MAX-1,"line":u64::MAX.to_string(),"filed":u64::MAX.to_string(),
        "revision":{"format":1,"revision":{"key":{"character":u32::MAX,"mind":u64::MAX,"number":u64::MAX},"digest":d},"source":d},
        "offered_at_ic":{"clock":{"kind":"world_ic","generation":u64::MAX},"at":i64::MAX-86400},
        "offered_at_utc":i64::MAX-86400,"expires_at_utc":i64::MAX
    }))?)
}
fn transfer(offer: &CharterOffer) -> TestResult<CharterTransfer> {
    let transaction = TransactionId(u64::MAX);
    let at = i64::MAX - 1;
    let title = mush_economy::EconomicAsset {
        object: offer.charter.object(),
        title_owner: mush_org::PrincipalId::Object(offer.buyer),
        scope: None,
        assignment: None,
        maintenance_account: None,
        catalog: Some(mush_economy::CatalogRef {
            ruleset: "roll".into(),
            code: "personal_mind_charter".into(),
        }),
        encumbrances: vec![],
        acquisition: Some(mush_economy::AcquisitionRecord { transaction, at }),
        lifecycle: mush_economy::AssetLifecycle::Active,
        last_journal_seq: u64::MAX,
        revision: u64::MAX,
        stolen: None,
        frozen: false,
        frozen_reason: None,
    };
    let row = CharterTransfer {
        format: 1,
        operation: CommitToken::from_bytes([255; 16]),
        action: protocol::accept_action()?,
        charter: offer.charter,
        generation: 65537,
        birth: offer.birth,
        predecessor: offer.ownership.clone(),
        offer: offer.operation,
        offer_digest: offer.digest()?,
        accepted_at_ic: offer.offered_at_ic.clone(),
        economic: mush_economy::registry_filing::charter_transfer::CharterTransferAuditSubject {
            commit: CommitToken::from_bytes([255; 16]),
            transaction,
            journal_seq: u64::MAX,
            posted_at_utc: at,
            plan_hash: [255; 32],
            offer: offer.operation,
            offer_digest: offer.digest()?,
            seller: offer.seller,
            buyer: offer.buyer,
            charter: offer.charter.object(),
            title_revision: offer.title_revision,
            payer: AccountId(u64::MAX - 1),
            seller_account: offer.seller_account,
            price: offer.price,
            instrument: offer.instrument.clone(),
            issuer: mush_org::SystemPrincipalId::new("registry").ok_or("issuer")?,
            title,
        }
        .into(),
    };
    row.matches_offer(offer)?;
    Ok(row)
}
#[test]
fn exact_offer_transfer_and_head_are_closed_and_generation_bounded() -> TestResult {
    let offer = offer()?;
    let row = transfer(&offer)?;
    let head = CharterOwnershipHead {
        format: 1,
        charter: row.charter,
        current: row.ownership()?,
    };
    for value in [
        serde_json::to_value(&offer)?,
        serde_json::to_value(&row)?,
        serde_json::to_value(&head)?,
    ] {
        check!(value.is_object());
    }
    check_eq!(
        serde_json::from_value::<CharterOffer>(serde_json::to_value(&offer)?)?,
        offer
    );
    check_eq!(
        serde_json::from_value::<CharterTransfer>(serde_json::to_value(&row)?)?,
        row
    );
    check_eq!(
        serde_json::from_value::<CharterOwnershipHead>(serde_json::to_value(&head)?)?,
        head
    );
    for field in ["source", "validated", "permission", "parent"] {
        let mut value = serde_json::to_value(&offer)?;
        value
            .as_object_mut()
            .ok_or("offer")?
            .insert(field.into(), json!(true));
        check!(serde_json::from_value::<CharterOffer>(value).is_err());
    }
    for generation in [0, 65538] {
        let mut bad = row.clone();
        bad.generation = generation;
        check!(bad.validate().is_err());
    }
    for generation in [0, 1, 65538] {
        let mut bad = head.clone();
        bad.current.generation = generation;
        check!(bad.validate().is_err());
    }
    let mut bad = offer.clone();
    bad.ownership.generation = 65537;
    check!(
        bad.validate().is_err(),
        "last allowed transfer cannot issue another offer"
    );
    for field in [
        "ownership",
        "offered_at_ic",
        "expires_at_utc",
        "seller_account",
    ] {
        let mut bad = serde_json::to_value(&offer)?;
        bad.as_object_mut().ok_or("offer")?.remove(field);
        check!(serde_json::from_value::<CharterOffer>(bad).is_err());
    }
    Ok(())
}
#[test]
fn native_and_seller_terms_must_match_without_netting_or_substitution() -> TestResult {
    let offer = offer()?;
    let row = transfer(&offer)?;
    for mode in 0..10 {
        let mut bad = offer.clone();
        match mode {
            0 => bad.buyer = ObjId(7),
            1 => bad.seller = ObjId(8),
            2 => bad.price = Amount::from_minor(1),
            3 => bad.seller_account = AccountId(0),
            4 => bad.instrument = InstrumentId::new("other").ok_or("instrument")?,
            5 => bad.title_revision -= 1,
            6 => bad.ownership.digest = DefinitionDigest::from_bytes([17; 32]),
            7 => bad.birth = CommitToken::from_bytes([11; 16]),
            8 => bad.line = crate::commerce::LineId::new(1)?,
            _ => bad.offered_at_ic.clock = mush_core::ClockRef::WorldIc { generation: 1 },
        }
        check!(row.matches_offer(&bad).is_err());
    }
    for mode in 0..4 {
        let mut bad = row.clone();
        let economic::TransferEconomic::Immediate(native) = &mut bad.economic else {
            return Err("expected exact immediate native fixture".into());
        };
        match mode {
            0 => native.posted_at_utc = offer.offered_at_utc - 1,
            1 => native.posted_at_utc = offer.expires_at_utc,
            2 => bad.accepted_at_ic.at = offer.offered_at_ic.at - 1,
            _ => bad.accepted_at_ic.clock = mush_core::ClockRef::WorldIc { generation: 1 },
        }
        // Preserve native acquisition consistency when testing the offer window.
        if let Some(acquisition) = &mut native.title.acquisition {
            acquisition.at = native.posted_at_utc;
        }
        check!(bad.matches_offer(&offer).is_err());
    }
    Ok(())
}
type Handles = (
    StateFamilyHandle<String, CharterOffer>,
    StateFamilyHandle<String, CharterTransfer>,
    StateFamilyHandle<String, CharterOwnershipHead>,
);
struct Rows(Arc<Mutex<Option<Handles>>>);
impl GameModule for Rows {
    fn name(&self) -> &'static str {
        "roll"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let f = CommerceFamilies::register(reg)?;
        *self.0.lock().map_err(|_| ModError::msg("handles"))? = Some((
            f.charter_transfer_offers,
            f.charter_transfers,
            f.charter_ownership,
        ));
        Ok(())
    }
}
#[test]
fn wide_scalar_complete_documents_fit_actual_families_and_keys_are_exact() -> TestResult {
    let shared = Arc::new(Mutex::new(None));
    let mut engine = Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        vec![Box::new(Rows(shared.clone()))],
    )?;
    let (offers, transfers, heads) = shared
        .lock()
        .map_err(|_| "handles")?
        .clone()
        .ok_or("handles")?;
    let offer = offer()?;
    let transfer = transfer(&offer)?;
    let head = CharterOwnershipHead {
        format: 1,
        charter: transfer.charter,
        current: transfer.ownership()?,
    };
    for (owner, id, schema, key, value, max) in [
        (
            offers.owner(),
            offers.descriptor().id(),
            offers.descriptor().value_schema(),
            offer.key(),
            serde_json::to_value(&offer)?,
            CharterOffer::MAX_BYTES,
        ),
        (
            transfers.owner(),
            transfers.descriptor().id(),
            transfers.descriptor().value_schema(),
            transfer.key(),
            serde_json::to_value(&transfer)?,
            CharterTransfer::MAX_BYTES,
        ),
        (
            heads.owner(),
            heads.descriptor().id(),
            heads.descriptor().value_schema(),
            head.key(),
            serde_json::to_value(&head)?,
            CharterOwnershipHead::MAX_BYTES,
        ),
    ] {
        let document = json!({"family":{"owner":owner,"id":id},"key":key,"schema":{"id":schema,"version":1},"value":value});
        check!(mush_core::canonical_json_bytes_with_limit(&document, max)?.len() < max);
    }
    let mut txn = engine.action_transaction(&offers)?;
    txn.insert(&offers, &offer.key(), offer.clone())?;
    txn.insert(&transfers, &transfer.key(), transfer.clone())?;
    txn.insert(&heads, &head.key(), head.clone())?;
    let (_, install) = txn.prepare()?.into_parts();
    engine.install_prepared_action(install)?;
    let mut txn = engine.action_transaction(&offers)?;
    check!(txn.insert(&offers, &"not-a-commit".into(), offer).is_err());
    Ok(())
}
#[test]
fn transfer_configuration_is_optional_active_once_and_manifest_independent() -> TestResult {
    use crate::commerce::bootstrap_tests::{manifest_descriptor, reference};
    use crate::commerce::{
        filing::InitialFilingTerms,
        module::{CommerceConfig, CommerceMode},
        rating::RatingCompatibility,
    };
    let manifest = reference("tacsim", manifest_descriptor());
    let config = |mode| {
        CommerceConfig::new(
            mode,
            InitialFilingTerms::new(100, 20).unwrap(),
            manifest.clone(),
            RatingCompatibility::combat_v1(manifest.digest()),
        )
    };
    check!(
        config(CommerceMode::Initialize)?
            .with_charter_transfers(CharterTransferConfig::new())
            .is_err()
    );
    let active =
        config(CommerceMode::Active)?.with_charter_transfers(CharterTransferConfig::new())?;
    check!(
        active
            .with_charter_transfers(CharterTransferConfig::new())
            .is_err()
    );
    let [offer, accept] = CharterTransferConfig::new().actions()?;
    check_ne!(offer, accept);
    for descriptor in protocol::definitions()? {
        check!(
            !descriptor
                .references()
                .iter()
                .any(|r| r.key().kind.as_str() == "registry_manifest")
        );
    }
    Ok(())
}

#[test]
fn actual_registration_requires_matching_native_owner_and_preserves_default_absence() -> TestResult
{
    use crate::commerce::bootstrap_tests::{Manifest, manifest_descriptor, reference};
    use crate::commerce::{
        filing::InitialFilingTerms,
        module::{CommerceConfig, CommerceMode},
        rating::RatingCompatibility,
    };
    let manifest = reference("tacsim", manifest_descriptor());
    let [offer, accept] = CharterTransferConfig::new().actions()?;
    for reverse in [false, true] {
        for (enabled, native) in [(false, false), (true, false), (true, true)] {
            let config = CommerceConfig::new(
                CommerceMode::Active,
                InitialFilingTerms::new(100, 20)?,
                manifest.clone(),
                RatingCompatibility::combat_v1(manifest.digest()),
            )?;
            let config = if enabled {
                config.with_charter_transfers(CharterTransferConfig::new())?
            } else {
                config
            };
            let issuer = mush_org::SystemPrincipalId::new("roll.registry").ok_or("issuer")?;
            let policy = mush_economy::registry_filing::InitialFilingPolicy::new(
                issuer.clone(),
                AccountId(1),
                Amount::from_minor(100),
                mush_economy::CatalogRef {
                    ruleset: "roll".into(),
                    code: "mind_charter".into(),
                },
            )?;
            let economy = mush_economy::EconomyModule::new()
                .with_title_issuers(vec![issuer])
                .with_initial_filing(policy, config.initial_filing_action()?);
            let economy = if native {
                economy.with_mind_charter_transfers(accept.clone())
            } else {
                economy
            };
            let roll = crate::RollModule::with_writers(["tacsim"]).with_commerce(config);
            let mut modules: Vec<Box<dyn GameModule>> = vec![Box::new(Manifest)];
            if reverse {
                modules.extend([Box::new(economy) as Box<dyn GameModule>, Box::new(roll)]);
            } else {
                modules.extend([Box::new(roll) as Box<dyn GameModule>, Box::new(economy)]);
            }
            let result = Engine::with_modules(World::new_minimal(), Registry::new(), modules);
            if enabled && !native {
                check!(result.is_err());
                continue;
            }
            let engine = result?;
            let definitions = engine.registry.definitions().ok_or("actual definitions")?;
            for action in [&offer, &accept] {
                check_eq!(
                    definitions
                        .iter()
                        .filter(|entry| entry.reference() == action)
                        .count(),
                    usize::from(enabled)
                );
            }
        }
    }
    Ok(())
}

#[test]
fn immediate_native_wire_and_v1_ownership_digest_remain_exact() -> TestResult {
    let offer = offer()?;
    let row = transfer(&offer)?;
    let economic::TransferEconomic::Immediate(native) = &row.economic else {
        return Err("actual immediate fixture variant".into());
    };
    check_eq!(
        serde_json::to_vec(&row.economic)?,
        serde_json::to_vec(native)?
    );
    let legacy = json!({"format":1,"operation":row.operation,"action":row.action,
        "charter":row.charter,"generation":row.generation,"birth":row.birth,
        "predecessor":row.predecessor,"offer":row.offer,"offer_digest":row.offer_digest,
        "accepted_at_ic":row.accepted_at_ic,"economic":native});
    check_eq!(serde_json::to_value(&row)?, legacy);
    check_eq!(
        row.ownership()?.digest,
        DefinitionDigest::for_canonical_value("roll.charter_ownership.transfer.v1", &legacy)?
    );
    check_eq!(serde_json::from_value::<CharterTransfer>(legacy)?, row);
    Ok(())
}
