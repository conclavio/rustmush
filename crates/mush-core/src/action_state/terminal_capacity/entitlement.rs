//! Exact six-target completion for closed OriginalV3 and DerivedV1 purchases.
//! Actual money, account authority and source-root proofs remain owner checks.
use super::*;
use crate::action::{ActionProviderRegistry, PrincipalId};
use crate::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistry,
    DefinitionRegistryBuilder,
};

const ORIGINAL_ACTIONS: [&str; 3] = [
    "purchase_original_mind",
    "settle_original_mind",
    "refund_original_mind",
];
const DERIVED_ACTIONS: [&str; 3] = [
    "purchase_derived_mind",
    "settle_derived_mind",
    "refund_derived_mind",
];
const TRANSITIONS: [&str; 3] = ["fund", "grant", "refund"];
const PURCHASE: &str = "market_mind_entitlement_purchase";
const RECEIPT: &str = "market_mind_entitlement_receipt";
const ALLOCATION: &str = "roll_mind_entitlement_purchase";
const LICENCE: &str = "roll_mind_personal_paid_licence";
const TERM: &str = "roll_mind_paid_term";
const ECONOMICS: &str = "roll_mind_paid_term_economics";
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
    Original,
    Derived,
    OriginalFollow,
    DerivedFollow,
}
impl Kind {
    fn follow(self) -> bool {
        matches!(self, Self::OriginalFollow | Self::DerivedFollow)
    }
    fn original(self) -> bool {
        matches!(self, Self::Original | Self::OriginalFollow)
    }
    fn economics_version(self) -> u64 {
        if self.follow() { 2 } else { 1 }
    }

