//! Closed seven-target renewal completion. Money and current seller authority
//! remain actual owner obligations; core binds paired images and future room.
use super::*;
use crate::action::{ActionProviderRegistry, PrincipalId};
use crate::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistry,
    DefinitionRegistryBuilder,
};

const ACTIONS: [&str; 5] = [
    "draft_mind_renewal",
    "fund_mind_renewal",
    "settle_mind_renewal",
    "refund_mind_renewal",
    "close_mind_renewal",
];
const SERVICE_ACTION: &str = "close_mind_renewal_service";
const TRANSITIONS: [&str; 5] = ["draft", "fund", "grant", "refund", "unfunded_close"];
const MARKET: &str = "market_mind_renewal_operation";
const ROLL: &str = "roll_mind_renewal_operation";
const HEAD: &str = "roll_mind_renewal_head";
const RECEIPT: &str = "market_mind_renewal_receipt";
const LICENCE: &str = "roll_mind_personal_paid_licence";
const TERM: &str = "roll_mind_paid_term";
const ECONOMICS: &str = "roll_mind_paid_term_economics";
const FAMILIES: [(&str, &str, usize); 7] = [
    ("market", MARKET, 32768),
    ("roll", ROLL, 16384),
    ("roll", HEAD, 4096),
    ("roll", LICENCE, 4096),
    ("roll", TERM, 8192),
    ("roll", ECONOMICS, 8192),
    ("market", RECEIPT, 8192),
];

#[derive(Clone, Debug, Serialize)]
pub(in crate::action_state) struct Protocol {
    manifest: DefinitionRef,
    participants: [DefinitionRef; 2],
    actions: [DefinitionRef; 5],
    #[serde(skip_serializing_if = "Option::is_none")]
    service_close: Option<DefinitionRef>,
}
impl Protocol {
    pub(in crate::action_state) fn new(
        owner: &OwnerId,
        manifest: DefinitionRef,
        participants: [DefinitionRef; 2],
        actions: [DefinitionRef; 5],
    ) -> Result<Self, ActionStateError> {
        if owner.as_str() != "market"
            || manifest.key().owner.as_str() != "tacsim"
            || manifest.key().kind.as_str() != "registry_manifest"
            || manifest.key().id.as_str() != "combat"
            || manifest.version() != 1
        {
            return Err(refused("renewal declaration owner or manifest"));
        }
        check_participants(&participants)?;
        for (index, action) in actions.iter().enumerate() {
            if *action != action_reference(&manifest, &participants, index)? {
                return Err(refused("renewal complete action identity or order"));
            }
        }
        Ok(Self {
            manifest,
            participants,
            actions,
            service_close: None,
        })
    }
    pub(in crate::action_state) fn with_service(
        owner: &OwnerId,
        manifest: DefinitionRef,
        participants: [DefinitionRef; 2],
        actions: [DefinitionRef; 5],
        service_close: DefinitionRef,
    ) -> Result<Self, ActionStateError> {
        let mut protocol = Self::new(owner, manifest, participants, actions)?;
        if service_close != action_reference(&protocol.manifest, &protocol.participants, 5)? {
            return Err(refused("renewal complete service action differs"));
        }
        protocol.service_close = Some(service_close);
        Ok(protocol)
    }
    pub(in crate::action_state) fn has_service(&self) -> bool {
        self.service_close.is_some()
    }
    pub(super) fn admission_action(&self) -> &DefinitionRef {
        &self.actions[0]
    }
    pub(super) fn recognizes_historical(&self, effect: &DefinitionRef) -> bool {
        effect.key() == self.admission_action().key() && effect.version() == 1
    }
    fn action_index(&self, identity: &RegisteredTxnIdentity) -> Result<usize, ActionStateError> {
        self.actions
            .iter()
            .chain(self.service_close.iter())
            .position(|action| action == &identity.action)
            .ok_or_else(|| refused("renewal invocation action differs"))
    }
    pub(in crate::action_state) fn validate_families(
        &self,
        registry: &StateFamilyRegistry,
    ) -> Result<(), ActionStateError> {
        // Renewal shares immutable birth records with both closed issuance paths.
        if registry.original_purchase.is_none() || registry.entitlement_purchase.is_none() {
            return Err(refused("renewal requires both closed purchase audits"));
        }
        for (owner, family, bytes) in FAMILIES {
            let id = StateFamilyId::new(family).map_err(|_| refused("renewal family"))?;
            let canonical = |suffix: &str| {
                CanonicalId::new(format!("{family}.{suffix}.v1"))
                    .map_err(|_| refused("renewal schema"))
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
            .map_err(|_| refused("renewal descriptor"))?;
            let owner = OwnerId::new(owner).map_err(|_| refused("renewal owner"))?;
            let actual = registry
                .entries
                .get(&(owner.clone(), id.clone()))
                .ok_or_else(|| refused("renewal family unavailable"))?;
            if actual.descriptor().canonical_value() != expected.canonical_value() {
                return Err(refused("renewal descriptor differs"));
            }
            if owner.as_str() == "roll"
                && !registry.delegates_family(
                    &owner,
                    &id,
                    &OwnerId::new("market").map_err(|_| refused("renewal owner"))?,
                )
            {
                return Err(refused("renewal Roll family not delegated"));
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
            .map_err(|_| refused("renewal manifest unavailable"))?;
        for (index, action) in self
            .actions
            .iter()
            .chain(self.service_close.iter())
            .enumerate()
        {
            let definition = definitions
                .get_exact(action)
                .map_err(|_| refused("renewal action unavailable"))?;
            let provider = providers
                .get_exact(action)
                .map_err(|_| refused("renewal provider unavailable"))?;
            if provider.owner().as_str() != "market"
                || definition.payload() != &action_payload(index)
            {
                return Err(refused("renewal action protocol differs"));
            }
            let refs = definition.references();
            if refs.len() != 3
                || !refs.contains(&self.manifest)
                || self
                    .participants
                    .iter()
                    .any(|reference| !refs.contains(reference))
            {
                return Err(refused("renewal exact reference set differs"));
            }
            for reference in &self.participants {
                definitions
                    .get_exact(reference)
                    .map_err(|_| refused("renewal participant unavailable"))?;
                if !participants.grants_exact(reference, provider.owner(), action) {
                    return Err(refused("renewal participant grant differs"));
                }
            }
        }
        Ok(())
    }
}
fn check_participants(refs: &[DefinitionRef; 2]) -> Result<(), ActionStateError> {
    for (reference, (owner, id)) in refs
        .iter()
        .zip([("economy", "initial_filing"), ("roll", "commercial_roots")])
    {
        if reference.key().owner.as_str() != owner
            || reference.key().kind.as_str() != "owner_participant"
            || reference.key().id.as_str() != id
            || reference.version() != 1
        {
            return Err(refused("renewal historical participant identity"));
        }
    }
    Ok(())
}
fn action_id(index: usize) -> &'static str {
    if index == 5 {
        SERVICE_ACTION
    } else {
        ACTIONS[index]
    }
}
fn action_payload(index: usize) -> Value {
    if index == 5 {
        return json!({"format":1,"protocol":"personal_mind_renewal_service_v1",
            "transition":"unfunded_service_close","service_actor":crate::GOD});
    }
    json!({"format":1,"protocol":"personal_mind_renewal_v1","transition":TRANSITIONS[index]})
}
fn action_reference(
    manifest: &DefinitionRef,
    participants: &[DefinitionRef; 2],
    index: usize,
) -> Result<DefinitionRef, ActionStateError> {
    check_participants(participants)?;
    let descriptor = DefinitionDescriptor::new(
        DefinitionKind::new("action").map_err(|_| refused("renewal action kind"))?,
        DefinitionId::new(action_id(index)).map_err(|_| refused("renewal action id"))?,
        1,
        action_payload(index),
    )
    .map_err(|_| refused("renewal action descriptor"))?
    .with_references(vec![
        manifest.clone(),
        participants[0].clone(),
        participants[1].clone(),
    ]);
    DefinitionRegistryBuilder::new()
        .register(
            OwnerId::new("market").map_err(|_| refused("renewal owner"))?,
            descriptor,
        )
        .map_err(|_| refused("renewal action reference"))
}
fn field<'a>(value: &'a Value, name: &str) -> Result<&'a Value, ActionStateError> {
    value
        .get(name)
        .ok_or_else(|| refused("renewal field missing"))
}
fn text<'a>(value: &'a Value, name: &str) -> Result<&'a str, ActionStateError> {
    field(value, name)?
        .as_str()
        .ok_or_else(|| refused("renewal string field"))
}
fn number(value: &Value, name: &str) -> Result<u64, ActionStateError> {
    field(value, name)?
        .as_u64()
        .ok_or_else(|| refused("renewal unsigned field"))
}
fn token(value: &Value) -> Result<CommitToken, ActionStateError> {
    serde_json::from_value(value.clone()).map_err(|_| refused("renewal token"))
}
fn positive_string(value: &Value) -> Result<u64, ActionStateError> {
    let text = value
        .as_str()
        .ok_or_else(|| refused("renewal canonical positive integer"))?;
    let n: u64 = text
        .parse()
        .map_err(|_| refused("renewal canonical positive integer"))?;
    if n == 0 || n.to_string() != text {
        return Err(refused("renewal canonical positive integer"));
    }
    Ok(n)
}
fn digest(domain: &str, value: &Value) -> Result<DefinitionDigest, ActionStateError> {
    DefinitionDigest::for_canonical_value(domain, value).map_err(|_| refused("renewal digest"))
}
fn key(family: &str, encoded: String) -> Result<Key, ActionStateError> {
    Ok(Key {
        owner: OwnerId::new(if matches!(family, MARKET | RECEIPT) {
            "market"
        } else {
            "roll"
        })
        .map_err(|_| refused("renewal owner"))?,
        family: StateFamilyId::new(family).map_err(|_| refused("renewal family"))?,
        encoded: encoded.into(),
    })
}
fn operation_key(family: &str, operation: CommitToken) -> Result<Key, ActionStateError> {
    key(family, operation.to_hex())
}
fn value(view: &View<'_>, raw: &Key) -> Result<Option<Value>, ActionStateError> {
    let key = raw.resolve(view.registry)?;
    let document = match view.mutations.and_then(|m| m.get(&key)) {
        Some(TxnMutation::Put { document, .. }) => Some(document.clone()),
        Some(TxnMutation::Delete) => None,
        None => view
            .records
            .get(&key)
            .map(|row| {
                serde_json::from_slice(row.canonical.as_slice())
                    .map_err(|_| refused("renewal retained encoding"))
            })
            .transpose()?,
    };
    document.map(|v| field(&v, "value").cloned()).transpose()
}
fn required(view: &View<'_>, key: &Key) -> Result<Value, ActionStateError> {
    value(view, key)?.ok_or_else(|| refused("renewal paired record missing"))
}

