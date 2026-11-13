#![cfg(test)]

//! Doc 46 CT.0: the House renewal twin's committed snapshot, and the walk both
//! renewal twins share.
//!
//! `renewal_walk!` is instantiated once in each twin's `tests` module (this one
//! and `mind_renewal::tests::ct0_snapshot`), so every name it uses that is not
//! spelled with a path (`fixture`, `funded`, `terminal`, `closed`, `token`,
//! `stamp`, `MindRenewalOperationV1`, `RenewalStateV1`, `Transition`, ...)
//! resolves to that twin's own test helpers and types. The walk drives the
//! real code: the rows come from the twin's own fixture helpers (validated by
//! the twin's `validate` and `expected_action`), the receipt from
//! `publication::receipt`, the families from `families::family` registered
//! in a real engine, and each transition's documents from one real typed
//! transaction over those families (the writes the provider's arms make:
//! Draft inserts the operation, Fund replaces it, a terminal replaces it and
//! inserts the receipt). It does not run the providers themselves (they need
//! the composed Economy and Roll owners).
use super::*;

macro_rules! renewal_walk {
    (
        test: $test:ident,
        twin: $twin:literal,
        file: $file:literal,
        configs: $configs:path $(,)?
    ) => {
        use crate::mind_renewal::player::{
            FrontAnswer, FrontStep, RenewalPlayerOutcome, front_argument, front_line,
            front_request,
        };
        use mush_core::authoritative::ReplayWrite;
        use mush_core::mechanics::{DefinitionDescriptor, DefinitionRegistryBuilder, OwnerId};
        use mush_core::{Engine, GameModule, ModError, ModuleWrite, Registrar, Registry, World};
        use mush_testkit::snapshot::Snapshot;
        use std::collections::BTreeMap;
        use std::sync::{Arc, Mutex};

        type Ct0Families = super::super::Families;
        type Ct0Disk = BTreeMap<String, Value>;
        type Ct0Config = super::super::RenewalConfig;
        /// The current term's end the status lines are read against.
        const CT0_TERM_END: i64 = 130;

        struct Ct0Module(Arc<Mutex<Option<Ct0Families>>>);
        impl GameModule for Ct0Module {
            fn name(&self) -> &'static str {
                "market"
            }
            fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
                let families = Ct0Families {
                    operations: super::super::families::family(reg)?,
                    receipts: super::super::families::family(reg)?,
                };
                *self
                    .0
                    .lock()
                    .map_err(|_| ModError::msg("ct0 family lock"))? = Some(families);
                Ok(())
            }
        }

        #[inline(never)]
        fn ct0_rig() -> TestResult<(Box<Engine>, Ct0Families)> {
            let slot = Arc::new(Mutex::new(None));
            let engine = Engine::with_modules(
                World::new_minimal(),
                Registry::new(),
                vec![Box::new(Ct0Module(slot.clone()))],
            )?;
            let families = slot
                .lock()
                .map_err(|_| "ct0 family lock")?
                .take()
                .ok_or("ct0 families")?;
            Ok((Box::new(engine), families))
        }

        /// One transition's documents in one real typed transaction: the
        /// operation row inserted (Draft) or replaced, and the receipt
        /// inserted once the row is terminal. Returns the replay writes and
        /// folds the store writes into `disk`.
        fn ct0_step(
            engine: &mut Engine,
            families: &Ct0Families,
            disk: &mut Ct0Disk,
            row: &MindRenewalOperationV1,
            insert: bool,
        ) -> TestResult<Vec<ReplayWrite>> {
            let mut txn = engine.action_transaction(&families.operations)?;
            if insert {
                txn.insert(&families.operations, &row.operation(), row.clone())?;
            } else {
                txn.replace(&families.operations, &row.operation(), row.clone())?;
            }
            if !row.state().pending() {
                txn.insert(&families.receipts, &row.operation(), row.receipt()?)?;
            }
            let prepared = txn.prepare()?;
            let writes = prepared.authoritative_replay_writes();
            let (ops, install) = prepared.into_parts();
            engine.install_prepared_action(install)?;
            for op in ops {
                match op.op {
                    ModuleWrite::JsonSet { key, value } => {
                        disk.insert(key.to_string(), value);
                    }
                    ModuleWrite::Del { key } => {
                        disk.remove(key.as_str());
                    }
                    _ => return Err("a typed renewal step wrote a non-document".into()),
                }
            }
            Ok(writes)
        }

        /// The written documents hold the live rows, and the kernel's boot
        /// audit recognises them as renewal records: a cold rebuild without
        /// the registered renewal protocol (which needs the composed Roll and
        /// purchase audits) is refused, so the rig cannot boot them itself.
        fn ct0_cold(
            engine: &Engine,
            families: &Ct0Families,
            disk: &Ct0Disk,
            row: &MindRenewalOperationV1,
        ) -> TestResult {
            check_eq!(
                engine
                    .action_state()
                    .get(&families.operations, &row.operation())?,
                Some(row)
            );
            let receipt = engine
                .action_state()
                .get(&families.receipts, &row.operation())?
                .cloned();
            let documents: Vec<&Value> = disk.values().collect();
            if row.state().pending() {
                check!(receipt.is_none());
                check_eq!(documents.len(), 1);
            } else {
                check_eq!(receipt, Some(row.receipt()?));
                check_eq!(documents.len(), 2);
            }
            let mut found = 0;
            for document in documents {
                let value = serde_json::to_vec(document.get("value").ok_or("value")?)?;
                if let Ok(decoded) = MindRenewalOperationV1::decode(&value) {
                    check_eq!(&decoded, row);
                    found += 1;
                } else {
                    check_eq!(MindRenewalReceiptV1::decode(&value)?, row.receipt()?);
                    found += 1;
                }
            }
            check!(found > 0);
            let (mut cold, _) = ct0_rig()?;
            check!(
                cold.hydrate_action_state(disk.iter().map(|(key, value)| {
                    mush_core::ActionStateBootRecord::new(crate::NS, key.clone(), value.clone())
                }))
                .is_err(),
                "renewal documents boot only under the kernel's renewal protocol"
            );
            Ok(())
        }

        /// The stored document's value, as the bytes a decode reads.
        fn ct0_value(document: &[u8]) -> TestResult<Vec<u8>> {
            let document: Value = serde_json::from_slice(document)?;
            Ok(serde_json::to_vec(
                document.get("value").ok_or("stored value")?,
            )?)
        }

        fn ct0_front(answer: FrontAnswer) -> String {
            match answer {
                FrontAnswer::Say(lines) => format!("say: {}", lines.join(" || ")),
                FrontAnswer::Refuse(line) => format!("refuse: {line}"),
                FrontAnswer::Ask => "ask".to_owned(),
            }
        }

        fn ct0_status(row: Option<&MindRenewalOperationV1>) -> String {
            super::super::service::command_line(RenewalPlayerOutcome::Status(
                super::super::service::status_view(CT0_TERM_END, row),
            ))
        }

        fn ct0_register(descriptor: DefinitionDescriptor) -> TestResult<mush_core::DefinitionRef> {
            Ok(DefinitionRegistryBuilder::new().register(OwnerId::new("market")?, descriptor)?)
        }

        /// One state: the row in field order, its keys, digests and Roll
        /// projection, the transition's replay writes, the stored bytes, the
        /// receipt once terminal, and every sentence the twin prints for it.
        fn ct0_state(
            snap: &mut Snapshot,
            engine: &Engine,
            families: &Ct0Families,
            title: &str,
            row: &MindRenewalOperationV1,
            writes: &[ReplayWrite],
        ) -> TestResult {
            snap.section(&format!("{} renewal: {title}", $twin));
            let state = serde_json::to_value(row.state())?;
            snap.line(
                "state kind",
                state.get("kind").and_then(Value::as_str).ok_or("state kind")?,
            );
            snap.line("operation key", &row.key());
            snap.wire("operation", row)?;
            let roll = row.roll_projection()?;
            row.matches_roll(&roll)?;
            snap.wire("roll projection", &roll)?;
            snap.line("offer digest", &row.offer_digest().to_hex());
            check_eq!(row.offer().digest()?, row.offer_digest());
            snap.line(
                "binding template digest",
                &row.binding().template_digest()?.to_hex(),
            );
            snap.line(
                "binding grant economics digest",
                &row.binding().grant().digest()?.to_hex(),
            );
            snap.hex("consent hash", row.consent_hash());
            snap.replay_writes(writes)?;
            let stored = engine
                .action_state()
                .canonical(&families.operations, &row.operation())?
                .ok_or("stored operation")?;
            snap.bytes("stored operation", stored.as_slice());
            check_eq!(
                &MindRenewalOperationV1::decode(&ct0_value(stored.as_slice())?)?,
                row
            );
            check_eq!(
                &MindRenewalOperationV1::decode(&serde_json::to_vec(row)?)?,
                row
            );
            if row.state().pending() {
                check!(row.receipt().is_err(), "a pending row has no receipt");
            } else {
                let receipt = row.receipt()?;
                receipt.matches_operation(row)?;
                snap.line("receipt key", &receipt.key());
                snap.wire("receipt", &receipt)?;
                let stored = engine
                    .action_state()
                    .canonical(&families.receipts, &row.operation())?
                    .ok_or("stored receipt")?;
                snap.bytes("stored receipt", stored.as_slice());
                check_eq!(
                    MindRenewalReceiptV1::decode(&ct0_value(stored.as_slice())?)?,
                    receipt
                );
            }
            snap.line("status line", &ct0_status(Some(row)));
            let view = super::super::service::status_view(CT0_TERM_END, Some(row));
            let shown = row.binding().licence().get().to_string();
            for now in [100, CT0_TERM_END, 150] {
                snap.line(
                    &format!("front request at world-IC {now}"),
                    &ct0_front(front_request(&shown, &view, now)),
                );
            }
            Ok(())
        }

        /// The drafted row closed by the renewal service (the sixth action):
        /// the deadline reached, or the clock invalidated.
        fn ct0_service_closed(
            row: &MindRenewalOperationV1,
            service: &mush_core::DefinitionRef,
            invalidated: bool,
        ) -> TestResult<MindRenewalOperationV1> {
            let at = ClockStamp {
                clock: ClockRef::WorldIc {
                    generation: if invalidated { 2 } else { 1 },
                },
                at: if invalidated { 100 } else { 200 },
            };
            let mut value = serde_json::to_value(row)?;
            value
                .as_object_mut()
                .ok_or("operation object")?
                .insert(
                    "state".into(),
                    json!({"kind":"unfunded_service_closed","closure":{
                        "format":1,"commit":token(13),"action":service,"actor":mush_core::GOD,
                        "at_ic":at,"posted_at_utc":1030,
                        "reason":if invalidated {"clock_invalidated"} else {"deadline_reached"}
                    }}),
                );
            let closed: MindRenewalOperationV1 = serde_json::from_value(value)?;
            closed.validate()?;
            Ok(closed)
        }

        /// A host policy with every optional cut present.
        fn ct0_policy() -> super::super::RenewalPolicy {
            let cut = |permille, account| GrossRecipientCut {
                permille,
                recipient: Some(Recipient {
                    account: AccountId(account),
                    owner: mush_org::PrincipalId::Object(ObjId(6)),
                }),
            };
            super::super::RenewalPolicy {
                registry: cut(50, 14),
                venue: Some(cut(25, 15)),
                offer_ic_seconds: 100,
            }
        }

        /// What the configuration declares and registers: the five actions
        /// and the service's sixth, every declared descriptor, the host policy
        /// and the outcome, and each action's projected parameters.
        fn ct0_config(snap: &mut Snapshot, label: &str, config: &Ct0Config) -> TestResult {
            snap.section(&format!("{} renewal configuration: {label}", $twin));
            for (kind, action) in Transition::ALL.into_iter().zip(config.actions()?.iter()) {
                snap.wire(&format!("action {}", kind.id()), action)?;
            }
            let policy = config.policy_descriptor()?;
            snap.wire("policy descriptor payload", policy.payload())?;
            snap.wire("policy ref", &config.policy_ref()?)?;
            check_eq!(ct0_register(policy)?, config.policy_ref()?);
            let outcome = config.outcome_descriptor()?;
            snap.wire("outcome payload", outcome.payload())?;
            snap.wire("outcome ref", &ct0_register(outcome)?)?;
            check!(config.service_action()?.is_none());
            let service = config.clone().with_service()?;
            snap.wire(
                "service action",
                &service.service_action()?.ok_or("service action")?,
            )?;
            for descriptor in service.declarations()? {
                snap.wire("declared payload", descriptor.payload())?;
                snap.wire("declared ref", &ct0_register(descriptor)?)?;
            }
            check_eq!(
                service.declarations()?.len(),
                config.declarations()?.len() + 1
            );
            Ok(())
        }

        #[inline(never)]
        fn ct0_identity(snap: &mut Snapshot, drafted: &MindRenewalOperationV1) -> TestResult {
            snap.section(&format!("{} renewal: identity", $twin));
            snap.line("operation family", MindRenewalOperationV1::FAMILY);
            snap.line(
                "operation max bytes",
                &MindRenewalOperationV1::MAX_BYTES.to_string(),
            );
            snap.line("receipt family", MindRenewalReceiptV1::FAMILY);
            snap.line(
                "receipt max bytes",
                &MindRenewalReceiptV1::MAX_BYTES.to_string(),
            );
            let (_engine, families) = ct0_rig()?;
            for (name, owner, d) in [
                (
                    "operation",
                    families.operations.owner(),
                    families.operations.descriptor(),
                ),
                (
                    "receipt",
                    families.receipts.owner(),
                    families.receipts.descriptor(),
                ),
            ] {
                snap.line(&format!("{name} family owner"), owner.as_str());
                snap.line(&format!("{name} family id"), d.id().as_str());
                snap.line(&format!("{name} key codec"), d.key_codec().as_str());
                snap.line(&format!("{name} value schema"), d.value_schema().as_str());
                snap.line(
                    &format!("{name} schema version"),
                    &d.schema_version().to_string(),
                );
                snap.line(&format!("{name} decoder"), d.decoder().as_str());
                snap.line(&format!("{name} validator"), d.validator().as_str());
                snap.line(&format!("{name} serializer"), d.serializer().as_str());
                snap.line(
                    &format!("{name} max key bytes"),
                    &d.max_key_bytes().to_string(),
                );
                snap.line(
                    &format!("{name} max record bytes"),
                    &d.max_record_bytes().to_string(),
                );
            }
            super::super::capacity::check(&families, drafted)?;
            for kind in Transition::ALL {
                snap.line("transition", kind.id());
                snap.wire(
                    &format!("expected action {}", kind.id()),
                    &drafted.expected_action(kind)?,
                )?;
                snap.wire(
                    &format!("parameters {}", kind.id()),
                    &super::super::actions::parameters(kind)?,
                )?;
            }
            Ok(())
        }

        /// The sentences that do not depend on a row's state.
        fn ct0_words(snap: &mut Snapshot, drafted: &MindRenewalOperationV1) -> TestResult {
            use mush_roll::commerce::renewal::reader::RenewalStop;
            snap.section(&format!("{} renewal: player words", $twin));
            super::super::service::ct0_service::ct0_lines(snap)?;
            snap.line("status, no operation", &ct0_status(None));
            let view = super::super::service::status_view(CT0_TERM_END, None);
            for now in [100, CT0_TERM_END] {
                snap.line(
                    &format!("front request, no operation, at world-IC {now}"),
                    &ct0_front(front_request("7", &view, now)),
                );
            }
            snap.line(
                "queued",
                &super::super::service::command_line(RenewalPlayerOutcome::Queued),
            );
            snap.line(
                "unresolved",
                &super::super::service::command_line(RenewalPlayerOutcome::Unresolved),
            );
            let licence = drafted.binding().licence();
            let shown = licence.get().to_string();
            let price = drafted.offer().terms.price.minor();
            let operation = drafted.operation();
            for (name, step) in [
                ("request", FrontStep::Request),
                ("accept", FrontStep::Accept(operation)),
                ("decline", FrontStep::Decline(operation)),
            ] {
                snap.line(
                    &format!("front line {name}"),
                    &front_line(&shown, step, price),
                );
                snap.line(
                    &format!("front argument {name}"),
                    &front_argument(licence, step),
                );
            }
            for stop in [
                RenewalStop::Recalled,
                RenewalStop::Withdrawn,
                RenewalStop::OwnCharter,
            ] {
                snap.line(
                    &format!("stopped {stop:?}"),
                    &crate::mind_renewal::player::renew_stopped(stop),
                );
            }
            Ok(())
        }

        /// A codec pin of each `data!` type.
        fn ct0_codecs(
            snap: &mut Snapshot,
            drafted: &MindRenewalOperationV1,
            granted: &MindRenewalOperationV1,
        ) -> TestResult {
            snap.codec(
                &format!("{} MindRenewalOperationV1 (drafted)", $twin),
                drafted,
                |json| json["format"] = 9.into(),
            )?;
            let retry: MindRenewalOperationV1 = {
                let mut value = serde_json::to_value(drafted)?;
                value["attempt"] = json!({"kind":"buyer_retry","previous_operation":token(8)});
                serde_json::from_value(value)?
            };
            snap.codec(
                &format!("{} MindRenewalOperationV1 (buyer retry)", $twin),
                &retry,
                |json| json["format"] = 9.into(),
            )?;
            snap.codec(
                &format!("{} MindRenewalReceiptV1 (granted)", $twin),
                &granted.receipt()?,
                |json| json["format"] = 9.into(),
            )?;
            Ok(())
        }

        /// Drafted, funded and granted in one store; returns the Draft's and
        /// the Fund's writes, which every other end must repeat.
        #[inline(never)]
        fn ct0_granted_path(
            snap: &mut Snapshot,
            drafted: &MindRenewalOperationV1,
            fund: &MindRenewalOperationV1,
        ) -> TestResult<Box<(Vec<ReplayWrite>, Vec<ReplayWrite>)>> {
            let (mut engine, families) = ct0_rig()?;
            let mut disk = Ct0Disk::new();
            let draft_writes = ct0_step(&mut engine, &families, &mut disk, drafted, true)?;
            ct0_state(snap, &engine, &families, "drafted", drafted, &draft_writes)?;
            ct0_cold(&engine, &families, &disk, drafted)?;
            let fund_writes = ct0_step(&mut engine, &families, &mut disk, fund, false)?;
            ct0_state(snap, &engine, &families, "funded", fund, &fund_writes)?;
            ct0_cold(&engine, &families, &disk, fund)?;
            let granted = Box::new(terminal(fund, true)?);
            let writes = ct0_step(&mut engine, &families, &mut disk, &granted, false)?;
            ct0_state(snap, &engine, &families, "granted", &granted, &writes)?;
            ct0_cold(&engine, &families, &disk, &granted)?;
            Ok(Box::new((draft_writes, fund_writes)))
        }

        /// One other end, in its own store from the same start.
        #[inline(never)]
        fn ct0_end(
            snap: &mut Snapshot,
            title: &str,
            drafted: &MindRenewalOperationV1,
            fund: Option<&MindRenewalOperationV1>,
            end: &MindRenewalOperationV1,
            before: &(Vec<ReplayWrite>, Vec<ReplayWrite>),
        ) -> TestResult {
            let (mut engine, families) = ct0_rig()?;
            let mut disk = Ct0Disk::new();
            check_eq!(
                ct0_step(&mut engine, &families, &mut disk, drafted, true)?,
                before.0
            );
            if let Some(fund) = fund {
                check_eq!(
                    ct0_step(&mut engine, &families, &mut disk, fund, false)?,
                    before.1
                );
            }
            let writes = ct0_step(&mut engine, &families, &mut disk, end, false)?;
            check!(end.state().retryable(), "{title} is retryable");
            ct0_state(snap, &engine, &families, title, end, &writes)?;
            ct0_cold(&engine, &families, &disk, end)
        }

        /// The service's private close intent: its capability id is the
        /// `renewal_service_intent` digest of the exact drafted row.
        #[inline(never)]
        fn ct0_intent(
            snap: &mut Snapshot,
            drafted: &MindRenewalOperationV1,
            service: &mush_core::DefinitionRef,
        ) -> TestResult {
            snap.section(&format!("{} renewal: service close intent", $twin));
            for reason in ["deadline_reached", "clock_invalidated"] {
                let intent = Box::new(super::super::service::CloseIntent {
                    before: drafted.clone(),
                    action: service.clone(),
                    reason: serde_json::from_value(json!(reason))?,
                    key: mush_core::action::IdempotencyKeyText::parse_canonical(
                        "ct0.renewal.close.intent",
                    )?,
                });
                snap.wire(&format!("{reason} parameters"), &intent.parameters())?;
                let mush_core::action::AuthorityGuard::Capability {
                    provider,
                    actor,
                    capability,
                    generation,
                } = intent.guard()?
                else {
                    return Err("the close intent is a capability guard".into());
                };
                snap.line(
                    &format!("{reason} guard"),
                    &format!(
                        "provider {} actor {} capability {} generation {generation}",
                        provider.as_str(),
                        actor.0,
                        capability.as_str()
                    ),
                );
            }
            Ok(())
        }

        #[test]
        fn $test() -> TestResult {
            let mut snap = Snapshot::new();
            let drafted = Box::new(fixture()?);
            ct0_identity(&mut snap, &drafted)?;
            let configs = $configs(&drafted, ct0_policy())?;
            for (label, config) in &configs {
                ct0_config(&mut snap, label, config)?;
            }
            let (_, config) = configs.first().ok_or("a configuration")?;
            let service = config
                .clone()
                .with_service()?
                .service_action()?
                .ok_or("service action")?;
            let fund = Box::new(funded(&drafted)?);
            let before = ct0_granted_path(&mut snap, &drafted, &fund)?;
            let ends = vec![
                ("refunded", true, terminal(&fund, false)?),
                ("unfunded closed: declined", false, closed(&drafted, false)?),
                ("unfunded closed: expired", false, closed(&drafted, true)?),
                (
                    "service closed: deadline reached",
                    false,
                    ct0_service_closed(&drafted, &service, false)?,
                ),
                (
                    "service closed: clock invalidated",
                    false,
                    ct0_service_closed(&drafted, &service, true)?,
                ),
            ];
            for (title, through_fund, end) in &ends {
                let fund = through_fund.then_some(fund.as_ref());
                ct0_end(&mut snap, title, &drafted, fund, end, &before)?;
            }
            ct0_intent(&mut snap, &drafted, &service)?;
            ct0_words(&mut snap, &drafted)?;
            ct0_codecs(&mut snap, &drafted, &terminal(&fund, true)?)?;
            snap.check(env!("CARGO_MANIFEST_DIR"), $file)
                .map_err(Into::into)
        }
    };
}
pub(crate) use renewal_walk;

