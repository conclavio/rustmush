//! **The `tacsim.procurement` tick** (doc 17h §3 M8f P3; the module note in [`super`] has the
//! four steps). GOD; one step per policy per fire, queued as the House's agent through
//! [`crate::port_tick::Fire::as_principal`]; infallible: every refusal pages once (a log line)
//! and the policy moves on.
use std::collections::{BTreeMap, BTreeSet};

use mush_core::read_service::OwnerReadHandle;
use mush_core::{
    CommitToken, Ctx, DefinitionRef, Flags, IdempotencyKeyText, ObjId, ObjKind, PerceivedRef,
    SceneId, StateFamilyHandle, World,
};
use mush_market::mind_commands::procurement::{
    AgentPurchaseView, ProcurementListing, ProcurementOperationView, ProcurementPhase,
};
use mush_market::mind_commands::{
    MarketCommandActions, MarketCommandReader, MarketCommandRequest, OwnOpenView, Protocol,
    Step as MarketStep,
};
use mush_org::OrgId;
use mush_roll::commerce::command_reader::RollCommandReadRequest;
use mush_roll::commerce::holding_reader::{OriginalOwnerReader, OriginalReadRequest};
use mush_roll::commerce::rating::{RatingRef, RatingScore, RatingTally};
use mush_roll::commerce::{CharterId, LicenceId};
use serde_json::json;

use super::ledger::{Period, ProcurementKey, ProcurementRow, RowPhase};
use super::rank::{self, Candidate, SCORE_CACHE_BOUND};
use super::{MAX_GRANT_TRIES, MAX_REFUND_TRIES};
use crate::launch::ProcurementPolicy;
use crate::port_tick::{Fire, Step};

/// Listings pages one fire reads at most; the scan resumes on the next fire.
pub const MAX_LISTING_PAGES_PER_FIRE: usize = 4;
/// Rating reads (each up to `TALLY_PAGES_PER_READ` result pages) one fire makes at most.
pub const MAX_TALLY_READS_PER_FIRE: usize = 8;
/// Holdings reads one choice makes at most; an organisation with more current licences than
/// they reach counts only those (the cap then errs toward buying: paged once).
pub const MAX_HOLDINGS_READS: usize = 16;
/// Ledger rows one fire reads for one organisation.
pub const MAX_LEDGER_ROWS: usize = 256;
/// Fires a quote may stay unseen after its Quote before the listing is skipped for the month.
pub const MAX_QUOTE_WAITS: u8 = 6;
/// Organisation assets one hull count walks at most.
const MAX_HULL_ASSETS: usize = 4_096;

/// **The tick's RAM slot** (P3/P7): pacing only. The ledger rows, the market and the Roll are
/// the memory; a restart empties this and the next fire re-derives everything.
#[derive(Clone)]
pub(crate) struct ProcurementState {
    rows: StateFamilyHandle<ProcurementKey, ProcurementRow>,
    action: DefinitionRef,
    runs: BTreeMap<u64, Run>,
    /// The ONE ledger row each agent's `tacsim_procurement` offer fixes (the yard's rows
    /// precedent; H5: one row, so the offer never grows): written by the tick just before it
    /// submits the record as the agent.
    records: BTreeMap<ObjId, BTreeMap<String, ProcurementRow>>,
    paged: BTreeSet<String>,
    /// `(IC second, ordinal)` of the submission keys.
    keys: (i64, u64),
    /// The deployment's policies, set once the launch composes
    /// (`crate::launch::registration::Registration::finalize`); empty, the tick visits nothing.
    policies: Vec<ProcurementPolicy>,
    /// Test only: `H` taken as given, for a rig with no programme port (its titled hulls
    /// are none of `world_hulls`'s).
    #[cfg(test)]
    assumed_hulls: Option<u32>,
    /// Test only: admit a 0‰ line, for a rig whose one genuine rating won no trial (H9 keeps
    /// such a line out in production).
    #[cfg(test)]
    admit_zero: bool,
}

/// One policy's month in RAM.
#[derive(Clone, Debug, Default)]
struct Run {
    period: Option<Period>,
    /// The month is done: nothing affordable left, or the treasury could not pay.
    closed: bool,
    /// Why it closed (for the staff log and the tests).
    closed_why: Option<&'static str>,
    quoting: Option<Quoting>,
    skipped: BTreeSet<CommitToken>,
    /// Terminal steps submitted per funded purchase: the Grants, then the Refunds (H3).
    grants: BTreeMap<CommitToken, u8>,
    scan: Scan,
    /// The month's scores per `(revision, rating id)` (P1; H4: kept for the month, reset at a
    /// period change, never stored on the Roll). A read failure is not kept.
    scores: BTreeMap<String, Option<RatingScore>>,
}

