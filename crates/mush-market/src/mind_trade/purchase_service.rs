//! Volatile continuation custody for already funded original purchases.
//! The permanent Market/Roll/E4 records are the inventory and completion proof.
//! A timer supplies no authority and no missing reply is treated as absence.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mush_core::action::{IdempotencyKeyText, PerceivedRef};
use mush_core::read_service::OwnerReadHandle;
use mush_core::submission_observation::SubmissionObservation;
use mush_core::{
    ArgShape, CmdError, CommandCompatibility, CommandSpec, CommitToken, ContinuationHandle, Ctx,
    DefinitionRef, Flags, ModError, ModuleCtx, ObjId, ObjKind, Registrar, TickCadence,
};
use mush_roll::commerce::purchase::PurchasePhase;
use mush_roll::commerce::sale_reader::{OriginalSaleReader, OriginalSaleRequest};

use super::Families;
use super::purchase_guard::PurchaseGuard;
use super::purchase_provider::{OriginalPurchaseConfig, PurchaseFamilies};
use super::purchase_records::{GRANT, Purchase, REFUND};
use super::records::OriginalSubject;

const COMMAND: &str = "@mindpurchase";
const TICK: &str = "original_mind_purchase_completion";
const MAX_ATTEMPTS: usize = 64;
const SCAN_ROWS: usize = 16;
const RETRY_SECONDS: i64 = 1;

#[derive(Clone)]
struct Service {
    config: OriginalPurchaseConfig,
    offers: Families,
    families: PurchaseFamilies,
    guard: PurchaseGuard,
    reader: OwnerReadHandle<OriginalSaleReader>,
    actions: [DefinitionRef; 3],
    continuation: ContinuationHandle,
    runtime: Arc<Mutex<Runtime>>,
}

#[derive(Default)]
struct Runtime {
    enabled: bool,
    after: Option<String>,
    schedule_cursor: Option<CommitToken>,
    entries: BTreeMap<CommitToken, Entry>,
}
impl Runtime {
    fn admit(&mut self, operation: CommitToken, buyer: ObjId) -> Result<&mut Entry, CmdError> {
        if !self.entries.contains_key(&operation) && self.entries.len() >= MAX_ATTEMPTS {
            return Err(unavailable());
        }
        let entry = self.entries.entry(operation).or_insert(Entry {
            buyer,
            schedule_after: 0,
            attempt: None,
        });
        if entry.buyer != buyer {
            return Err(unavailable());
        }
        Ok(entry)
    }
}
struct Entry {
    buyer: ObjId,
    schedule_after: i64,
    // The action remains attached to the exact observation, including after
    // the quote expires. Unknown must not be replaced with a refund attempt.
    attempt: Option<(DefinitionRef, SubmissionObservation)>,
}

pub(super) fn register(
    reg: &mut Registrar<'_>,
    config: &OriginalPurchaseConfig,
    offers: Families,
    families: PurchaseFamilies,
    guard: PurchaseGuard,
) -> Result<(), ModError> {
    let reader = offers.reader.clone();
    reg.classify_commands(CommandCompatibility::Authoritative);
    reg.command(
        CommandSpec {
            name: COMMAND.into(),
            switches: vec!["continue".into()],
            shape: ArgShape::Raw,
            help: "Resume your retained original mind purchase.".into(),
            ..CommandSpec::default()
        },
        Arc::new(continue_command),
    )?;
    let continuation = reg.command_continuation(COMMAND, &["continue"])?;
    reg.state(Service {
        config: config.clone(),
        offers,
        families,
        guard,
        reader,
        actions: config.actions()?,
        continuation,
        runtime: Arc::new(Mutex::new(Runtime::default())),
    });
    reg.tick(
        TICK,
        TickCadence::Coarse(Duration::from_secs(1)),
        Arc::new(
            |ctx: &mut Ctx<'_>, _: &str, _: Option<mush_core::SceneId>| {
                if let Err(error) = tick(ctx) {
                    tracing::debug!(%error, "original purchase continuation retained");
                }
            },
        ),
    )
}

