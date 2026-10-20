//! Compact owner projection reconstructed from every actual permanent quota
//! row, immutable admission index and complete artifact bytes. No decoded row
//! alone can set this projection, and no live request accepts its contents.
use super::{
    BulkError,
    artifacts::{Artifacts, Key, Part, Read},
    conduct_index::Index,
    conduct_projection, invalid,
    model::{ArtifactBytes, RunId, Status},
    owner::BulkStore,
};
use crate::minds::registry::conduct::history;
use mush_core::{CommitToken, DefinitionDigest};
use mush_roll::commerce::conduct::{
    MAX_TRIALS,
    records::{ConductJob, ConductMeasurement},
};
use serde_json::Value;
use smol_str::SmolStr;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Evidence {
    job: DefinitionDigest,
    measurement: Option<ConductMeasurement>,
}
impl Evidence {
    pub(super) fn admission(job: &ConductJob) -> Result<Self, BulkError> {
        Ok(Self {
            job: job.digest().map_err(|_| invalid("Conduct immutable job"))?,
            measurement: None,
        })
    }
    pub(super) fn completed(
        &self,
        job: &ConductJob,
        measurement: &ConductMeasurement,
        charged_unit_ms: u64,
    ) -> Result<Self, BulkError> {
        measurement
            .validate()
            .map_err(|_| invalid("Conduct measurement"))?;
        if self.job != job.digest().map_err(|_| invalid("Conduct immutable job"))?
            || self.measurement.is_some()
            || measurement.policy() != job.admission().policy()
            || measurement.subject() != job.admission().subject()
            || measurement.charged_unit_ms() != charged_unit_ms
        {
            return Err(invalid("Conduct retained result differs"));
        }
        Ok(Self {
            job: self.job,
            measurement: Some(measurement.clone()),
        })
    }
    pub(super) fn measurement(&self) -> Option<&ConductMeasurement> {
        self.measurement.as_ref()
    }
    fn matches(&self, job: &ConductJob) -> Result<(), BulkError> {
        if self.job != job.digest().map_err(|_| invalid("Conduct immutable job"))? {
            return Err(invalid("Conduct native job projection differs"));
        }
        Ok(())
    }
}

#[derive(Clone, Default)]
pub(super) struct History {
    loaded: bool,
    invalid: bool,
    legacy_quarantine_only: bool,
    rows: BTreeMap<CommitToken, Evidence>,
}
impl History {
    pub(super) fn hydrate(
        &mut self,
        store: &BulkStore,
        index: &Index,
        artifacts: &Artifacts,
        docs: &[(SmolStr, Value)],
    ) {
        if self.loaded {
            self.invalid = true;
            self.legacy_quarantine_only = false;
            return;
        }
        self.loaded = true;
        match reconstruct(store, index, artifacts, docs) {
            Ok(rows) => self.rows = rows,
            Err(_) => {
                self.invalid = true;
                // Ordinary legacy bulk keeps account-local quarantine. This
                // state never satisfies require(), count(), or a Conduct read.
                self.legacy_quarantine_only = store.conduct_runs().is_err()
                    && !store.has_known_conduct()
                    && index.entries().is_ok_and(|mut rows| rows.next().is_none())
                    && !docs.iter().any(|(key, value)| {
                        conduct_physical_key(key)
                            || key.starts_with("tacsim:conduct:")
                            || (key.starts_with("tacsim:mindbulk:account:")
                                && value.get("runs").is_some_and(|runs| {
                                    runs.as_object().is_none_or(|runs| {
                                        runs.values().any(|run| {
                                            !run.is_object()
                                                || run
                                                    .get("conduct_job")
                                                    .is_some_and(|v| !v.is_null())
                                                || run
                                                    .get("plan")
                                                    .is_some_and(|plan| !plan.is_object())
                                                || run
                                                    .pointer("/plan/conduct")
                                                    .is_some_and(|v| !v.is_null())
                                                || run.pointer("/plan/kind").and_then(Value::as_str)
                                                    == Some("conduct")
                                        })
                                    })
                                }))
                    });
            }
        }
    }
    pub(super) fn require(&self) -> Result<(), BulkError> {
        if !self.loaded || self.invalid {
            Err(BulkError::Quarantined)
        } else {
            Ok(())
        }
    }
    /// Only disabled composition may preserve unrelated legacy quarantine.
    /// Roll independently refuses any typed Conduct row or permanent root.
    pub(super) fn require_disabled(&self) -> Result<(), BulkError> {
        if self.loaded && self.legacy_quarantine_only {
            return Ok(());
        }
        self.require()?;
        if self.rows.is_empty() {
            Ok(())
        } else {
            Err(BulkError::Quarantined)
        }
    }
    pub(super) fn get(&self, job: &ConductJob) -> Result<&Evidence, BulkError> {
        self.require()?;
        let row = self.rows.get(&job.operation()).ok_or(BulkError::Missing)?;
        row.matches(job)?;
        Ok(row)
    }
    pub(super) fn check(
        &self,
        job: CommitToken,
        before: Option<&Evidence>,
    ) -> Result<(), BulkError> {
        self.require()?;
        if self.rows.get(&job) != before || before.is_none() && self.rows.len() >= MAX_TRIALS {
            return Err(BulkError::Stale);
        }
        Ok(())
    }
    /// Only an actual bound artifact image may advance the checked projection.
    pub(super) fn install(&mut self, job: CommitToken, after: Evidence) {
        self.rows.insert(job, after);
    }
    pub(super) fn count(&self) -> Result<usize, BulkError> {
        self.require()?;
        Ok(self.rows.len())
    }
}

