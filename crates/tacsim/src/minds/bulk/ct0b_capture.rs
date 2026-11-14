#![cfg(test)]

//! Doc 46 unit CT.0b: the capture on the real composed path.
//!
//! CT.0's snapshots walk each crate over its own stores, and none of them runs the Roll's
//! writers (`commerce/renewal/writer.rs`, `house_renewal/writer.rs`, the purchase `fund` and
//! `terminal`) or the market's providers and dispatched service: those need the composed
//! Economy, Roll and Market owners, and the only rig that composes them is the M7 story this
//! module sits beside. While that story runs, every preparation of one of the renewal twins'
//! or the purchase twins' market actions is recorded here: who asked and with what, what the
//! provider answered (its outcome, or its refusal as printed), and the transition's replay
//! writes. When the story installs a transition that ends an operation, the store's documents
//! under the keys the twin wrote since its last end are recorded too. At the story's end each
//! twin's record is compared with a committed file by [`mush_testkit::snapshot`].
//!
//! **The files hold only what the twins' families own**, so that a change elsewhere does not
//! move them:
//!
//! - A write in the `market`, `roll` or `economy` namespace is written with its value; any
//!   other write (an object, another module's document) by its kind, namespace and key alone.
//! - Of a journal chunk, only the entries the transition appended: the chunk's earlier
//!   entries are other transactions'.
//! - No digest. Every digest on this path inherits TacSim's registry manifest (each action's
//!   descriptor refers to it, and it pins the unit catalogue and the minds' own sources), so a
//!   digest here would move with another programme's edit. A 64-digit hex string, and a hash
//!   written as an array of 32 bytes, is `<dN>`, numbered by first appearance in its section:
//!   what the digests join is kept, their values are not.
//! - No object number. The story's worlds load the shipped out-of-character content, so a
//!   charter or a character made after it has a number that moves with every room a content
//!   pack adds. An object number is `<oN>`, numbered the same way: the numeric value of a
//!   field that holds a charter, a character or a principal (the list is `OBJECT_FIELDS`),
//!   and the actor of a request.
//!
//! The values are not lost. With `CT0B_RAW_DIR` set, each record is also written there
//! unprojected (`<file>.raw`), and two such runs of one tree, a unit's base and its tip, are
//! compared with `diff`. `docs/design/46-cut-notes.md`, section CT.0b, says how.
//!
//! Nothing is recorded unless the story called [`begin`]: every other test that shares these
//! rigs passes through the hooks untouched.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::{Display, Write as _};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use mush_core::action::PreparedAuthoritativeAction;
use mush_core::authoritative::ReplayWrite;
use mush_core::composed::PreparedComposedTxn;
use mush_core::{AuthoritativeSubmit, DefinitionRef, ModuleBootData, ObjId};
use mush_testkit::snapshot::Snapshot;
use serde_json::Value;
use smol_str::SmolStr;

/// What a rig's prepare returns: the transaction and the provider's products.
pub(super) type Prepared = (PreparedComposedTxn, PreparedAuthoritativeAction);

/// The variable that names a directory for the unprojected record (digests as they are).
const RAW_DIR_ENV: &str = "CT0B_RAW_DIR";

/// Where the committed files live, under the crate.
const DIRECTORY: &str = "src/minds/bulk/ct0b";

/// The namespaces the twins' families own: their writes are recorded with their values.
const OWNED: [&str; 3] = ["economy", "market", "roll"];

/// The namespaces whose documents are read back from the store at an operation's end.
const STORED: [&str; 2] = ["market", "roll"];

/// The JSON fields whose value, when it is a number, is an object's number: a charter, a
/// character, a principal. (`payer` is an account and `org` an organisation: neither is.)
const OBJECT_FIELDS: [&str; 10] = [
    "charter",
    "acting",
    "object",
    "Object",
    "holder",
    "buyer",
    "seller",
    "actor",
    "character",
    "publisher",
];

