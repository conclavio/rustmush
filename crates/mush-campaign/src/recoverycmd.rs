//! **E7r1: R3 — `@recovery`, and the fourth arm on the campaign tick.**
//!
//! The verb is one primary verb with four switches, entered in the pinned rule-1
//! allowlist in the same commit (08i §4). Its class is `LegacyPostImage`: the three
//! documents are exempt workflow records and the money rides
//! `mush_economy::post`'s own carve-out lane **from the command**, never a second
//! `Ctx::submit_authoritative`.
//!
//! # Staff are never the gate (Q7A(f))
//!
//! The **tick** grants. [`sweep`] is a fourth arm on cut 9's ONE
//! `campaign.operations` Coarse tick, with its own [`crate::recovery::MAX_RECOVERY_PER_FIRE`]
//! budget and its own rotation cursor, so it can never starve the settlement lane.
//! It stages **no authoritative submission**, so the tick-cadence rule cannot bind
//! this tick however the fires line up. `@recovery/grant` exists for a staffer who
//! wants it *now*; it runs the identical path, so there is one grant seam and not two.
//!
//! # Disclosure
//!
//! `@recovery/status` renders **the caller's own** state and one aggregate — how many
//! are waiting — and never another applicant's identity or queue position. The
//! aggregate names nobody, which is what makes it safe to show at all.

use std::sync::Arc;

use mush_core::module::{ModError, Registrar};
use mush_core::{ArgShape, CmdError, CommandCompatibility, CommandSpec, Ctx, ObjId};
use mush_economy::{
    Amount, ContractId, EconomicCause, EconomyPlan, EconomyStore, IdempotencyKey, LeaseEnd,
    LeaseKey, LeaseSpec, PostOutcome, RequestedLine,
};
use mush_org::PrincipalId;
use smol_str::SmolStr;

use crate::recovery::{
    ApplicationState, MAX_RECOVERY_PER_FIRE, RecoveryApplication, RecoveryConfig, RecoveryGrant,
    RecoveryRefusal, accessible_operational_assets, cooldown_until, eligible, next_loaner, policy,
    pool_balance, pool_can_fund, provider, upkeep_template,
};
use crate::render::is_staff;
use crate::store::{
    CampaignStore, put_recovery_application, put_recovery_grant, put_recovery_queue,
};

/// What a disabled provider answers. The verb still registers (E5c deviation 1): a
/// disabled layer refuses out loud rather than hiding a verb, which is also what
/// keeps the rule-1 allowlist tripwire able to see `@recovery` in a default
/// composition.
pub const NOT_ENABLED: &str = "Recovery is not enabled on this server.";

/// The one existence-hiding line for an applicant nobody may ask about.
pub const NO_SUCH_APPLICANT: &str = "No such applicant.";

/// Register the verb, in its own classified group (K8.5).
///
/// # Errors
///
/// [`ModError`] if the name is already taken.
pub fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.classify_commands(CommandCompatibility::LegacyPostImage);
    reg.command(
        CommandSpec {
            name: "@recovery".into(),
            switches: vec![
                "status".into(),
                "apply".into(),
                "grant".into(),
                "return".into(),
            ],
            shape: ArgShape::EqPair,
            help: "@recovery/status                    your own application, your own queue \
                   position, and how many are waiting. It never names anybody else.\n\
                   @recovery/apply [<tier>]            join the queue. Eligibility is a \
                   deterministic test over recorded facts and is re-run when the grant is \
                   made, not when you apply.\n\
                   @recovery/grant <#applicant>        (staff) grant now, through exactly the \
                   path the campaign tick uses. An empty pool is not a decision: it queues and \
                   pages.\n\
                   @recovery/return [<#holder>]        end your loaner's lease and start the \
                   cooldown."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_recovery),
    )?;
    // Restored last, so any later registration in this crate keeps the module
    // default (K8.5).
    reg.classify_commands(CommandCompatibility::LegacyPostImage);
    Ok(())
}

fn info(ctx: &mut Ctx<'_>, msg: impl Into<String>) {
    let actor = ctx.actor;
    ctx.out.sys_to_actor(actor, "campaign", msg.into());
}

