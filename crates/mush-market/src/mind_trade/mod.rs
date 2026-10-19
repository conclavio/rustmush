//! Opt-in typed original personal listings and purchase quotes. Neither action
//! moves money, creates a contract or publishes a paid licence. The actual
//! managed purchase must consume these exact rows and recheck every live fact.
//! Legacy market quotes, allocators, observers and delivery remain separate.

use mush_core::mechanics::{
    CanonicalId, DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder,
    OwnerId,
};
use mush_core::{
    CommitToken, DefinitionDigest, DefinitionRef, ModError, Registrar, StateFamilyDescriptor,
    StateFamilyHandle, StateFamilyId,
};
use mush_economy::{AccountId, Amount, InstrumentId};
use mush_org::SystemPrincipalId;
use serde::Serialize;
use serde_json::{Value, json};
use std::sync::Arc;

mod index;
mod protocol;
mod provider;
mod purchase_audit;
mod purchase_guard;
mod purchase_provider;
mod purchase_records;
mod purchase_service;
pub(crate) use purchase_guard::PurchaseGuard;
pub use purchase_provider::OriginalPurchaseConfig;
pub(crate) use purchase_provider::{PurchaseFamilies, register as register_purchase};
pub(crate) use purchase_service::enable as enable_purchase_service;
mod query;
pub mod records;
use records::{MAX_QUOTE_SECONDS, MindListing, MindQuote, QuoteVenue, Row};

pub const LIST_ACTION: &str = "list_original_mind";
pub const QUOTE_ACTION: &str = "quote_original_mind";
pub const RETIRE_LISTING_ACTION: &str = "retire_original_mind_listing";
pub const RETIRE_QUOTE_ACTION: &str = "retire_original_mind_quote";
const EXPIRE_ACTION: &str = "expire_original_mind_quotes";
pub const QUERY: &str = "market.original_minds";
const EXPIRY_TICK: &str = "original_mind_quote_expiry";
const OUTCOME: &str = "original_mind_offer_recorded";

/// Composition-frozen prices/recipients. Names of accounts are not accepted
/// from listing or quote requests. Real E4 ownership, instrument and status are
/// still checked before deriving either quote; these terms prove no account.
#[derive(Clone, Debug, Serialize)]
pub struct OriginalQuotePolicy {
    price: Amount,
    instrument: InstrumentId,
    registry_recipient: SystemPrincipalId,
    registry_account: AccountId,
    registry_permille: u16,
    venue: QuoteVenue,
    transferable: bool,
    term_seconds: i64,
    grace_seconds: i64,
    quote_seconds: i64,
}
impl OriginalQuotePolicy {
    pub fn new(
        price: OriginalPrice,
        venue: QuoteVenue,
        duration: OriginalDuration,
    ) -> Result<Self, ModError> {
        let value = Self {
            price: price.amount,
            instrument: price.instrument,
            registry_recipient: price.registry_recipient,
            registry_account: price.registry_account,
            registry_permille: price.registry_permille,
            venue,
            transferable: duration.transferable,
            term_seconds: duration.term_seconds,
            grace_seconds: duration.grace_seconds,
            quote_seconds: duration.quote_seconds,
        };
        value.validate().map_err(error)?;
        Ok(value)
    }
    fn validate(&self) -> Result<(), String> {
        if self.price.is_negative()
            || self.price.is_zero()
            || u32::from(self.registry_permille) + u32::from(self.venue.permille) > 1000
            || !(1..=mush_roll::commerce::MAX_TERM_SECONDS).contains(&self.term_seconds)
            || !(0..=mush_roll::commerce::MAX_GRACE_SECONDS).contains(&self.grace_seconds)
            || !(1..=MAX_QUOTE_SECONDS).contains(&self.quote_seconds)
        {
            return Err("invalid original mind quote policy".into());
        }
        self.venue.validate()
    }
    fn digest(&self) -> Result<DefinitionDigest, String> {
        records::digest("market.original_quote_policy.v1", self, 8192)
    }
}