const TWINS: [(&str, &str); 4] = [
    ("renewal_personal", "the personal renewal twin"),
    ("renewal_house", "the House renewal twin"),
    ("purchase_personal", "the personal purchase twin"),
    ("purchase_house", "the House purchase twin"),
];

/// The digest domain of the dispatched service's submission key, by twin. The purchase twins
/// have no dispatched service.
const SUBMISSION_DOMAINS: [&str; 4] = [
    "market.renewal_submission.v1",
    "market.house_renewal_submission.v1",
    "",
    "",
];

static CAPTURE: Mutex<Option<Capture>> = Mutex::new(None);

enum Item {
    Section(String),
    Line(String, String),
}

#[derive(Default)]
struct Twin {
    items: Vec<Item>,
    /// How many preparations this twin has recorded: a section's number.
    count: usize,
    /// Every `(namespace, key)` under [`STORED`] that a recorded transition of this twin
    /// wrote since the twin's last end.
    keys: BTreeSet<(String, String)>,
}

impl Twin {
    fn section(&mut self, title: String) {
        self.items.push(Item::Section(title));
    }
    fn line(&mut self, label: &str, value: impl Into<String>) {
        self.items.push(Item::Line(label.to_owned(), value.into()));
    }
}

/// The last recorded preparation that succeeded: an install of exactly its writes is its.
struct Pending {
    twin: usize,
    writes: Vec<ReplayWrite>,
    ends: bool,
}

#[derive(Default)]
struct Capture {
    twins: [Twin; 4],
    pending: Option<Pending>,
    /// A recorder failure (a value that would not encode, a key that is not the digest it
    /// should be): reported by [`finish`].
    faults: Vec<String>,
    /// The time the story spent in this recorder's hooks, printed by [`finish`]: what the
    /// record adds to a pinned story.
    spent: Duration,
}

fn with<T>(run: impl FnOnce(&mut Capture) -> T) -> Option<T> {
    let mut guard = match CAPTURE.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let capture = guard.as_mut()?;
    let started = Instant::now();
    let out = run(capture);
    capture.spent += started.elapsed();
    Some(out)
}

/// Which twin an action belongs to, if any: the market's renewal and purchase actions.
fn twin_of(action: &DefinitionRef) -> Option<usize> {
    let key = action.key();
    if key.owner.as_str() != "market" || key.kind.as_str() != "action" {
        return None;
    }
    match key.id.as_str() {
        "draft_mind_renewal"
        | "fund_mind_renewal"
        | "settle_mind_renewal"
        | "refund_mind_renewal"
        | "close_mind_renewal"
        | "close_mind_renewal_service" => Some(0),
        "draft_house_mind_renewal"
        | "fund_house_mind_renewal"
        | "settle_house_mind_renewal"
        | "refund_house_mind_renewal"
        | "close_house_mind_renewal"
        | "close_house_mind_renewal_service" => Some(1),
        // Versions 1 and 2 of the original purchase are the legacy protocol, not a twin.
        "list_original_mind"
        | "quote_original_mind"
        | "purchase_original_mind"
        | "settle_original_mind"
        | "refund_original_mind" => (action.version() == 3).then_some(2),
        "list_derived_mind"
        | "quote_derived_mind"
        | "purchase_derived_mind"
        | "settle_derived_mind"
        | "refund_derived_mind" => Some(2),
        "list_house_mind"
        | "quote_house_mind"
        | "purchase_house_mind"
        | "settle_house_mind"
        | "refund_house_mind" => Some(3),
        _ => None,
    }
}

fn name(action: &DefinitionRef) -> String {
    let key = action.key();
    format!(
        "{}/{}@{}",
        key.owner.as_str(),
        key.id.as_str(),
        action.version()
    )
}

/// Starts the record. The pinned M7 story calls it once, before its first branch.
pub(super) fn begin() {
    let mut guard = match CAPTURE.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    *guard = Some(Capture::default());
}