/// The agent's last Quote, awaited, and whether its Fund was sent (H2).
#[derive(Clone, Debug)]
struct Quoting {
    listing: CommitToken,
    waits: u8,
    fund_sent: bool,
}

/// The month's listings scan, resumable across fires.
#[derive(Clone, Debug, Default)]
struct Scan {
    after: Option<CommitToken>,
    done: bool,
    listings: Vec<ProcurementListing>,
    /// A rating count in progress: its cache key, rating, next page and the count so far.
    tally: Option<(String, RatingRef, u16, RatingTally)>,
}

impl ProcurementState {
    pub(super) fn new(
        rows: StateFamilyHandle<ProcurementKey, ProcurementRow>,
        action: DefinitionRef,
    ) -> Self {
        Self {
            rows,
            action,
            runs: BTreeMap::new(),
            records: BTreeMap::new(),
            paged: BTreeSet::new(),
            keys: (i64::MIN, 0),
            policies: Vec::new(),
            #[cfg(test)]
            assumed_hulls: None,
            #[cfg(test)]
            admit_zero: false,
        }
    }
    /// Test only: admit a 0‰ line (the composed rig's one rating won no trial).
    #[cfg(test)]
    pub(crate) fn admit_zero_scores(&mut self) {
        self.admit_zero = true;
    }
    fn zero_admitted(&self) -> bool {
        #[cfg(test)]
        if self.admit_zero {
            return true;
        }
        false
    }
    /// The launch's procurement policies (set at finalize; boot re-sets them).
    pub(crate) fn set_policies(&mut self, policies: Vec<ProcurementPolicy>) {
        self.policies = policies;
    }
    /// Whether `agent` is the procurement agent of `org` under a policy now.
    pub(crate) fn is_agent(&self, org: OrgId, agent: ObjId) -> bool {
        self.policies
            .iter()
            .any(|policy| policy.org == org && policy.agent == agent)
    }
    /// Test only: take `H` as `hulls` (the composed rig has no programme port).
    #[cfg(test)]
    pub(crate) fn assume_hulls(&mut self, hulls: u32) {
        self.assumed_hulls = Some(hulls);
    }
    fn hulls(&self, w: &World, org: OrgId) -> u32 {
        #[cfg(test)]
        if let Some(hulls) = self.assumed_hulls {
            return hulls;
        }
        titled_hulls(w, org)
    }
    /// Test only: what the month knows of `org`: its scores, its listings scanned, whether
    /// it is closed and the paged refusals so far.
    #[cfg(test)]
    pub(crate) fn describe(&self, org: OrgId) -> String {
        let run = self.runs.get(&org.0);
        format!(
            "scores={:?} listings={:?} closed={:?} skipped={:?} paged={:?}",
            run.map(|r| r.scores.values().copied().collect::<Vec<_>>()),
            run.map(|r| {
                r.scan
                    .listings
                    .iter()
                    .map(|l| (l.view().price_minor(), l.view().form()))
                    .collect::<Vec<_>>()
            }),
            run.map(|r| r.closed_why),
            run.map(|r| r.skipped.len()),
            self.paged
        )
    }
    /// Test only: fix `row` into `agent`'s record offer, as the tick does before it records.
    #[cfg(test)]
    pub(crate) fn offer_row(&mut self, agent: ObjId, row: ProcurementRow) {
        self.records
            .insert(agent, BTreeMap::from([(row.operation.to_string(), row)]));
    }
    /// The rows `agent`'s record offer fixes, by operation token text.
    pub(super) fn offered(&self, agent: ObjId) -> Option<&BTreeMap<String, ProcurementRow>> {
        self.records.get(&agent)
    }
    fn page(&mut self, org: OrgId, what: &str, detail: &str) {
        if self.paged.insert(format!("{}/{what}", org.0)) {
            tracing::warn!(
                log_id = "tacsim.procurement.refused",
                org = org.0,
                what,
                detail,
                "tacsim: a House's procurement step is refused; the policy moves on"
            );
        }
    }
    fn key(
        &mut self,
        at: i64,
        org: OrgId,
        verb: &str,
        subject: &str,
    ) -> Option<IdempotencyKeyText> {
        if self.keys.0 != at {
            self.keys = (at, 0);
        }
        let ordinal = self.keys.1;
        self.keys.1 = ordinal.checked_add(1)?;
        IdempotencyKeyText::parse_canonical(format!(
            "procurement.{}.{verb}.{subject}.{at:x}.{ordinal:x}",
            org.0
        ))
        .ok()
    }
}

