//! Exact six-target completion for the closed original house purchase.
//! An organization is the issuer; a curator is never a personal seller.
//! Actual money, account authority and source-root proofs remain owner checks.
use super::*;
use crate::action::{ActionProviderRegistry, PrincipalId};
use crate::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistry,
    DefinitionRegistryBuilder,
};

const HOUSE_ACTIONS: [&str; 3] = [
    "purchase_house_mind",
    "settle_house_mind",
    "refund_house_mind",
];
const TRANSITIONS: [&str; 3] = ["fund", "grant", "refund"];
const PURCHASE: &str = "market_mind_house_purchase";
const RECEIPT: &str = "market_mind_house_receipt";
const ALLOCATION: &str = "roll_mind_house_purchase";
const LICENCE: &str = "roll_mind_personal_paid_licence";
const TERM: &str = "roll_mind_paid_term";
const ECONOMICS: &str = "roll_mind_house_term_economics";
const FAMILIES: [(&str, &str, usize); 6] = [
    ("market", PURCHASE, 32768),
    ("market", RECEIPT, 8192),
    ("roll", ALLOCATION, 8192),
    ("roll", LICENCE, 4096),
    ("roll", TERM, 8192),
    ("roll", ECONOMICS, 8192),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Kind {
    House,
}
impl Kind {
    fn version(self) -> u32 {
        1
    }
    fn wire(self) -> &'static str {
        "house_v1"
    }
    fn protocol(self) -> &'static str {
        "house_mind_purchase_v1"
    }
    fn actions(self) -> [&'static str; 3] {
        HOUSE_ACTIONS
    }
    fn product_domain(self) -> &'static str {
        "market.house_product.v1"
    }
    fn quote_domain(self) -> &'static str {
        "market.house_quote.v1"
    }
}

#[derive(Clone, Debug, Serialize)]
pub(in crate::action_state) struct Protocol {
    manifest: DefinitionRef,
    participants: [DefinitionRef; 2],
    house: [DefinitionRef; 3],
}
impl Protocol {
    pub(in crate::action_state) fn new(
        owner: &OwnerId,
        manifest: DefinitionRef,
        participants: [DefinitionRef; 2],
        house: [DefinitionRef; 3],
    ) -> Result<Self, ActionStateError> {
        if owner.as_str() != "market"
            || manifest.key().owner.as_str() != "tacsim"
            || manifest.key().kind.as_str() != "registry_manifest"
            || manifest.key().id.as_str() != "combat"
            || manifest.version() != 1
        {
            return Err(refused("house declaration owner or manifest"));
        }
        check_participants(&participants)?;
        let result = Self {
            manifest,
            participants,
            house,
        };
        for kind in [Kind::House] {
            for (action, id) in result.actions(kind).iter().zip(kind.actions()) {
                if action.key().owner != *owner
                    || action.key().kind.as_str() != "action"
                    || action.key().id.as_str() != id
                    || action.version() != kind.version()
                {
                    return Err(refused("house action identity or order"));
                }
            }
            if *result.admission_action(kind)
                != admission_reference(&result.manifest, &result.participants, kind)?
            {
                return Err(refused("house admission definition differs"));
            }
        }
        Ok(result)
    }
    fn actions(&self, kind: Kind) -> &[DefinitionRef; 3] {
        match kind {
            Kind::House => &self.house,
        }
    }
    fn admission_action(&self, kind: Kind) -> &DefinitionRef {
        &self.actions(kind)[0]
    }
    pub(super) fn admission_actions(&self) -> [&DefinitionRef; 1] {
        [self.admission_action(Kind::House)]
    }
    pub(super) fn recognizes_historical(&self, effect: &DefinitionRef) -> bool {
        [Kind::House].into_iter().any(|kind| {
            effect.key() == self.admission_action(kind).key() && effect.version() == kind.version()
        })
    }
    fn action_index(
        &self,
        identity: &RegisteredTxnIdentity,
    ) -> Result<(Kind, usize), ActionStateError> {
        for kind in [Kind::House] {
            if let Some(index) = self
                .actions(kind)
                .iter()
                .position(|action| action == &identity.action)
            {
                return Ok((kind, index));
            }
        }
        Err(refused("house invocation action differs"))
    }
    pub(in crate::action_state) fn validate_families(
        &self,
        registry: &StateFamilyRegistry,
    ) -> Result<(), ActionStateError> {
        for (owner, family, bytes) in FAMILIES.into_iter().chain([
            ("roll", "roll_mind_commerce", 4096),
            ("market", "market_mind_house_quote", 24576),
        ]) {
            let id = StateFamilyId::new(family).map_err(|_| refused("house family"))?;
            let canonical = |suffix: &str| {
                CanonicalId::new(format!("{family}.{suffix}.v1"))
                    .map_err(|_| refused("house schema"))
            };
            let (decode, validate, key_bytes) = if owner == "roll" {
                ("strict_decode", "invariants", 192)
            } else {
                ("decode", "validate", 32)
            };
            let expected = StateFamilyDescriptor::new(
                id.clone(),
                canonical("key")?,
                canonical("value")?,
                1,
                canonical(decode)?,
                canonical(validate)?,
                canonical("canonical")?,
            )
            .and_then(|row| row.with_limits(key_bytes, bytes))
            .map_err(|_| refused("house descriptor"))?;
            let owner = OwnerId::new(owner).map_err(|_| refused("house owner"))?;
            let actual = registry
                .entries
                .get(&(owner.clone(), id.clone()))
                .ok_or_else(|| refused("house family unavailable"))?;
            if actual.descriptor().canonical_value() != expected.canonical_value() {
                return Err(refused("house descriptor differs"));
            }
            if owner.as_str() == "roll"
                && !registry.delegates_family(
                    &owner,
                    &id,
                    &OwnerId::new("market").map_err(|_| refused("house owner"))?,
                )
            {
                return Err(refused("house Roll family not delegated"));
            }
        }
        Ok(())
    }
    pub(in crate::action_state) fn validate_composition(
        &self,
        definitions: &DefinitionRegistry,
        providers: &ActionProviderRegistry,
        participants: &crate::participant::ParticipantRegistry,
    ) -> Result<(), ActionStateError> {
        definitions
            .get_exact(&self.manifest)
            .map_err(|_| refused("house manifest unavailable"))?;
        for kind in [Kind::House] {
            for (action, transition) in self.actions(kind).iter().zip(TRANSITIONS) {
                let definition = definitions
                    .get_exact(action)
                    .map_err(|_| refused("house action unavailable"))?;
                let provider = providers
                    .get_exact(action)
                    .map_err(|_| refused("house provider unavailable"))?;
                if provider.owner().as_str() != "market"
                    || definition.payload() != &action_payload(kind, transition)
                {
                    return Err(refused("house action protocol differs"));
                }
                let refs = definition.references();
                if refs.len() != 3
                    || !refs.contains(&self.manifest)
                    || self.participants.iter().any(|p| !refs.contains(p))
                {
                    return Err(refused("house reference set differs"));
                }
                for reference in &self.participants {
                    definitions
                        .get_exact(reference)
                        .map_err(|_| refused("house participant unavailable"))?;
                    if !participants.grants_exact(reference, provider.owner(), action) {
                        return Err(refused("house participant grant differs"));
                    }
                }
            }
        }
        Ok(())
    }
}

