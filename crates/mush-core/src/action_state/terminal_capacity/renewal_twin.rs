//! Closed seven-target renewal completion, written once for both issuers (doc 46 CT.3).
//! Money and current issuer authority remain actual owner obligations; core binds paired
//! images and future room.
//!
//! The personal renewal and the House renewal are this one validator over a sealed
//! [`Issuer`], resolved at compile time: [`Personal`] is `renewal.rs` and [`House`] is
//! `house_renewal.rs`, and nothing else can be a third. What the two share is here. What
//! differs is the trait: every identity string as an associated constant, and the birth
//! proof each issuer owes as its methods. The registry's fields, its builder methods and
//! the enums that name each family stay closed and hand-edited; no module hands the kernel
//! a callback for this.
use super::*;
use crate::action::{ActionProviderRegistry, PrincipalId};
use crate::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistry,
    DefinitionRegistryBuilder,
};
use std::marker::PhantomData;

mod issuer;
pub(in crate::action_state::terminal_capacity) use issuer::{House, Issuer, Personal};

pub(super) const TRANSITIONS: [&str; 5] = ["draft", "fund", "grant", "refund", "unfunded_close"];
/// The licence pointer and the term rows are one family each for both issuers: a House
/// licence renews into the same paid-term family a personal one does.
pub(super) const LICENCE: &str = "roll_mind_personal_paid_licence";
pub(super) const TERM: &str = "roll_mind_paid_term";

/// The seven target families of one issuer, each with its owner and byte ceiling, in the
/// order the reservation is built.
pub(super) fn families<I: Issuer>() -> [(&'static str, &'static str, usize); 7] {
    [
        ("market", I::MARKET, 32768),
        ("roll", I::ROLL, 16384),
        ("roll", I::HEAD, 4096),
        ("roll", LICENCE, 4096),
        ("roll", TERM, 8192),
        ("roll", I::ECONOMICS, 8192),
        ("market", I::RECEIPT, 8192),
    ]
}

/// The entry points the rest of the kernel calls, for one issuer: `renewal.rs` expands this
/// for [`Personal`] and `house_renewal.rs` for [`House`], so `renewal::verify` and
/// `house_renewal::verify` are the two instantiations of the one [`verify`]. The kernel's
/// lists of families (the verifier rows, the boot audits, the change validators, the
/// registry's fields) name each module by hand, as they always have.
macro_rules! entry_points {
    ($issuer:ty) => {
        /// This issuer's registered protocol.
        pub(in crate::action_state) type Protocol = renewal_twin::Protocol<$issuer>;

        /// The reservation's verifier at its three phases.
        pub(super) fn verify(
            phase: Phase,
            reservation: &Reservation,
            view: &View<'_>,
        ) -> Result<(), ActionStateError> {
            renewal_twin::verify::<$issuer>(phase, reservation, view)
        }
        /// Reserve at a Draft, or name the reservation a later transition advances.
        pub(in crate::action_state) fn renewal_change(
            txn: &mut ActionTxn<'_>,
            commit: CommitToken,
            operation: Option<CommitToken>,
        ) -> Result<(), ActionStateError> {
            renewal_twin::renewal_change::<$issuer>(txn, commit, operation)
        }
        /// Hold a transaction that touches this issuer's families to its one transition.
        pub(super) fn validate_changes(
            store: &ActionStateStore,
            mutations: &BTreeMap<StateKey, TxnMutation>,
            changes: &Changes,
            identity: Option<&RegisteredTxnIdentity>,
        ) -> Result<(), ActionStateError> {
            renewal_twin::validate_changes::<$issuer>(store, mutations, changes, identity)
        }
        /// Whether a term or economics row is a Grant of this issuer's renewal.
        pub(super) fn shared_output_is_renewal(
            view: &View<'_>,
            state_key: &StateKey,
        ) -> Result<bool, ActionStateError> {
            renewal_twin::shared_output_is_renewal::<$issuer>(view, state_key)
        }
        /// Old purchase audits retain their exact birth proof. Only the separate
        /// registered renewal protocol can explain a newer current pointer.
        pub(super) fn validate_current_birth(
            view: &View<'_>,
            licence: &Value,
            birth: CommitToken,
        ) -> Result<(), ActionStateError> {
            renewal_twin::validate_current_birth::<$issuer>(view, licence, birth)
        }
        /// Only the complete closed renewal transition may update the existing licence.
        pub(super) fn owns_licence_change(
            view: &View<'_>,
            key: &StateKey,
            identity: Option<&RegisteredTxnIdentity>,
            changes: &Changes,
        ) -> Result<bool, ActionStateError> {
            renewal_twin::owns_licence_change::<$issuer>(view, key, identity, changes)
        }
        /// A purchase operation may not also name a renewal of this issuer.
        pub(super) fn reject_other_operation(
            view: &View<'_>,
            operation: CommitToken,
        ) -> Result<(), ActionStateError> {
            renewal_twin::reject_other_operation::<$issuer>(view, operation)
        }
        /// The cold audit of every retained row of this issuer's families.
        pub(super) fn audit_boot(
            view: &View<'_>,
            reservations: &BTreeMap<ReservationId, Arc<StoredReservation>>,
        ) -> Result<(), ActionStateError> {
            renewal_twin::audit_boot::<$issuer>(view, reservations)
        }
    };
}
pub(super) use entry_points;

