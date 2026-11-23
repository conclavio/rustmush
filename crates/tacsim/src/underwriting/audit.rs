//! Independent complete typed/native/host/root inverse. Loaded bytes never
//! mint departure or loss authority, and runtime also requires Roll readiness.
use super::{
    CoverageError,
    families::{Families, MAX_ROWS, Row},
    host::read,
    metadata::Publications,
    policy::{Exposure, HullHead, Policy, Receipt, Terminal},
    records::{Publication, Quote},
};
use mush_core::{ClockStamp, CommitToken, DefinitionDigest, ModuleCtx, ObjId, StateFamilyHandle};
use mush_economy::EconomyStore;
use mush_roll::commerce::coverage::CoverageWriter;
use std::{
    any::TypeId,
    collections::{BTreeMap, BTreeSet},
};

#[derive(Default)]
pub(super) struct BootAuditSummary {
    pub(super) used_requests: BTreeSet<DefinitionDigest>,
    pub(super) bound_quotes: BTreeSet<CommitToken>,
    pub(super) unbound_quotes: Vec<(CommitToken, ObjId, CommitToken, i64)>,
    pub(super) known_hulls: BTreeSet<ObjId>,
    pub(super) active_policies: Vec<(CommitToken, ObjId, ClockStamp, ClockStamp)>,
}
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
) -> Result<BootAuditSummary, CoverageError> {
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
        return Ok(BootAuditSummary::default());
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
    let mut known_hulls = BTreeSet::new();
    let mut active_policies = Vec::new();
    for (hull, chain) in chains {
        known_hulls.insert(hull);
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
            if p.history.terminal().is_none() {
                let terms = &p.history.consent().terms;
                active_policies.push((
                    p.operation,
                    hull,
                    terms.starts_at.clone(),
                    terms.ends_at.clone(),
                ));
            }
            previous = Some(*p);
        }
        let latest = previous.ok_or(CoverageError::Product)?;
        if hulls.get(&format!("{:08x}", hull.0)).copied() != Some(&HullHead::from_policy(latest)?) {
            return Err(CoverageError::Product);
        }
    }
    let unbound_quotes = quotes
        .values()
        .filter(|q| !used_quotes.contains(&q.operation))
        .map(|q| {
            (
                q.operation,
                q.terms.buyer,
                q.risk.ic_open,
                q.terms.expires_at_utc,
            )
        })
        .collect();
    Ok(BootAuditSummary {
        used_requests: requests,
        bound_quotes: used_quotes,
        unbound_quotes,
        known_hulls,
        active_policies,
    })
}

/// O(1) runtime owner-slot check replacing full-table `audit::all()` on hot paths.
pub(super) fn check_runtime_context<'a>(
    ctx: &'a ModuleCtx<'_>,
) -> Result<(&'a Publications, &'a EconomyStore), CoverageError> {
    if ctx.module.as_deref() != Some(crate::NS)
        || ctx
            .w
            .module_state_owner(TypeId::of::<Publications>())
            .is_none_or(|o| o.as_str() != crate::NS)
        || ctx
            .w
            .module_state_owner(TypeId::of::<EconomyStore>())
            .is_none_or(|o| o.as_str() != "economy")
    {
        return Err(CoverageError::Authority);
    }
    let native_products = ctx
        .state::<Publications>()
        .ok_or(CoverageError::Authority)?;
    let economy = ctx
        .state::<EconomyStore>()
        .ok_or(CoverageError::Authority)?;
    Ok((native_products, economy))
}

/// O(1) point verification for a single committed [`Quote`] at runtime.
pub(super) fn verify_quote_runtime(
    ctx: &ModuleCtx<'_>,
    f: &Families,
    q: &Quote,
) -> Result<(), CoverageError> {
    let (native_products, _) = check_runtime_context(ctx)?;
    let p: &Publication = read(ctx, &f.products, q.publication.to_hex())?;
    if native_products
        .rows
        .get(&q.publication)
        .is_none_or(|native| native.as_ref() != p)
    {
        return Err(CoverageError::Product);
    }
    q.matches_publication(p)?;
    raw_open(ctx, q)
}

/// O(1) point verification for a single committed [`Policy`] at runtime.
pub(super) fn verify_policy_runtime(
    ctx: &ModuleCtx<'_>,
    f: &Families,
    service: ObjId,
    p: &Policy,
) -> Result<(), CoverageError> {
    let (_, economy) = check_runtime_context(ctx)?;
    let c = p.history.consent();
    if p.history.terminal().is_none() && c.terms.adjudicator != service {
        return Err(CoverageError::Authority);
    }
    let q: &Quote = read(ctx, &f.quotes, p.quote.to_hex())?;
    p.matches_quote(q)?;
    let actual = economy.coverage_history(p.operation).map_err(bad)?;
    if actual != &p.history {
        return Err(CoverageError::Product);
    }
    let exposure = read(ctx, &f.exposures, p.operation.to_hex())?;
    if *exposure != Exposure::from_policy(p)? {
        return Err(CoverageError::Product);
    }
    let hull = read(ctx, &f.hulls, format!("{:08x}", c.basis.hull.0))?;
    if p.history.terminal().is_none()
        && (*hull != HullHead::from_policy(p)? || hull.active != Some(p.operation))
    {
        return Err(CoverageError::Product);
    }
    Ok(())
}

/// O(1) point verification for a single committed [`Terminal`] and its [`Policy`] at runtime.
pub(super) fn verify_terminal_runtime(
    ctx: &ModuleCtx<'_>,
    f: &Families,
    service: ObjId,
    p: &Policy,
    terminal: &Terminal,
) -> Result<(), CoverageError> {
    verify_policy_runtime(ctx, f, service, p)?;
    terminal.matches_policy(p)?;
    let receipt = read(ctx, &f.receipts, p.operation.to_hex())?;
    if *receipt != Receipt::from_policy(p)? {
        return Err(CoverageError::Product);
    }
    Ok(())
}

/// Full-scan fallback used only if the bounded runtime candidate index ever overflows.
pub(super) fn request_used_scan(
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