fn cmd_recovery(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.has("apply") {
        recovery_apply(ctx)
    } else if ctx.switches.has("grant") {
        recovery_grant(ctx)
    } else if ctx.switches.has("return") {
        recovery_return(ctx)
    } else {
        recovery_status(ctx)
    }
}

// ---- the reads -------------------------------------------------------------

fn recovery_status(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let cfg = policy(ctx);
    if !cfg.enabled {
        return Err(CmdError::msg(NOT_ENABLED));
    }
    let me = ctx.actor;
    let now = ctx.w.world_ic_now().at;
    let store = ctx
        .w
        .module_state::<CampaignStore>()
        .ok_or_else(|| CmdError::msg(NOT_ENABLED))?;
    // The ONE aggregate: a count that names nobody. A position is rendered only to
    // the applicant who holds it.
    let waiting = store.recovery_queue().len();
    let mine = store.recovery_queue().position_of(me);
    let application = store.recovery_application(me).cloned();
    let grant = store.recovery_grant(me).cloned();
    let assets = accessible_operational_assets(ctx.w, me);
    let verdict = eligible(ctx.w, &cfg, me, grant.as_ref(), now);
    let cooldown = cooldown_until(&cfg, grant.as_ref());

    let mut rows = vec![format!(
        "Recovery: {waiting} waiting, {} loaners out, tiers [{}].",
        store.recovery_live_grants(),
        cfg.tiers
            .iter()
            .map(SmolStr::as_str)
            .collect::<Vec<_>>()
            .join(", ")
    )];
    rows.push(match (&application, mine) {
        (Some(app), Some(position)) => format!(
            "You applied for `{}` and are #{position} in the queue.",
            app.tier()
        ),
        (Some(app), None) => format!(
            "Your application for `{}` is {}.",
            app.tier(),
            match app.state() {
                ApplicationState::Queued => "recorded but not queued",
                ApplicationState::Granted => "granted",
                ApplicationState::Closed => "closed",
            }
        ),
        (None, _) => "You have no application.".to_owned(),
    });
    if let Some(grant) = &grant {
        rows.push(if grant.is_live() {
            format!(
                "You hold loaner #{} on tier `{}`, leased under contract #{}.",
                grant.loaner().0,
                grant.tier(),
                grant.contract()
            )
        } else {
            format!("Your last loaner (#{}) was returned.", grant.loaner().0)
        });
    }
    rows.push(match verdict {
        Ok(()) => format!("You are eligible ({assets} accessible operational assets)."),
        Err(refusal) => refusal.public().to_owned(),
    });
    if let Some(until) = cooldown {
        rows.push(format!("Your cooldown runs to IC second {until}."));
    }
    // Staff, and only staff, are told whether the provider can actually pay — it is
    // an operational figure, not a player-facing one.
    if is_staff(ctx.w, me) {
        rows.push(format!(
            "[staff] pool #{} balance {:?}, floor {}, {}.",
            cfg.pool_account.0,
            pool_balance(ctx.w, &cfg),
            cfg.pool_floor,
            if pool_can_fund(ctx.w, &cfg) {
                "fundable"
            } else {
                "DRY — the queue holds"
            }
        ));
    }
    for row in rows {
        info(ctx, row);
    }
    Ok(())
}

// ---- the writes ------------------------------------------------------------

