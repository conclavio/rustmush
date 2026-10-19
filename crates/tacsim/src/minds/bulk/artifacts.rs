//! Durable chunk manifests and a bounded read assembler. Large artifact text
//! never decodes on the world loop; chunks are prepared on the private worker.
use super::{
    BulkError, invalid,
    model::{Artifact, ArtifactBytes, MAX_ARTIFACT_BYTES, RunId},
};
use mush_core::{AccountId, ModuleWrite};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Part {
    Start,
    Result,
}
impl Part {
    fn word(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Result => "result",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Key {
    pub account: AccountId,
    pub run: RunId,
    pub part: Part,
}
impl Key {
    pub fn document(self) -> SmolStr {
        format!(
            "tacsim:mindbulk:blob:{}:{}:{}",
            self.account.canonical_hash().to_hex(),
            self.run.0,
            self.part.word()
        )
        .into()
    }
    pub fn chunk(self, n: u16) -> SmolStr {
        format!("{}:chunk:{n}", self.document()).into()
    }
}

/// Exact physical write codec for the registered artifact participant. The
/// adapter still proves its private ticket/run before producing these writes.
pub(super) fn accepts_registry_write(write: &mush_core::authoritative::ReplayWrite) -> bool {
    use mush_core::authoritative::ReplayWrite;
    let ReplayWrite::ModuleJsonSet {
        namespace,
        key,
        value,
    } = write
    else {
        return false;
    };
    if namespace.as_str() != crate::NS {
        return false;
    }
    let parts = key.as_str().split(':').collect::<Vec<_>>();
    let (account, run, part, chunk) = match parts.as_slice() {
        ["tacsim", "mindbulk", "blob", account, run, part] => (*account, *run, *part, None),
        [
            "tacsim",
            "mindbulk",
            "blob",
            account,
            run,
            part,
            "chunk",
            chunk,
        ] => (*account, *run, *part, Some(*chunk)),
        _ => return false,
    };
    let Ok(account) =
        serde_json::from_value::<AccountId>(serde_json::Value::String(account.into()))
    else {
        return false;
    };
    let Ok(run) = run.parse::<u64>() else {
        return false;
    };
    if run == 0 {
        return false;
    }
    let part = match part {
        "start" => Part::Start,
        "result" => Part::Result,
        _ => return false,
    };
    let exact = Key {
        account,
        run: RunId(run),
        part,
    };
    if let Some(chunk) = chunk {
        let Ok(chunk) = chunk.parse::<u16>() else {
            return false;
        };
        chunk < 513
            && key.as_str() == exact.chunk(chunk).as_str()
            && value
                .as_str()
                .is_some_and(|text| !text.is_empty() && text.len() <= 32 * 1024)
    } else {
        key.as_str() == exact.document().as_str()
            && serde_json::from_value::<Manifest>(value.clone())
                .is_ok_and(|manifest| manifest.key == exact && manifest.validate().is_ok())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Manifest {
    pub format: u16,
    pub key: Key,
    pub artifact: Artifact,
    pub chunks: u16,
    /// Worker-rendered bounded report, independent of the full replay artifact.
    pub summary: Vec<String>,
    pub context: Option<super::report::Context>,
}
impl Manifest {
    fn validate(&self) -> Result<(), BulkError> {
        if let Some(context) = &self.context {
            context.validate()?;
        }
        if self.format != 1
            || self.key.run.0 == 0
            || self.artifact.bytes == 0
            || self.artifact.bytes > MAX_ARTIFACT_BYTES
            || self.chunks == 0
            || self.chunks > 513
            || u64::from(self.chunks) * 32 * 1024 < self.artifact.bytes
            || self.summary.len() > 200
            || self
                .summary
                .iter()
                .any(|s| s.len() > 512 || s.chars().any(char::is_control))
        {
            return Err(invalid("artifact manifest bounds"));
        }
        Ok(())
    }
}
pub(crate) struct PreparedBlob {
    pub(super) manifest: Manifest,
    pub(super) writes: Vec<ModuleWrite>,
}
impl PreparedBlob {
    pub fn prepare(
        key: Key,
        bytes: &mut ArtifactBytes,
        summary: Vec<String>,
    ) -> Result<Self, BulkError> {
        let chunks = bytes.take_chunks();
        let manifest = Manifest {
            format: 1,
            key,
            artifact: bytes.descriptor().clone(),
            chunks: u16::try_from(chunks.len()).map_err(|_| invalid("chunk count"))?,
            summary,
            context: bytes.take_context(),
        };
        manifest.validate()?;
        let mut writes = Vec::with_capacity(chunks.len() + 1);
        for (n, chunk) in chunks.into_iter().enumerate() {
            writes.push(ModuleWrite::JsonSet {
                key: key.chunk(n as u16),
                value: serde_json::Value::String(chunk),
            });
        }
        writes.push(ModuleWrite::JsonSet {
            key: key.document(),
            value: serde_json::to_value(&manifest).map_err(|e| invalid(e.to_string()))?,
        });
        Ok(Self { manifest, writes })
    }
}
#[derive(Clone, Default)]
pub(crate) struct Artifacts {
    manifests: BTreeMap<Key, Manifest>,
    quarantine: BTreeSet<SmolStr>,
    pub(super) registry: super::registry_tickets::Tickets,
}
impl Artifacts {
    pub fn load(&mut self, docs: &[(SmolStr, serde_json::Value)]) {
        for (key, value) in docs
            .iter()
            .filter(|(k, _)| k.starts_with("tacsim:mindbulk:blob:") && !k.contains(":chunk:"))
        {
            match serde_json::from_value::<Manifest>(value.clone())
                .ok()
                .filter(|m| m.key.document() == *key && m.validate().is_ok())
            {
                Some(manifest) => {
                    self.manifests.insert(manifest.key, manifest);
                }
                None => {
                    self.quarantine.insert(key.clone());
                }
            }
        }
    }
    pub fn get(&self, key: Key) -> Result<&Manifest, BulkError> {
        if self.quarantine.contains(&key.document()) {
            return Err(BulkError::Quarantined);
        }
        self.manifests.get(&key).ok_or(BulkError::Missing)
    }
    pub fn check_new(&self, blob: &PreparedBlob) -> Result<(), BulkError> {
        if self.quarantine.contains(&blob.manifest.key.document()) {
            return Err(BulkError::Quarantined);
        }
        if self.manifests.contains_key(&blob.manifest.key) {
            return Err(BulkError::Consumed);
        }
        Ok(())
    }
    pub fn install(&mut self, manifest: Manifest) {
        self.manifests.insert(manifest.key, manifest);
    }
    pub fn remove(&mut self, key: Key) {
        self.manifests.remove(&key);
    }
    pub fn deletion(&self, key: Key) -> Result<Vec<ModuleWrite>, BulkError> {
        let m = match self.get(key) {
            Ok(m) => m,
            Err(BulkError::Missing) => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        Ok((0..m.chunks)
            .map(|n| ModuleWrite::Del { key: key.chunk(n) })
            .chain(std::iter::once(ModuleWrite::Del {
                key: key.document(),
            }))
            .collect())
    }
}
/// One authorized, admitted bench read. At most two chunks/request fit the
/// existing 512KiB StoreRead envelope even with six-byte JSON escaping.
pub(crate) struct Read {
    manifest: Manifest,
    next: u16,
    bytes: Vec<u8>,
}
impl Read {
    pub fn new(manifest: Manifest) -> Result<Self, BulkError> {
        manifest.validate()?;
        Ok(Self {
            manifest,
            next: 0,
            bytes: Vec::new(),
        })
    }
    pub fn keys(&self) -> Vec<SmolStr> {
        (self.next..self.manifest.chunks.min(self.next.saturating_add(2)))
            .map(|n| self.manifest.key.chunk(n))
            .collect()
    }
    pub fn accept(&mut self, docs: &[(SmolStr, serde_json::Value)]) -> Result<bool, BulkError> {
        let keys = self.keys();
        if docs.len() != keys.len() {
            return Err(invalid("artifact chunk missing"));
        }
        for key in keys {
            let text = docs
                .iter()
                .find(|(k, _)| *k == key)
                .and_then(|(_, v)| v.as_str())
                .ok_or_else(|| invalid("artifact chunk missing or malformed"))?;
            if text.is_empty()
                || text.len() > 32 * 1024
                || self.bytes.len() as u64 + text.len() as u64 > self.manifest.artifact.bytes
            {
                return Err(invalid("artifact chunk byte bound"));
            }
            self.bytes.extend_from_slice(text.as_bytes());
            self.next += 1;
        }
        Ok(self.next == self.manifest.chunks)
    }
    /// Worker-only hash/parse work follows the bounded chunk accumulation.
    pub fn finish(self) -> Result<ArtifactBytes, BulkError> {
        if self.next != self.manifest.chunks {
            return Err(invalid("artifact incomplete"));
        }
        let value = ArtifactBytes::new(self.bytes)?;
        if value.descriptor() != &self.manifest.artifact {
            return Err(invalid("artifact digest mismatch"));
        }
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_chunk_roundtrip_and_digest_tamper_fail_closed() {
        let key = Key {
            account: AccountId::mint("a", 1).unwrap(),
            run: RunId(1),
            part: Part::Start,
        };
        let original = "ä".repeat(40_000);
        let mut bytes = ArtifactBytes::new(original.as_bytes().to_vec()).unwrap();
        let prepared = PreparedBlob::prepare(key, &mut bytes, vec![]).unwrap();
        assert_eq!(prepared.manifest.chunks, 3);
        let docs = prepared
            .writes
            .iter()
            .filter_map(|w| match w {
                ModuleWrite::JsonSet { key, value } => Some((key.clone(), value.clone())),
                _ => None,
            })
            .collect::<Vec<_>>();
        let mut read = Read::new(prepared.manifest.clone()).unwrap();
        while !read.keys().is_empty() {
            let keys = read.keys();
            let page = docs
                .iter()
                .filter(|(key, _)| keys.contains(key))
                .cloned()
                .collect::<Vec<_>>();
            read.accept(&page).unwrap();
        }
        assert_eq!(read.finish().unwrap().bytes(), original.as_bytes());
        let mut bad = prepared.manifest;
        bad.artifact.sha256[0] ^= 1;
        let mut read = Read::new(bad).unwrap();
        while !read.keys().is_empty() {
            let keys = read.keys();
            read.accept(
                &docs
                    .iter()
                    .filter(|(key, _)| keys.contains(key))
                    .cloned()
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        }
        assert!(read.finish().is_err());
    }
}
