//! Doc 46 CT.0: the writing helpers the Roll's commerce snapshots share.
//!
//! A walk writes each typed family it meets through [`family`] (the registered
//! descriptor: owner, id, the five contract ids, the bounds) and each row it
//! persists through [`document`] (the full key, the record in its struct's field
//! order, the stored canonical bytes), and each definition it registers through
//! [`descriptor`] (kind, id, version, payload, references, and the registered
//! reference with its digest). The renewal twins share one walk, [`ct0_renewal_walk`].

use super::super::families::{CommerceFamilies, valid_key};
use super::super::purchase::PurchaseParticipants;
use super::super::records::Record;
use mush_core::{
    DefinitionRef, Engine, GameModule, ModError, Registrar, Registry, StateFamilyHandle, World,
    mechanics::{
        DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
    },
};
use mush_testkit::snapshot::Snapshot;
use std::sync::{Arc, Mutex};

pub(crate) type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

struct Owner(Arc<Mutex<Option<CommerceFamilies>>>);
impl GameModule for Owner {
    fn name(&self) -> &'static str {
        "roll"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        *self.0.lock().map_err(|_| ModError::msg("family lock"))? =
            Some(CommerceFamilies::register(reg)?);
        Ok(())
    }
}

/// The Roll's real registered commerce families, in a minimal engine.
#[inline(never)]
pub(crate) fn families() -> TestResult<(Box<Engine>, CommerceFamilies)> {
    let shared = Arc::new(Mutex::new(None));
    let engine = Box::new(Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        vec![Box::new(Owner(shared.clone()))],
    )?);
    let families = shared
        .lock()
        .map_err(|_| "family lock")?
        .clone()
        .ok_or("families")?;
    Ok((engine, families))
}

/// One registered family: everything its descriptor declares.
pub(crate) fn family<T: Record>(s: &mut Snapshot, handle: &StateFamilyHandle<String, T>) {
    let d = handle.descriptor();
    let id = d.id().as_str().to_owned();
    s.line(&format!("family {id} owner"), handle.owner().as_str());
    s.line(&format!("family {id} record FAMILY"), T::FAMILY);
    s.line(&format!("family {id} key codec"), d.key_codec().as_str());
    s.line(
        &format!("family {id} value schema"),
        d.value_schema().as_str(),
    );
    s.line(
        &format!("family {id} schema version"),
        &d.schema_version().to_string(),
    );
    s.line(&format!("family {id} decoder"), d.decoder().as_str());
    s.line(&format!("family {id} validator"), d.validator().as_str());
    s.line(&format!("family {id} serializer"), d.serializer().as_str());
    s.line(
        &format!("family {id} max key bytes"),
        &d.max_key_bytes().to_string(),
    );
    s.line(
        &format!("family {id} max record bytes"),
        &d.max_record_bytes().to_string(),
    );
    s.line(
        &format!("family {id} record MAX_BYTES"),
        &T::MAX_BYTES.to_string(),
    );
}

/// One persisted row: its full key, the record in its struct's field order and the
/// stored canonical bytes. Returns the stored bytes for a cold read-back.
pub(crate) fn document<T: Record>(
    s: &mut Snapshot,
    label: &str,
    handle: &StateFamilyHandle<String, T>,
    row: &T,
) -> TestResult<Vec<u8>> {
    let key = row.key();
    check!(
        valid_key(T::FAMILY, &key),
        "{label}: {key} is not a valid {} key",
        T::FAMILY
    );
    document_keyed(s, label, handle, row)
}

/// [`document`] for a family whose key codec is its own (the Conduct families key
/// by the bare commit token), which the caller checks.
pub(crate) fn document_keyed<T: Record>(
    s: &mut Snapshot,
    label: &str,
    handle: &StateFamilyHandle<String, T>,
    row: &T,
) -> TestResult<Vec<u8>> {
    row.validate()?;
    let key = row.key();
    s.line(
        &format!("{label} key"),
        &format!(
            "{}/{}/{key}",
            handle.owner().as_str(),
            handle.descriptor().id().as_str()
        ),
    );
    s.wire(&format!("{label} record"), row)?;
    let stored = mush_core::canonical_json_bytes(&serde_json::to_value(row)?)?;
    s.bytes(&format!("{label} stored"), &stored);
    Ok(stored)
}