/// One issuer's registered renewal protocol. The registry holds one field an issuer
/// (`mind_renewal`, `house_mind_renewal`); the type parameter keeps either from standing
/// in the other's field, and writes nothing.
#[derive(Clone, Debug, Serialize)]
#[serde(bound = "")]
pub(in crate::action_state) struct Protocol<I> {
    manifest: DefinitionRef,
    participants: [DefinitionRef; 2],
    actions: [DefinitionRef; 5],
    #[serde(skip_serializing_if = "Option::is_none")]
    service_close: Option<DefinitionRef>,
    #[serde(skip)]
    issuer: PhantomData<fn() -> I>,
}
// `action_state` holds and calls the two instantiations by name (`RenewalProtocol`,
// `HouseRenewalProtocol`); only this layer may write code generic over the issuer.
#[expect(
    private_bounds,
    reason = "the sealed trait is deliberately narrower than the registry that holds its two protocols"
)]
impl<I: Issuer> Protocol<I> {
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
            if *action != action_reference::<I>(&manifest, &participants, index)? {
                return Err(refused("renewal complete action identity or order"));
            }
        }
        Ok(Self {
            manifest,
            participants,
            actions,
            service_close: None,
            issuer: PhantomData,
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
        if service_close != action_reference::<I>(&protocol.manifest, &protocol.participants, 5)? {
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
        I::birth_audits(registry)?;
        for (owner, family, bytes) in families::<I>() {
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
                || definition.payload() != &action_payload::<I>(index)
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
fn action_id<I: Issuer>(index: usize) -> &'static str {
    if index == 5 {
        I::SERVICE_ACTION
    } else {
        I::ACTIONS[index]
    }
}
fn action_payload<I: Issuer>(index: usize) -> Value {
    if index == 5 {
        return json!({"format":1,"protocol":I::SERVICE_PROTOCOL,
            "transition":"unfunded_service_close","service_actor":crate::GOD});
    }
    json!({"format":1,"protocol":I::PROTOCOL,"transition":TRANSITIONS[index]})
}
fn action_reference<I: Issuer>(
    manifest: &DefinitionRef,
    participants: &[DefinitionRef; 2],
    index: usize,
) -> Result<DefinitionRef, ActionStateError> {
    check_participants(participants)?;
    let descriptor = DefinitionDescriptor::new(
        DefinitionKind::new("action").map_err(|_| refused("renewal action kind"))?,
        DefinitionId::new(action_id::<I>(index)).map_err(|_| refused("renewal action id"))?,
        1,
        action_payload::<I>(index),
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
pub(super) fn field<'a>(value: &'a Value, name: &str) -> Result<&'a Value, ActionStateError> {
    value
        .get(name)
        .ok_or_else(|| refused("renewal field missing"))
}
fn text<'a>(value: &'a Value, name: &str) -> Result<&'a str, ActionStateError> {
    field(value, name)?
        .as_str()
        .ok_or_else(|| refused("renewal string field"))
}
pub(super) fn number(value: &Value, name: &str) -> Result<u64, ActionStateError> {
    field(value, name)?
        .as_u64()
        .ok_or_else(|| refused("renewal unsigned field"))
}
pub(super) fn token(value: &Value) -> Result<CommitToken, ActionStateError> {
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
        owner: OwnerId::new(if family.starts_with("market_") {
            "market"
        } else {
            "roll"
        })
        .map_err(|_| refused("renewal owner"))?,
        family: StateFamilyId::new(family).map_err(|_| refused("renewal family"))?,
        encoded: encoded.into(),
    })
}
pub(super) fn operation_key(family: &str, operation: CommitToken) -> Result<Key, ActionStateError> {
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
pub(super) fn required(view: &View<'_>, key: &Key) -> Result<Value, ActionStateError> {
    value(view, key)?.ok_or_else(|| refused("renewal paired record missing"))
}

/// One operation's paired images, the market's and the Roll's, after `pair` has held them
/// to each other and to the predecessor term.
pub(super) struct Pair {
    pub(super) market: Value,
    roll: Value,
    licence: u64,
    predecessor: u64,
    next: u64,
    phase: String,
}
impl Pair {
    pub(super) fn binding(&self) -> Result<&Value, ActionStateError> {
        field(&self.market, "binding")
    }
    fn state(&self) -> Result<&Value, ActionStateError> {
        field(&self.market, "state")
    }
    fn pending(&self) -> bool {
        matches!(self.phase.as_str(), "drafted" | "funded")
    }
}
pub(super) fn optional_family_value(
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
pub(super) fn require_disjoint_operation<I: Issuer>(
    view: &View<'_>,
    operation: CommitToken,
) -> Result<(), ActionStateError> {
    let [other_roll, other_market] = I::OTHER_ISSUER;
    for family in [
        "roll_mind_paid_purchase",
        "roll_mind_entitlement_purchase",
        "roll_mind_house_purchase",
        other_roll,
        other_market,
    ] {
        if optional_family_value(view, family, operation)?.is_some() {
            return Err(refused("renewal operation has another purchase recipe"));
        }
    }
    Ok(())
}
fn historical_action<I: Issuer>(
    pair: &Pair,
    index: usize,
) -> Result<DefinitionRef, ActionStateError> {
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
    action_reference::<I>(&manifest, &participants, index)
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
pub(super) fn licence_key(family: &str, licence: u64) -> Result<Key, ActionStateError> {
    key(family, format!("licence/{licence:016x}"))
}
pub(super) fn pair<I: Issuer>(
    view: &View<'_>,
    operation: CommitToken,
) -> Result<Pair, ActionStateError> {
    require_disjoint_operation::<I>(view, operation)?;
    let market = required(view, &operation_key(I::MARKET, operation)?)?;
    let roll = required(view, &operation_key(I::ROLL, operation)?)?;
    let binding = field(&market, "binding")?;
    let licence = positive_string(field(binding, "licence")?)?;
    let predecessor = positive_string(field(binding, "predecessor")?)?;
    let next = positive_string(field(binding, "next")?)?;
    let phase = text(field(&market, "state")?, "kind")?.to_owned();
    if number(&market, "format")? != 1
        || number(&roll, "format")? != 1
        || !matches!(number(binding, "format")?, 1..=4)
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
    let revision_policy = entitlement::binding_revision(binding)?;
    I::follow_birth(view, binding)?;
    let prior = required(view, &term_key(TERM, licence, predecessor)?)?;
    let economics = required(view, &term_key(I::ECONOMICS, licence, predecessor)?)?;
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
        || field(binding, "predecessor_term")? != &json!(digest(I::PREDECESSOR_DOMAIN, &prior)?)
        || field(binding, "predecessor_economics")?
            != &json!(digest(I::economics_domain(&economics)?, &economics)?)
        || field(&market, "offer_digest")? != &json!(digest(I::OFFER_DOMAIN, offer)?)
        || number(basis, "licence")? != licence
        || number(basis, "predecessor_generation")? != predecessor
        || number(basis, "next_generation")? != next
        || field(basis, "predecessor_operation")? != field(&prior, "operation")?
        || field(basis, "product_digest")? != field(binding, "product_digest")?
        || field(basis, "template_digest")? != &json!(digest(I::TEMPLATE_DOMAIN, template)?)
        || field(basis, "grant_economics_digest")?
            != &json!(digest(I::grant_domain(grant)?, grant)?)
        || I::royalty_chain_differs(basis, grant)?
        || field(terms, "buyer")? != field(binding, "holder")?
        || number(terms, "charter")? != positive_string(field(binding, "charter")?)?
        || field(terms, "instrument")? != field(template, "instrument")?
        || field(terms, "price")? != field(template, "price")?
        || field(template, "terms")? != field(binding, "terms")?
        || field(template, "term_seconds")? != field(binding, "term_seconds")?
        || field(template, "grace_seconds")? != field(binding, "grace_seconds")?
        || field(template, "revision")? != &revision_policy
        || field(&prior, "terms")? != field(binding, "terms")?
        || field(&prior, "revision")? != field(template, "revision")?
    {
        return Err(refused("renewal predecessor or disclosed terms differ"));
    }
    I::birth_route(view, binding, basis, terms)?;
    stamp(field(&market, "created_at_ic")?)?;
    let pair = Pair {
        market,
        roll,
        licence,
        predecessor,
        next,
        phase,
    };
    if field(&pair.market, "action")? != &json!(historical_action::<I>(&pair, 0)?) {
        return Err(refused("renewal historical Draft differs"));
    }
    paid_resale::renewal_holder(view, pair.binding()?, &pair.market)?;
    Ok(pair)
}
fn fingerprint<I: Issuer>(pair: &Pair) -> Result<DefinitionDigest, ActionStateError> {
    let mut values = [pair.market.clone(), pair.roll.clone()];
    for value in &mut values {
        value
            .as_object_mut()
            .ok_or_else(|| refused("renewal record shape"))?
            .remove("state");
    }
    digest(I::CAPACITY_DOMAIN, &json!(values))
}
fn targets<I: Issuer>(
    operation: CommitToken,
    pair: &Pair,
) -> Result<Vec<Target>, ActionStateError> {
    let mut rows = Vec::with_capacity(7);
    for (_, family, ceiling) in families::<I>() {
        let key = if [I::HEAD, LICENCE].contains(&family) {
            licence_key(family, pair.licence)?
        } else if [TERM, I::ECONOMICS].contains(&family) {
            term_key(family, pair.licence, pair.next)?
        } else {
            operation_key(family, operation)?
        };
        rows.push(Target {
            key,
            schema: CanonicalId::new(format!("{family}.value.v1"))
                .map_err(|_| refused("renewal target schema"))?,
            schema_version: 1,
            ceiling,
            mode: if [TERM, I::ECONOMICS, I::RECEIPT].contains(&family) {
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

fn receipt_action<I: Issuer>(value: &Value, index: usize) -> Result<(), ActionStateError> {
    let action: DefinitionRef = serde_json::from_value(field(value, "action")?.clone())
        .map_err(|_| refused("renewal receipt action"))?;
    if action.key().owner.as_str() != "market"
        || action.key().kind.as_str() != "action"
        || action.key().id.as_str() != action_id::<I>(index)
        || action.version() != 1
    {
        return Err(refused("renewal receipt action identity"));
    }
    Ok(())
}
fn state_receipts<I: Issuer>(pair: &Pair, operation: CommitToken) -> Result<(), ActionStateError> {
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
        receipt_action::<I>(closure, 5)?;
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
        receipt_action::<I>(closure, 4)?;
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
        || field(fund, "action")? != &json!(historical_action::<I>(pair, 1)?)
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
    receipt_action::<I>(terminal, if pair.phase == "granted" { 2 } else { 3 })?;
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
        if field(terminal, "action")? != &json!(historical_action::<I>(pair, 2)?)
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
    for name in ["charter", "line"] {
        if field(&licence, name)? != field(binding, name)? {
            return Err(refused("renewal immutable licence identity"));
        }
    }
    if field(&licence, "id")? != field(binding, "licence")?
        || field(&licence, "operation")? != field(binding, "birth_operation")?
    {
        return Err(refused("renewal immutable licence birth"));
    }
    paid_resale::renewal_current_holder(view, &licence, binding, pair.pending())?;
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
fn outputs<I: Issuer>(
    view: &View<'_>,
    operation: CommitToken,
    pair: &Pair,
    at_transition: bool,
) -> Result<(), ActionStateError> {
    state_receipts::<I>(pair, operation)?;
    let licence = current_licence(view, pair)?;
    let current = positive_string(field(&licence, "term")?)?;
    let receipt = value(view, &operation_key(I::RECEIPT, operation)?)?;
    let term = value(view, &term_key(TERM, pair.licence, pair.next)?)?;
    let economics = value(view, &term_key(I::ECONOMICS, pair.licence, pair.next)?)?;
    if pair.pending() {
        if receipt.is_some()
            || term.is_some()
            || economics.is_some()
            || current != pair.predecessor
            || required(view, &licence_key(I::HEAD, pair.licence)?)?
                != expected_head(pair, operation)?
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
        && required(view, &licence_key(I::HEAD, pair.licence)?)? != expected_head(pair, operation)?
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
        "terms":field(binding,"terms")?,"revision":entitlement::binding_revision(binding)?,
        "window":{"format":1,"eligibility_start":issued,"issued_at":issued,"term_end":end,"grace_end":grace},"operation":operation});
    let expected_economics = json!({"format":I::grant_version(field(binding,"grant")?)?,"licence":field(binding,"licence")?,"generation":field(binding,"next")?,
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
pub(super) fn verify<I: Issuer>(
    phase: Phase,
    reservation: &Reservation,
    view: &View<'_>,
) -> Result<(), ActionStateError> {
    let protocol =
        I::protocol(view.registry).ok_or_else(|| refused("renewal protocol unavailable"))?;
    let pair = pair::<I>(view, reservation.funding)?;
    if reservation.effect != historical_action::<I>(&pair, 0)?
        || reservation.sponsor != operation_key(I::MARKET, reservation.funding)?
        || reservation.targets != targets::<I>(reservation.funding, &pair)?
        || reservation.protocol_fingerprint != Some(fingerprint::<I>(&pair)?)
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
    outputs::<I>(view, reservation.funding, &pair, true)
}
pub(in crate::action_state) fn renewal_change<I: Issuer>(
    txn: &mut ActionTxn<'_>,
    commit: CommitToken,
    operation: Option<CommitToken>,
) -> Result<(), ActionStateError> {
    let protocol =
        I::protocol(&txn.store.registry).ok_or_else(|| refused("renewal protocol unavailable"))?;
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
        let pair = pair::<I>(&view, commit)?;
        let record = Reservation {
            format: 1,
            funding: commit,
            effect: protocol.admission_action().clone(),
            sponsor: operation_key(I::MARKET, commit)?,
            targets: targets::<I>(commit, &pair)?,
            protocol_fingerprint: Some(fingerprint::<I>(&pair)?),
        };
        verify::<I>(Phase::Funding, &record, &view)?;
        txn.capacity_changes.funding = Some(record);
    }
    Ok(())
}

fn protected<I: Issuer>(key: &StateKey) -> bool {
    families::<I>()
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
fn attempt_before<I: Issuer>(
    before: &View<'_>,
    after_pair: &Pair,
    operation: CommitToken,
) -> Result<(), ActionStateError> {
    let current = current_licence(before, after_pair)?;
    if positive_string(field(&current, "term")?)? != after_pair.predecessor {
        return Err(refused("renewal Draft predecessor is not current"));
    }
    let head = value(before, &licence_key(I::HEAD, after_pair.licence)?)?;
    let attempt = field(&after_pair.market, "attempt")?;
    let generation = paid_resale::renewal_holder_generation(after_pair.binding()?)?;
    match (text(attempt, "kind")?, head) {
        ("automatic" | "holder_automatic", None) if after_pair.predecessor == 1 => (),
        ("automatic" | "holder_automatic", Some(head)) => {
            let old = pair::<I>(before, token(field(&head, "last_operation")?)?)?;
            let old_generation = paid_resale::renewal_holder_generation(old.binding()?)?;
            if !field(&head, "pending")?.is_null()
                || positive_string(field(&head, "current")?)? != after_pair.predecessor
                || old_generation > generation
                || (old.predecessor == after_pair.predecessor
                    && (old_generation >= generation || !closed_attempt(&old)))
                || old.predecessor > after_pair.predecessor
                || (old.predecessor < after_pair.predecessor && old.phase != "granted")
            {
                return Err(refused("renewal automatic offer already attempted"));
            }
            require_after_terminal(after_pair, &old)?;
        }
        ("buyer_retry" | "holder_retry", Some(head)) => {
            let previous = token(field(attempt, "previous_operation")?)?;
            let prior = pair::<I>(before, previous)?;
            if previous == operation
                || field(&head, "last_operation")? != field(attempt, "previous_operation")?
                || !field(&head, "pending")?.is_null()
                || positive_string(field(&head, "current")?)? != after_pair.predecessor
                || positive_string(field(&head, "attempted_predecessor")?)?
                    != after_pair.predecessor
                || !closed_attempt(&prior)
                || prior.binding()? != after_pair.binding()?
            {
                return Err(refused(
                    "renewal retry lacks exact latest terminal predecessor",
                ));
            }
            require_after_terminal(after_pair, &prior)?;
            outputs::<I>(before, previous, &prior, false)?;
        }
        _ => return Err(refused("renewal attempt cannot replace pending custody")),
    }
    Ok(())
}
pub(super) fn validate_changes<I: Issuer>(
    store: &ActionStateStore,
    mutations: &BTreeMap<StateKey, TxnMutation>,
    changes: &Changes,
    identity: Option<&RegisteredTxnIdentity>,
) -> Result<(), ActionStateError> {
    let Some(protocol) = I::protocol(&store.registry) else {
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
        .any(|key| [I::MARKET, I::ROLL, I::HEAD, I::RECEIPT].contains(&key.family.as_str()));
    for key in mutations.keys().filter(|key| {
        key.owner.as_str() == "roll" && [TERM, I::ECONOMICS].contains(&key.family.as_str())
    }) {
        let raw = Key {
            owner: key.owner.clone(),
            family: key.family.clone(),
            encoded: key.encoded.clone(),
        };
        for view in [&before, &after] {
            if value(view, &raw)?.is_some() && shared_output_is_renewal::<I>(view, key)? {
                touches = true;
            }
        }
    }
    let progressing_here = changes
        .progress
        .as_ref()
        .and_then(|id| store.capacity.records.get(id))
        .is_some_and(|r| protocol.recognizes_historical(&r.record.effect));
    if !touches && !progressing_here {
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
    let next = pair::<I>(&after, operation)?;
    if action <= 2 {
        paid_resale::require_no_pending(&before, next.licence)?;
        I::birth_product(&after, next.binding()?)?;
    }
    if identity.principal != (PrincipalId::Object { id: identity.actor }) {
        return Err(refused("renewal actual principal differs"));
    }
    if action == 5 {
        if identity.actor != crate::GOD {
            return Err(refused("renewal actual service identity differs"));
        }
    } else if owner_index::acting(field(next.binding()?, "holder")?)? != u64::from(identity.actor.0)
    {
        return Err(refused("renewal actual buyer identity differs"));
    }
    let mut keys = vec![
        operation_key(I::MARKET, operation)?,
        operation_key(I::ROLL, operation)?,
    ];
    let id = ReservationId(
        OwnerId::new("market").map_err(|_| refused("renewal owner"))?,
        operation.to_hex(),
    );
    if action == 0 {
        if value(&before, &operation_key(I::MARKET, operation)?)?.is_some()
            || value(&before, &operation_key(I::ROLL, operation)?)?.is_some()
            || changes.funding.as_ref().map(|record| record.funding) != Some(operation)
        {
            return Err(refused("renewal Draft identity reused"));
        }
        attempt_before::<I>(&before, &next, operation)?;
        keys.push(licence_key(I::HEAD, next.licence)?);
        exact_mutations(mutations, &after, keys)?;
        verify::<I>(
            Phase::Funding,
            changes
                .funding
                .as_ref()
                .ok_or_else(|| refused("renewal reserve missing"))?,
            &after,
        )?;
        return Ok(());
    }
    let prior = pair::<I>(&before, operation)?;
    let stored = store
        .capacity
        .records
        .get(&id)
        .ok_or_else(|| refused("renewal acknowledged reservation missing"))?;
    verify::<I>(Phase::Retained, &stored.record, &before)?;
    if fingerprint::<I>(&prior)? != fingerprint::<I>(&next)? || identity.commit == operation {
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
        verify::<I>(Phase::Retained, &stored.record, &after)?;
    } else {
        if changes.terminal.as_ref() != Some(&id) {
            return Err(refused("renewal terminal must consume reservation"));
        }
        keys.extend([
            licence_key(I::HEAD, next.licence)?,
            operation_key(I::RECEIPT, operation)?,
        ]);
        let old_licence = current_licence(&before, &prior)?;
        let mut expected_licence = old_licence.clone();
        if action == 2 {
            // M8c F4: the renewed pointer names the binding's holder, the
            // same principal re-stamped with the renewing actor as acting.
            let holder = field(next.binding()?, "holder")?.clone();
            if !owner_index::same_holder(field(&old_licence, "holder")?, &holder)? {
                return Err(refused("renewal changed the licence holder"));
            }
            let fields = expected_licence
                .as_object_mut()
                .ok_or_else(|| refused("renewal licence shape"))?;
            fields.insert("term".into(), json!(next.next.to_string()));
            fields.insert("holder".into(), holder);
            keys.extend([
                licence_key(LICENCE, next.licence)?,
                term_key(TERM, next.licence, next.next)?,
                term_key(I::ECONOMICS, next.licence, next.next)?,
            ]);
        }
        if current_licence(&after, &next)? != expected_licence {
            return Err(refused("renewal changed immutable licence data"));
        }
        exact_mutations(mutations, &after, keys)?;
        verify::<I>(Phase::Terminal, &stored.record, &after)?;
    }
    Ok(())
}

pub(super) fn shared_output_is_renewal<I: Issuer>(
    view: &View<'_>,
    state_key: &StateKey,
) -> Result<bool, ActionStateError> {
    if state_key.owner.as_str() != "roll"
        || ![TERM, I::ECONOMICS].contains(&state_key.family.as_str())
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
    if optional_family_value(view, I::ROLL, operation)?.is_none() {
        return Ok(false);
    }
    if I::protocol(view.registry).is_none() {
        return Err(refused("renewal grant without registered protocol"));
    }
    require_disjoint_operation::<I>(view, operation)?;
    let pair = pair::<I>(view, operation)?;
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
pub(super) fn validate_current_birth<I: Issuer>(
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
    if I::protocol(view.registry).is_none() {
        return Err(refused("new paid generation lacks renewal protocol"));
    }
    let id = positive_string(field(licence, "id")?)?;
    let head = required(view, &licence_key(I::HEAD, id)?)?;
    let term = required(view, &term_key(TERM, id, current)?)?;
    let operation = token(field(&term, "operation")?)?;
    let pair = pair::<I>(view, operation)?;
    if pair.phase != "granted"
        || pair.next != current
        || pair.licence != id
        || field(&head, "current")? != field(licence, "term")?
        || field(&head, "birth_operation")? != &json!(birth)
        || field(pair.binding()?, "birth_operation")? != &json!(birth)
    {
        return Err(refused("paid current generation lacks exact renewal Grant"));
    }
    outputs::<I>(view, operation, &pair, false)
}
/// The renewal validator still checks its exact before/after bytes and targets.
pub(super) fn owns_licence_change<I: Issuer>(
    view: &View<'_>,
    key: &StateKey,
    identity: Option<&RegisteredTxnIdentity>,
    changes: &Changes,
) -> Result<bool, ActionStateError> {
    if key.owner.as_str() != "roll" || key.family.as_str() != LICENCE {
        return Ok(false);
    }
    let Some(protocol) = I::protocol(view.registry) else {
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
    let pair = pair::<I>(view, operation)?;
    if id.0.as_str() != "market"
        || pair.phase != "granted"
        || *key != licence_key(LICENCE, pair.licence)?.resolve(view.registry)?
        || token(field(field(pair.state()?, "terminal")?, "commit")?)? != identity.commit
    {
        return Err(refused("renewal licence update crosses operation"));
    }
    Ok(true)
}
pub(super) fn reject_other_operation<I: Issuer>(
    view: &View<'_>,
    operation: CommitToken,
) -> Result<(), ActionStateError> {
    if optional_family_value(view, I::ROLL, operation)?.is_some()
        || optional_family_value(view, I::MARKET, operation)?.is_some()
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
fn closed_attempt(pair: &Pair) -> bool {
    matches!(
        pair.phase.as_str(),
        "refunded" | "unfunded_closed" | "unfunded_service_closed"
    )
}
fn require_after_terminal(next: &Pair, prior: &Pair) -> Result<(), ActionStateError> {
    let terminal = if matches!(prior.phase.as_str(), "granted" | "refunded") {
        field(prior.state()?, "terminal")?
    } else {
        field(prior.state()?, "closure")?
    };
    if signed(&next.market, "created_at_utc")? < signed(terminal, "posted_at_utc")? {
        return Err(refused("renewal attempt predates prior terminal"));
    }
    Ok(())
}
fn audit_attempts<I: Issuer>(
    view: &View<'_>,
    operations: &BTreeMap<CommitToken, Pair>,
    heads: &BTreeSet<u64>,
) -> Result<(), ActionStateError> {
    // Transfer generation orders independent rooted attempt chains within one
    // term. A later holder can start after an old holder's terminal refusal;
    // only an actual Grant advances the term. Token ordering is irrelevant.
    let mut groups: BTreeMap<u64, BTreeMap<(u64, u64), Vec<CommitToken>>> = BTreeMap::new();
    for (operation, pair) in operations {
        groups
            .entry(pair.licence)
            .or_default()
            .entry((
                pair.predecessor,
                paid_resale::renewal_holder_generation(pair.binding()?)?,
            ))
            .or_default()
            .push(*operation);
    }
    if groups.keys().copied().collect::<BTreeSet<_>>() != *heads {
        return Err(refused("renewal head reverse completeness"));
    }
    for (licence, generations) in groups {
        let mut current = 1_u64;
        let mut last = None;
        let mut last_holder = 0_u64;
        let mut birth_identity = None;
        for ((predecessor, holder_generation), ids) in generations {
            if predecessor != current || holder_generation < last_holder {
                return Err(refused("renewal term or holder history moves backwards"));
            }
            let mut root = None;
            let mut successors = BTreeMap::new();
            let mut tuple_binding = None;
            for operation in &ids {
                let pair = operations
                    .get(operation)
                    .ok_or_else(|| refused("renewal attempt index"))?;
                let binding = pair.binding()?;
                let identity = json!([
                    field(binding, "birth_operation")?,
                    field(binding, "charter")?,
                    field(binding, "line")?
                ]);
                if birth_identity.as_ref().is_some_and(|old| old != &identity)
                    || tuple_binding.as_ref().is_some_and(|old| old != binding)
                {
                    return Err(refused("renewal historical birth or retry binding drift"));
                }
                birth_identity = Some(identity);
                tuple_binding = Some(binding.clone());
                let attempt = field(&pair.market, "attempt")?;
                match text(attempt, "kind")? {
                    "automatic" | "holder_automatic" => {
                        if root.replace(*operation).is_some() {
                            return Err(refused("renewal repeated automatic holder predecessor"));
                        }
                    }
                    "buyer_retry" | "holder_retry" => {
                        let previous = token(field(attempt, "previous_operation")?)?;
                        if successors.insert(previous, *operation).is_some() {
                            return Err(refused("renewal retry branch"));
                        }
                    }
                    _ => return Err(refused("renewal attempt kind")),
                }
            }
            let mut cursor = root.ok_or_else(|| refused("renewal automatic root missing"))?;
            if let Some(prior_id) = last {
                let prior: &Pair = operations
                    .get(&prior_id)
                    .ok_or_else(|| refused("renewal prior group"))?;
                let next = operations
                    .get(&cursor)
                    .ok_or_else(|| refused("renewal next group"))?;
                if prior.pending()
                    || (prior.predecessor == predecessor
                        && (!closed_attempt(prior) || holder_generation <= last_holder))
                {
                    return Err(refused(
                        "renewal later holder replaced open or granted attempt",
                    ));
                }
                require_after_terminal(next, prior)?;
            }
            let mut visited = BTreeSet::new();
            loop {
                if !visited.insert(cursor) {
                    return Err(refused("renewal retry cycle"));
                }
                let pair = operations
                    .get(&cursor)
                    .ok_or_else(|| refused("renewal retry predecessor missing"))?;
                if pair.licence != licence
                    || pair.predecessor != predecessor
                    || paid_resale::renewal_holder_generation(pair.binding()?)? != holder_generation
                {
                    return Err(refused("renewal retry crossed licence term or holder"));
                }
                if let Some(next) = successors.get(&cursor) {
                    if !closed_attempt(pair) {
                        return Err(refused("renewal retry after pending or granted operation"));
                    }
                    let retry = operations
                        .get(next)
                        .ok_or_else(|| refused("renewal retry successor missing"))?;
                    require_after_terminal(retry, pair)?;
                    cursor = *next;
                } else {
                    if visited.len() != ids.len() {
                        return Err(refused("renewal orphan or disconnected retry"));
                    }
                    if pair.phase == "granted" {
                        current = pair.next;
                    }
                    last = Some(cursor);
                    last_holder = holder_generation;
                    break;
                }
            }
        }
        let last = last.ok_or_else(|| refused("renewal head has no attempts"))?;
        let pair = operations
            .get(&last)
            .ok_or_else(|| refused("renewal last attempt"))?;
        if required(view, &licence_key(I::HEAD, licence)?)? != expected_head(pair, last)?
            || positive_string(field(&current_licence(view, pair)?, "term")?)? != current
        {
            return Err(refused("renewal head is not exact final linked attempt"));
        }
    }
    Ok(())
}
pub(super) fn audit_boot<I: Issuer>(
    view: &View<'_>,
    reservations: &BTreeMap<ReservationId, Arc<StoredReservation>>,
) -> Result<(), ActionStateError> {
    let own = |key: &StateKey| {
        let family = key.family.as_str();
        match key.owner.as_str() {
            "market" => [I::MARKET, I::RECEIPT].contains(&family),
            "roll" => [I::ROLL, I::HEAD].contains(&family),
            _ => false,
        }
    };
    if I::protocol(view.registry).is_none() {
        if view.records.keys().any(own) {
            return Err(refused("retained renewal without registered protocol"));
        }
        return Ok(());
    }
    let mut ids = BTreeSet::new();
    let mut heads = BTreeSet::new();
    for key in view.records.keys().filter(|key| protected::<I>(key)) {
        let family = key.family.as_str();
        if [I::MARKET, I::ROLL, I::RECEIPT].contains(&family) {
            ids.insert(own_row_operation(view, key)?);
        } else if family == I::HEAD {
            let row = required(
                view,
                &Key {
                    owner: key.owner.clone(),
                    family: key.family.clone(),
                    encoded: key.encoded.clone(),
                },
            )?;
            let licence = positive_string(field(&row, "licence")?)?;
            if *key != licence_key(I::HEAD, licence)?.resolve(view.registry)?
                || !heads.insert(licence)
            {
                return Err(refused("renewal duplicate or crossed head"));
            }
            ids.insert(token(field(&row, "last_operation")?)?);
        } else if [TERM, I::ECONOMICS].contains(&family)
            && shared_output_is_renewal::<I>(view, key)?
        {
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
    let mut operations = BTreeMap::new();
    for id in ids {
        let pair = pair::<I>(view, id)?;
        let reservation = reservations.get(&ReservationId(
            OwnerId::new("market").map_err(|_| refused("renewal owner"))?,
            id.to_hex(),
        ));
        match (pair.pending(), reservation) {
            (true, Some(row)) => verify::<I>(Phase::Retained, &row.record, view)?,
            (false, None) => outputs::<I>(view, id, &pair, false)?,
            _ => return Err(refused("renewal reverse reservation completeness")),
        }
        operations.insert(id, pair);
    }
    audit_attempts::<I>(view, &operations, &heads)
}

#[cfg(test)]
pub(super) mod tests;