fn reconstruct(
    store: &BulkStore,
    index: &Index,
    artifacts: &Artifacts,
    docs: &[(SmolStr, Value)],
) -> Result<BTreeMap<CommitToken, Evidence>, BulkError> {
    let mut documents = BTreeMap::new();
    let mut duplicates = BTreeSet::new();
    for (key, value) in docs {
        if documents.insert(key.as_str(), value).is_some() {
            duplicates.insert(key.as_str());
        }
    }
    let mut native = BTreeMap::new();
    for (account, run) in store.conduct_runs()? {
        let job = run
            .conduct_job
            .ok_or_else(|| invalid("Conduct quota has no admission"))?;
        if native.len() == MAX_TRIALS || native.insert(job, (account, run)).is_some() {
            return Err(invalid("Conduct quota admission inventory"));
        }
    }
    let mut rows = BTreeMap::new();
    let mut physical = BTreeSet::new();
    for entry in index.entries()? {
        let job = entry.job();
        let (account, run) = native
            .remove(&job.operation())
            .ok_or_else(|| invalid("Conduct admission has no quota row"))?;
        if account != entry.account()
            || conduct_projection::admission(account, job.operation(), run)? != *job.admission()
        {
            return Err(invalid("Conduct admission and quota disagree"));
        }
        if duplicates.contains(super::publication::account_key(account).as_str()) {
            return Err(invalid("Conduct quota document repeated"));
        }
        let observation = conduct_projection::observation(run)?;
        let key = |part| Key {
            account,
            run: RunId(job.admission().run()),
            part,
        };
        let start_bytes = read(artifacts, &documents, &duplicates, key(Part::ConductStart))?;
        retain_keys(artifacts, key(Part::ConductStart), &mut physical)?;
        let start = history::Start::decode(&start_bytes, job, &run.plan)
            .map_err(|_| invalid("Conduct retained opening differs"))?;
        let mut evidence = Evidence::admission(job)?;
        if run.status == Status::Complete {
            let result_bytes = read(artifacts, &documents, &duplicates, key(Part::ConductResult))?;
            retain_keys(artifacts, key(Part::ConductResult), &mut physical)?;
            if run.result.as_ref() != Some(result_bytes.descriptor()) {
                return Err(invalid("Conduct quota result differs"));
            }
            let measurement = history::result(&result_bytes, &start)
                .map_err(|_| invalid("Conduct retained measurement differs"))?;
            evidence = evidence.completed(job, &measurement, observation.charged_unit_ms)?;
        } else {
            let prefix = key(Part::ConductResult).document();
            if run.result.is_some()
                || documents
                    .keys()
                    .any(|k| *k == prefix.as_str() || k.starts_with(&format!("{prefix}:")))
            {
                return Err(invalid("Conduct premature result artifact"));
            }
        }
        rows.insert(job.operation(), evidence);
    }
    if !native.is_empty() {
        return Err(invalid("Conduct quota has no immutable admission history"));
    }
    // These closed physical names are exclusive to Conduct. Even the loss of
    // every index/quota/typed row cannot make retained artifact bytes disappear
    // from this independent inverse. Unknown or malformed Conduct suffixes fail.
    for key in documents.keys().filter(|key| conduct_physical_key(key)) {
        if !physical.contains(*key) {
            return Err(invalid("orphan Conduct artifact evidence"));
        }
    }
    Ok(rows)
}

