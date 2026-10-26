//! Fixed native receipt affinity. Economy separately proves the actual journal,
//! consent, account identities and gross payout plan.
use super::*;

fn bound(v: &Value, bytes: usize) -> Result<(), ActionStateError> {
    canonical_json_bytes_with_limit(v, bytes)
        .map(|_| ())
        .map_err(|_| refused("resale native bound"))
}
fn recipient(v: &Value, accounts: &mut BTreeMap<u64, Value>) -> Result<(), ActionStateError> {
    shape(v, &["account", "owner"])?;
    let account = num(v, "account")?;
    let owner = field(v, "owner")?;
    // The actual native owner decoder validates PrincipalId's closed enum.
    // Here retain exact account/owner equality, including rounded-zero roles.
    if accounts
        .insert(account, owner.clone())
        .is_some_and(|old| old != *owner)
    {
        return Err(refused("resale conflicting account owner"));
    }
    Ok(())
}
pub(super) fn terms(v: &Value) -> Result<(), ActionStateError> {
    shape(
        v,
        &[
            "format",
            "offer",
            "offer_action",
            "basis",
            "seller",
            "buyer",
            "instrument",
            "price",
            "registry",
            "venue",
            "parents",
            "remainder",
            "offered_at_ic",
            "offered_at_utc",
            "expires_at_utc",
            "settle_seconds",
        ],
    )?;
    let seller = person(field(v, "seller")?)?;
    let buyer = person(field(v, "buyer")?)?;
    let offered = token(field(v, "offer")?)?;
    reference(field(v, "offer_action")?)?;
    stamp(field(v, "offered_at_ic")?)?;
    let duration =
        i128::from(signed(v, "expires_at_utc")?) - i128::from(signed(v, "offered_at_utc")?);
    if num(v, "format")? != 1
        || seller == buyer
        || signed(v, "price")? <= 0
        || !(1..=86400).contains(&duration)
        || signed(v, "settle_seconds")? != 259200
    {
        return Err(refused("resale terms differ"));
    }
    let basis = field(v, "basis")?;
    shape(
        basis,
        &[
            "licence",
            "charter",
            "line",
            "birth",
            "holder",
            "term",
            "economics_digest",
            "royalty_chain_digest",
        ],
    )?;
    for n in ["birth", "holder", "term"] {
        if token(field(field(basis, n)?, "operation")?)? == offered {
            return Err(refused("resale offer reuses prior operation"));
        }
    }
    let mut accounts = BTreeMap::new();
    recipient(field(v, "remainder")?, &mut accounts)?;
    if field(field(v, "remainder")?, "owner")? != &json!({"Object":seller}) {
        return Err(refused("resale seller remainder"));
    }
    let mut total = 0u64;
    for cut in [field(v, "registry")?, field(v, "venue")?] {
        if cut.is_null() {
            continue;
        }
        shape(cut, &["permille", "recipient"])?;
        let rate = num(cut, "permille")?;
        let r = field(cut, "recipient")?;
        if rate > 1000 || (rate == 0) != r.is_null() {
            return Err(refused("resale gross cut"));
        }
        total += rate;
        if !r.is_null() {
            recipient(r, &mut accounts)?;
        }
    }
    if field(v, "registry")?.is_null() {
        return Err(refused("resale registry cut missing"));
    }
    let parents = field(v, "parents")?
        .as_array()
        .ok_or_else(|| refused("resale parent cuts"))?;
    if parents.len() > 3 {
        return Err(refused("resale parent count"));
    }
    let mut seen = BTreeSet::from([num(basis, "charter")?]);
    let mut parent_total = 0u64;
    for cut in parents {
        shape(cut, &["charter", "permille", "title"])?;
        let charter = person(field(cut, "charter")?)?;
        let rate = num(cut, "permille")?;
        let title = field(cut, "title")?;
        if !seen.insert(charter) || rate > 300 || (rate == 0) != title.is_null() {
            return Err(refused("resale parent cut"));
        }
        parent_total += rate;
        if !title.is_null() {
            shape(title, &["title_revision", "recipient"])?;
            if num(title, "title_revision")? == 0 {
                return Err(refused("resale title revision"));
            }
            let r = field(title, "recipient")?;
            recipient(r, &mut accounts)?;
            let owner = field(r, "owner")?;
            shape(owner, &["Object"])?;
            person(field(owner, "Object")?)?;
        }
    }
    if parent_total > 300 || total + parent_total > 1000 {
        return Err(refused("resale aggregate cuts"));
    }
    bound(v, 8192)
}
fn plan_hash(v: &Value) -> Result<(), ActionStateError> {
    let _: [u8; 32] = serde_json::from_value(v.clone()).map_err(|_| refused("resale plan hash"))?;
    Ok(())
}
pub(super) fn history(
    view: &View<'_>,
    row: &Value,
    offer: &Value,
) -> Result<String, ActionStateError> {
    let protocol = view
        .registry
        .paid_resale
        .as_ref()
        .ok_or_else(|| refused("resale protocol absent"))?;
    let economic = field(row, "economic")?;
    shape(economic, &["intent", "acceptance", "terminal"])?;
    bound(economic, 32768)?;
    let intent = field(economic, "intent")?;
    shape(intent, &["consent", "issuer", "contract", "hold"])?;
    num(intent, "contract")?;
    if field(intent, "issuer")?.as_str().is_none_or(str::is_empty) {
        return Err(refused("resale issuer encoding"));
    }
    let c = field(intent, "consent")?;
    shape(
        c,
        &[
            "format",
            "operation",
            "effect",
            "offer_digest",
            "terms",
            "payer",
            "mode",
            "accepted_at_ic",
            "accepted_at_utc",
            "deadline",
            "claims",
        ],
    )?;
    let immediate = match text(c, "mode")? {
        "immediate" => true,
        "pending" => false,
        _ => return Err(refused("resale mode")),
    };
    let accepted = stamp(field(c, "accepted_at_ic")?)?;
    let t = field(offer, "terms")?;
    let offered = stamp(field(t, "offered_at_ic")?)?;
    let deadline = stamp(field(c, "deadline")?)?;
    let binding = field(row, "binding")?;
    let term = field(binding, "term")?;
    let end = signed(field(term, "window")?, "term_end")?;
    let due = i128::from(end).min(i128::from(accepted.at) + 259200);
    let observation = field(row, "claim_observation")?;
    let claims = field(c, "claims")?;
    if num(c, "format")? != 1
        || field(c, "operation")? != field(row, "operation")?
        || field(c, "effect")? != field(row, "action")?
        || reference(field(row, "action")?)? != protocol.actions[if immediate { 1 } else { 2 }]
        || field(c, "offer_digest")? != field(row, "offer_digest")?
        || field(c, "terms")? != t
        || claims
            != &json!({"next_transition":field(observation,"next_transition")?,"open_paid":field(observation,"open_paid")?})
        || immediate != (num(claims, "open_paid")? == 0)
        || accepted.clock != offered.clock
        || accepted.at < offered.at
        || deadline.clock != accepted.clock
        || i128::from(deadline.at) != due
        || signed(c, "accepted_at_utc")? < signed(t, "offered_at_utc")?
        || signed(c, "accepted_at_utc")? >= signed(t, "expires_at_utc")?
    {
        return Err(refused("resale consent affinity"));
    }
    num(c, "payer")?;
    for v in [
        field(offer, "operation")?,
        field(field(binding, "birth")?, "operation")?,
        field(field(binding, "ownership")?, "operation")?,
        field(term, "operation")?,
    ] {
        if field(row, "operation")? == v {
            return Err(refused("resale acceptance reuses prior operation"));
        }
    }
    bound(c, 8192)?;
    let a = field(economic, "acceptance")?;
    shape(a, &["transaction", "journal_seq", "plan_hash", "hold"])?;
    num(a, "transaction")?;
    num(a, "journal_seq")?;
    plan_hash(field(a, "plan_hash")?)?;
    let state = field(row, "state")?;
    let phase = text(state, "phase")?;
    let terminal = field(economic, "terminal")?;
    let h = field(a, "hold")?;
    if immediate {
        shape(state, &["phase", "terminal", "ownership"])?;
        if phase != "settled"
            || !h.is_null()
            || !field(intent, "hold")?.is_null()
            || !terminal.is_null()
            || field(state, "terminal")? != field(row, "operation")?
        {
            return Err(refused("resale immediate terminal"));
        }
    } else {
        let operation = token(field(row, "operation")?)?;
        pending_hold(h, intent, c, t, operation, a)?;
        if phase == "funded" {
            shape(state, &["phase"])?;
            if !terminal.is_null() {
                return Err(refused("resale premature terminal"));
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
                ],
            )?;
            plan_hash(field(terminal, "plan_hash")?)?;
            if num(terminal, "transaction")? == num(a, "transaction")?
                || num(terminal, "journal_seq")? <= num(a, "journal_seq")?
            {
                return Err(refused("resale terminal journal differs"));
            }
            let d = field(terminal, "decision")?;
            decision(view, row, offer, d)?;
            let reason = field(d, "reason")?;
            let settles = matches!(
                text(reason, "kind")?,
                "last_claim_closed" | "deadline_reached"
            );
            let mut expected = h.clone();
            let object = expected
                .as_object_mut()
                .ok_or_else(|| refused("resale hold"))?;
            object.insert("revision".into(), json!(2));
            object.insert(
                "last_journal_seq".into(),
                field(terminal, "journal_seq")?.clone(),
            );
            object.insert("status".into(),if settles {json!({"Consumed":{"transaction":field(terminal,"transaction")?,"amount":field(t,"price")?}})}else{json!("Released")});
            if field(terminal, "hold")? != &expected
                || field(state, "terminal")? != field(d, "operation")?
            {
                return Err(refused("resale terminal hold affinity"));
            }
            if settles {
                shape(state, &["phase", "terminal", "ownership"])?;
                if phase != "settled" {
                    return Err(refused("resale settled phase"));
                }
            } else {
                shape(state, &["phase", "terminal", "reason"])?;
                if phase != "refunded" || field(state, "reason")? != reason {
                    return Err(refused("resale refund phase"));
                }
            }
        }
    }
    if phase == "settled" {
        let h = field(state, "ownership")?;
        if ownership(h)? != ownership(field(binding, "ownership")?)? + 1
            || field(h, "operation")? != field(state, "terminal")?
        {
            return Err(refused("resale holder successor"));
        }
    }
    Ok(phase.to_owned())
}
#[inline(never)]
fn pending_hold(
    h: &Value,
    intent: &Value,
    c: &Value,
    t: &Value,
    operation: CommitToken,
    a: &Value,
) -> Result<(), ActionStateError> {
    shape(
        h,
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
    num(h, "id")?;
    if field(h, "id")? != field(intent, "hold")?
        || field(h, "account")? != field(c, "payer")?
        || field(h, "amount")? != field(t, "price")?
        || field(h, "status")? != "Held"
        || field(h, "placed_by")? != &json!({"Object":field(t,"buyer")?})
        || field(h, "beneficiary")? != &json!({"System":"economy.contract"})
        || field(h, "purpose")? != &json!({"Escrow":{"contract":field(intent,"contract")?}})
        || field(h, "holder")?
            != &json!({"namespace":"market.paid_resale.v1","key":format!("{}/fund",operation.to_hex())})
        || !field(h, "expires")?.is_null()
        || num(h, "revision")? != 1
        || field(h, "last_journal_seq")? != field(a, "journal_seq")?
    {
        return Err(refused("resale acceptance hold differs"));
    }
    Ok(())
}

fn decision(
    view: &View<'_>,
    row: &Value,
    offer: &Value,
    d: &Value,
) -> Result<(), ActionStateError> {
    shape(
        d,
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
    let protocol = view
        .registry
        .paid_resale
        .as_ref()
        .ok_or_else(|| refused("resale protocol absent"))?;
    let c = field(field(field(row, "economic")?, "intent")?, "consent")?;
    let binding = field(row, "binding")?;
    let term = field(binding, "term")?;
    let at = stamp(field(d, "at_ic")?)?;
    let accepted = stamp(field(c, "accepted_at_ic")?)?;
    let action = reference(field(d, "action")?)?;
    person(field(d, "actor")?)?;
    token(field(d, "operation")?)?;
    if num(d, "format")? != 1 || signed(d, "posted_at_utc")? < signed(c, "accepted_at_utc")? {
        return Err(refused("resale terminal identity"));
    }
    for prior in [
        field(row, "operation")?,
        field(offer, "operation")?,
        field(field(binding, "birth")?, "operation")?,
        field(field(binding, "ownership")?, "operation")?,
        field(term, "operation")?,
    ] {
        if field(d, "operation")? == prior {
            return Err(refused("resale terminal reuses prior operation"));
        }
    }
    let reason = field(d, "reason")?;
    let kind = text(reason, "kind")?;
    let same = at.clock == accepted.clock;
    if kind != "clock_changed" && same && at.at < accepted.at {
        return Err(refused("resale terminal clock rollback"));
    }
    let claims = field(c, "claims")?;
    match kind {
        "last_claim_closed" => {
            shape(reason, &["kind", "next_transition"])?;
            if !same
                || num(reason, "next_transition")? <= num(claims, "next_transition")?
                || action != protocol.actions[3]
            {
                return Err(refused("resale close observation"));
            }
        }
        "deadline_reached" => {
            shape(reason, &["kind", "next_transition", "open_paid"])?;
            if !same
                || at.at < stamp(field(c, "deadline")?)?.at
                || num(reason, "next_transition")? < num(claims, "next_transition")?
                || num(reason, "open_paid")? > num(claims, "open_paid")?
                || action != protocol.actions[3]
            {
                return Err(refused("resale deadline observation"));
            }
        }
        "clock_changed" => {
            shape(reason, &["kind"])?;
            if same || action != protocol.actions[4] {
                return Err(refused("resale changed clock"));
            }
        }
        "recipient_unavailable" | "monetary_binding_changed" => {
            shape(reason, &["kind"])?;
            if !same || action != protocol.actions[4] {
                return Err(refused("resale refund observation"));
            }
        }
        "registry_revoked" => {
            shape(
                reason,
                &["kind", "sanction", "licence", "generation", "term_digest"],
            )?;
            if num(reason, "licence")? != decimal(field(binding, "licence")?, u64::MAX)?
                || num(reason, "generation")? != decimal(field(term, "generation")?, u64::MAX)?
                || field(reason, "term_digest")?
                    != &json!(digest("roll.paid_resale.term.v1", term)?)
                || at.at >= signed(field(term, "window")?, "grace_end")?
            {
                return Err(refused("resale revoked accepted term"));
            }
            let compound = field(reason, "sanction")? == field(d, "operation")?;
            if (!compound && action != protocol.actions[4])
                || (compound && protocol.registry_revocation.as_ref() != Some(&action))
            {
                return Err(refused("resale Registry action"));
            }
            sanction(view, row, d, compound)?;
        }
        _ => return Err(refused("resale terminal reason")),
    }
    bound(d, 2048)
}
pub(super) fn sanction_keys(row: &Value, d: &Value) -> Result<[Key; 2], ActionStateError> {
    let binding = field(row, "binding")?;
    Ok([
        key(
            SANCTION,
            token(field(field(d, "reason")?, "sanction")?)?.to_hex(),
        )?,
        key(
            STATUS,
            term_key(
                decimal(field(binding, "licence")?, u64::MAX)?,
                decimal(field(field(binding, "term")?, "generation")?, u64::MAX)?,
            ),
        )?,
    ])
}
fn sanction(
    view: &View<'_>,
    row: &Value,
    d: &Value,
    compound: bool,
) -> Result<(), ActionStateError> {
    let protocol = view
        .registry
        .paid_resale
        .as_ref()
        .ok_or_else(|| refused("resale protocol absent"))?;
    let [cause_key, status_key] = sanction_keys(row, d)?;
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
    let binding = field(row, "binding")?;
    let term = field(binding, "term")?;
    let accepted = json!({"kind":"paid","term":term});
    let action =
        protocol::revocation_reference(field(&cause, "authority")?, &protocol.participants[1])?;
    if num(&cause, "format")? != 1
        || field(&cause, "term")? != &accepted
        || field(&cause, "operation")? != field(field(d, "reason")?, "sanction")?
        || reference(field(&cause, "action")?)? != action
        || field(&cause, "charter")? != field(binding, "charter")?
        || field(&cause, "line")? != field(binding, "line")?
        || field(&cause, "birth_operation")? != field(field(binding, "birth")?, "operation")?
        || text(&cause, "reason")? != "registry_review"
    {
        return Err(refused("resale exact Registry cause"));
    }
    let at = stamp(field(&cause, "at_ic")?)?;
    let window = field(term, "window")?;
    if at.at < signed(window, "eligibility_start")? || at.at >= signed(window, "grace_end")? {
        return Err(refused("resale Registry term window"));
    }
    if required(view, &status_key)?
        != json!({"format":1,"licence":field(term,"licence")?,"generation":field(term,"generation")?,"operation":field(&cause,"operation")?,"term_digest":digest("roll.registry_revocation.accepted_term.v1",&accepted)?,"window":window})
    {
        return Err(refused("resale Registry status"));
    }
    if compound
        && ["operation", "actor", "action", "at_ic"]
            .iter()
            .any(|n| cause.get(*n) != d.get(*n))
    {
        return Err(refused("resale compound Registry identity"));
    }
    Ok(())
}
