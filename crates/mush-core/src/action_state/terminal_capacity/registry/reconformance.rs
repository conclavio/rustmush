//! Fixed service profile and admission-head integrity. Filed-source authority
//! remains with Roll; these checks never turn a structural binding into access.
use super::*;

pub(super) const HEADS: &str = "roll_registry_reconformance_heads";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Profile {
    Ordinary,
    Reconformance,
}
impl Profile {
    pub(super) fn ids(self) -> [&'static str; 5] {
        match self {
            Self::Ordinary => ACTIONS,
            Self::Reconformance => [
                "reconformance_admit",
                "reconformance_grant",
                "reconformance_settle",
                "reconformance_cancel",
                "reconformance_recover",
            ],
        }
    }
    pub(super) fn word(self) -> &'static str {
        match self {
            Self::Ordinary => "registry_rating_v1",
            Self::Reconformance => "registry_reconformance_v1",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct Birth {
    job: CommitToken,
    id: DefinitionDigest,
    schema: u16,
    receipt_digest: DefinitionDigest,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Binding {
    format: u16,
    filed: String,
    birth_rating: Birth,
    #[serde(deserialize_with = "required_previous")]
    previous_attempt: Option<CommitToken>,
}
fn required_previous<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<CommitToken>, D::Error> {
    Option::<CommitToken>::deserialize(d)
}
fn filed(text: &str) -> Result<(), ActionStateError> {
    let n = text
        .parse::<u64>()
        .map_err(|_| refused("Registry filed identity"))?;
    if n == 0 || n.to_string() != text {
        return Err(refused("Registry canonical filed identity"));
    }
    Ok(())
}
pub(super) fn binding(
    admission: &Value,
    rejection: bool,
) -> Result<Option<Binding>, ActionStateError> {
    let Some(raw) = admission.get("reconformance") else {
        return Ok(None);
    };
    let row: Binding = serde_json::from_value(raw.clone())
        .map_err(|_| refused("Registry reconformance binding"))?;
    filed(&row.filed)?;
    if row.format != 1
        || row.birth_rating.schema != 1
        || !rejection
        || number(admission, "character")? != u64::from(crate::GOD.0)
        || row.previous_attempt == Some(row.birth_rating.job)
    {
        return Err(refused("Registry reconformance binding fields"));
    }
    Ok(Some(row))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Head {
    format: u16,
    filed: String,
    compatibility: DefinitionDigest,
    job: CommitToken,
    admitted_at_utc: i64,
}
fn head_key(filed: &str, compatibility: DefinitionDigest) -> Result<Key, ActionStateError> {
    Ok(Key {
        owner: OwnerId::new("roll").map_err(|_| refused("Registry head owner"))?,
        family: StateFamilyId::new(HEADS).map_err(|_| refused("Registry head family"))?,
        encoded: digest(
            "roll.registry.reconformance.head-key.v1",
            &json!([filed, compatibility]),
        )?
        .to_string()
        .into(),
    })
}
fn read(view: &View<'_>, key: &Key) -> Result<Option<Head>, ActionStateError> {
    let resolved = key.resolve(view.registry)?;
    let document = match view.mutations.and_then(|m| m.get(&resolved)) {
        Some(TxnMutation::Put { document, .. }) => Some(document.clone()),
        Some(TxnMutation::Delete) => None,
        None => view
            .records
            .get(&resolved)
            .map(|r| {
                serde_json::from_slice::<Value>(r.canonical.as_slice())
                    .map_err(|_| refused("Registry head encoding"))
            })
            .transpose()?,
    };
    document
        .map(|d| {
            let head: Head = serde_json::from_value(field(&d, "value")?.clone())
                .map_err(|_| refused("Registry head shape"))?;
            filed(&head.filed)?;
            if head.format != 1
                || head.admitted_at_utc < 0
                || head_key(&head.filed, head.compatibility)? != *key
            {
                return Err(refused("Registry head key or fields"));
            }
            Ok(head)
        })
        .transpose()
}
pub(super) fn descriptor() -> Result<StateFamilyDescriptor, ActionStateError> {
    let c = |suffix: &str| {
        CanonicalId::new(format!("{HEADS}.{suffix}.v1"))
            .map_err(|_| refused("Registry head schema"))
    };
    StateFamilyDescriptor::new(
        StateFamilyId::new(HEADS).map_err(|_| refused("Registry head family"))?,
        c("key")?,
        c("value")?,
        1,
        c("strict_decode")?,
        c("invariants")?,
        c("canonical")?,
    )
    .and_then(|d| d.with_limits(192, 4096))
    .map_err(|_| refused("Registry head descriptor"))
}
pub(super) fn validate_family(registry: &StateFamilyRegistry) -> Result<(), ActionStateError> {
    let expected = descriptor()?;
    let owner = OwnerId::new("roll").map_err(|_| refused("Registry owner"))?;
    let id = StateFamilyId::new(HEADS).map_err(|_| refused("Registry head family"))?;
    let actual = registry
        .entries
        .get(&(owner, id))
        .ok_or_else(|| refused("Registry head family absent"))?;
    if actual.descriptor().canonical_value() != expected.canonical_value() {
        return Err(refused("Registry head family differs"));
    }
    Ok(())
}
fn enabled(view: &View<'_>) -> Result<(), ActionStateError> {
    if view
        .registry
        .registry_rating
        .as_ref()
        .is_none_or(|p| p.reconformance.is_none())
    {
        return Err(refused("Registry reconformance not configured"));
    }
    Ok(())
}
pub(super) fn protected(key: &StateKey) -> bool {
    key.owner.as_str() == "roll" && key.family.as_str() == HEADS
}
pub(super) fn require_head(
    view: &View<'_>,
    id: CommitToken,
    pair: &Pair,
) -> Result<(), ActionStateError> {
    let Some(binding) = &pair.reconformance else {
        return Ok(());
    };
    enabled(view)?;
    let compatibility = captured_compatibility(pair)?;
    let head = read(view, &head_key(&binding.filed, compatibility)?)?
        .ok_or_else(|| refused("Registry head absent"))?;
    if head.job != id {
        return Err(refused("Registry head job differs"));
    }
    Ok(())
}
fn retry(prior: &Pair, current: &Pair, prior_id: CommitToken) -> Result<(), ActionStateError> {
    let old = prior
        .reconformance
        .as_ref()
        .ok_or_else(|| refused("Registry retry predecessor profile"))?;
    let new = current
        .reconformance
        .as_ref()
        .ok_or_else(|| refused("Registry retry profile"))?;
    if !matches!(prior.phase.as_str(), "failed" | "interrupted" | "cancelled")
        || new.previous_attempt != Some(prior_id)
        || old.filed != new.filed
        || old.birth_rating != new.birth_rating
        || captured_compatibility(prior)? != captured_compatibility(current)?
        || field(&prior.job, "subject")? != field(&current.job, "subject")?
    {
        return Err(refused("Registry retry predecessor differs"));
    }
    Ok(())
}
pub(super) fn validate_changes(
    before: &View<'_>,
    after: &View<'_>,
    mutations: &BTreeMap<StateKey, TxnMutation>,
    identity: &RegisteredTxnIdentity,
    jobs: &BTreeSet<CommitToken>,
) -> Result<(), ActionStateError> {
    let mut expected = BTreeSet::new();
    for id in jobs {
        let current = pair(after, *id)?;
        let Some(binding) = &current.reconformance else {
            continue;
        };
        enabled(after)?;
        if value(before, "rating_jobs", *id, None)?.is_some() {
            continue;
        }
        if identity.actor != crate::GOD
            || identity.principal != (PrincipalId::Object { id: crate::GOD })
            || *id != identity.commit
        {
            return Err(refused("Registry service head invocation"));
        }
        let key = head_key(&binding.filed, captured_compatibility(&current)?)?;
        let resolved = key.resolve(after.registry)?;
        if !matches!(mutations.get(&resolved), Some(TxnMutation::Put { .. })) {
            return Err(refused("Registry head not admitted atomically"));
        }
        let prior = read(before, &key)?;
        match (binding.previous_attempt, prior) {
            (None, None) => {}
            (Some(previous), Some(head)) if previous == head.job && previous != *id => {
                retry(&pair(before, previous)?, &current, previous)?
            }
            _ => return Err(refused("Registry retry head differs")),
        }
        require_head(after, *id, &current)?;
        expected.insert(resolved);
    }
    let actual = mutations
        .keys()
        .filter(|k| protected(k))
        .cloned()
        .collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(refused("Registry head changes outside exact admission"));
    }
    Ok(())
}
// One direct predecessor read per admitted job and one linear traversal per
// group: no recursive ancestry scan per public reader or quadratic boot walk.
pub(super) fn audit(
    view: &View<'_>,
    jobs: &BTreeMap<CommitToken, Pair>,
) -> Result<(), ActionStateError> {
    let mut groups: BTreeMap<String, BTreeSet<CommitToken>> = BTreeMap::new();
    for (id, pair) in jobs {
        let Some(binding) = &pair.reconformance else {
            continue;
        };
        enabled(view)?;
        if binding.previous_attempt == Some(*id) {
            return Err(refused("Registry self retry"));
        }
        if let Some(previous) = binding.previous_attempt {
            retry(
                jobs.get(&previous)
                    .ok_or_else(|| refused("Registry retry predecessor absent"))?,
                pair,
                previous,
            )?;
        }
        groups
            .entry(
                head_key(&binding.filed, captured_compatibility(pair)?)?
                    .encoded
                    .to_string(),
            )
            .or_default()
            .insert(*id);
    }
    let retained = view
        .records
        .keys()
        .filter(|key| protected(key))
        .map(|key| key.encoded.to_string())
        .collect::<BTreeSet<_>>();
    if retained != groups.keys().cloned().collect() {
        return Err(refused("Registry head reverse completeness"));
    }
    for (key, ids) in groups {
        let key = Key {
            owner: OwnerId::new("roll").map_err(|_| refused("Registry owner"))?,
            family: StateFamilyId::new(HEADS).map_err(|_| refused("Registry head family"))?,
            encoded: key.into(),
        };
        let head = read(view, &key)?.ok_or_else(|| refused("Registry retained head absent"))?;
        let mut next = Some(head.job);
        let mut seen = BTreeSet::new();
        while let Some(id) = next {
            if !ids.contains(&id) || !seen.insert(id) {
                return Err(refused("Registry head chain differs"));
            }
            next = jobs
                .get(&id)
                .and_then(|p| p.reconformance.as_ref())
                .ok_or_else(|| refused("Registry head profile missing"))?
                .previous_attempt;
        }
        if seen != ids {
            return Err(refused("Registry unindexed attempt or branched history"));
        }
    }
    Ok(())
}