/// One preparation by a story rig, passed through unchanged. Recorded when the record is
/// running and `action` is one of the twins': the request, then the provider's outcome and
/// the transition's replay writes, or its refusal as printed. `stored` reads the rig's store
/// as it was before the transition (a namespace and a key to the document there).
pub(super) fn seen<E: Display>(
    action: &DefinitionRef,
    actor: ObjId,
    parameters: &Value,
    result: Result<Prepared, E>,
    stored: impl Fn(&str, &str) -> Option<String>,
) -> Result<Prepared, E> {
    let Some(twin) = twin_of(action) else {
        return result;
    };
    with(|capture| {
        let Capture {
            twins,
            pending,
            faults,
            ..
        } = capture;
        *pending = None;
        let slot = &mut twins[twin];
        slot.count += 1;
        let head = format!("{:04} {}", slot.count, name(action));
        match &result {
            Err(refusal) => {
                slot.section(format!("{head}, refused"));
                slot.line("actor", actor.to_string());
                slot.line("parameters", parameters.to_string());
                slot.line("refused", refusal.to_string());
            }
            Ok((prepared, output)) => {
                slot.section(head);
                slot.line("actor", actor.to_string());
                slot.line("parameters", parameters.to_string());
                match serde_json::to_string(output.outcome()) {
                    Ok(outcome) => slot.line("outcome", outcome),
                    Err(error) => faults.push(format!("outcome: {error}")),
                }
                let writes = prepared.authoritative_replay_writes();
                let mut ends = false;
                for write in &writes {
                    match recorded(write, &stored) {
                        Ok(lines) => {
                            for (label, line) in lines {
                                slot.line(label, line);
                            }
                        }
                        Err(error) => faults.push(format!("write: {error}")),
                    }
                    if let Some((namespace, key, family)) = target(write) {
                        ends |= family.is_some_and(|id| id.ends_with("_receipt"));
                        if STORED.contains(&namespace.as_str()) {
                            slot.keys.insert((namespace, key));
                        }
                    }
                }
                *pending = Some(Pending { twin, writes, ends });
            }
        }
    });
    result
}

/// A submission the market's dispatched service queued and the story's rig took from the
/// engine: its actor, its action, its parameters and its idempotency key.
pub(super) fn selected(request: &AuthoritativeSubmit) {
    let Some(twin) = twin_of(&request.action) else {
        return;
    };
    with(|capture| {
        let slot = &mut capture.twins[twin];
        slot.section(format!("the service submits {}", name(&request.action)));
        slot.line("actor", request.actor.to_string());
        slot.line("parameters", request.parameters.to_string());
        let key = match serde_json::to_value(&request.key) {
            Ok(Value::String(key)) => key,
            Ok(other) => other.to_string(),
            Err(error) => {
                capture.faults.push(format!("submission key: {error}"));
                return;
            }
        };
        slot.line("key", key.as_str());
        // The service's own close is keyed by the operation and the action's digest, in the
        // clear. Every other submission is keyed by a digest, which the file cannot hold: so
        // the digest is made again here, from this literal domain, and must be the key.
        if key.starts_with("renewal.close.") {
            return;
        }
        match submission_selection(twin, request, &key) {
            Some(selection) => slot.line(
                "key is",
                format!(
                    "renewal.<the digest of {} over the actor, the action, the parameters \
                     and the selection {selection}>",
                    SUBMISSION_DOMAINS[twin]
                ),
            ),
            None => {
                slot.line("key is", "not the service's submission digest");
                capture.faults.push(format!(
                    "{}: the service's submission key {key} is not `renewal.` and the digest \
                     of {} over the actor, the action, the parameters and a selection",
                    name(&request.action),
                    SUBMISSION_DOMAINS[twin]
                ));
            }
        }
    });
}