fn check_participants(participants: &[DefinitionRef; 2]) -> Result<(), ActionStateError> {
    for (reference, (owner, id)) in participants
        .iter()
        .zip([("economy", "initial_filing"), ("roll", "commercial_roots")])
    {
        if reference.key().owner.as_str() != owner
            || reference.key().kind.as_str() != "owner_participant"
            || reference.key().id.as_str() != id
            || reference.version() != 1
        {
            return Err(refused("purchase historical participant identity"));
        }
    }
    Ok(())
}
fn participants(value: &Value) -> Result<[DefinitionRef; 2], ActionStateError> {
    let value = field(value, "participants")?;
    if value.as_object().is_none_or(|object| object.len() != 2) {
        return Err(refused("purchase participant fields"));
    }
    let references = [
        serde_json::from_value(field(value, "economy")?.clone())
            .map_err(|_| refused("purchase economy reference"))?,
        serde_json::from_value(field(value, "retention")?.clone())
            .map_err(|_| refused("purchase retention reference"))?,
    ];
    check_participants(&references)?;
    Ok(references)
}
fn action_payload(kind: Kind, transition: &str) -> Value {
    json!({"format":kind.version(),"protocol":kind.protocol(),"transition":transition})
}
fn admission_reference(
    manifest: &DefinitionRef,
    participants: &[DefinitionRef; 2],
    kind: Kind,
) -> Result<DefinitionRef, ActionStateError> {
    check_participants(participants)?;
    let descriptor = DefinitionDescriptor::new(
        DefinitionKind::new("action").map_err(|_| refused("purchase action kind"))?,
        DefinitionId::new(kind.actions()[0]).map_err(|_| refused("purchase action id"))?,
        kind.version(),
        action_payload(kind, "fund"),
    )
    .map_err(|_| refused("purchase action descriptor"))?
    .with_references(vec![
        manifest.clone(),
        participants[0].clone(),
        participants[1].clone(),
    ]);
    DefinitionRegistryBuilder::new()
        .register(
            OwnerId::new("market").map_err(|_| refused("purchase owner"))?,
            descriptor,
        )
        .map_err(|_| refused("purchase admission reference"))
}
fn historical_admission(
    protocol: &Protocol,
    purchase: &Value,
) -> Result<DefinitionRef, ActionStateError> {
    let compatibility = serde_json::from_value(
        field(
            field(field(purchase, "quote")?, "subject")?,
            "compatibility",
        )?
        .clone(),
    )
    .map_err(|_| refused("purchase historical manifest digest"))?;
    let manifest = DefinitionRef::new(
        protocol.manifest.key().clone(),
        protocol.manifest.version(),
        compatibility,
    )
    .map_err(|_| refused("purchase historical manifest"))?;
    admission_reference(
        &manifest,
        &participants(purchase)?,
        quote_kind(field(purchase, "quote")?)?,
    )
}

