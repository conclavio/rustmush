//! Independent immutable native admission history. Neither a decoded typed
//! result nor a caller-set health flag can recreate this owner census.
use super::{BulkError, invalid};
use mush_core::{AccountId, CommitToken, DefinitionDigest, ModuleWrite};
use mush_roll::commerce::conduct::{MAX_TRIALS, records::ConductJob};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use std::collections::{BTreeMap, BTreeSet};

const PREFIX: &str = "tacsim:conduct:admission:";
const HEAD: &str = "tacsim:conduct:admission:head";
const ENTRY_BYTES: usize = 12 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Entry {
    format: u16,
    sequence: u64,
    previous: Option<DefinitionDigest>,
    account: AccountId,
    job: ConductJob,
    digest: DefinitionDigest,
}
impl Entry {
    fn calculated(&self) -> Result<DefinitionDigest, BulkError> {
        DefinitionDigest::for_canonical_value_with_limit(
            "tacsim.conduct.admission.v1",
            &serde_json::json!([
                self.format,
                self.sequence,
                self.previous,
                self.account,
                self.job
            ]),
            ENTRY_BYTES,
        )
        .map_err(|_| invalid("Conduct admission bound"))
    }
    fn validate(&self) -> Result<(), BulkError> {
        self.job
            .validate()
            .map_err(|_| invalid("Conduct immutable job"))?;
        if self.format != 1
            || self.sequence == 0
            || self.sequence > MAX_TRIALS as u64
            || (self.sequence == 1) != self.previous.is_none()
            || self.digest != self.calculated()?
        {
            return Err(invalid("Conduct admission history"));
        }
        let binding = DefinitionDigest::for_canonical_value_with_limit(
            "roll.conduct.job-account.v1",
            &serde_json::json!([
                self.account,
                self.job.operation(),
                self.job.admission().run(),
                self.job.admission().nonce()
            ]),
            4096,
        )
        .map_err(|_| invalid("Conduct account binding"))?;
        if binding != self.job.admission().account_binding() {
            return Err(invalid("Conduct admission account differs"));
        }
        require_write_room(&self.write()?, ENTRY_BYTES)
    }
    pub(super) fn key(&self) -> SmolStr {
        format!("{PREFIX}{}", self.job.operation().to_hex()).into()
    }
    fn write(&self) -> Result<ModuleWrite, BulkError> {
        Ok(ModuleWrite::JsonSet {
            key: self.key(),
            value: serde_json::to_value(self).map_err(|_| invalid("Conduct admission encoding"))?,
        })
    }
    pub(super) fn account(&self) -> AccountId {
        self.account
    }
    pub(super) fn job(&self) -> &ConductJob {
        &self.job
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Head {
    format: u16,
    count: u64,
    digest: DefinitionDigest,
}
impl Head {
    fn write(&self) -> Result<ModuleWrite, BulkError> {
        let write = ModuleWrite::JsonSet {
            key: HEAD.into(),
            value: serde_json::to_value(self).map_err(|_| invalid("Conduct head encoding"))?,
        };
        require_write_room(&write, 1024)?;
        Ok(write)
    }
}

#[derive(Clone, Default)]
pub(super) struct Index {
    loaded: bool,
    invalid: bool,
    head: Option<Head>,
    rows: BTreeMap<CommitToken, Entry>,
}
pub(super) struct PreparedAdmission {
    before: Option<Head>,
    entry: Entry,
    after: Head,
}
impl PreparedAdmission {
    pub(super) fn writes(&self) -> Result<Vec<ModuleWrite>, BulkError> {
        Ok(vec![self.entry.write()?, self.after.write()?])
    }
}
impl Index {
    pub(super) fn load(&mut self, docs: &[(SmolStr, serde_json::Value)]) {
        if self.loaded {
            self.invalid = true;
            return;
        }
        self.loaded = true;
        self.invalid = self.recover(docs).is_err();
    }
    fn recover(&mut self, docs: &[(SmolStr, serde_json::Value)]) -> Result<(), BulkError> {
        let mut keys = BTreeSet::new();
        let mut sequence = BTreeMap::new();
        let mut allocated = BTreeSet::new();
        for (key, value) in docs.iter().filter(|(k, _)| k.starts_with(PREFIX)) {
            if !keys.insert(key) || keys.len() > MAX_TRIALS + 1 {
                return Err(BulkError::Quarantined);
            }
            if key == HEAD {
                let head: Head =
                    serde_json::from_value(value.clone()).map_err(|_| BulkError::Quarantined)?;
                if head.format != 1 || head.count == 0 || head.count > MAX_TRIALS as u64 {
                    return Err(BulkError::Quarantined);
                }
                head.write()?;
                self.head = Some(head);
            } else {
                let entry: Entry =
                    serde_json::from_value(value.clone()).map_err(|_| BulkError::Quarantined)?;
                entry.validate()?;
                if entry.key() != *key
                    || !allocated.insert((entry.account, entry.job.admission().run()))
                    || sequence
                        .insert(entry.sequence, (entry.previous, entry.digest))
                        .is_some()
                    || self.rows.insert(entry.job.operation(), entry).is_some()
                {
                    return Err(BulkError::Quarantined);
                }
            }
        }
        let mut previous = None;
        for (i, (n, (before, after))) in sequence.into_iter().enumerate() {
            if n != i as u64 + 1 || before != previous {
                return Err(BulkError::Quarantined);
            }
            previous = Some(after);
        }
        match (&self.head, previous) {
            (None, None) if self.rows.is_empty() => Ok(()),
            (Some(head), Some(last))
                if head.count == self.rows.len() as u64 && head.digest == last =>
            {
                Ok(())
            }
            _ => Err(BulkError::Quarantined),
        }
    }
    fn require(&self) -> Result<(), BulkError> {
        if !self.loaded || self.invalid {
            Err(BulkError::Quarantined)
        } else {
            Ok(())
        }
    }
    pub(super) fn entries(&self) -> Result<impl Iterator<Item = &Entry>, BulkError> {
        self.require()?;
        Ok(self.rows.values())
    }
    pub(super) fn get(&self, job: CommitToken) -> Result<&Entry, BulkError> {
        self.require()?;
        self.rows.get(&job).ok_or(BulkError::Missing)
    }
    pub(super) fn prepare(
        &self,
        account: AccountId,
        job: ConductJob,
    ) -> Result<PreparedAdmission, BulkError> {
        self.require()?;
        if self.rows.len() >= MAX_TRIALS
            || self.rows.contains_key(&job.operation())
            || self
                .rows
                .values()
                .any(|e| e.account == account && e.job.admission().run() == job.admission().run())
        {
            return Err(BulkError::Retention);
        }
        let mut entry = Entry {
            format: 1,
            sequence: self.rows.len() as u64 + 1,
            previous: self.head.as_ref().map(|h| h.digest),
            account,
            job,
            digest: DefinitionDigest::from_bytes([0; 32]),
        };
        entry.digest = entry.calculated()?;
        entry.validate()?;
        let after = Head {
            format: 1,
            count: entry.sequence,
            digest: entry.digest,
        };
        after.write()?;
        Ok(PreparedAdmission {
            before: self.head.clone(),
            entry,
            after,
        })
    }
    pub(super) fn check(&self, prepared: &PreparedAdmission) -> Result<(), BulkError> {
        self.require()?;
        if self.head != prepared.before || self.rows.contains_key(&prepared.entry.job.operation()) {
            return Err(BulkError::Stale);
        }
        Ok(())
    }
    /// Called only after the actual participant has bound this exact index.
    pub(super) fn install(&mut self, prepared: PreparedAdmission) {
        self.rows
            .insert(prepared.entry.job.operation(), prepared.entry);
        self.head = Some(prepared.after);
    }
}

pub(super) fn replay(
    write: &ModuleWrite,
) -> Result<mush_core::authoritative::ReplayWrite, BulkError> {
    use mush_core::authoritative::{AuthoritativeKey, ReplayWrite};
    let ModuleWrite::JsonSet { key, value } = write else {
        return Err(invalid("Conduct native write family"));
    };
    Ok(ReplayWrite::ModuleJsonSet {
        namespace: mush_core::OwnerId::new(crate::NS).map_err(|_| invalid("Conduct owner"))?,
        key: AuthoritativeKey::parse_canonical(key).map_err(|_| invalid("Conduct native key"))?,
        value: value.clone(),
    })
}
fn require_write_room(write: &ModuleWrite, limit: usize) -> Result<(), BulkError> {
    mush_core::canonical_json_bytes_with_limit(
        &serde_json::to_value(replay(write)?).map_err(|_| invalid("Conduct native envelope"))?,
        limit,
    )
    .map(|_| ())
    .map_err(|_| invalid("Conduct native envelope bound"))
}
pub(super) fn accepts(write: &mush_core::authoritative::ReplayWrite) -> bool {
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
    if key.as_str() == HEAD {
        serde_json::from_value::<Head>(value.clone()).is_ok_and(|h| {
            h.format == 1 && h.count > 0 && h.count <= MAX_TRIALS as u64 && h.write().is_ok()
        })
    } else {
        key.as_str().starts_with(PREFIX)
            && serde_json::from_value::<Entry>(value.clone())
                .is_ok_and(|e| e.key().as_str() == key.as_str() && e.validate().is_ok())
    }
}