fn conduct_physical_key(key: &str) -> bool {
    key.starts_with("tacsim:mindbulk:blob:")
        && key
            .split(':')
            .nth(5)
            .is_some_and(|part| part.starts_with("conduct_"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::minds::bulk::{artifacts::PreparedBlob, model::BulkConfig};
    type TestResult = Result<(), Box<dyn std::error::Error>>;
    fn loaded(docs: &[(SmolStr, Value)]) -> Result<Artifacts, BulkError> {
        let store = BulkStore::new(BulkConfig::default())?;
        let mut artifacts = Artifacts::default();
        artifacts.load(docs);
        artifacts.hydrate_conduct(&store, docs);
        Ok(artifacts)
    }
    #[test]
    fn empty_actual_native_load_is_distinct_from_unloaded_history() -> TestResult {
        assert!(Artifacts::default().conduct_history.require().is_err());
        assert_eq!(loaded(&[])?.conduct_history.count()?, 0);
        Ok(())
    }
    #[test]
    fn orphan_conduct_manifests_chunks_and_unknown_parts_refuse_even_without_index_or_quota()
    -> TestResult {
        let account = mush_core::AccountId::mint("conduct-orphan-physical", 1)?;
        for part in [Part::ConductStart, Part::ConductResult] {
            let key = Key {
                account,
                run: RunId(1),
                part,
            };
            for physical in [
                key.document(),
                key.chunk(0),
                format!("{}:unexpected", key.document()).into(),
            ] {
                let docs = vec![(physical, serde_json::json!({"retained":"orphan"}))];
                assert!(loaded(&docs)?.conduct_history.require().is_err());
            }
        }
        let malformed = format!(
            "tacsim:mindbulk:blob:{}:1:conduct_unknown",
            account.canonical_hash().to_hex()
        );
        assert!(
            loaded(&[(malformed.into(), Value::Null)])?
                .conduct_history
                .require()
                .is_err()
        );
        Ok(())
    }
    #[test]
    fn legacy_account_quarantine_never_becomes_verified_empty_conduct() -> TestResult {
        let account = mush_core::AccountId::mint("conduct-legacy-quarantine", 1)?;
        let key = super::super::publication::account_key(account);
        for extra in [
            None,
            Some(("tacsim:conduct:admission:head".into(), Value::Null)),
            Some((
                Key {
                    account,
                    run: RunId(1),
                    part: Part::ConductStart,
                }
                .chunk(0),
                Value::Null,
            )),
        ] {
            let mut docs = vec![(
                key.clone(),
                serde_json::json!({"malformed_legacy_account":true}),
            )];
            if let Some(extra) = extra.clone() {
                docs.push(extra);
            }
            let mut store = BulkStore::new(BulkConfig::default())?;
            assert!(store.load(account, b"{}").is_err());
            let mut artifacts = Artifacts::default();
            artifacts.load(&docs);
            artifacts.hydrate_conduct(&store, &docs);
            assert!(artifacts.conduct_history.require().is_err());
            assert!(artifacts.conduct_history.count().is_err());
            assert_eq!(
                artifacts.conduct_history.require_disabled().is_ok(),
                extra.is_none()
            );
        }
        // Even a malformed quota document with an actual Conduct marker is
        // retained evidence; it cannot take the legacy-only compatibility path.
        for value in [
            serde_json::json!({"runs":{"1":{"plan":{"kind":"conduct"}}}}),
            serde_json::json!({"runs":[{"conduct_job":"retained"}]}),
            serde_json::json!({"runs":null}),
            serde_json::json!({"runs":{"1":"unreadable"}}),
            serde_json::json!({"runs":{"1":{"plan":[]}}}),
        ] {
            let docs = vec![(key.clone(), value)];
            let mut store = BulkStore::new(BulkConfig::default())?;
            assert!(store.load(account, b"{}").is_err());
            let mut artifacts = Artifacts::default();
            artifacts.load(&docs);
            artifacts.hydrate_conduct(&store, &docs);
            assert!(artifacts.conduct_history.require_disabled().is_err());
        }
        Ok(())
    }
    #[test]
    fn ordinary_actual_blob_namespace_does_not_become_conduct_evidence() -> TestResult {
        let key = Key {
            account: mush_core::AccountId::mint("ordinary-blob", 1)?,
            run: RunId(1),
            part: Part::Start,
        };
        let mut bytes = ArtifactBytes::new(br#"{"ordinary":true}"#.to_vec())?;
        let prepared = PreparedBlob::prepare(key, &mut bytes, vec![])?;
        let docs = prepared
            .writes
            .iter()
            .filter_map(|w| match w {
                mush_core::ModuleWrite::JsonSet { key, value } => {
                    Some((key.clone(), value.clone()))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let artifacts = loaded(&docs)?;
        assert!(artifacts.get(key).is_ok());
        assert_eq!(artifacts.conduct_history.count()?, 0);
        Ok(())
    }
}
fn retain_keys(
    artifacts: &Artifacts,
    key: Key,
    keys: &mut BTreeSet<String>,
) -> Result<(), BulkError> {
    let manifest = artifacts.get(key)?;
    if !keys.insert(key.document().to_string()) {
        return Err(invalid("duplicate Conduct artifact"));
    }
    for n in 0..manifest.chunks {
        keys.insert(key.chunk(n).to_string());
    }
    Ok(())
}

fn read(
    artifacts: &Artifacts,
    docs: &BTreeMap<&str, &Value>,
    duplicates: &BTreeSet<&str>,
    key: Key,
) -> Result<ArtifactBytes, BulkError> {
    let manifest = artifacts.get(key)?;
    if manifest.artifact.bytes > 256 * 1024 {
        return Err(invalid("Conduct artifact bound"));
    }
    let prefix = key.document();
    let expected = std::iter::once(prefix.clone())
        .chain((0..manifest.chunks).map(|n| key.chunk(n)))
        .collect::<BTreeSet<_>>();
    let actual = docs
        .range(prefix.as_str()..)
        .take_while(|(key, _)| key.starts_with(prefix.as_str()))
        .map(|(key, _)| SmolStr::new(*key))
        .collect::<BTreeSet<_>>();
    if actual != expected || expected.iter().any(|k| duplicates.contains(k.as_str())) {
        return Err(invalid("Conduct artifact physical inventory differs"));
    }
    let mut reader = Read::new(manifest.clone())?;
    while !reader.keys().is_empty() {
        let page = reader
            .keys()
            .into_iter()
            .map(|key| {
                let value = docs.get(key.as_str()).ok_or(BulkError::Missing)?;
                Ok((key, (*value).clone()))
            })
            .collect::<Result<Vec<_>, BulkError>>()?;
        reader.accept(&page)?;
    }
    reader.finish()
}

fn owners(world: &mush_core::World) -> Result<(&BulkStore, &Artifacts), BulkError> {
    use std::any::TypeId;
    for ty in [TypeId::of::<BulkStore>(), TypeId::of::<Artifacts>()] {
        if world
            .module_state_owner(ty)
            .is_none_or(|o| o.as_str() != crate::NS)
        {
            return Err(BulkError::Missing);
        }
    }
    Ok((
        world
            .module_state::<BulkStore>()
            .ok_or(BulkError::Missing)?,
        world
            .module_state::<Artifacts>()
            .ok_or(BulkError::Missing)?,
    ))
}
pub(super) fn check(
    world: &mush_core::World,
    row: mush_roll::commerce::conduct::protocol::ConductHistoryRow<'_>,
) -> Result<(), BulkError> {
    let (quota, artifacts) = owners(world)?;
    let job = row.job;
    row.execution
        .matches_job(job)
        .map_err(|_| invalid("Conduct typed execution"))?;
    let entry = artifacts.conduct_index.get(job.operation())?;
    if entry.job() != job {
        return Err(invalid("Conduct immutable native and typed job disagree"));
    }
    let run = quota.run(
        entry.account(),
        job.admission().character(),
        RunId(job.admission().run()),
    )?;
    conduct_projection::require_before(
        entry.account(),
        job.operation(),
        run,
        job.admission(),
        row.execution.observation(),
    )?;
    let evidence = artifacts.conduct_history.get(job)?;
    match row.result {
        Some(result) => {
            result
                .matches(job, row.execution)
                .map_err(|_| invalid("Conduct typed terminal"))?;
            if result.measurement() != evidence.measurement() {
                return Err(invalid("Conduct measured native and typed result disagree"));
            }
        }
        None if !row.execution.observation().phase.terminal()
            && evidence.measurement().is_none() => {}
        None => return Err(invalid("Conduct native terminal lacks typed result")),
    }
    Ok(())
}
pub(super) fn audit(
    world: &mush_core::World,
    rows: &[mush_roll::commerce::conduct::protocol::ConductHistoryRow<'_>],
) -> Result<(), BulkError> {
    let (quota, artifacts) = owners(world)?;
    if rows.len() > MAX_TRIALS || artifacts.conduct_history.count()? != rows.len() {
        return Err(invalid(
            "Conduct complete typed and native inventories disagree",
        ));
    }
    let mut jobs = BTreeSet::new();
    for entry in artifacts.conduct_index.entries()? {
        if !jobs.insert(entry.job().operation()) {
            return Err(invalid("Conduct duplicate native admission"));
        }
    }
    if jobs.len() != rows.len() || quota.conduct_runs()?.count() != rows.len() {
        return Err(invalid(
            "Conduct complete permanent quota inventory differs",
        ));
    }
    for row in rows {
        if !jobs.remove(&row.job.operation()) {
            return Err(invalid("Conduct duplicate or absent typed admission"));
        }
        check(
            world,
            mush_roll::commerce::conduct::protocol::ConductHistoryRow {
                job: row.job,
                execution: row.execution,
                result: row.result,
            },
        )?;
    }
    if !jobs.is_empty() {
        return Err(invalid("Conduct omitted typed history"));
    }
    Ok(())
}
