use super::*;
pub(super) fn reservation_id(operation: CommitToken) -> Result<ReservationId, ActionStateError> {
    Ok(ReservationId(
        OwnerId::new("market").map_err(|_| refused("resale owner"))?,
        operation.to_hex(),
    ))
}
pub(super) fn targets(p: &Pair) -> Result<Vec<Target>, ActionStateError> {
    let mut targets = Vec::with_capacity(6);
    for (family, max, mode, encoded) in [
        (ROLL, 40960, SlotMode::UpdateExisting, p.operation.to_hex()),
        (
            MARKET,
            40960,
            SlotMode::UpdateExisting,
            p.operation.to_hex(),
        ),
        (HEAD, 4096, SlotMode::UpdateExisting, licence_key(p.licence)),
        (
            LICENCE,
            4096,
            SlotMode::UpdateExisting,
            licence_key(p.licence),
        ),
        (
            HOLDER,
            32768,
            SlotMode::Create,
            holder_key(p.licence, p.generation + 1),
        ),
        (RECEIPT, 8192, SlotMode::Create, p.operation.to_hex()),
    ] {
        targets.push(Target {
            key: key(family, encoded)?,
            schema: CanonicalId::new(format!("{family}.value.v1"))
                .map_err(|_| refused("resale schema"))?,
            schema_version: 1,
            ceiling: max,
            mode,
        });
    }
    targets.sort_by(|a, b| {
        (&a.key.owner, &a.key.family, &a.key.encoded).cmp(&(
            &b.key.owner,
            &b.key.family,
            &b.key.encoded,
        ))
    });
    Ok(targets)
}
fn envelope(family: &str, encoded: String, row: Value, max: usize) -> Result<(), ActionStateError> {
    let k = key(family, encoded)?;
    canonical_json_bytes_with_limit(&json!({"family":{"owner":k.owner,"id":family},"key":k.encoded,"schema":{"id":format!("{family}.value.v1"),"version":1},"value":row}),max)
        .map(|_|()).map_err(|_|refused("resale complete terminal envelope"))
}
pub(super) fn require_terminal_room(view: &View<'_>, p: &Pair) -> Result<(), ActionStateError> {
    let protocol = view
        .registry
        .paid_resale
        .as_ref()
        .ok_or_else(|| refused("resale protocol absent"))?;
    let commit = CommitToken::from_bytes([255; 16]);
    let max_digest = DefinitionDigest::from_bytes([255; 32]);
    let successor = json!({"generation":p.generation+1,"operation":commit,"digest":max_digest});
    // These are canonical-size projections only. They cannot decode as native
    // evidence and are never passed to an owner or installed as a receipt.
    let history_size = Value::String("x".repeat(32766));
    let reason_size = Value::String("x".repeat(2046));
    for state in [
        json!({"phase":"settled","terminal":commit,"ownership":successor}),
        json!({"phase":"refunded","terminal":commit,"reason":reason_size}),
    ] {
        let mut roll = p.row.clone();
        let o = roll.as_object_mut().ok_or_else(|| refused("resale row"))?;
        o.insert("economic".into(), history_size.clone());
        o.insert("state".into(), state.clone());
        envelope(ROLL, p.operation.to_hex(), roll, 40960)?;
        let mut market = p.market.clone();
        let o = market
            .as_object_mut()
            .ok_or_else(|| refused("resale Market row"))?;
        o.insert("roll_digest".into(), json!(max_digest));
        o.insert("state".into(), state.clone());
        envelope(MARKET, p.operation.to_hex(), market, 40960)?;
        envelope(
            HEAD,
            licence_key(p.licence),
            json!({"format":1,"licence":field(field(&p.row,"binding")?,"licence")?,"current":{"sequence":p.sequence,"operation":p.operation,"digest":max_digest},"state":state}),
            4096,
        )?;
    }
    let binding = field(&p.row, "binding")?;
    let term = field(binding, "term")?;
    let c = field(field(field(&p.row, "economic")?, "intent")?, "consent")?;
    envelope(
        HOLDER,
        holder_key(p.licence, p.generation + 1),
        json!({"format":1,"licence":field(binding,"licence")?,"generation":p.generation+1,"holder":field(field(&p.offer,"terms")?,"buyer")?,"birth":field(binding,"birth")?,"predecessor":field(binding,"ownership")?,"operation":commit,"action":protocol.actions[3],"acceptance":p.operation,"offer":field(&p.row,"offer")?,"offer_digest":field(&p.row,"offer_digest")?,"term":{"generation":field(term,"generation")?,"operation":field(term,"operation")?,"digest":max_digest},"accepted_at_ic":field(c,"accepted_at_ic")?,"economic_digest":max_digest}),
        32768,
    )?;
    let mut pointer = history::require_current(view, p)?;
    let o = pointer
        .as_object_mut()
        .ok_or_else(|| refused("resale pointer"))?;
    o.insert("format".into(), json!(2));
    o.insert(
        "holder".into(),
        field(field(&p.offer, "terms")?, "buyer")?.clone(),
    );
    o.insert("ownership".into(), successor);
    envelope(LICENCE, licence_key(p.licence), pointer, 4096)?;
    envelope(
        RECEIPT,
        p.operation.to_hex(),
        json!({"format":1,"operation":commit,"acceptance":p.operation,"licence":field(binding,"licence")?,"offer":field(&p.row,"offer")?,"offer_digest":field(&p.row,"offer_digest")?,"roll_digest":max_digest,"economic_digest":max_digest,"outcome":"refunded"}),
        8192,
    )
}
pub(in crate::action_state::terminal_capacity) fn verify(
    phase: Phase,
    reservation: &Reservation,
    view: &View<'_>,
) -> Result<(), ActionStateError> {
    let p = pair(view, reservation.funding)?;
    let protocol = view
        .registry
        .paid_resale
        .as_ref()
        .ok_or_else(|| refused("resale protocol absent"))?;
    if reservation.format != 1
        || reservation.effect != protocol.actions[2]
        || reservation.sponsor != key(MARKET, p.operation.to_hex())?
        || reservation.targets != targets(&p)?
        || reservation.protocol_fingerprint != Some(fingerprint(&p)?)
        || reference(field(&p.row, "action")?)? != protocol.actions[2]
        || (matches!(phase, Phase::Funding | Phase::Retained) && p.phase != "funded")
        || (matches!(phase, Phase::Terminal) && p.phase == "funded")
    {
        return Err(refused("resale reservation differs"));
    }
    head_matches(view, &p)?;
    history::outputs(view, &p, matches!(phase, Phase::Terminal))
}
pub(in crate::action_state) fn paid_resale_change(
    txn: &mut ActionTxn<'_>,
    commit: CommitToken,
    terminal: Option<CommitToken>,
) -> Result<(), ActionStateError> {
    let protocol = txn
        .store
        .registry
        .paid_resale
        .as_ref()
        .ok_or_else(|| refused("resale protocol absent"))?;
    let identity = txn
        .registered_identity
        .as_ref()
        .ok_or_else(|| refused("resale actual invocation absent"))?;
    if identity.commit != commit
        || txn.provider_ordinal != Some(identity.ordinal)
        || txn.owner != identity.action.key().owner
        || txn.capacity_changes.funding.is_some()
        || txn.capacity_changes.terminal.is_some()
        || txn.capacity_changes.progress.is_some()
        || txn.capacity_changes.industry_delivery.is_some()
    {
        return Err(refused("resale preparation differs"));
    }
    let action = protocol.action_index(identity)?;
    if let Some(operation) = terminal {
        if !(3..=5).contains(&action) {
            return Err(refused("resale terminal action differs"));
        }
        txn.capacity_changes.terminal = Some(reservation_id(operation)?);
    } else {
        if action != 2 {
            return Err(refused("resale Fund action differs"));
        }
        let view = View {
            registry: &txn.store.registry,
            records: &txn.store.records,
            mutations: Some(&txn.mutations),
        };
        let p = pair(&view, commit)?;
        require_terminal_room(&view, &p)?;
        let r = Reservation {
            format: 1,
            funding: commit,
            effect: protocol.actions[2].clone(),
            sponsor: key(MARKET, commit.to_hex())?,
            targets: targets(&p)?,
            protocol_fingerprint: Some(fingerprint(&p)?),
        };
        verify(Phase::Funding, &r, &view)?;
        txn.capacity_changes.funding = Some(r);
    }
    Ok(())
}