/// The selection under which `key` is the service's submission digest for `request`, tried
/// over every selection the service can hold for these parameters: `d/<licence>/<term>` (and
/// with a holder generation) for a due licence, `r/<licence>/<term>/<operation>` for a retry,
/// `p/<operation>` for a pending operation.
fn submission_selection(twin: usize, request: &AuthoritativeSubmit, key: &str) -> Option<String> {
    let domain = SUBMISSION_DOMAINS[twin];
    let reproduces = |selection: &str| {
        mush_core::DefinitionDigest::for_canonical_value(
            domain,
            &serde_json::json!({
                "actor": request.actor,
                "action": request.action,
                "parameters": request.parameters,
                "selection": selection,
            }),
        )
        .is_ok_and(|digest| key == format!("renewal.{}", digest.to_hex()))
    };
    let parameters = &request.parameters;
    let mut candidates = Vec::new();
    if let Some(operation) = parameters.get("operation").and_then(Value::as_str) {
        candidates.push(format!("p/{operation}"));
    }
    if let Some(licence) = parameters.get("licence").and_then(Value::as_u64) {
        for term in 1..=32_u64 {
            candidates.push(format!("d/{licence}/{term}"));
            for holder in 2..=8_u64 {
                candidates.push(format!("d/{licence}/{term}/{holder}"));
            }
        }
    }
    if let Some(previous) = parameters.get("previous_operation").and_then(Value::as_str) {
        for licence in 1..=64_u64 {
            for term in 1..=32_u64 {
                candidates.push(format!("r/{licence}/{term}/{previous}"));
            }
        }
    }
    candidates
        .into_iter()
        .find(|selection| reproduces(selection))
}

/// A story rig installed `writes` and folded them into its store, which `stored` reads (a
/// namespace and a key to the document there). When they are the writes of the last recorded
/// preparation, the transition is marked installed; when it ended an operation (it wrote a
/// receipt), the store's documents under every key the twin has written since its last end
/// are recorded as the store now holds them.
pub(super) fn installed(writes: &[ReplayWrite], stored: impl Fn(&str, &str) -> Option<String>) {
    with(|capture| {
        let Some(pending) = capture.pending.take() else {
            return;
        };
        if pending.writes != writes {
            capture.pending = Some(pending);
            return;
        }
        let slot = &mut capture.twins[pending.twin];
        slot.line("installed", "yes");
        if !pending.ends {
            return;
        }
        for (namespace, key) in std::mem::take(&mut slot.keys) {
            if let Some(document) = stored(&namespace, &key) {
                slot.line(&format!("stored {namespace}/{key}"), document);
            }
        }
    });
}

/// A reader over a rig that keeps its store as the boot image of each module.
pub(super) fn image_reader(
    image: &HashMap<SmolStr, ModuleBootData>,
) -> impl Fn(&str, &str) -> Option<String> + '_ {
    |namespace, key| {
        let data = image.get(namespace)?;
        data.kv
            .iter()
            .find(|(name, _)| name.as_str() == key)
            .map(|(_, value)| value.to_string())
            .or_else(|| {
                data.ledger
                    .iter()
                    .find(|(name, _)| name.as_str() == key)
                    .map(|(_, text)| text.clone())
            })
    }
}

/// A reader over a rig that keeps its store as one map of documents.
pub(super) fn disk_reader(
    disk: &BTreeMap<(String, String), Value>,
) -> impl Fn(&str, &str) -> Option<String> + '_ {
    |namespace, key| {
        disk.get(&(namespace.to_owned(), key.to_owned()))
            .map(Value::to_string)
    }
}

/// A write's namespace and key, and its family id when it is a typed family row.
fn target(write: &ReplayWrite) -> Option<(String, String, Option<String>)> {
    match write {
        ReplayWrite::ModuleJsonSet {
            namespace,
            key,
            value,
        } => Some((
            namespace.to_string(),
            key.to_string(),
            value
                .pointer("/family/id")
                .and_then(Value::as_str)
                .map(str::to_owned),
        )),
        ReplayWrite::ModuleDelete { namespace, key } => {
            Some((namespace.to_string(), key.to_string(), None))
        }
        ReplayWrite::LedgerChunkSet { write } => {
            Some((write.namespace().to_string(), write.key().to_string(), None))
        }
        ReplayWrite::ObjectSet { .. }
        | ReplayWrite::ObjectDelete { .. }
        | ReplayWrite::ObjectAllocatorSet { .. } => None,
    }
}

