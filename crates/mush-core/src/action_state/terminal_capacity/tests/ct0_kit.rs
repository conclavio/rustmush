//! Doc 46 CT.0: the shared recorder of the per-family snapshot walks. It writes what a
//! prepared transaction persists, what the capacity index holds after an install, the refusal
//! a forbidden move answers, and proves a cold rebuild from the written documents equals the
//! live store. `mush-testkit` links its own copy of this crate, so every value goes through
//! `Snapshot::wire` (serde), never `Snapshot::replay_writes`.

use super::*;
use mush_testkit::snapshot::Snapshot;

pub(in crate::action_state::terminal_capacity) type CtResult<T = ()> =
    Result<T, Box<dyn std::error::Error>>;

/// Every write `prepared` would persist: each typed record's canonical post-image length and
/// module op target, then every replay write (typed records and the
/// capacity metadata) as the tagged JSON a bundle carries.
pub(in crate::action_state::terminal_capacity) fn prepared(
    snap: &mut Snapshot,
    prepared: &PreparedTxn,
) -> CtResult {
    snap.line("prepared owner", prepared.owner().as_str());
    snap.line(
        "prepared operation count",
        &prepared.operation_count().to_string(),
    );
    snap.line(
        "prepared canonical bytes",
        &prepared.canonical_bytes().to_string(),
    );
    for record in prepared.records() {
        snap.line(
            "record canonical length",
            &record
                .canonical_post_image()
                .map_or(0, |bytes| bytes.as_slice().len())
                .to_string(),
        );
        let op = record.module_op();
        let target = match &op.op {
            ModuleWrite::JsonSet { key, .. } => format!("json_set {} {key}", op.ns),
            ModuleWrite::Del { key } => format!("del {} {key}", op.ns),
            ModuleWrite::StreamAppend { key, .. } => format!("stream_append {} {key}", op.ns),
        };
        snap.line("record module op", &target);
    }
    for write in prepared.authoritative_replay_writes() {
        snap.wire("write", &write)?;
    }
    Ok(())
}

/// The capacity index after an install: every retained reservation's id, path, record in its
/// struct's field order and canonical bytes, and the charged usage.
pub(in crate::action_state::terminal_capacity) fn reservations(
    snap: &mut Snapshot,
    store: &ActionStateStore,
) -> CtResult {
    snap.line("reservations", &store.capacity.records.len().to_string());
    for (id, stored) in &store.capacity.records {
        snap.line("reservation id", &format!("{} {}", id.0.as_str(), id.1));
        snap.line("reservation path", stored.path.as_str());
        snap.wire("reservation record", &stored.record)?;
        snap.bytes("reservation canonical", &stored.canonical);
        snap.line(
            "reservation target ceiling sum",
            &stored
                .record
                .targets
                .iter()
                .map(|target| target.ceiling)
                .sum::<usize>()
                .to_string(),
        );
    }
    snap.line(
        "capacity metadata bytes",
        &store.capacity.metadata_bytes().to_string(),
    );
    snap.line(
        "capacity usage",
        &format!(
            "{} bytes {} records",
            store.capacity_usage.bytes, store.capacity_usage.records
        ),
    );
    Ok(())
}

/// The refusal a forbidden move answers, as its sentence. An accepted move fails the walk.
pub(in crate::action_state::terminal_capacity) fn refusal<T>(
    snap: &mut Snapshot,
    label: &str,
    result: CtResult<T>,
) -> CtResult {
    match result {
        Ok(_) => Err(format!("{label}: the forbidden move was accepted").into()),
        Err(error) => {
            snap.line(&format!("refused {label}"), &error.to_string());
            Ok(())
        }
    }
}

/// A cold rebuild from the written documents equals the live store: the same typed records
/// byte for byte, the same reservations byte for byte, the same charged usage.
pub(in crate::action_state::terminal_capacity) fn same_cold(
    snap: &mut Snapshot,
    live: &ActionStateStore,
    cold: &ActionStateStore,
) -> CtResult {
    let rows = |store: &ActionStateStore| -> Vec<(StateKey, Vec<u8>)> {
        store
            .records
            .iter()
            .map(|(key, row)| (key.clone(), row.canonical.as_slice().to_vec()))
            .collect()
    };
    let metadata = |store: &ActionStateStore| -> Vec<(String, Vec<u8>)> {
        store
            .capacity
            .records
            .values()
            .map(|stored| (stored.path.as_str().to_owned(), stored.canonical.clone()))
            .collect()
    };
    if rows(live) != rows(cold)
        || metadata(live) != metadata(cold)
        || live.capacity_usage != cold.capacity_usage
    {
        return Err("a cold rebuild from the written documents differs from the live store".into());
    }
    snap.line(
        "cold rebuild",
        &format!(
            "equal: {} records, {} reservations",
            cold.records.len(),
            cold.capacity.records.len()
        ),
    );
    Ok(())
}