/// One definition: what its descriptor declares and the reference `owner`
/// registers it as (its digest included).
pub(crate) fn descriptor(
    s: &mut Snapshot,
    label: &str,
    owner: &str,
    d: &DefinitionDescriptor,
) -> TestResult<DefinitionRef> {
    s.line(&format!("{label} kind"), d.kind().as_str());
    s.line(&format!("{label} id"), d.id().as_str());
    s.line(&format!("{label} version"), &d.version().to_string());
    s.wire(&format!("{label} payload"), d.payload())?;
    for (n, reference) in d.references().iter().enumerate() {
        s.wire(&format!("{label} reference {n}"), reference)?;
    }
    s.line(&format!("{label} current"), &d.is_current().to_string());
    let registered = DefinitionRegistryBuilder::new().register(OwnerId::new(owner)?, d.clone())?;
    s.wire(&format!("{label} registered"), &registered)?;
    Ok(registered)
}

/// The stored bytes decode to the live row.
pub(crate) fn read_back<T>(stored: &[u8], live: &T) -> TestResult
where
    T: serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let back: T = serde_json::from_slice(stored)?;
    check_eq!(back, *live);
    Ok(())
}

/// A definition `owner` registers under `kind`/`id`, version 1, with a fixture payload.
pub(crate) fn reference(owner: &str, kind: &str, id: &str) -> TestResult<DefinitionRef> {
    Ok(DefinitionRegistryBuilder::new().register(
        OwnerId::new(owner)?,
        DefinitionDescriptor::new(
            DefinitionKind::new(kind)?,
            DefinitionId::new(id)?,
            1,
            serde_json::json!({"fixture":1}),
        )?,
    )?)
}

/// The combat registry manifest every commerce protocol descriptor names.
pub(crate) fn manifest() -> TestResult<DefinitionRef> {
    reference("tacsim", "registry_manifest", "combat")
}

/// The two owner participants every purchase-shaped protocol names.
pub(crate) fn participants() -> TestResult<PurchaseParticipants> {
    Ok(PurchaseParticipants::new(
        reference("economy", "owner_participant", "initial_filing")?,
        reference("roll", "owner_participant", "commercial_roots")?,
    )?)
}

