//! World-side input tape ownership, asynchronous writes, retention and admission.
//! Kept payloads stay encoded; only an admitted private bench decodes events.
use super::MatchId;
use crate::arena::input_tape::*;
use crate::minds::resimulation::VerifiedTape;
use mush_core::{Ctx, ObjId, World};
use mush_roll::{ReferenceKey, ReferenceKind, RollStore};
use smol_str::SmolStr;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub const DEFAULT_RETAINED_BYTES: u64 = 128 * 1024 * 1024;
pub const MAX_RETAINED_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_RECORDS: usize = super::HISTORY_DEPTH + 1;
// Recorder failures already cap their reason at 240 characters. Reserve the
// complete encoded marker per entry, including an entry with no retained head.
const MARKER_BYTES: u64 = 4096;
const REASON_CHARS: usize = 240;
// The recorder flushes before adding an event that crosses CHUNK_TARGET. Allow
// one maximum event plus its bounded envelope without decoding a huge array.
const CHUNK_DOCUMENT_BYTES: u64 = (MAX_EVENT_BYTES + CHUNK_TARGET) as u64 + MARKER_BYTES;
pub fn key(id: MatchId) -> SmolStr {
    format!("tacsim:tape:{}", id.0).into()
}
fn head_key(id: MatchId) -> SmolStr {
    format!("tacsim:tape:{}:head", id.0).into()
}
fn chunk_key(id: MatchId, seq: u32) -> SmolStr {
    format!("tacsim:tape:{}:chunk:{seq}", id.0).into()
}
fn root_key(id: MatchId) -> ReferenceKey {
    ReferenceKey {
        owner: crate::store::NS.into(),
        kind: ReferenceKind::Tape,
        record: format!("match/{}", id.0),
    }
}
#[derive(Clone, Debug)]
struct Entry {
    head: Option<Arc<[u8]>>,
    chunks: Vec<Arc<[u8]>>,
    mark: TapeMark,
    bytes: u64,
    owners: BTreeSet<ObjId>,
    pin: Arc<()>,
    retired: bool,
    limit: u64,
}
impl Entry {
    fn unavailable(id: MatchId, reason: String) -> Self {
        Self {
            head: None,
            chunks: Vec::new(),
            mark: TapeMark {
                format: TAPE_FORMAT,
                match_id: id.0,
                cursor: TapeCursor::default(),
                bytes: 0,
                status: TapeStatus::Incomplete { reason },
            },
            bytes: 0,
            owners: BTreeSet::new(),
            pin: Arc::new(()),
            retired: false,
            limit: DEFAULT_MAX_BYTES,
        }
    }
}
/// Module-owned bounded cache, restored from exactly the kept/live document set.
#[derive(Clone, Debug)]
pub struct TapeStore {
    entries: BTreeMap<MatchId, Entry>,
    limit: u64,
    // Encoded head/chunks plus one fixed marker reservation for each entry.
    resident: u64,
}
impl Default for TapeStore {
    fn default() -> Self {
        Self {
            entries: BTreeMap::new(),
            limit: DEFAULT_RETAINED_BYTES,
            resident: 0,
        }
    }
}
#[derive(Clone, Debug)]
pub struct TapeLease {
    pub tape: Arc<VerifiedTape>,
    artifact: [u8; 32],
    bytes: u64,
    _pin: Arc<()>,
}
impl TapeLease {
    pub fn artifact_digest(&self) -> [u8; 32] {
        self.artifact
    }
    pub fn encoded_bytes(&self) -> u64 {
        self.bytes
    }
    pub fn owns(&self, actor: ObjId, unit: ObjId) -> bool {
        self.tape.header().seals.iter().any(|seal| {
            seal.unit == unit
                && seal
                    .principal
                    .as_ref()
                    .is_some_and(|owner| matches!(owner, crate::deliberation::order::CharacterRef::Character { id } if *id == actor))
        })
    }
}
#[derive(Clone, Debug)]
pub struct ResumeTape {
    pub limit: u64,
    pub bytes: u64,
    pub cursor: TapeCursor,
}