/// Every persisted document's namespace and key, in store order.
pub(in crate::action_state::terminal_capacity) fn disk_keys(
    snap: &mut Snapshot,
    disk: &BTreeMap<(SmolStr, SmolStr), Value>,
) {
    for (namespace, key) in disk.keys() {
        snap.line("disk key", &format!("{namespace} {key}"));
    }
}

/// One write of a validator-level transition: a put, or a delete when the value is `None`.
pub(in crate::action_state::terminal_capacity) type Write = (Key, Option<Value>);

/// A family's closed `validate_changes`.
pub(in crate::action_state::terminal_capacity) type Validate = fn(
    &ActionStateStore,
    &BTreeMap<StateKey, TxnMutation>,
    &Changes,
    Option<&RegisteredTxnIdentity>,
) -> Result<(), ActionStateError>;

/// The persisted envelope of one typed row, as the store writes it.
pub(in crate::action_state::terminal_capacity) fn envelope(
    registry: &StateFamilyRegistry,
    k: &Key,
    v: &Value,
) -> CtResult<(StateKey, Value)> {
    let k = k.resolve(registry)?;
    let d = registry
        .entries
        .get(&(k.owner.clone(), k.family.clone()))
        .ok_or("descriptor")?
        .descriptor();
    let e = json!({"family":{"owner":k.owner,"id":k.family},"key":k.encoded,"schema":{"id":d.value_schema(),"version":d.schema_version()},"value":v});
    Ok((k, e))
}

/// The committed state of a validator-level walk (the refit families' rigs drive the closed
/// `validate_changes` with inert typed rows, not a provider transaction): every typed row and
/// every retained reservation. An accepted write set is written to the snapshot and applied,
/// and the result is booted cold through the coordinator's own `hydrate` (every family's
/// audit, the capacity rebuild and the charged usage).
pub(in crate::action_state::terminal_capacity) struct Rig {
    pub(in crate::action_state::terminal_capacity) reg: StateFamilyRegistry,
    pub(in crate::action_state::terminal_capacity) rows: BTreeMap<StateKey, (Key, Value)>,
    pub(in crate::action_state::terminal_capacity) reservations:
        BTreeMap<ReservationId, Arc<StoredReservation>>,
    validate: Validate,
    family: &'static str,
}