fn recovery_apply(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let cfg = policy(ctx);
    if !cfg.enabled {
        return Err(CmdError::msg(NOT_ENABLED));
    }
    let me = ctx.actor;
    let now = ctx.w.world_ic_now().at;
    let raw = ctx.args.raw().trim().to_owned();
    let tier = if raw.is_empty() {
        cfg.default_tier().cloned().ok_or_else(|| {
            CmdError::msg("This server declares no recovery tiers, so nothing can be granted.")
        })?
    } else {
        if !cfg.declares(&raw) {
            return Err(CmdError::msg(format!(
                "No such tier. This server offers: {}.",
                cfg.tiers
                    .iter()
                    .map(SmolStr::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
        SmolStr::new(&raw)
    };
    let store = ctx
        .w
        .module_state::<CampaignStore>()
        .ok_or_else(|| CmdError::msg(NOT_ENABLED))?;
    if store.recovery_grant(me).is_some_and(RecoveryGrant::is_live) {
        return Err(CmdError::msg(RecoveryRefusal::LiveLease.public()));
    }
    // The per-principal application cap, narrowed by the block and never widened past
    // `MAX_RECOVERY_APPLICATIONS_PER_PRINCIPAL`.
    let live = usize::from(
        store
            .recovery_application(me)
            .is_some_and(|app| app.state() == ApplicationState::Queued),
    );
    let cap = (cfg.applications_per_principal as usize)
        .min(crate::recovery::MAX_RECOVERY_APPLICATIONS_PER_PRINCIPAL);
    if live >= cap {
        return Err(CmdError::msg(
            "You already hold a recovery application; `@recovery/status` shows it.",
        ));
    }
    let queue = store.recovery_queue().clone();
    let joined = queue
        .joined(me)
        .map_err(|_| CmdError::msg("The recovery queue is full; try again shortly."))?;
    let position = joined.position_of(me).unwrap_or(0);
    let app = RecoveryApplication::new(me, now, tier.clone(), ApplicationState::Queued)
        .map_err(|error| CmdError::msg(error.to_string()))?;
    put_recovery_queue(ctx, joined);
    put_recovery_application(ctx, app);
    info(
        ctx,
        format!(
            "Applied for a `{tier}` loaner; you are #{position} in the queue. Eligibility is \
             re-checked when the grant is made."
        ),
    );
    ctx.gate_on_durable_ack();
    Ok(())
}

fn recovery_grant(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let cfg = policy(ctx);
    if !cfg.enabled {
        return Err(CmdError::msg(NOT_ENABLED));
    }
    if !is_staff(ctx.w, ctx.actor) {
        return Err(CmdError::msg(NO_SUCH_APPLICANT));
    }
    let raw = ctx.args.lhs().trim().trim_start_matches('#').to_owned();
    let applicant = raw
        .parse::<u32>()
        .ok()
        .map(ObjId)
        .filter(|id| id.0 != 0)
        .ok_or_else(|| CmdError::msg(NO_SUCH_APPLICANT))?;
    if ctx
        .w
        .module_state::<CampaignStore>()
        .and_then(|store| store.recovery_application(applicant))
        .is_none()
    {
        return Err(CmdError::msg(NO_SUCH_APPLICANT));
    }
    let now = ctx.w.world_ic_now().at;
    match try_grant(ctx, &cfg, applicant, now) {
        GrantOutcome::Granted { contract, loaner } => {
            info(
                ctx,
                format!(
                    "Granted loaner #{} to #{applicant} under contract #{contract}. They accept it with `@contract/accept {contract}`.",
                    loaner.0
                ),
            );
            ctx.gate_on_durable_ack();
            Ok(())
        }
        GrantOutcome::Ineligible(refusal) => Err(CmdError::msg(refusal.public())),
        GrantOutcome::Held(why) => Err(CmdError::msg(format!(
            "The provider cannot grant right now ({why}); the application stays queued and \
             staff have been paged."
        ))),
    }
}

fn recovery_return(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let cfg = policy(ctx);
    if !cfg.enabled {
        return Err(CmdError::msg(NOT_ENABLED));
    }
    let raw = ctx.args.lhs().trim().trim_start_matches('#').to_owned();
    let holder = if raw.is_empty() {
        ctx.actor
    } else {
        let named = raw
            .parse::<u32>()
            .ok()
            .map(ObjId)
            .ok_or_else(|| CmdError::msg(NO_SUCH_APPLICANT))?;
        // Only staff may return somebody else's loaner, and a non-holder gets the
        // byte-identical existence-hiding line.
        if named != ctx.actor && !is_staff(ctx.w, ctx.actor) {
            return Err(CmdError::msg(NO_SUCH_APPLICANT));
        }
        named
    };
    let now = ctx.w.world_ic_now().at;
    let Some(grant) = ctx
        .w
        .module_state::<CampaignStore>()
        .and_then(|store| store.recovery_grant(holder))
        .filter(|grant| grant.is_live())
        .cloned()
    else {
        return Err(CmdError::msg("You hold no loaner."));
    };
    let Some(lessor) = provider(ctx.w, &cfg) else {
        return Err(CmdError::msg(NOT_ENABLED));
    };
    let key = LeaseKey {
        lessor,
        premises: grant.loaner(),
    };
    match mush_economy::end_lease(ctx, ContractId(grant.contract()), &key) {
        Ok(LeaseEnd::Ended | LeaseEnd::AlreadyEnded) => {}
        Ok(LeaseEnd::Retrying) => {
            return Err(CmdError::msg(
                "That lease could not be unwound yet; the economy will retry it and staff have \
                 been paged. Nothing about your loaner changed.",
            ));
        }
        Err(error) => {
            if is_staff(ctx.w, ctx.actor) {
                info(ctx, format!("[staff] the lease refused: {error}"));
            }
            return Err(CmdError::msg(
                "That loaner could not be returned. Nothing changed.",
            ));
        }
    }
    // Past the unwind nothing else is fallible: the two documents are deferred
    // persists.
    close_grant(ctx, &grant, now);
    info(
        ctx,
        format!(
            "Loaner #{} returned; the lease is ended and its upkeep with it.",
            grant.loaner().0
        ),
    );
    Ok(())
}

/// End one grant's records — the ONE place a lease's campaign side closes, so the
/// command, the sweep and a dissolution cannot disagree.
fn close_grant(ctx: &mut Ctx<'_>, grant: &RecoveryGrant, now: i64) {
    put_recovery_grant(ctx, grant.ended(now));
    if let Some(app) = ctx
        .w
        .module_state::<CampaignStore>()
        .and_then(|store| store.recovery_application(grant.applicant()))
        .cloned()
    {
        put_recovery_application(ctx, app.with_state(ApplicationState::Closed));
    }
}

// ---- the ONE grant path ----------------------------------------------------

/// What one attempt to grant did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GrantOutcome {
    /// A lease was offered and the stipend posted.
    Granted {
        /// The lease contract the applicant accepts.
        contract: u64,
        /// The hull.
        loaner: ObjId,
    },
    /// The applicant does not qualify. They leave the queue.
    Ineligible(RecoveryRefusal),
    /// The **provider** cannot grant: no free hull, or a pool at its floor. The
    /// applicant **stays queued** and staff are paged level 1 — an empty pool is not
    /// a decision (Q7A).
    Held(&'static str),
}

/// **The one grant seam.** `@recovery/grant` and the tick's fourth arm both call it,
/// so there is one definition of what a grant is.
///
/// The order is the whole never-negative argument, and it is `bond::place`'s:
/// everything fallible happens **before** the last one, and the post is last. The
/// pool is checked before a lease is offered, so a dry provider offers nothing; the
/// lease is offered before the stipend is posted, so a refused offer costs nothing;
/// and `mush_economy::post` is the final fallible call, after which only deferred
/// document writes remain.
pub(crate) fn try_grant(
    ctx: &mut Ctx<'_>,
    cfg: &RecoveryConfig,
    applicant: ObjId,
    now: i64,
) -> GrantOutcome {
    let grant = ctx
        .w
        .module_state::<CampaignStore>()
        .and_then(|store| store.recovery_grant(applicant))
        .cloned();
    // **Re-evaluated at GRANT time, not at queue time** (Q7A).
    if let Err(refusal) = eligible(ctx.w, cfg, applicant, grant.as_ref(), now) {
        return GrantOutcome::Ineligible(refusal);
    }
    let Some(lessor) = provider(ctx.w, cfg) else {
        return GrantOutcome::Held("its pool account does not resolve");
    };
    // **Funded, never infinite**: both resources are checked before anything is
    // written, and either one being out holds the queue rather than deciding it.
    if !pool_can_fund(ctx.w, cfg) {
        return GrantOutcome::Held("its budget is at its floor");
    }
    let Some(loaner) = next_loaner(ctx.w, cfg) else {
        return GrantOutcome::Held("it holds no free hull");
    };
    let tenant = PrincipalId::Object(applicant);
    let Some(account) = applicant_account(ctx, cfg, &tenant) else {
        return GrantOutcome::Held("the applicant holds no account in the pool's instrument");
    };
    let tier = ctx
        .w
        .module_state::<CampaignStore>()
        .and_then(|store| store.recovery_application(applicant))
        .map_or_else(
            || cfg.default_tier().cloned().unwrap_or_default(),
            |app| SmolStr::new(app.tier()),
        );
    // The record is built from inputs that are already resolved, **before** the post,
    // so the last fallible call really is the post and nothing after it can fail.
    let rent = upkeep_template(cfg, &lessor, &tenant, account, ctx.w.now());
    let contract = match mush_economy::offer_lease(
        ctx,
        LeaseSpec {
            name: SmolStr::new(format!("recovery loaner ({tier})")),
            lessor: lessor.clone(),
            scope: None,
            tenants: vec![tenant.clone()],
            premises: loaner,
            rent,
        },
    ) {
        Ok(contract) => contract,
        Err(_) => return GrantOutcome::Held("the lease could not be offered"),
    };
    let Ok(record) = RecoveryGrant::new(applicant, contract.0, loaner, tier, now, None) else {
        return GrantOutcome::Held("the grant record would not construct");
    };
    // The outfitting stipend: one period's upkeep, out of the pool. Two lines,
    // exactly zero-sum, so the ledger total does not move — and **nothing mints**:
    // the money is the provider's own, and when it runs out the provider stops.
    if cfg.upkeep_per_period > 0 {
        let amount = Amount::from_minor(cfg.upkeep_per_period);
        let Some(debit) = amount.checked_neg() else {
            return GrantOutcome::Held("the stipend does not fit an Amount");
        };
        let plan = EconomyPlan {
            description: format!("recovery outfitting stipend for #{}", applicant.0),
            ..EconomyPlan::new(
                grant_key(applicant, now),
                lessor.clone(),
                EconomicCause::Internal {
                    kind: SmolStr::new_static("campaign.recovery.grant"),
                    id: u64::from(applicant.0),
                },
                vec![
                    RequestedLine {
                        account: cfg.pool_account,
                        delta: debit,
                        memo: SmolStr::new_static("campaign.recovery.stipend"),
                    },
                    RequestedLine {
                        account,
                        delta: amount,
                        memo: SmolStr::new_static("campaign.recovery.stipend"),
                    },
                ],
            )
        };
        match mush_economy::post(ctx, &plan) {
            Ok(PostOutcome::Posted(_) | PostOutcome::AlreadyPosted(_)) => {}
            // The engine's own never-negative gate, or an approval rule: the
            // provider is out, and out is a wait, not a decision.
            _ => return GrantOutcome::Held("its budget would not fund the stipend"),
        }
    }
    // Past the post nothing is fallible: the three writes are deferred persists.
    put_recovery_grant(ctx, record);
    if let Some(app) = ctx
        .w
        .module_state::<CampaignStore>()
        .and_then(|store| store.recovery_application(applicant))
        .cloned()
    {
        put_recovery_application(ctx, app.with_state(ApplicationState::Granted));
    }
    let queue = ctx
        .w
        .module_state::<CampaignStore>()
        .map(|store| store.recovery_queue().left(applicant));
    if let Some(queue) = queue {
        put_recovery_queue(ctx, queue);
    }
    GrantOutcome::Granted {
        contract: contract.0,
        loaner,
    }
}

/// The deterministic key one grant's stipend posts under —
/// `recovery.grant/{applicant}/{cycle}`, where the cycle is the IC second the grant
/// was made. A retry inside the same second is `AlreadyPosted`, which is the same
/// transaction and no second effect.
#[must_use]
pub fn grant_key(applicant: ObjId, cycle: i64) -> IdempotencyKey {
    IdempotencyKey::new(
        SmolStr::new_static("recovery.grant"),
        SmolStr::new(format!("{}/{cycle}", applicant.0)),
    )
}

/// The applicant's own account in the pool's instrument.
fn applicant_account(
    ctx: &Ctx<'_>,
    cfg: &RecoveryConfig,
    who: &PrincipalId,
) -> Option<mush_economy::AccountId> {
    let store = ctx.w.module_state::<EconomyStore>()?;
    let instrument = store
        .snapshot()
        .account(cfg.pool_account)?
        .instrument
        .clone();
    store
        .snapshot()
        .accounts_of(who)
        .find(|account| {
            account.instrument == instrument
                && matches!(account.status, mush_economy::AccountStatus::Active)
        })
        .map(|account| account.id)
}

// ---- the FOURTH arm on cut 9's ONE campaign tick ---------------------------

/// **The recovery arm.** A pull over durable state, bounded on records **visited**
/// with its own budget, advancing a rotation cursor past the last visited applicant,
/// and a no-op past the applied point — so a reboot re-running it *is* the reconcile.
///
/// It stages **no authoritative submission**: every money movement rides
/// `mush_economy::post`'s carve-out lane, so the tick-cadence rule cannot bind this
/// tick however the fires line up.
pub(crate) fn sweep(ctx: &mut Ctx<'_>, budget: usize) -> usize {
    let cfg = policy(ctx);
    if !cfg.enabled {
        return 0;
    }
    let (waiting, cursor) = {
        let Some(store) = ctx.w.module_state::<CampaignStore>() else {
            return 0;
        };
        (store.recovery_queue().waiting(), store.recovery_cursor)
    };
    if waiting.is_empty() {
        return 0;
    }
    // The rotation: start at the first applicant at or past the cursor, wrap once, so
    // nobody at the back of a long queue is starved by the front.
    let start = waiting.iter().position(|id| id.0 >= cursor).unwrap_or(0);
    let order: Vec<ObjId> = waiting[start..]
        .iter()
        .chain(waiting[..start].iter())
        .copied()
        .collect();

    let now = ctx.w.world_ic_now().at;
    let mut visited = 0usize;
    let mut last = None;
    for applicant in order {
        if visited >= budget {
            break;
        }
        visited = visited.saturating_add(1);
        last = Some(applicant);
        match try_grant(ctx, &cfg, applicant, now) {
            GrantOutcome::Granted { .. } => {}
            GrantOutcome::Ineligible(_) => {
                // **A grant reaches no ineligible applicant, and the queue does not
                // advance past them silently**: they leave it and their application
                // is closed, so `@recovery/status` tells them why and they may apply
                // again when the facts change.
                let queue = ctx
                    .w
                    .module_state::<CampaignStore>()
                    .map(|store| store.recovery_queue().left(applicant));
                if let Some(queue) = queue {
                    put_recovery_queue(ctx, queue);
                }
                if let Some(app) = ctx
                    .w
                    .module_state::<CampaignStore>()
                    .and_then(|store| store.recovery_application(applicant))
                    .cloned()
                {
                    put_recovery_application(ctx, app.with_state(ApplicationState::Closed));
                }
            }
            GrantOutcome::Held(why) => {
                // **An exhausted pool queues and pages level 1** — it is never a
                // staff decision, and the applicant keeps their place. Paged once per
                // subject, so a dry provider opens one record and not one per fire.
                crate::settle::page(
                    ctx,
                    SmolStr::new_static("recovery:provider"),
                    &format!("the recovery provider cannot grant: {why}"),
                    "the queue is holding rather than granting. Add a hull the provider owns \
                     (`@claim/transfer` or `@econ/pay` a title to it), or fund its pool account, \
                     then `@recovery/grant <#applicant>`",
                );
                // Nothing else in the queue can be granted either, so stop here
                // rather than paying for a whole budget of the same answer.
                break;
            }
        }
    }
    if let Some(last) = last
        && let Some(store) = ctx.w.module_state_mut::<CampaignStore>()
    {
        store.recovery_cursor = last.0.saturating_add(1);
    }
    visited
}

/// The arm's budget, re-exported so the tick can cite it rather than restate it.
pub(crate) const RECOVERY_BUDGET: usize = MAX_RECOVERY_PER_FIRE;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn e7r1_r3_the_grant_key_is_deterministic_and_inside_the_kernel_cap() {
        let key = grant_key(ObjId(7), 1_000);
        assert_eq!(
            (key.namespace.as_str(), key.key.as_str()),
            ("recovery.grant", "7/1000")
        );
        assert_eq!(key, grant_key(ObjId(7), 1_000));
        assert_ne!(key, grant_key(ObjId(7), 1_001));
        assert_ne!(key, grant_key(ObjId(8), 1_000));
        let widest = grant_key(ObjId(u32::MAX), i64::MAX);
        let encoded = widest.namespace.len() + 1 + widest.key.len();
        assert!(
            encoded <= mush_core::MAX_IDEMPOTENCY_KEY_BYTES,
            "{encoded} bytes is past the kernel's cap"
        );
    }

    #[test]
    fn e7r1_r3_the_arm_budget_is_the_constant_and_not_a_second_number() {
        assert_eq!(RECOVERY_BUDGET, MAX_RECOVERY_PER_FIRE);
    }
}