/// The reads and the actions one visit needs.
struct Handles {
    market: OwnerReadHandle<MarketCommandReader>,
    roll: OwnerReadHandle<OriginalOwnerReader>,
    actions: MarketCommandActions,
}

/// **The tick** (registered by [`super::register`]).
pub(crate) fn procurement_tick(ctx: &mut Ctx<'_>, _tag: &str, _scene: Option<SceneId>) {
    let Some(defaults) = ctx
        .w
        .module_state::<crate::TacsimStore>()
        .filter(|store| store.enabled())
        .map(|store| store.cfg.procurement)
    else {
        return;
    };
    let Some(mut state) = ctx.w.module_state::<ProcurementState>().cloned() else {
        return;
    };
    let policies = state.policies.clone();
    if policies.is_empty() {
        return;
    }
    let handles = ctx
        .w
        .module_state::<crate::minds::market_commands::MindCommandService>()
        .and_then(|service| {
            Some(Handles {
                market: service.market.clone()?,
                roll: service.roll.clone(),
                actions: MarketCommandActions::capture(ctx.reg.definitions()?),
            })
        });
    let mut fire = Fire::new(BTreeMap::new());
    for policy in &policies {
        let Some(handles) = &handles else {
            state.page(
                policy.org,
                "unwired",
                "the player market's reads are not composed",
            );
            continue;
        };
        let step = visit(ctx, &mut fire, &mut state, handles, policy, defaults);
        fire.settle(policy.agent, step);
    }
    if let Some(slot) = ctx.state_mut::<ProcurementState>() {
        *slot = state;
    }
}

/// Whether `agent` is a live, non-GOING Player-kind character.
fn live(w: &World, agent: ObjId) -> bool {
    w.get(agent)
        .is_some_and(|o| o.kind == ObjKind::Player && !o.flags.contains(Flags::GOING))
}

