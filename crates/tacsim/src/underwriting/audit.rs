//! Independent complete typed/native/host/root inverse. Loaded bytes never
//! mint departure or loss authority, and runtime also requires Roll readiness.
use super::{
    CoverageError,
    families::{Families, MAX_ROWS, Row},
    metadata::Publications,
    policy::{Exposure, HullHead, Receipt},
    records::{Publication, Quote},
};
use mush_core::{CommitToken, ModuleCtx, StateFamilyHandle};
use mush_economy::EconomyStore;
use mush_roll::commerce::coverage::CoverageWriter;
use std::{
    any::TypeId,
    collections::{BTreeMap, BTreeSet},
};
fn bad(_: impl std::fmt::Display) -> CoverageError {
    CoverageError::Authority
}
fn scan<'a, R: Row>(
    ctx: &'a ModuleCtx<'_>,
    family: &StateFamilyHandle<String, R>,
) -> Result<BTreeMap<String, &'a R>, CoverageError> {
    let mut rows = BTreeMap::new();
    let mut after = None;
    loop {
        let page = ctx
            .scan_action_state(family, after.as_deref(), 128)
            .map_err(bad)?;
        for entry in page.entries() {
            let row = entry.value();
            row.validate()?;
            if row.key() != entry.encoded_key()
                || rows.len() >= MAX_ROWS as usize
                || rows.insert(row.key(), row).is_some()
            {
                return Err(CoverageError::Product);
            }
        }
        let Some(next) = page.next_after() else {
            return Ok(rows);
        };
        if page.entries().is_empty() || after.as_deref().is_some_and(|old| old >= next.as_str()) {
            return Err(CoverageError::Product);
        }
        after = Some(next.to_string());
    }
}
fn raw_open(ctx: &ModuleCtx<'_>, quote: &Quote) -> Result<(), CoverageError> {
    crate::minds::ic_sortie::audit_coverage_history(
        ctx.w,
        &quote.native_basis()?,
        quote.terms.buyer,
        None,
    )
    .map_err(bad)
}
pub(super) fn all(
    ctx: &ModuleCtx<'_>,
    f: &Families,
    writer: Option<&CoverageWriter>,
    service: Option<mush_core::ObjId>,
) -> Result<(), CoverageError> {
    if ctx.module.as_deref() != Some(crate::NS)
        || ctx
            .w
            .module_state_owner(TypeId::of::<Publications>())
            .is_none_or(|o| o.as_str() != crate::NS)
    {
        return Err(CoverageError::Authority);
    }
    let native_products = ctx
        .state::<Publications>()
        .ok_or(CoverageError::Authority)?;
    let products = scan(ctx, &f.products)?;
    let quotes = scan(ctx, &f.quotes)?;
    let policies = scan(ctx, &f.policies)?;
    let exposures = scan(ctx, &f.exposures)?;
    let hulls = scan(ctx, &f.hulls)?;
    let terminals = scan(ctx, &f.terminals)?;
    let receipts = scan(ctx, &f.receipts)?;
    let counts = scan(ctx, &f.counts)?;
    if products.len() != native_products.rows.len() {
        return Err(CoverageError::Product);
    }
    if let Some(c) = counts.get("registry") {
        if c.products != products.len() as u64
            || c.quotes != quotes.len() as u64
            || c.policies != policies.len() as u64
            || c.products == 0
        {
            return Err(CoverageError::Product);
        }
    } else if !products.is_empty() || !quotes.is_empty() || !policies.is_empty() {
        return Err(CoverageError::Product);
    }
    let enabled = writer.is_some();
    if !enabled
        && (!products.is_empty()
            || !quotes.is_empty()
            || !policies.is_empty()
            || !exposures.is_empty()
            || !hulls.is_empty()
            || !terminals.is_empty()
            || !receipts.is_empty()
            || !counts.is_empty())
    {
        return Err(CoverageError::Authority);
    }
    let retained = writer
        .map(|w| w.publications_for_boot(ctx).map_err(bad))
        .transpose()?
        .unwrap_or_default();
    let qualified_products: BTreeMap<_, _> = retained.iter().map(|p| (p.operation(), p)).collect();
    if qualified_products.len() != retained.len()
        || qualified_products.len() != products.values().filter(|p| !p.conduct.is_empty()).count()
    {
        return Err(CoverageError::Product);
    }
    for p in products.values() {
        let native = native_products
            .rows
            .get(&p.operation)
            .ok_or(CoverageError::Product)?;
        if native.as_ref() != *p {
            return Err(CoverageError::Product);
        }
        if !p.conduct.is_empty() {
            let retained = qualified_products
                .get(&p.operation)
                .ok_or(CoverageError::Product)?;
            if retained.publication_digest() != p.digest()?
                || retained.product_digest() != p.product.digest()?
                || retained.trials().len() != p.conduct.len()
                || retained
                    .trials()
                    .iter()
                    .zip(&p.conduct)
                    .any(|(proof, selection)| {
                        !super::qualification::matches_retained(selection, proof)
                    })
            {
                return Err(CoverageError::Product);
            }
        }
    }
    for q in quotes.values() {
        let p: &&Publication = products
            .get(&q.publication.to_hex())
            .ok_or(CoverageError::Product)?;
        q.matches_publication(p)?;
        raw_open(ctx, q)?;
        if let Some(selected) = &q.risk.qualified {
            let retention = qualified_products
                .get(&q.publication)
                .ok_or(CoverageError::Product)?;
            let proof = retention
                .trials()
                .iter()
                .find(|r| r.trial() == selected.trial)
                .ok_or(CoverageError::Product)?;
            if !super::qualification::matches_retained(selected, proof) {
                return Err(CoverageError::Product);
            }
            writer
                .ok_or(CoverageError::Authority)?
                .verify_qualified_quote_for_boot(ctx, q.risk.ic_open, proof)
                .map_err(bad)?;
            crate::minds::ic_sortie::audit_qualified_coverage_history(
                ctx.w,
                &q.native_basis()?,
                q.terms.buyer,
                proof,
            )
            .map_err(bad)?;
        }
    }
    let economy = ctx.state::<EconomyStore>();
    if economy.is_none() && !enabled && policies.is_empty() {
        return Ok(());
    }
    if ctx
        .w
        .module_state_owner(TypeId::of::<EconomyStore>())
        .is_none_or(|o| o.as_str() != "economy")
    {
        return Err(CoverageError::Authority);
    }
    let economy = economy.ok_or(CoverageError::Authority)?;
    let native = economy.coverage_histories().map_err(bad)?;
    if native.len() != policies.len()
        || exposures.len() != policies.len()
        || receipts.len() != terminals.len()
    {
        return Err(CoverageError::Product);
    }
    let bindings = writer
        .map(|w| w.bindings_for_boot(ctx).map_err(bad))
        .transpose()?
        .unwrap_or_default();
    if bindings.len() != policies.len() {
        return Err(CoverageError::Product);
    }
    let roots = bindings
        .iter()
        .map(|r| (r.operation(), r))
        .collect::<BTreeMap<_, _>>();
    let mut used_quotes = BTreeSet::new();
    let mut used_opens = BTreeSet::new();
    let mut terminal_ids = BTreeSet::new();
    let mut chains = BTreeMap::<_, BTreeMap<u64, _>>::new();
    let mut all_commits = BTreeSet::<CommitToken>::new();
    let mut requests = BTreeSet::new();
    for id in quotes
        .values()
        .map(|r| r.request)
        .chain(policies.values().map(|r| r.request))
        .chain(terminals.values().map(|r| r.request))
    {
        if !requests.insert(id) {
            return Err(CoverageError::Product);
        }
    }
    for p in products.values() {
        if !all_commits.insert(p.operation) {
            return Err(CoverageError::Product);
        }
    }
    for q in quotes.values() {
        if !all_commits.insert(q.operation) {
            return Err(CoverageError::Product);
        }
    }
    for p in policies.values() {
        if p.history.terminal().is_none() && Some(p.history.consent().terms.adjudicator) != service
        {
            return Err(CoverageError::Authority);
        }
        let c = p.history.consent();
        if !all_commits.insert(p.operation)
            || !used_quotes.insert(p.quote)
            || !used_opens.insert(c.basis.ic_open)
        {
            return Err(CoverageError::Product);
        }
        p.matches_quote(
            quotes
                .get(&p.quote.to_hex())
                .ok_or(CoverageError::Product)?,
        )?;
        let actual = economy.coverage_history(p.operation).map_err(bad)?;
        if actual != &p.history
            || exposures.get(&p.operation.to_hex()).copied() != Some(&Exposure::from_policy(p)?)
        {
            return Err(CoverageError::Product);
        }
        roots
            .get(&p.operation)
            .ok_or(CoverageError::Product)?
            .matches_history(actual)
            .map_err(bad)?;
        if chains
            .entry(c.basis.hull)
            .or_default()
            .insert(p.sequence, *p)
            .is_some()
        {
            return Err(CoverageError::Product);
        }
        if let Some(t) = p.history.terminal() {
            let id = t.decision().operation;
            if !all_commits.insert(id) || !terminal_ids.insert(id) {
                return Err(CoverageError::Product);
            }
            let terminal = terminals
                .get(&p.operation.to_hex())
                .ok_or(CoverageError::Product)?;
            terminal.matches_policy(p)?;
            if receipts.get(&p.operation.to_hex()).copied() != Some(&Receipt::from_policy(p)?) {
                return Err(CoverageError::Product);
            }
            let close = &terminal.close;
            crate::minds::ic_sortie::audit_coverage_history(
                ctx.w,
                &c.basis,
                c.terms.buyer,
                Some((
                    close.close,
                    close.digest,
                    close.transition,
                    &close.at_ic,
                    &close.zone,
                    close.kind,
                )),
            )
            .map_err(bad)?;
        }
    }
    if terminal_ids.len() != terminals.len() || chains.len() != hulls.len() {
        return Err(CoverageError::Product);
    }
    for history in native {
        if !policies.contains_key(&history.consent().operation.to_hex()) {
            return Err(CoverageError::Product);
        }
    }
    for (hull, chain) in chains {
        let mut previous = None;
        for (index, p) in chain.values().enumerate() {
            if p.sequence != index as u64 + 1
                || p.previous != previous.map(|old: &super::policy::Policy| old.operation)
            {
                return Err(CoverageError::Product);
            }
            if let Some(old) = previous {
                let terminal = old
                    .history
                    .terminal()
                    .ok_or(CoverageError::Product)?
                    .decision();
                let start = &p.history.consent().terms.starts_at;
                if terminal.at_ic.clock == start.clock && terminal.at_ic.at > start.at {
                    return Err(CoverageError::Clock);
                }
                if terminal.posted_at_utc > p.history.consent().accepted_at_utc {
                    return Err(CoverageError::Clock);
                }
            }
            previous = Some(*p);
        }
        let latest = previous.ok_or(CoverageError::Product)?;
        if hulls.get(&format!("{:08x}", hull.0)).copied() != Some(&HullHead::from_policy(latest)?) {
            return Err(CoverageError::Product);
        }
    }
    Ok(())
}

pub(super) fn request_used(
    ctx: &ModuleCtx<'_>,
    f: &Families,
    request: mush_core::DefinitionDigest,
) -> Result<bool, CoverageError> {
    Ok(scan(ctx, &f.quotes)?.values().any(|r| r.request == request)
        || scan(ctx, &f.policies)?
            .values()
            .any(|r| r.request == request)
        || scan(ctx, &f.terminals)?
            .values()
            .any(|r| r.request == request))
}