/// The renewal service's private names, read by a child of each twin's
/// service (`service::ct0_service`), which calls this once.
macro_rules! service_lines {
    () => {
        use mush_testkit::snapshot::Snapshot;

        /// The service's command, its uniform refusal, its lasting-refusal
        /// line, its bounds, and the retained-entry key of each selection.
        pub(crate) fn ct0_lines(snap: &mut Snapshot) -> Result<(), Box<dyn std::error::Error>> {
            snap.line("service command", COMMAND);
            snap.line("service max entries", &MAX_ENTRIES.to_string());
            snap.line("service page", &PAGE.to_string());
            snap.line("service refusal", &unavailable().to_string());
            snap.line(
                "service stopped",
                &stopped(RenewalStop::Withdrawn).to_string(),
            );
            let licence = LicenceId::new(7)?;
            let generation = TermGeneration::new(1)?;
            let operation = CommitToken::from_bytes([10; 16]);
            let previous = CommitToken::from_bytes([8; 16]);
            for (name, selection) in [
                ("due", Selection::Due(licence, generation, 1)),
                ("due, later holder", Selection::Due(licence, generation, 3)),
                ("pending", Selection::Pending(operation)),
                ("retry", Selection::Retry(licence, generation, previous)),
            ] {
                snap.line(&format!("entry key {name}"), &key(selection));
            }
            Ok(())
        }
    };
}
pub(crate) use service_lines;