/// One policy's one step (the module note's 1–4). Settling and recording run BEFORE the
/// agent's liveness and standing gates (the fix round's H3): standing gates only a new buy, so
/// an agent that lost its membership still settles the House's escrow.
fn visit(
    ctx: &mut Ctx<'_>,
    fire: &mut Fire,
    state: &mut ProcurementState,
    handles: &Handles,
    policy: &ProcurementPolicy,
    defaults: crate::config::ProcurementConfig,
) -> Step {
    let (org, agent) = (policy.org, policy.agent);
    let now_ic = ctx.w.world_ic_now().at;
    let period = Period::of_ic(now_ic);
    let run = state.runs.entry(org.0).or_default();
    if run.period != Some(period) {
        *run = Run {
            period: Some(period),
            ..Run::default()
        };
    }
    if ctx.authoritative_pending(agent) {
        // The agent's last step has not landed; read it next fire (D-18-32).
        return Step::Deferred;
    }
    let Some(rows) = ledger_rows(ctx, &state.rows, org, period) else {
        state.page(org, "ledger", "the procurement ledger is unreadable");
        return Step::Refused;
    };
    // 1. Settle a funded purchase (through the GOD view of the operation, which needs no
    //    grant: a revoked agent still settles, H3).
    for row in rows.iter().filter(|row| row.phase == RowPhase::Funded) {
        let Some(view) = operation(ctx, handles, row.operation) else {
            state.page(org, "operation", "a funded purchase is unreadable");
            continue;
        };
        match view.phase() {
            ProcurementPhase::Granted => {
                return record(ctx, fire, state, row.with_phase(RowPhase::Granted));
            }
            ProcurementPhase::Refunded => {
                return record(ctx, fire, state, row.with_phase(RowPhase::Refunded));
            }
            ProcurementPhase::Funded => {
                let protocol = view.protocol();
                if let Some(step) = settle(ctx, fire, state, handles, policy, (row, protocol)) {
                    return step;
                }
            }
        }
    }
    // 2. Record a funded purchase the ledger lacks, as soon as its Fund lands and before any
    //    Grant (so no completion escapes the ledger; a restart between Fund and record too).
    //    The House's funded purchases are its buyer index row (M8c S1: every member's), read
    //    as the agent; the tick records only its own agent's. An unreadable index refuses the
    //    visit (round 2): a funded purchase it hides would fall out of the spend and the holdings,
    //    and the next buy could overspend.
    let Some(open) = own_open(ctx, handles, agent, org) else {
        state.page(org, "open", "the House's open purchases are unreadable");
        return Step::Refused;
    };
    for purchase in open {
        if rows.iter().any(|row| row.operation == purchase.operation()) {
            continue;
        }
        let Some(view) = operation(ctx, handles, purchase.operation()) else {
            continue;
        };
        // Another member's purchase for the House is that member's to settle, and no
        // part of the procurement budget.
        if view.org() != Some(org)
            || view.acting() != agent
            || view.phase() != ProcurementPhase::Funded
        {
            continue;
        }
        // The month's next ordinal (the rows of `period` are all read: a full month closes
        // before it can buy again, H6).
        let ordinal = rows
            .iter()
            .filter(|row| row.period == period)
            .filter_map(|row| row.ordinal.checked_add(1))
            .max()
            .unwrap_or(0);
        let row = ProcurementRow {
            format: 1,
            org,
            period,
            ordinal,
            operation: view.operation(),
            listing: view.listing(),
            charter: view.charter(),
            licence: view.licence(),
            price_minor: view.price_minor(),
            phase: RowPhase::Funded,
            agent,
        };
        // The awaited Fund landed: the quote is done with.
        if let Some(run) = state.runs.get_mut(&org.0)
            && run
                .quoting
                .as_ref()
                .is_some_and(|q| q.listing == row.listing)
        {
            run.quoting = None;
        }
        return record(ctx, fire, state, row);
    }
    // New buys only from here: the agent must be live and, under a backing faction, a member.
    if !live(ctx.w, agent) {
        state.page(
            org,
            "agent",
            "the procurement agent is gone or not a character",
        );
        return Step::Refused;
    }
    // P4: the House's standing is its backing faction's, held by its agent's membership.
    if let Some(faction) = policy.standing
        && !ctx
            .w
            .module_state::<mush_faction::FactionState>()
            .is_some_and(|s| s.is_member(agent, faction))
    {
        state.page(
            org,
            "standing",
            "the agent is not a member of the backing faction",
        );
        return Step::Refused;
    }
    // H6: a month whose rows fill one read closes (its spend and ordinals stay exact).
    if rank::ledger_full(&rows, period, MAX_LEDGER_ROWS) {
        close(state, org, period, "the month's ledger is full");
        return Step::Idle;
    }
    let budget = policy.budget(&defaults);
    let remaining = rank::spent(&rows, period).map_or(0, |spent| budget.saturating_sub(spent));
    // 3. Pay the agent's quote.
    if let Some(quoting) = state.runs.get(&org.0).and_then(|run| run.quoting.clone()) {
        return pay(ctx, fire, state, handles, policy, quoting, remaining);
    }
    // 4. Choose and quote.
    if state.runs.get(&org.0).is_some_and(|run| run.closed) {
        return Step::Idle;
    }
    choose_and_quote(
        ctx,
        fire,
        state,
        handles,
        policy,
        (period, remaining, &rows),
        defaults,
    )
}

/// The organisation's ledger rows of `period` and of the month before it (a purchase funded
/// at a month's end is settled in the next), each month one bounded prefix scan of at most
/// [`MAX_LEDGER_ROWS`]; older months are never read, so a long history costs nothing.
fn ledger_rows(
    ctx: &Ctx<'_>,
    rows: &StateFamilyHandle<ProcurementKey, ProcurementRow>,
    org: OrgId,
    period: Period,
) -> Option<Vec<ProcurementRow>> {
    let mut out = Vec::new();
    for month in [period.previous(), period] {
        let prefix = ProcurementKey::period_prefix(org, month);
        let page = ctx
            .scan_action_state(rows, Some(prefix.as_str()), MAX_LEDGER_ROWS)
            .ok()?;
        out.extend(
            page.entries()
                .iter()
                .take_while(|entry| entry.encoded_key().starts_with(&prefix))
                .map(|entry| entry.value().clone())
                .filter(|row| row.org == org && row.period == month),
        );
    }
    Some(out)
}

fn operation(
    ctx: &Ctx<'_>,
    handles: &Handles,
    operation: CommitToken,
) -> Option<ProcurementOperationView> {
    ctx.read_owner(
        &handles.market,
        MarketCommandRequest::procurement_operation(operation),
    )
    .ok()?
    .into_procurement_operation()?
}

