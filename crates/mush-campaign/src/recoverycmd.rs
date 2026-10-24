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
//! The **tick** grants. `sweep` is a fourth arm on cut 9's ONE
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
    RecoveryRefusal, accessible_operational_assets, cooldown_until, eligible, loss_is_covered,
    next_loaner, policy, pool_balance, pool_can_fund, provider, upkeep_template,
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
    let voided = mush_sim::relay::cover_voided(ctx.w, me);
    let cooldown = if voided {
        grant
            .as_ref()
            .and_then(RecoveryGrant::ended_at)
            .map(|at| at.saturating_add(cfg.cooldown_secs))
    } else {
        cooldown_until(&cfg, grant.as_ref())
    };

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
        // E7r2 P0b: the marker is rendered to the applicant it belongs to, because it
        // is the whole reason their cooldown line reads the way it does. A grant
        // without it says nothing extra — the cooldown row below already is the
        // sentence.
        if grant.loss_covered() && !voided {
            rows.push(
                "That loss was recorded as an approved-operation loss, so no cooldown \
                 applies to it."
                    .to_owned(),
            );
        }
    }
    if voided {
        rows.push("Tycho cover voided: permanent relay-destruction sanction; no insured-loss cooldown waiver.".to_owned());
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
                // Z1 (independent verify fold): `ObjId`'s own `Display` already writes
                // the `#`, so `#{applicant}` rendered `##3` at a player.
                format!(
                    "Granted loaner #{} to {applicant} under contract #{contract}. They accept \
                     it with `@contract/accept {contract}`.",
                    loaner.0
                ),
            );
            ctx.gate_on_durable_ack();
            Ok(())
        }
        GrantOutcome::Ineligible(refusal) => Err(CmdError::msg(refusal.public())),
        GrantOutcome::ProviderExhausted(why) => Err(CmdError::msg(format!(
            "The provider cannot grant right now ({why}); the application stays queued and \
             staff have been paged."
        ))),
        // F5(a): the staff arm names the applicant's own blocker and the command that
        // clears it, rather than the provider's.
        GrantOutcome::ApplicantBlocked(why) => {
            let instrument = pool_instrument(ctx, &cfg);
            Err(CmdError::msg(format!(
                "{applicant} cannot be granted right now ({why}); they keep their place in the \
                 queue. `@account/open {applicant}={instrument}` clears the usual cause."
            )))
        }
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
///
/// **`Held` split in two** (independent verify fold, C10/B3; owner decision F5(a)).
/// It used to carry all eight reasons a grant did not happen, three of which are
/// facts about **one applicant** and not about the provider — and `sweep` treated
/// every one of them as provider-wide: it paged `recovery:provider` with a
/// recommendation (*"Add a hull the provider owns … or fund its pool account"*) that
/// fixes none of the three, and then `break`, so **one applicant the provider could
/// not pay stopped every other grant in the queue**. The rotation that should have
/// contained the damage was broken independently (see `sweep`), so a blocker
/// holding the highest object id was re-visited first on every fire, for ever.
/// Applying costs nothing and is not eligibility-gated, so it was reachable on
/// purpose.
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
    /// **The provider** cannot grant at all: its pool is at its floor, its pool
    /// account does not resolve, it holds no free hull, or the stipend would not
    /// fund. Nothing else in the queue can be granted either, so the fire pages
    /// `recovery:provider` and stops. The applicant **stays queued** — an empty pool
    /// is not a decision (Q7A).
    ProviderExhausted(&'static str),
    /// **This applicant** cannot be granted, for a state they can fix: no account in
    /// the pool's instrument, a lease that would not offer, a record that would not
    /// construct. The fire pages `recovery:applicant:{id}` with the command that
    /// clears it and **continues the rotation**, so nobody behind them is starved.
    /// They keep their place in claim order, because "you have no account in gold" is
    /// a fixable state and not an ineligibility.
    ApplicantBlocked(&'static str),
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
    // ---- D3 / F6(a): the MONEY fact is the authority on "already granted" ----
    //
    // `offer_lease` and the stipend `post` are durable through the economy's
    // `install_commit`, which pins its own writes; the three campaign documents are
    // ordinary **deferred** writes. Lose that batch — a tear, a restore from a
    // checkpoint predating it, or a `recovery_grant:` document the wire refuses at
    // `store::load`, which HOLDS it and so leaves it absent — and the next fire used
    // to lease the SAME hull again and post a SECOND stipend, because the campaign
    // mirror was the only marker and `grant_key(applicant, now)` was scoped to the IC
    // second. R1 got this right one crate over by making the live hold found *by
    // purpose* the marker; this is the same move for a lease, and it runs BEFORE the
    // campaign document is read for anything.
    if let Some((contract, loaner)) = crate::recovery::live_recovery_lease(ctx.w, cfg, applicant) {
        // The mirror already names this very lease: the ordinary "you already hold a
        // loaner", answered exactly as it always was. Only a mirror that is **absent
        // or disagrees** — the torn batch, the restore, the held document — reaches
        // the reconcile below, and it never leases a second hull.
        if grant
            .as_ref()
            .is_some_and(|g| g.is_live() && g.contract() == contract.0)
        {
            return GrantOutcome::Ineligible(RecoveryRefusal::LiveLease);
        }
        // A lease already stands for this applicant. Reconcile the mirror onto it
        // rather than leasing a second hull: the stipend re-posts under the SAME
        // contract-scoped key and answers `AlreadyPosted` with no second effect.
        let tier = ctx
            .w
            .module_state::<CampaignStore>()
            .and_then(|store| store.recovery_application(applicant))
            .map_or_else(
                || cfg.default_tier().cloned().unwrap_or_default(),
                |app| SmolStr::new(app.tier()),
            );
        let granted_at = grant.as_ref().map_or(now, RecoveryGrant::granted_at);
        // E7r2 P0b: on the RESUME path the marker is re-read from the record exactly
        // as `granted_at` is, and re-derived only when there is no record to read. The
        // predicate is over live world state, so a resume after the wreck was salvaged
        // would otherwise answer differently from the grant it is resuming — and the
        // waiver would silently disappear from a lease already in force.
        let loss_covered = grant.as_ref().map_or_else(
            || loss_is_covered(ctx.w, applicant),
            RecoveryGrant::loss_covered,
        );
        let Ok(record) = RecoveryGrant::with_cover(
            applicant,
            contract.0,
            loaner,
            tier,
            granted_at,
            None,
            loss_covered,
        ) else {
            return GrantOutcome::ApplicantBlocked("the grant record would not construct");
        };
        if let Some(account) = applicant_account(ctx, cfg, &PrincipalId::Object(applicant))
            && let Err(why) = post_stipend(ctx, cfg, applicant, contract, account)
        {
            return GrantOutcome::ProviderExhausted(why);
        }
        record_grant(ctx, record);
        return GrantOutcome::Granted {
            contract: contract.0,
            loaner,
        };
    }
    // **Re-evaluated at GRANT time, not at queue time** (Q7A).
    if let Err(refusal) = eligible(ctx.w, cfg, applicant, grant.as_ref(), now) {
        return GrantOutcome::Ineligible(refusal);
    }
    let Some(lessor) = provider(ctx.w, cfg) else {
        return GrantOutcome::ProviderExhausted("its pool account does not resolve");
    };
    // **Funded, never infinite**: both resources are checked before anything is
    // written, and either one being out holds the queue rather than deciding it.
    if !pool_can_fund(ctx.w, cfg) {
        return GrantOutcome::ProviderExhausted("its budget is at its floor");
    }
    let Some(loaner) = next_loaner(ctx.w, cfg) else {
        return GrantOutcome::ProviderExhausted("it holds no free hull");
    };
    let tenant = PrincipalId::Object(applicant);
    let Some(account) = applicant_account(ctx, cfg, &tenant) else {
        // F5(a): a fact about THIS applicant, and one they can fix in one command.
        return GrantOutcome::ApplicantBlocked(
            "the applicant holds no account in the pool's instrument",
        );
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
        Err(_) => return GrantOutcome::ApplicantBlocked("the lease could not be offered"),
    };
    // ---- E7r2: P0b — the insured-loss marker, written HERE and nowhere else ----
    //
    // `loss_is_covered` is a deterministic predicate over recorded facts — the
    // applicant holds title to a hull whose journey reached a terminal loss state
    // (`Failed`/`Captured`) and which is a member of a formation this world records.
    // It is evaluated **once**, at the moment the grant is made, and frozen onto the
    // record: the cooldown is decided by what was true when the lease was granted, not
    // by what happens to the wreck afterwards. Nothing else in this crate ever sets it.
    let loss_covered = loss_is_covered(ctx.w, applicant);
    let Ok(record) =
        RecoveryGrant::with_cover(applicant, contract.0, loaner, tier, now, None, loss_covered)
    else {
        return GrantOutcome::ApplicantBlocked("the grant record would not construct");
    };
    if let Err(why) = post_stipend(ctx, cfg, applicant, contract, account) {
        return GrantOutcome::ProviderExhausted(why);
    }
    // Past the post nothing is fallible: the three writes are deferred persists.
    record_grant(ctx, record);
    GrantOutcome::Granted {
        contract: contract.0,
        loaner,
    }
}

/// **The outfitting stipend**: one period's upkeep, out of the pool. Two lines,
/// exactly zero-sum, so the ledger total does not move — and **nothing mints**: the
/// money is the provider's own, and when it runs out the provider stops.
///
/// Keyed by the **contract** (D3; F6(a)), so a re-run after a lost campaign batch
/// answers `AlreadyPosted` and writes nothing a second time. Under the old IC-second
/// key a re-run one second later was a genuinely new key and a genuinely second
/// stipend.
fn post_stipend(
    ctx: &mut Ctx<'_>,
    cfg: &RecoveryConfig,
    applicant: ObjId,
    contract: ContractId,
    account: mush_economy::AccountId,
) -> Result<(), &'static str> {
    if cfg.upkeep_per_period <= 0 {
        return Ok(());
    }
    let Some(lessor) = provider(ctx.w, cfg) else {
        return Err("its pool account does not resolve");
    };
    let amount = Amount::from_minor(cfg.upkeep_per_period);
    let Some(debit) = amount.checked_neg() else {
        return Err("the stipend does not fit an Amount");
    };
    let plan = EconomyPlan {
        description: format!("recovery outfitting stipend for #{}", applicant.0),
        ..EconomyPlan::new(
            grant_key(applicant, contract),
            lessor,
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
        Ok(PostOutcome::Posted(_) | PostOutcome::AlreadyPosted(_)) => Ok(()),
        // The engine's own never-negative gate, or an approval rule: the provider is
        // out, and out is a wait, not a decision.
        _ => Err("its budget would not fund the stipend"),
    }
}

/// The three campaign documents a grant writes. Shared by the fresh grant and by
/// D3's reconcile path, so a rebuilt mirror is byte-identical to a first one.
fn record_grant(ctx: &mut Ctx<'_>, record: RecoveryGrant) {
    let applicant = record.applicant();
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
}

/// The deterministic key one grant's stipend posts under —
/// `recovery.grant/{applicant}/{contract}`.
///
/// **Contract-scoped, not IC-second-scoped** (independent verify fold, D3; owner
/// decision F6(a)). The cycle used to be the IC second the grant was made, so a
/// re-run after a lost campaign batch — the very case the key exists for — landed
/// under a *different* key and posted a *second* stipend. The contract is the money
/// fact the lease itself is, so a reconcile of the same lease is `AlreadyPosted`:
/// the same transaction id, no second effect.
#[must_use]
pub fn grant_key(applicant: ObjId, contract: ContractId) -> IdempotencyKey {
    IdempotencyKey::new(
        SmolStr::new_static("recovery.grant"),
        SmolStr::new(format!("{}/{}", applicant.0, contract.0)),
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
    // ---- D7 (independent verify fold): age an unaccepted loaner ----
    //
    // It runs before the queue, and on every fire including the fires where the queue
    // is empty, because the applicant it frees is precisely one who has already LEFT
    // the queue.
    let expired = expire_unaccepted(ctx, &cfg, budget);
    let (waiting, cursor) = {
        let Some(store) = ctx.w.module_state::<CampaignStore>() else {
            return expired;
        };
        (store.recovery_queue().waiting(), store.recovery_cursor)
    };
    if waiting.is_empty() {
        return expired;
    }
    // ---- C10/B3 (independent verify fold; owner decision F5(a)): the rotation ----
    //
    // `waiting.iter().position(|id| id.0 >= cursor).unwrap_or(0)` ran over a
    // **claim-ordered** queue, so when nobody satisfied the predicate — which is
    // exactly what happens once the cursor has passed the largest object id present —
    // `start` collapsed to 0 and the head of the queue was re-visited first on every
    // fire, for ever. Combined with the provider-wide `break` below, one applicant the
    // provider could not pay starved everybody behind them permanently.
    //
    // The cursor is still the raw applicant id (it is what `campaign_meta` persists),
    // but the fallback is now the **position** the cursor names rather than the head:
    // a cursor past every id wraps to 0 only because the rotation has genuinely
    // completed a lap, and with `ApplicantBlocked` continuing, a lap now reaches
    // everybody.
    let start = waiting
        .iter()
        .position(|id| id.0 >= cursor)
        .unwrap_or(0)
        .min(waiting.len().saturating_sub(1));
    let order: Vec<ObjId> = waiting[start..]
        .iter()
        .chain(waiting[..start].iter())
        .copied()
        .collect();

    let now = ctx.w.world_ic_now().at;
    let mut visited = expired;
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
            GrantOutcome::ProviderExhausted(why) => {
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
            GrantOutcome::ApplicantBlocked(why) => {
                // F5(a): a fact about ONE applicant. It pages **their** subject with
                // the command that clears it, and the rotation **continues** — the
                // whole point of the split. They keep their claim-order place, because
                // "you hold no account in the pool's instrument" is a state they can
                // fix, not an ineligibility.
                let instrument = pool_instrument(ctx, &cfg);
                crate::settle::page(
                    ctx,
                    SmolStr::new(format!("recovery:applicant:{}", applicant.0)),
                    &format!("recovery applicant #{}: {why}", applicant.0),
                    &format!(
                        "the queue moved past them and everybody behind them was granted \
                         normally. They keep their place. `@account/open #{}={instrument}` opens \
                         the account a stipend can be paid into, and `@recovery/grant #{}` then \
                         grants without waiting for the next fire",
                        applicant.0, applicant.0
                    ),
                );
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

/// The pool's instrument, for a page that names the command that clears it (F5(a)).
fn pool_instrument(ctx: &Ctx<'_>, cfg: &RecoveryConfig) -> String {
    ctx.w
        .module_state::<EconomyStore>()
        .and_then(|store| store.snapshot().account(cfg.pool_account).cloned())
        .map_or_else(
            || "<instrument>".to_owned(),
            |account| account.instrument.to_string(),
        )
}

/// **D7 — take back a loaner that was offered and never accepted.**
///
/// `try_grant` completes at `offer_lease`, which produces a contract the applicant
/// must still `@contract/accept`; until they do, no `Encumbrance::Lease` exists and
/// the hull is held out of the motor pool by the grant document alone. Nothing aged
/// that: the tick's fourth arm walked only the **queue**, and a granted applicant has
/// already left it, so an applicant who applied, was granted and never accepted held
/// a hull out of the pool **for ever** and was `LiveLease`-refused themselves — and
/// with `MAX_RECOVERY_APPLICATIONS_PER_PRINCIPAL` 1 a handful of them dried a small
/// provider, whose page then recommended adding a hull it already owned.
///
/// `SEAT_OFFER_TTL`'s shape: past [`crate::recovery::RECOVERY_ACCEPT_TTL_SECS`] the
/// lease is `end_lease`d and the grant closed, which frees the hull and lets the
/// applicant apply again. An **accepted** lease is never touched — the acceptance is
/// the applicant's consent and the hull is genuinely theirs until they return it.
fn expire_unaccepted(ctx: &mut Ctx<'_>, cfg: &RecoveryConfig, budget: usize) -> usize {
    let now = ctx.w.world_ic_now().at;
    let Some(lessor) = provider(ctx.w, cfg) else {
        return 0;
    };
    let doomed: Vec<RecoveryGrant> = {
        let Some(store) = ctx.w.module_state::<CampaignStore>() else {
            return 0;
        };
        store
            .recovery_grants()
            .filter(|grant| {
                grant.is_live()
                    && now.saturating_sub(grant.granted_at())
                        >= crate::recovery::RECOVERY_ACCEPT_TTL_SECS
            })
            .take(budget)
            .cloned()
            .collect()
    };
    let mut worked = 0usize;
    for grant in doomed {
        // Still `Offered` means the tenant never accepted — `offer_lease` records the
        // lessor's own acceptance at once, so the acceptance LIST is not the test.
        // Anything past `Offered` is a lease somebody is flying and is left alone.
        let unaccepted = ctx
            .w
            .module_state::<EconomyStore>()
            .and_then(|store| {
                store
                    .snapshot()
                    .contract(ContractId(grant.contract()))
                    .cloned()
            })
            .is_some_and(|contract| {
                matches!(
                    contract.state,
                    mush_economy::ContractState::Offered | mush_economy::ContractState::Draft
                )
            });
        if !unaccepted {
            continue;
        }
        let key = LeaseKey {
            lessor: lessor.clone(),
            premises: grant.loaner(),
        };
        match mush_economy::end_lease(ctx, ContractId(grant.contract()), &key) {
            Ok(LeaseEnd::Ended | LeaseEnd::AlreadyEnded) => {}
            // A retry or a refusal leaves the lease exactly as it was; the next fire
            // sees the same grant and tries again. Nothing is written here on either.
            _ => continue,
        }
        // **Forgotten, not ended.** An offer nobody took is not a loan that finished,
        // so it must not leave an `ended_at` behind — `cooldown_until` would then
        // charge the applicant a fourteen-day cooldown for a hull they never had, and
        // the next fire would refuse them `Cooldown` and drop them out of the queue
        // as ineligible. The record is deleted with the lease it described.
        crate::store::del_recovery_grant(ctx, grant.applicant());
        // Their application reopens and they rejoin the queue at the back, so an
        // expiry costs them their turn and not their access.
        let queue = ctx
            .w
            .module_state::<CampaignStore>()
            .and_then(|store| store.recovery_queue().joined(grant.applicant()).ok());
        if let Some(queue) = queue {
            put_recovery_queue(ctx, queue);
        }
        if let Some(app) = ctx
            .w
            .module_state::<CampaignStore>()
            .and_then(|store| store.recovery_application(grant.applicant()))
            .cloned()
        {
            put_recovery_application(ctx, app.with_state(ApplicationState::Queued));
        }
        worked = worked.saturating_add(1);
    }
    worked
}

/// The arm's budget, re-exported so the tick can cite it rather than restate it.
pub(crate) const RECOVERY_BUDGET: usize = MAX_RECOVERY_PER_FIRE;

#[cfg(test)]
mod tests {
    use super::*;

    /// The key is **contract-scoped** since the fold (D3; owner decision F6(a)): the
    /// IC second it used to carry made a re-run after a lost campaign batch a
    /// genuinely new key, which is the whole way a second stipend was posted.
    #[test]
    fn e7r1_r3_the_grant_key_is_deterministic_and_inside_the_kernel_cap() {
        let key = grant_key(ObjId(7), ContractId(1_000));
        assert_eq!(
            (key.namespace.as_str(), key.key.as_str()),
            ("recovery.grant", "7/1000")
        );
        assert_eq!(key, grant_key(ObjId(7), ContractId(1_000)));
        assert_ne!(key, grant_key(ObjId(7), ContractId(1_001)));
        assert_ne!(key, grant_key(ObjId(8), ContractId(1_000)));
        // Contract #0 is a legal first lease, and it is a legal key.
        assert_ne!(
            grant_key(ObjId(7), ContractId(0)),
            grant_key(ObjId(7), ContractId(1))
        );
        let widest = grant_key(ObjId(u32::MAX), ContractId(u64::MAX));
        let encoded = widest.namespace.len() + 1 + widest.key.len();
        assert!(
            encoded <= mush_core::MAX_IDEMPOTENCY_KEY_BYTES,
            "{encoded} bytes is past the kernel's cap"
        );
    }

    /// **Every `GrantOutcome` arm is either provider-wide or applicant-scoped, and
    /// the two are distinct types** (C10/B3; owner decision F5(a)). `Held` carried
    /// both and [`sweep`] read every one of them as provider-wide.
    #[test]
    fn vf_c10d_the_grant_outcome_names_whose_problem_it_is() {
        let provider_side = GrantOutcome::ProviderExhausted("its budget is at its floor");
        let applicant_side = GrantOutcome::ApplicantBlocked(
            "the applicant holds no account in the pool's instrument",
        );
        assert_ne!(provider_side, applicant_side);
        assert!(matches!(provider_side, GrantOutcome::ProviderExhausted(_)));
        assert!(matches!(applicant_side, GrantOutcome::ApplicantBlocked(_)));
    }

    #[test]
    fn e7r1_r3_the_arm_budget_is_the_constant_and_not_a_second_number() {
        assert_eq!(RECOVERY_BUDGET, MAX_RECOVERY_PER_FIRE);
    }
}
