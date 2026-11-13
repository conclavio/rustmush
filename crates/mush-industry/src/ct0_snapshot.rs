#![cfg(test)]

//! Doc 46 CT.0: what the two refit walks share. The bay walk
//! (`bay_refit::rig_tests::ct0_snapshot`) and the core walk
//! (`core_production::…::refit_tests::ct0_snapshot`) each drive their own private rig, which
//! the other cannot see; the writing of a transition (the staged bundle, every owner's replay
//! write, anything the store took outside the bundle) and of a registered descriptor is the
//! same for both and lives here once.
//!
//! **Scope** (the lead's ruling for CT.7): the files pin the refit family's own bytes and
//! nothing another programme owns. A value is written in full only for Economy's documents
//! (its namespace and its journal chunks), the kernel's capacity reservation, and Industry's
//! refit-family records ([`Scope`]); every other write (Tactics, Inventory, Sim, Org, object
//! images and the allocator, Industry's non-refit records such as facility slots and books) is
//! written as kind, namespace and key only. No hash over a whole bundle is written.
use mush_core::action::{PreparedAuthoritativeAction, PreparedCausalRef};
use mush_core::authoritative::{AuthoritativeBundle, AuthoritativeBundleDraft, ReplayWrite};
use mush_core::composed::PreparedComposedTxn;
use mush_core::mechanics::{DefinitionDescriptor, OwnerId};
use mush_core::{
    ActionInvocation, CanonicalHash, CommitToken, DefinitionRef, ModuleBootData,
    StateFamilyDescriptor,
};
use mush_testkit::snapshot::Snapshot;
use smol_str::SmolStr;
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub(crate) type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
pub(crate) type Prepared = (PreparedComposedTxn, PreparedAuthoritativeAction);
/// A store image: every document, keyed `(namespace, key)`, as stored text.
pub(crate) type Documents = BTreeMap<(String, String), String>;

pub(crate) fn json<T: serde::Serialize + ?Sized>(value: &T) -> Result<String> {
    Ok(serde_json::to_string(value)?)
}

/// Every persisted document of a rig's store (JSON documents, ledger chunks, streams).
pub(crate) fn documents(boot: &HashMap<SmolStr, ModuleBootData>) -> Result<Documents> {
    let mut out = BTreeMap::new();
    for (namespace, data) in boot {
        for (key, value) in &data.kv {
            out.insert((namespace.to_string(), key.to_string()), json(value)?);
        }
        for (key, raw) in &data.ledger {
            out.insert((namespace.to_string(), key.to_string()), raw.clone());
        }
        for (key, entries) in &data.streams {
            out.insert(
                (namespace.to_string(), format!("stream {key}")),
                json(entries)?,
            );
        }
    }
    Ok(out)
}

/// Which Industry documents are the refit family's own: its state families' typed records
/// and its census key. Everything else Industry writes is pinned by key only.
pub(crate) struct Scope<'a> {
    pub(crate) families: &'a [&'a str],
    pub(crate) index_key: &'a str,
}
impl Scope<'_> {
    /// Whether the document's value is the refit family's (or Economy's, or the kernel's
    /// capacity reservation) and is written in full.
    pub(crate) fn pinned(&self, namespace: &str, key: &str) -> bool {
        namespace == "economy"
            || (namespace == "industry"
                && (key.starts_with("action-state/capacity/")
                    || key == self.index_key
                    || self.families.iter().any(|f| {
                        key.strip_prefix("action-state/v1/")
                            .and_then(|rest| rest.strip_prefix(f))
                            .is_some_and(|rest| rest.starts_with('/'))
                    })))
    }
}

/// The store changes between two images that `written` does not account for: a value in
/// scope in full, any other by its key only.
pub(crate) fn store_changes(
    s: &mut Snapshot,
    scope: &Scope<'_>,
    label: &str,
    before: &Documents,
    after: &Documents,
    written: &BTreeSet<(String, String)>,
) {
    let mut any = false;
    for (target, value) in after {
        if before.get(target) != Some(value) && !written.contains(target) {
            let (namespace, key) = target;
            if scope.pinned(namespace, key) {
                s.line(&format!("{label} set {namespace} {key}"), value);
            } else {
                s.line(&format!("{label} set (key only) {namespace} {key}"), "");
            }
            any = true;
        }
    }
    for target in before.keys() {
        if !after.contains_key(target) && !written.contains(target) {
            s.line(&format!("{label} deleted {} {}", target.0, target.1), "");
            any = true;
        }
    }
    if !any {
        s.line(label, "none");
    }
}