fn own_open(
    ctx: &Ctx<'_>,
    handles: &Handles,
    agent: ObjId,
    org: OrgId,
) -> Option<Vec<OwnOpenView>> {
    ctx.read_owner(
        &handles.market,
        MarketCommandRequest::procurement_open(agent, org),
    )
    .ok()?
    .into_own_open()
}

fn agent_purchase(
    ctx: &Ctx<'_>,
    handles: &Handles,
    agent: ObjId,
    listing: CommitToken,
    org: OrgId,
) -> Option<Option<AgentPurchaseView>> {
    ctx.read_owner(
        &handles.market,
        MarketCommandRequest::procurement_purchase(agent, listing, org),
    )
    .ok()?
    .into_procurement_purchase()
}

/// Submit `action` as the agent with `parameters`, keyed `procurement.<org>.<verb>.<subject>`.
fn submit(
    ctx: &mut Ctx<'_>,
    fire: &mut Fire,
    state: &mut ProcurementState,
    (org, agent): (OrgId, ObjId),
    (verb, subject): (&str, &str),
    action: DefinitionRef,
    parameters: serde_json::Value,
) -> Step {
    let at = ctx.w.world_ic_now().at;
    let Some(key) = state.key(at, org, verb, subject) else {
        state.page(org, "key", "a procurement key did not form");
        return Step::Refused;
    };
    crate::port::stock::swapped(fire.as_principal(ctx, agent, agent, |ctx| {
        ctx.submit_authoritative(action, PerceivedRef::SelfActor, Vec::new(), parameters, key);
    }))
}

/// Record `row` in the ledger, as its agent.
fn record(
    ctx: &mut Ctx<'_>,
    fire: &mut Fire,
    state: &mut ProcurementState,
    row: ProcurementRow,
) -> Step {
    let operation = row.operation.to_string();
    let (org, agent) = (row.org, row.agent);
    state
        .records
        .insert(agent, BTreeMap::from([(operation.clone(), row)]));
    let action = state.action.clone();
    submit(
        ctx,
        fire,
        state,
        (org, agent),
        ("record", &operation),
        action,
        json!({ "operation": operation }),
    )
}

/// Complete a funded purchase (`+minds/buy/complete`'s Grant), or refund it after
/// [`MAX_GRANT_TRIES`] Grants that did not land; after [`MAX_REFUND_TRIES`] Refunds too, page
/// `escrow held` for staff and answer `None`, so the policy moves on (H3). A try is counted
/// when the submission was queued (`Step::Submitted`): a deferral (the principal's pending
/// submission, the drain's back-pressure) never counts, but a queued submission the drain later
/// drops or voids does, so a try can be spent without reaching the market, and the bound then
/// reaches `escrow held` sooner.
fn settle(
    ctx: &mut Ctx<'_>,
    fire: &mut Fire,
    state: &mut ProcurementState,
    handles: &Handles,
    policy: &ProcurementPolicy,
    (row, protocol): (&ProcurementRow, Protocol),
) -> Option<Step> {
    let org = policy.org;
    let tries = state
        .runs
        .get(&org.0)
        .and_then(|run| run.grants.get(&row.operation).copied())
        .unwrap_or(0);
    let (step, verb) = if tries < MAX_GRANT_TRIES {
        (MarketStep::Grant, "complete")
    } else if tries < MAX_GRANT_TRIES.saturating_add(MAX_REFUND_TRIES) {
        (MarketStep::Refund, "refund")
    } else {
        state.page(
            org,
            &format!("escrow held {}", row.operation),
            "a funded purchase neither completes nor refunds; staff must settle its escrow",
        );
        return None;
    };
    let Some(action) = handles.actions.entitlement(protocol, step) else {
        state.page(
            org,
            "terminal",
            "the purchase's terminal action is not registered",
        );
        return Some(Step::Refused);
    };
    let operation = row.operation.to_string();
    let submitted = submit(
        ctx,
        fire,
        state,
        (org, policy.agent),
        (verb, &operation),
        action,
        crate::minds::market_commands::market::terminal_parameters(row.operation),
    );
    if submitted == Step::Submitted
        && let Some(run) = state.runs.get_mut(&org.0)
    {
        let count = run.grants.entry(row.operation).or_insert(0);
        *count = count.saturating_add(1);
    }
    Some(submitted)
}