#[derive(Clone, Debug)]
pub struct OriginalPrice {
    amount: Amount,
    instrument: InstrumentId,
    registry_recipient: SystemPrincipalId,
    registry_account: AccountId,
    registry_permille: u16,
}
impl OriginalPrice {
    pub fn new(
        amount: Amount,
        instrument: InstrumentId,
        registry_recipient: SystemPrincipalId,
        registry_account: AccountId,
        registry_permille: u16,
    ) -> Result<Self, ModError> {
        if amount.is_negative() || amount.is_zero() || registry_permille > 1000 {
            return Err(ModError::msg("invalid original price"));
        }
        Ok(Self {
            amount,
            instrument,
            registry_recipient,
            registry_account,
            registry_permille,
        })
    }
}
#[derive(Clone, Debug)]
pub struct OriginalDuration {
    transferable: bool,
    term_seconds: i64,
    grace_seconds: i64,
    quote_seconds: i64,
}
impl OriginalDuration {
    pub fn new(
        transferable: bool,
        term_seconds: i64,
        grace_seconds: i64,
        quote_seconds: i64,
    ) -> Result<Self, ModError> {
        if !(1..=mush_roll::commerce::MAX_TERM_SECONDS).contains(&term_seconds)
            || !(0..=mush_roll::commerce::MAX_GRACE_SECONDS).contains(&grace_seconds)
            || !(1..=MAX_QUOTE_SECONDS).contains(&quote_seconds)
        {
            return Err(ModError::msg("invalid original quote duration"));
        }
        Ok(Self {
            transferable,
            term_seconds,
            grace_seconds,
            quote_seconds,
        })
    }
}

#[derive(Clone, Debug)]
pub struct OriginalQuoteConfig {
    policy: OriginalQuotePolicy,
    manifest: DefinitionRef,
    source_forms: bool,
}
impl OriginalQuoteConfig {
    pub fn new(policy: OriginalQuotePolicy, manifest: DefinitionRef) -> Result<Self, ModError> {
        if manifest.key().owner.as_str() != "tacsim"
            || manifest.key().kind.as_str() != "registry_manifest"
            || manifest.key().id.as_str() != "combat"
            || manifest.version() != 1
        {
            return Err(ModError::msg(
                "original quotes require the actual combat Registry manifest",
            ));
        }
        policy.validate().map_err(error)?;
        Ok(Self {
            policy,
            manifest,
            source_forms: false,
        })
    }
    /// Explicit host opt-in to version two. Listing consent must then name one
    /// of the three closed source forms. This does not grant source access.
    #[must_use]
    pub fn with_source_forms(mut self) -> Self {
        self.source_forms = true;
        self
    }
    fn protocol(&self) -> protocol::PurchaseProtocol {
        if self.source_forms {
            protocol::PurchaseProtocol::SourceFormsV2
        } else {
            protocol::PurchaseProtocol::SealedV1
        }
    }
    fn policy_digest(&self) -> Result<DefinitionDigest, String> {
        if self.source_forms {
            records::digest(
                "market.original_quote_policy.v2",
                &json!({"policy":self.policy,"source_forms":["sealed","open","open_resale"]}),
                8192,
            )
        } else {
            self.policy.digest()
        }
    }
    pub fn listing_action(&self) -> Result<DefinitionRef, ModError> {
        self.action_ref(LIST_ACTION)
    }
    pub fn quote_action(&self) -> Result<DefinitionRef, ModError> {
        self.action_ref(QUOTE_ACTION)
    }
    pub fn retire_listing_action(&self) -> Result<DefinitionRef, ModError> {
        self.action_ref(RETIRE_LISTING_ACTION)
    }
    pub fn retire_quote_action(&self) -> Result<DefinitionRef, ModError> {
        self.action_ref(RETIRE_QUOTE_ACTION)
    }
    fn action_ref(&self, id: &str) -> Result<DefinitionRef, ModError> {
        DefinitionRegistryBuilder::new()
            .register(OwnerId::new(crate::NS).map_err(error)?, self.action(id)?)
            .map_err(error)
    }
    fn action(&self, id: &str) -> Result<DefinitionDescriptor, ModError> {
        if matches!(
            id,
            RETIRE_LISTING_ACTION | RETIRE_QUOTE_ACTION | EXPIRE_ACTION
        ) {
            return descriptor(
                "action",
                id,
                json!({"format":1,"unfunded_only":true,
                "subject":"self_actor","targets":0,"funding":false,
                "authority":if id == EXPIRE_ACTION {"god_expired_only"} else {"actual_offer_owner"},
                "batch":if id == EXPIRE_ACTION {index::RETIRE_BATCH} else {1}}),
            )
            .map(|d| d.with_references(vec![self.manifest.clone()]));
        }
        let payload = if self.source_forms {
            json!({"format":2,"original_only":true,"personal_only":true,
            "subject":"self_actor","targets":0,"funding":false,"terms":self.policy,
            "product":{"source_forms":["sealed","open","open_resale"],"explicit_listing_consent":true,"seats":1,"revision":"pin","class":"certified_minimum"}})
        } else {
            json!({"format":1,"original_only":true,"personal_only":true,
            "subject":"self_actor","targets":0,"funding":false,"terms":self.policy,
            "product":{"source":"sealed","seats":1,"revision":"pin","class":"certified_minimum"}})
        };
        versioned_descriptor("action", id, self.protocol().version(), payload)
            .map(|d| d.with_references(vec![self.manifest.clone()]))
    }
    pub(crate) fn declarations(&self) -> Result<Vec<DefinitionDescriptor>, ModError> {
        Ok(vec![
            self.action(LIST_ACTION)?,
            self.action(QUOTE_ACTION)?,
            self.action(RETIRE_LISTING_ACTION)?,
            self.action(RETIRE_QUOTE_ACTION)?,
            self.action(EXPIRE_ACTION)?,
            descriptor(
                "outcome",
                OUTCOME,
                json!({"format":1,"fields":["kind","publication"],"funded":false}),
            )?,
        ])
    }
}