/// One replay write as the record holds it, a label and a value a line. A write a twin's
/// family owns is the tagged JSON a bundle carries; a journal chunk is the entries this
/// transition appended to what `stored` held; a write another owner owns is its kind,
/// namespace and key alone.
fn recorded(
    write: &ReplayWrite,
    stored: &impl Fn(&str, &str) -> Option<String>,
) -> Result<Vec<(&'static str, String)>, String> {
    let Some((namespace, key, _)) = target(write) else {
        return Ok(vec![(
            "write",
            match write {
                ReplayWrite::ObjectSet { .. } => "an object set, not the twins'",
                ReplayWrite::ObjectDelete { .. } => "an object delete, not the twins'",
                _ => "an object allocator set, not the twins'",
            }
            .to_owned(),
        )]);
    };
    if !OWNED.contains(&namespace.as_str()) {
        let kind = match write {
            ReplayWrite::ModuleDelete { .. } => "module_delete",
            ReplayWrite::LedgerChunkSet { .. } => "ledger_chunk_set",
            _ => "module_json_set",
        };
        return Ok(vec![(
            "write",
            format!("{kind} {namespace}/{key}, not the twins': its value is not recorded"),
        )]);
    }
    let ReplayWrite::LedgerChunkSet { write: chunk } = write else {
        return Ok(vec![(
            "write",
            serde_json::to_string(write).map_err(|error| error.to_string())?,
        )]);
    };
    let after = serde_json::to_value(chunk.chunk()).map_err(|error| error.to_string())?;
    let entries = after
        .get("entries")
        .and_then(Value::as_array)
        .ok_or("a journal chunk without entries")?;
    let before: Vec<Value> = stored(&namespace, &key)
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|chunk| chunk.get("entries").and_then(Value::as_array).cloned())
        .unwrap_or_default();
    if entries.len() < before.len() || entries.iter().zip(&before).any(|(new, old)| new != old) {
        return Err(format!(
            "{namespace}/{key}: the chunk does not extend the one in the store"
        ));
    }
    let mut lines = vec![(
        "write",
        format!(
            "ledger_chunk_set {namespace}/{key}: {} entries appended after {}, sealed {}",
            entries.len() - before.len(),
            before.len(),
            after.get("sealed").and_then(Value::as_bool) == Some(true)
        ),
    )];
    for entry in entries.iter().skip(before.len()) {
        lines.push(("appended", entry.to_string()));
    }
    Ok(lines)
}

/// The projection of one section: digests and object numbers by first appearance.
#[derive(Default)]
struct Names {
    digests: BTreeMap<String, usize>,
    objects: BTreeMap<String, usize>,
}

impl Names {
    fn digest(&mut self, hex: String) -> String {
        let next = self.digests.len() + 1;
        format!("<d{}>", *self.digests.entry(hex).or_insert(next))
    }
    fn object(&mut self, number: &str) -> String {
        let next = self.objects.len() + 1;
        format!(
            "<o{}>",
            *self.objects.entry(number.to_owned()).or_insert(next)
        )
    }

    /// `text` with every digest and every object number replaced by its name.
    fn project(&mut self, text: &str) -> String {
        let text = self.without_digests(text);
        self.without_objects(&text)
    }