/// The bundle both rigs stage for a preparation (`Rig::stage`, `FactoryRig::stage_action`),
/// built from the same parts and the same fixed hashes.
pub(crate) fn bundle(
    action: DefinitionRef,
    inv: &ActionInvocation,
    n: u8,
    p: &Prepared,
) -> Result<AuthoritativeBundle> {
    let token = CommitToken::from_bytes([n; 16]);
    let (outcome, resolution, mut definitions, eligible_witnesses, durable_outbox, semantic_events) =
        p.1.clone().into_parts();
    definitions.push(action.clone());
    definitions.sort();
    definitions.dedup();
    Ok(AuthoritativeBundle::new(AuthoritativeBundleDraft {
        token,
        request: inv.request(),
        idempotency: inv.idempotency().clone(),
        ingress_hash: CanonicalHash::from_bytes([1; 32]),
        request_hash: inv.request_hash(),
        plan_hash: CanonicalHash::from_bytes([3; 32]),
        action,
        provider: OwnerId::new("industry")?,
        cause: PreparedCausalRef::new(inv.request(), None, 0)?,
        definitions,
        writes: p.0.authoritative_replay_writes(),
        outcome,
        resolution: resolution.map(|r| r.bind(token)),
        semantic_events,
        eligible_witnesses,
        durable_outbox,
    })?)
}

/// One staged transition: the invocation's parameters, the action, the definitions (the
/// refit family's actions, outcomes and participants in full, any other by its key), the
/// refit outcome, every replay write of every owner (in scope with its value, else by its key:
/// [`Scope`]), and whatever the store took outside the bundle.
pub(crate) fn transition(
    s: &mut Snapshot,
    scope: &Scope<'_>,
    title: &str,
    now: i64,
    inv: &ActionInvocation,
    b: &AuthoritativeBundle,
    images: [&Documents; 2],
) -> Result {
    let [before, after] = images;
    s.section(title);
    s.line("now utc", &now.to_string());
    s.line("invocation parameters", &json(inv.parameters())?);
    s.wire("action", b.action())?;
    for d in b.definitions() {
        let key = d.key();
        if ["action", "outcome", "owner_participant"].contains(&key.kind.as_str()) {
            s.wire("definition", d)?;
        } else {
            s.line(
                "definition (key only)",
                &format!(
                    "{}/{}/{}/{}",
                    key.owner.as_str(),
                    key.kind.as_str(),
                    key.id.as_str(),
                    d.version()
                ),
            );
        }
    }
    // The outcome is the refit family's own record (`bay_refit_changed` /
    // `core_refit_changed`: operation, offer, order, state).
    s.wire("outcome", b.outcome())?;
    let mut written = BTreeSet::new();
    for write in b.writes() {
        match write {
            ReplayWrite::ModuleJsonSet { namespace, key, .. }
            | ReplayWrite::ModuleDelete { namespace, key } => {
                let kind = match write {
                    ReplayWrite::ModuleDelete { .. } => "module_delete",
                    _ => "module_json_set",
                };
                if scope.pinned(namespace.as_str(), key.as_str()) {
                    s.wire("write", write)?;
                } else {
                    s.line(
                        "write (key only)",
                        &format!("{kind} {} {}", namespace.as_str(), key.as_str()),
                    );
                }
                written.insert((namespace.as_str().to_owned(), key.as_str().to_owned()));
            }
            ReplayWrite::LedgerChunkSet { write: chunk } => {
                s.wire("write", write)?;
                written.insert((
                    chunk.namespace().as_str().to_owned(),
                    chunk.key().as_str().to_owned(),
                ));
            }
            ReplayWrite::ObjectSet { object } => {
                s.line("write (key only)", &format!("object_set {}", object.id.0));
            }
            ReplayWrite::ObjectDelete { id } => {
                s.line("write (key only)", &format!("object_delete {}", id.0));
            }
            ReplayWrite::ObjectAllocatorSet { .. } => {
                s.line("write (key only)", "object_allocator_set");
            }
        }
    }
    store_changes(
        s,
        scope,
        "store outside the bundle",
        before,
        after,
        &written,
    );
    Ok(())
}