impl Rig {
    pub(in crate::action_state::terminal_capacity) fn new(
        reg: StateFamilyRegistry,
        validate: Validate,
        family: &'static str,
        before: Vec<(Key, Value)>,
    ) -> CtResult<Self> {
        let mut rows = BTreeMap::new();
        for (k, v) in before {
            rows.insert(k.resolve(&reg)?, (k, v));
        }
        Ok(Self {
            reg,
            rows,
            reservations: BTreeMap::new(),
            validate,
            family,
        })
    }
    pub(in crate::action_state::terminal_capacity) fn store(&self) -> CtResult<ActionStateStore> {
        let mut store = ActionStateStore::new(self.reg.clone());
        for (k, v) in self.rows.values() {
            let (k, e) = envelope(&self.reg, k, v)?;
            store.records.insert(
                k,
                StoredStateRecord {
                    value: Box::new(v.clone()),
                    canonical: CanonicalStateBytes(
                        canonical_json_bytes_with_limit(&e, 262_144)?.into(),
                    ),
                },
            );
        }
        store.capacity = Arc::new(CapacityState {
            records: self.reservations.clone(),
            ..CapacityState::default()
        });
        Ok(store)
    }
    /// The current value of `k`, if the walk holds it.
    pub(in crate::action_state::terminal_capacity) fn get(&self, k: &Key) -> CtResult<Value> {
        Ok(self
            .rows
            .get(&k.resolve(&self.reg)?)
            .ok_or("no such row")?
            .1
            .clone())
    }
    fn mutations(&self, writes: &[Write]) -> CtResult<BTreeMap<StateKey, TxnMutation>> {
        let mut out = BTreeMap::new();
        for (k, v) in writes {
            let mutation = match v {
                Some(v) => {
                    let (_, document) = envelope(&self.reg, k, v)?;
                    TxnMutation::Put {
                        value: Box::new(v.clone()),
                        canonical: CanonicalStateBytes(
                            canonical_json_bytes_with_limit(&document, 262_144)?.into(),
                        ),
                        document,
                    }
                }
                None => TxnMutation::Delete,
            };
            out.insert(k.resolve(&self.reg)?, mutation);
        }
        Ok(out)
    }
    fn check(
        &self,
        writes: &[Write],
        changes: &Changes,
        identity: &RegisteredTxnIdentity,
    ) -> CtResult {
        (self.validate)(
            &self.store()?,
            &self.mutations(writes)?,
            changes,
            Some(identity),
        )?;
        Ok(())
    }
    /// Validates `writes`; on acceptance writes them and the capacity change, applies them,
    /// and boots the result cold.
    pub(in crate::action_state::terminal_capacity) fn accept(
        &mut self,
        snap: &mut Snapshot,
        title: &str,
        writes: Vec<Write>,
        changes: Changes,
        identity: &RegisteredTxnIdentity,
    ) -> CtResult {
        self.check(&writes, &changes, identity)?;
        snap.section(&format!("{}: {title}", self.family));
        snap.wire("action", &identity.action)?;
        snap.line("commit", &identity.commit.to_hex());
        snap.line("actor", &identity.actor.0.to_string());
        snap.wire("principal", &identity.principal)?;
        for (k, v) in &writes {
            match v {
                Some(v) => {
                    let (_, document) = envelope(&self.reg, k, v)?;
                    snap.bytes(
                        &format!("put {} {} {}", k.owner.as_str(), k.family, k.encoded),
                        &canonical_json_bytes_with_limit(&document, 262_144)?,
                    );
                }
                None => snap.line(
                    "delete",
                    &format!("{} {} {}", k.owner.as_str(), k.family, k.encoded),
                ),
            }
        }
        for (k, v) in writes {
            let resolved = k.resolve(&self.reg)?;
            match v {
                Some(v) => {
                    self.rows.insert(resolved, (k, v));
                }
                None => {
                    self.rows.remove(&resolved);
                }
            }
        }
        if let Some(record) = changes.funding {
            let stored = Arc::new(encode(record)?);
            snap.line("reservation path", stored.path.as_str());
            snap.wire("reservation record", &stored.record)?;
            snap.bytes("reservation canonical", &stored.canonical);
            self.reservations.insert(stored.id(), stored);
        }
        if let Some(id) = &changes.progress {
            snap.line("progress retains", &format!("{} {}", id.0.as_str(), id.1));
        }
        if let Some(id) = &changes.terminal {
            snap.line("terminal consumes", &format!("{} {}", id.0.as_str(), id.1));
            self.reservations.remove(id);
        }
        self.cold(snap)
    }
    /// The coordinator's cold boot of the written rows and reservations.
    pub(in crate::action_state::terminal_capacity) fn cold(&self, snap: &mut Snapshot) -> CtResult {
        let s = self.store()?;
        let typed: usize = s
            .records
            .values()
            .map(|r| r.canonical.as_slice().len())
            .sum();
        let metadata: usize = self.reservations.values().map(|r| r.len()).sum();
        let (state, usage) = crate::action_state::terminal_capacity::hydrate(
            &self.reg,
            &Verifiers::for_registry(&self.reg),
            self.reservations.clone(),
            &s.records,
            typed + metadata,
        )?;
        snap.line(
            "cold boot",
            &format!(
                "{} rows, {} reservations, {} metadata bytes, usage {} bytes {} records",
                s.records.len(),
                state.records.len(),
                state.metadata_bytes(),
                usage.bytes,
                usage.records
            ),
        );
        Ok(())
    }
    /// The refusal `validate_changes` answers to a forbidden write set.
    pub(in crate::action_state::terminal_capacity) fn refuse(
        &self,
        snap: &mut Snapshot,
        label: &str,
        writes: &[Write],
        changes: &Changes,
        identity: &RegisteredTxnIdentity,
    ) -> CtResult {
        refusal(snap, label, self.check(writes, changes, identity))
    }
}

