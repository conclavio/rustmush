//! **Heirloom and grey dies** (doc 17h cut M8b-2, H1 and H3; doc 17 §10, D-UM-42).
//!
//! Two die sources beside the importer's `certified_core_die.v1`, issued only by staff: a
//! **heirloom** (`enclave_heirloom.v1`, the grade word `die.h`, always Class III), a lot whose
//! lineage is `Lineage::Heirloom` and names no importer; and a **grey** die, a lot of a declared
//! class whose lineage is `Lineage::Grey` (no origin recorded at all). Neither is certified,
//! neither counts against an importer's monthly quota, no vendor sells either (doc 17h §4), and
//! either integrates only at the ungoverning site (`CorePaymentPolicy::site`). This family is
//! their issue history: an issuance proof, never custody or consumption authority.
use super::records::{DieClass, lot_id};
use super::*;
use mush_core::action::{ActionError, ActionErrorCode, ActionProvider, AuthoritativeActionOutput};
use mush_core::mechanics::{DefinitionDescriptor, RulesValue};
use mush_core::read_service::OwnerReadHandle;
use mush_core::{
    ActionInvocation, ActionTxn, ClockRef, ClockStamp, CommitToken, DefinitionDigest, ModuleCtx,
    ObjId, PerceivedRef, PrepareCtx, Registrar, StateFamilyHandle,
};
use mush_economy::core_integration::DieSource;
use mush_economy::{ExternalRef, VaultId, VaultStatus};
use mush_inventory::{EndpointRef, Grade, InventoryFamilies, InventoryStore, Lineage, Lot, LotId};
use serde::{Deserialize, Serialize};
use serde_json::json;

/// Staff issue of one heirloom die (H3).
pub const ISSUE_HEIRLOOM_ACTION: &str = "issue_heirloom_die";
/// Staff issue of one grey die of a declared class (H3).
pub const ISSUE_GREY_ACTION: &str = "issue_grey_die";
/// The heirloom lot's source kind.
pub(crate) const HEIRLOOM_KIND: &str = "enclave_heirloom.v1";
/// The heirloom's grade word, beside the pinned `die.c1/c2/c3`.
pub(crate) const HEIRLOOM_LABEL: &str = "die.h";
pub(crate) const RECORDS: &str = "industry_core_die_uncertified";
pub(crate) const INDEX: &str = "industry_core_die_uncertified_index";

/// The two issue actions, in [`DieSource`] order (heirloom, grey).
pub(crate) const ACTIONS: [(&str, DieSource); 2] = [
    (ISSUE_HEIRLOOM_ACTION, DieSource::Heirloom),
    (ISSUE_GREY_ACTION, DieSource::Grey),
];

/// The issue action's fixed descriptor, referencing the die stock content.
pub(crate) fn action(
    policy: &OriginPolicy,
    source: DieSource,
) -> Result<DefinitionDescriptor, ModError> {
    let (name, word) = match source {
        DieSource::Heirloom => (ISSUE_HEIRLOOM_ACTION, "heirloom"),
        DieSource::Grey => (ISSUE_GREY_ACTION, "grey"),
        DieSource::Certified => return Err(mod_error(Error::Invalid)),
    };
    config::descriptor(
        "action",
        name,
        json!({"format":1,"protocol":"uncertified_core_die_v1","source":word,
            "commodity":"tacsim.electronics","quantity":1,"authority":"god",
            "content":policy.content}),
    )
    .map(|d| d.with_references(vec![policy.content.clone()]))
}