/// What a walk read back: `(written, label, value)`. A record of the refit family (or
/// Economy's) is written; another owner's (a unit image, a lot, an object's location) is only
/// compared, live against a cold boot, outside the snapshot.
pub(crate) type Records = Vec<(bool, String, String)>;

/// The written lines of `records`.
pub(crate) fn lines(s: &mut Snapshot, records: &Records) {
    for (written, label, value) in records {
        if *written {
            s.line(label, value);
        }
    }
}

/// A cold boot's report: what the store took at boot, and that every record (written or
/// only compared) equals the live one, which the caller has asserted.
pub(crate) fn cold_boot(
    s: &mut Snapshot,
    scope: &Scope<'_>,
    title: &str,
    before: &Documents,
    after: &Documents,
    live: &Records,
    recovered: &Records,
) -> Result {
    if recovered != live {
        return Err(format!("{title}: the cold boot did not rebuild every live record").into());
    }
    s.section(title);
    store_changes(s, scope, "store at boot", before, after, &BTreeSet::new());
    s.line("records recovered equal live", "true");
    s.line(
        "records written",
        &recovered.iter().filter(|r| r.0).count().to_string(),
    );
    Ok(())
}

/// A registered family's exact descriptor, every identity in it.
pub(crate) fn family(s: &mut Snapshot, label: &str, d: &StateFamilyDescriptor) {
    s.line(
        label,
        &format!(
            "{} key={} value={} schema={} decoder={} validator={} serializer={} max_key={} max_record={}",
            d.id().as_str(),
            d.key_codec().as_str(),
            d.value_schema().as_str(),
            d.schema_version(),
            d.decoder().as_str(),
            d.validator().as_str(),
            d.serializer().as_str(),
            d.max_key_bytes(),
            d.max_record_bytes()
        ),
    );
}

/// Every definition an owner of the refit (Economy, Industry, Inventory, Tactics; never the
/// ruleset's content) registered whose kind or id holds one of `words`, in reference order:
/// its reference (with its digest) and its payload. This reaches the definitions no bundle
/// names, such as Economy's own refit policy definitions.
pub(crate) fn registered(s: &mut Snapshot, engine: &mush_core::Engine, words: &[&str]) -> Result {
    let definitions = engine.registry.definitions().ok_or("definitions")?;
    let mut rows: Vec<_> = definitions
        .iter()
        .filter(|d| {
            let key = d.reference().key();
            ["economy", "industry", "inventory", "tactics"].contains(&key.owner.as_str())
                && words
                    .iter()
                    .any(|w| key.id.as_str().contains(w) || key.kind.as_str().contains(w))
        })
        .collect();
    rows.sort_by(|a, b| a.reference().cmp(b.reference()));
    for d in rows {
        s.wire("registered", d.reference())?;
        s.line("registered payload", &json(d.payload())?);
    }
    Ok(())
}

/// A codec pin's invalidating edit: drop a field the strict decoder requires (a
/// `required_option` field, which a refactor to a plain `Option` would make optional).
pub(crate) fn without(field: &'static str) -> impl FnOnce(&mut serde_json::Value) {
    move |json| {
        if let Some(object) = json.as_object_mut() {
            object.remove(field);
        }
    }
}

/// A codec pin's invalidating edit: a field of the wrong type.
pub(crate) fn mistyped(field: &'static str) -> impl FnOnce(&mut serde_json::Value) {
    move |json| {
        if let Some(object) = json.as_object_mut() {
            object.insert(field.to_owned(), "ct0_mistyped".into());
        }
    }
}

/// A definition's descriptor: its identity, payload, references and currency.
pub(crate) fn definition(s: &mut Snapshot, label: &str, d: &DefinitionDescriptor) -> Result {
    s.line(
        &format!("{label} kind/id/version"),
        &format!("{}/{}/{}", d.kind().as_str(), d.id().as_str(), d.version()),
    );
    s.line(&format!("{label} payload"), &json(d.payload())?);
    s.wire(&format!("{label} references"), d.references())?;
    s.line(&format!("{label} current"), &d.is_current().to_string());
    Ok(())
}