impl TapeStore {
    pub fn new(limit: u64) -> Result<Self, TapeError> {
        if !(MIN_MAX_BYTES..=MAX_RETAINED_BYTES).contains(&limit) {
            return Err(TapeError::Unavailable(
                "tape_retained_max_bytes must be4 to1024MiB".into(),
            ));
        }
        Ok(Self {
            limit,
            ..Self::default()
        })
    }
    pub fn resident_bytes(&self) -> u64 {
        self.resident
    }
    pub fn resident_limit(&self) -> u64 {
        self.limit
    }
    pub fn status(&self, id: MatchId) -> String {
        self.entries.get(&id).map_or_else(
            || "Bench recording unavailable (not retained).".into(),
            |entry| match &entry.mark.status {
                TapeStatus::Recording => "Bench recording in progress.".into(),
                TapeStatus::Complete { .. } => {
                    format!("Bench recording complete ({} bytes).", entry.bytes)
                }
                TapeStatus::Incomplete { reason } => {
                    format!("Bench recording incomplete: {reason}.")
                }
            },
        )
    }
    fn removable(&self, id: MatchId) -> bool {
        self.entries.get(&id).is_some_and(|e| {
            Arc::strong_count(&e.pin) == 1 && !matches!(e.mark.status, TapeStatus::Recording)
        })
    }
    fn victim(&self, except: MatchId) -> Option<MatchId> {
        self.entries
            .keys()
            .copied()
            .find(|id| *id != except && self.removable(*id))
    }
    fn checkout_for(&self, id: MatchId, actor: ObjId) -> Result<TapeCheckout, TapeError> {
        let entry = self
            .entries
            .get(&id)
            .ok_or_else(|| missing("not retained"))?;
        if entry.retired || !entry.owners.contains(&actor) {
            return Err(missing("that character has no retained hull recording"));
        }
        if !matches!(entry.mark.status, TapeStatus::Complete { .. }) {
            return Err(missing(self.status(id)));
        }
        Ok(TapeCheckout {
            head: Arc::clone(
                entry
                    .head
                    .as_ref()
                    .ok_or_else(|| missing("opening is missing"))?,
            ),
            chunks: entry.chunks.clone(),
            mark: entry.mark.clone(),
            artifact: hash_value(b"tacsim-tape-artifact-v1", &entry.mark)?,
            pin: Arc::clone(&entry.pin),
        })
    }
    fn apply(&mut self, id: MatchId, write: &TapeWrite) -> Result<(), TapeError> {
        match write {
            TapeWrite::Open(header) => {
                header.validate_shape()?;
                if header.match_id != id.0
                    || self.entries.contains_key(&id)
                    || self.entries.len() >= MAX_RECORDS
                {
                    return Err(missing("opening identity or retained-record bound"));
                }
                crate::arena::snapshot_capacity::json_len(
                    &**header,
                    header.limit.saturating_sub(MARKER_BYTES),
                )
                .map_err(|_| missing("opening exceeds recording byte bound"))?;
                let bytes = encode(&**header)?;
                let len = bytes.len() as u64;
                if len.saturating_add(MARKER_BYTES) > header.limit
                    || self
                        .resident
                        .saturating_add(len)
                        .saturating_add(MARKER_BYTES)
                        > self.limit
                {
                    return Err(missing("resident recording byte bound is full or pinned"));
                }
                let cursor = header.cursor()?;
                let owners = header
                    .seals
                    .iter()
                    .filter_map(|seal| match seal.principal.as_ref() {
                        Some(crate::deliberation::order::CharacterRef::Character { id }) => {
                            Some(*id)
                        }
                        _ => None,
                    })
                    .collect();
                self.entries.insert(
                    id,
                    Entry {
                        head: Some(bytes.into()),
                        chunks: Vec::new(),
                        mark: TapeMark {
                            format: TAPE_FORMAT,
                            match_id: id.0,
                            cursor,
                            bytes: len,
                            status: TapeStatus::Recording,
                        },
                        bytes: len,
                        owners,
                        pin: Arc::new(()),
                        retired: false,
                        limit: header.limit,
                    },
                );
                self.resident += len + MARKER_BYTES;
            }
            TapeWrite::Chunk(chunk) => {
                let entry = self
                    .entries
                    .get_mut(&id)
                    .ok_or_else(|| missing("input chunk precedes opening"))?;
                if !matches!(entry.mark.status, TapeStatus::Recording) {
                    return Err(missing("recording is already closed"));
                }
                crate::arena::snapshot_capacity::json_len(&**chunk, CHUNK_DOCUMENT_BYTES)
                    .map_err(|_| missing("input chunk exceeds encoded byte bound"))?;
                let cursor = chunk.after(entry.mark.cursor)?;
                let bytes = encode(&**chunk)?;
                let len = bytes.len() as u64;
                if len > CHUNK_DOCUMENT_BYTES
                    || entry.bytes.saturating_add(len).saturating_add(MARKER_BYTES) > entry.limit
                    || self.resident.saturating_add(len) > self.limit
                {
                    return Err(missing("recording or resident byte bound reached"));
                }
                entry.chunks.push(bytes.into());
                entry.bytes += len;
                self.resident += len;
                entry.mark.cursor = cursor;
                entry.mark.bytes = entry.bytes;
            }
            TapeWrite::Mark(mark) => {
                validate_mark(mark)?;
                if mark.match_id != id.0 || mark.format != TAPE_FORMAT {
                    return Err(missing("recording marker identity"));
                }
                if !self.entries.contains_key(&id) {
                    if self.entries.len() >= MAX_RECORDS
                        || self.resident.saturating_add(MARKER_BYTES) > self.limit
                    {
                        return Err(missing("retained marker bound"));
                    }
                    self.resident += MARKER_BYTES;
                }
                let entry = self
                    .entries
                    .entry(id)
                    .or_insert_with(|| Entry::unavailable(id, "opening not retained".into()));
                // Incomplete always wins; a later completion cannot heal a gap.
                if matches!(mark.status, TapeStatus::Incomplete { .. }) {
                    entry.mark.status = mark.status.clone();
                    return Ok(());
                }
                if !matches!(entry.mark.status, TapeStatus::Recording)
                    || entry.mark.cursor != mark.cursor
                    || entry.bytes != mark.bytes
                {
                    return Err(missing(
                        "completion/checkpoint cursor differs from durable input prefix",
                    ));
                }
                entry.mark = mark.clone();
            }
        }
        Ok(())
    }
    fn fail(&mut self, id: MatchId, why: String) -> TapeMark {
        let why: String = why.chars().take(REASON_CHARS).collect();
        if !self.entries.contains_key(&id) {
            if self.entries.len() >= MAX_RECORDS
                || self.resident.saturating_add(MARKER_BYTES) > self.limit
            {
                return Entry::unavailable(id, why).mark;
            }
            self.resident += MARKER_BYTES;
        }
        let entry = self
            .entries
            .entry(id)
            .or_insert_with(|| Entry::unavailable(id, why.clone()));
        entry.mark.status = TapeStatus::Incomplete { reason: why };
        entry.mark.clone()
    }
    fn remove(&mut self, id: MatchId) -> Option<Entry> {
        let entry = self.entries.remove(&id)?;
        self.resident = self.resident.saturating_sub(entry.bytes + MARKER_BYTES);
        Some(entry)
    }
}