    fn version(self) -> u32 {
        match self {
            Self::Original => 3,
            Self::Derived => 1,
            Self::OriginalFollow => 4,
            Self::DerivedFollow => 2,
        }
    }
    fn wire(self) -> &'static str {
        match self {
            Self::Original => "original_v3",
            Self::Derived => "derived_v1",
            Self::OriginalFollow => "original_v4",
            Self::DerivedFollow => "derived_v2",
        }
    }
    fn protocol(self) -> &'static str {
        match self {
            Self::Original => "original_mind_purchase_v3",
            Self::Derived => "derived_mind_purchase_v1",
            Self::OriginalFollow => "original_mind_purchase_v4",
            Self::DerivedFollow => "derived_mind_purchase_v2",
        }
    }
    fn actions(self) -> [&'static str; 3] {
        match self {
            Self::Original | Self::OriginalFollow => ORIGINAL_ACTIONS,
            Self::Derived | Self::DerivedFollow => DERIVED_ACTIONS,
        }
    }
    fn product_domain(self) -> &'static str {
        match self {
            Self::Original => "market.original_product.v3",
            Self::Derived => "market.derived_product.v1",
            Self::OriginalFollow => "market.original_product.v4",
            Self::DerivedFollow => "market.derived_product.v2",
        }
    }
    fn quote_domain(self) -> &'static str {
        match self {
            Self::Original => "market.original_quote.v3",
            Self::Derived => "market.derived_quote.v1",
            Self::OriginalFollow => "market.original_quote.v4",
            Self::DerivedFollow => "market.derived_quote.v2",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub(in crate::action_state) struct Protocol {
    manifest: DefinitionRef,
    participants: [DefinitionRef; 2],
    original: [DefinitionRef; 3],
    derived: [DefinitionRef; 3],
    #[serde(skip_serializing_if = "Option::is_none")]
    follow: Option<[[DefinitionRef; 3]; 2]>,
}
impl Protocol {
    pub(in crate::action_state) fn new(
        owner: &OwnerId,
        manifest: DefinitionRef,
        participants: [DefinitionRef; 2],
        original: [DefinitionRef; 3],
        derived: [DefinitionRef; 3],
    ) -> Result<Self, ActionStateError> {
        if owner.as_str() != "market"
            || manifest.key().owner.as_str() != "tacsim"
            || manifest.key().kind.as_str() != "registry_manifest"
            || manifest.key().id.as_str() != "combat"
            || manifest.version() != 1
        {
            return Err(refused("entitlement declaration owner or manifest"));
        }
        check_participants(&participants)?;
        let result = Self {
            manifest,
            participants,
            original,
            derived,
            follow: None,
        };
        for kind in [Kind::Original, Kind::Derived] {
            for (action, id) in result.actions(kind)?.iter().zip(kind.actions()) {
                if action.key().owner != *owner
                    || action.key().kind.as_str() != "action"
                    || action.key().id.as_str() != id
                    || action.version() != kind.version()
                {
                    return Err(refused("entitlement action identity or order"));
                }
            }
            if *result.admission_action(kind)?
                != admission_reference(&result.manifest, &result.participants, kind)?
            {
                return Err(refused("entitlement admission definition differs"));
            }
        }
        Ok(result)
    }
    pub(in crate::action_state) fn with_follow(
        owner: &OwnerId,
        manifest: DefinitionRef,
        participants: [DefinitionRef; 2],
        original: [DefinitionRef; 3],
        derived: [DefinitionRef; 3],
        original_follow: [DefinitionRef; 3],
        derived_follow: [DefinitionRef; 3],
    ) -> Result<Self, ActionStateError> {
        let mut result = Self::new(owner, manifest, participants, original, derived)?;
        for (kind, actions) in [
            (Kind::OriginalFollow, &original_follow),
            (Kind::DerivedFollow, &derived_follow),
        ] {
            for (index, action) in actions.iter().enumerate() {
                if *action != action_reference(&result.manifest, &result.participants, kind, index)?
                {
                    return Err(refused("entitlement Follow exact action recipe"));
                }
            }
        }
        result.follow = Some([original_follow, derived_follow]);
        Ok(result)
    }
    fn kinds(&self) -> impl Iterator<Item = Kind> {
        [Kind::Original, Kind::Derived].into_iter().chain(
            self.follow
                .iter()
                .flat_map(|_| [Kind::OriginalFollow, Kind::DerivedFollow]),
        )
    }
    fn actions(&self, kind: Kind) -> Result<&[DefinitionRef; 3], ActionStateError> {
        match kind {
            Kind::Original => Ok(&self.original),
            Kind::Derived => Ok(&self.derived),
            Kind::OriginalFollow => self
                .follow
                .as_ref()
                .map(|a| &a[0])
                .ok_or_else(|| refused("entitlement Follow recipe unavailable")),
            Kind::DerivedFollow => self
                .follow
                .as_ref()
                .map(|a| &a[1])
                .ok_or_else(|| refused("entitlement Follow recipe unavailable")),
        }
    }
    fn admission_action(&self, kind: Kind) -> Result<&DefinitionRef, ActionStateError> {
        Ok(&self.actions(kind)?[0])
    }
    pub(super) fn admission_actions(&self) -> Vec<&DefinitionRef> {
        [&self.original[0], &self.derived[0]]
            .into_iter()
            .chain(self.follow.iter().flat_map(|a| [&a[0][0], &a[1][0]]))
            .collect()
    }
    pub(super) fn recognizes_historical(&self, effect: &DefinitionRef) -> bool {
        (effect.key() == self.original[0].key() && matches!(effect.version(), 3 | 4))
            || (effect.key() == self.derived[0].key() && matches!(effect.version(), 1 | 2))
    }
    pub(super) fn require_product(&self, allocation: &Value) -> Result<(), ActionStateError> {
        let kind = allocation_kind(allocation)?;
        self.admission_action(kind)?;
        Ok(())
    }
    fn action_index(
        &self,
        identity: &RegisteredTxnIdentity,
    ) -> Result<(Kind, usize), ActionStateError> {
        for kind in self.kinds() {
            if let Some(index) = self
                .actions(kind)?
                .iter()
                .position(|action| action == &identity.action)
            {
                return Ok((kind, index));
            }
        }
        Err(refused("entitlement invocation action differs"))
    }
    pub(in crate::action_state) fn validate_families(
        &self,
        registry: &StateFamilyRegistry,
    ) -> Result<(), ActionStateError> {
        for (owner, family, bytes) in FAMILIES.into_iter().chain([
            ("roll", "roll_mind_commerce", 4096),
            ("market", "market_mind_entitlement_quote", 24576),
        ]) {
            let id = StateFamilyId::new(family).map_err(|_| refused("entitlement family"))?;
            let canonical = |suffix: &str| {
                CanonicalId::new(format!("{family}.{suffix}.v1"))
                    .map_err(|_| refused("entitlement schema"))
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
            .map_err(|_| refused("entitlement descriptor"))?;
            let owner = OwnerId::new(owner).map_err(|_| refused("entitlement owner"))?;
            let actual = registry
                .entries
                .get(&(owner.clone(), id.clone()))
                .ok_or_else(|| refused("entitlement family unavailable"))?;
            if actual.descriptor().canonical_value() != expected.canonical_value() {
                return Err(refused("entitlement descriptor differs"));
            }
            if owner.as_str() == "roll"
                && !registry.delegates_family(
                    &owner,
                    &id,
                    &OwnerId::new("market").map_err(|_| refused("entitlement owner"))?,
                )
            {
                return Err(refused("entitlement Roll family not delegated"));
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
            .map_err(|_| refused("entitlement manifest unavailable"))?;
        for kind in self.kinds() {
            for (action, transition) in self.actions(kind)?.iter().zip(TRANSITIONS) {
                let definition = definitions
                    .get_exact(action)
                    .map_err(|_| refused("entitlement action unavailable"))?;
                let provider = providers
                    .get_exact(action)
                    .map_err(|_| refused("entitlement provider unavailable"))?;
                if provider.owner().as_str() != "market"
                    || definition.payload() != &action_payload(kind, transition)
                {
                    return Err(refused("entitlement action protocol differs"));
                }
                let refs = definition.references();
                if refs.len() != 3
                    || !refs.contains(&self.manifest)
                    || self.participants.iter().any(|p| !refs.contains(p))
                {
                    return Err(refused("entitlement reference set differs"));
                }
                for reference in &self.participants {
                    definitions
                        .get_exact(reference)
                        .map_err(|_| refused("entitlement participant unavailable"))?;
                    if !participants.grants_exact(reference, provider.owner(), action) {
                        return Err(refused("entitlement participant grant differs"));
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
    action_reference(manifest, participants, kind, 0)
}
fn action_reference(
    manifest: &DefinitionRef,
    participants: &[DefinitionRef; 2],
    kind: Kind,
    index: usize,
) -> Result<DefinitionRef, ActionStateError> {
    check_participants(participants)?;
    let descriptor = DefinitionDescriptor::new(
        DefinitionKind::new("action").map_err(|_| refused("purchase action kind"))?,
        DefinitionId::new(kind.actions()[index]).map_err(|_| refused("purchase action id"))?,
        kind.version(),
        action_payload(kind, TRANSITIONS[index]),
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
    let kind = match text(product, "protocol")? {
        "original_v3" => Kind::Original,
        "derived_v1" => Kind::Derived,
        "original_v4" => Kind::OriginalFollow,
        "derived_v2" => Kind::DerivedFollow,
        _ => return Err(refused("unknown entitlement protocol")),
    };
    let terms = field(product, "terms")?;
    let form = field(terms, "form")?;
    let form_name = text(form, "kind")?;
    let grant = field(product, "grant")?;
    let downstream = field(grant, "downstream")?;
    let royalty = field(quote, "royalty")?;
    let edges = field(royalty, "edges")?
        .as_array()
        .ok_or_else(|| refused("entitlement edges"))?;
    let parent = field(field(quote, "subject")?, "parent")?;
    let pin = field(product, "revision")?;
    let subject = field(quote, "subject")?;
    let source = match form_name {
        "sealed" => downstream == &json!({"kind":"sealed"}),
        "open" | "open_resale" => {
            text(downstream, "kind")? == "source" && number(downstream, "royalty_permille")? <= 300
        }
        _ => false,
    };
    if number(quote, "format")? != 1
        || grant_version(grant)? != kind.economics_version()
        || number(terms, "seats")? != 1
        || form.as_object().is_none_or(|f| f.len() != 1)
        || !revision_matches(pin, subject, kind.follow())?
        || !source
        || edges.len() > 3
        || (kind.original() && (!edges.is_empty() || !parent.is_null()))
        || (!kind.original() && (edges.is_empty() || parent.is_null()))
    {
        return Err(refused("entitlement product or ancestry shape"));
    }
    if let Some(first) = edges.first()
        && field(first, "parent")? != parent
    {
        return Err(refused("entitlement immediate parent differs"));
    }
    let sum = edges.iter().try_fold(0_u64, |sum, edge| {
        sum.checked_add(number(edge, "royalty_permille")?)
            .ok_or_else(|| refused("entitlement royalty sum"))
    })?;
    if sum > 300
        || (form_name == "open_resale"
            && (edges.len() >= 3
                || sum
                    .checked_add(number(downstream, "royalty_permille")?)
                    .is_none_or(|n| n > 300)))
    {
        return Err(refused("entitlement downstream bound"));
    }
    let chain = field(grant, "royalty_chain")?;
    if field(chain, "filed")? != field(royalty, "filed")?
        || field(chain, "digest")? != &json!(digest("roll.mind.filed_royalties.v1", royalty)?)
        || field(royalty, "filed")? != field(field(quote, "subject")?, "filed")?
        || field(royalty, "revision")? != field(field(quote, "subject")?, "revision")?
    {
        return Err(refused("entitlement royalty content differs"));
    }
    let renewal = field(grant, "renewal")?;
    if number(renewal, "format")? != kind.economics_version()
        || field(renewal, "terms")? != terms
        || field(renewal, "revision")? != field(product, "revision")?
        || field(renewal, "instrument")? != field(quote, "instrument")?
        || field(renewal, "policy")? != field(quote, "policy")?
    {
        return Err(refused("entitlement renewal consent differs"));
    }
    Ok(kind)
}

/// A Follow policy names the purchased line; acquisition fields remain immutable.
pub(super) fn revision_matches(
    policy: &Value,
    subject: &Value,
    follow: bool,
) -> Result<bool, ActionStateError> {
    Ok(if follow {
        policy == &json!({"kind":"follow", "line":field(subject,"line")?})
    } else {
        policy
            == &json!({"kind":"pin", "filed":field(subject,"filed")?, "revision":field(subject,"revision")?})
    })
}
pub(super) fn require_follow_birth(
    view: &View<'_>,
    binding: &Value,
) -> Result<(), ActionStateError> {
    if !matches!(number(binding, "format")?, 3 | 4) {
        return Ok(());
    }
    let operation = token(field(binding, "birth_operation")?)?;
    let allocation = required(view, &operation_key(ALLOCATION, operation)?)?;
    let protocol = view
        .registry
        .entitlement_purchase
        .as_ref()
        .ok_or_else(|| refused("renewal Follow birth protocol absent"))?;
    protocol.require_product(&allocation)?;
    if !allocation_kind(&allocation)?.follow()
        || text(field(&allocation, "phase")?, "kind")? != "granted"
        || field(&allocation, "operation")? != &json!(operation)
        || field(field(&allocation, "economics")?, "grant")? != field(binding, "grant")?
    {
        return Err(refused("renewal Follow immutable birth differs"));
    }
    for name in [
        "licence",
        "charter",
        "line",
        "filed",
        "revision",
        "product_digest",
        "terms",
    ] {
        if field(&allocation, name)? != field(binding, name)? {
            return Err(refused("renewal Follow acquisition changed"));
        }
    }
    Ok(())
}
pub(super) fn binding_revision(binding: &Value) -> Result<Value, ActionStateError> {
    match number(binding, "format")? {
        1 | 2 if binding.get("revision_policy").is_none() => Ok(
            json!({"kind":"pin", "filed":field(binding,"filed")?, "revision":field(binding,"revision")?}),
        ),
        3 | 4 => {
            let policy = field(binding, "revision_policy")?;
            if !revision_matches(policy, binding, true)? {
                return Err(refused("renewal Follow line differs"));
            }
            Ok(policy.clone())
        }
        _ => Err(refused("renewal revision policy format differs")),
    }
}
pub(super) fn grant_version(grant: &Value) -> Result<u64, ActionStateError> {
    let version = number(grant, "format")?;
    let template = field(grant, "renewal")?;
    let policy = field(template, "revision")?;
    let kind = text(policy, "kind")?;
    if !matches!((version, kind), (1, "pin") | (2, "follow"))
        || number(template, "format")? != version
    {
        return Err(refused("personal grant and renewal policy format differ"));
    }
    if version == 2 {
        let r: DefinitionRef = serde_json::from_value(field(template, "policy")?.clone())
            .map_err(|_| refused("personal Follow policy reference"))?;
        if r.key().owner.as_str() != "market"
            || r.key().kind.as_str() != "entitlement_policy"
            || r.key().id.as_str() != "personal"
            || r.version() != 2
        {
            return Err(refused("personal Follow policy identity"));
        }
    }
    Ok(version)
}
pub(super) fn grant_domain(grant: &Value) -> Result<&'static str, ActionStateError> {
    Ok(if grant_version(grant)? == 1 {
        "roll.mind.grant_economics.v1"
    } else {
        "roll.mind.grant_economics.v2"
    })
}
pub(super) fn economics_domain(economics: &Value) -> Result<&'static str, ActionStateError> {
    let version = grant_version(field(economics, "grant")?)?;
    if number(economics, "format")? != version {
        return Err(refused("personal economics format differs"));
    }
    Ok(if version == 1 {
        "roll.mind.paid_term_economics.v1"
    } else {
        "roll.mind.paid_term_economics.v2"
    })
}
fn allocation_kind(allocation: &Value) -> Result<Kind, ActionStateError> {
    let kind = match text(allocation, "protocol")? {
        "original_v3" => Kind::Original,
        "derived_v1" => Kind::Derived,
        "original_v4" => Kind::OriginalFollow,
        "derived_v2" => Kind::DerivedFollow,
        _ => return Err(refused("personal allocation protocol")),
    };
    let economics = field(allocation, "economics")?;
    let grant = field(economics, "grant")?;
    if number(allocation, "format")? != 1
        || number(economics, "format")? != kind.economics_version()
        || grant_version(grant)? != kind.economics_version()
        || !revision_matches(
            field(field(grant, "renewal")?, "revision")?,
            allocation,
            kind.follow(),
        )?
    {
        return Err(refused("personal allocation economics differs"));
    }
    Ok(kind)
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
    require_disjoint_operation(view, id, true)?;
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
        .entitlement_purchase
        .as_ref()
        .ok_or_else(|| refused("purchase protocol unavailable"))?;
    protocol.require_product(&allocation)?;
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
    for name in ["charter", "line", "filed", "revision", "rating"] {
        if field(&allocation, name)? != field(subject, name)? {
            return Err(refused("purchase subject differs"));
        }
    }
    let economics = field(&allocation, "economics")?;
    if economics
        != &json!({"format":kind.economics_version(),"licence":field(&allocation,"licence")?,"generation":field(&allocation,"term")?,"operation":id,"product_digest":field(&allocation,"product_digest")?,"grant":field(product,"grant")?})
    {
        return Err(refused("entitlement allocated economics differs"));
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
    digest("core.mind_entitlement_purchase.capacity.v1", &json!(values))
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
                grant_domain(field(
                    field(field(&pair.purchase, "quote")?, "product")?,
                    "grant"
                )?)?,
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
        return Err(refused("entitlement granted economics differs"));
    }
    let current = current.ok_or_else(|| refused("purchase paid licence missing"))?;
    renewal::validate_current_birth(view, &current, id)?;
    let term = term.ok_or_else(|| refused("purchase paid term missing"))?;
    paid_resale::validate_current_holder(view, &current, field(&pair.allocation, "holder")?, id)?;
    for name in ["charter", "line"] {
        if field(&current, name)? != field(&pair.allocation, name)? {
            return Err(refused("purchase paid licence subject differs"));
        }
    }
    if number(&term, "format")? != 1
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
        return Err(refused("entitlement grant clock differs"));
    }
    let issued = stamp(field(&receipt, "issued_at_ic")?)?;
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
        .entitlement_purchase
        .as_ref()
        .ok_or_else(|| refused("purchase protocol unavailable"))?;
    let pair = pair(view, reservation.funding)?;
    let kind = quote_kind(field(&pair.purchase, "quote")?)?;
    if reservation.effect != historical_admission(protocol, &pair.purchase)?
        || (matches!(phase, Phase::Funding)
            && reservation.effect != *protocol.admission_action(kind)?)
        || (matches!(phase, Phase::Terminal)
            && pair.phase == "granted"
            && reservation.effect != *protocol.admission_action(kind)?)
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
pub(in crate::action_state) fn entitlement_change(
    txn: &mut ActionTxn<'_>,
    commit: CommitToken,
    terminal: Option<CommitToken>,
) -> Result<(), ActionStateError> {
    let protocol = txn
        .store
        .registry
        .entitlement_purchase
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
            return Err(refused("entitlement funding protocol differs"));
        }
        let record = Reservation {
            format: 1,
            funding: commit,
            effect: protocol.admission_action(kind)?.clone(),
            sponsor: operation_key(PURCHASE, commit)?,
            targets: targets(commit, pair.licence)?,
            protocol_fingerprint: Some(fingerprint(&pair)?),
        };
        verify(Phase::Funding, &record, &view)?;
        txn.capacity_changes.funding = Some(record);
    }
    Ok(())
}
pub(super) fn shared_output_is_entitlement(
    view: &View<'_>,
    key: &StateKey,
) -> Result<bool, ActionStateError> {
    if key.owner.as_str() != "roll" || !matches!(key.family.as_str(), LICENCE | TERM) {
        return Err(refused("shared purchase output family"));
    }
    let row = required(
        view,
        &Key {
            owner: key.owner.clone(),
            family: key.family.clone(),
            encoded: key.encoded.clone(),
        },
    )?;
    let operation = token(field(&row, "operation")?)?;
    let allocation = |family: &str| -> Result<bool, ActionStateError> {
        let key = key_for_shared(family, operation)?;
        if !view
            .registry
            .entries
            .contains_key(&(key.owner.clone(), key.family.clone()))
        {
            return Ok(false);
        }
        Ok(value(view, &key)?.is_some())
    };
    match (
        allocation("roll_mind_paid_purchase")?,
        allocation(ALLOCATION)?,
    ) {
        (true, false) if view.registry.original_purchase.is_some() => Ok(false),
        (false, true) if view.registry.entitlement_purchase.is_some() => Ok(true),
        _ => Err(refused(
            "shared purchase output requires one registered allocation",
        )),
    }
}
// A global operation cannot acquire two purchase recipes, even when both end
// in refund and therefore have no shared licence/term row to route.
pub(super) fn require_disjoint_operation(
    view: &View<'_>,
    operation: CommitToken,
    entitlement: bool,
) -> Result<(), ActionStateError> {
    renewal::reject_other_operation(view, operation)?;
    house_renewal::reject_other_operation(view, operation)?;
    house::reject_house_operation(view, operation)?;
    let other = if entitlement {
        "roll_mind_paid_purchase"
    } else {
        ALLOCATION
    };
    let other = key_for_shared(other, operation)?;
    if view
        .registry
        .entries
        .contains_key(&(other.owner.clone(), other.family.clone()))
        && value(view, &other)?.is_some()
    {
        return Err(refused("purchase operation has two allocation recipes"));
    }
    Ok(())
}
fn key_for_shared(family: &str, operation: CommitToken) -> Result<Key, ActionStateError> {
    key("roll", family, operation.to_hex())
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
    if view.registry.entitlement_purchase.is_none() {
        return Ok(());
    }
    let mut operations = BTreeSet::new();
    for key in view.records.keys().filter(|key| protected(key)) {
        if house::shared_output_is_house(view, key)? {
            continue;
        }
        if house_renewal::shared_output_is_renewal(view, key)? {
            continue;
        }
        if renewal::shared_output_is_renewal(view, key)? {
            continue;
        }
        if matches!(key.family.as_str(), LICENCE | TERM)
            && !shared_output_is_entitlement(view, key)?
        {
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
    let Some(protocol) = &store.registry.entitlement_purchase else {
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
        if paid_resale::owns_licence_change(&after, key, identity, changes)? {
            continue;
        }
        if house::shared_output_is_house(&after, key)? {
            continue;
        }
        if house_renewal::shared_output_is_renewal(&after, key)? {
            continue;
        }
        if renewal::shared_output_is_renewal(&after, key)?
            || renewal::owns_licence_change(&after, key, identity, changes)?
        {
            continue;
        }
        if matches!(key.family.as_str(), LICENCE | TERM)
            && !shared_output_is_entitlement(&after, key)?
        {
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
            return Err(refused("entitlement invocation protocol differs"));
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
                "market_mind_entitlement_quote",
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