    /// A 64-digit hex string, and an array of 32 bytes, is a digest. A longer or shorter run
    /// of hex digits (a commit token is 32) is kept.
    fn without_digests(&mut self, text: &str) -> String {
        let bytes = text.as_bytes();
        let hex = |byte: u8| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte);
        let mut out = String::with_capacity(text.len());
        let mut at = 0;
        let mut copied = 0;
        while at < bytes.len() {
            if bytes[at] == b'[' {
                if let Some((end, digest)) = byte_array(bytes, at) {
                    out.push_str(text.get(copied..at).unwrap_or_default());
                    out.push_str(&self.digest(digest));
                    at = end;
                    copied = end;
                    continue;
                }
                at += 1;
            } else if hex(bytes[at]) {
                let start = at;
                while at < bytes.len() && hex(bytes[at]) {
                    at += 1;
                }
                if at - start == 64 {
                    out.push_str(text.get(copied..start).unwrap_or_default());
                    let digest = text.get(start..at).unwrap_or_default().to_owned();
                    out.push_str(&self.digest(digest));
                    copied = at;
                }
            } else {
                at += 1;
            }
        }
        out.push_str(text.get(copied..).unwrap_or_default());
        out
    }

    /// The value of an [`OBJECT_FIELDS`] field, a number or a number in quotes, in JSON or in
    /// JSON quoted inside a JSON string.
    fn without_objects(&mut self, text: &str) -> String {
        let bytes = text.as_bytes();
        let mut out = String::with_capacity(text.len());
        let mut at = 0;
        let mut copied = 0;
        while at < bytes.len() {
            if bytes[at] != b'"' {
                at += 1;
                continue;
            }
            at += 1;
            let Some(field) = OBJECT_FIELDS
                .iter()
                .find(|field| bytes.get(at..at + field.len()) == Some(field.as_bytes()))
            else {
                continue;
            };
            // The closing quote of the name (escaped or not), the colon, an opening quote.
            let mut cursor = at + field.len();
            let quotes = |cursor: &mut usize| {
                let from = *cursor;
                while matches!(bytes.get(*cursor), Some(b'"' | b'\\')) {
                    *cursor += 1;
                }
                *cursor > from
            };
            if !quotes(&mut cursor) || bytes.get(cursor) != Some(&b':') {
                continue;
            }
            cursor += 1;
            quotes(&mut cursor);
            let start = cursor;
            while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
                cursor += 1;
            }
            if cursor == start {
                continue;
            }
            out.push_str(text.get(copied..start).unwrap_or_default());
            let number = text.get(start..cursor).unwrap_or_default();
            out.push_str(&self.object(number));
            copied = cursor;
            at = cursor;
        }
        out.push_str(text.get(copied..).unwrap_or_default());
        out
    }
}

/// The array of exactly 32 bytes that starts at `bytes[at]` (a `[`): where it ends and its
/// bytes as hex, the same text a digest written as hex has.
fn byte_array(bytes: &[u8], at: usize) -> Option<(usize, String)> {
    let mut cursor = at + 1;
    let mut hex = String::with_capacity(64);
    for index in 0..32 {
        let start = cursor;
        let mut value: u32 = 0;
        while let Some(digit) = bytes.get(cursor).filter(|byte| byte.is_ascii_digit()) {
            value = value * 10 + u32::from(digit - b'0');
            cursor += 1;
            if cursor - start > 3 {
                return None;
            }
        }
        if cursor == start || value > 255 {
            return None;
        }
        let _ = write!(hex, "{value:02x}");
        let closes = if index == 31 { b']' } else { b',' };
        if bytes.get(cursor) != Some(&closes) {
            return None;
        }
        cursor += 1;
    }
    Some((cursor, hex))
}