/// **One staff-issued heirloom or grey die** (H1, H3): the issuing operation (the funding
/// subject's `die_certificate` word and the lot's derivation), its source and class, the lot
/// minted and where. Public as an opaque type so a granted reader ([`UncertifiedDies`]) reads
/// the record's words through its accessors.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UncertifiedDie {
    pub(crate) format: u16,
    pub(crate) sequence: u64,
    pub(crate) operation: CommitToken,
    pub(crate) action: DefinitionRef,
    pub(crate) agent: ObjId,
    pub(crate) source: DieSource,
    pub(crate) class: DieClass,
    pub(crate) label: CanonicalId,
    pub(crate) lot: LotId,
    pub(crate) commodity: CanonicalId,
    pub(crate) grade: Grade,
    pub(crate) endpoint: EndpointRef,
    pub(crate) observed_ic: ClockStamp,
    pub(crate) observed_utc: i64,
}
impl UncertifiedDie {
    /// The issuing operation: the record's key and the die lot's derivation.
    #[must_use]
    pub fn operation(&self) -> CommitToken {
        self.operation
    }
    /// `Heirloom` or `Grey`.
    #[must_use]
    pub fn source(&self) -> DieSource {
        self.source
    }
    /// The die's class (a heirloom is always Class III).
    #[must_use]
    pub fn class(&self) -> DieClass {
        self.class
    }
    /// The die lot the issue minted.
    #[must_use]
    pub fn lot(&self) -> LotId {
        self.lot
    }
    /// The record's key: the die lot (a grey lot names no issue, so the lot is the join).
    pub(crate) fn key(&self) -> String {
        UncertifiedDies::key(self.lot)
    }
    /// The lot's lineage: `Heirloom` naming this issue, or `Grey` naming nothing. A
    /// certified source is never an issue of this family.
    pub(crate) fn lineage(&self) -> Result<Lineage, Error> {
        match self.source {
            DieSource::Heirloom => Ok(Lineage::Heirloom {
                source: source(self.operation),
            }),
            DieSource::Grey => Ok(Lineage::Grey {}),
            DieSource::Certified => Err(Error::Invalid),
        }
    }
    pub(crate) fn validate(&self) -> Result<(), Error> {
        let (name, label) = match self.source {
            DieSource::Heirloom => (ISSUE_HEIRLOOM_ACTION, HEIRLOOM_LABEL),
            DieSource::Grey => (ISSUE_GREY_ACTION, self.class.label()),
            DieSource::Certified => return Err(Error::Invalid),
        };
        if self.format != 1
            || !(1..=MAX_CERTIFICATES).contains(&self.sequence)
            || !fixed(&self.action, "action", name)
            || self.agent != mush_core::GOD
            || (self.source == DieSource::Heirloom && self.class != DieClass::III)
            || self.label.as_str() != label
            || self.commodity.as_str() != "tacsim.electronics"
            || self.lot != lot_id(self.operation)?
            || !matches!(self.observed_ic.clock, ClockRef::WorldIc { generation } if generation > 0)
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
    pub(crate) fn matches_lot(&self, lot: &Lot) -> Result<(), Error> {
        self.validate()?;
        if lot.id() != self.lot
            || lot.quantity() != 1
            || lot.commodity() != &self.commodity
            || lot.grade() != self.grade
            || lot.lineage() != &self.lineage()?
            || lot.depth() != 0
        {
            return Err(Error::Integrity);
        }
        Ok(())
    }
}
/// The heirloom lot's lineage source: this issue, under no importer.
pub(crate) fn source(operation: CommitToken) -> ExternalRef {
    ExternalRef {
        provider: crate::NS.into(),
        kind: HEIRLOOM_KIND.into(),
        id: operation.to_hex().into(),
        revision: Some(1),
    }
}

/// The issue census: how many were issued and the chain over them, so whole loss of the
/// record family is detectable at boot. Absent until the first issue.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UncertifiedIndex {
    pub(crate) format: u16,
    pub(crate) issued: u64,
    pub(crate) chain: DefinitionDigest,
}
impl UncertifiedIndex {
    pub(crate) fn genesis() -> Result<Self, Error> {
        Ok(Self {
            format: 1,
            issued: 0,
            chain: DefinitionDigest::for_canonical_value(
                "industry.core_die_uncertified_genesis.v1",
                &json!({"format":1}),
            )
            .map_err(bad)?,
        })
    }
    pub(crate) fn validate(&self) -> Result<(), Error> {
        if self.format != 1
            || self.issued == 0
            || self.issued > MAX_CERTIFICATES
            || self.chain.as_bytes() == &[0; 32]
        {
            return Err(Error::Integrity);
        }
        Ok(())
    }
    pub(crate) fn issue(&self, die: &UncertifiedDie) -> Result<Self, Error> {
        die.validate()?;
        if die.sequence != self.issued.checked_add(1).ok_or(Error::Capacity)? {
            return Err(Error::Integrity);
        }
        let chain = DefinitionDigest::for_canonical_value(
            "industry.core_die_uncertified_chain.v1",
            &json!({"prior":self.chain,"die":encode(die).map_err(bad)?}),
        )
        .map_err(bad)?;
        let next = Self {
            format: 1,
            issued: die.sequence,
            chain,
        };
        next.validate()?;
        Ok(next)
    }
}