/// The walk of one renewal twin (doc 46 CT.0), written once for both.
///
/// The instantiating module is a child of the twin's `tests` module (so the twin's
/// rig, `chain`, `audit`, `capacity`, `history`, `roots`, `protocol` and `reader` are
/// in scope) and supplies four adapters: `ct0_offer(row)` (the Economy offer the row
/// carries), `ct0_prepare(action, commit, publication, before, after)` (the twin's
/// `RenewalRoots::prepare` for a transition that holds no fresh facts),
/// `ct0_verify(licence, head, rows, terms, economics)` (the twin's `chain::verify`)
/// and `ct0_birth(s, f, b)` (the birth's rows and the twin's own digests).
///
/// Not reached: the paired writer itself, which needs the Economy owner's opaque
/// outputs, and the roots of the three fresh transitions (Draft, Fund, Grant),
/// which need the reader's `RenewalFacts`; the walk drives every check those
/// call on the rows the writer would write.
macro_rules! ct0_renewal_walk {
    (
        test: $test:ident,
        title: $title:literal,
        file: $file:literal,
        economy: $econ:ident,
        operations: $ops:ident,
        heads: $heads:ident,
        economics: $economics:ident,
    ) => {
        use $crate::commerce::tests::ct0_rig::{self as rig, TestResult as Ct0Result};
        use mush_testkit::snapshot::Snapshot;
        use mush_economy::$econ as econ;
        use mush_economy::managed_paid_resale::{PaidHolderRefV1, RenewalHolderBasisV1};

        fn ct0_section(s: &mut Snapshot, title: &str) {
            s.section(&format!("{}: {title}", $title));
        }

        fn ct0_basis(generation: u8) -> RenewalHolderBasisV1 {
            RenewalHolderBasisV1 {
                acceptance: token(210 + generation),
                ownership: PaidHolderRefV1 {
                    generation: u64::from(generation),
                    operation: token(220 + generation),
                    digest: digest(230 + generation),
                },
            }
        }

        /// A walked operation row, boxed: the walk keeps many alive and each is
        /// several KiB, so no function's frame holds them (the M7 64 KiB lint).
        type Ct0Row = Box<RenewalOperationV1>;

        /// A drafted row on the real Draft action, carrying the real offer digest.
        #[inline(never)]
        fn ct0_drafted(
            b: &Birth,
            n: u8,
            at: i64,
            attempt: RenewalAttemptV1,
            holder: Option<u8>,
            refs: &[mush_core::DefinitionRef; 5],
        ) -> Ct0Result<Ct0Row> {
            let mut row = draft(b, n, at, RenewalAttemptV1::Automatic {})?;
            if let Some(generation) = holder {
                row.binding.format = 2;
                row.binding.holder = mush_core::ObjId(40 + u32::from(generation)).into();
                row.binding.holder_basis = Some(ct0_basis(generation));
            }
            row.attempt = attempt;
            row.action = protocol::Transition::Draft.select(refs).clone();
            row.offer_digest = ct0_offer(&row)?.digest()?;
            row.validate()?;
            Ok(Box::new(row))
        }

        #[inline(never)]
        fn ct0_funded(
            row: &RenewalOperationV1,
            commit: u8,
            refs: &[mush_core::DefinitionRef; 5],
        ) -> Ct0Result<Ct0Row> {
            let at = row.created_at_utc + 1;
            let receipt: econ::RenewalFundingReceiptV1 = serde_json::from_value(
                serde_json::json!({"format":1,"commit":token(commit),
                "action":protocol::Transition::Fund.select(refs),"contract":row.contract,
                "hold":at,"transaction":at,"journal_seq":at,"consent_hash":row.consent_hash,
                "at_ic":stamp(at),"posted_at_utc":at}),
            )?;
            let next = RenewalOperationV1 {
                state: RenewalStateV1::Funded { funding: receipt },
                ..row.clone()
            };
            next.follows(row)?;
            Ok(Box::new(next))
        }

        #[inline(never)]
        fn ct0_terminal(
            row: &RenewalOperationV1,
            commit: u8,
            grant: bool,
            refs: &[mush_core::DefinitionRef; 5],
        ) -> Ct0Result<Ct0Row> {
            let funding = row.state.funding().ok_or("funding")?.clone();
            let at = row.created_at_utc + 2;
            let transition = if grant {
                protocol::Transition::Grant
            } else {
                protocol::Transition::Refund
            };
            let receipt: econ::RenewalTerminalReceiptV1 = serde_json::from_value(
                serde_json::json!({"format":1,"commit":token(commit),
                "action":transition.select(refs),
                "outcome":if grant {"granted"} else {"refunded"},
                "transaction":at,"journal_seq":at,"at_ic":stamp(at),"posted_at_utc":at,
                "issued_at_ic":if grant {Some(stamp(at))} else {None}}),
            )?;
            let state = if grant {
                RenewalStateV1::Granted {
                    funding,
                    terminal: receipt,
                }
            } else {
                RenewalStateV1::Refunded {
                    funding,
                    terminal: receipt,
                }
            };
            let next = RenewalOperationV1 {
                state,
                ..row.clone()
            };
            next.follows(row)?;
            Ok(Box::new(next))
        }

        #[inline(never)]
        fn ct0_closed(
            row: &RenewalOperationV1,
            commit: u8,
            reason: econ::UnfundedCloseReason,
            refs: &[mush_core::DefinitionRef; 5],
        ) -> Ct0Result<Ct0Row> {
            let at = row.created_at_utc + 2;
            let next = RenewalOperationV1 {
                state: RenewalStateV1::UnfundedClosed {
                    closure: RenewalClosureV1 {
                        commit: token(commit),
                        action: protocol::Transition::Close.select(refs).clone(),
                        at_ic: stamp(at),
                        posted_at_utc: at,
                        reason,
                    },
                },
                ..row.clone()
            };
            next.follows(row)?;
            Ok(Box::new(next))
        }

        #[inline(never)]
        fn ct0_service_closed(
            row: &RenewalOperationV1,
            commit: u8,
            reason: econ::RenewalServiceCloseReason,
            service: &mush_core::DefinitionRef,
        ) -> Ct0Result<Ct0Row> {
            let at = row.created_at_utc + 3;
            // A closure for a stopped clock is stamped on the clock that replaced it.
            let generation = match reason {
                econ::RenewalServiceCloseReason::DeadlineReached => 1,
                econ::RenewalServiceCloseReason::ClockInvalidated => 2,
            };
            let at_ic = mush_core::ClockStamp {
                clock: mush_core::ClockRef::WorldIc { generation },
                at,
            };
            let closure: econ::RenewalServiceClosureV1 = serde_json::from_value(
                serde_json::json!({"format":1,"commit":token(commit),"action":service,
                "actor":mush_core::GOD,"at_ic":at_ic,"posted_at_utc":at,"reason":reason}),
            )?;
            let next = RenewalOperationV1 {
                state: RenewalStateV1::UnfundedServiceClosed { closure },
                ..row.clone()
            };
            next.follows(row)?;
            Ok(Box::new(next))
        }

        /// Everything one state persists and derives. Returns the stored operation
        /// and head bytes, decoded back and compared.
        #[inline(never)]
        fn ct0_state(
            s: &mut Snapshot,
            f: &$crate::commerce::families::CommerceFamilies,
            title: &str,
            row: &RenewalOperationV1,
        ) -> Ct0Result<Ct0Row> {
            ct0_section(s, title);
            let state = serde_json::to_value(row.state())?;
            s.line(
                "state kind",
                state.get("kind").and_then(|k| k.as_str()).ok_or("kind")?,
            );
            let attempt = serde_json::to_value(row.attempt())?;
            s.line(
                "attempt kind",
                attempt.get("kind").and_then(|k| k.as_str()).ok_or("kind")?,
            );
            s.line(
                "attempt record format",
                &row.attempt().record_format().to_string(),
            );
            s.line(
                "pending / granted / retryable",
                &format!(
                    "{} / {} / {}",
                    row.state().pending(),
                    row.state().granted(),
                    row.state().retryable()
                ),
            );
            s.wire("terminal commit", &row.state().terminal_commit())?;
            let stored = rig::document(s, "operation", &f.$ops, row)?;
            let head = RenewalHeadV1::for_operation(row)?;
            let stored_head = rig::document(s, "head", &f.$heads, &head)?;
            // The cold read: the stored bytes decode to the live rows.
            let cold = Box::new(RenewalOperationV1::decode(&stored)?);
            check_eq!(*cold, *row);
            check_eq!(RenewalHeadV1::decode(&stored_head)?, head);
            let offer = ct0_offer(row)?;
            history::check_offer(row, &offer)?;
            s.wire("offer", &offer)?;
            s.wire("offer digest", &offer.digest()?)?;
            s.wire("binding template digest", &row.binding().template_digest()?)?;
            s.wire("binding grant digest", &row.binding().grant().digest()?)?;
            for root in audit::required(row)? {
                s.line("required root", &root.anchor.record());
                s.wire("required root identity", &root.identity)?;
            }
            capacity::check(f, row)?;
            Ok(cold)
        }

        /// The roots a transition that holds no fresh facts prepares.
        #[inline(never)]
        fn ct0_roots(
            s: &mut Snapshot,
            action: &mush_core::DefinitionRef,
            publication: roots::Publication,
            before: &RenewalOperationV1,
            after: &RenewalOperationV1,
        ) -> Ct0Result {
            let commit = after.state().terminal_commit().ok_or("terminal commit")?;
            let prepared = ct0_prepare(action, commit, publication, Some(before), after)?;
            s.wire("prepared commit", &prepared.commit())?;
            for root in prepared.before() {
                s.line("prepared before root", &root.anchor.record());
            }
            for root in prepared.after() {
                s.line("prepared after root", &root.anchor.record());
            }
            Ok(())
        }

        /// The rows a grant writes: the advanced pointer, the next term and its economics.
        #[inline(never)]
        fn ct0_granted(
            s: &mut Snapshot,
            f: &$crate::commerce::families::CommerceFamilies,
            row: &RenewalOperationV1,
        ) -> Ct0Result<Box<Birth>> {
            let next = Box::new(granted_birth(row)?);
            let licence = rig::document(s, "granted licence", &f.paid_licences, &next.licence)?;
            let term = rig::document(s, "granted term", &f.paid_terms, &next.term)?;
            let economics =
                rig::document(s, "granted economics", &f.$economics, &next.economics)?;
            rig::read_back(&licence, &next.licence)?;
            rig::read_back(&term, &next.term)?;
            rig::read_back(&economics, &next.economics)?;
            s.wire("granted economics digest", &next.economics.digest()?)?;
            s.wire(
                "granted term digest",
                &RenewalBindingV1::term_digest(&next.term)?,
            )?;
            Ok(next)
        }

        #[inline(never)]
        fn ct0_stage_identity(
            s: &mut Snapshot,
            f: &$crate::commerce::families::CommerceFamilies,
        ) -> Ct0Result {
            ct0_section(s, "identity strings");
            for t in protocol::Transition::ALL {
                s.line(&format!("transition {t:?} id"), t.id());
            }
            s.line("service close action", econ::SERVICE_CLOSE_ACTION);
            s.line("operation FAMILY", RenewalOperationV1::FAMILY);
            s.line("head FAMILY", RenewalHeadV1::FAMILY);
            s.line("MAX_OPERATIONS", &MAX_OPERATIONS.to_string());
            s.line("MAX_OPERATION_BYTES", &MAX_OPERATION_BYTES.to_string());
            s.line("MAX_HEAD_BYTES", &MAX_HEAD_BYTES.to_string());
            s.line(
                "MAX_RENEWAL_RECEIPT_BYTES",
                &econ::MAX_RENEWAL_RECEIPT_BYTES.to_string(),
            );
            rig::family(s, &f.$ops);
            rig::family(s, &f.$heads);
            rig::family(s, &f.paid_licences);
            rig::family(s, &f.paid_terms);
            rig::family(s, &f.$economics);
            Ok(())
        }

        /// The five registered actions and the service close.
        #[inline(never)]
        fn ct0_stage_protocol(
            s: &mut Snapshot,
        ) -> Ct0Result<Box<([mush_core::DefinitionRef; 5], mush_core::DefinitionRef)>> {
            ct0_section(s, "protocol");
            let manifest = rig::manifest()?;
            let participants = rig::participants()?;
            s.wire("manifest", &manifest)?;
            s.wire("participant economy", participants.economy())?;
            s.wire("participant retention", participants.retention())?;
            let mut refs = Vec::new();
            for t in protocol::Transition::ALL {
                refs.push(rig::descriptor(
                    s,
                    &format!("action {}", t.id()),
                    "market",
                    &protocol::descriptor(t, &manifest, &participants)?,
                )?);
            }
            let refs: [mush_core::DefinitionRef; 5] =
                refs.try_into().map_err(|_| "five actions")?;
            protocol::validate_refs(&manifest, &participants, &refs)?;
            let service = rig::descriptor(
                s,
                "action service close",
                "market",
                &protocol::service_descriptor(&manifest, &participants)?,
            )?;
            check_eq!(
                service,
                protocol::service_reference(&manifest, &participants)?
            );
            rig::descriptor(s, "reader", $crate::NS, &reader::descriptor(&manifest)?)?;
            rig::descriptor(
                s,
                "reader with service",
                $crate::NS,
                &reader::configured_descriptor(&manifest, true)?,
            )?;
            let licence = $crate::commerce::LicenceId::new(7)?;
            s.wire("request own", &reader::RenewalRequest::own(licence))?;
            s.wire(
                "request inventory",
                &reader::RenewalRequest::inventory(Some("x".into())),
            )?;
            s.wire("request status", &reader::RenewalRequest::status(licence))?;
            s.wire(
                "request operation",
                &reader::RenewalRequest::operation(token(10)),
            )?;
            Ok(Box::new((refs, service)))
        }

        #[inline(never)]
        fn ct0_stage_birth(
            s: &mut Snapshot,
            f: &$crate::commerce::families::CommerceFamilies,
        ) -> Ct0Result<Box<Birth>> {
            ct0_section(s, "birth");
            let b = Box::new(birth()?);
            ct0_birth(s, f, &b)?;
            Ok(b)
        }

        /// A: one automatic draft and each of its unfunded ends. Returns the draft,
        /// the declined close and its cold read.
        #[inline(never)]
        fn ct0_stage_a_closes(
            s: &mut Snapshot,
            f: &$crate::commerce::families::CommerceFamilies,
            b: &Birth,
            refs: &[mush_core::DefinitionRef; 5],
            service: &mush_core::DefinitionRef,
        ) -> Ct0Result<(Ct0Row, Ct0Row, Ct0Row)> {
            let a = ct0_drafted(b, 10, 20, RenewalAttemptV1::Automatic {}, None, refs)?;
            let a_cold = ct0_state(s, f, "A automatic: drafted", &a)?;
            s.wire("binding", a.binding())?;
            s.wire("binding term digest", &RenewalBindingV1::term_digest(&b.term)?)?;
            chain::check_draft(&b.licence, &a, None, None)?;
            ct0_verify(
                &b.licence,
                Some(&RenewalHeadV1::for_operation(&a_cold)?),
                &[&*a_cold],
                &std::collections::BTreeMap::from([(b.term.generation(), &b.term)]),
                &std::collections::BTreeMap::from([(b.economics.generation(), &b.economics)]),
            )?;
            let declined = ct0_closed(&a, 12, econ::UnfundedCloseReason::Declined, refs)?;
            let declined_cold = ct0_state(
                s,
                f,
                "A automatic: unfunded closed (declined)",
                &declined,
            )?;
            s.wire("closure", &declined_cold.state())?;
            ct0_roots(
                s,
                protocol::Transition::Close.select(refs),
                protocol::Transition::Close.into(),
                &a,
                &declined,
            )?;
            let expired = ct0_closed(&a, 13, econ::UnfundedCloseReason::Expired, refs)?;
            ct0_state(s, f, "A automatic: unfunded closed (expired)", &expired)?;
            ct0_roots(
                s,
                protocol::Transition::Close.select(refs),
                protocol::Transition::Close.into(),
                &a,
                &expired,
            )?;
            for (reason, commit) in [
                (econ::RenewalServiceCloseReason::DeadlineReached, 14),
                (econ::RenewalServiceCloseReason::ClockInvalidated, 15),
            ] {
                let closed = ct0_service_closed(&a, commit, reason, service)?;
                ct0_state(
                    s,
                    f,
                    &format!(
                        "A automatic: unfunded service closed ({})",
                        serde_json::to_value(reason)?.as_str().ok_or("reason")?
                    ),
                    &closed,
                )?;
                ct0_roots(s, service, roots::Publication::ServiceClose, &a, &closed)?;
            }
            Ok((a, declined, declined_cold))
        }

        /// A: the funded draft, refunded and granted. Returns the refund and its
        /// cold read.
        #[inline(never)]
        fn ct0_stage_a_money(
            s: &mut Snapshot,
            f: &$crate::commerce::families::CommerceFamilies,
            refs: &[mush_core::DefinitionRef; 5],
            a: &RenewalOperationV1,
        ) -> Ct0Result<(Ct0Row, Ct0Row)> {
            let funded = ct0_funded(a, 11, refs)?;
            ct0_state(s, f, "A automatic: funded", &funded)?;
            check_eq!(
                RenewalHeadV1::for_operation(&funded)?,
                RenewalHeadV1::for_operation(a)?
            );
            let refunded = ct0_terminal(&funded, 16, false, refs)?;
            let refunded_cold = ct0_state(s, f, "A automatic: refunded", &refunded)?;
            ct0_roots(
                s,
                protocol::Transition::Refund.select(refs),
                protocol::Transition::Refund.into(),
                &funded,
                &refunded,
            )?;
            let granted = ct0_terminal(&funded, 17, true, refs)?;
            ct0_state(s, f, "A automatic: granted", &granted)?;
            ct0_granted(s, f, &granted)?;
            Ok((refunded, refunded_cold))
        }

        /// B: the buyer retries the refunded attempt, and it is granted. C: the
        /// next generation renews automatically after that grant.
        #[inline(never)]
        fn ct0_stage_b_c(
            s: &mut Snapshot,
            f: &$crate::commerce::families::CommerceFamilies,
            b: &Birth,
            refs: &[mush_core::DefinitionRef; 5],
            a: &RenewalOperationV1,
            refunded: &RenewalOperationV1,
            refunded_cold: &RenewalOperationV1,
        ) -> Ct0Result {
            let retry = ct0_drafted(
                b,
                40,
                40,
                RenewalAttemptV1::BuyerRetry {
                    previous_operation: a.operation(),
                },
                None,
                refs,
            )?;
            chain::check_draft(
                &b.licence,
                &retry,
                Some(&RenewalHeadV1::for_operation(refunded)?),
                Some(refunded),
            )?;
            ct0_state(s, f, "B buyer retry: drafted", &retry)?;
            let retry_funded = ct0_funded(&retry, 41, refs)?;
            ct0_state(s, f, "B buyer retry: funded", &retry_funded)?;
            let retry_granted = ct0_terminal(&retry_funded, 42, true, refs)?;
            let retry_cold = ct0_state(s, f, "B buyer retry: granted", &retry_granted)?;
            let second = ct0_granted(s, f, &retry_granted)?;
            // The cold rebuild of the whole B chain from the written documents.
            ct0_verify(
                &second.licence,
                Some(&RenewalHeadV1::for_operation(&retry_cold)?),
                &[&*retry_cold, refunded_cold],
                &std::collections::BTreeMap::from([
                    (b.term.generation(), &b.term),
                    (second.term.generation(), &second.term),
                ]),
                &std::collections::BTreeMap::from([
                    (b.economics.generation(), &b.economics),
                    (second.economics.generation(), &second.economics),
                ]),
            )?;
            let renewed =
                ct0_drafted(&second, 60, 60, RenewalAttemptV1::Automatic {}, None, refs)?;
            chain::check_draft(
                &second.licence,
                &renewed,
                Some(&RenewalHeadV1::for_operation(&retry_granted)?),
                Some(&retry_granted),
            )?;
            ct0_state(s, f, "C generation two automatic: drafted", &renewed)?;
            Ok(())
        }

        /// D: a new holder renews after the declined close of A, closes, retries,
        /// and is granted. Returns the holder draft and its grant.
        #[inline(never)]
        fn ct0_stage_d(
            s: &mut Snapshot,
            f: &$crate::commerce::families::CommerceFamilies,
            b: &Birth,
            refs: &[mush_core::DefinitionRef; 5],
            declined: &RenewalOperationV1,
            declined_cold: &RenewalOperationV1,
        ) -> Ct0Result<(Ct0Row, Ct0Row)> {
            let holder = ct0_drafted(
                b,
                70,
                70,
                RenewalAttemptV1::HolderAutomatic { basis: ct0_basis(2) },
                Some(2),
                refs,
            )?;
            let pointer = Box::new(holder.binding().accepted_pointer(b.licence.term())?);
            chain::check_draft(
                &pointer,
                &holder,
                Some(&RenewalHeadV1::for_operation(declined)?),
                Some(declined),
            )?;
            ct0_state(s, f, "D holder automatic: drafted", &holder)?;
            s.wire("holder binding", holder.binding())?;
            let holder_closed =
                ct0_closed(&holder, 72, econ::UnfundedCloseReason::Declined, refs)?;
            let holder_closed_cold = ct0_state(
                s,
                f,
                "D holder automatic: unfunded closed (declined)",
                &holder_closed,
            )?;
            let holder_retry = ct0_drafted(
                b,
                80,
                80,
                RenewalAttemptV1::HolderRetry {
                    basis: ct0_basis(2),
                    previous_operation: holder.operation(),
                },
                Some(2),
                refs,
            )?;
            chain::check_draft(
                &pointer,
                &holder_retry,
                Some(&RenewalHeadV1::for_operation(&holder_closed)?),
                Some(&holder_closed),
            )?;
            ct0_state(s, f, "D holder retry: drafted", &holder_retry)?;
            let holder_funded = ct0_funded(&holder_retry, 81, refs)?;
            ct0_state(s, f, "D holder retry: funded", &holder_funded)?;
            let holder_granted = ct0_terminal(&holder_funded, 82, true, refs)?;
            let holder_cold = ct0_state(s, f, "D holder retry: granted", &holder_granted)?;
            let third = ct0_granted(s, f, &holder_granted)?;
            ct0_verify(
                &third.licence,
                Some(&RenewalHeadV1::for_operation(&holder_cold)?),
                &[&*holder_cold, declined_cold, &*holder_closed_cold],
                &std::collections::BTreeMap::from([
                    (b.term.generation(), &b.term),
                    (third.term.generation(), &third.term),
                ]),
                &std::collections::BTreeMap::from([
                    (b.economics.generation(), &b.economics),
                    (third.economics.generation(), &third.economics),
                ]),
            )?;
            Ok((holder, holder_granted))
        }

        /// The macro-built wire types and the hand-written head pair.
        #[inline(never)]
        fn ct0_stage_codecs(
            s: &mut Snapshot,
            refs: &[mush_core::DefinitionRef; 5],
            a: &RenewalOperationV1,
            declined: &RenewalOperationV1,
            holder: &RenewalOperationV1,
            holder_granted: &RenewalOperationV1,
        ) -> Ct0Result {
            let draft_ref = serde_json::to_value(protocol::Transition::Draft.select(refs))?;
            s.codec("RenewalBindingV1 (format 1, no holder basis)", a.binding(), |j| {
                j["format"] = 2.into()
            })?;
            s.codec(
                "RenewalBindingV1 (format 2, holder basis)",
                holder.binding(),
                |j| j["format"] = 1.into(),
            )?;
            let RenewalStateV1::UnfundedClosed { closure } = declined.state() else {
                return Err("closure".into());
            };
            s.codec("RenewalClosureV1", closure, |j| j["action"] = draft_ref)?;
            s.codec("RenewalOperationV1 (granted)", holder_granted, |j| {
                j["format"] = 2.into()
            })?;
            s.codec("RenewalOperationV1 (drafted)", a, |j| j["format"] = 2.into())?;
            s.codec(
                "RenewalHeadV1 (pending)",
                &RenewalHeadV1::for_operation(a)?,
                |j| j["format"] = 2.into(),
            )?;
            s.codec(
                "RenewalHeadV1 (granted)",
                &RenewalHeadV1::for_operation(holder_granted)?,
                |j| j["format"] = 2.into(),
            )?;
            Ok(())
        }

        #[test]
        fn $test() -> Ct0Result {
            let (_engine, f) = rig::families()?;
            let mut s = Snapshot::new();
            ct0_stage_identity(&mut s, &f)?;
            let declared = ct0_stage_protocol(&mut s)?;
            let (refs, service) = &*declared;
            let b = ct0_stage_birth(&mut s, &f)?;
            let (a, declined, declined_cold) = ct0_stage_a_closes(&mut s, &f, &b, refs, service)?;
            let (refunded, refunded_cold) = ct0_stage_a_money(&mut s, &f, refs, &a)?;
            ct0_stage_b_c(&mut s, &f, &b, refs, &a, &refunded, &refunded_cold)?;
            let (holder, holder_granted) =
                ct0_stage_d(&mut s, &f, &b, refs, &declined, &declined_cold)?;
            ct0_stage_codecs(&mut s, refs, &a, &declined, &holder, &holder_granted)?;
            s.check(env!("CARGO_MANIFEST_DIR"), $file)?;
            Ok(())
        }
    };
}
pub(crate) use ct0_renewal_walk;