/// The House twin's configurations: one-seat House Pins, and with two-seat
/// Pins added (a second purchase policy the renewal policy references).
fn ct0_house_configs(
    row: &MindRenewalOperationV1,
    policy: super::super::RenewalPolicy,
) -> TestResult<Vec<(&'static str, super::super::RenewalConfig)>> {
    let [economy, retention] = row.participants();
    let house_policy = crate::mind_house::HousePolicy {
        price: Amount::from_minor(500),
        instrument: InstrumentId::new("cr").ok_or("instrument")?,
        registry: GrossRecipientCut {
            permille: 0,
            recipient: None,
        },
        venue_name: crate::MarketId::new("house").ok_or("venue")?,
        venue: GrossRecipientCut {
            permille: 0,
            recipient: None,
        },
        transferable: false,
        term_seconds: 100,
        grace_seconds: 20,
        quote_seconds: 60,
        renewal_price: Amount::from_minor(500),
        renewal_term_seconds: 100,
        renewal_grace_seconds: 20,
    };
    let house = crate::mind_house::HouseConfig::new(
        house_policy.clone(),
        row.manifest().clone(),
        PurchaseParticipants::new(economy.clone(), retention.clone())?,
        mush_roll::commerce::house_purchase::writer::HousePurchaseWriter::new(),
    )?;
    let two_seat = house.clone().with_two_seat_pins(house_policy)?;
    let writer = mush_roll::commerce::house_renewal::writer::RenewalWriter::new;
    Ok(vec![
        (
            "one-seat House Pins",
            super::super::RenewalConfig::new(&house, writer(), policy.clone())?,
        ),
        (
            "with two-seat House Pins",
            super::super::RenewalConfig::new(&two_seat, writer(), policy)?,
        ),
    ])
}

renewal_walk! {
    test: ct0_house_renewal_snapshot,
    twin: "house",
    file: "tests/ct0/renewal_house.snap",
    configs: ct0_house_configs,
}
