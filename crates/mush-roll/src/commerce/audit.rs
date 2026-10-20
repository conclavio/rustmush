//! Complete, synchronous recovery check for the initial original-filing cut.
//!
//! This runs last, after actual source hydration and M6 retention reconciliation.
//! The gate closes before the first typed read, including an installation with
//! no roots. Missing/unavailable families, rating work or E4 replay never mean
//! a healthy empty deployment. No caller-provided list can stand in for a scan.

use mush_core::{
    ModuleCtx, StateFamilyHandle, participant::ParticipantError, reserved_shell::ShellReservations,
};
use mush_economy::EconomyStore;

use super::families::CommerceFamilies;
use super::rating::families::RatingFamilies;
use super::records::*;
use super::roots::{InitialRoots, RequiredRoot, RootAnchor};
use crate::{MAX_RECORDS, RollStore};

const PAGE: usize = 128;

/// No receipt/capability escapes this synchronous final audit. The World task
/// cannot interleave a source change or typed transaction between scan and open.
#[cfg(test)]
pub(crate) fn audit_and_open(
    ctx: &mut ModuleCtx<'_>,
    families: &CommerceFamilies,
    ratings: &RatingFamilies,
    shells: &ShellReservations,
    paid: bool,
    forks: bool,
    dependent_owners: bool,
    registry: Option<&dyn super::rating::registration::Installer>,
) -> Result<(), PublicationError> {
    audit_and_open_configured(
        ctx,
        families,
        ratings,
        shells,
        paid,
        forks,
        dependent_owners,
        false,
        registry,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn audit_and_open_configured(
    ctx: &mut ModuleCtx<'_>,
    families: &CommerceFamilies,
    ratings: &RatingFamilies,
    shells: &ShellReservations,
    paid: bool,
    forks: bool,
    dependent_owners: bool,
    entitlements: bool,
    registry: Option<&dyn super::rating::registration::Installer>,
) -> Result<(), PublicationError> {
    audit_and_open_renewal_configured(
        ctx,
        families,
        ratings,
        shells,
        paid,
        forks,
        dependent_owners,
        entitlements,
        false,
        registry,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn audit_and_open_renewal_configured(
    ctx: &mut ModuleCtx<'_>,
    families: &CommerceFamilies,
    ratings: &RatingFamilies,
    shells: &ShellReservations,
    paid: bool,
    forks: bool,
    dependent_owners: bool,
    entitlements: bool,
    renewals: bool,
    registry: Option<&dyn super::rating::registration::Installer>,
) -> Result<(), PublicationError> {
    audit_and_open_update_configured(
        ctx,
        families,
        ratings,
        shells,
        paid,
        forks,
        dependent_owners,
        entitlements,
        renewals,
        None,
        registry,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn audit_and_open_update_configured(
    ctx: &mut ModuleCtx<'_>,
    families: &CommerceFamilies,
    ratings: &RatingFamilies,
    shells: &ShellReservations,
    paid: bool,
    forks: bool,
    dependent_owners: bool,
    entitlements: bool,
    renewals: bool,
    update: Option<&mush_core::DefinitionRef>,
    registry: Option<&dyn super::rating::registration::Installer>,
) -> Result<(), PublicationError> {
    audit_and_open_owner_configured(
        ctx,
        families,
        ratings,
        shells,
        paid,
        forks,
        dependent_owners,
        entitlements,
        renewals,
        update,
        false,
        registry,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn audit_and_open_owner_configured(
    ctx: &mut ModuleCtx<'_>,
    families: &CommerceFamilies,
    ratings: &RatingFamilies,
    shells: &ShellReservations,
    paid: bool,
    forks: bool,
    dependent_owners: bool,
    entitlements: bool,
    renewals: bool,
    update: Option<&mush_core::DefinitionRef>,
    owner_qualification: bool,
    registry: Option<&dyn super::rating::registration::Installer>,
) -> Result<(), PublicationError> {
    audit_and_open_attestation_configured(
        ctx,
        families,
        ratings,
        shells,
        paid,
        forks,
        dependent_owners,
        entitlements,
        renewals,
        update,
        owner_qualification,
        false,
        registry,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn audit_and_open_attestation_configured(
    ctx: &mut ModuleCtx<'_>,
    families: &CommerceFamilies,
    ratings: &RatingFamilies,
    shells: &ShellReservations,
    paid: bool,
    forks: bool,
    dependent_owners: bool,
    entitlements: bool,
    renewals: bool,
    update: Option<&mush_core::DefinitionRef>,
    owner_qualification: bool,
    owner_attestations: bool,
    registry: Option<&dyn super::rating::registration::Installer>,
) -> Result<(), PublicationError> {
    audit_and_open_ic_configured(
        ctx,
        families,
        ratings,
        shells,
        paid,
        forks,
        dependent_owners,
        entitlements,
        renewals,
        update,
        owner_qualification,
        owner_attestations,
        None,
        registry,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn audit_and_open_ic_configured(
    ctx: &mut ModuleCtx<'_>,
    families: &CommerceFamilies,
    ratings: &RatingFamilies,
    shells: &ShellReservations,
    paid: bool,
    forks: bool,
    dependent_owners: bool,
    entitlements: bool,
    renewals: bool,
    update: Option<&mush_core::DefinitionRef>,
    owner_qualification: bool,
    owner_attestations: bool,
    ic: Option<&dyn super::ic_claims::registration::Installer>,
    registry: Option<&dyn super::rating::registration::Installer>,
) -> Result<(), PublicationError> {
    audit_and_open_house_configured(
        ctx,
        families,
        ratings,
        shells,
        paid,
        forks,
        dependent_owners,
        entitlements,
        renewals,
        update,
        owner_qualification,
        owner_attestations,
        ic,
        false,
        registry,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn audit_and_open_house_configured(
    ctx: &mut ModuleCtx<'_>,
    families: &CommerceFamilies,
    ratings: &RatingFamilies,
    shells: &ShellReservations,
    paid: bool,
    forks: bool,
    dependent_owners: bool,
    entitlements: bool,
    renewals: bool,
    update: Option<&mush_core::DefinitionRef>,
    owner_qualification: bool,
    owner_attestations: bool,
    ic: Option<&dyn super::ic_claims::registration::Installer>,
    houses: bool,
    registry: Option<&dyn super::rating::registration::Installer>,
) -> Result<(), PublicationError> {
    audit_and_open_pending_configured(
        ctx,
        families,
        ratings,
        shells,
        paid,
        forks,
        dependent_owners,
        entitlements,
        renewals,
        update,
        owner_qualification,
        owner_attestations,
        ic,
        houses,
        registry,
        false,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn audit_and_open_pending_configured(
    ctx: &mut ModuleCtx<'_>,
    families: &CommerceFamilies,
    ratings: &RatingFamilies,
    shells: &ShellReservations,
    paid: bool,
    forks: bool,
    dependent_owners: bool,
    entitlements: bool,
    renewals: bool,
    update: Option<&mush_core::DefinitionRef>,
    owner_qualification: bool,
    owner_attestations: bool,
    ic: Option<&dyn super::ic_claims::registration::Installer>,
    houses: bool,
    registry: Option<&dyn super::rating::registration::Installer>,
    pending_charters: bool,
) -> Result<(), PublicationError> {
    audit_and_open_house_renewal_configured(
        ctx,
        families,
        ratings,
        shells,
        paid,
        forks,
        dependent_owners,
        entitlements,
        renewals,
        update,
        owner_qualification,
        owner_attestations,
        ic,
        houses,
        false,
        registry,
        pending_charters,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn audit_and_open_house_renewal_configured(
    ctx: &mut ModuleCtx<'_>,
    families: &CommerceFamilies,
    ratings: &RatingFamilies,
    shells: &ShellReservations,
    paid: bool,
    forks: bool,
    dependent_owners: bool,
    entitlements: bool,
    renewals: bool,
    update: Option<&mush_core::DefinitionRef>,
    owner_qualification: bool,
    owner_attestations: bool,
    ic: Option<&dyn super::ic_claims::registration::Installer>,
    houses: bool,
    house_renewals: bool,
    registry: Option<&dyn super::rating::registration::Installer>,
    pending_charters: bool,
) -> Result<(), PublicationError> {
    audit_and_open_paid_resale_configured(
        ctx,
        families,
        ratings,
        shells,
        paid,
        forks,
        dependent_owners,
        entitlements,
        renewals,
        update,
        owner_qualification,
        owner_attestations,
        ic,
        houses,
        house_renewals,
        registry,
        pending_charters,
        false,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn audit_and_open_paid_resale_configured(
    ctx: &mut ModuleCtx<'_>,
    families: &CommerceFamilies,
    ratings: &RatingFamilies,
    shells: &ShellReservations,
    paid: bool,
    forks: bool,
    dependent_owners: bool,
    entitlements: bool,
    renewals: bool,
    update: Option<&mush_core::DefinitionRef>,
    owner_qualification: bool,
    owner_attestations: bool,
    ic: Option<&dyn super::ic_claims::registration::Installer>,
    houses: bool,
    house_renewals: bool,
    registry: Option<&dyn super::rating::registration::Installer>,
    pending_charters: bool,
    paid_resales: bool,
) -> Result<(), PublicationError> {
    audit_and_open_coverage_configured(
        ctx,
        families,
        ratings,
        shells,
        paid,
        forks,
        dependent_owners,
        entitlements,
        renewals,
        update,
        owner_qualification,
        owner_attestations,
        ic,
        houses,
        house_renewals,
        registry,
        pending_charters,
        paid_resales,
        false,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn audit_and_open_coverage_configured(
    ctx: &mut ModuleCtx<'_>,
    families: &CommerceFamilies,
    ratings: &RatingFamilies,
    shells: &ShellReservations,
    paid: bool,
    forks: bool,
    dependent_owners: bool,
    entitlements: bool,
    renewals: bool,
    update: Option<&mush_core::DefinitionRef>,
    owner_qualification: bool,
    owner_attestations: bool,
    ic: Option<&dyn super::ic_claims::registration::Installer>,
    houses: bool,
    house_renewals: bool,
    registry: Option<&dyn super::rating::registration::Installer>,
    pending_charters: bool,
    paid_resales: bool,
    coverage: bool,
) -> Result<(), PublicationError> {
    ctx.state_mut::<RollStore>()
        .ok_or(PublicationError::Unavailable)?
        .close_commerce_for_audit()
        .map_err(|_| PublicationError::Unavailable)?;
    let ((generation, source_epoch), ic_inventory, follow) = {
        let roll = ctx
            .state::<RollStore>()
            .ok_or(PublicationError::Unavailable)?;
        let stamp = roll
            .commerce_audit_base()
            .map_err(|_| PublicationError::Unavailable)?;
        audit(
            ctx,
            roll,
            families,
            ratings,
            shells,
            paid,
            dependent_owners,
            entitlements,
            renewals,
            update,
            owner_attestations,
            houses,
            house_renewals,
        )?;
        super::owner_qualification::history::audit(
            ctx,
            roll,
            families,
            ratings,
            owner_qualification,
        )?;
        super::parent_selector::audit(ctx, roll, families, ratings, forks)?;
        let stopped = ratings
            .stopped_executions(ctx)
            .map_err(|_| PublicationError::Mismatch)?;
        match registry {
            // Mandatory even for an empty typed list: private orphaned work or
            // quarantined quota must not turn into vacuous readiness.
            Some(owner) => owner
                .audit_stopped_jobs(ctx, &stopped)
                .map_err(|_| PublicationError::Mismatch)?,
            None if stopped.is_empty() => {}
            None => return Err(PublicationError::Unavailable),
        }
        super::conduct::history::audit(ctx, roll, families, roll.conduct.configuration().is_ok())?;
        super::coverage::audit::all(ctx, roll, families, coverage)?;
        super::coverage::publication::all(ctx, roll, families, coverage)?;
        let ic_inventory = match ic {
            Some(owner) => Some(owner.audit(ctx, roll, families, ratings)?),
            None => {
                super::ic_claims::audit::disabled(ctx, families)?;
                None
            }
        };
        super::charter_pending::audit::audit(
            ctx,
            families,
            pending_charters,
            ic_inventory.as_ref(),
        )?;
        super::paid_resale::audit::audit(
            ctx,
            families,
            ratings,
            paid_resales,
            ic_inventory.as_ref(),
        )?;
        let follow = super::follow::rebuild(ctx, families, ratings)?;
        (stamp, ic_inventory, follow)
    };
    ctx.state_mut::<RollStore>()
        .ok_or(PublicationError::Unavailable)?
        .install_ic_claim_audit((generation, source_epoch), ic_inventory)
        .map_err(|_| PublicationError::Stale)?;
    ctx.state_mut::<RollStore>()
        .ok_or(PublicationError::Unavailable)?
        .install_follow_audit((generation, source_epoch), follow)
        .map_err(|_| PublicationError::Stale)?;
    ctx.state_mut::<RollStore>()
        .ok_or(PublicationError::Unavailable)?
        .finish_commerce_audit(generation, source_epoch)
        .map_err(|_| PublicationError::Stale)
}

pub(super) fn read<'a, R: Record>(
    ctx: &'a ModuleCtx<'_>,
    handle: &StateFamilyHandle<String, R>,
    key: String,
) -> Result<&'a R, PublicationError> {
    let row = ctx
        .action_state(handle, &key)
        .map_err(|_| PublicationError::Unavailable)?
        .ok_or(PublicationError::Missing)?;
    row.validate()?;
    if row.key() != key {
        return Err(PublicationError::Mismatch);
    }
    Ok(row)
}

/// Only one page of borrowed entries and its cursor are resident. The existing
/// typed store owns aggregate byte/record bounds; this cut independently bounds
/// each complete family to the Roll retention owner's record ceiling.
pub(super) fn scan<R: Record>(
    ctx: &ModuleCtx<'_>,
    handle: &StateFamilyHandle<String, R>,
    mut inspect: impl FnMut(&R) -> Result<(), PublicationError>,
) -> Result<u64, PublicationError> {
    let mut after = None;
    let mut count = 0_u64;
    loop {
        let page = ctx
            .scan_action_state(handle, after.as_deref(), PAGE)
            .map_err(|_| PublicationError::Unavailable)?;
        for entry in page.entries() {
            count = count.checked_add(1).ok_or(PublicationError::Capacity)?;
            if count > MAX_RECORDS as u64 {
                return Err(PublicationError::Capacity);
            }
            let row = entry.value();
            row.validate()?;
            if row.key() != entry.encoded_key() {
                return Err(PublicationError::Mismatch);
            }
            inspect(row)?;
        }
        let Some(next) = page.next_after() else {
            return Ok(count);
        };
        // Defensive progress check; the real typed-store scanner already
        // enforces canonical ascending keys and bounded pages.
        if page.entries().is_empty() || after.as_deref().is_some_and(|old| old >= next.as_str()) {
            return Err(PublicationError::Mismatch);
        }
        after = Some(next.to_string());
    }
}

#[cfg(test)]
pub(super) fn check_status_keys_for_test(
    ctx: &ModuleCtx<'_>,
    families: &CommerceFamilies,
) -> Result<(), PublicationError> {
    scan(ctx, &families.status, |_| Ok(())).map(|_| ())
}

#[derive(Default)]
struct Allocated {
    count: u64,
    max: u64,
}
impl Allocated {
    fn observe(&mut self, id: u64) -> Result<(), PublicationError> {
        self.count = self
            .count
            .checked_add(1)
            .ok_or(PublicationError::Capacity)?;
        self.max = self.max.max(id);
        Ok(())
    }
    fn verify(&self, next: u64) -> Result<(), PublicationError> {
        // Initial filing allocates only in its one committed transaction.
        // IDs are never separately reserved/burned or legitimately deleted.
        let used = next.checked_sub(1).ok_or(PublicationError::Invalid)?;
        if self.count != used || self.max != used {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
fn audit(
    ctx: &ModuleCtx<'_>,
    roll: &RollStore,
    f: &CommerceFamilies,
    ratings: &RatingFamilies,
    shells: &ShellReservations,
    paid: bool,
    dependent_owners: bool,
    entitlements: bool,
    renewals: bool,
    update: Option<&mush_core::DefinitionRef>,
    owner_attestations: bool,
    houses: bool,
    house_renewals: bool,
) -> Result<(), PublicationError> {
    let deployment = read(ctx, &f.deployment, "registry".into())?;
    if scan(ctx, &f.deployment, |_| Ok(()))? != 1 {
        return Err(PublicationError::Mismatch);
    }
    shells
        .verify_deployment(ctx, deployment.initialized_by)
        .map_err(|_| PublicationError::Mismatch)?;
    // Actual registered receipt/page/job/completion families are all audited,
    // including empty families. The real Registry producer supplies this seam;
    // it is not implemented here as a digest-only or always-success adapter.
    ratings
        .audit_for_collection(ctx, roll)
        .map_err(|_| PublicationError::Unavailable)?;
    let economic = ctx
        .state::<EconomyStore>()
        .ok_or(PublicationError::Unavailable)?;
    economic
        .filing_receipt_audit()
        .require_complete()
        .map_err(|_| PublicationError::Unavailable)?;

    let mut dependent_count = 0_u64;
    let operation_count = scan(ctx, &f.operations, |op| {
        if op.parent.is_some() {
            if !dependent_owners {
                return Err(PublicationError::Unavailable);
            }
            dependent_count = dependent_count
                .checked_add(1)
                .ok_or(PublicationError::Capacity)?;
        }
        verify_operation(ctx, roll, f, ratings, op)?;
        shells
            .verify_consumption(
                ctx,
                op.shell,
                op.charter.object(),
                op.shell_receipt,
                op.commit,
            )
            .map_err(|_| PublicationError::Mismatch)?;
        economic
            .filing_receipt_audit()
            .verify(&op.audit_subject()?)
            .map_err(|_| PublicationError::Unavailable)
    })?;
    // Mandatory reverse proof, including an empty family: each immutable
    // owner selector belongs to one real dependent operation and no other row.
    let binding_count = scan(ctx, &f.owner_parents, |binding| {
        if !dependent_owners {
            return Err(PublicationError::Unavailable);
        }
        let op = read(ctx, &f.operations, binding.operation.to_string())?;
        let filed = read(ctx, &f.filed, binding.filed.key())?;
        let term = read(
            ctx,
            &f.terms,
            OwnerTerm::encoded_key(binding.licence, binding.term),
        )?;
        super::dependent_owner::verify_history(ctx, roll, f, ratings, binding, filed, term, op)
    })?;
    if binding_count != dependent_count {
        return Err(PublicationError::Mismatch);
    }
    // History is an independent retained obligation. This reverse join also
    // rejects a reverted/missing typed deployment whose counters happen to
    // agree with its surviving rows. Only actual completed E4 replay yields
    // this iterator; current titles never substitute for historical receipts.
    for subject in economic
        .filing_receipt_audit()
        .receipts()
        .map_err(|_| PublicationError::Unavailable)?
    {
        let op = read(ctx, &f.operations, subject.commit().to_string())?;
        if &op.audit_subject()? != subject {
            return Err(PublicationError::Mismatch);
        }
    }
    // Actual shell rows are a separate owner-controlled obligation. The
    // forward pass above detects a completely missing reservation family;
    // this reverse pass rejects consumed shells with no matching filing.
    shells
        .audit(ctx, |shell| {
            let op = read(ctx, &f.operations, shell.filing().to_string())
                .map_err(|_| ParticipantError::Authority)?;
            if op.shell != shell.key()
                || op.charter.object() != shell.object()
                || op.shell_receipt != shell.reservation()
                || op.commit != shell.filing()
            {
                return Err(ParticipantError::Authority);
            }
            Ok(())
        })
        .map_err(|_| PublicationError::Mismatch)?;
    let update_count = super::line_update::audit::audit(ctx, roll, f, ratings, update)?;
    let attestation_count =
        super::owner_attestation::history::audit(ctx, roll, f, ratings, owner_attestations)?;
    let mut lines = Allocated::default();
    let mut filed = Allocated::default();
    let mut licences = Allocated::default();
    let charter_count = scan(ctx, &f.charters, |row| {
        let op = read(ctx, &f.operations, row.operation.to_string())?;
        if op.charter != row.id {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    })?;
    scan(ctx, &f.lines, |row| {
        let op = read(ctx, &f.operations, row.operation.to_string())?;
        if op.line != row.id {
            return Err(PublicationError::Mismatch);
        }
        lines.observe(row.id.get())
    })?;
    scan(ctx, &f.filed, |row| {
        if row.format == 3 {
            read(ctx, &f.line_updates, row.operation.to_string())?.check_filed(row)?;
        } else {
            let op = read(ctx, &f.operations, row.operation.to_string())?;
            if op.filed != row.content.id() {
                return Err(PublicationError::Mismatch);
            }
        }
        filed.observe(row.content.id().get())
    })?;
    scan(ctx, &f.licences, |row| {
        let op = read(ctx, &f.operations, row.operation.to_string())?;
        if op.licence != row.id {
            return Err(PublicationError::Mismatch);
        }
        licences.observe(row.id.get())
    })?;
    let term_count = scan(ctx, &f.terms, |row| {
        if row.generation.get() > 1 {
            let operation = read(ctx, &f.owner_attestations, row.operation.to_hex())?;
            if &operation.next != row {
                return Err(PublicationError::Mismatch);
            }
            return Ok(());
        }
        let op = read(ctx, &f.operations, row.operation.to_string())?;
        if op.licence != row.licence || op.term != row.generation {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    })?;
    let status_count = scan(ctx, &f.status, |row| {
        read(ctx, &f.filed, row.filed.key())?;
        super::withdrawal::audit::status(ctx, f, row)
    })?;
    super::withdrawal::audit::causes(ctx, roll, f)?;
    super::registry_recall::audit::causes(ctx, roll, f, ratings)?;
    super::registry_revocation::audit::causes(ctx, f, ratings)?;
    super::facts::audit::causes(ctx, roll, f, ratings)?;
    let identity_count = scan(ctx, &f.identities, |row| {
        let file = read(ctx, &f.filed, row.filed.key())?;
        if file.content.revision() != row.revision || file.duplicate != row.duplicate {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    })?;
    let position_count = scan(ctx, &f.positions, |row| {
        let file = read(ctx, &f.filed, row.filed.key())?;
        if file.content.position().line != row.line {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    })?;
    let owner_licences = licences.count;
    let paid_roots = super::paid_audit::audit_with_house_renewals(
        ctx,
        roll,
        f,
        ratings,
        paid,
        entitlements,
        renewals,
        houses,
        house_renewals,
        |id| licences.observe(id),
    )?;
    lines.verify(deployment.next_line)?;
    filed.verify(deployment.next_filed)?;
    licences.verify(deployment.next_licence)?;
    if [charter_count, lines.count, owner_licences, position_count]
        .into_iter()
        .any(|n| n != operation_count)
    {
        return Err(PublicationError::Mismatch);
    }
    if term_count
        != operation_count
            .checked_add(attestation_count)
            .ok_or(PublicationError::Capacity)?
    {
        return Err(PublicationError::Mismatch);
    }
    let filed_count = operation_count
        .checked_add(update_count)
        .ok_or(PublicationError::Capacity)?;
    if [filed.count, status_count, identity_count]
        .into_iter()
        .any(|n| n != filed_count)
    {
        return Err(PublicationError::Mismatch);
    }
    // Reverse audit rejects dangling/extra roots as well as wrong targets.
    // Forward checks above detect even the complete absence of every root.
    for entry in roll.commercial_roots() {
        let (anchor, target) = entry.map_err(|_| PublicationError::Mismatch)?;
        let identity = match &anchor {
            RootAnchor::ConductJob(job, digest) => {
                let row = read(ctx, &f.conduct_jobs, job.to_hex())?;
                if row.digest()? != *digest {
                    return Err(PublicationError::Mismatch);
                }
                row.subject().revision()
            }
            RootAnchor::TermLapse(licence, generation, operation, digest) => {
                super::facts::audit::root_subject(
                    ctx,
                    f,
                    *licence,
                    *generation,
                    *operation,
                    *digest,
                )?
            }
            RootAnchor::Filed(id) => read(ctx, &f.filed, id.key())?.content.revision(),
            RootAnchor::Operation(token) => read(ctx, &f.operations, token.to_string())?.revision,
            RootAnchor::UpdateOperation(token) => {
                read(ctx, &f.line_updates, token.to_string())?.revision
            }
            RootAnchor::Coverage(commit, digest) => {
                super::coverage::audit::root(ctx, f, *commit, *digest)?
            }
            RootAnchor::CoveragePublication(commit, trial, digest) => {
                super::coverage::publication::root(ctx, f, *commit, *trial, *digest)?
            }
            RootAnchor::IcClaim(commit, digest) => {
                let claim = read(ctx, &f.ic_claims, commit.to_hex())?;
                if claim.open.digest()? != *digest {
                    return Err(PublicationError::Mismatch);
                }
                claim.open.revision
            }
            RootAnchor::OwnerParent(licence, owner) => {
                read(
                    ctx,
                    &f.owner_parents,
                    super::dependent_owner::OwnerParentBinding::key_for(*licence, *owner),
                )?
                .parent
                .parent
                .revision
            }
            RootAnchor::OwnerAttestation(commit, digest)
            | RootAnchor::OwnerAttestationParent(commit, digest) => {
                let operation = read(ctx, &f.owner_attestations, commit.to_hex())?;
                if operation.digest()? != *digest {
                    return Err(PublicationError::Mismatch);
                }
                operation
                    .roots()?
                    .into_iter()
                    .find(|root| root.anchor == anchor)
                    .ok_or(PublicationError::Mismatch)?
                    .identity
            }
            RootAnchor::OwnerQualification(commit, digest) => {
                let operation = read(ctx, &f.owner_qualification_operations, commit.to_hex())?;
                if operation.digest()? != *digest {
                    return Err(PublicationError::Mismatch);
                }
                operation.selected_parent.revision
            }
            RootAnchor::RegistryRecall(commit, digest) => {
                let required = super::registry_recall::audit::cause_root(ctx, f, ratings, *commit)?;
                if required.anchor != RootAnchor::RegistryRecall(*commit, *digest) {
                    return Err(PublicationError::Mismatch);
                }
                required.identity
            }
            RootAnchor::RegistryRevocation(commit, digest) => {
                let operation = read(ctx, &f.registry_revocations, commit.to_hex())?;
                if operation.digest()? != *digest {
                    return Err(PublicationError::Mismatch);
                }
                operation.revision
            }
            RootAnchor::CharterTransferOffer(_, _) | RootAnchor::CharterTransfer(_, _, _, _) => {
                let actual = super::charter_transfer::audit::root(ctx, f, &anchor)?;
                if actual.anchor != anchor {
                    return Err(PublicationError::Mismatch);
                }
                actual.identity
            }
            RootAnchor::PaidResaleFunding(..) | RootAnchor::PaidResaleTerminal(..) => {
                let actual = super::paid_resale::audit::root(ctx, f, ratings, &anchor)?;
                if actual.anchor != anchor {
                    return Err(PublicationError::Mismatch);
                }
                actual.identity
            }
            RootAnchor::CharterPendingOffer(..)
            | RootAnchor::CharterPendingFunding(..)
            | RootAnchor::CharterPendingTerminal(..) => {
                let actual = super::charter_pending::audit::root(ctx, f, &anchor)?;
                if actual.anchor != anchor {
                    return Err(PublicationError::Mismatch);
                }
                actual.identity
            }
            RootAnchor::AuthorWithdrawal(commit, digest) => {
                let operation = read(ctx, &f.withdrawals, commit.to_hex())?;
                if operation.digest()? != *digest {
                    return Err(PublicationError::Mismatch);
                }
                operation.revision
            }
            RootAnchor::DraftRetirement(commit, digest) => {
                let operation = read(ctx, &f.authored_retirements, commit.to_hex())?;
                if operation.digest()? != *digest {
                    return Err(PublicationError::Mismatch);
                }
                operation.parent
            }
            RootAnchor::PaidOperation(_)
            | RootAnchor::RenewalOperation(_)
            | RootAnchor::HouseRenewalOperation(_) => {
                *paid_roots.get(&anchor).ok_or(PublicationError::Missing)?
            }
            RootAnchor::Term(_, _) if paid_roots.contains_key(&anchor) => {
                *paid_roots.get(&anchor).ok_or(PublicationError::Missing)?
            }
            RootAnchor::Term(id, generation) => {
                let term = read(ctx, &f.terms, OwnerTerm::encoded_key(*id, *generation))?;
                match term.revision {
                    RevisionPolicy::Pin { revision, .. } => revision,
                    RevisionPolicy::Follow { .. } => return Err(PublicationError::Mismatch),
                }
            }
            RootAnchor::Rating { .. } | RootAnchor::RatingJob(_) | RootAnchor::RatingPending(_) => {
                ratings
                    .root_subject(ctx, &anchor)
                    .map_err(|_| PublicationError::Unavailable)?
                    .ok_or(PublicationError::Missing)?
            }
        };
        if target != identity.revision() {
            return Err(PublicationError::Mismatch);
        }
        roll.check_required_root(&RequiredRoot { anchor, identity })
            .map_err(|_| PublicationError::Mismatch)?;
    }
    Ok(())
}

fn verify_operation(
    ctx: &ModuleCtx<'_>,
    roll: &RollStore,
    f: &CommerceFamilies,
    ratings: &RatingFamilies,
    op: &InitialFilingOperation,
) -> Result<(), PublicationError> {
    let charter = read(ctx, &f.charters, op.charter.key())?;
    let line = read(ctx, &f.lines, op.line.key())?;
    let filed = read(ctx, &f.filed, op.filed.key())?;
    let licence = read(ctx, &f.licences, op.licence.key())?;
    let term = read(ctx, &f.terms, OwnerTerm::encoded_key(op.licence, op.term))?;
    let status = read(ctx, &f.status, op.filed.key())?;
    let identity = read(ctx, &f.identities, filed.duplicate.to_string())?;
    let position = read(
        ctx,
        &f.positions,
        format!("{}/0000000000000001", op.line.key()),
    )?;
    if charter.operation != op.commit
        || charter.shell_receipt != op.shell_receipt
        || charter.line != op.line
        || line.operation != op.commit
        || line.charter != op.charter
        || (line.format == 1 && line.head != op.filed)
        || filed.content.position().charter != op.charter
        || filed.content.position().line != op.line
        || licence.operation != op.commit
        || licence.charter != op.charter
        || licence.line != op.line
        || licence.term.get() < op.term.get()
        || identity.revision != op.revision
        || identity.duplicate != filed.duplicate
        || identity.filed != op.filed
        || position.filed != op.filed
        || term.window.issued_at() != filed.content.filed_at()
    {
        return Err(PublicationError::Mismatch);
    }
    super::withdrawal::audit::status(ctx, f, status)?;
    let rating = ratings
        .resolve_receipt(ctx, &filed.rating)
        .map_err(|_| PublicationError::Unavailable)?;
    if rating.reference().map_err(|_| PublicationError::Invalid)? != filed.rating
        || rating.subject().revision() != op.revision
        || rating.subject().definition() != filed.definition
        || rating.subject().schema() != line.schema_digest
    {
        return Err(PublicationError::Mismatch);
    }
    let reference = op.revision.revision();
    let revision = roll
        .revision(reference.key.character, reference.key)
        .map_err(|_| PublicationError::Unavailable)?;
    if !op
        .revision
        .matches(revision)
        .map_err(|_| PublicationError::Invalid)?
        || schema_line_digest(revision.envelope())? != line.schema_digest
        || canonical_duplicate_digest(revision)? != filed.duplicate
    {
        return Err(PublicationError::Mismatch);
    }
    match &op.parent {
        Some(parent) => {
            let binding = read(
                ctx,
                &f.owner_parents,
                super::dependent_owner::OwnerParentBinding::key_for(op.licence, parent.owner),
            )?;
            super::dependent_owner::verify_history(
                ctx, roll, f, ratings, binding, filed, term, op,
            )?;
        }
        None => {
            revision
                .require_original_access()
                .map_err(|_| PublicationError::Mismatch)?;
            if filed.format != 1
                || filed.content.parent().is_some()
                || term.format != 1
                || term.parent.is_some()
            {
                return Err(PublicationError::Mismatch);
            }
        }
    }
    filed
        .content
        .check_links(&super::dependent_owner::ContentView {
            ctx,
            roll,
            families: f,
        })
        .map_err(|_| PublicationError::Mismatch)?;
    for required in InitialRoots::from_records(filed, term, op)?.roots() {
        roll.check_required_root(required)
            .map_err(|_| PublicationError::Mismatch)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn committed_allocation_audit_refuses_holes_and_all_missing_business_rows() {
        assert_eq!(
            Allocated::default().verify(2),
            Err(PublicationError::Mismatch)
        );
        let mut ids = Allocated::default();
        ids.observe(1).unwrap();
        ids.observe(3).unwrap();
        assert_eq!(ids.verify(4), Err(PublicationError::Mismatch));
        ids.observe(2).unwrap();
        assert_eq!(ids.verify(4), Ok(()));
        assert_eq!(ids.verify(5), Err(PublicationError::Mismatch));
        assert_eq!(Allocated::default().verify(1), Ok(()));
        assert_eq!(
            Allocated::default().verify(0),
            Err(PublicationError::Invalid)
        );
    }
}