struct Pair {
    market: Value,
    roll: Value,
    licence: u64,
    predecessor: u64,
    next: u64,
    phase: String,
}
impl Pair {
    fn binding(&self) -> Result<&Value, ActionStateError> {
        field(&self.market, "binding")
    }
    fn state(&self) -> Result<&Value, ActionStateError> {
        field(&self.market, "state")
    }
    fn pending(&self) -> bool {
        matches!(self.phase.as_str(), "drafted" | "funded")
    }
}
fn optional_family_value(
    view: &View<'_>,
    family: &str,
    operation: CommitToken,
) -> Result<Option<Value>, ActionStateError> {
    let key = operation_key(family, operation)?;
    if !view
        .registry
        .entries
        .contains_key(&(key.owner.clone(), key.family.clone()))
    {
        return Ok(None);
    }
    value(view, &key)
}
pub(super) fn require_disjoint_operation(
    view: &View<'_>,
    operation: CommitToken,
) -> Result<(), ActionStateError> {
    for family in ["roll_mind_paid_purchase", "roll_mind_entitlement_purchase"] {
        if optional_family_value(view, family, operation)?.is_some() {
            return Err(refused("renewal operation has another purchase recipe"));
        }
    }
    Ok(())
}
fn historical_action(pair: &Pair, index: usize) -> Result<DefinitionRef, ActionStateError> {
    let manifest: DefinitionRef = serde_json::from_value(field(&pair.market, "manifest")?.clone())
        .map_err(|_| refused("renewal historical manifest"))?;
    if manifest.key().owner.as_str() != "tacsim"
        || manifest.key().kind.as_str() != "registry_manifest"
        || manifest.key().id.as_str() != "combat"
        || manifest.version() != 1
    {
        return Err(refused("renewal historical manifest identity"));
    }
    let participants: [DefinitionRef; 2] =
        serde_json::from_value(field(&pair.market, "participants")?.clone())
            .map_err(|_| refused("renewal historical participant array"))?;
    action_reference(&manifest, &participants, index)
}
fn signed(value: &Value, name: &str) -> Result<i64, ActionStateError> {
    field(value, name)?
        .as_i64()
        .ok_or_else(|| refused("renewal signed field"))
}
fn stamp(value: &Value) -> Result<crate::ClockStamp, ActionStateError> {
    let stamp: crate::ClockStamp =
        serde_json::from_value(value.clone()).map_err(|_| refused("renewal IC stamp"))?;
    if !matches!(stamp.clock, crate::ClockRef::WorldIc { generation } if generation != 0) {
        return Err(refused("renewal versioned WorldIc required"));
    }
    Ok(stamp)
}
fn term_key(family: &str, licence: u64, generation: u64) -> Result<Key, ActionStateError> {
    key(
        family,
        format!("licence/{licence:016x}/term/{generation:016x}"),
    )
}
fn licence_key(family: &str, licence: u64) -> Result<Key, ActionStateError> {
    key(family, format!("licence/{licence:016x}"))
}
fn pair(view: &View<'_>, operation: CommitToken) -> Result<Pair, ActionStateError> {
    require_disjoint_operation(view, operation)?;
    let market = required(view, &operation_key(MARKET, operation)?)?;
    let roll = required(view, &operation_key(ROLL, operation)?)?;
    let binding = field(&market, "binding")?;
    let licence = positive_string(field(binding, "licence")?)?;
    let predecessor = positive_string(field(binding, "predecessor")?)?;
    let next = positive_string(field(binding, "next")?)?;
    let phase = text(field(&market, "state")?, "kind")?.to_owned();
    if number(&market, "format")? != 1
        || number(&roll, "format")? != 1
        || number(binding, "format")? != 1
        || token(field(&market, "operation")?)? != operation
        || predecessor.checked_add(1) != Some(next)
        || !matches!(
            phase.as_str(),
            "drafted"
                | "funded"
                | "granted"
                | "refunded"
                | "unfunded_closed"
                | "unfunded_service_closed"
        )
    {
        return Err(refused("renewal paired shape or generation"));
    }
    for name in [
        "format",
        "operation",
        "action",
        "attempt",
        "binding",
        "offer_digest",
        "contract",
        "consent_hash",
        "created_at_ic",
        "created_at_utc",
        "state",
    ] {
        if field(&market, name)? != field(&roll, name)? {
            return Err(refused("renewal paired identity differs"));
        }
    }
    let offer = field(&market, "offer")?;
    let basis = field(offer, "basis")?;
    let terms = field(offer, "terms")?;
    let grant = field(binding, "grant")?;
    let template = field(grant, "renewal")?;
    let prior = required(view, &term_key(TERM, licence, predecessor)?)?;
    let economics = required(view, &term_key(ECONOMICS, licence, predecessor)?)?;
    let prior_operation = token(field(&prior, "operation")?)?;
    if operation == prior_operation
        || operation == token(field(binding, "birth_operation")?)?
        || token(field(binding, "predecessor_operation")?)? != prior_operation
        || field(&prior, "licence")? != field(binding, "licence")?
        || field(&prior, "generation")? != field(binding, "predecessor")?
        || field(&economics, "licence")? != field(binding, "licence")?
        || field(&economics, "generation")? != field(binding, "predecessor")?
        || field(&economics, "operation")? != field(&prior, "operation")?
        || field(&economics, "product_digest")? != field(binding, "product_digest")?
        || field(&economics, "grant")? != grant
        || field(binding, "predecessor_term")?
            != &json!(digest("roll.mind.renewal_predecessor.v1", &prior)?)
        || field(binding, "predecessor_economics")?
            != &json!(digest("roll.mind.paid_term_economics.v1", &economics)?)
        || field(&market, "offer_digest")?
            != &json!(digest("economy.mind_renewal_offer.v1", offer)?)
        || number(basis, "licence")? != licence
        || number(basis, "predecessor_generation")? != predecessor
        || number(basis, "next_generation")? != next
        || field(basis, "predecessor_operation")? != field(&prior, "operation")?
        || field(basis, "product_digest")? != field(binding, "product_digest")?
        || field(basis, "template_digest")?
            != &json!(digest("roll.mind.renewal_template.v1", template)?)
        || field(basis, "grant_economics_digest")?
            != &json!(digest("roll.mind.grant_economics.v1", grant)?)
        || field(basis, "royalty_chain_digest")? != field(field(grant, "royalty_chain")?, "digest")?
        || field(terms, "buyer")? != field(binding, "holder")?
        || number(terms, "charter")? != positive_string(field(binding, "charter")?)?
        || field(terms, "instrument")? != field(template, "instrument")?
        || field(terms, "price")? != field(template, "price")?
        || field(template, "terms")? != field(binding, "terms")?
        || field(template, "term_seconds")? != field(binding, "term_seconds")?
        || field(template, "grace_seconds")? != field(binding, "grace_seconds")?
        || field(template, "revision")?
            != &json!({"kind":"pin","filed":field(binding,"filed")?,"revision":field(binding,"revision")?})
        || field(&prior, "terms")? != field(binding, "terms")?
        || field(&prior, "revision")? != field(template, "revision")?
    {
        return Err(refused("renewal predecessor or disclosed terms differ"));
    }
    stamp(field(&market, "created_at_ic")?)?;
    let pair = Pair {
        market,
        roll,
        licence,
        predecessor,
        next,
        phase,
    };
    if field(&pair.market, "action")? != &json!(historical_action(&pair, 0)?) {
        return Err(refused("renewal historical Draft differs"));
    }
    let attempt = field(&pair.market, "attempt")?;
    match text(attempt, "kind")? {
        "automatic" if attempt.as_object().is_some_and(|row| row.len() == 1) => (),
        "buyer_retry"
            if attempt.as_object().is_some_and(|row| row.len() == 2)
                && token(field(attempt, "previous_operation")?)? != operation =>
        {
            ()
        }
        _ => return Err(refused("renewal attempt shape")),
    }
    Ok(pair)
}
fn fingerprint(pair: &Pair) -> Result<DefinitionDigest, ActionStateError> {
    let mut values = [pair.market.clone(), pair.roll.clone()];
    for value in &mut values {
        value
            .as_object_mut()
            .ok_or_else(|| refused("renewal record shape"))?
            .remove("state");
    }
    digest("core.mind_renewal.capacity.v1", &json!(values))
}
fn targets(operation: CommitToken, pair: &Pair) -> Result<Vec<Target>, ActionStateError> {
    let mut rows = Vec::with_capacity(7);
    for (_, family, ceiling) in FAMILIES {
        let key = match family {
            HEAD | LICENCE => licence_key(family, pair.licence)?,
            TERM | ECONOMICS => term_key(family, pair.licence, pair.next)?,
            _ => operation_key(family, operation)?,
        };
        rows.push(Target {
            key,
            schema: CanonicalId::new(format!("{family}.value.v1"))
                .map_err(|_| refused("renewal target schema"))?,
            schema_version: 1,
            ceiling,
            mode: if matches!(family, TERM | ECONOMICS | RECEIPT) {
                SlotMode::Create
            } else {
                SlotMode::UpdateExisting
            },
        });
    }
    rows.sort_by(|a, b| {
        (&a.key.owner, &a.key.family, &a.key.encoded).cmp(&(
            &b.key.owner,
            &b.key.family,
            &b.key.encoded,
        ))
    });
    Ok(rows)
}