#[derive(Clone)]
pub(crate) struct Families {
    listings: StateFamilyHandle<CommitToken, MindListing>,
    quotes: StateFamilyHandle<CommitToken, MindQuote>,
    indices: index::Indices,
    // One owner-issued grant shared by all Market consumers in this composition.
    reader: mush_core::read_service::OwnerReadHandle<
        mush_roll::commerce::sale_reader::OriginalSaleReader,
    >,
}
impl Families {
    pub(crate) fn resale_quote_for_boot(
        &self,
        ctx: &mush_core::ModuleCtx<'_>,
        id: CommitToken,
    ) -> Result<MindQuote, String> {
        let row = ctx
            .action_state(&self.quotes, &id)
            .map_err(|_| "retained quote unavailable")?
            .cloned()
            .ok_or("retained quote missing")?;
        if row.publication() != id {
            return Err("retained quote identity differs".into());
        }
        row.resale_rates()?;
        Ok(row)
    }
    pub(crate) fn resale_quote(
        &self,
        txn: &mut mush_core::ActionTxn<'_>,
        id: CommitToken,
    ) -> Result<MindQuote, String> {
        let row = txn
            .get(&self.quotes, &id)
            .map_err(|_| "retained quote unavailable")?
            .cloned()
            .ok_or("retained quote missing")?;
        if row.publication() != id {
            return Err("retained quote identity differs".into());
        }
        row.resale_rates()?;
        Ok(row)
    }
    pub(crate) fn audit(&self, ctx: &mush_core::ModuleCtx<'_>) -> Result<(), ModError> {
        index::audit(ctx, self)
    }
}
fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
fn descriptor(kind: &str, id: &str, payload: Value) -> Result<DefinitionDescriptor, ModError> {
    versioned_descriptor(kind, id, 1, payload)
}
fn versioned_descriptor(
    kind: &str,
    id: &str,
    version: u32,
    payload: Value,
) -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new(kind).map_err(error)?,
        DefinitionId::new(id).map_err(error)?,
        version,
        payload,
    )
    .map_err(error)
}
fn family<R: Row>(reg: &mut Registrar<'_>) -> Result<StateFamilyHandle<CommitToken, R>, ModError> {
    let contract =
        |suffix: &str| CanonicalId::new(format!("{}.{}.v1", R::FAMILY, suffix)).map_err(error);
    let d = StateFamilyDescriptor::new(
        StateFamilyId::new(R::FAMILY).map_err(error)?,
        contract("key")?,
        contract("value")?,
        1,
        contract("decode")?,
        contract("validate")?,
        contract("canonical")?,
    )
    .and_then(|d| d.with_limits(32, R::MAX_BYTES))
    .map_err(error)?;
    reg.state_family(
        d,
        |k: &CommitToken| Ok(k.to_hex()),
        records::decode_value::<R>,
        R::validate,
        |r: &R| serde_json::to_value(r).map_err(|e| e.to_string()),
    )
}
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    config: &OriginalQuoteConfig,
    gate: Arc<dyn crate::VenueGate>,
    validators_registered: bool,
) -> Result<Families, ModError> {
    register_with_purchase_definitions(reg, config, gate, validators_registered, &[])
}
pub(crate) fn register_with_purchase_definitions(
    reg: &mut Registrar<'_>,
    config: &OriginalQuoteConfig,
    gate: Arc<dyn crate::VenueGate>,
    validators_registered: bool,
    extra: &[DefinitionDescriptor],
) -> Result<Families, ModError> {
    if reg.module() != crate::NS {
        return Err(ModError::msg("original quotes are Market owned"));
    }
    let mut declarations = config.declarations()?;
    declarations.extend_from_slice(extra);
    if !validators_registered {
        for kind in ["action", "outcome"] {
            let matching: Vec<_> = declarations
                .iter()
                .filter(|d| d.kind().as_str() == kind)
                .cloned()
                .collect();
            reg.definition_validator(
                DefinitionKind::new(kind).map_err(error)?,
                crate::lot::exact_validator(&matching),
            )?;
        }
    }
    let families = Families {
        listings: family(reg)?,
        quotes: family(reg)?,
        indices: index::register(reg)?,
        reader: mush_roll::commerce::sale_reader::OriginalSaleReader::request(
            reg,
            &config.manifest,
        )?,
    };
    let outcome = reg.definition(descriptor(
        "outcome",
        OUTCOME,
        json!({"format":1,"fields":["kind","publication"],"funded":false}),
    )?)?;
    for (id, kind) in [
        (LIST_ACTION, provider::Kind::List),
        (QUOTE_ACTION, provider::Kind::Quote),
        (RETIRE_LISTING_ACTION, provider::Kind::RetireListing),
        (RETIRE_QUOTE_ACTION, provider::Kind::RetireQuote),
        (EXPIRE_ACTION, provider::Kind::Expire),
    ] {
        let action = reg.definition(config.action(id)?)?;
        reg.action_provider(
            action.clone(),
            provider::Provider::new(
                action,
                outcome.clone(),
                kind,
                config.clone(),
                families.clone(),
                families.reader.clone(),
                gate.clone(),
            )?,
        )?;
    }
    reg.query_handler(
        QUERY,
        Arc::new(query::Offers::new(config.clone(), families.clone(), gate)),
    )?;
    let action = config.action_ref(EXPIRE_ACTION)?;
    let tick_families = families.clone();
    reg.tick(
        EXPIRY_TICK,
        mush_core::TickCadence::Coarse(std::time::Duration::from_secs(5)),
        Arc::new(
            move |ctx: &mut mush_core::Ctx<'_>, _: &str, _: Option<mush_core::SceneId>| {
                query::expire_tick(ctx, &tick_families, &action);
            },
        ),
    )?;
    Ok(families)
}

#[cfg(test)]
mod tests;