/// Cheap world-loop admission token. Verification, decoding, source compilation
/// and private-arena construction belong to the bounded bulk preparation worker.
#[derive(Clone, Debug)]
pub struct TapeCheckout {
    head: Arc<[u8]>,
    chunks: Vec<Arc<[u8]>>,
    mark: TapeMark,
    artifact: [u8; 32],
    pin: Arc<()>,
}
impl TapeCheckout {
    pub fn artifact_digest(&self) -> [u8; 32] {
        self.artifact
    }
    pub fn encoded_bytes(&self) -> u64 {
        self.mark.bytes
    }
    pub fn verify(self) -> Result<TapeLease, TapeError> {
        self.verify_with(None)
    }
    /// Preparation-worker cache seam. Only an already verified immutable tape
    /// with this exact complete artifact identity can reuse the checkout's pin.
    pub fn verify_with(self, existing: Option<Arc<VerifiedTape>>) -> Result<TapeLease, TapeError> {
        if let Some(tape) = existing {
            if tape.artifact_digest() != self.artifact || tape.encoded_bytes() != self.mark.bytes {
                return Err(missing(
                    "cached tape identity differs from admitted recording",
                ));
            }
            return Ok(TapeLease {
                tape,
                artifact: self.artifact,
                bytes: self.mark.bytes,
                _pin: self.pin,
            });
        }
        let header: TapeHeader =
            serde_json::from_slice(&self.head).map_err(|_| missing("opening is unreadable"))?;
        let chunks: Result<Vec<Arc<TapeChunk>>, _> = self
            .chunks
            .iter()
            .map(|bytes| serde_json::from_slice(bytes).map(Arc::new))
            .collect();
        let bytes = self.mark.bytes;
        let tape = VerifiedTape::verify(
            Arc::new(header),
            &chunks.map_err(|_| missing("input chunk is unreadable"))?,
            self.mark,
        )?;
        Ok(TapeLease {
            tape: Arc::new(tape),
            artifact: self.artifact,
            bytes,
            _pin: self.pin,
        })
    }
}
/// Character authority is checked before retaining a pin; account quota and
/// preparation-worker admission are separate. This performs no JSON decoding.
pub fn checkout(w: &World, id: MatchId, actor: ObjId) -> Result<TapeCheckout, TapeError> {
    let store = w
        .module_state::<TapeStore>()
        .ok_or_else(|| missing("recording service unavailable"))?;
    store.checkout_for(id, actor)
}