/// Pay the awaited quote, if it is the AGENT'S (H1: never another member's) and live and still
/// fits the budget; else wait, and skip the listing for the month once it never appears. A Fund
/// sent whose purchase never appeared was refused (the account's approval window, a grant limit,
/// held funds): the listing is skipped for the month and `fund refused` pages (H2). The cap is
/// not re-checked here: nothing the cap counts can change between the Quote and the Fund but
/// this agent's own purchases, which step 1 and step 2 settle first.
fn pay(
    ctx: &mut Ctx<'_>,
    fire: &mut Fire,
    state: &mut ProcurementState,
    handles: &Handles,
    policy: &ProcurementPolicy,
    quoting: Quoting,
    remaining: i64,
) -> Step {
    let (org, agent) = (policy.org, policy.agent);
    let found = agent_purchase(ctx, handles, agent, quoting.listing, org).flatten();
    let now = ctx.w.now();
    if quoting.fund_sent {
        // A landed Fund is recorded by step 2, which clears the quote first.
        if let Some(run) = state.runs.get_mut(&org.0) {
            run.quoting = None;
            if !found.is_some_and(|view| view.funded()) {
                run.skipped.insert(quoting.listing);
            }
        }
        if !found.is_some_and(|view| view.funded()) {
            state.page(
                org,
                &format!("fund refused {}", quoting.listing),
                "the agent's Fund was refused; the listing is skipped for the month",
            );
        }
        return Step::Idle;
    }
    let live = found.filter(|view| view.quote().is_some() && now < view.expires_at_utc());
    let Some(run) = state.runs.get_mut(&org.0) else {
        return Step::Idle;
    };
    let (Some(view), Some(quote)) = (live, live.and_then(|view| view.quote())) else {
        let waits = quoting.waits.saturating_add(1);
        if waits > MAX_QUOTE_WAITS {
            run.skipped.insert(quoting.listing);
            run.quoting = None;
        } else {
            run.quoting = Some(Quoting { waits, ..quoting });
        }
        return Step::Idle;
    };
    if view.price_minor() > remaining {
        run.quoting = None;
        run.skipped.insert(quoting.listing);
        return Step::Idle;
    }
    let Some(action) = handles
        .actions
        .entitlement(view.protocol(), MarketStep::Fund)
    else {
        state.page(org, "fund", "the Fund action is not registered");
        return Step::Refused;
    };
    let subject = quote.to_string();
    let step = submit(
        ctx,
        fire,
        state,
        (org, agent),
        ("pay", &subject),
        action,
        crate::minds::market_commands::market::fund_parameters(quote),
    );
    if step == Step::Submitted
        && let Some(run) = state.runs.get_mut(&org.0)
    {
        run.quoting = Some(Quoting {
            fund_sent: true,
            ..quoting
        });
    }
    step
}

/// Step 4: scan, score, rank, check the cap and the treasury, and quote the best.
fn choose_and_quote(
    ctx: &mut Ctx<'_>,
    fire: &mut Fire,
    state: &mut ProcurementState,
    handles: &Handles,
    policy: &ProcurementPolicy,
    (period, remaining, rows): (Period, i64, &[ProcurementRow]),
    defaults: crate::config::ProcurementConfig,
) -> Step {
    let org = policy.org;
    if remaining <= 0 {
        close(state, org, period, "the month's budget is spent");
        return Step::Idle;
    }
    let Some(unread) = scan(ctx, state, handles, org) else {
        // More listings or ratings to read: on the next fire.
        return Step::Worked;
    };
    let Some(candidates) = candidates(state, policy) else {
        return Step::Worked;
    };
    let hulls = state.hulls(ctx.w, org);
    let cap = rank::cap_for(policy.cap(&defaults), hulls);
    let Some(holdings) = holdings(ctx, handles, org) else {
        state.page(
            org,
            "holdings",
            "the House's current licences are unreadable",
        );
        return Step::Refused;
    };
    let held = rank::held_per_charter(&holdings, rows);
    let skipped = state
        .runs
        .get(&org.0)
        .map(|run| run.skipped.clone())
        .unwrap_or_default();
    let Some(best) = rank::choose(&candidates, remaining, &held, cap, &skipped).copied() else {
        if unread {
            // Round 2: a rating this fire could not read may be the line it would buy; the
            // month stays open and the next fire retries it.
            return Step::Worked;
        }
        close(
            state,
            org,
            period,
            "nothing affordable under the cap is listed",
        );
        return Step::Idle;
    };
    let Some(listing) = state
        .runs
        .get(&org.0)
        .and_then(|run| {
            run.scan
                .listings
                .iter()
                .find(|l| l.view().publication() == best.listing)
        })
        .cloned()
    else {
        return Step::Idle;
    };
    if treasury_available(ctx.w, org, policy.scope, listing.instrument())
        .is_none_or(|available| available < best.price_minor)
    {
        // The ore-bid precedent: a treasury that cannot pay skips the month.
        close(state, org, period, "the treasury cannot pay");
        return Step::Idle;
    }
    let Some(action) = handles
        .actions
        .entitlement(listing.view().protocol(), MarketStep::Quote)
    else {
        state.page(org, "quote", "the Quote action is not registered");
        return Step::Refused;
    };
    let subject = best.listing.to_string();
    let step = submit(
        ctx,
        fire,
        state,
        (org, policy.agent),
        ("buy", &subject),
        action,
        crate::minds::market_commands::market::quote_parameters_for(best.listing, Some(org)),
    );
    if step == Step::Submitted
        && let Some(run) = state.runs.get_mut(&org.0)
    {
        run.quoting = Some(Quoting {
            listing: best.listing,
            waits: 0,
            fund_sent: false,
        });
    }
    step
}