/// The renewal twins' walk (doc 46 CT.3 will merge the two validators). Both test rigs name the
/// same private items (`fixture_config`, `draft`, `advance_with_service`, `pair`, the family
/// consts, the service rig's `service_actor` / `closure` / `close` / `actor`), so the walk is
/// written once and expanded inside each twin's `service_tests` module, where those resolve.
macro_rules! renewal_walk {
    (
        test: $test:ident,
        file: $file:literal,
        twin: $twin:literal,
        protocol: $protocol:ident,
        domains: $domains:ident,
        follow_seed: $follow:expr $(,)?
    ) => {
        use $crate::action_state::terminal_capacity::tests::ct0_kit as kit;
        use mush_testkit::snapshot::Snapshot;

        fn ct0_op(byte: u8) -> CommitToken {
            CommitToken::from_bytes([byte; 16])
        }

        /// The pair's identity, digests and derived outputs at one state.
        fn ct0_state(snap: &mut Snapshot, f: &Fixture, operation: CommitToken) -> TestResult {
            let p = pair(&view(f), operation)?;
            snap.line("phase", &p.phase);
            snap.line(
                "licence predecessor next",
                &format!("{} {} {}", p.licence, p.predecessor, p.next),
            );
            let binding = p.binding()?;
            let offer = field(&p.market, "offer")?;
            let basis = field(offer, "basis")?;
            snap.wire("offer digest", field(&p.market, "offer_digest")?)?;
            snap.wire(
                "predecessor term digest",
                field(binding, "predecessor_term")?,
            )?;
            snap.wire(
                "predecessor economics digest",
                field(binding, "predecessor_economics")?,
            )?;
            snap.wire("template digest", field(basis, "template_digest")?)?;
            snap.wire(
                "grant economics digest",
                field(basis, "grant_economics_digest")?,
            )?;
            if let Some(chain) = basis.get("royalty_chain_digest") {
                snap.wire("royalty chain digest", chain)?;
            }
            snap.hex("capacity fingerprint", fingerprint(&p)?.as_bytes());
            snap.line(
                "grant domain",
                $domains::grant_domain(field(binding, "grant")?)?,
            );
            let economics = required(
                &view(f),
                &term_key(ECONOMICS, p.licence, p.predecessor)?,
            )?;
            snap.line(
                "predecessor economics domain",
                $domains::economics_domain(&economics)?,
            );
            snap.wire(
                "binding revision",
                &entitlement::binding_revision(binding)?,
            )?;
            snap.wire("historical draft action", &historical_action(&p, 0)?)?;
            snap.wire("expected head", &expected_head(&p, operation)?)?;
            snap.line(
                "target count",
                &targets(operation, &p)?.len().to_string(),
            );
            snap.wire(
                "receipt",
                &value(&view(f), &operation_key(RECEIPT, operation)?)?,
            )?;
            snap.wire(
                "licence",
                &required(&view(f), &licence_key(LICENCE, p.licence)?)?,
            )?;
            snap.wire(
                "next term",
                &value(&view(f), &term_key(TERM, p.licence, p.next)?)?,
            )?;
            snap.wire(
                "next economics",
                &value(&view(f), &term_key(ECONOMICS, p.licence, p.next)?)?,
            )?;
            Ok(())
        }

        /// One accepted transition: its writes, the index after the install, the pair, and a
        /// cold rebuild from the written documents.
        fn ct0_step(
            snap: &mut Snapshot,
            f: &mut Fixture,
            title: &str,
            operation: CommitToken,
            prepared: PreparedTxn,
        ) -> TestResult {
            snap.section(&format!("{} renewal: {title}", $twin));
            kit::prepared(snap, &prepared)?;
            install(f, prepared)?;
            kit::reservations(snap, &f.store)?;
            ct0_state(snap, f, operation)?;
            let cold = hydrate(f, &f.disk)?;
            kit::same_cold(snap, &f.store, &cold)
        }

        fn ct0_identity(snap: &mut Snapshot) -> TestResult {
            snap.section(&format!("{} renewal: identity", $twin));
            for (index, id) in ACTIONS.iter().enumerate() {
                snap.line(&format!("action id {index}"), id);
            }
            snap.line("service action id", SERVICE_ACTION);
            for (index, transition) in TRANSITIONS.iter().enumerate() {
                snap.line(&format!("transition {index}"), transition);
            }
            for (owner, family, ceiling) in FAMILIES {
                snap.line("family", &format!("{owner} {family} {ceiling}"));
            }
            for index in 0..6 {
                snap.wire(&format!("action payload {index}"), &action_payload(index))?;
            }
            let plain = fixture()?;
            snap.wire("manifest", &plain.manifest)?;
            snap.wire("participants", &plain.participants)?;
            for index in 0..6 {
                snap.wire(
                    &format!("action reference {index}"),
                    &action_reference(&plain.manifest, &plain.participants, index)?,
                )?;
            }
            let protocol = plain
                .store
                .registry
                .$protocol
                .as_ref()
                .ok_or("protocol")?;
            snap.wire("protocol", protocol)?;
            protocol.validate_families(&plain.store.registry)?;
            snap.line("families", "valid");
            let market = OwnerId::new("market")?;
            for (owner, family, _) in FAMILIES {
                let owner = OwnerId::new(owner)?;
                let id = StateFamilyId::new(family)?;
                let entry = plain
                    .store
                    .registry
                    .entries
                    .get(&(owner.clone(), id.clone()))
                    .ok_or("family entry")?;
                snap.wire(
                    &format!("descriptor {family}"),
                    &entry.descriptor().canonical_value(),
                )?;
                snap.line(
                    &format!("delegated to market {family}"),
                    &plain
                        .store
                        .registry
                        .delegates_family(&owner, &id, &market)
                        .to_string(),
                );
            }
            let service = fixture_config(1, true, true)?;
            snap.wire(
                "service protocol",
                service
                    .store
                    .registry
                    .$protocol
                    .as_ref()
                    .ok_or("service protocol")?,
            )?;
            snap.wire("service action", &service.service)?;
            kit::refusal(
                snap,
                "declaration by another owner",
                Protocol::new(
                    &OwnerId::new("roll")?,
                    plain.manifest.clone(),
                    plain.participants.clone(),
                    plain.actions.clone(),
                )
                .map_err(Into::into),
            )?;
            let mut swapped = plain.actions.clone();
            swapped.swap(0, 1);
            kit::refusal(
                snap,
                "declaration with reordered actions",
                Protocol::new(
                    &market,
                    plain.manifest.clone(),
                    plain.participants.clone(),
                    swapped,
                )
                .map_err(Into::into),
            )?;
            let [first, second] = plain.participants.clone();
            kit::refusal(
                snap,
                "declaration with swapped participants",
                Protocol::new(
                    &market,
                    plain.manifest.clone(),
                    [second, first],
                    plain.actions.clone(),
                )
                .map_err(Into::into),
            )?;
            kit::refusal(
                snap,
                "declaration with a foreign service action",
                Protocol::with_service(
                    &market,
                    plain.manifest.clone(),
                    plain.participants.clone(),
                    plain.actions.clone(),
                    plain.actions[4].clone(),
                )
                .map_err(Into::into),
            )
        }

        fn ct0_grant(snap: &mut Snapshot) -> TestResult {
            let mut f = fixture()?;
            let op = ct0_op(31);
            let prepared = draft(&mut f, 31, None)?;
            ct0_step(snap, &mut f, "grant walk: drafted (automatic)", op, prepared)?;
            kit::refusal(
                snap,
                "second automatic draft while pending",
                draft(&mut f, 34, None),
            )?;
            let prepared = advance(&mut f, op, 1, 32)?;
            ct0_step(snap, &mut f, "grant walk: funded", op, prepared)?;
            kit::refusal(snap, "fund twice", advance(&mut f, op, 1, 40))?;
            kit::refusal(
                snap,
                "unfunded close while funded",
                advance(&mut f, op, 4, 40),
            )?;
            let prepared = advance(&mut f, op, 2, 33)?;
            ct0_step(snap, &mut f, "grant walk: granted", op, prepared)?;
            kit::refusal(snap, "grant twice", advance(&mut f, op, 2, 34))?;
            kit::refusal(snap, "refund after grant", advance(&mut f, op, 3, 34))?;
            let next = ct0_op(37);
            let prepared = draft(&mut f, 37, None)?;
            ct0_step(
                snap,
                &mut f,
                "grant walk: next term drafted (automatic)",
                next,
                prepared,
            )?;
            kit::disk_keys(snap, &f.disk);
            Ok(())
        }

        fn ct0_refund(snap: &mut Snapshot) -> TestResult {
            let mut f = fixture()?;
            let op = ct0_op(31);
            let prepared = draft(&mut f, 31, None)?;
            ct0_step(snap, &mut f, "refund walk: drafted", op, prepared)?;
            let prepared = advance(&mut f, op, 1, 32)?;
            ct0_step(snap, &mut f, "refund walk: funded", op, prepared)?;
            let prepared = advance(&mut f, op, 3, 33)?;
            ct0_step(snap, &mut f, "refund walk: refunded", op, prepared)?;
            kit::refusal(snap, "grant after refund", advance(&mut f, op, 2, 34))?;
            kit::refusal(
                snap,
                "automatic draft after a refusal",
                draft(&mut f, 34, None),
            )?;
            kit::refusal(
                snap,
                "retry prepared before the refund",
                draft(&mut f, 20, Some(op)),
            )?;
            let retry = ct0_op(34);
            let prepared = draft(&mut f, 34, Some(op))?;
            ct0_step(snap, &mut f, "refund walk: retry drafted", retry, prepared)?;
            let prepared = advance(&mut f, retry, 1, 35)?;
            ct0_step(snap, &mut f, "refund walk: retry funded", retry, prepared)?;
            let prepared = advance(&mut f, retry, 2, 36)?;
            ct0_step(snap, &mut f, "refund walk: retry granted", retry, prepared)?;
            snap.section(&format!(
                "{} renewal: refund walk: the refunded attempt after the retry grant",
                $twin
            ));
            ct0_state(snap, &f, op)?;
            kit::disk_keys(snap, &f.disk);
            Ok(())
        }

        fn ct0_unfunded(snap: &mut Snapshot, reason: &str) -> TestResult {
            let mut f = fixture()?;
            let op = ct0_op(31);
            let prepared = draft(&mut f, 31, None)?;
            ct0_step(
                snap,
                &mut f,
                &format!("unfunded {reason} walk: drafted"),
                op,
                prepared,
            )?;
            let action = f.actions[4].clone();
            let closure = |reason: &str| {
                json!({"kind":"unfunded_closed","closure":{"commit":ct0_op(32),
                    "action":action,
                    "at_ic":{"clock":{"kind":"world_ic","generation":1},"at":1032},
                    "posted_at_utc":1032,"reason":reason}})
            };
            kit::refusal(
                snap,
                "unfunded close with an unknown reason",
                advance_with_service(&mut f, op, 4, 32, Some(closure("lapsed"))),
            )?;
            let prepared = advance_with_service(&mut f, op, 4, 32, Some(closure(reason)))?;
            ct0_step(
                snap,
                &mut f,
                &format!("unfunded {reason} walk: closed"),
                op,
                prepared,
            )?;
            kit::refusal(snap, "fund after close", advance(&mut f, op, 1, 33))?;
            let retry = ct0_op(33);
            let prepared = draft(&mut f, 33, Some(op))?;
            ct0_step(
                snap,
                &mut f,
                &format!("unfunded {reason} walk: retry drafted"),
                retry,
                prepared,
            )?;
            let prepared = advance(&mut f, retry, 4, 34)?;
            ct0_step(
                snap,
                &mut f,
                &format!("unfunded {reason} walk: retry closed (declined)"),
                retry,
                prepared,
            )?;
            kit::disk_keys(snap, &f.disk);
            Ok(())
        }

        fn ct0_service(snap: &mut Snapshot, reason: &str) -> TestResult {
            let mut f = fixture_config(1, true, true)?;
            let op = ct0_op(31);
            let prepared = draft(&mut f, 31, None)?;
            ct0_step(
                snap,
                &mut f,
                &format!("service {reason} walk: drafted"),
                op,
                prepared,
            )?;
            let state = closure(&f, reason)?;
            kit::refusal(
                snap,
                "service close by the holder",
                close(&mut f, state.clone()),
            )?;
            service_actor(&mut f)?;
            kit::refusal(
                snap,
                "unfunded close by the service",
                advance(&mut f, op, 4, 32),
            )?;
            let mut wrong = state.clone();
            wrong["closure"]["reason"] = json!("expired");
            kit::refusal(snap, "service close for a holder reason", close(&mut f, wrong))?;
            let prepared = close(&mut f, state.clone())?;
            ct0_step(
                snap,
                &mut f,
                &format!("service {reason} walk: service closed"),
                op,
                prepared,
            )?;
            kit::refusal(snap, "service close twice", close(&mut f, state))?;
            actor(&mut f, ObjId(7), PrincipalId::Object { id: ObjId(7) })?;
            let retry = ct0_op(33);
            let prepared = draft(&mut f, 33, Some(op))?;
            ct0_step(
                snap,
                &mut f,
                &format!("service {reason} walk: retry drafted"),
                retry,
                prepared,
            )?;
            kit::disk_keys(snap, &f.disk);
            Ok(())
        }

        fn ct0_service_funded(snap: &mut Snapshot) -> TestResult {
            let mut f = fixture_config(1, true, true)?;
            let op = ct0_op(31);
            let prepared = draft(&mut f, 31, None)?;
            ct0_step(snap, &mut f, "service funded walk: drafted", op, prepared)?;
            let prepared = advance(&mut f, op, 1, 32)?;
            ct0_step(snap, &mut f, "service funded walk: funded", op, prepared)?;
            service_actor(&mut f)?;
            let state = closure(&f, "clock_invalidated")?;
            kit::refusal(snap, "service close of funded work", close(&mut f, state))?;
            kit::refusal(snap, "grant by the service", advance(&mut f, op, 2, 33))
        }

        fn ct0_manifest_changed(snap: &mut Snapshot) -> TestResult {
            let mut old = fixture()?;
            let op = ct0_op(31);
            let prepared = draft(&mut old, 31, None)?;
            ct0_step(snap, &mut old, "manifest changed walk: drafted", op, prepared)?;
            let prepared = advance(&mut old, op, 1, 32)?;
            ct0_step(snap, &mut old, "manifest changed walk: funded", op, prepared)?;
            let mut current = fixture_with_manifest(2)?;
            current.store = hydrate(&current, &old.disk)?;
            current.disk = old.disk.clone();
            snap.wire("current manifest", &current.manifest)?;
            kit::refusal(
                snap,
                "grant under a changed manifest",
                advance(&mut current, op, 2, 33),
            )?;
            let prepared = advance(&mut current, op, 3, 34)?;
            ct0_step(
                snap,
                &mut current,
                "manifest changed walk: refunded under the current manifest",
                op,
                prepared,
            )
        }

        fn ct0_follow(snap: &mut Snapshot) -> TestResult {
            let mut f = fixture_from_seed(1, false, false, $follow?)?;
            let op = ct0_op(31);
            let prepared = draft(&mut f, 31, None)?;
            ct0_step(snap, &mut f, "follow walk: drafted", op, prepared)?;
            let prepared = advance(&mut f, op, 1, 32)?;
            ct0_step(snap, &mut f, "follow walk: funded", op, prepared)?;
            let prepared = advance(&mut f, op, 2, 33)?;
            ct0_step(snap, &mut f, "follow walk: granted", op, prepared)?;
            kit::disk_keys(snap, &f.disk);
            Ok(())
        }

        #[test]
        fn $test() -> TestResult {
            let mut snap = Snapshot::new();
            ct0_identity(&mut snap)?;
            ct0_grant(&mut snap)?;
            ct0_refund(&mut snap)?;
            ct0_unfunded(&mut snap, "declined")?;
            ct0_unfunded(&mut snap, "expired")?;
            ct0_service(&mut snap, "deadline_reached")?;
            ct0_service(&mut snap, "clock_invalidated")?;
            ct0_service_funded(&mut snap)?;
            ct0_manifest_changed(&mut snap)?;
            ct0_follow(&mut snap)?;
            snap.check(env!("CARGO_MANIFEST_DIR"), $file)?;
            Ok(())
        }
    };
}
pub(in crate::action_state::terminal_capacity) use renewal_walk;

