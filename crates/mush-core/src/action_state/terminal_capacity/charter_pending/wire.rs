//! Closed data joins. Actual native journal and title authority are audited by
//! Economy; these checks bind its retained evidence to the fixed typed targets.
use super::*;
pub(super) fn field<'a>(v: &'a Value, n: &str) -> Result<&'a Value, ActionStateError> {
    v.get(n).ok_or_else(|| refused("pending field missing"))
}
pub(super) fn shape(v: &Value, names: &[&str]) -> Result<(), ActionStateError> {
    let o = v.as_object().ok_or_else(|| refused("pending object"))?;
    if o.len() != names.len() || names.iter().any(|n| !o.contains_key(*n)) {
        return Err(refused("pending fields differ"));
    }
    Ok(())
}
pub(super) fn num(v: &Value, n: &str) -> Result<u64, ActionStateError> {
    field(v, n)?
        .as_u64()
        .ok_or_else(|| refused("pending integer"))
}
pub(super) fn signed(v: &Value, n: &str) -> Result<i64, ActionStateError> {
    field(v, n)?
        .as_i64()
        .ok_or_else(|| refused("pending signed integer"))
}
pub(super) fn text<'a>(v: &'a Value, n: &str) -> Result<&'a str, ActionStateError> {
    field(v, n)?.as_str().ok_or_else(|| refused("pending text"))
}
pub(super) fn token(v: &Value) -> Result<CommitToken, ActionStateError> {
    serde_json::from_value(v.clone()).map_err(|_| refused("pending commit"))
}
pub(super) fn reference(v: &Value) -> Result<DefinitionRef, ActionStateError> {
    serde_json::from_value(v.clone()).map_err(|_| refused("pending definition"))
}
pub(super) fn digest(domain: &str, v: &Value) -> Result<DefinitionDigest, ActionStateError> {
    DefinitionDigest::for_canonical_value_with_limit(domain, v, 40960)
        .map_err(|_| refused("pending digest"))
}
fn digest_value(v: &Value) -> Result<DefinitionDigest, ActionStateError> {
    serde_json::from_value(v.clone()).map_err(|_| refused("pending digest encoding"))
}
pub(super) fn decimal(v: &Value, max: u64) -> Result<u64, ActionStateError> {
    let s = v.as_str().ok_or_else(|| refused("pending decimal"))?;
    let n: u64 = s.parse().map_err(|_| refused("pending decimal"))?;
    if n == 0 || n > max || n.to_string() != s {
        return Err(refused("pending canonical decimal"));
    }
    Ok(n)
}
pub(super) fn stamp(v: &Value) -> Result<crate::ClockStamp, ActionStateError> {
    let s: crate::ClockStamp =
        serde_json::from_value(v.clone()).map_err(|_| refused("pending stamp"))?;
    if !matches!(s.clock,crate::ClockRef::WorldIc{generation} if generation!=0) {
        return Err(refused("pending WorldIc"));
    }
    Ok(s)
}
pub(super) fn key(family: &str, encoded: String) -> Result<Key, ActionStateError> {
    Ok(Key {
        owner: OwnerId::new("roll").map_err(|_| refused("pending owner"))?,
        family: StateFamilyId::new(family).map_err(|_| refused("pending family"))?,
        encoded: encoded.into(),
    })
}
pub(super) fn charter_key(id: u64) -> String {
    format!("charter/{id:016x}")
}
pub(super) fn transfer_key(id: u64, generation: u64) -> String {
    format!("charter/{id:016x}/{generation:016x}")
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
                    .map_err(|_| refused("pending retained encoding"))
            })
            .transpose()?,
    };
    document.map(|v| field(&v, "value").cloned()).transpose()
}
pub(super) fn required(view: &View<'_>, k: &Key) -> Result<Value, ActionStateError> {
    value(view, k)?.ok_or_else(|| refused("pending paired record missing"))
}
fn ownership(v: &Value) -> Result<u64, ActionStateError> {
    shape(v, &["generation", "operation", "digest"])?;
    let g = num(v, "generation")?;
    if !(1..=65537).contains(&g) {
        return Err(refused("pending ownership generation"));
    }
    token(field(v, "operation")?)?;
    digest_value(field(v, "digest")?)?;
    Ok(g)
}
fn native_term(v: &Value) -> Result<Value, ActionStateError> {
    shape(
        v,
        &["licence", "generation", "operation", "digest", "window"],
    )?;
    let licence = decimal(field(v, "licence")?, u64::MAX)?;
    let generation = decimal(field(v, "generation")?, u64::MAX)?;
    token(field(v, "operation")?)?;
    digest_value(field(v, "digest")?)?;
    let w = field(v, "window")?;
    shape(
        w,
        &[
            "format",
            "eligibility_start",
            "issued_at",
            "term_end",
            "grace_end",
        ],
    )?;
    let start = signed(w, "eligibility_start")?;
    let issued = signed(w, "issued_at")?;
    let end = signed(w, "term_end")?;
    let grace = signed(w, "grace_end")?;
    if num(w, "format")? != 1 || start > issued || issued >= end || end > grace {
        return Err(refused("pending term window"));
    }
    Ok(
        json!({"licence":licence,"generation":generation,"operation":field(v,"operation")?,"digest":field(v,"digest")?,"term_end":end,"grace_end":grace}),
    )
}
pub(super) fn offer(view: &View<'_>, id: CommitToken) -> Result<Value, ActionStateError> {
    let v = required(view, &key(OFFER, id.to_hex())?)?;
    shape(
        &v,
        &[
            "format",
            "operation",
            "action",
            "charter",
            "birth",
            "ownership",
            "seller",
            "buyer",
            "seller_account",
            "instrument",
            "price",
            "title_revision",
            "line",
            "filed",
            "revision",
            "owner_term",
            "offered_at_ic",
            "offered_at_utc",
            "expires_at_utc",
            "settle_seconds",
        ],
    )?;
    let protocol = view
        .registry
        .charter_pending
        .as_ref()
        .ok_or_else(|| refused("pending protocol unavailable"))?;
    let seller = num(&v, "seller")?;
    let buyer = num(&v, "buyer")?;
    let at = signed(&v, "offered_at_utc")?;
    let expiry = signed(&v, "expires_at_utc")?;
    if num(&v, "format")? != 1
        || token(field(&v, "operation")?)? != id
        || reference(field(&v, "action")?)? != protocol.actions[0]
        || seller == 0
        || seller > u32::MAX as u64
        || buyer == 0
        || buyer > u32::MAX as u64
        || seller == buyer
        || signed(&v, "price")? <= 0
        || !(1..=u64::MAX - 1).contains(&num(&v, "title_revision")?)
        || !(1..=259200).contains(&signed(&v, "settle_seconds")?)
        || i128::from(expiry) - i128::from(at) <= 0
        || i128::from(expiry) - i128::from(at) > 86400
    {
        return Err(refused("pending offer differs"));
    }
    decimal(field(&v, "charter")?, u32::MAX as u64)?;
    decimal(field(&v, "line")?, u64::MAX)?;
    decimal(field(&v, "filed")?, u64::MAX)?;
    token(field(&v, "birth")?)?;
    if ownership(field(&v, "ownership")?)? > 65536 {
        return Err(refused("pending final ownership already reached"));
    }
    native_term(field(&v, "owner_term")?)?;
    stamp(field(&v, "offered_at_ic")?)?;
    Ok(v)
}
pub(super) fn native_terms(offer: &Value) -> Result<Value, ActionStateError> {
    let mut o = offer
        .as_object()
        .ok_or_else(|| refused("pending offer"))?
        .clone();
    for n in ["operation", "action", "line", "filed", "revision"] {
        o.remove(n);
    }
    o.insert("offer".into(), field(offer, "operation")?.clone());
    o.insert("offer_action".into(), field(offer, "action")?.clone());
    o.insert(
        "offer_digest".into(),
        json!(digest("roll.charter_pending.offer.v1", offer)?),
    );
    o.insert(
        "charter".into(),
        json!(decimal(field(offer, "charter")?, u32::MAX as u64)?),
    );
    o.insert(
        "owner_term".into(),
        native_term(field(offer, "owner_term")?)?,
    );
    Ok(Value::Object(o))
}
pub(super) struct Pair {
    pub(super) row: Value,
    pub(super) offer: Value,
    pub(super) operation: CommitToken,
    pub(super) charter: u64,
    pub(super) generation: u64,
    pub(super) sequence: u64,
    pub(super) phase: String,
}
pub(super) fn pair(view: &View<'_>, id: CommitToken) -> Result<Pair, ActionStateError> {
    let row = required(view, &key(PENDING, id.to_hex())?)?;
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
            "charter",
            "ownership",
            "owner_term",
            "accepted_at_ic",
            "deadline",
            "claim_observation",
            "economic",
            "state",
        ],
    )?;
    let protocol = view
        .registry
        .charter_pending
        .as_ref()
        .ok_or_else(|| refused("pending protocol unavailable"))?;
    let o = offer(view, token(field(&row, "offer")?)?)?;
    let seq = num(&row, "sequence")?;
    let charter = decimal(field(&row, "charter")?, u32::MAX as u64)?;
    let generation = ownership(field(&row, "ownership")?)?;
    if num(&row, "format")? != 1
        || token(field(&row, "operation")?)? != id
        || reference(field(&row, "action")?)? != protocol.actions[1]
        || !(1..=65536).contains(&seq)
        || field(&row, "offer_digest")? != &json!(digest("roll.charter_pending.offer.v1", &o)?)
        || ["charter", "ownership", "owner_term"]
            .iter()
            .any(|n| row.get(*n) != o.get(*n))
    {
        return Err(refused("pending immutable identity differs"));
    }
    let previous = field(&row, "previous")?;
    if seq == 1 {
        if !previous.is_null() {
            return Err(refused("pending first ancestry"));
        }
    } else {
        shape(previous, &["sequence", "operation", "digest"])?;
        let prior = token(field(previous, "operation")?)?;
        if num(previous, "sequence")?.checked_add(1) != Some(seq)
            || prior == id
            || field(previous, "operation")? == field(&row, "offer")?
        {
            return Err(refused("pending ancestry"));
        }
        digest_value(field(previous, "digest")?)?;
    }
    let claims = field(&row, "claim_observation")?;
    shape(claims, &["generation", "next_transition", "open_owned"])?;
    num(claims, "generation")?;
    if num(claims, "next_transition")? == 0 || num(claims, "open_owned")? > u32::MAX as u64 {
        return Err(refused("pending claims"));
    }
    let economic = field(&row, "economic")?;
    shape(economic, &["intent", "funding", "terminal"])?;
    canonical_json_bytes_with_limit(economic, 32768)
        .map_err(|_| refused("pending native history bound"))?;
    let intent = field(economic, "intent")?;
    shape(
        intent,
        &["consent", "issuer", "catalog", "contract", "hold"],
    )?;
    let consent = field(intent, "consent")?;
    shape(
        consent,
        &[
            "format",
            "operation",
            "effect",
            "terms",
            "payer",
            "accepted_at_ic",
            "accepted_at_utc",
            "deadline",
        ],
    )?;
    let accepted = stamp(field(&row, "accepted_at_ic")?)?;
    let offered = stamp(field(&o, "offered_at_ic")?)?;
    let deadline = stamp(field(&row, "deadline")?)?;
    let term = native_term(field(&row, "owner_term")?)?;
    let due = i128::from(accepted.at) + i128::from(signed(&o, "settle_seconds")?);
    let due = due.min(i128::from(signed(&term, "term_end")?));
    if num(consent, "format")? != 1
        || field(consent, "operation")? != field(&row, "operation")?
        || field(consent, "effect")? != field(&row, "action")?
        || field(consent, "terms")? != &native_terms(&o)?
        || field(consent, "accepted_at_ic")? != field(&row, "accepted_at_ic")?
        || field(consent, "deadline")? != field(&row, "deadline")?
        || accepted.clock != offered.clock
        || accepted.at < offered.at
        || accepted.at >= signed(&term, "term_end")?
        || deadline.clock != accepted.clock
        || i128::from(deadline.at) != due
        || signed(consent, "accepted_at_utc")? < signed(&o, "offered_at_utc")?
        || signed(consent, "accepted_at_utc")? >= signed(&o, "expires_at_utc")?
    {
        return Err(refused("pending native consent differs"));
    }
    let funding = field(economic, "funding")?;
    shape(
        funding,
        &["transaction", "journal_seq", "plan_hash", "hold"],
    )?;
    plan_hash(field(funding, "plan_hash")?)?;
    num(funding, "transaction")?;
    num(funding, "journal_seq")?;
    let hold = field(funding, "hold")?;
    shape(
        hold,
        &[
            "id",
            "account",
            "amount",
            "status",
            "placed_by",
            "beneficiary",
            "purpose",
            "holder",
            "expires",
            "revision",
            "last_journal_seq",
        ],
    )?;
    if field(hold, "id")? != field(intent, "hold")?
        || field(hold, "account")? != field(consent, "payer")?
        || field(hold, "amount")? != field(&o, "price")?
        || field(hold, "status")? != &json!("Held")
        || field(hold, "placed_by")? != &json!({"Object":field(&o,"buyer")?})
        || field(hold, "beneficiary")? != &json!({"System":"economy.contract"})
        || field(hold, "purpose")? != &json!({"Escrow":{"contract":field(intent,"contract")?}})
        || field(hold, "holder")?
            != &json!({"namespace":"roll.charter_pending.v1","key":format!("{}/fund",id.to_hex())})
        || !field(hold, "expires")?.is_null()
        || num(hold, "revision")? != 1
        || field(hold, "last_journal_seq")? != field(funding, "journal_seq")?
    {
        return Err(refused("pending actual funding hold differs"));
    }
    let state = field(&row, "state")?;
    let phase = text(state, "phase")?.to_owned();
    let terminal = field(economic, "terminal")?;
    if phase == "funded" {
        shape(state, &["phase"])?;
        if !terminal.is_null() {
            return Err(refused("pending premature terminal"));
        }
    } else {
        shape(
            terminal,
            &[
                "decision",
                "transaction",
                "journal_seq",
                "plan_hash",
                "hold",
                "title",
            ],
        )?;
        plan_hash(field(terminal, "plan_hash")?)?;
        let decision = field(terminal, "decision")?;
        shape(
            decision,
            &[
                "format",
                "operation",
                "action",
                "actor",
                "at_ic",
                "posted_at_utc",
                "reason",
            ],
        )?;
        let commit = token(field(decision, "operation")?)?;
        let action = reference(field(decision, "action")?)?;
        let terminal_at = stamp(field(decision, "at_ic")?)?;
        let reason = field(decision, "reason")?;
        let kind = text(reason, "kind")?;
        let settles = matches!(kind, "last_claim_closed" | "deadline_reached");
        if num(decision, "format")? != 1
            || commit == id
            || field(state, "terminal")? != field(decision, "operation")?
            || num(terminal, "transaction")? == num(funding, "transaction")?
            || num(terminal, "journal_seq")? <= num(funding, "journal_seq")?
        {
            return Err(refused("pending terminal identity differs"));
        }
        if !(1..=u64::from(u32::MAX)).contains(&num(decision, "actor")?)
            || signed(decision, "posted_at_utc")? < signed(consent, "accepted_at_utc")?
            || [
                field(&o, "operation")?,
                field(&o, "birth")?,
                field(field(&o, "ownership")?, "operation")?,
                field(field(&o, "owner_term")?, "operation")?,
            ]
            .iter()
            .any(|v| **v == json!(commit))
        {
            return Err(refused("pending terminal consent affinity"));
        }
        canonical_json_bytes_with_limit(decision, 2048)
            .map_err(|_| refused("pending decision bound"))?;
        match kind {
            "last_claim_closed" => {
                shape(reason, &["kind", "next_transition"])?;
                if num(reason, "next_transition")? == 0
                    || terminal_at.clock != accepted.clock
                    || terminal_at.at < accepted.at
                {
                    return Err(refused("pending close clock"));
                }
            }
            "deadline_reached" => {
                shape(reason, &["kind", "next_transition", "open_owned"])?;
                if num(reason, "next_transition")? == 0
                    || num(reason, "open_owned")? > u32::MAX as u64
                    || terminal_at.clock != deadline.clock
                    || terminal_at.at < deadline.at
                {
                    return Err(refused("pending deadline clock"));
                }
            }
            "clock_changed" => {
                shape(reason, &["kind"])?;
                if terminal_at.clock == accepted.clock {
                    return Err(refused("pending unchanged clock"));
                }
            }
            "recipient_unavailable" => {
                shape(reason, &["kind"])?;
                if terminal_at.clock != accepted.clock || terminal_at.at < accepted.at {
                    return Err(refused("pending unavailable clock"));
                }
            }
            "registry_revoked" => {
                shape(
                    reason,
                    &["kind", "sanction", "licence", "generation", "term_digest"],
                )?;
                if num(reason, "licence")?
                    != decimal(field(field(&row, "owner_term")?, "licence")?, u64::MAX)?
                    || num(reason, "generation")?
                        != decimal(field(field(&row, "owner_term")?, "generation")?, u64::MAX)?
                    || field(reason, "term_digest")? != field(field(&row, "owner_term")?, "digest")?
                {
                    return Err(refused("pending revocation term"));
                }
                token(field(reason, "sanction")?)?;
            }
            _ => return Err(refused("pending terminal reason")),
        }
        let mut expected = hold.clone();
        let hm = expected
            .as_object_mut()
            .ok_or_else(|| refused("pending hold"))?;
        hm.insert("status".into(),if settles {json!({"Consumed":{"transaction":field(terminal,"transaction")?,"amount":field(&o,"price")?}})}else{json!("Released")});
        hm.insert("revision".into(), json!(2));
        hm.insert(
            "last_journal_seq".into(),
            field(terminal, "journal_seq")?.clone(),
        );
        if field(terminal, "hold")? != &expected {
            return Err(refused("pending terminal hold differs"));
        }
        if settles {
            shape(state, &["phase", "terminal", "transfer"])?;
            let transfer = field(state, "transfer")?;
            if phase != "settled"
                || action != protocol.actions[2]
                || ownership(transfer)? != generation + 1
                || field(transfer, "operation")? != field(decision, "operation")?
            {
                return Err(refused("pending settled state"));
            }
            let expected = json!({"object":charter,"title_owner":{"Object":field(&o,"buyer")?},"scope":null,"catalog":field(intent,"catalog")?,"encumbrances":[],"assignment":null,"maintenance_account":null,"acquisition":{"transaction":field(terminal,"transaction")?,"at":field(decision,"posted_at_utc")?},"lifecycle":"Active","last_journal_seq":field(terminal,"journal_seq")?,"revision":num(&o,"title_revision")?+1,"stolen":null,"frozen":false,"frozen_reason":null});
            if field(terminal, "title")? != &expected {
                return Err(refused("pending terminal title differs"));
            }
        } else {
            shape(state, &["phase", "terminal", "reason"])?;
            if phase != "refunded"
                || field(state, "reason")? != reason
                || !field(terminal, "title")?.is_null()
            {
                return Err(refused("pending refunded state"));
            }
            if kind == "registry_revoked"
                && field(reason, "sanction")? == field(decision, "operation")?
            {
                verify_sanction(view, &row, &o, decision, true)?;
            } else if action != protocol.actions[3] {
                return Err(refused("pending refund action"));
            }
            if kind == "registry_revoked" {
                verify_sanction(view, &row, &o, decision, false)?;
            }
        }
    }
    Ok(Pair {
        row,
        offer: o,
        operation: id,
        charter,
        generation,
        sequence: seq,
        phase,
    })
}
fn plan_hash(v: &Value) -> Result<(), ActionStateError> {
    let _: [u8; 32] =
        serde_json::from_value(v.clone()).map_err(|_| refused("pending plan hash"))?;
    Ok(())
}
pub(super) fn funding_value(p: &Pair) -> Result<Value, ActionStateError> {
    let mut v = p.row.clone();
    let o = v.as_object_mut().ok_or_else(|| refused("pending row"))?;
    o.remove("state");
    let economic = field(&p.row, "economic")?;
    o.insert("economic".into(),json!({"consent":field(field(economic,"intent")?,"consent")?,"funding":field(economic,"funding")?}));
    Ok(v)
}
pub(super) fn fingerprint(p: &Pair) -> Result<DefinitionDigest, ActionStateError> {
    digest(
        "core.charter_pending.capacity.v1",
        &json!({"accepted":funding_value(p)?,"intent":field(field(&p.row,"economic")?,"intent")?}),
    )
}
pub(super) fn pending_ref(p: &Pair) -> Result<Value, ActionStateError> {
    Ok(
        json!({"sequence":p.sequence,"operation":p.operation,"digest":digest("roll.charter_pending.record.v1",&p.row)?}),
    )
}
pub(super) fn head_matches(view: &View<'_>, p: &Pair) -> Result<(), ActionStateError> {
    let head = required(view, &key(HEAD, charter_key(p.charter))?)?;
    if head
        != json!({"format":1,"charter":field(&p.row,"charter")?,"current":pending_ref(p)?,"state":field(&p.row,"state")?})
    {
        return Err(refused("pending head differs"));
    }
    Ok(())
}
pub(super) fn require_current_ownership(view: &View<'_>, p: &Pair) -> Result<(), ActionStateError> {
    let head = value(view, &key(OWNERSHIP, charter_key(p.charter))?)?;
    if p.generation == 1 {
        if head.is_some() {
            return Err(refused("pending unexpected birth head"));
        }
    } else if head
        != Some(
            json!({"format":1,"charter":field(&p.row,"charter")?,"current":field(&p.row,"ownership")?}),
        )
    {
        return Err(refused("pending current ownership differs"));
    }
    Ok(())
}
pub(super) fn transfer_value(p: &Pair) -> Result<Value, ActionStateError> {
    let state = field(&p.row, "state")?;
    let economic = field(&p.row, "economic")?;
    let decision = field(field(economic, "terminal")?, "decision")?;
    Ok(
        json!({"format":2,"operation":field(state,"terminal")?,"action":field(decision,"action")?,"charter":field(&p.row,"charter")?,"generation":p.generation+1,"birth":field(&p.offer,"birth")?,"predecessor":field(&p.row,"ownership")?,"offer":field(&p.row,"offer")?,"offer_digest":field(&p.row,"offer_digest")?,"accepted_at_ic":field(&p.row,"accepted_at_ic")?,"economic":{"pending":economic}}),
    )
}
pub(super) fn require_transfer(view: &View<'_>, p: &Pair) -> Result<(), ActionStateError> {
    let row = required(
        view,
        &key(TRANSFER, transfer_key(p.charter, p.generation + 1))?,
    )?;
    if row != transfer_value(p)?
        || field(field(&p.row, "state")?, "transfer")?
            != &json!({"generation":p.generation+1,"operation":field(&row,"operation")?,"digest":digest("roll.charter_ownership.transfer.v2",&row)?})
    {
        return Err(refused("pending transferred ownership differs"));
    }
    Ok(())
}
pub(super) fn require_new_ownership(view: &View<'_>, p: &Pair) -> Result<(), ActionStateError> {
    let head = required(view, &key(OWNERSHIP, charter_key(p.charter))?)?;
    if head
        != json!({"format":1,"charter":field(&p.row,"charter")?,"current":field(field(&p.row,"state")?,"transfer")?})
    {
        return Err(refused("pending transferred head differs"));
    }
    Ok(())
}