/// Close the month for `org` (logged once per month).
fn close(state: &mut ProcurementState, org: OrgId, period: Period, why: &'static str) {
    if let Some(run) = state.runs.get_mut(&org.0)
        && !run.closed
    {
        run.closed = true;
        run.closed_why = Some(why);
        tracing::info!(
            log_id = "tacsim.procurement.month_closed",
            org = org.0,
            year = period.year,
            month = period.month,
            why,
            "tacsim: a House's procurement month is closed"
        );
    }
}

/// Advance the month's scan within this fire's budget: listings pages, then rating counts.
/// `Some(unread)` once every listing is read and every readable rating scored, `unread` when a
/// rating read failed this fire; `None` while there is more to read. The scan is bounded at
/// [`SCORE_CACHE_BOUND`] listings a month (H4: `scan bound` pages when it is hit); a rating read
/// that fails is not kept, so it is retried on the next fire, and this fire goes on without it:
/// it may still buy the best line it scored, but it never closes the month as having nothing to
/// buy (round 2).
fn scan(
    ctx: &Ctx<'_>,
    state: &mut ProcurementState,
    handles: &Handles,
    org: OrgId,
) -> Option<bool> {
    for _ in 0..MAX_LISTING_PAGES_PER_FIRE {
        let run = state.runs.get(&org.0)?;
        if run.scan.done {
            break;
        }
        let after = run.scan.after;
        let Some(page) = ctx
            .read_owner(
                &handles.market,
                MarketCommandRequest::procurement_listings(after),
            )
            .ok()
            .and_then(mush_market::mind_commands::MarketCommandResult::into_procurement_listings)
        else {
            state.page(org, "listings", "the market's listings are unreadable");
            return None;
        };
        let run = state.runs.get_mut(&org.0)?;
        let room = SCORE_CACHE_BOUND.saturating_sub(run.scan.listings.len());
        let bounded = page.rows().len() > room;
        run.scan
            .listings
            .extend(page.rows().iter().take(room).cloned());
        run.scan.after = page.next();
        run.scan.done = page.next().is_none() || bounded;
        if bounded || (run.scan.listings.len() >= SCORE_CACHE_BOUND && page.next().is_some()) {
            run.scan.done = true;
            state.page(
                org,
                "scan bound",
                "more listings than one month's scan reads; the rest wait for next month",
            );
        }
    }
    let mut reads = 0;
    let mut failed: BTreeSet<String> = BTreeSet::new();
    loop {
        let run = state.runs.get(&org.0)?;
        if !run.scan.done {
            return None;
        }
        let pending = match run.scan.tally.clone() {
            Some(pending) => Some(pending),
            None => run.scan.listings.iter().find_map(|l| {
                let key = cache_key(l)?;
                (!run.scores.contains_key(&key) && !failed.contains(&key))
                    .then(|| (key, l.rating(), 0, RatingTally::default()))
            }),
        };
        let Some((key, rating, from, tally)) = pending else {
            return Some(!failed.is_empty());
        };
        if reads >= MAX_TALLY_READS_PER_FIRE {
            if let Some(run) = state.runs.get_mut(&org.0) {
                run.scan.tally = Some((key, rating, from, tally));
            }
            return None;
        }
        reads += 1;
        let page = ctx
            .read_owner(
                &handles.roll,
                OriginalReadRequest::command(RollCommandReadRequest::procurement_tally(
                    rating, from,
                )),
            )
            .ok()
            .and_then(mush_roll::commerce::holding_reader::OriginalReadResult::into_command)
            .and_then(
                mush_roll::commerce::command_reader::RollCommandReadResult::into_procurement_tally,
            );
        let Some((next, merged)) = page.and_then(|p| Some((p.next(), tally.merge(p.tally())?)))
        else {
            // H4: not kept; this fire goes on without it, the next retries it.
            state.page(org, "rating", "a listed revision's rating is unreadable");
            failed.insert(key);
            if let Some(run) = state.runs.get_mut(&org.0) {
                run.scan.tally = None;
            }
            continue;
        };
        let run = state.runs.get_mut(&org.0)?;
        match next {
            Some(from) => run.scan.tally = Some((key, rating, from, merged)),
            None => {
                run.scores.insert(key, merged.score());
                run.scan.tally = None;
            }
        }
    }
}