/// Called by ArenaOut::Tape on the world's ordinary persistence lane. Whole
/// mutations are validated before serialization/writes; live arenas never wait.
pub(crate) fn accept(ctx: &mut Ctx<'_>, id: MatchId, write: TapeWrite) {
    maintain(ctx);
    if !ctx
        .w
        .module_state::<super::MatchBook>()
        .and_then(|book| book.get(id))
        .is_some_and(|record| record.state.is_live())
    {
        return;
    }
    let added = match &write {
        TapeWrite::Open(h) => {
            encoded_len(&**h).map(|bytes| bytes.saturating_add(MARKER_BYTES as usize))
        }
        TapeWrite::Chunk(c) => encoded_len(&**c),
        TapeWrite::Mark(_) => Ok(
            if ctx
                .w
                .module_state::<TapeStore>()
                .is_some_and(|store| store.entries.contains_key(&id))
            {
                0
            } else {
                MARKER_BYTES as usize
            },
        ),
    }
    .unwrap_or(usize::MAX) as u64;
    loop {
        let victim = ctx.w.module_state::<TapeStore>().and_then(|store| {
            (store.resident.saturating_add(added) > store.limit
                || (matches!(&write, TapeWrite::Open(_)) && store.entries.len() >= MAX_RECORDS))
                .then(|| store.victim(id))
                .flatten()
        });
        let Some(victim) = victim else {
            break;
        };
        if evict(ctx, victim).is_err() {
            break;
        }
    }
    let result = (|| {
        if let TapeWrite::Open(header) = &write {
            // Validate the entire admission before replacing durable roots. A
            // duplicate/malformed opening must not change an existing tape's roots.
            let mut prepared_store = ctx
                .w
                .module_state::<TapeStore>()
                .ok_or_else(|| missing("recording service unavailable"))?
                .clone();
            prepared_store.apply(id, &write)?;
            let refs = super::selection_refs::revisions(header.seals.iter());
            if !refs.is_empty() {
                let prepared = ctx
                    .w
                    .module_state::<RollStore>()
                    .ok_or_else(|| missing("source retention owner unavailable"))?
                    .prepare_reference_batch(crate::store::NS, vec![(root_key(id), refs)], vec![])
                    .map_err(|e| missing(e.to_string()))?;
                // Root publication is conservative if the following store bound
                // changed; failure retains extra roots, never loses sources.
                super::selection_refs::publish(ctx, prepared)
                    .map_err(|e| missing(e.to_string()))?;
            }
            *ctx.w
                .module_state_mut::<TapeStore>()
                .ok_or_else(|| missing("recording service unavailable"))? = prepared_store;
            return Ok(());
        }
        ctx.w
            .module_state_mut::<TapeStore>()
            .ok_or_else(|| missing("recording service unavailable"))?
            .apply(id, &write)
    })();
    if let Err(error) = result {
        tracing::warn!(%id,%error,"bench recording became incomplete; live play continues");
        if let Some(store) = ctx.w.module_state_mut::<TapeStore>() {
            let mark = store.fail(id, error.to_string());
            persist(ctx, key(id), &mark);
        }
        return;
    }
    match write {
        TapeWrite::Open(header) => persist(ctx, head_key(id), &*header),
        TapeWrite::Chunk(chunk) => persist(ctx, chunk_key(id, chunk.seq), &*chunk),
        TapeWrite::Mark(mark) => persist(ctx, key(id), &mark),
    }
}
fn persist(ctx: &mut Ctx<'_>, key: SmolStr, value: &impl serde::Serialize) {
    if let Ok(value) = serde_json::to_value(value) {
        crate::store::push_op(ctx.out, mush_core::ModuleWrite::JsonSet { key, value });
    }
}
fn release_root(ctx: &mut Ctx<'_>, id: MatchId) -> Result<(), TapeError> {
    let Some(owner) = ctx.w.module_state::<RollStore>() else {
        return Ok(());
    };
    let key = root_key(id);
    if !owner
        .reference_keys(crate::store::NS, ReferenceKind::Tape)
        .contains(&key)
    {
        return Ok(());
    }
    let prepared = owner
        .prepare_reference_batch(crate::store::NS, vec![], vec![key])
        .map_err(|e| missing(e.to_string()))?;
    super::selection_refs::publish(ctx, prepared).map_err(|e| missing(e.to_string()))
}
/// History eviction and resident-pressure eviction share exact source cleanup.
/// An open bench pin postpones deletion; maintenance retries after it is dropped.
pub(crate) fn evict(ctx: &mut Ctx<'_>, id: MatchId) -> Result<(), TapeError> {
    if let Some(store) = ctx.w.module_state_mut::<TapeStore>()
        && let Some(entry) = store.entries.get_mut(&id)
    {
        entry.retired = true;
        if Arc::strong_count(&entry.pin) > 1 {
            return Err(missing("recording is pinned by a bench"));
        }
    }
    release_root(ctx, id)?;
    let entry = ctx
        .w
        .module_state_mut::<TapeStore>()
        .and_then(|s| s.remove(id));
    if let Some(entry) = entry {
        for key in std::iter::once(key(id))
            .chain(std::iter::once(head_key(id)))
            .chain((0..entry.chunks.len()).map(|n| chunk_key(id, n as u32)))
        {
            crate::store::push_op(ctx.out, mush_core::ModuleWrite::Del { key });
        }
    }
    Ok(())
}
pub(crate) fn maintain(ctx: &mut Ctx<'_>) {
    let stale = ctx
        .w
        .module_state::<TapeStore>()
        .map(|s| {
            s.entries
                .iter()
                .filter_map(|(id, e)| (e.retired && Arc::strong_count(&e.pin) == 1).then_some(*id))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for id in stale {
        if let Err(error) = evict(ctx, id) {
            tracing::warn!(%id,%error,"tape roots retained pending cleanup");
        }
    }
}

/// A terminal/boot-aborted match with no complete recording cannot leave an
/// apparently resumable in-progress marker behind.
pub(crate) fn close_incomplete(ctx: &mut Ctx<'_>, id: MatchId, reason: &str) {
    let mark = ctx.w.module_state_mut::<TapeStore>().and_then(|store| {
        store
            .entries
            .get(&id)
            .is_some_and(|e| matches!(e.mark.status, TapeStatus::Recording))
            .then(|| store.fail(id, reason.to_owned()))
    });
    if let Some(mark) = mark {
        persist(ctx, key(id), &mark);
    }
}

/// Resume from the snapshot's exact prefix. A missing cursor/chunk fails only
/// recording. A durable tail beyond a snapshot is removed before appending the
/// restarted branch; otherwise the same sequence number could have two meanings.
pub(crate) fn resume(
    ctx: &mut Ctx<'_>,
    id: MatchId,
    cursor: Option<TapeCursor>,
) -> Option<ResumeTape> {
    let result = (|| {
        let cursor = cursor.ok_or_else(|| missing("snapshot has no input cursor"))?;
        let store = ctx
            .w
            .module_state_mut::<TapeStore>()
            .ok_or_else(|| missing("recording service unavailable"))?;
        let entry = store
            .entries
            .get_mut(&id)
            .ok_or_else(|| missing("opening missing at resume"))?;
        if !matches!(entry.mark.status, TapeStatus::Recording) {
            return Err(missing("recording already incomplete or completed"));
        }
        let header: TapeHeader = serde_json::from_slice(
            entry
                .head
                .as_ref()
                .ok_or_else(|| missing("opening missing"))?,
        )
        .map_err(|_| missing("opening unreadable"))?;
        let mut actual = header.cursor()?;
        let mut bytes = encoded_len(&header)? as u64;
        if cursor.chunks as usize > entry.chunks.len() {
            return Err(missing("durable input cursor is behind snapshot"));
        }
        for raw in entry.chunks.iter().take(cursor.chunks as usize) {
            let chunk: TapeChunk =
                serde_json::from_slice(raw).map_err(|_| missing("input chunk unreadable"))?;
            actual = chunk.after(actual)?;
            bytes += raw.len() as u64;
        }
        if actual != cursor {
            return Err(missing("snapshot and durable input cursor differ"));
        }
        let old_count = entry.chunks.len();
        store.resident = store.resident.saturating_sub(entry.bytes - bytes);
        entry.bytes = bytes;
        entry.chunks.truncate(cursor.chunks as usize);
        entry.mark.cursor = cursor;
        entry.mark.bytes = bytes;
        Ok((
            ResumeTape {
                limit: header.limit,
                bytes,
                cursor,
            },
            old_count,
            entry.mark.clone(),
        ))
    })();
    match result {
        Ok((resume, old_count, mark)) => {
            for n in resume.cursor.chunks as usize..old_count {
                crate::store::push_op(
                    ctx.out,
                    mush_core::ModuleWrite::Del {
                        key: chunk_key(id, n as u32),
                    },
                );
            }
            persist(ctx, key(id), &mark);
            Some(resume)
        }
        Err(error) => {
            if let Some(store) = ctx.w.module_state_mut::<TapeStore>() {
                let mark = store.fail(id, error.to_string());
                persist(ctx, key(id), &mark);
            }
            None
        }
    }
}

/// Boot loader, called with the centre's bounded history plus live match id.
/// Missing/unknown/tampered documents become explicit incomplete markers. Source
/// roots are never guessed away when any retained record cannot be decoded.
pub(crate) fn restore(
    ctx: &mut Ctx<'_>,
    docs: &[(SmolStr, serde_json::Value)],
    keep: &BTreeSet<MatchId>,
) {
    // Index borrowed JSON documents, never decoded whole tapes. Newer kept
    // records claim the resident budget first; older excess records fail closed.
    let mut ordered = BTreeMap::new();
    let mut malformed = BTreeSet::new();
    let mut observed = BTreeSet::new();
    let mut obsolete = BTreeMap::<MatchId, Vec<SmolStr>>::new();
    for (name, value) in docs {
        let Some(tail) = name.strip_prefix("tacsim:tape:") else {
            continue;
        };
        let mut words = tail.split(':');
        let Some(id) = words
            .next()
            .and_then(|n| n.parse::<u64>().ok())
            .map(MatchId)
        else {
            continue;
        };
        if !keep.contains(&id) {
            obsolete.entry(id).or_default().push(name.clone());
            continue;
        }
        observed.insert(id);
        let part = match words.next() {
            None => Some((2u8, 0u32)),
            Some("head") if words.next().is_none() => Some((0, 0)),
            Some("chunk") => words
                .next()
                .and_then(|s| s.parse::<u32>().ok())
                .filter(|n| (*n as usize) < MAX_CHUNKS && words.next().is_none())
                .map(|n| (1, n)),
            _ => None,
        };
        if let Some((phase, seq)) = part {
            if ordered
                .insert((std::cmp::Reverse(id), phase, seq), value)
                .is_some()
            {
                malformed.insert(id);
            }
        } else {
            malformed.insert(id);
        }
    }
    for ((std::cmp::Reverse(id), phase, seq), value) in ordered {
        let parsed = decode_boot_document(phase, seq, value).ok();
        if let Some(write) = parsed {
            if let Some(store) = ctx.w.module_state_mut::<TapeStore>()
                && let Err(error) = store.apply(id, &write)
            {
                store.fail(id, error.to_string());
            }
        } else {
            malformed.insert(id);
        }
    }
    for id in observed {
        let head = ctx
            .w
            .module_state::<TapeStore>()
            .and_then(|s| s.entries.get(&id))
            .and_then(|e| e.head.clone());
        let result = (|| {
            if malformed.contains(&id) {
                return Err(missing("unreadable retained input tape document"));
            }
            if !docs.iter().any(|(k, _)| k == &key(id)) {
                return Err(missing("missing checkpoint/completion marker"));
            }
            let Some(head) = head else {
                return Err(missing("opening not retained"));
            };
            let header: TapeHeader =
                serde_json::from_slice(&head).map_err(|_| missing("opening unreadable"))?;
            let refs = super::selection_refs::revisions(header.seals.iter());
            if !refs.is_empty() {
                let prepared = ctx
                    .w
                    .module_state::<RollStore>()
                    .ok_or_else(|| missing("source retention owner unavailable"))?
                    .prepare_reference_batch(crate::store::NS, vec![(root_key(id), refs)], vec![])
                    .map_err(|e| missing(e.to_string()))?;
                super::selection_refs::publish(ctx, prepared)
                    .map_err(|e| missing(e.to_string()))?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            // A missing or malformed kept header may hide references that no
            // surviving root names. Incomplete is a replay status, not proof
            // that its immutable revisions are collectible. The Roll owner
            // blocks collection until a corrected boot can rebuild the facts.
            if let Some(owner) = ctx.w.module_state_mut::<RollStore>()
                && let Err(guard) =
                    owner.quarantine_collection(crate::store::NS, "retained-tape-unreadable")
            {
                tracing::error!(%guard, "tape collection guard could not be installed");
            }
            if let Some(store) = ctx.w.module_state_mut::<TapeStore>() {
                let mark = store.fail(id, error.to_string());
                persist(ctx, key(id), &mark);
            }
        }
    }
    // Only syntactically exact, module-owned orphan roots can be reclaimed.
    // Corrupt retained records keep their roots conservatively.
    let roots = ctx
        .w
        .module_state::<RollStore>()
        .map(|r| r.reference_keys(crate::store::NS, ReferenceKind::Tape))
        .unwrap_or_default();
    for root in roots {
        if let Some(id) = root
            .record
            .strip_prefix("match/")
            .and_then(|n| n.parse::<u64>().ok())
            .map(MatchId)
            && !keep.contains(&id)
        {
            obsolete.entry(id).or_default();
        }
    }
    for (id, keys) in obsolete {
        match release_root(ctx, id) {
            Ok(()) => {
                for key in keys {
                    crate::store::push_op(ctx.out, mush_core::ModuleWrite::Del { key });
                }
            }
            Err(error) => tracing::warn!(%id,%error,"orphan tape roots retained pending cleanup"),
        }
    }
}

fn encode(value: &impl serde::Serialize) -> Result<Vec<u8>, TapeError> {
    encoded_len(value)?;
    serde_json::to_vec(value).map_err(|e| missing(e.to_string()))
}
fn validate_mark(mark: &TapeMark) -> Result<(), TapeError> {
    if let TapeStatus::Incomplete { reason } = &mark.status
        && reason.chars().take(REASON_CHARS + 1).count() > REASON_CHARS
    {
        return Err(missing("recording failure reason exceeds bound"));
    }
    crate::arena::snapshot_capacity::json_len(mark, MARKER_BYTES)
        .map_err(|_| missing("recording marker exceeds bound"))?;
    Ok(())
}

/// Reject oversized persisted payloads while they are still borrowed. Neither
/// a typed allocation nor a clone of an untrusted string/array precedes this.
fn decode_boot_document(
    phase: u8,
    seq: u32,
    value: &serde_json::Value,
) -> Result<TapeWrite, TapeError> {
    let limit = match phase {
        0 => {
            let limit = value
                .get("limit")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| missing("opening byte bound is missing"))?;
            validate_limit(limit)?;
            limit.saturating_sub(MARKER_BYTES)
        }
        1 => CHUNK_DOCUMENT_BYTES,
        _ => MARKER_BYTES,
    };
    crate::arena::snapshot_capacity::json_len(value, limit)
        .map_err(|_| missing("retained input document exceeds bound"))?;
    match phase {
        0 => serde_json::from_value(value.clone())
            .map(|header| TapeWrite::Open(Arc::new(header)))
            .map_err(|_| missing("opening is unreadable")),
        1 => {
            let chunk: TapeChunk = serde_json::from_value(value.clone())
                .map_err(|_| missing("input chunk is unreadable"))?;
            if chunk.seq != seq {
                return Err(missing("input chunk key differs"));
            }
            Ok(TapeWrite::Chunk(Arc::new(chunk)))
        }
        _ => {
            let mark: TapeMark = serde_json::from_value(value.clone())
                .map_err(|_| missing("recording marker is unreadable"))?;
            validate_mark(&mark)?;
            Ok(TapeWrite::Mark(mark))
        }
    }
}
fn missing(why: impl Into<String>) -> TapeError {
    TapeError::Unavailable(why.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn complete(store: &mut TapeStore, id: MatchId) {
        let mut mark = store.entries[&id].mark.clone();
        mark.status = TapeStatus::Complete {
            at: 0,
            hash: 0,
            end: crate::arena::state::MatchEnd {
                t_ms: 0,
                reason: crate::arena::state::EndReason::Abort,
            },
        };
        store.apply(id, &TapeWrite::Mark(mark)).unwrap();
    }
    #[test]
    fn m6_tape_store_enforces_aggregate_bytes_and_pinned_oldest_eviction() {
        let (_, header) = crate::minds::resimulation::tests::opening(2);
        let size = encoded_len(&*header).unwrap() as u64;
        let mut store = TapeStore {
            limit: (size + MARKER_BYTES) * 2,
            ..Default::default()
        };
        for n in [7, 8] {
            let mut h = (*header).clone();
            h.match_id = n;
            store
                .apply(MatchId(n), &TapeWrite::Open(Arc::new(h)))
                .unwrap();
            complete(&mut store, MatchId(n));
        }
        assert_eq!(store.resident_bytes(), (size + MARKER_BYTES) * 2);
        let checkout = store.checkout_for(MatchId(7), ObjId(1000)).unwrap();
        assert_eq!(checkout.encoded_bytes(), size);
        assert_eq!(store.victim(MatchId(9)), Some(MatchId(8)));
        let mut h = (*header).clone();
        h.match_id = 9;
        assert!(
            store
                .apply(MatchId(9), &TapeWrite::Open(Arc::new(h)))
                .is_err()
        );
        drop(checkout);
        assert_eq!(store.victim(MatchId(9)), Some(MatchId(7)));
        store.remove(MatchId(7));
        assert_eq!(store.resident_bytes(), size + MARKER_BYTES);
    }
    #[test]
    fn m6_tape_checkout_is_exact_character_scoped_and_incomplete_is_terminal() {
        let (_, header) = crate::minds::resimulation::tests::opening(2);
        let id = MatchId(7);
        let mut store = TapeStore::default();
        store.apply(id, &TapeWrite::Open(header)).unwrap();
        assert!(store.checkout_for(id, ObjId(1000)).is_err());
        complete(&mut store, id);
        assert!(store.checkout_for(id, ObjId(9999)).is_err());
        let checkout = store.checkout_for(id, ObjId(1000)).unwrap();
        let same = store.checkout_for(id, ObjId(1001)).unwrap();
        assert_eq!(checkout.artifact_digest(), same.artifact_digest());
        let mark = store.fail(id, "missing final chunk".into());
        assert!(matches!(mark.status, TapeStatus::Incomplete { .. }));
        assert!(store.checkout_for(id, ObjId(1000)).is_err());
        let mut complete_mark = mark;
        complete_mark.status = TapeStatus::Complete {
            at: 0,
            hash: 0,
            end: crate::arena::state::MatchEnd {
                t_ms: 0,
                reason: crate::arena::state::EndReason::Abort,
            },
        };
        assert!(store.apply(id, &TapeWrite::Mark(complete_mark)).is_err());
    }
    #[test]
    fn m6_tape_store_bounds_unknown_failure_markers() {
        let mut store = TapeStore::default();
        for n in 1..=100 {
            store.fail(MatchId(n), "failed admission".into());
        }
        assert_eq!(store.entries.len(), MAX_RECORDS);
        assert_eq!(store.resident_bytes(), MAX_RECORDS as u64 * MARKER_BYTES);
        let mut limited = TapeStore {
            limit: MARKER_BYTES,
            ..Default::default()
        };
        limited.fail(MatchId(1), "x".repeat(100_000));
        limited.fail(MatchId(2), "not retained".into());
        assert_eq!(limited.entries.len(), 1);
        assert_eq!(limited.resident_bytes(), MARKER_BYTES);
        let TapeStatus::Incomplete { reason } = &limited.entries[&MatchId(1)].mark.status else {
            panic!("explicit incomplete marker");
        };
        assert_eq!(reason.chars().count(), REASON_CHARS);
    }

    #[test]
    fn m6_boot_document_caps_precede_typed_decode_and_marker_publication() {
        for (phase, value) in [
            (
                0,
                serde_json::json!({"limit":MIN_MAX_BYTES,"unexpected":"x".repeat(MIN_MAX_BYTES as usize)}),
            ),
            (
                1,
                serde_json::json!({"unexpected":"x".repeat(CHUNK_DOCUMENT_BYTES as usize)}),
            ),
            (
                2,
                serde_json::json!({"unexpected":"x".repeat(MARKER_BYTES as usize)}),
            ),
        ] {
            let error = decode_boot_document(phase, 0, &value).unwrap_err();
            assert_eq!(
                error,
                missing("retained input document exceeds bound"),
                "size refusal must precede missing-field/typed parsing"
            );
        }
        let mut mark = Entry::unavailable(MatchId(7), "\u{1}".repeat(REASON_CHARS)).mark;
        let value = serde_json::to_value(&mark).unwrap();
        assert!(
            decode_boot_document(2, 0, &value).is_ok(),
            "worst JSON escaping fits reservation"
        );
        let mut store = TapeStore::default();
        mark.status = TapeStatus::Incomplete {
            reason: "x".repeat(REASON_CHARS + 1),
        };
        assert!(
            store
                .apply(MatchId(7), &TapeWrite::Mark(mark.clone()))
                .is_err()
        );
        assert!(decode_boot_document(2, 0, &serde_json::to_value(mark).unwrap()).is_err());
        assert!(store.entries.is_empty());
        assert_eq!(store.resident_bytes(), 0);
    }

    fn retained_revision() -> (RollStore, mush_roll::RevisionRef) {
        struct Valid;
        impl mush_roll::RevisionAdapter for Valid {
            fn normalize_source(&self, source: &str) -> Result<String, mush_roll::RollError> {
                Ok(source.into())
            }
            fn validate(
                &self,
                _: &str,
                _: &mush_roll::Envelope,
            ) -> Result<(), mush_roll::RollError> {
                Ok(())
            }
        }
        let actor = ObjId(17);
        let mut roll = RollStore::default();
        let draft = mush_roll::WorkingCopy {
            source: "fn think(v,c) {}".into(),
            envelope: mush_roll::Envelope {
                schema: serde_json::json!({"sections":[]}),
                presets: serde_json::json!({}),
                profiles: serde_json::json!({"Default":{}}),
                rules: serde_json::json!({}),
                declared_class: "II".into(),
                mind_abi: crate::minds::MIND_ABI.into(),
            },
        };
        let created = roll.prepare_create(actor, "owned", draft).unwrap();
        roll.publish_with(created, |_| {}).unwrap();
        let sources = crate::TacsimStore::default();
        let mut first = None;
        for _ in 0..51 {
            let prepared = roll
                .prepare_save(
                    actor,
                    "owned",
                    roll.mind(actor, "owned").unwrap().base(),
                    "",
                    None,
                    &Valid,
                    &sources,
                )
                .unwrap();
            first.get_or_insert(prepared.revision());
            roll.publish_with(prepared.into_parts().0, |_| {}).unwrap();
        }
        (roll, first.unwrap())
    }

    #[test]
    fn m6_oversized_kept_boot_documents_preserve_roots_and_block_unknown_collection() {
        struct Owner(Option<RollStore>);
        impl mush_core::GameModule for Owner {
            fn name(&self) -> &'static str {
                "roll"
            }
            fn register(
                &mut self,
                reg: &mut mush_core::Registrar<'_>,
            ) -> Result<(), mush_core::ModError> {
                reg.state(self.0.take().unwrap());
                reg.persist_delegation(crate::store::NS, mush_roll::NS, "test retention owner")?;
                Ok(())
            }
        }
        struct Game;
        impl mush_core::GameModule for Game {
            fn name(&self) -> &'static str {
                crate::store::NS
            }
            fn register(
                &mut self,
                reg: &mut mush_core::Registrar<'_>,
            ) -> Result<(), mush_core::ModError> {
                reg.state(TapeStore::default());
                Ok(())
            }
        }
        for retained_root in [false, true] {
            let (mut roll, revision) = retained_revision();
            assert_eq!(
                roll.prepare_prune_owned(revision.key.character, 128)
                    .unwrap()
                    .removed,
                [revision]
            );
            let id = MatchId(7);
            let root = root_key(id);
            if retained_root {
                let prepared = roll
                    .prepare_reference(crate::store::NS, root.clone(), vec![revision])
                    .unwrap();
                roll.publish_with(prepared, |_| {}).unwrap();
            }
            let stale_prune = roll
                .prepare_prune_owned(revision.key.character, 128)
                .unwrap();
            let mut engine = mush_core::Engine::with_modules(
                World::new_minimal(),
                mush_core::Registry::new(),
                vec![Box::new(Owner(Some(roll))), Box::new(Game)],
            )
            .unwrap();
            let docs = vec![
                (
                    head_key(id),
                    serde_json::json!({"limit":MIN_MAX_BYTES,"unexpected":"x".repeat(MIN_MAX_BYTES as usize)}),
                ),
                (
                    chunk_key(id, 0),
                    serde_json::json!({"unexpected":"x".repeat(CHUNK_DOCUMENT_BYTES as usize)}),
                ),
                (
                    key(id),
                    serde_json::json!({"unexpected":"x".repeat(MARKER_BYTES as usize)}),
                ),
            ];
            let mut out = mush_core::Outbox::new();
            let mut ctx = Ctx {
                w: &mut engine.world,
                actor: revision.key.character,
                switches: Default::default(),
                args: Default::default(),
                out: &mut out,
                reg: &engine.registry,
                module: Some(crate::store::NS.into()),
                typed_state: mush_core::ModuleActionState::unavailable(),
            };
            restore(&mut ctx, &docs, &BTreeSet::from([id]));
            let tapes = ctx.w.module_state::<TapeStore>().unwrap();
            assert!(tapes.entries[&id].head.is_none());
            assert!(tapes.entries[&id].chunks.is_empty());
            assert!(matches!(
                tapes.entries[&id].mark.status,
                TapeStatus::Incomplete { .. }
            ));
            assert_eq!(tapes.resident_bytes(), MARKER_BYTES);
            let roll = ctx.w.module_state::<RollStore>().unwrap();
            assert_eq!(
                roll.reference_keys(crate::store::NS, ReferenceKind::Tape),
                if retained_root { vec![root] } else { vec![] }
            );
            let refusal = roll
                .prepare_prune_owned(revision.key.character, 128)
                .err()
                .expect("unresolved retained documents block collection");
            assert!(matches!(&refusal, mush_roll::RollError::Quarantined(_)));
            assert!(
                !refusal.to_string().contains("owned"),
                "collection refusal contains no private name"
            );
            assert!(
                !refusal
                    .to_string()
                    .contains(&revision.key.character.0.to_string()),
                "no character selector"
            );
            assert!(roll.prepare_prune(128).is_err());
            assert!(roll.check_prepared(&stale_prune.change).is_err());
            assert!(roll.revision(revision.key.character, revision.key).is_ok());
        }
    }
}