pub(super) fn sanction_keys(row: &Value, decision: &Value) -> Result<[Key; 2], ActionStateError> {
    let reason = field(decision, "reason")?;
    let term = field(row, "owner_term")?;
    Ok([
        key(SANCTION, token(field(reason, "sanction")?)?.to_hex())?,
        key(
            STATUS,
            format!(
                "licence/{:016x}/term/{:016x}",
                decimal(field(term, "licence")?, u64::MAX)?,
                decimal(field(term, "generation")?, u64::MAX)?
            ),
        )?,
    ])
}
fn verify_sanction(
    view: &View<'_>,
    row: &Value,
    offer: &Value,
    decision: &Value,
    compound: bool,
) -> Result<(), ActionStateError> {
    let protocol = view
        .registry
        .charter_pending
        .as_ref()
        .ok_or_else(|| refused("pending protocol unavailable"))?;
    if protocol.registry_revocation.is_none() {
        return Err(refused("pending Registry capability absent"));
    }
    let [cause_key, status_key] = sanction_keys(row, decision)?;
    let cause = required(view, &cause_key)?;
    shape(
        &cause,
        &[
            "format",
            "operation",
            "action",
            "authority",
            "actor",
            "authorization",
            "term",
            "charter",
            "line",
            "birth_operation",
            "initial_operation",
            "initial_issuer",
            "filed",
            "revision",
            "reason",
            "at_ic",
        ],
    )?;
    let accepted = field(&cause, "term")?;
    shape(accepted, &["kind", "term"])?;
    let actual = field(accepted, "term")?;
    let captured = field(row, "owner_term")?;
    let authority = field(&cause, "authority")?;
    let action = protocol::revocation_reference(authority, &protocol.participants[1])?;
    if num(&cause, "format")? != 1
        || text(accepted, "kind")? != "owner"
        || field(&cause, "operation")? != field(field(decision, "reason")?, "sanction")?
        || reference(field(&cause, "action")?)? != action
        || field(&cause, "charter")? != field(row, "charter")?
        || field(&cause, "line")? != field(offer, "line")?
        || field(&cause, "birth_operation")? != field(offer, "birth")?
        || text(&cause, "reason")? != "registry_review"
        || ["licence", "generation", "operation", "window"]
            .iter()
            .any(|n| actual.get(*n) != captured.get(*n))
        || field(captured, "digest")?
            != &json!(digest("roll.charter_pending.owner_term.v1", actual)?)
    {
        return Err(refused("pending exact Registry cause differs"));
    }
    let at = stamp(field(&cause, "at_ic")?)?;
    let window = field(actual, "window")?;
    if at.at < signed(window, "eligibility_start")? || at.at >= signed(window, "grace_end")? {
        return Err(refused("pending Registry original term window"));
    }
    let status = required(view, &status_key)?;
    if status
        != json!({"format":1,"licence":field(actual,"licence")?,"generation":field(actual,"generation")?,"operation":field(&cause,"operation")?,"term_digest":digest("roll.registry_revocation.accepted_term.v1",accepted)?,"window":window})
    {
        return Err(refused("pending exact Registry status differs"));
    }
    if compound
        && (field(&cause, "operation")? != field(decision, "operation")?
            || field(&cause, "actor")? != field(decision, "actor")?
            || field(&cause, "action")? != field(decision, "action")?
            || field(&cause, "at_ic")? != field(decision, "at_ic")?)
    {
        return Err(refused("pending same-union Registry evidence differs"));
    }
    Ok(())
}
pub(super) fn require_terminal_room(p: &Pair, protocol: &Protocol) -> Result<(), ActionStateError> {
    // A native history is bounded to 32768 complete canonical bytes by its actual
    // strict codec. Substitute precisely that size only to bound future storage;
    // this sizing value is never installed or used as native authority.
    let padding = Value::String("x".repeat(32766));
    let terminal = CommitToken::from_bytes([255; 16]);
    let transfer = json!({"generation":p.generation+1,"operation":terminal,"digest":DefinitionDigest::from_bytes([255;32])});
    let mut pending = p.row.clone();
    let o = pending
        .as_object_mut()
        .ok_or_else(|| refused("pending row"))?;
    o.insert("economic".into(), padding.clone());
    o.insert(
        "state".into(),
        json!({"phase":"refunded","terminal":terminal,"reason":Value::String("x".repeat(2046))}),
    );
    envelope(PENDING, p.operation.to_hex(), pending, 40960)?;
    let ownership = json!({"format":2,"operation":terminal,"action":protocol.actions[2],"charter":field(&p.row,"charter")?,"generation":p.generation+1,"birth":field(&p.offer,"birth")?,"predecessor":field(&p.row,"ownership")?,"offer":field(&p.row,"offer")?,"offer_digest":field(&p.row,"offer_digest")?,"accepted_at_ic":field(&p.row,"accepted_at_ic")?,"economic":{"pending":padding}});
    envelope(
        TRANSFER,
        transfer_key(p.charter, p.generation + 1),
        ownership,
        40960,
    )?;
    envelope(
        HEAD,
        charter_key(p.charter),
        json!({"format":1,"charter":field(&p.row,"charter")?,"current":{"sequence":p.sequence,"operation":p.operation,"digest":DefinitionDigest::from_bytes([255;32])},"state":{"phase":"refunded","terminal":terminal,"reason":Value::String("x".repeat(2046))}}),
        4096,
    )?;
    envelope(
        OWNERSHIP,
        charter_key(p.charter),
        json!({"format":1,"charter":field(&p.row,"charter")?,"current":transfer}),
        4096,
    )
}
fn envelope(family: &str, key: String, row: Value, max: usize) -> Result<(), ActionStateError> {
    canonical_json_bytes_with_limit(&json!({"family":{"owner":"roll","id":family},"schema":{"id":format!("{family}.value.v1"),"version":1},"key":key,"value":row}),max).map(|_|()).map_err(|_|refused("pending full terminal envelope"))
}