/// The purchase twins' walk (personal entitlement and House). Both rigs name the same private
/// items (`fund`, `finish`, `install`, `reboot`, `Mode`, `FundFault`, `FinishFault`, `pair`,
/// `fingerprint`, `targets`, the family consts), so the walk is written once and expanded inside
/// each twin's test module. Each mode is walked to Grant and, from a fresh fixture, to Refund.
macro_rules! purchase_walk {
    (
        test: $test:ident,
        file: $file:literal,
        twin: $twin:literal,
        protocol: $protocol:ident,
        fixture: $fixture:expr,
        declare: $declare:expr,
        modes: [$($mode:ident),+ $(,)?],
        faults: [$(($fault:ident, $grant:literal)),+ $(,)?] $(,)?
    ) => {
        use $crate::action_state::terminal_capacity::tests::ct0_kit as kit;
        use mush_testkit::snapshot::Snapshot;

        fn ct0_view(f: &Fixture) -> View<'_> {
            View {
                registry: &f.store.registry,
                records: &f.store.records,
                mutations: None,
            }
        }

        /// The pair's identity and digests at one state.
        fn ct0_state(snap: &mut Snapshot, f: &Fixture, mode: Mode) -> TestResult {
            let id = mode.operation();
            let p = pair(&ct0_view(f), id)?;
            snap.line("phase", &p.phase);
            snap.line("licence", &p.licence.to_string());
            let quote = field(&p.purchase, "quote")?;
            snap.wire("quote digest", field(&p.purchase, "quote_digest")?)?;
            snap.wire("product digest", field(quote, "product_digest")?)?;
            if let Some(chain) = field(field(quote, "product")?, "grant")?.get("royalty_chain") {
                snap.wire("royalty chain", chain)?;
            }
            snap.hex("capacity fingerprint", fingerprint(&p)?.as_bytes());
            snap.line(
                "grant domain",
                grant_domain(field(field(quote, "product")?, "grant")?)?,
            );
            snap.wire("effect", field(&p.purchase, "effect")?)?;
            snap.line(
                "target count",
                &targets(id, p.licence)?.len().to_string(),
            );
            let receipt = value(&ct0_view(f), &operation_key(RECEIPT, id)?)?;
            snap.wire("receipt", &receipt)?;
            let economics = value(
                &ct0_view(f),
                &key(
                    "roll",
                    ECONOMICS,
                    format!("licence/{:016x}/term/0000000000000001", p.licence),
                )?,
            )?;
            if let Some(economics) = &economics {
                snap.line("economics domain", economics_domain(economics)?);
            }
            Ok(())
        }

        /// One accepted transition: its writes, the index after the install, the pair, and a
        /// cold rebuild from the written documents.
        fn ct0_step(
            snap: &mut Snapshot,
            f: &mut Fixture,
            title: &str,
            mode: Mode,
            prepared: PreparedTxn,
        ) -> TestResult {
            snap.section(&format!("{} purchase: {mode:?}: {title}", $twin));
            kit::prepared(snap, &prepared)?;
            install(f, prepared)?;
            kit::reservations(snap, &f.store)?;
            ct0_state(snap, f, mode)?;
            let cold = reboot(f)?;
            kit::same_cold(snap, &f.store, &cold)
        }

        fn ct0_identity(snap: &mut Snapshot) -> TestResult {
            snap.section(&format!("{} purchase: identity", $twin));
            for (index, transition) in TRANSITIONS.iter().enumerate() {
                snap.line(&format!("transition {index}"), transition);
            }
            for (owner, family, ceiling) in FAMILIES {
                snap.line("family", &format!("{owner} {family} {ceiling}"));
            }
            let f = $fixture?;
            let protocol = f
                .store
                .registry
                .$protocol
                .as_ref()
                .ok_or("protocol")?;
            snap.wire("protocol", protocol)?;
            for action in protocol.admission_actions() {
                snap.wire("admission action", action)?;
            }
            protocol.validate_families(&f.store.registry)?;
            snap.line("families", "valid");
            let market = OwnerId::new("market")?;
            for (owner, family, _) in FAMILIES {
                let owner = OwnerId::new(owner)?;
                let id = StateFamilyId::new(family)?;
                let entry = f
                    .store
                    .registry
                    .entries
                    .get(&(owner.clone(), id.clone()))
                    .ok_or("family entry")?;
                snap.wire(
                    &format!("descriptor {family}"),
                    &entry.descriptor().canonical_value(),
                )?;
                snap.line(
                    &format!("delegated to market {family}"),
                    &f.store
                        .registry
                        .delegates_family(&owner, &id, &market)
                        .to_string(),
                );
            }
            let declare = $declare;
            declare(&market, &**protocol)?;
            kit::refusal(
                snap,
                "declaration by another owner",
                declare(&OwnerId::new("roll")?, &**protocol).map_err(Into::into),
            )?;
            for mode in [$(Mode::$mode),+] {
                let kind = mode.kind().ok_or("kind")?;
                snap.line(&format!("{mode:?} wire"), kind.wire());
                snap.line(&format!("{mode:?} protocol"), kind.protocol());
                snap.line(&format!("{mode:?} version"), &kind.version().to_string());
                snap.line(&format!("{mode:?} product domain"), kind.product_domain());
                snap.line(&format!("{mode:?} quote domain"), kind.quote_domain());
                snap.line(&format!("{mode:?} follow"), &kind.follow().to_string());
                for (index, id) in kind.actions().iter().enumerate() {
                    snap.line(&format!("{mode:?} action id {index}"), id);
                    snap.wire(
                        &format!("{mode:?} action payload {index}"),
                        &action_payload(kind, TRANSITIONS[index]),
                    )?;
                    snap.wire(
                        &format!("{mode:?} registered action {index}"),
                        f.action(mode, index)?,
                    )?;
                }
            }
            Ok(())
        }

        fn ct0_grant(snap: &mut Snapshot, mode: Mode) -> TestResult {
            let mut f = $fixture?;
            for (label, fault) in [
                (
                    "fund that keeps the quote",
                    FundFault {
                        keep_quote: true,
                        ..FundFault::default()
                    },
                ),
                (
                    "fund that skips a licence",
                    FundFault {
                        wrong_allocator: true,
                        ..FundFault::default()
                    },
                ),
                (
                    "fund without the reservation",
                    FundFault {
                        no_capacity: true,
                        ..FundFault::default()
                    },
                ),
                (
                    "fund as an ordinary write",
                    FundFault {
                        ordinary: true,
                        ..FundFault::default()
                    },
                ),
            ] {
                kit::refusal(snap, label, fund(&mut f, mode, fault))?;
            }
            let prepared = fund(&mut f, mode, FundFault::default())?;
            ct0_step(snap, &mut f, "funded", mode, prepared)?;
            $(
                kit::refusal(
                    snap,
                    concat!("finish with fault ", stringify!($fault)),
                    finish(&mut f, mode, $grant, FinishFault::$fault),
                )?;
            )+
            let prepared = finish(&mut f, mode, true, FinishFault::None)?;
            ct0_step(snap, &mut f, "granted", mode, prepared)?;
            kit::disk_keys(snap, &f.disk);
            Ok(())
        }

        fn ct0_refund(snap: &mut Snapshot, mode: Mode) -> TestResult {
            let mut f = $fixture?;
            let prepared = fund(&mut f, mode, FundFault::default())?;
            ct0_step(snap, &mut f, "funded (refund walk)", mode, prepared)?;
            let prepared = finish(&mut f, mode, false, FinishFault::None)?;
            ct0_step(snap, &mut f, "refunded", mode, prepared)?;
            kit::disk_keys(snap, &f.disk);
            Ok(())
        }

        #[test]
        fn $test() -> TestResult {
            let mut snap = Snapshot::new();
            ct0_identity(&mut snap)?;
            for mode in [$(Mode::$mode),+] {
                ct0_grant(&mut snap, mode)?;
                ct0_refund(&mut snap, mode)?;
            }
            snap.check(env!("CARGO_MANIFEST_DIR"), $file)?;
            Ok(())
        }
    };
}
pub(in crate::action_state::terminal_capacity) use purchase_walk;