/// Called only after the actual complete paired final audit. Server boot first
/// reconciles every retained authoritative intent, then hydrates all owners and
/// runs these final hooks; a failed hook never starts the world task.
pub(crate) fn enable(ctx: &ModuleCtx<'_>) -> Result<(), ModError> {
    let service = ctx
        .state::<Service>()
        .ok_or_else(|| ModError::msg("purchase service missing"))?;
    let mut runtime = service
        .runtime
        .lock()
        .map_err(|_| ModError::msg("purchase service unavailable"))?;
    if runtime.enabled || !runtime.entries.is_empty() {
        return Err(ModError::msg("purchase service was already opened"));
    }
    runtime.enabled = true;
    Ok(())
}

fn unavailable() -> CmdError {
    CmdError::msg("Retained purchase is temporarily unavailable.")
}
fn service(ctx: &Ctx<'_>) -> Result<Service, CmdError> {
    ctx.state::<Service>().cloned().ok_or_else(unavailable)
}
fn live_buyer(ctx: &Ctx<'_>, buyer: ObjId) -> bool {
    ctx.w.get(buyer).is_some_and(|object| {
        object.kind == ObjKind::Player && !object.flags.contains(Flags::GOING)
    })
}

fn tick(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let service = service(ctx)?;
    let mut runtime = service.runtime.lock().map_err(|_| unavailable())?;
    if !runtime.enabled {
        return Ok(());
    }
    // Terminal readback can finish even when the original observer has no ACK
    // or the buyer is now offline/GOING. Every retained owner is checked first.
    let mut finished = Vec::new();
    for (operation, entry) in &runtime.entries {
        match service
            .families
            .checked_operation(ctx, &service.config, &service.offers, *operation)
        {
            Ok(row) if !matches!(row.phase, PurchasePhase::Funded {}) => finished.push(*operation),
            Ok(_) if entry.attempt.is_none() && !live_buyer(ctx, entry.buyer) => {
                finished.push(*operation)
            }
            Err(_) if entry.attempt.is_none() => finished.push(*operation),
            _ => {}
        }
    }
    for operation in finished {
        runtime.entries.remove(&operation);
    }
    // This is a bounded cursor over actual permanent operations, not a second
    // mutable funding index. No volatile entry is needed before a real wake.
    let page = ctx
        .scan_action_state(
            &service.families.operations,
            runtime.after.as_deref(),
            SCAN_ROWS,
        )
        .map_err(|_| unavailable())?;
    for row in page.entries().iter().map(|entry| entry.value()) {
        if runtime.entries.len() >= MAX_ATTEMPTS {
            break;
        }
        if matches!(row.phase, PurchasePhase::Funded {}) && live_buyer(ctx, row.quote.buyer) {
            runtime.admit(row.operation, row.quote.buyer)?;
        }
    }
    runtime.after = page.next_after().map(ToString::to_string);
    let now = ctx.w.now();
    let mut buyers = BTreeSet::new();
    // Rotate the starting operation even when another buyer always has work.
    // Advancing only to the last scheduled buyer could repeatedly select the
    // same first operation for a buyer with several unresolved purchases.
    let keys = runtime.entries.keys().copied().collect::<Vec<_>>();
    let start = runtime
        .schedule_cursor
        .and_then(|cursor| keys.iter().position(|key| *key > cursor))
        .unwrap_or(0);
    let order = keys
        .iter()
        .cycle()
        .skip(start)
        .take(keys.len())
        .copied()
        .collect::<Vec<_>>();
    runtime.schedule_cursor = order.first().copied();
    for operation in order {
        let Some(entry) = runtime.entries.get_mut(&operation) else {
            continue;
        };
        if entry.schedule_after > now
            || ctx.authoritative_pending(entry.buyer)
            || !live_buyer(ctx, entry.buyer)
            || !buyers.insert(entry.buyer)
        {
            continue;
        }
        entry.schedule_after = now.saturating_add(RETRY_SECONDS);
        // Refusal or dropped scheduling retains the entry. Neither produces
        // NoIntent; a later tick retries through the same actual command gate.
        if let Err(error) =
            ctx.schedule_continuation(&service.continuation, entry.buyer, &operation.to_hex(), 0)
        {
            tracing::debug!(%error, "purchase continuation scheduling retained");
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Choice {
    Grant,
    Refund,
    Retain,
}

fn choose(ctx: &Ctx<'_>, service: &Service, row: &Purchase) -> Result<Choice, CmdError> {
    if ctx.w.now() < row.quote.quoted_at_utc {
        return Ok(Choice::Retain);
    }
    if ctx.w.now() >= row.quote.expires_at_utc
        || row.participants != service.config.participants
        || row.quote.policy
            != service
                .config
                .quotes
                .policy
                .digest()
                .map_err(|_| unavailable())?
        || row.quote.subject.compatibility != service.config.quotes.manifest.digest()
        || service.guard.current(ctx.w, row.quote.buyer).is_err()
    {
        return Ok(Choice::Refund);
    }
    // Generic owner/readiness/identity failures are not proof of ineligibility.
    // In particular a Refused read may conceal an unavailable rating owner.
    let Ok(facts) = ctx.read_owner(
        &service.reader,
        OriginalSaleRequest::listed(row.quote.subject.seller, row.quote.subject.owner_licence),
    ) else {
        return Ok(Choice::Retain);
    };
    if facts.requester() != row.quote.buyer {
        return Ok(Choice::Retain);
    }
    if OriginalSubject::capture(&facts) != row.quote.subject
        || facts.minimum() != row.quote.product.class
    {
        return Ok(Choice::Refund);
    }
    // Actual Grant repeats account identity, venue and all owner/E4 facts at
    // preparation and the pre-intent custody boundary. This read grants none.
    Ok(Choice::Grant)
}

fn continue_command(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if !ctx.switches.has("continue") {
        return Err(unavailable());
    }
    let operation = CommitToken::from_hex(ctx.args.raw()).map_err(|_| unavailable())?;
    let service = service(ctx)?;
    let mut runtime = service.runtime.lock().map_err(|_| unavailable())?;
    if !runtime.enabled || !live_buyer(ctx, ctx.actor) {
        return Err(unavailable());
    }
    let row =
        match service
            .families
            .checked_operation(ctx, &service.config, &service.offers, operation)
        {
            Ok(row) => row,
            Err(_) => {
                if runtime
                    .entries
                    .get(&operation)
                    .is_some_and(|entry| entry.attempt.is_none())
                {
                    runtime.entries.remove(&operation);
                }
                return Err(unavailable());
            }
        };
    if row.quote.buyer != ctx.actor {
        return Err(unavailable());
    }
    if !matches!(row.phase, PurchasePhase::Funded {}) {
        runtime.entries.remove(&operation);
        return Ok(());
    }
    let entry = runtime.admit(operation, row.quote.buyer)?;
    if let Some((_, observation)) = &mut entry.attempt {
        if observation.take_terminal().is_none() {
            return Ok(());
        }
        // Consume exactly once. Both terminal absence dispositions permit a
        // fresh eligibility decision; neither alone selects Refund.
        entry.attempt = None;
    }
    if ctx.authoritative_pending(ctx.actor) {
        return Ok(());
    }
    let choice = choose(ctx, &service, &row)?;
    if choice == Choice::Retain {
        // No unresolved attempt exists. Permanent rows retain the obligation;
        // release only this volatile scan slot so other buyers can progress.
        runtime.entries.remove(&operation);
        return Ok(());
    }
    let id = if choice == Choice::Grant {
        GRANT
    } else {
        REFUND
    };
    let action = service
        .actions
        .iter()
        .find(|a| a.key().id.as_str() == id)
        .cloned()
        .ok_or_else(unavailable)?;
    let key = IdempotencyKeyText::parse_canonical(format!(
        "mind.purchase.{}.{}.{}",
        if choice == Choice::Grant { "g" } else { "r" },
        operation.to_hex(),
        action.digest().to_hex(),
    ))
    .map_err(|_| unavailable())?;
    let observation = ctx.submit_authoritative_observed(
        action.clone(),
        PerceivedRef::SelfActor,
        vec![],
        serde_json::json!({"operation":operation}),
        key,
    )?;
    entry.attempt = Some((action, observation));
    entry.schedule_after = ctx.w.now().saturating_add(RETRY_SECONDS);
    Ok(())
}

#[cfg(test)]
mod tests;
