//! One prepared publication across quota, artifact and Roll retention owners.
//! Every check and JSON preparation precedes all RAM mutation and deferred ops.
use super::{
    BulkError,
    artifacts::{Artifacts, Key, Part, PreparedBlob},
    invalid,
    model::{RunId, WorkKind},
    owner::{BulkStore, Lease, Prepared},
};
use mush_core::{AccountId, Ctx, ModuleOp, ModuleWrite};
use mush_roll::{ReferenceKey, ReferenceKind, RevisionRef, RollStore};
use smol_str::SmolStr;

pub(crate) fn account_key(account: AccountId) -> SmolStr {
    format!(
        "tacsim:mindbulk:account:{}",
        account.canonical_hash().to_hex()
    )
    .into()
}
fn root(account: AccountId, run: RunId) -> ReferenceKey {
    ReferenceKey {
        owner: crate::store::NS.into(),
        kind: ReferenceKind::ProvingRun,
        record: format!("{}/{}", account.canonical_hash().to_hex(), run.0),
    }
}

/// The command's final fallible call. Returns only an opaque already-published
/// worker lease; callers may subsequently enqueue it but never roll back RAM.
pub(crate) fn publish(
    ctx: &mut Ctx<'_>,
    change: Prepared,
    blob: &mut Option<PreparedBlob>,
    new_roots: Option<(RunId, Vec<RevisionRef>)>,
) -> Result<Option<Lease>, BulkError> {
    if let Some(module) = &ctx.module
        && (module != crate::store::NS
            || ctx.reg.persist_delegation(module, mush_roll::NS).is_none())
    {
        return Err(invalid("bulk publication lacks owner authority"));
    }
    let account = change.account();
    let store = ctx
        .w
        .module_state::<BulkStore>()
        .ok_or(BulkError::Missing)?;
    store.check(&change)?;
    let artifacts = ctx
        .w
        .module_state::<Artifacts>()
        .ok_or(BulkError::Missing)?;
    if let Some(blob) = blob.as_ref() {
        artifacts.check_new(blob)?;
        let run = change
            .resulting_run(blob.manifest.key.run)
            .ok_or(BulkError::Missing)?;
        let expected = match blob.manifest.key.part {
            Part::Start => Some(&run.plan.start),
            Part::Result => run.result.as_ref(),
        };
        if blob.manifest.key.account != account
            || run.plan.kind != WorkKind::Proving
            || expected != Some(&blob.manifest.artifact)
        {
            return Err(invalid("artifact is not bound to this account change"));
        }
    }
    let roll = ctx
        .w
        .module_state::<RollStore>()
        .ok_or(BulkError::Missing)?;
    let existing = roll.reference_keys(crate::store::NS, ReferenceKind::ProvingRun);
    let mut releases = Vec::new();
    let mut removals = Vec::new();
    let mut writes = vec![ModuleWrite::JsonSet {
        key: account_key(account),
        value: change.document()?,
    }];
    for run in change
        .evicted()
        .iter()
        .filter(|r| r.plan.kind == WorkKind::Proving)
    {
        let key = root(account, run.id);
        if existing.contains(&key) {
            releases.push(key);
        }
        for part in [Part::Start, Part::Result] {
            let key = Key {
                account,
                run: run.id,
                part,
            };
            writes.extend(artifacts.deletion(key)?);
            removals.push(key);
        }
    }
    let replacements = if let Some((run, refs)) = new_roots {
        let row = change.resulting_run(run).ok_or(BulkError::Missing)?;
        if row.plan.kind != WorkKind::Proving
            || refs.len() > 2
            || refs.iter().any(|r| r.key.character != row.character)
            || blob.as_ref().is_none_or(|b| {
                b.manifest.key
                    != Key {
                        account,
                        run,
                        part: Part::Start,
                    }
            })
        {
            return Err(invalid("proving source roots do not match admission"));
        }
        if refs.is_empty() {
            Vec::new()
        } else {
            vec![(root(account, run), refs)]
        }
    } else {
        Vec::new()
    };
    let roll_change = if replacements.is_empty() && releases.is_empty() {
        None
    } else {
        Some(
            roll.prepare_reference_batch(crate::store::NS, replacements, releases)
                .map_err(|e| invalid(e.to_string()))?,
        )
    };
    if let Some(p) = &roll_change {
        roll.check_prepared(p).map_err(|e| invalid(e.to_string()))?;
    }
    if let Some(change) = roll_change {
        let mut roll_ops = Vec::new();
        ctx.w
            .module_state_mut::<RollStore>()
            .expect("checked Roll owner")
            .publish_with(change, |ops| roll_ops.extend_from_slice(ops))
            .map_err(|e| invalid(e.to_string()))?;
        // No fallible operation remains after the first owner's publication.
        for operation in roll_ops {
            ctx.persist(operation);
        }
    }
    let lease = ctx
        .w
        .module_state_mut::<BulkStore>()
        .expect("checked bulk owner")
        .apply_checked(change);
    let artifacts = ctx
        .w
        .module_state_mut::<Artifacts>()
        .expect("checked artifact owner");
    for key in removals {
        artifacts.remove(key);
    }
    if let Some(blob) = blob.take() {
        artifacts.install(blob.manifest);
        writes.extend(blob.writes);
    }
    for op in writes {
        ctx.persist(ModuleOp {
            ns: crate::store::NS.into(),
            op,
        });
    }
    Ok(lease)
}

/// Recover every readable account before accepting commands. Corrupt documents
/// remain quarantined and never receive a replacement that resets their quota.
pub(crate) fn hydrate(
    ctx: &mut Ctx<'_>,
    docs: &[(SmolStr, serde_json::Value)],
    now: i64,
) -> Result<(), BulkError> {
    let store = ctx
        .w
        .module_state_mut::<BulkStore>()
        .ok_or(BulkError::Missing)?;
    for (key, value) in docs
        .iter()
        .filter(|(key, _)| key.starts_with("tacsim:mindbulk:account:"))
    {
        let suffix = key
            .strip_prefix("tacsim:mindbulk:account:")
            .expect("filtered prefix");
        let account = serde_json::from_value::<AccountId>(serde_json::Value::String(suffix.into()));
        if let Ok(account) = account {
            let bytes = super::encoding::encode(value, super::model::MAX_ACCOUNT_DOCUMENT)
                .unwrap_or_default();
            if store.load(account, &bytes).is_err() {
                tracing::error!("proving quota document is quarantined");
            }
        } else {
            tracing::error!("proving quota document key is unreadable");
        }
    }
    ctx.w
        .module_state_mut::<Artifacts>()
        .ok_or(BulkError::Missing)?
        .load(docs);
    let ids = ctx
        .w
        .module_state::<BulkStore>()
        .expect("checked store")
        .account_ids();
    for account in ids {
        let prepared = ctx
            .w
            .module_state::<BulkStore>()
            .expect("checked store")
            .prepare_recover(account, now)?;
        publish(ctx, prepared, &mut None, None)?;
    }
    super::bench_roots::recover(ctx)?;
    Ok(())
}