/// The month's score key: `(revision, rating id)` (P1); `None` (the listing skipped) when the
/// revision does not encode.
fn cache_key(listing: &ProcurementListing) -> Option<String> {
    let revision = serde_json::to_string(&listing.revision()).ok()?;
    Some(format!("{revision}/{}", listing.rating().id()))
}

/// The scanned listings the policy may buy, each with its score: its form in `roles` and a
/// passing score above 0‰ (P1, P4; H9: a 0‰ line is never a candidate). A listing not scored
/// this fire is left out.
fn candidates(state: &ProcurementState, policy: &ProcurementPolicy) -> Option<Vec<Candidate>> {
    let run = state.runs.get(&policy.org.0)?;
    let mut out = Vec::new();
    for listing in &run.scan.listings {
        let view = listing.view();
        if !rank::admits(&policy.roles, view.form()) {
            continue;
        }
        let Some(Some(score)) = cache_key(listing).and_then(|key| run.scores.get(&key).copied())
        else {
            continue;
        };
        if score.permille == 0 && !state.zero_admitted() {
            continue;
        }
        out.push(Candidate {
            listing: view.publication(),
            charter: view.charter(),
            price_minor: view.price_minor(),
            permille: score.permille,
        });
    }
    Some(out)
}

/// The organisation's current licences by charter (the Roll's GOD-only read, paged).
fn holdings(ctx: &Ctx<'_>, handles: &Handles, org: OrgId) -> Option<Vec<(LicenceId, CharterId)>> {
    let mut out = Vec::new();
    let mut after = None;
    for _ in 0..MAX_HOLDINGS_READS {
        let page = ctx
            .read_owner(
                &handles.roll,
                OriginalReadRequest::command(RollCommandReadRequest::procurement_holdings(
                    org, after,
                )),
            )
            .ok()?
            .into_command()?
            .into_procurement_holdings()?;
        out.extend(page.rows().iter().map(|h| (h.licence(), h.charter())));
        match page.next() {
            Some(next) => after = Some(next),
            None => return Some(out),
        }
    }
    tracing::warn!(
        log_id = "tacsim.procurement.holdings_cut",
        org = org.0,
        "tacsim: a House holds more licences than one choice reads; the cap counts the first"
    );
    Some(out)
}

/// **`H`, the organisation's titled hulls** (P6): its Active E4 titles whose object is a world
/// hull.
fn titled_hulls(w: &World, org: OrgId) -> u32 {
    let hulls = crate::surface::recovery::world_hulls(w);
    let Some(economy) = w.module_state::<mush_economy::EconomyStore>() else {
        return 0;
    };
    let count = economy
        .snapshot()
        .assets_of(&mush_org::PrincipalId::Organization(org))
        .take(MAX_HULL_ASSETS)
        .filter(|asset| {
            asset.lifecycle == mush_economy::AssetLifecycle::Active && hulls.contains(&asset.object)
        })
        .count();
    u32::try_from(count).unwrap_or(u32::MAX)
}

/// The organisation's treasury on `scope` (its root) in the listing's `instrument` (the account
/// the market's Quote freezes as the payer): posted less held, in minor units.
fn treasury_available(
    w: &World,
    org: OrgId,
    scope: mush_org::OrgScopeId,
    instrument: &mush_economy::InstrumentId,
) -> Option<i64> {
    let economy = w.module_state::<mush_economy::EconomyStore>()?;
    if economy.is_frozen() {
        return None;
    }
    let snapshot = economy.snapshot();
    let account = snapshot
        .accounts_of(&mush_org::PrincipalId::Organization(org))
        .find(|account| {
            account.instrument == *instrument
                && account.scope == Some(scope)
                && account.status == mush_economy::AccountStatus::Active
        })?;
    account
        .posted_balance
        .minor()
        .checked_sub(account.held_amount.minor())
}