fn quote_kind(quote: &Value) -> Result<Kind, ActionStateError> {
    let product = field(quote, "product")?;
    let terms = field(product, "terms")?;
    let grant = field(product, "grant")?;
    let pin = field(product, "revision")?;
    let subject = field(quote, "subject")?;
    let issuer = field(subject, "issuer")?;
    if number(quote, "format")? != 1
        || text(product, "protocol")? != "house_v1"
        || number(grant, "format")? != 1
        || number(terms, "seats")? != 1
        || field(terms, "form")? != &json!({"kind":"sealed"})
        || field(grant, "downstream")? != &json!({"kind":"sealed"})
        || text(pin, "kind")? != "pin"
        || field(pin, "filed")? != field(subject, "filed")?
        || field(pin, "revision")? != field(subject, "revision")?
        || issuer.as_object().is_none_or(|v| v.len() != 3)
        || number(issuer, "format")? != 1
        || field(issuer, "organization")?.as_u64().is_none()
        || field(issuer, "scope")?.as_u64().is_none()
        || field(subject, "parent").is_ok()
        || field(subject, "curator").is_ok()
    {
        return Err(refused("house sealed product or issuer shape"));
    }
    let renewal = field(grant, "renewal")?;
    if number(renewal, "format")? != 1
        || field(renewal, "terms")? != terms
        || field(renewal, "revision")? != pin
        || field(renewal, "instrument")? != field(quote, "instrument")?
        || field(renewal, "price")?
            .as_i64()
            .is_none_or(|price| price <= 0)
        || field(renewal, "policy")? != field(quote, "policy")?
        || !(1..=7_776_000).contains(&number(renewal, "term_seconds")?)
        || number(renewal, "grace_seconds")? > 259_200
    {
        return Err(refused("house renewal consent differs"));
    }
    Ok(Kind::House)
}
fn field<'a>(value: &'a Value, name: &str) -> Result<&'a Value, ActionStateError> {
    value
        .get(name)
        .ok_or_else(|| refused("purchase field missing"))
}
fn text<'a>(value: &'a Value, name: &str) -> Result<&'a str, ActionStateError> {
    field(value, name)?
        .as_str()
        .ok_or_else(|| refused("purchase string field"))
}
fn number(value: &Value, name: &str) -> Result<u64, ActionStateError> {
    field(value, name)?
        .as_u64()
        .ok_or_else(|| refused("purchase unsigned field"))
}
fn token(value: &Value) -> Result<CommitToken, ActionStateError> {
    serde_json::from_value(value.clone()).map_err(|_| refused("purchase token"))
}
fn digest(domain: &str, value: &Value) -> Result<DefinitionDigest, ActionStateError> {
    DefinitionDigest::for_canonical_value(domain, value).map_err(|_| refused("purchase digest"))
}
fn licence(value: &Value) -> Result<u64, ActionStateError> {
    let text = value.as_str().ok_or_else(|| refused("purchase licence"))?;
    let id: u64 = text.parse().map_err(|_| refused("purchase licence"))?;
    if id == 0 || id.to_string() != text {
        return Err(refused("purchase canonical licence"));
    }
    Ok(id)
}
fn key(owner: &str, family: &str, encoded: String) -> Result<Key, ActionStateError> {
    Ok(Key {
        owner: OwnerId::new(owner).map_err(|_| refused("purchase owner"))?,
        family: StateFamilyId::new(family).map_err(|_| refused("purchase family"))?,
        encoded: encoded.into(),
    })
}
fn operation_key(family: &str, id: CommitToken) -> Result<Key, ActionStateError> {
    key(
        if family == ALLOCATION {
            "roll"
        } else {
            "market"
        },
        family,
        id.to_hex(),
    )
}
fn value(view: &View<'_>, key: &Key) -> Result<Option<Value>, ActionStateError> {
    let key = key.resolve(view.registry)?;
    let document = match view.mutations.and_then(|m| m.get(&key)) {
        Some(TxnMutation::Put { document, .. }) => Some(document.clone()),
        Some(TxnMutation::Delete) => None,
        None => view
            .records
            .get(&key)
            .map(|row| {
                serde_json::from_slice(row.canonical.as_slice())
                    .map_err(|_| refused("purchase retained encoding"))
            })
            .transpose()?,
    };
    document.map(|v| field(&v, "value").cloned()).transpose()
}
fn required(view: &View<'_>, key: &Key) -> Result<Value, ActionStateError> {
    value(view, key)?.ok_or_else(|| refused("purchase paired record missing"))
}
fn stamp(value: &Value) -> Result<i64, ActionStateError> {
    let stamp: crate::ClockStamp =
        serde_json::from_value(value.clone()).map_err(|_| refused("purchase IC stamp"))?;
    if !matches!(stamp.clock, crate::ClockRef::WorldIc { generation } if generation != 0) {
        return Err(refused("purchase WorldIc stamp required"));
    }
    Ok(stamp.at)
}
struct Pair {
    purchase: Value,
    allocation: Value,
    phase: String,
    licence: u64,
}
fn pair(view: &View<'_>, id: CommitToken) -> Result<Pair, ActionStateError> {
    require_disjoint_operation(view, id)?;
    let purchase = required(view, &operation_key(PURCHASE, id)?)?;
    let allocation = required(view, &operation_key(ALLOCATION, id)?)?;
    let quote = field(&purchase, "quote")?;
    let product = field(quote, "product")?;
    let subject = field(quote, "subject")?;
    let phase = text(field(&purchase, "phase")?, "kind")?;
    let kind = quote_kind(quote)?;
    let paid_terms = field(product, "terms")?;
    let id_number = licence(field(&purchase, "licence")?)?;
    let protocol = view
        .registry
        .house_purchase
        .as_ref()
        .ok_or_else(|| refused("purchase protocol unavailable"))?;
    if number(&purchase, "format")? != 1
        || number(&allocation, "format")? != 1
        || text(&allocation, "protocol")? != kind.wire()
        || field(&purchase, "effect")? != &json!(historical_admission(protocol, &purchase)?)
        || token(field(&purchase, "operation")?)? != id
        || token(field(&allocation, "operation")?)? != id
        || text(&purchase, "term")? != "1"
        || text(&allocation, "term")? != "1"
        || !matches!(phase, "funded" | "granted" | "refunded")
        || field(&purchase, "phase")? != field(&allocation, "phase")?
        || field(&purchase, "updated_by")? != field(&allocation, "updated_by")?
        || field(&purchase, "funding")? != field(&allocation, "funding")?
        || field(&purchase, "participants")? != field(&allocation, "participants")?
        || field(&purchase, "licence")? != field(&allocation, "licence")?
        || field(&allocation, "quote")? != field(quote, "publication")?
        || field(&purchase, "quote_digest")? != field(&allocation, "quote_digest")?
        || *field(&purchase, "quote_digest")? != json!(digest(kind.quote_domain(), quote)?)
        || field(&allocation, "product_digest")? != field(quote, "product_digest")?
        || *field(quote, "product_digest")? != json!(digest(kind.product_domain(), product)?)
        || field(&allocation, "holder")? != field(quote, "buyer")?
        || field(&allocation, "terms")? != paid_terms
    {
        return Err(refused("purchase immutable paired identity differs"));
    }
    for name in ["issuer", "charter", "line", "filed", "revision", "rating"] {
        if field(&allocation, name)? != field(subject, name)? {
            return Err(refused("purchase subject differs"));
        }
    }
    let economics = field(&allocation, "economics")?;
    if economics
        != &json!({"format":1,"licence":field(&allocation,"licence")?,"generation":field(&allocation,"term")?,"operation":id,"product_digest":field(&allocation,"product_digest")?,"grant":field(product,"grant")?})
    {
        return Err(refused("house allocated economics differs"));
    }
    let updated = token(field(&purchase, "updated_by")?)?;
    if (phase == "funded" && updated != id)
        || (phase != "funded"
            && (updated == id || token(field(field(&purchase, "phase")?, "commit")?)? != updated))
    {
        return Err(refused("purchase phase commit differs"));
    }
    stamp(field(field(&purchase, "funding")?, "at_ic")?)?;
    Ok(Pair {
        phase: phase.into(),
        purchase,
        allocation,
        licence: id_number,
    })
}
fn fingerprint(pair: &Pair) -> Result<DefinitionDigest, ActionStateError> {
    let mut values = [pair.purchase.clone(), pair.allocation.clone()];
    for value in &mut values {
        let object = value
            .as_object_mut()
            .ok_or_else(|| refused("purchase record shape"))?;
        object.remove("phase");
        object.remove("updated_by");
    }
    digest("core.house_mind_purchase.capacity.v1", &json!(values))
}
fn targets(id: CommitToken, licence: u64) -> Result<Vec<Target>, ActionStateError> {
    let mut out = Vec::with_capacity(6);
    for (owner, family, ceiling) in FAMILIES {
        let encoded = match family {
            LICENCE => format!("licence/{licence:016x}"),
            TERM | ECONOMICS => format!("licence/{licence:016x}/term/0000000000000001"),
            _ => id.to_hex(),
        };
        out.push(Target {
            key: key(owner, family, encoded)?,
            schema: CanonicalId::new(format!("{family}.value.v1"))
                .map_err(|_| refused("purchase target schema"))?,
            schema_version: 1,
            ceiling,
            mode: if matches!(family, PURCHASE | ALLOCATION) {
                SlotMode::UpdateExisting
            } else {
                SlotMode::Create
            },
        });
    }
    out.sort_by(|a, b| {
        (&a.key.owner, &a.key.family, &a.key.encoded).cmp(&(
            &b.key.owner,
            &b.key.family,
            &b.key.encoded,
        ))
    });
    Ok(out)
}
fn outputs(view: &View<'_>, id: CommitToken, pair: &Pair) -> Result<(), ActionStateError> {
    let receipt = value(view, &operation_key(RECEIPT, id)?)?;
    let current = value(
        view,
        &key("roll", LICENCE, format!("licence/{:016x}", pair.licence))?,
    )?;
    let term = value(
        view,
        &key(
            "roll",
            TERM,
            format!("licence/{:016x}/term/0000000000000001", pair.licence),
        )?,
    )?;
    let economics = value(
        view,
        &key(
            "roll",
            ECONOMICS,
            format!("licence/{:016x}/term/0000000000000001", pair.licence),
        )?,
    )?;
    if pair.phase == "funded" {
        if receipt.is_some() || current.is_some() || term.is_some() || economics.is_some() {
            return Err(refused("purchase premature terminal output"));
        }
        return Ok(());
    }
    let receipt = receipt.ok_or_else(|| refused("purchase terminal receipt missing"))?;
    let funding = field(&pair.purchase, "funding")?;
    if number(&receipt, "format")? != 1
        || token(field(&receipt, "operation")?)? != id
        || field(&receipt, "commit")? != field(&pair.purchase, "updated_by")?
        || text(field(&receipt, "outcome")?, "kind")? != pair.phase
        || field(&receipt, "contract")? != field(funding, "contract")?
        || field(&receipt, "hold")? != field(funding, "hold")?
        || field(&receipt, "licence")? != field(&pair.purchase, "licence")?
        || text(&receipt, "term")? != "1"
        || field(&receipt, "quote_digest")? != field(&pair.purchase, "quote_digest")?
        || field(&receipt, "product_digest")? != field(&pair.allocation, "product_digest")?
        || field(&receipt, "grant_economics_digest")?
            != &json!(digest(
                "roll.mind.house_grant_economics.v1",
                field(field(field(&pair.purchase, "quote")?, "product")?, "grant")?
            )?)
    {
        return Err(refused("purchase terminal receipt differs"));
    }
    if pair.phase == "refunded" {
        if current.is_some()
            || term.is_some()
            || economics.is_some()
            || !field(&receipt, "issued_at_ic")?.is_null()
        {
            return Err(refused("purchase refunded grant exists"));
        }
        return Ok(());
    }
    if economics.as_ref() != Some(field(&pair.allocation, "economics")?) {
        return Err(refused("house granted economics differs"));
    }
    let current = current.ok_or_else(|| refused("purchase paid licence missing"))?;
    house_renewal::validate_current_birth(view, &current, id)?;
    let term = term.ok_or_else(|| refused("purchase paid term missing"))?;
    for name in ["charter", "line", "holder"] {
        if field(&current, name)? != field(&pair.allocation, name)? {
            return Err(refused("purchase paid licence subject differs"));
        }
    }
    if number(&current, "format")? != 1
        || number(&term, "format")? != 1
        || field(&current, "id")? != field(&pair.purchase, "licence")?
        || field(&term, "licence")? != field(&pair.purchase, "licence")?
        || text(&term, "generation")? != "1"
        || token(field(&current, "operation")?)? != id
        || token(field(&term, "operation")?)? != id
        || field(&term, "terms")? != field(&pair.allocation, "terms")?
        || field(&term, "revision")?
            != field(
                field(field(&pair.purchase, "quote")?, "product")?,
                "revision",
            )?
    {
        return Err(refused("purchase paid grant differs"));
    }
    if field(field(&receipt, "issued_at_ic")?, "clock")?
        != field(field(funding, "at_ic")?, "clock")?
    {
        return Err(refused("house grant clock differs"));
    }
    let issued = stamp(field(&receipt, "issued_at_ic")?)?;
    if issued < stamp(field(funding, "at_ic")?)? {
        return Err(refused("house grant precedes funding"));
    }
    let product = field(field(&pair.purchase, "quote")?, "product")?;
    let end = issued
        .checked_add(
            field(product, "term_seconds")?
                .as_i64()
                .ok_or_else(|| refused("purchase duration"))?,
        )
        .ok_or_else(|| refused("purchase duration overflow"))?;
    let grace = end
        .checked_add(
            field(product, "grace_seconds")?
                .as_i64()
                .ok_or_else(|| refused("purchase grace"))?,
        )
        .ok_or_else(|| refused("purchase grace overflow"))?;
    if field(&term, "window")?
        != &json!({"format":1,"eligibility_start":issued,"issued_at":issued,"term_end":end,"grace_end":grace})
    {
        return Err(refused("purchase grant interval differs"));
    }
    Ok(())
}
pub(super) fn verify(
    phase: Phase,
    reservation: &Reservation,
    view: &View<'_>,
) -> Result<(), ActionStateError> {
    let protocol = view
        .registry
        .house_purchase
        .as_ref()
        .ok_or_else(|| refused("purchase protocol unavailable"))?;
    let pair = pair(view, reservation.funding)?;
    let kind = quote_kind(field(&pair.purchase, "quote")?)?;
    if reservation.effect != historical_admission(protocol, &pair.purchase)?
        || (matches!(phase, Phase::Funding)
            && reservation.effect != *protocol.admission_action(kind))
        || (matches!(phase, Phase::Terminal)
            && pair.phase == "granted"
            && reservation.effect != *protocol.admission_action(kind))
        || field(&pair.purchase, "effect")? != &json!(reservation.effect)
        || reservation.sponsor != operation_key(PURCHASE, reservation.funding)?
        || reservation.targets != targets(reservation.funding, pair.licence)?
        || reservation.protocol_fingerprint != Some(fingerprint(&pair)?)
        || match phase {
            Phase::Funding | Phase::Retained => pair.phase != "funded",
            Phase::Terminal => pair.phase == "funded",
        }
    {
        return Err(refused("purchase reservation or phase differs"));
    }
    outputs(view, reservation.funding, &pair)
}
pub(in crate::action_state) fn house_change(
    txn: &mut ActionTxn<'_>,
    commit: CommitToken,
    terminal: Option<CommitToken>,
) -> Result<(), ActionStateError> {
    let protocol = txn
        .store
        .registry
        .house_purchase
        .as_ref()
        .ok_or_else(|| refused("purchase protocol unavailable"))?;
    let identity = txn
        .registered_identity
        .as_ref()
        .ok_or_else(|| refused("purchase actual invocation unavailable"))?;
    if identity.commit != commit
        || txn.provider_ordinal != Some(identity.ordinal)
        || txn.owner != identity.action.key().owner
        || txn.capacity_changes.funding.is_some()
        || txn.capacity_changes.terminal.is_some()
        || txn.capacity_changes.progress.is_some()
    {
        return Err(refused("purchase preparation identity differs"));
    }
    let (kind, action) = protocol.action_index(identity)?;
    if let Some(operation) = terminal {
        if !matches!(action, 1 | 2) {
            return Err(refused("purchase terminal action differs"));
        }
        txn.capacity_changes.terminal = Some(ReservationId(txn.owner.clone(), operation.to_hex()));
    } else {
        if action != 0 {
            return Err(refused("purchase funding action differs"));
        }
        let view = View {
            registry: &txn.store.registry,
            records: &txn.store.records,
            mutations: Some(&txn.mutations),
        };
        let pair = pair(&view, commit)?;
        if quote_kind(field(&pair.purchase, "quote")?)? != kind {
            return Err(refused("house funding protocol differs"));
        }
        let record = Reservation {
            format: 1,
            funding: commit,
            effect: protocol.admission_action(kind).clone(),
            sponsor: operation_key(PURCHASE, commit)?,
            targets: targets(commit, pair.licence)?,
            protocol_fingerprint: Some(fingerprint(&pair)?),
        };
        verify(Phase::Funding, &record, &view)?;
        txn.capacity_changes.funding = Some(record);
    }
    Ok(())
}
fn allocation_present(
    view: &View<'_>,
    family: &str,
    operation: CommitToken,
) -> Result<bool, ActionStateError> {
    let row_key = key("roll", family, operation.to_hex())?;
    if !view
        .registry
        .entries
        .contains_key(&(row_key.owner.clone(), row_key.family.clone()))
    {
        return Ok(false);
    }
    Ok(value(view, &row_key)?.is_some())
}
pub(super) fn shared_output_is_house(
    view: &View<'_>,
    state_key: &StateKey,
) -> Result<bool, ActionStateError> {
    if state_key.owner.as_str() != "roll" || !matches!(state_key.family.as_str(), LICENCE | TERM) {
        return Ok(false);
    }
    let row = required(
        view,
        &Key {
            owner: state_key.owner.clone(),
            family: state_key.family.clone(),
            encoded: state_key.encoded.clone(),
        },
    )?;
    let operation = token(field(&row, "operation")?)?;
    if !allocation_present(view, ALLOCATION, operation)? {
        if house_renewal::shared_output_is_renewal(view, state_key)? {
            return Ok(false);
        }
        if renewal::shared_output_is_renewal(view, state_key)? {
            return Ok(false);
        }
        // The shared row must still have one complete registered personal birth
        // route. Missing every allocation never becomes an ignored house row.
        entitlement::shared_output_is_entitlement(view, state_key)?;
        return Ok(false);
    }
    if view.registry.house_purchase.is_none() {
        return Err(refused("house grant lacks registered protocol"));
    }
    require_disjoint_operation(view, operation)?;
    Ok(true)
}
pub(super) fn reject_house_operation(
    view: &View<'_>,
    operation: CommitToken,
) -> Result<(), ActionStateError> {
    if allocation_present(view, ALLOCATION, operation)? {
        return Err(refused("operation also names house purchase"));
    }
    Ok(())
}
fn require_disjoint_operation(
    view: &View<'_>,
    operation: CommitToken,
) -> Result<(), ActionStateError> {
    renewal::reject_other_operation(view, operation)?;
    house_renewal::reject_other_operation(view, operation)?;
    for family in ["roll_mind_paid_purchase", "roll_mind_entitlement_purchase"] {
        if allocation_present(view, family, operation)? {
            return Err(refused("house operation has another purchase allocation"));
        }
    }
    Ok(())
}
fn protected(key: &StateKey) -> bool {
    FAMILIES
        .iter()
        .any(|(owner, family, _)| key.owner.as_str() == *owner && key.family.as_str() == *family)
}
fn row_operation(view: &View<'_>, key: &StateKey) -> Result<CommitToken, ActionStateError> {
    let row = required(
        view,
        &Key {
            owner: key.owner.clone(),
            family: key.family.clone(),
            encoded: key.encoded.clone(),
        },
    )?;
    let operation = token(field(&row, "operation")?)?;
    if matches!(key.family.as_str(), LICENCE | TERM | ECONOMICS) {
        let allocation = required(view, &operation_key(ALLOCATION, operation)?)?;
        let field_name = if key.family.as_str() == LICENCE {
            "id"
        } else {
            "licence"
        };
        if field(&row, field_name)? != field(&allocation, "licence")? {
            return Err(refused("purchase extra grant row or crossed allocation"));
        }
    }
    let expected = match key.family.as_str() {
        LICENCE => format!("licence/{:016x}", licence(field(&row, "id")?)?),
        TERM | ECONOMICS => format!(
            "licence/{:016x}/term/0000000000000001",
            licence(field(&row, "licence")?)?
        ),
        _ => operation.to_hex(),
    };
    if expected != key.encoded.as_str() {
        return Err(refused("purchase record key differs"));
    }
    Ok(operation)
}
pub(super) fn audit_boot(
    view: &View<'_>,
    reservations: &BTreeMap<ReservationId, Arc<StoredReservation>>,
) -> Result<(), ActionStateError> {
    if view.registry.house_purchase.is_none() {
        if view
            .records
            .keys()
            .any(|key| protected(key) && !matches!(key.family.as_str(), LICENCE | TERM))
        {
            return Err(refused("retained house rows require registered protocol"));
        }
        return Ok(());
    }
    let mut operations = BTreeSet::new();
    for key in view.records.keys().filter(|key| protected(key)) {
        if house_renewal::shared_output_is_renewal(view, key)? {
            continue;
        }
        if matches!(key.family.as_str(), LICENCE | TERM) && !shared_output_is_house(view, key)? {
            continue;
        }
        operations.insert(row_operation(view, key)?);
    }
    for id in operations {
        let pair = pair(view, id)?;
        let reservation = reservations.get(&ReservationId(
            OwnerId::new("market").map_err(|_| refused("purchase owner"))?,
            id.to_hex(),
        ));
        match (pair.phase.as_str(), reservation) {
            ("funded", Some(row)) => verify(Phase::Retained, &row.record, view)?,
            ("granted" | "refunded", None) => outputs(view, id, &pair)?,
            _ => return Err(refused("purchase reverse capacity completeness")),
        }
    }
    Ok(())
}
pub(super) fn validate_changes(
    store: &ActionStateStore,
    mutations: &BTreeMap<StateKey, TxnMutation>,
    changes: &Changes,
    identity: Option<&RegisteredTxnIdentity>,
) -> Result<(), ActionStateError> {
    let Some(protocol) = &store.registry.house_purchase else {
        if mutations
            .keys()
            .any(|key| protected(key) && !matches!(key.family.as_str(), LICENCE | TERM))
        {
            return Err(refused("house writes require registered protocol"));
        }
        return Ok(());
    };
    let before = View {
        registry: &store.registry,
        records: &store.records,
        mutations: None,
    };
    let after = View {
        registry: &store.registry,
        records: &store.records,
        mutations: Some(mutations),
    };
    let mut operations = BTreeSet::new();
    for key in mutations.keys().filter(|key| protected(key)) {
        if house_renewal::shared_output_is_renewal(&after, key)?
            || house_renewal::owns_licence_change(&after, key, identity, changes)?
        {
            continue;
        }
        if matches!(key.family.as_str(), LICENCE | TERM) && !shared_output_is_house(&after, key)? {
            continue;
        }
        operations.insert(row_operation(&after, key)?);
    }
    if operations.is_empty() {
        return Ok(());
    }
    let identity = identity.ok_or_else(|| refused("purchase write lacks actual invocation"))?;
    let (kind, action) = protocol.action_index(identity)?;
    if operations.len() != 1 {
        return Err(refused("purchase one operation per invocation"));
    }
    for id in operations {
        let pair = pair(&after, id)?;
        if quote_kind(field(&pair.purchase, "quote")?)? != kind {
            return Err(refused("house invocation protocol differs"));
        }
        for family in [PURCHASE, ALLOCATION] {
            if !matches!(
                mutations.get(&operation_key(family, id)?.resolve(&store.registry)?),
                Some(TxnMutation::Put { .. })
            ) {
                return Err(refused("purchase requires both operation postimages"));
            }
        }
        if token(field(&pair.purchase, "updated_by")?)? != identity.commit {
            return Err(refused("purchase update commit differs"));
        }
        if value(&before, &operation_key(PURCHASE, id)?)?.is_none() {
            if action != 0
                || id != identity.commit
                || identity.principal != (PrincipalId::Object { id: identity.actor })
                || number(field(&pair.purchase, "quote")?, "buyer")? != u64::from(identity.actor.0)
                || value(&before, &operation_key(ALLOCATION, id)?)?.is_some()
                || changes.funding.as_ref().map(|row| row.funding) != Some(id)
            {
                return Err(refused("purchase funding invocation differs"));
            }
            let quote = field(&pair.purchase, "quote")?;
            let quote_key = key(
                "market",
                "market_mind_house_quote",
                token(field(quote, "publication")?)?.to_hex(),
            )?;
            if required(&before, &quote_key)? != *quote
                || !matches!(
                    mutations.get(&quote_key.resolve(&store.registry)?),
                    Some(TxnMutation::Delete)
                )
            {
                return Err(refused("purchase did not consume exact acknowledged quote"));
            }
            let allocation_key = key("roll", "roll_mind_commerce", "registry".into())?;
            let old = required(&before, &allocation_key)?;
            let mut expected = old.clone();
            let next = number(&old, "next_licence")?;
            expected
                .as_object_mut()
                .ok_or_else(|| refused("purchase deployment"))?
                .insert(
                    "next_licence".into(),
                    json!(
                        next.checked_add(1)
                            .ok_or_else(|| refused("purchase allocator overflow"))?
                    ),
                );
            if pair.licence != next
                || required(&after, &allocation_key)? != expected
                || !matches!(
                    mutations.get(&allocation_key.resolve(&store.registry)?),
                    Some(TxnMutation::Put { .. })
                )
            {
                return Err(refused("purchase shared allocation differs"));
            }
            verify(
                Phase::Funding,
                changes
                    .funding
                    .as_ref()
                    .ok_or_else(|| refused("purchase funding reservation missing"))?,
                &after,
            )?;
        } else {
            let prior = pair_fn(&before, id)?;
            let expected_phase = match action {
                1 => "granted",
                2 => "refunded",
                _ => return Err(refused("purchase terminal invocation differs")),
            };
            if identity.principal != (PrincipalId::Object { id: identity.actor })
                || (number(field(&pair.purchase, "quote")?, "buyer")?
                    != u64::from(identity.actor.0)
                    && !(action == 2 && identity.actor == crate::GOD))
            {
                return Err(refused("house terminal actor differs"));
            }
            let reservation_id = ReservationId(
                OwnerId::new("market").map_err(|_| refused("purchase owner"))?,
                id.to_hex(),
            );
            if prior.phase != "funded"
                || pair.phase != expected_phase
                || identity.commit == id
                || fingerprint(&prior)? != fingerprint(&pair)?
                || changes.terminal.as_ref() != Some(&reservation_id)
            {
                return Err(refused("purchase terminal transition differs"));
            }
            let stored = store
                .capacity
                .records
                .get(&reservation_id)
                .ok_or_else(|| refused("purchase retained reservation missing"))?;
            verify(Phase::Terminal, &stored.record, &after)?;
        }
    }
    Ok(())
}
// Avoid shadowing the validated pair local in the transition audit.
fn pair_fn(view: &View<'_>, id: CommitToken) -> Result<Pair, ActionStateError> {
    pair(view, id)
}

#[cfg(test)]
pub(super) mod tests;