fn render(twin: usize, slot: &Twin, raw: bool) -> Snapshot {
    let (file, title) = TWINS[twin];
    let mut snapshot = Snapshot::new();
    snapshot.section("doc 46 unit CT.0b");
    snapshot.line("what", &format!("{title} on the composed path"));
    snapshot.line(
        "written by",
        "the M7 story genuine_registry_original_purchase_grant_refund_and_paired_recovery",
    );
    snapshot.line(
        "blessed with",
        "COMMERCE_SNAPSHOT_BLESS=1 on that story, once, by the unit that added the file",
    );
    snapshot.line(
        "projection",
        if raw {
            "none: this is the unprojected record, which is never committed"
        } else {
            "<dN> is a digest and <oN> an object's number, each numbered by first appearance \
             in its section"
        },
    );
    snapshot.line("file", file);
    let mut names = Names::default();
    for item in &slot.items {
        match item {
            Item::Section(title) => {
                names = Names::default();
                snapshot.section(title);
            }
            Item::Line(label, value) if raw => snapshot.line(label, value),
            Item::Line(label, value) if label == "actor" => {
                let number = value.trim_start_matches('#');
                snapshot.line(label, &format!("#{}", names.object(number)));
            }
            Item::Line(label, value) => {
                let label = names.project(label);
                let value = names.project(value);
                snapshot.line(&label, &value);
            }
        }
    }
    snapshot
}

/// Ends the record and compares each twin's with its committed file. The pinned M7 story
/// calls it last, after every branch passed.
pub(super) fn finish() -> Result<(), String> {
    let capture = {
        let mut guard = match CAPTURE.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard.take()
    };
    let Some(capture) = capture else {
        return Err("the CT.0b record was not begun".to_owned());
    };
    let comparing = Instant::now();
    let mut failures = capture.faults;
    let raw_dir = std::env::var(RAW_DIR_ENV)
        .ok()
        .filter(|dir| !dir.is_empty());
    for (twin, slot) in capture.twins.iter().enumerate() {
        let (file, _) = TWINS[twin];
        let relative = format!("{DIRECTORY}/{file}.snap");
        eprintln!(
            "CT0B_CAPTURE file={file} preparations={} lines={}",
            slot.count,
            slot.items.len()
        );
        if slot.count == 0 {
            failures.push(format!("{file}: the story prepared none of its actions"));
            continue;
        }
        if let Some(dir) = &raw_dir {
            // The unprojected record, and beside it this run's projected one (what the
            // committed file is compared with), so that two runs can be compared by `diff`.
            for (extension, raw) in [("raw", true), ("snap", false)] {
                let path = std::path::Path::new(dir).join(format!("{file}.{extension}"));
                let text = render(twin, slot, raw).render(&relative);
                let written =
                    std::fs::create_dir_all(dir).and_then(|()| std::fs::write(&path, text));
                if let Err(error) = written {
                    failures.push(format!("{}: {error}", path.display()));
                }
            }
        }
        if let Err(report) = render(twin, slot, false).check(env!("CARGO_MANIFEST_DIR"), &relative)
        {
            failures.push(report);
        }
    }
    eprintln!(
        "CT0B_CAPTURE recorder_ms={} compare_ms={}",
        capture.spent.as_millis(),
        comparing.elapsed().as_millis()
    );
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_digest_and_an_object_number_are_named_by_first_appearance() {
        let digest = "ab".repeat(32);
        let other = "cd".repeat(32);
        let token = "0".repeat(32);
        let hash: Vec<String> = (0..32).map(|_| "171".to_owned()).collect();
        let text = format!(
            r#"{{"a":"{digest}","b":[{}],"c":"{other}","token":"{token}","charter":"104","acting":105,"note":"{{\"charter\":104,\"object\":7}}","price":104,"list":[1,2,3],"Object":104,"holder":105,"buyer":{{"acting":105,"org":104}},"payer":104}}"#,
            hash.join(",")
        );
        let mut names = Names::default();
        assert_eq!(
            names.project(&text),
            format!(
                r#"{{"a":"<d1>","b":<d1>,"c":"<d2>","token":"{token}","charter":"<o1>","acting":<o2>,"note":"{{\"charter\":<o1>,\"object\":<o3>}}","price":104,"list":[1,2,3],"Object":<o1>,"holder":<o2>,"buyer":{{"acting":<o2>,"org":104}},"payer":104}}"#
            )
        );
    }
}