fn receipt_action(value: &Value, index: usize) -> Result<(), ActionStateError> {
    let action: DefinitionRef = serde_json::from_value(field(value, "action")?.clone())
        .map_err(|_| refused("renewal receipt action"))?;
    if action.key().owner.as_str() != "market"
        || action.key().kind.as_str() != "action"
        || action.key().id.as_str() != action_id(index)
        || action.version() != 1
    {
        return Err(refused("renewal receipt action identity"));
    }
    Ok(())
}
fn state_receipts(pair: &Pair, operation: CommitToken) -> Result<(), ActionStateError> {
    let state = pair.state()?;
    let created = stamp(field(&pair.market, "created_at_ic")?)?;
    let created_utc = signed(&pair.market, "created_at_utc")?;
    if pair.phase == "drafted" {
        return Ok(());
    }
    if pair.phase == "unfunded_service_closed" {
        let closure = field(state, "closure")?;
        let closed = stamp(field(closure, "at_ic")?)?;
        let expires = stamp(field(
            field(field(&pair.market, "offer")?, "terms")?,
            "expires_at",
        )?)?;
        receipt_action(closure, 5)?;
        let commit = token(field(closure, "commit")?)?;
        if !state.as_object().is_some_and(|row| row.len() == 2)
            || !closure.as_object().is_some_and(|row| row.len() == 7)
            || number(closure, "format")? != 1
            || field(closure, "actor")? != &json!(crate::GOD)
            || commit == operation
            || commit == token(field(pair.binding()?, "birth_operation")?)?
            || commit == token(field(pair.binding()?, "predecessor_operation")?)?
            || signed(closure, "posted_at_utc")? < created_utc
            || expires.clock != created.clock
            || expires.at < created.at
        {
            return Err(refused("renewal service closure identity"));
        }
        match text(closure, "reason")? {
            "deadline_reached" if closed.clock == created.clock && closed.at >= expires.at => (),
            "clock_invalidated" if closed.clock != created.clock => (),
            _ => return Err(refused("renewal service closure cause or clock")),
        }
        return Ok(());
    }
    if pair.phase == "unfunded_closed" {
        let closure = field(state, "closure")?;
        receipt_action(closure, 4)?;
        stamp(field(closure, "at_ic")?)?;
        if token(field(closure, "commit")?)? == operation
            || signed(closure, "posted_at_utc")? < created_utc
            || !matches!(text(closure, "reason")?, "declined" | "expired")
        {
            return Err(refused("renewal unfunded closure identity"));
        }
        return Ok(());
    }
    let fund = field(state, "funding")?;
    let fund_commit = token(field(fund, "commit")?)?;
    let funded_at = stamp(field(fund, "at_ic")?)?;
    let terms = field(field(&pair.market, "offer")?, "terms")?;
    let start = stamp(field(terms, "not_before")?)?;
    let expiry = stamp(field(terms, "expires_at")?)?;
    if number(fund, "format")? != 1
        || fund_commit == operation
        || fund_commit == token(field(pair.binding()?, "predecessor_operation")?)?
        || field(fund, "action")? != &json!(historical_action(pair, 1)?)
        || field(fund, "contract")? != field(&pair.market, "contract")?
        || field(fund, "consent_hash")? != field(&pair.market, "consent_hash")?
        || signed(fund, "posted_at_utc")? < created_utc
        || funded_at.clock != created.clock
        || funded_at.at < created.at
        || start.clock != funded_at.clock
        || expiry.clock != funded_at.clock
        || funded_at.at < start.at
        || funded_at.at >= expiry.at
    {
        return Err(refused("renewal funding receipt identity or interval"));
    }
    if pair.phase == "funded" {
        return Ok(());
    }
    let terminal = field(state, "terminal")?;
    let commit = token(field(terminal, "commit")?)?;
    let terminal_at = stamp(field(terminal, "at_ic")?)?;
    receipt_action(terminal, if pair.phase == "granted" { 2 } else { 3 })?;
    if number(terminal, "format")? != 1
        || commit == operation
        || commit == fund_commit
        || commit == token(field(pair.binding()?, "predecessor_operation")?)?
        || field(terminal, "transaction")? == field(fund, "transaction")?
        || number(terminal, "journal_seq")? <= number(fund, "journal_seq")?
        || signed(terminal, "posted_at_utc")? < signed(fund, "posted_at_utc")?
        || text(terminal, "outcome")? != pair.phase
    {
        return Err(refused("renewal terminal receipt identity"));
    }
    if pair.phase == "granted" {
        if field(terminal, "action")? != &json!(historical_action(pair, 2)?)
            || field(terminal, "issued_at_ic")? != field(terminal, "at_ic")?
            || terminal_at.clock != funded_at.clock
            || terminal_at.at < funded_at.at
            || terminal_at.at >= expiry.at
        {
            return Err(refused("renewal Grant receipt interval"));
        }
    } else if !field(terminal, "issued_at_ic")?.is_null() {
        return Err(refused("renewal Refund conveys no issued term"));
    }
    Ok(())
}
fn current_licence(view: &View<'_>, pair: &Pair) -> Result<Value, ActionStateError> {
    let licence = required(view, &licence_key(LICENCE, pair.licence)?)?;
    let binding = pair.binding()?;
    for name in ["holder", "charter", "line"] {
        if field(&licence, name)? != field(binding, name)? {
            return Err(refused("renewal immutable licence identity"));
        }
    }
    if number(&licence, "format")? != 1
        || field(&licence, "id")? != field(binding, "licence")?
        || field(&licence, "operation")? != field(binding, "birth_operation")?
    {
        return Err(refused("renewal immutable licence birth"));
    }
    Ok(licence)
}
fn expected_head(pair: &Pair, operation: CommitToken) -> Result<Value, ActionStateError> {
    let binding = pair.binding()?;
    Ok(
        json!({"format":1,"licence":field(binding,"licence")?,"holder":field(binding,"holder")?,
        "birth_operation":field(binding,"birth_operation")?,
        "current": if pair.phase=="granted" {pair.next.to_string()} else {pair.predecessor.to_string()},
        "attempted_predecessor":pair.predecessor.to_string(),"last_operation":operation,
        "pending":if pair.pending() {json!(operation)} else {Value::Null}}),
    )
}
fn outputs(
    view: &View<'_>,
    operation: CommitToken,
    pair: &Pair,
    at_transition: bool,
) -> Result<(), ActionStateError> {
    state_receipts(pair, operation)?;
    let licence = current_licence(view, pair)?;
    let current = positive_string(field(&licence, "term")?)?;
    let receipt = value(view, &operation_key(RECEIPT, operation)?)?;
    let term = value(view, &term_key(TERM, pair.licence, pair.next)?)?;
    let economics = value(view, &term_key(ECONOMICS, pair.licence, pair.next)?)?;
    if pair.pending() {
        if receipt.is_some()
            || term.is_some()
            || economics.is_some()
            || current != pair.predecessor
            || required(view, &licence_key(HEAD, pair.licence)?)? != expected_head(pair, operation)?
        {
            return Err(refused("renewal pending outputs or current pointer"));
        }
        return Ok(());
    }
    let binding = pair.binding()?;
    let expected = json!({"format":1,"operation":operation,"licence":field(binding,"licence")?,
        "predecessor":field(binding,"predecessor")?,"next":field(binding,"next")?,
        "contract":field(&pair.market,"contract")?,"consent_hash":field(&pair.market,"consent_hash")?,"state":pair.state()?});
    if receipt.as_ref() != Some(&expected) {
        return Err(refused("renewal exact terminal receipt missing"));
    }
    if at_transition
        && required(view, &licence_key(HEAD, pair.licence)?)? != expected_head(pair, operation)?
    {
        return Err(refused("renewal terminal head differs"));
    }
    if pair.phase != "granted" {
        // A later linked retry may own the same next-generation key. Never
        // attribute its eventual Grant to this older refunded/closed attempt.
        if current < pair.predecessor
            || (at_transition
                && (term.is_some() || economics.is_some() || current != pair.predecessor))
        {
            return Err(refused("renewal non-Grant published a new generation"));
        }
        for row in [&term, &economics].into_iter().flatten() {
            if token(field(row, "operation")?)? == operation {
                return Err(refused("closed renewal owns a grant row"));
            }
        }
        return Ok(());
    }
    let issued = stamp(field(field(pair.state()?, "terminal")?, "issued_at_ic")?)?.at;
    let end = issued
        .checked_add(signed(binding, "term_seconds")?)
        .ok_or_else(|| refused("renewal term overflow"))?;
    let grace = end
        .checked_add(signed(binding, "grace_seconds")?)
        .ok_or_else(|| refused("renewal grace overflow"))?;
    let expected_term = json!({"format":1,"licence":field(binding,"licence")?,"generation":field(binding,"next")?,
        "terms":field(binding,"terms")?,"revision":{"kind":"pin","filed":field(binding,"filed")?,"revision":field(binding,"revision")?},
        "window":{"format":1,"eligibility_start":issued,"issued_at":issued,"term_end":end,"grace_end":grace},"operation":operation});
    let expected_economics = json!({"format":1,"licence":field(binding,"licence")?,"generation":field(binding,"next")?,
        "operation":operation,"product_digest":field(binding,"product_digest")?,"grant":field(binding,"grant")?});
    if term.as_ref() != Some(&expected_term)
        || economics.as_ref() != Some(&expected_economics)
        || current < pair.next
        || (at_transition && current != pair.next)
    {
        return Err(refused("renewal granted term or economics differ"));
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
        .mind_renewal
        .as_ref()
        .ok_or_else(|| refused("renewal protocol unavailable"))?;
    let pair = pair(view, reservation.funding)?;
    if reservation.effect != historical_action(&pair, 0)?
        || reservation.sponsor != operation_key(MARKET, reservation.funding)?
        || reservation.targets != targets(reservation.funding, &pair)?
        || reservation.protocol_fingerprint != Some(fingerprint(&pair)?)
        || (matches!(phase, Phase::Funding)
            && (pair.phase != "drafted" || reservation.effect != *protocol.admission_action()))
        || (matches!(phase, Phase::Terminal)
            && pair.phase == "granted"
            && reservation.effect != *protocol.admission_action())
        || match phase {
            Phase::Funding => false,
            Phase::Retained => !pair.pending(),
            Phase::Terminal => pair.pending(),
        }
    {
        return Err(refused("renewal reservation or phase differs"));
    }
    outputs(view, reservation.funding, &pair, true)
}
pub(in crate::action_state) fn renewal_change(
    txn: &mut ActionTxn<'_>,
    commit: CommitToken,
    operation: Option<CommitToken>,
) -> Result<(), ActionStateError> {
    let protocol = txn
        .store
        .registry
        .mind_renewal
        .as_ref()
        .ok_or_else(|| refused("renewal protocol unavailable"))?;
    let identity = txn
        .registered_identity
        .as_ref()
        .ok_or_else(|| refused("renewal actual invocation unavailable"))?;
    if identity.commit != commit
        || txn.provider_ordinal != Some(identity.ordinal)
        || txn.owner != identity.action.key().owner
        || txn.capacity_changes.funding.is_some()
        || txn.capacity_changes.progress.is_some()
        || txn.capacity_changes.terminal.is_some()
    {
        return Err(refused("renewal preparation identity differs"));
    }
    let action = protocol.action_index(identity)?;
    if let Some(operation) = operation {
        if commit == operation || action == 0 {
            return Err(refused("renewal advance identity differs"));
        }
        let id = ReservationId(txn.owner.clone(), operation.to_hex());
        if action == 1 {
            txn.capacity_changes.progress = Some(id);
        } else {
            txn.capacity_changes.terminal = Some(id);
        }
    } else {
        if action != 0 {
            return Err(refused("renewal Draft action differs"));
        }
        let view = View {
            registry: &txn.store.registry,
            records: &txn.store.records,
            mutations: Some(&txn.mutations),
        };
        let pair = pair(&view, commit)?;
        let record = Reservation {
            format: 1,
            funding: commit,
            effect: protocol.admission_action().clone(),
            sponsor: operation_key(MARKET, commit)?,
            targets: targets(commit, &pair)?,
            protocol_fingerprint: Some(fingerprint(&pair)?),
        };
        verify(Phase::Funding, &record, &view)?;
        txn.capacity_changes.funding = Some(record);
    }
    Ok(())
}

fn protected(key: &StateKey) -> bool {
    FAMILIES
        .iter()
        .any(|(owner, family, _)| key.owner.as_str() == *owner && key.family.as_str() == *family)
}
fn exact_mutations(
    mutations: &BTreeMap<StateKey, TxnMutation>,
    view: &View<'_>,
    keys: Vec<Key>,
) -> Result<(), ActionStateError> {
    let expected: BTreeSet<_> = keys
        .into_iter()
        .map(|key| key.resolve(view.registry))
        .collect::<Result<_, _>>()?;
    if expected.len() != mutations.len()
        || mutations.iter().any(|(key, mutation)| {
            !expected.contains(key) || !matches!(mutation, TxnMutation::Put { .. })
        })
    {
        return Err(refused("renewal requires exact closed postimages"));
    }
    Ok(())
}
fn attempt_before(
    before: &View<'_>,
    after_pair: &Pair,
    operation: CommitToken,
) -> Result<(), ActionStateError> {
    let current = current_licence(before, after_pair)?;
    if positive_string(field(&current, "term")?)? != after_pair.predecessor {
        return Err(refused("renewal Draft predecessor is not current"));
    }
    let head = value(before, &licence_key(HEAD, after_pair.licence)?)?;
    let attempt = field(&after_pair.market, "attempt")?;
    match (text(attempt, "kind")?, head) {
        ("automatic", None) if after_pair.predecessor == 1 => (),
        ("automatic", Some(head)) => {
            if !field(&head, "pending")?.is_null()
                || positive_string(field(&head, "current")?)? != after_pair.predecessor
                || positive_string(field(&head, "attempted_predecessor")?)?
                    >= after_pair.predecessor
            {
                return Err(refused("renewal automatic offer already attempted"));
            }
        }
        ("buyer_retry", Some(head)) => {
            let previous = token(field(attempt, "previous_operation")?)?;
            let prior = pair(before, previous)?;
            if previous == operation
                || field(&head, "last_operation")? != field(attempt, "previous_operation")?
                || !field(&head, "pending")?.is_null()
                || positive_string(field(&head, "current")?)? != after_pair.predecessor
                || positive_string(field(&head, "attempted_predecessor")?)?
                    != after_pair.predecessor
                || !matches!(
                    prior.phase.as_str(),
                    "refunded" | "unfunded_closed" | "unfunded_service_closed"
                )
            {
                return Err(refused(
                    "renewal retry lacks exact latest terminal predecessor",
                ));
            }
            for name in ["licence", "holder", "birth_operation", "predecessor"] {
                if field(prior.binding()?, name)? != field(after_pair.binding()?, name)? {
                    return Err(refused("renewal retry crosses identity"));
                }
            }
            let closed = if prior.phase == "refunded" {
                field(prior.state()?, "terminal")?
            } else {
                field(prior.state()?, "closure")?
            };
            if signed(&after_pair.market, "created_at_utc")? < signed(closed, "posted_at_utc")? {
                return Err(refused("renewal retry predates closure"));
            }
            outputs(before, previous, &prior, false)?;
        }
        _ => return Err(refused("renewal attempt cannot replace pending custody")),
    }
    Ok(())
}
pub(super) fn validate_changes(
    store: &ActionStateStore,
    mutations: &BTreeMap<StateKey, TxnMutation>,
    changes: &Changes,
    identity: Option<&RegisteredTxnIdentity>,
) -> Result<(), ActionStateError> {
    let Some(protocol) = &store.registry.mind_renewal else {
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
    let mut touches = mutations
        .keys()
        .any(|key| matches!(key.family.as_str(), MARKET | ROLL | HEAD | RECEIPT));
    for key in mutations.keys().filter(|key| {
        key.owner.as_str() == "roll" && matches!(key.family.as_str(), TERM | ECONOMICS)
    }) {
        let raw = Key {
            owner: key.owner.clone(),
            family: key.family.clone(),
            encoded: key.encoded.clone(),
        };
        for view in [&before, &after] {
            if value(view, &raw)?.is_some() && shared_output_is_renewal(view, key)? {
                touches = true;
            }
        }
    }
    if !touches && changes.progress.is_none() {
        return Ok(());
    }
    let identity = identity.ok_or_else(|| refused("renewal write lacks actual invocation"))?;
    let action = protocol.action_index(identity)?;
    let operation = if action == 0 {
        identity.commit
    } else {
        let id = if action == 1 {
            changes.progress.as_ref()
        } else {
            changes.terminal.as_ref()
        }
        .ok_or_else(|| refused("renewal reservation transition missing"))?;
        if id.0.as_str() != "market" {
            return Err(refused("renewal reservation owner"));
        }
        token(&json!(id.1))?
    };
    let next = pair(&after, operation)?;
    if identity.principal != (PrincipalId::Object { id: identity.actor }) {
        return Err(refused("renewal actual principal differs"));
    }
    if action == 5 {
        if identity.actor != crate::GOD {
            return Err(refused("renewal actual service identity differs"));
        }
    } else if number(next.binding()?, "holder")? != u64::from(identity.actor.0) {
        return Err(refused("renewal actual buyer identity differs"));
    }
    let mut keys = vec![
        operation_key(MARKET, operation)?,
        operation_key(ROLL, operation)?,
    ];
    let id = ReservationId(
        OwnerId::new("market").map_err(|_| refused("renewal owner"))?,
        operation.to_hex(),
    );
    if action == 0 {
        if value(&before, &operation_key(MARKET, operation)?)?.is_some()
            || value(&before, &operation_key(ROLL, operation)?)?.is_some()
            || changes.funding.as_ref().map(|record| record.funding) != Some(operation)
        {
            return Err(refused("renewal Draft identity reused"));
        }
        attempt_before(&before, &next, operation)?;
        keys.push(licence_key(HEAD, next.licence)?);
        exact_mutations(mutations, &after, keys)?;
        verify(
            Phase::Funding,
            changes
                .funding
                .as_ref()
                .ok_or_else(|| refused("renewal reserve missing"))?,
            &after,
        )?;
        return Ok(());
    }
    let prior = pair(&before, operation)?;
    let stored = store
        .capacity
        .records
        .get(&id)
        .ok_or_else(|| refused("renewal acknowledged reservation missing"))?;
    verify(Phase::Retained, &stored.record, &before)?;
    if fingerprint(&prior)? != fingerprint(&next)? || identity.commit == operation {
        return Err(refused("renewal immutable consent changed"));
    }
    let expected_phase = match action {
        1 => "funded",
        2 => "granted",
        3 => "refunded",
        4 => "unfunded_closed",
        5 => "unfunded_service_closed",
        _ => return Err(refused("renewal transition")),
    };
    if next.phase != expected_phase
        || prior.phase
            != if matches!(action, 1 | 4 | 5) {
                "drafted"
            } else {
                "funded"
            }
    {
        return Err(refused("renewal state transition differs"));
    }
    let receipt = match action {
        1 => field(next.state()?, "funding")?,
        4 | 5 => field(next.state()?, "closure")?,
        _ => field(next.state()?, "terminal")?,
    };
    if token(field(receipt, "commit")?)? != identity.commit
        || field(receipt, "action")? != &json!(identity.action)
        || (matches!(action, 1 | 2)
            && field(&next.market, "action")? != &json!(protocol.admission_action()))
    {
        return Err(refused("renewal actual update or active protocol differs"));
    }
    if matches!(action, 2 | 3)
        && field(prior.state()?, "funding")? != field(next.state()?, "funding")?
    {
        return Err(refused("renewal accepted funding replaced"));
    }
    if action == 1 {
        if changes.progress.as_ref() != Some(&id) {
            return Err(refused("renewal Fund must retain reservation"));
        }
        exact_mutations(mutations, &after, keys)?;
        verify(Phase::Retained, &stored.record, &after)?;
    } else {
        if changes.terminal.as_ref() != Some(&id) {
            return Err(refused("renewal terminal must consume reservation"));
        }
        keys.extend([
            licence_key(HEAD, next.licence)?,
            operation_key(RECEIPT, operation)?,
        ]);
        let old_licence = current_licence(&before, &prior)?;
        let mut expected_licence = old_licence.clone();
        if action == 2 {
            expected_licence
                .as_object_mut()
                .ok_or_else(|| refused("renewal licence shape"))?
                .insert("term".into(), json!(next.next.to_string()));
            keys.extend([
                licence_key(LICENCE, next.licence)?,
                term_key(TERM, next.licence, next.next)?,
                term_key(ECONOMICS, next.licence, next.next)?,
            ]);
        }
        if current_licence(&after, &next)? != expected_licence {
            return Err(refused("renewal changed immutable licence data"));
        }
        exact_mutations(mutations, &after, keys)?;
        verify(Phase::Terminal, &stored.record, &after)?;
    }
    Ok(())
}

pub(super) fn shared_output_is_renewal(
    view: &View<'_>,
    state_key: &StateKey,
) -> Result<bool, ActionStateError> {
    if state_key.owner.as_str() != "roll" || !matches!(state_key.family.as_str(), TERM | ECONOMICS)
    {
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
    if optional_family_value(view, ROLL, operation)?.is_none() {
        return Ok(false);
    }
    if view.registry.mind_renewal.is_none() {
        return Err(refused("renewal grant without registered protocol"));
    }
    require_disjoint_operation(view, operation)?;
    let pair = pair(view, operation)?;
    if pair.phase != "granted"
        || field(&row, "licence")? != field(pair.binding()?, "licence")?
        || field(&row, "generation")? != field(pair.binding()?, "next")?
        || *state_key
            != term_key(state_key.family.as_str(), pair.licence, pair.next)?
                .resolve(view.registry)?
    {
        return Err(refused("renewal extra or crossed grant row"));
    }
    Ok(true)
}
/// Old purchase audits retain their exact birth proof. Only the separate
/// registered renewal protocol can explain a newer current pointer.
pub(super) fn validate_current_birth(
    view: &View<'_>,
    licence: &Value,
    birth: CommitToken,
) -> Result<(), ActionStateError> {
    if token(field(licence, "operation")?)? != birth {
        return Err(refused("paid immutable birth operation"));
    }
    let current = positive_string(field(licence, "term")?)?;
    if current == 1 {
        return Ok(());
    }
    if view.registry.mind_renewal.is_none() {
        return Err(refused("new paid generation lacks renewal protocol"));
    }
    let id = positive_string(field(licence, "id")?)?;
    let head = required(view, &licence_key(HEAD, id)?)?;
    let term = required(view, &term_key(TERM, id, current)?)?;
    let operation = token(field(&term, "operation")?)?;
    let pair = pair(view, operation)?;
    if pair.phase != "granted"
        || pair.next != current
        || pair.licence != id
        || field(&head, "current")? != field(licence, "term")?
        || field(&head, "birth_operation")? != &json!(birth)
        || field(pair.binding()?, "birth_operation")? != &json!(birth)
    {
        return Err(refused("paid current generation lacks exact renewal Grant"));
    }
    outputs(view, operation, &pair, false)
}
/// Only the complete closed renewal transition may update the existing licence.
/// The renewal validator still checks its exact before/after bytes and targets.
pub(super) fn owns_licence_change(
    view: &View<'_>,
    key: &StateKey,
    identity: Option<&RegisteredTxnIdentity>,
    changes: &Changes,
) -> Result<bool, ActionStateError> {
    if key.owner.as_str() != "roll" || key.family.as_str() != LICENCE {
        return Ok(false);
    }
    let Some(protocol) = &view.registry.mind_renewal else {
        return Ok(false);
    };
    let Some(identity) = identity else {
        return Ok(false);
    };
    if protocol.action_index(identity).ok() != Some(2) {
        return Ok(false);
    }
    let Some(id) = &changes.terminal else {
        return Err(refused("renewal licence update lacks terminal reservation"));
    };
    let operation = token(&json!(id.1))?;
    let pair = pair(view, operation)?;
    if id.0.as_str() != "market"
        || pair.phase != "granted"
        || *key != licence_key(LICENCE, pair.licence)?.resolve(view.registry)?
        || token(field(field(pair.state()?, "terminal")?, "commit")?)? != identity.commit
    {
        return Err(refused("renewal licence update crosses operation"));
    }
    Ok(true)
}
pub(super) fn reject_other_operation(
    view: &View<'_>,
    operation: CommitToken,
) -> Result<(), ActionStateError> {
    if optional_family_value(view, ROLL, operation)?.is_some()
        || optional_family_value(view, MARKET, operation)?.is_some()
    {
        return Err(refused("purchase operation also names a renewal"));
    }
    Ok(())
}
fn own_row_operation(
    view: &View<'_>,
    state_key: &StateKey,
) -> Result<CommitToken, ActionStateError> {
    let row = required(
        view,
        &Key {
            owner: state_key.owner.clone(),
            family: state_key.family.clone(),
            encoded: state_key.encoded.clone(),
        },
    )?;
    let operation = token(field(&row, "operation")?)?;
    if *state_key != operation_key(state_key.family.as_str(), operation)?.resolve(view.registry)? {
        return Err(refused("renewal record key differs"));
    }
    Ok(operation)
}
fn audit_attempts(
    view: &View<'_>,
    operations: &BTreeMap<CommitToken, Pair>,
    heads: &BTreeSet<u64>,
) -> Result<(), ActionStateError> {
    let mut groups: BTreeMap<u64, BTreeMap<u64, Vec<CommitToken>>> = BTreeMap::new();
    for (operation, pair) in operations {
        groups
            .entry(pair.licence)
            .or_default()
            .entry(pair.predecessor)
            .or_default()
            .push(*operation);
    }
    if groups.keys().copied().collect::<BTreeSet<_>>() != *heads {
        return Err(refused("renewal head reverse completeness"));
    }
    for (licence, generations) in groups {
        let mut current = 1_u64;
        let mut last = None;
        let mut closed = false;
        let mut birth_identity = None;
        for (predecessor, ids) in generations {
            if closed || predecessor != current {
                return Err(refused("renewal generations are not consecutive Grants"));
            }
            let mut root = None;
            let mut successors = BTreeMap::new();
            for operation in &ids {
                let pair = operations
                    .get(operation)
                    .ok_or_else(|| refused("renewal attempt index"))?;
                let binding = pair.binding()?;
                let identity = json!([
                    field(binding, "holder")?,
                    field(binding, "birth_operation")?,
                    field(binding, "charter")?,
                    field(binding, "line")?
                ]);
                if birth_identity.as_ref().is_some_and(|old| old != &identity) {
                    return Err(refused("renewal historical buyer or birth drift"));
                }
                birth_identity = Some(identity);
                match text(field(&pair.market, "attempt")?, "kind")? {
                    "automatic" => {
                        if root.replace(*operation).is_some() {
                            return Err(refused("renewal repeated automatic predecessor"));
                        }
                    }
                    "buyer_retry" => {
                        let previous = token(field(
                            field(&pair.market, "attempt")?,
                            "previous_operation",
                        )?)?;
                        if successors.insert(previous, *operation).is_some() {
                            return Err(refused("renewal retry branch"));
                        }
                    }
                    _ => return Err(refused("renewal attempt kind")),
                }
            }
            let mut cursor = root.ok_or_else(|| refused("renewal automatic root missing"))?;
            let mut visited = BTreeSet::new();
            loop {
                if !visited.insert(cursor) {
                    return Err(refused("renewal retry cycle"));
                }
                let pair = operations
                    .get(&cursor)
                    .ok_or_else(|| refused("renewal retry predecessor missing"))?;
                if pair.licence != licence || pair.predecessor != predecessor {
                    return Err(refused("renewal retry crossed licence or generation"));
                }
                if let Some(next) = successors.get(&cursor) {
                    if !matches!(
                        pair.phase.as_str(),
                        "refunded" | "unfunded_closed" | "unfunded_service_closed"
                    ) {
                        return Err(refused("renewal retry after pending or granted operation"));
                    }
                    let retry = operations
                        .get(next)
                        .ok_or_else(|| refused("renewal retry successor missing"))?;
                    let closed = if pair.phase == "refunded" {
                        field(pair.state()?, "terminal")?
                    } else {
                        field(pair.state()?, "closure")?
                    };
                    if signed(&retry.market, "created_at_utc")? < signed(closed, "posted_at_utc")? {
                        return Err(refused("renewal retry predates closure"));
                    }
                    cursor = *next;
                } else {
                    if visited.len() != ids.len() {
                        return Err(refused("renewal orphan or disconnected retry"));
                    }
                    if pair.phase == "granted" {
                        current = pair.next;
                    } else {
                        closed = true;
                    }
                    last = Some(cursor);
                    break;
                }
            }
        }
        let last = last.ok_or_else(|| refused("renewal head has no attempts"))?;
        let pair = operations
            .get(&last)
            .ok_or_else(|| refused("renewal last attempt"))?;
        if required(view, &licence_key(HEAD, licence)?)? != expected_head(pair, last)?
            || positive_string(field(&current_licence(view, pair)?, "term")?)? != current
        {
            return Err(refused("renewal head is not exact final linked attempt"));
        }
    }
    Ok(())
}
pub(super) fn audit_boot(
    view: &View<'_>,
    reservations: &BTreeMap<ReservationId, Arc<StoredReservation>>,
) -> Result<(), ActionStateError> {
    let own = |key: &StateKey| {
        matches!(
            (key.owner.as_str(), key.family.as_str()),
            ("market", MARKET | RECEIPT) | ("roll", ROLL | HEAD)
        )
    };
    if view.registry.mind_renewal.is_none() {
        if view.records.keys().any(own) {
            return Err(refused("retained renewal without registered protocol"));
        }
        return Ok(());
    }
    let mut ids = BTreeSet::new();
    let mut heads = BTreeSet::new();
    for key in view.records.keys().filter(|key| protected(key)) {
        match key.family.as_str() {
            MARKET | ROLL | RECEIPT => {
                ids.insert(own_row_operation(view, key)?);
            }
            HEAD => {
                let row = required(
                    view,
                    &Key {
                        owner: key.owner.clone(),
                        family: key.family.clone(),
                        encoded: key.encoded.clone(),
                    },
                )?;
                let licence = positive_string(field(&row, "licence")?)?;
                if *key != licence_key(HEAD, licence)?.resolve(view.registry)?
                    || !heads.insert(licence)
                {
                    return Err(refused("renewal duplicate or crossed head"));
                }
                ids.insert(token(field(&row, "last_operation")?)?);
            }
            TERM | ECONOMICS => {
                if shared_output_is_renewal(view, key)? {
                    let row = required(
                        view,
                        &Key {
                            owner: key.owner.clone(),
                            family: key.family.clone(),
                            encoded: key.encoded.clone(),
                        },
                    )?;
                    ids.insert(token(field(&row, "operation")?)?);
                }
            }
            _ => (),
        }
    }
    let mut operations = BTreeMap::new();
    for id in ids {
        let pair = pair(view, id)?;
        let reservation = reservations.get(&ReservationId(
            OwnerId::new("market").map_err(|_| refused("renewal owner"))?,
            id.to_hex(),
        ));
        match (pair.pending(), reservation) {
            (true, Some(row)) => verify(Phase::Retained, &row.record, view)?,
            (false, None) => outputs(view, id, &pair, false)?,
            _ => return Err(refused("renewal reverse reservation completeness")),
        }
        operations.insert(id, pair);
    }
    audit_attempts(view, &operations, &heads)
}

#[cfg(test)]
mod tests;