/// **The retained heirloom and grey issues, for a granted reader** (doc 17h cut M8b-2, H4):
/// published beside [`DieCertificates`] when the composition names a certificate reader,
/// which is granted this family too. Data, not custody or consumption authority.
#[derive(Clone)]
pub struct UncertifiedDies(pub(crate) StateFamilyHandle<String, UncertifiedDie>);
impl UncertifiedDies {
    /// The family handle, keyed by [`Self::key`] of the die lot.
    #[must_use]
    pub fn handle(&self) -> &StateFamilyHandle<String, UncertifiedDie> {
        &self.0
    }
    /// The record key of a die lot: its decimal word (a state key admits no `#`).
    #[must_use]
    pub fn key(lot: LotId) -> String {
        lot.0.to_string()
    }
}

#[derive(Clone)]
pub(crate) struct Families {
    pub(crate) records: StateFamilyHandle<String, UncertifiedDie>,
    pub(crate) index: StateFamilyHandle<(), UncertifiedIndex>,
}
pub(crate) fn register_families(
    reg: &mut Registrar<'_>,
    reader: Option<&str>,
    family: impl Fn(&str) -> Result<mush_core::StateFamilyDescriptor, ModError>,
) -> Result<Families, ModError> {
    let records = reg.state_family(
        family(RECORDS)?,
        |s: &String| Ok(s.clone()),
        decode::<UncertifiedDie>,
        |r: &UncertifiedDie| r.validate().map_err(|e| e.to_string()),
        encode::<UncertifiedDie>,
    )?;
    let index = reg.state_family(
        family(INDEX)?,
        |_: &()| Ok("registry".into()),
        decode::<UncertifiedIndex>,
        |r: &UncertifiedIndex| r.validate().map_err(|e| e.to_string()),
        encode::<UncertifiedIndex>,
    )?;
    if let Some(reader) = reader {
        reg.grant_state_family_read(&records, reader)?;
        reg.state(UncertifiedDies(records.clone()));
    }
    Ok(Families { records, index })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HeirloomRequest {
    vault: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GreyRequest {
    class: DieClass,
    vault: u64,
}
/// The requested class and vault: a heirloom is always Class III.
fn parse(inv: &ActionInvocation, source: DieSource) -> Result<(DieClass, VaultId), Error> {
    if inv.principal() != &(mush_core::action::PrincipalId::Object { id: inv.actor() })
        || inv.subject() != &PerceivedRef::SelfActor
        || !inv.targets().is_empty()
        || inv.idempotency().scope().owner.as_str() != crate::NS
    {
        return Err(Error::Invalid);
    }
    let (class, vault) = match source {
        DieSource::Heirloom => {
            let r: HeirloomRequest =
                serde_json::from_value(inv.parameters().clone()).map_err(bad)?;
            (DieClass::III, r.vault)
        }
        DieSource::Grey => {
            let r: GreyRequest = serde_json::from_value(inv.parameters().clone()).map_err(bad)?;
            (r.class, r.vault)
        }
        DieSource::Certified => return Err(Error::Invalid),
    };
    if vault == 0 {
        return Err(Error::Invalid);
    }
    Ok((class, VaultId(vault)))
}

pub(crate) struct Provider {
    pub(crate) source: DieSource,
    pub(crate) action: DefinitionRef,
    pub(crate) outcome: DefinitionRef,
    pub(crate) policy: OriginPolicy,
    pub(crate) mode: CoreOriginMode,
    pub(crate) families: Families,
    pub(crate) inventory: InventoryFamilies,
    pub(crate) readiness: OwnerReadHandle<super::participant::Readiness>,
    pub(crate) refusal: ActionError,
}
impl Provider {
    /// The live vault the die is minted into: active, live and not quarantined.
    fn vault(world: &mush_core::World, vault: VaultId) -> Result<EndpointRef, Error> {
        let economy = world
            .module_state::<mush_economy::EconomyStore>()
            .ok_or(Error::Unavailable)?;
        let row = economy.snapshot().vault(vault).ok_or(Error::Unavailable)?;
        let inventory = world
            .module_state::<InventoryStore>()
            .ok_or(Error::Unavailable)?;
        if economy.is_frozen()
            || row.status != VaultStatus::Active
            || !mush_economy::endpoint_state(world, vault).is_live()
            || !inventory.is_complete()
            || inventory.is_quarantined(EndpointRef(vault))
        {
            return Err(Error::Unavailable);
        }
        Ok(EndpointRef(vault))
    }
    fn prepare(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, Error> {
        let (class, vault) = parse(inv, self.source)?;
        // H3: GOD-only, on an active origin, outside every importer's monthly quota.
        if inv.actor() != mush_core::GOD || self.mode != CoreOriginMode::Active {
            return Err(Error::Unavailable);
        }
        ctx.read_owner_preimage(txn, &self.readiness, ())
            .map_err(bad)?;
        let world = ctx.world().map_err(bad)?;
        let endpoint = Self::vault(world, vault)?;
        let observed_ic = world.world_ic_now();
        let observed_utc = world.now();
        let before = txn.get(&self.families.index, &()).map_err(bad)?.cloned();
        let index = match &before {
            Some(index) => {
                index.validate()?;
                index.clone()
            }
            None => UncertifiedIndex::genesis()?,
        };
        let operation = ctx.commit_token();
        if index.issued >= MAX_CERTIFICATES
            || txn
                .get(
                    &self.families.records,
                    &UncertifiedDies::key(lot_id(operation)?),
                )
                .map_err(bad)?
                .is_some()
        {
            return Err(Error::Capacity);
        }
        let die = UncertifiedDie {
            format: 1,
            sequence: index.issued.checked_add(1).ok_or(Error::Capacity)?,
            operation,
            action: self.action.clone(),
            agent: inv.actor(),
            source: self.source,
            class,
            label: id(if self.source == DieSource::Heirloom {
                HEIRLOOM_LABEL
            } else {
                class.label()
            })?,
            lot: lot_id(operation)?,
            commodity: id("tacsim.electronics")?,
            grade: self.policy.grades[class.index()],
            endpoint,
            observed_ic,
            observed_utc,
        };
        let after = index.issue(&die)?;
        let lot = self
            .inventory
            .mint_root(
                txn,
                die.endpoint,
                &die.commodity,
                1,
                die.grade,
                die.lineage()?,
                die.lot,
            )
            .map_err(bad)?;
        die.matches_lot(&lot)?;
        txn.insert(&self.families.records, &die.key(), die.clone())
            .map_err(bad)?;
        if before.is_some() {
            txn.replace(&self.families.index, &(), after).map_err(bad)?;
        } else {
            txn.insert(&self.families.index, &(), after).map_err(bad)?;
        }
        Ok(AuthoritativeActionOutput::new(
            RulesValue::new(
                self.outcome.clone(),
                json!({"operation":operation,"lot":die.lot,"class":class}),
            )
            .map_err(bad)?,
        )
        .with_definitions(vec![
            self.action.clone(),
            self.outcome.clone(),
            self.policy.content.clone(),
        ])
        .with_eligible_witnesses(vec![inv.principal().clone()]))
    }
}
impl ActionProvider for Provider {
    fn validate_invocation(&self, inv: &ActionInvocation) -> Result<(), ActionError> {
        parse(inv, self.source)
            .map(|_| ())
            .map_err(|_| self.refusal.clone())
    }
    fn prepare_authoritative_with_context(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        self.prepare(inv, txn, ctx)
            .map_err(|_| self.refusal.clone())
    }
    /// Offered to GOD alone, on an active and initialized origin (H3); a staff surface.
    fn project_offer(
        &self,
        action: &DefinitionRef,
        q: &mush_core::LegalActionQuery,
        engine: &mush_core::Engine,
    ) -> Result<Option<mush_core::ProviderOfferDraft>, ActionError> {
        if action != &self.action
            || q.actor() != mush_core::GOD
            || q.truth_subject() != &(mush_core::SubjectKey::Object { id: q.actor() })
            || q.projected_subject() != &PerceivedRef::SelfActor
            || self.mode != CoreOriginMode::Active
            || engine
                .world
                .module_state::<super::participant::CoreProductionState>()
                .is_none_or(|s| !s.ready || s.index.is_none())
        {
            return Ok(None);
        }
        let fail = || self.refusal.clone();
        let defs = engine.registry.definitions().ok_or_else(fail)?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(mush_core::RevisionSet::default())
            .map_err(|_| fail())?;
        guards
            .add_definition(action.clone(), defs)
            .map_err(|_| fail())?;
        guards
            .add_definition(self.policy.content.clone(), defs)
            .map_err(|_| fail())?;
        let mut parameters = vec![mush_core::ProjectedParameter {
            id: CanonicalId::new("vault").map_err(|_| fail())?,
            required: true,
            kind: mush_core::ProjectedParameterKind::Integer {
                minimum: 1,
                maximum: i64::MAX,
            },
        }];
        let (id, label, help) = match self.source {
            DieSource::Heirloom => (
                "issue-heirloom-die",
                "Issue a heirloom core die",
                "Mint one enclave heirloom die (Class III, no certificate) into a vault.",
            ),
            DieSource::Certified => return Ok(None),
            DieSource::Grey => {
                parameters.push(mush_core::ProjectedParameter {
                    id: CanonicalId::new("class").map_err(|_| fail())?,
                    required: true,
                    kind: mush_core::ProjectedParameterKind::Choice {
                        choices: DieClass::ALL
                            .into_iter()
                            .map(|class| {
                                serde_json::to_value(class)
                                    .map_err(|_| fail())?
                                    .as_str()
                                    .map(str::to_owned)
                                    .ok_or_else(fail)
                            })
                            .collect::<Result<Vec<_>, _>>()?,
                    },
                });
                (
                    "issue-grey-die",
                    "Issue a grey core die",
                    "Mint one die of the class with no origin recorded into a vault.",
                )
            }
        };
        Ok(Some(mush_core::ProviderOfferDraft {
            fixed_parameters: json!({}),
            parameters: mush_core::ProjectedParameterSchema::new(parameters).map_err(|_| fail())?,
            targets: mush_core::ProjectedTargetSchema::None,
            required_channels: vec![],
            expires: None,
            presentation: mush_core::ActionPresentation::new(id, label, help)
                .map_err(|_| fail())?,
            guards: guards.finish().map_err(|_| fail())?,
        }))
    }
}

/// The issue actions' refusal: the certified issue's own sentence, so a refused staff issue
/// names nothing new.
pub(crate) fn refusal() -> Result<ActionError, ModError> {
    ActionError::new(
        ActionErrorCode::ProviderRejected,
        false,
        "Certified core die stock is unavailable or changed.",
    )
    .map_err(mod_error)
}

/// **Boot audit of the issue records** (H6): every record valid under its key, one per lot,
/// the census equal to the record count and its chain, and every present lot of an issue
/// matching it. Returns the issues by lot for the origin audit's one lot scan
/// ([`check_lot`]; fix round 1, L4).
pub(crate) fn audit_records(
    ctx: &ModuleCtx<'_>,
    families: &Families,
    inventory: &InventoryFamilies,
) -> Result<std::collections::BTreeMap<LotId, UncertifiedDie>, Error> {
    let mut dies = std::collections::BTreeMap::new();
    let mut lots = std::collections::BTreeMap::new();
    let mut after = None;
    loop {
        let page = ctx
            .scan_action_state(&families.records, after.as_deref(), 128)
            .map_err(bad)?;
        for entry in page.entries() {
            let d = entry.value();
            d.validate()?;
            if entry.encoded_key() != d.key()
                || dies.insert(d.sequence, d.clone()).is_some()
                || lots.insert(d.lot, d.clone()).is_some()
                || dies.len() as u64 > MAX_CERTIFICATES
            {
                return Err(Error::Integrity);
            }
            if let Some(lot) = ctx.action_state(&inventory.lots, &d.lot).map_err(bad)? {
                d.matches_lot(lot)?;
            }
        }
        after = page.next_after().map(|cursor| cursor.to_string());
        if after.is_none() {
            break;
        }
    }
    let typed = ctx.action_state(&families.index, &()).map_err(bad)?;
    match typed {
        None if dies.is_empty() => {}
        Some(index) => {
            index.validate()?;
            let mut expected = UncertifiedIndex::genesis()?;
            for d in dies.values() {
                expected = expected.issue(d)?;
            }
            if &expected != index {
                return Err(Error::Integrity);
            }
        }
        None => return Err(Error::Integrity),
    }
    Ok(lots)
}

/// **One present lot against the issues** (fix round 1, L4): a heirloom lot in this crate's
/// namespace, and every grey lot (no other minter makes one), must be an issue's own lot.
pub(crate) fn check_lot(
    issues: &std::collections::BTreeMap<LotId, UncertifiedDie>,
    lot: &Lot,
) -> Result<(), Error> {
    let issued = match lot.lineage() {
        Lineage::Heirloom { source } => {
            source.provider.as_str() == crate::NS && source.kind.as_str() == HEIRLOOM_KIND
        }
        Lineage::Grey {} => true,
        Lineage::Origin { .. }
        | Lineage::Split { .. }
        | Lineage::Merge { .. }
        | Lineage::Tranche { .. } => false,
    };
    if issued {
        issues
            .get(&lot.id())
            .ok_or(Error::Integrity)?
            .matches_lot(lot)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    //! Doc 17h cut M8b-2 (H1, H3): the record's invariants, its lot's lineage and the census.
    use super::*;
    use mush_core::{DefinitionKind, DefinitionRegistryBuilder, OwnerId};
    type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

    fn action(name: &str) -> Result<DefinitionRef> {
        Ok(DefinitionRegistryBuilder::new().register(
            OwnerId::new(crate::NS)?,
            DefinitionDescriptor::new(
                DefinitionKind::new("action")?,
                mush_core::DefinitionId::new(name)?,
                1,
                json!({"fixture": name}),
            )?,
        )?)
    }
    fn die(source: DieSource, class: DieClass, sequence: u64) -> Result<UncertifiedDie> {
        let operation = CommitToken::from_bytes([u8::try_from(sequence)?; 16]);
        let (name, label) = match source {
            DieSource::Heirloom => (ISSUE_HEIRLOOM_ACTION, HEIRLOOM_LABEL),
            _ => (ISSUE_GREY_ACTION, class.label()),
        };
        Ok(UncertifiedDie {
            format: 1,
            sequence,
            operation,
            action: action(name)?,
            agent: mush_core::GOD,
            source,
            class,
            label: id(label)?,
            lot: lot_id(operation)?,
            commodity: id("tacsim.electronics")?,
            grade: Grade::new(30)?,
            endpoint: EndpointRef(VaultId(7)),
            observed_ic: ClockStamp {
                clock: ClockRef::WorldIc { generation: 1 },
                at: 1_000,
            },
            observed_utc: 1_000,
        })
    }

    #[test]
    fn m8b2_a_heirloom_is_a_class_three_die_h_and_a_grey_die_its_class_label() -> Result {
        let heirloom = die(DieSource::Heirloom, DieClass::III, 1)?;
        check!(heirloom.validate().is_ok());
        let grey = die(DieSource::Grey, DieClass::I, 2)?;
        check!(grey.validate().is_ok());
        let mut wrong = [
            heirloom.clone(),
            heirloom.clone(),
            grey.clone(),
            grey.clone(),
        ];
        wrong[0].class = DieClass::II;
        wrong[1].label = id("die.c3")?;
        wrong[2].label = id(HEIRLOOM_LABEL)?;
        wrong[3].source = DieSource::Certified;
        for d in &wrong {
            check!(d.validate().is_err());
        }
        // The lot's lineage: the heirloom names its issue under no importer; grey names none.
        check_eq!(
            heirloom.lineage()?,
            Lineage::Heirloom {
                source: ExternalRef {
                    provider: crate::NS.into(),
                    kind: HEIRLOOM_KIND.into(),
                    id: heirloom.operation.to_hex().into(),
                    revision: Some(1),
                }
            }
        );
        check_eq!(grey.lineage()?, Lineage::Grey {});
        let lot = Lot::new(
            grey.lot,
            grey.endpoint,
            grey.commodity.clone(),
            1,
            grey.grade,
            Lineage::Grey {},
            0,
        )?;
        check!(grey.matches_lot(&lot).is_ok());
        check!(heirloom.matches_lot(&lot).is_err());
        Ok(())
    }

    #[test]
    fn m8b2_the_census_chains_each_issue_in_order() -> Result {
        let first = die(DieSource::Heirloom, DieClass::III, 1)?;
        let second = die(DieSource::Grey, DieClass::II, 2)?;
        let genesis = UncertifiedIndex::genesis()?;
        check!(genesis.issue(&second).is_err(), "out of order");
        let one = genesis.issue(&first)?;
        let two = one.issue(&second)?;
        check_eq!(two.issued, 2);
        check!(two.chain != one.chain && one.chain != genesis.chain);
        check!(two.validate().is_ok());
        Ok(())
    }
}
