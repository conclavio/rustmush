//! Strict structural joins. Native authority is supplied by the actual Economy
//! participant; these values never constitute a receipt capability.
use super::*;

pub(super) fn field<'a>(v: &'a Value, n: &str) -> Result<&'a Value, ActionStateError> {
    v.get(n).ok_or_else(|| refused("resale field missing"))
}
pub(super) fn shape(v: &Value, names: &[&str]) -> Result<(), ActionStateError> {
    let o = v.as_object().ok_or_else(|| refused("resale object"))?;
    if o.len() != names.len() || names.iter().any(|n| !o.contains_key(*n)) {
        return Err(refused("resale fields differ"));
    }
    Ok(())
}
pub(super) fn num(v: &Value, n: &str) -> Result<u64, ActionStateError> {
    field(v, n)?
        .as_u64()
        .ok_or_else(|| refused("resale integer"))
}
pub(super) fn signed(v: &Value, n: &str) -> Result<i64, ActionStateError> {
    field(v, n)?
        .as_i64()
        .ok_or_else(|| refused("resale signed integer"))
}
pub(super) fn text<'a>(v: &'a Value, n: &str) -> Result<&'a str, ActionStateError> {
    field(v, n)?.as_str().ok_or_else(|| refused("resale text"))
}
pub(super) fn token(v: &Value) -> Result<CommitToken, ActionStateError> {
    serde_json::from_value(v.clone()).map_err(|_| refused("resale commit"))
}
pub(super) fn reference(v: &Value) -> Result<DefinitionRef, ActionStateError> {
    serde_json::from_value(v.clone()).map_err(|_| refused("resale definition"))
}
pub(super) fn digest_value(v: &Value) -> Result<DefinitionDigest, ActionStateError> {
    serde_json::from_value(v.clone()).map_err(|_| refused("resale digest encoding"))
}
pub(super) fn digest(domain: &str, v: &Value) -> Result<DefinitionDigest, ActionStateError> {
    DefinitionDigest::for_canonical_value_with_limit(domain, v, 40960)
        .map_err(|_| refused("resale digest"))
}
pub(super) fn decimal(v: &Value, max: u64) -> Result<u64, ActionStateError> {
    let s = v.as_str().ok_or_else(|| refused("resale decimal"))?;
    let n: u64 = s.parse().map_err(|_| refused("resale decimal"))?;
    if n == 0 || n > max || n.to_string() != s {
        return Err(refused("resale canonical decimal"));
    }
    Ok(n)
}
pub(super) fn person(v: &Value) -> Result<u64, ActionStateError> {
    let n = v.as_u64().ok_or_else(|| refused("resale person"))?;
    if !(1..=u64::from(u32::MAX)).contains(&n) {
        return Err(refused("resale person"));
    }
    Ok(n)
}
pub(super) fn clock(v: &Value) -> Result<crate::ClockRef, ActionStateError> {
    let c: crate::ClockRef =
        serde_json::from_value(v.clone()).map_err(|_| refused("resale clock"))?;
    if !matches!(c, crate::ClockRef::WorldIc { generation } if generation != 0) {
        return Err(refused("resale WorldIc"));
    }
    Ok(c)
}
pub(super) fn stamp(v: &Value) -> Result<crate::ClockStamp, ActionStateError> {
    let s: crate::ClockStamp =
        serde_json::from_value(v.clone()).map_err(|_| refused("resale stamp"))?;
    clock(&json!(s.clock))?;
    Ok(s)
}
pub(super) fn key(family: &str, encoded: String) -> Result<Key, ActionStateError> {
    Ok(Key {
        owner: OwnerId::new(if family.starts_with("market_") {
            "market"
        } else {
            "roll"
        })
        .map_err(|_| refused("resale owner"))?,
        family: StateFamilyId::new(family).map_err(|_| refused("resale family"))?,
        encoded: encoded.into(),
    })
}
pub(super) fn licence_key(id: u64) -> String {
    format!("licence/{id:016x}")
}
pub(super) fn holder_key(id: u64, generation: u64) -> String {
    format!("licence/{id:016x}/{generation:016x}")
}
pub(super) fn term_key(id: u64, generation: u64) -> String {
    format!("licence/{id:016x}/term/{generation:016x}")
}
pub(super) fn value(view: &View<'_>, k: &Key) -> Result<Option<Value>, ActionStateError> {
    let k = k.resolve(view.registry)?;
    let document = match view.mutations.and_then(|m| m.get(&k)) {
        Some(TxnMutation::Put { document, .. }) => Some(document.clone()),
        Some(TxnMutation::Delete) => None,
        None => view
            .records
            .get(&k)
            .map(|r| {
                serde_json::from_slice(r.canonical.as_slice())
                    .map_err(|_| refused("resale retained encoding"))
            })
            .transpose()?,
    };
    document.map(|v| field(&v, "value").cloned()).transpose()
}
pub(super) fn optional(
    view: &View<'_>,
    family: &str,
    encoded: String,
) -> Result<Option<Value>, ActionStateError> {
    let k = key(family, encoded)?;
    if !view
        .registry
        .entries
        .contains_key(&(k.owner.clone(), k.family.clone()))
    {
        return Ok(None);
    }
    value(view, &k)
}
pub(super) fn required(view: &View<'_>, k: &Key) -> Result<Value, ActionStateError> {
    value(view, k)?.ok_or_else(|| refused("resale paired record missing"))
}
pub(super) fn ownership(v: &Value) -> Result<u64, ActionStateError> {
    shape(v, &["generation", "operation", "digest"])?;
    let generation = num(v, "generation")?;
    if !(1..=65537).contains(&generation) {
        return Err(refused("resale holder generation"));
    }
    token(field(v, "operation")?)?;
    digest_value(field(v, "digest")?)?;
    Ok(generation)
}
pub(super) fn birth(v: &Value) -> Result<(), ActionStateError> {
    shape(v, &["route", "operation", "digest"])?;
    if !matches!(text(v, "route")?, "legacy" | "entitlement" | "house") {
        return Err(refused("resale birth route"));
    }
    token(field(v, "operation")?)?;
    digest_value(field(v, "digest")?)?;
    Ok(())
}
pub(super) fn window(v: &Value) -> Result<(), ActionStateError> {
    shape(
        v,
        &[
            "format",
            "eligibility_start",
            "issued_at",
            "term_end",
            "grace_end",
        ],
    )?;
    let duration = i128::from(signed(v, "term_end")?) - i128::from(signed(v, "eligibility_start")?);
    let grace = i128::from(signed(v, "grace_end")?) - i128::from(signed(v, "term_end")?);
    if num(v, "format")? != 1
        || !(1..=7_776_000).contains(&duration)
        || !(0..=259_200).contains(&grace)
        || signed(v, "issued_at")? >= signed(v, "grace_end")?
    {
        return Err(refused("resale term window"));
    }
    Ok(())
}
pub(super) fn native_basis(v: &Value) -> Result<Value, ActionStateError> {
    shape(
        v,
        &[
            "licence",
            "charter",
            "line",
            "birth",
            "ownership",
            "term",
            "term_clock",
            "economics_digest",
            "royalty_chain_digest",
        ],
    )?;
    let licence = decimal(field(v, "licence")?, u64::MAX)?;
    let charter = decimal(field(v, "charter")?, u64::from(u32::MAX))?;
    let line = decimal(field(v, "line")?, u64::MAX)?;
    let b = field(v, "birth")?;
    birth(b)?;
    let h = field(v, "ownership")?;
    let generation = ownership(h)?;
    if generation > 65536
        || (generation == 1
            && (field(h, "operation")? != field(b, "operation")?
                || field(h, "digest")? != field(b, "digest")?))
    {
        return Err(refused("resale captured holder"));
    }
    let t = field(v, "term")?;
    shape(
        t,
        &[
            "format",
            "licence",
            "generation",
            "window",
            "terms",
            "revision",
            "operation",
        ],
    )?;
    let term_generation = decimal(field(t, "generation")?, u64::MAX)?;
    if num(t, "format")? != 1 || field(t, "licence")? != field(v, "licence")? {
        return Err(refused("resale term identity"));
    }
    token(field(t, "operation")?)?;
    window(field(t, "window")?)?;
    clock(field(v, "term_clock")?)?;
    digest_value(field(v, "economics_digest")?)?;
    digest_value(field(v, "royalty_chain_digest")?)?;
    Ok(
        json!({"licence":licence,"charter":charter,"line":line,"birth":b,"holder":h,
        "term":{"generation":term_generation,"operation":field(t,"operation")?,"digest":digest("roll.paid_resale.term.v1",t)?,
            "clock":field(v,"term_clock")?,"window":field(t,"window")?,
            "terms_digest":digest("roll.paid_resale.terms.v1",field(t,"terms")?)?,
            "policy_digest":digest("roll.paid_resale.policy.v1",field(t,"revision")?)?},
        "economics_digest":field(v,"economics_digest")?,"royalty_chain_digest":field(v,"royalty_chain_digest")?}),
    )
}
pub(super) fn offer(view: &View<'_>, id: CommitToken) -> Result<Value, ActionStateError> {
    let row = required(view, &key(OFFER, id.to_hex())?)?;
    shape(&row, &["format", "operation", "action", "binding", "terms"])?;
    let protocol = view
        .registry
        .paid_resale
        .as_ref()
        .ok_or_else(|| refused("resale protocol absent"))?;
    let terms = field(&row, "terms")?;
    native::terms(terms)?;
    if num(&row, "format")? != 1
        || token(field(&row, "operation")?)? != id
        || reference(field(&row, "action")?)? != protocol.actions[0]
        || field(terms, "offer")? != field(&row, "operation")?
        || field(terms, "offer_action")? != field(&row, "action")?
        || field(terms, "basis")? != &native_basis(field(&row, "binding")?)?
    {
        return Err(refused("resale seller offer differs"));
    }
    Ok(row)
}
pub(super) struct Pair {
    pub(super) row: Value,
    pub(super) market: Value,
    pub(super) offer: Value,
    pub(super) operation: CommitToken,
    pub(super) licence: u64,
    pub(super) generation: u64,
    pub(super) sequence: u64,
    pub(super) phase: String,
}
pub(super) fn pair(view: &View<'_>, id: CommitToken) -> Result<Pair, ActionStateError> {
    let row = required(view, &key(ROLL, id.to_hex())?)?;
    shape(
        &row,
        &[
            "format",
            "operation",
            "action",
            "sequence",
            "previous",
            "offer",
            "offer_digest",
            "binding",
            "claim_observation",
            "economic",
            "state",
        ],
    )?;
    let protocol = view
        .registry
        .paid_resale
        .as_ref()
        .ok_or_else(|| refused("resale protocol absent"))?;
    let offer = offer(view, token(field(&row, "offer")?)?)?;
    let sequence = num(&row, "sequence")?;
    let binding = field(&row, "binding")?;
    let licence = decimal(field(binding, "licence")?, u64::MAX)?;
    let generation = ownership(field(binding, "ownership")?)?;
    let action = reference(field(&row, "action")?)?;
    if num(&row, "format")? != 1
        || token(field(&row, "operation")?)? != id
        || !(1..=65536).contains(&sequence)
        || ![&protocol.actions[1], &protocol.actions[2]].contains(&&action)
        || binding != field(&offer, "binding")?
        || field(&row, "offer_digest")? != &json!(offer_digest(&offer)?)
    {
        return Err(refused("resale immutable operation differs"));
    }
    let previous = field(&row, "previous")?;
    if sequence == 1 {
        if !previous.is_null() {
            return Err(refused("resale first ancestry"));
        }
    } else {
        shape(previous, &["sequence", "operation", "digest"])?;
        if num(previous, "sequence")?.checked_add(1) != Some(sequence)
            || token(field(previous, "operation")?)? == id
        {
            return Err(refused("resale ancestry"));
        }
        digest_value(field(previous, "digest")?)?;
    }
    let claims = field(&row, "claim_observation")?;
    shape(claims, &["generation", "next_transition", "open_paid"])?;
    num(claims, "generation")?;
    if num(claims, "next_transition")? == 0 || num(claims, "open_paid")? > 32768 {
        return Err(refused("resale claim observation"));
    }
    let phase = native::history(view, &row, &offer)?;
    let market = required(view, &key(MARKET, id.to_hex())?)?;
    if market
        != json!({"format":1,"operation":id,"action":field(&row,"action")?,"licence":field(binding,"licence")?,
        "sequence":sequence,"offer":field(&row,"offer")?,"offer_digest":field(&row,"offer_digest")?,
        "roll_digest":digest("roll.paid_resale.record.v1",&row)?,"state":field(&row,"state")?})
    {
        return Err(refused("resale Market pair differs"));
    }
    Ok(Pair {
        row,
        market,
        offer,
        operation: id,
        licence,
        generation,
        sequence,
        phase,
    })
}
pub(super) fn operation_ref(p: &Pair) -> Result<Value, ActionStateError> {
    Ok(
        json!({"sequence":p.sequence,"operation":p.operation,"digest":digest("roll.paid_resale.record.v1",&p.row)?}),
    )
}
pub(super) fn head_matches(view: &View<'_>, p: &Pair) -> Result<(), ActionStateError> {
    if required(view, &key(HEAD, licence_key(p.licence))?)?
        != json!({"format":1,"licence":field(field(&p.row,"binding")?,"licence")?,"current":operation_ref(p)?,"state":field(&p.row,"state")?})
    {
        return Err(refused("resale latest head differs"));
    }
    Ok(())
}
pub(super) fn fingerprint(p: &Pair) -> Result<DefinitionDigest, ActionStateError> {
    let mut row = p.row.clone();
    let object = row
        .as_object_mut()
        .ok_or_else(|| refused("resale operation"))?;
    object.remove("state");
    let economic = field(&p.row, "economic")?;
    object.insert(
        "economic".into(),
        json!({"intent":field(economic,"intent")?,"acceptance":field(economic,"acceptance")?}),
    );
    digest("core.paid_resale.capacity.v1", &row)
}
pub(super) fn terminal(p: &Pair) -> Result<CommitToken, ActionStateError> {
    token(field(field(&p.row, "state")?, "terminal")?)
}
pub(super) fn receipt(p: &Pair) -> Result<Value, ActionStateError> {
    Ok(
        json!({"format":1,"operation":terminal(p)?,"acceptance":p.operation,"licence":field(field(&p.row,"binding")?,"licence")?,
        "offer":field(&p.row,"offer")?,"offer_digest":field(&p.row,"offer_digest")?,"roll_digest":digest("roll.paid_resale.record.v1",&p.row)?,
        "economic_digest":digest("economy.paid_resale.history.v1",field(&p.row,"economic")?)?,"outcome":p.phase}),
    )
}

pub(super) fn offer_digest(row: &Value) -> Result<DefinitionDigest, ActionStateError> {
    DefinitionDigest::for_canonical_value_with_limit("market.paid_resale.offer.v1", row, 16384)
        .map_err(|_| refused("resale offer digest"))
}
