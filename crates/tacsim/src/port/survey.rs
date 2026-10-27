//! Occator's finite survey board. Contract money stays in the economy; accepted
//! work and completion evidence live in the authoritative survey family.

mod standing;

use mush_core::{CanonicalId, CmdError, Ctx, ObjId, World};
use mush_economy::{
    AcceptOutcome, AccountId, AccountStatus, Amount, ConsentSurface, Consideration,
    ConsiderationSpec, ContractId, ContractRoles, ContractSpec, ContractState, ContractTerms,
    EconomicClock, EconomicDeadline, EconomicVisibility, EconomyStore, EvidenceKind, Expiry,
    ExternalRef, OutcomeTerms, PayoutShare, Share, TypedCondition,
};
use mush_org::{OrgId, PrincipalId};

use crate::port::families::PortHandles;
use crate::store::TacsimStore;
use crate::surface::survey::{SurveyKey, SurveyRecord};

/// Player command.
pub const SURVEY_VERB: &str = "+survey";
/// The stable fact namespace, composed only with the game layer.
pub const FACT_PROVIDER: &str = "tacsim.survey";
/// Proof of completed commissioned work, not a disclosure of the pilot's chart.
pub const FACT_KIND: &str = "tacsim.survey.logged";
/// The contractor's recorded withdrawal returns the reward to Occator.
pub const CANCEL_FACT_KIND: &str = "tacsim.survey.cancelled";
/// Work completed at the deadline has one more IC day to settle.
pub const SETTLEMENT_GRACE_IC: i64 = 86_400;
/// Command syntax.
pub const SURVEY_USAGE: &str =
    "Usage: +survey [<id>]  or  +survey/accept <id> [go]  or  +survey/cancel <id> [go]";
/// Command help.
pub const SURVEY_HELP: &str = "+survey  --  Occator's survey commissions. Each commission pays \
    500.00cr for fresh detailed soundings of all 256 pockets in one block, within seven IC days. \
    +survey <id> shows its terms and your progress. +survey/accept <id> previews; add go to file. \
    Occator reserves the reward after the filing lands. Sound and quiet sound both count, by hand \
    or plan; work must begin strictly after the acceptance time. One commission per account at a time, and \
    one payment per board entry. New commissions grant +10 Occator standing when completed and paid; \
    the acceptance preview lists any authored effects on other institutions. \
    +survey/cancel <id> [go] returns unfinished funding to Occator.";
const UNAVAILABLE: &str = "Occator's survey board cannot be read right now.";
const NO_ACCOUNT: &str = "Occator needs your own active credit account and a registered pilot account before it can commission you.";
const TAKEN: &str = "That commission is already taken or completed.";
const BUSY: &str = "Your account already has an unfinished survey commission.";
const UNFUNDED: &str = "Occator cannot fund that commission right now; nothing was charged.";
const NOT_YOURS: &str = "You hold no unfinished commission by that name.";

/// One authored, globally finite commission. A paid entry never reopens; future
/// boards must use new IDs instead of paying again for the same retained proof.
#[derive(Clone, Copy, Debug)]
pub struct SurveyOffer {
    pub id: &'static str,
    pub site: &'static str,
    pub site_name: &'static str,
    pub block: u8,
    pub reward_minor: i64,
    pub term_ic_s: i64,
    pub revision: u64,
    /// Beneficiary standing granted only for the paid, completed commission.
    pub standing: i16,
    /// Explicit adverse consequences; the shipped surveys harm no other institution.
    pub secondary_effects: &'static [SurveySecondaryEffect],
}

#[derive(Clone, Copy, Debug)]
pub struct SurveySecondaryEffect {
    /// A legal person authored in the programme port's organization map.
    pub organization: &'static str,
    pub delta: i16,
}

const OFFERS: [SurveyOffer; 4] = [
    SurveyOffer {
        id: "ahuna-a",
        site: "ahuna_port",
        site_name: "Ahuna Flank",
        block: 0,
        reward_minor: 50_000,
        term_ic_s: 604_800,
        revision: 1,
        standing: 10,
        secondary_effects: &[],
    },
    SurveyOffer {
        id: "kerwan-f",
        site: "kerwan_port",
        site_name: "Kerwan Floor",
        block: 5,
        reward_minor: 50_000,
        term_ic_s: 604_800,
        revision: 1,
        standing: 10,
        secondary_effects: &[],
    },
    SurveyOffer {
        id: "juling-k",
        site: "juling_port",
        site_name: "Juling Shadow",
        block: 10,
        reward_minor: 50_000,
        term_ic_s: 604_800,
        revision: 1,
        standing: 10,
        secondary_effects: &[],
    },
    SurveyOffer {
        id: "occator-p",
        site: "occator_port",
        site_name: "Occator Rim",
        block: 15,
        reward_minor: 50_000,
        term_ic_s: 604_800,
        revision: 1,
        standing: 10,
        secondary_effects: &[],
    },
];

#[must_use]
pub fn offers() -> &'static [SurveyOffer] {
    &OFFERS
}

#[must_use]
pub fn offer(id: &str) -> Option<&'static SurveyOffer> {
    OFFERS.iter().find(|row| row.id == id)
}

/// Scoped attachment preflight sees only the exact candidate this synchronous
/// command authored. It is cleared before returning, never persisted or trusted
/// as completed work.
#[derive(Clone, Default)]
pub(crate) struct AttachmentCandidate(Option<SurveyRecord>);

pub(crate) fn attachment_candidate(
    w: &World,
    actor: ObjId,
    contract: ContractId,
) -> Option<SurveyRecord> {
    let row = w.module_state::<AttachmentCandidate>()?.0.as_ref()?;
    (row.pilot == actor && row.contract == contract.0).then(|| row.clone())
}

pub(crate) fn fact_ref(row: &SurveyRecord) -> ExternalRef {
    ExternalRef {
        provider: FACT_PROVIDER.into(),
        kind: FACT_KIND.into(),
        id: standing::fact_id(row).into(),
        revision: Some(row.revision),
    }
}

pub(crate) fn cancel_fact_ref(row: &SurveyRecord) -> ExternalRef {
    ExternalRef {
        kind: CANCEL_FACT_KIND.into(),
        ..fact_ref(row)
    }
}

pub(crate) fn contract_spec(row: &SurveyRecord) -> ContractSpec {
    ContractSpec {
        name: format!("Occator survey: {}", row.offer.key_text()).into(),
        scope: None,
        parties: vec![row.sponsor.clone()],
        considerations: vec![ConsiderationSpec {
            account: AccountId(row.sponsor_account),
            amount: Amount::from_minor(row.reward_minor),
        }],
        outcomes: vec![
            OutcomeTerms {
                name: "survey_logged".into(),
                when: vec![0],
                payouts: vec![PayoutShare {
                    to: AccountId(row.payee),
                    share: Share::Exact(Amount::from_minor(row.reward_minor)),
                }],
                expense_treatment: mush_economy::ExpenseTreatment::None,
                asset_consequences: Vec::new(),
            },
            OutcomeTerms {
                name: "survey_cancelled".into(),
                when: vec![1],
                payouts: vec![PayoutShare {
                    to: AccountId(row.sponsor_account),
                    share: Share::Exact(Amount::from_minor(row.reward_minor)),
                }],
                expense_treatment: mush_economy::ExpenseTreatment::None,
                asset_consequences: Vec::new(),
            },
        ],
        remainder_to: AccountId(row.sponsor_account),
        conditions: vec![
            TypedCondition::Fact {
                fact: fact_ref(row),
            },
            TypedCondition::Fact {
                fact: cancel_fact_ref(row),
            },
        ],
        adjudicator: None,
        evidence: vec![EvidenceKind::Fact],
        deadline: Some(EconomicDeadline {
            clock: EconomicClock::WorldIc,
            at: row.deadline_ic.saturating_add(SETTLEMENT_GRACE_IC),
        }),
        on_expiry: Expiry::Refund,
        visibility: EconomicVisibility::Public,
        roles: ContractRoles {
            author: Some(PrincipalId::Object(row.pilot)),
            payer: Some(row.sponsor.clone()),
            representative: None,
            facilitator: None,
        },
        deterministic: true,
        deliverables: Vec::new(),
    }
}

/// Exact consent before the economy adds its scoped attachment marker.
pub(crate) fn expected_consent(row: &SurveyRecord) -> ConsentSurface {
    let spec = contract_spec(row);
    ConsentSurface {
        fact_consent: None,
        managed: None,
        parties: vec![PrincipalId::Object(row.pilot), row.sponsor.clone()],
        terms: ContractTerms {
            consideration: vec![Consideration {
                from: row.sponsor.clone(),
                account: AccountId(row.sponsor_account),
                amount: Amount::from_minor(row.reward_minor),
            }],
            deliverables: spec.deliverables,
            split: Vec::new(),
            remainder_to: spec.remainder_to,
            on_expiry: spec.on_expiry,
            outcomes: spec.outcomes,
        },
        conditions: spec.conditions,
        deadline: spec.deadline,
        visibility: spec.visibility,
        scope: spec.scope,
        adjudicator: spec.adjudicator,
        evidence: spec.evidence,
    }
}

/// Retained contracts must still carry exactly the authored consent and the
/// pilot's original acceptance; names and arbitrary party membership grant nothing.
pub(crate) fn contract_valid(w: &World, row: &SurveyRecord) -> bool {
    let Some(store) = w.module_state::<EconomyStore>() else {
        return false;
    };
    let Some(c) = store.snapshot().contract(ContractId(row.contract)) else {
        return false;
    };
    let mut consent = c.consent.clone();
    consent.fact_consent = None;
    c.issuer == PrincipalId::Object(row.pilot)
        && c.amendments == 0
        && c.consent.fact_consent.is_some()
        && c.consent_intact()
        && consent == expected_consent(row)
        && c.acceptances
            .iter()
            .any(|a| a.party == c.issuer && a.consent_hash == c.consent_hash)
}

pub(crate) fn contract_active(w: &World, row: &SurveyRecord) -> bool {
    if !contract_valid(w, row) {
        return false;
    }
    let Some(store) = w.module_state::<EconomyStore>() else {
        return false;
    };
    let Some(c) = store.snapshot().contract(ContractId(row.contract)) else {
        return false;
    };
    if !c.state.is_live()
        || c.pending.is_some()
        || !c
            .acceptances
            .iter()
            .any(|a| a.party == row.sponsor && a.consent_hash == c.consent_hash)
    {
        return false;
    }
    let held = c.escrow.iter().try_fold(0_i64, |total, id| {
        let hold = store.snapshot().hold(*id)?;
        if hold.account != AccountId(row.sponsor_account) || !hold.status.is_held() {
            return None;
        }
        total.checked_add(hold.amount.minor())
    });
    held == Some(row.reward_minor)
}

pub(crate) fn contract_replaceable(w: &World, row: &SurveyRecord) -> bool {
    if row.completed.is_some() || !contract_valid(w, row) {
        return false;
    }
    let Some(store) = w.module_state::<EconomyStore>() else {
        return false;
    };
    let Some(c) = store.snapshot().contract(ContractId(row.contract)) else {
        return false;
    };
    (matches!(c.state, ContractState::Cancelled | ContractState::Expired)
        || (row.cancelled.is_some() && matches!(c.state, ContractState::Settled { .. })))
        && c.pending.is_none()
        && c.escrow.iter().all(|id| {
            store
                .snapshot()
                .hold(*id)
                .is_some_and(|h| !h.status.is_held())
        })
}

pub(crate) fn records(ctx: &Ctx<'_>) -> Result<Vec<SurveyRecord>, String> {
    let handles = ctx.w.module_state::<PortHandles>().ok_or(UNAVAILABLE)?;
    let mut result = Vec::new();
    for offer in offers() {
        let key = SurveyKey::parse(offer.id).ok_or(UNAVAILABLE)?;
        if let Some(row) = ctx
            .action_state(&handles.families.surveys, &key)
            .map_err(|_| UNAVAILABLE)?
        {
            row.check().map_err(|_| UNAVAILABLE)?;
            if row.offer != key {
                return Err(UNAVAILABLE.into());
            }
            result.push(row.clone());
        }
    }
    Ok(result)
}

fn sponsor(w: &World) -> Option<(PrincipalId, AccountId)> {
    let port = crate::port::coop::programme_port(w)?;
    let org = port.map.orgs.get(&CanonicalId::new("occator").ok()?)?;
    let principal = PrincipalId::Organization(OrgId(org.org));
    let account = AccountId(org.account?);
    let funds = w
        .module_state::<EconomyStore>()?
        .snapshot()
        .account(account)?;
    (funds.owner == principal
        && funds.status == AccountStatus::Active
        && funds.instrument.as_str() == crate::economy::CREDIT)
        .then_some((principal, account))
}

fn site_available(w: &World, offer: &SurveyOffer) -> bool {
    let Some(store) = w.module_state::<TacsimStore>() else {
        return false;
    };
    let (Ok(pack), Ok(site)) = (
        CanonicalId::new(crate::definitions::content::LANE_PACK),
        CanonicalId::new(offer.site),
    ) else {
        return false;
    };
    matches!(store.port_map(&pack, &site), crate::port::PortMapState::Present(map) if map.site.is_some())
}

fn plan(ctx: &Ctx<'_>, offer: &SurveyOffer) -> Result<SurveyRecord, String> {
    let w: &World = ctx.w;
    if !mush_economy::has_fact_provider(w, FACT_PROVIDER) || !site_available(w, offer) {
        return Err(UNAVAILABLE.into());
    }
    let handles = w.module_state::<PortHandles>().ok_or(UNAVAILABLE)?;
    if handles.survey_accept.is_none() {
        return Err(UNAVAILABLE.into());
    }
    let account = crate::port::licence::gateway_account(w, ctx.actor).ok_or(NO_ACCOUNT)?;
    let payee = crate::port::credit::own_account(w, ctx.actor)
        .ok_or(NO_ACCOUNT)?
        .id;
    for row in records(ctx)? {
        if row.offer.key_text() == offer.id && !contract_replaceable(w, &row) {
            return Err(TAKEN.into());
        }
        if row.account == account && row.completed.is_none() && !contract_replaceable(w, &row) {
            return Err(BUSY.into());
        }
    }
    let (sponsor, sponsor_account) = sponsor(w).ok_or(UNAVAILABLE)?;
    let economy = w.module_state::<EconomyStore>().ok_or(UNAVAILABLE)?;
    if economy.is_frozen() {
        return Err(UNAVAILABLE.into());
    }
    if economy
        .snapshot()
        .account(sponsor_account)
        .and_then(|a| a.available())
        .is_none_or(|a| a.minor() < offer.reward_minor)
    {
        return Err(UNFUNDED.into());
    }
    let now = w.world_ic_now().at;
    let standing = Some(standing::capture(w, offer, &sponsor)?);
    let row = SurveyRecord {
        schema: crate::surface::survey::SURVEY_SCHEMA,
        offer: SurveyKey::parse(offer.id).ok_or(UNAVAILABLE)?,
        revision: offer.revision,
        account,
        pilot: ctx.actor,
        sponsor,
        sponsor_account: sponsor_account.0,
        payee: payee.0,
        contract: economy.peek_next_contract(),
        site: CanonicalId::new(offer.site).map_err(|_| UNAVAILABLE)?,
        block: offer.block,
        reward_minor: offer.reward_minor,
        accepted_ic: now,
        deadline_ic: now.checked_add(offer.term_ic_s).ok_or(UNAVAILABLE)?,
        coverage: [0; 32],
        completed: None,
        cancelled: None,
        standing,
    };
    row.check().map_err(|_| UNAVAILABLE)?;
    Ok(row)
}

fn say(ctx: &mut Ctx<'_>, line: impl Into<String>) {
    ctx.out.sys_to_actor(ctx.actor, "tacsim", line);
}

fn show(ctx: &mut Ctx<'_>, id: Option<&str>) -> Result<(), CmdError> {
    let rows = records(ctx).map_err(CmdError::msg)?;
    if id.is_some_and(|id| offer(id).is_none()) {
        return Err(CmdError::msg(SURVEY_USAGE));
    }
    say(
        ctx,
        "Occator survey commissions — one payment per entry; one active commission per account.",
    );
    for offer in offers().iter().filter(|o| id.is_none_or(|id| o.id == id)) {
        let row = rows.iter().find(|r| r.offer.key_text() == offer.id);
        let state = match row {
            None => "open".to_owned(),
            Some(row) if contract_replaceable(ctx.w, row) => "open".to_owned(),
            Some(row) => {
                let contract = ctx
                    .w
                    .module_state::<EconomyStore>()
                    .and_then(|s| s.snapshot().contract(ContractId(row.contract)));
                if matches!(
                    contract.map(|c| c.state),
                    Some(ContractState::Settled { .. })
                ) {
                    "completed and paid".to_owned()
                } else if row.pilot != ctx.actor {
                    "taken".to_owned()
                } else {
                    let funding = if !contract_valid(ctx.w, row) {
                        "contract needs staff review"
                    } else if matches!(
                        contract.map(|c| c.state),
                        Some(ContractState::Cancelled | ContractState::Expired)
                    ) {
                        "contract ended; refund pending"
                    } else if row.cancelled.is_some() {
                        "cancelled; refund pending"
                    } else if row.completed.is_some() {
                        "survey logged; settlement pending"
                    } else if ctx.w.world_ic_now().at > row.deadline_ic {
                        "work deadline passed; awaiting expiry and refund"
                    } else if contract_active(ctx.w, row) {
                        "reward escrowed"
                    } else {
                        "waiting for funding"
                    };
                    format!(
                        "yours: {}/256 pockets; {funding}; due {}",
                        row.covered(),
                        crate::port::licence::ic_stamp(row.deadline_ic)
                    )
                }
            }
        };
        let reward = crate::port::coop::money_minor(ctx, offer.reward_minor);
        say(
            ctx,
            format!(
                "{}: {}, block {} — {reward}, 7 IC days; {state}.",
                offer.id,
                offer.site_name,
                char::from(b'A' + offer.block)
            ),
        );
        if id.is_some() {
            say(
                ctx,
                "Sound all four fields of the block (NW, NE, SW, SE). Both sound and sound quiet count, including a fill plan; scans, cores and earlier readings do not. Your chart stays yours.",
            );
            let terms = row
                .filter(|row| !contract_replaceable(ctx.w, row))
                .and_then(|row| row.standing.clone())
                .or_else(|| {
                    row.is_none_or(|row| contract_replaceable(ctx.w, row))
                        .then(|| sponsor(ctx.w))
                        .flatten()
                        .and_then(|(sponsor, _)| standing::capture(ctx.w, offer, &sponsor).ok())
                });
            if let Some(terms) = terms {
                say(ctx, standing::line(ctx.w, &terms));
            }
        }
    }
    Ok(())
}

fn accept(ctx: &mut Ctx<'_>, id: &str, go: bool) -> Result<(), CmdError> {
    let offer = offer(id).ok_or_else(|| CmdError::msg(SURVEY_USAGE))?;
    if go {
        crate::port::relationships::reconcile(ctx).map_err(CmdError::msg)?;
    }
    let row = plan(ctx, offer).map_err(CmdError::msg)?;
    if !go {
        let reward = crate::port::coop::money_minor(ctx, row.reward_minor);
        say(
            ctx,
            format!(
                "Occator offers {reward} for fresh detailed soundings of all 256 pockets in {} block {}, within 7 IC days. +survey/accept {id} go files it; funding follows once the filing lands.",
                offer.site_name,
                char::from(b'A' + offer.block)
            ),
        );
        if let Some(terms) = &row.standing {
            say(ctx, standing::preview_line(ctx.w, terms));
        }
        return Ok(());
    }
    let action = ctx
        .w
        .module_state::<PortHandles>()
        .and_then(|h| h.survey_accept.clone())
        .ok_or_else(|| CmdError::msg(UNAVAILABLE))?;
    let key = crate::surface::survey::accept_key(id, row.contract)
        .ok_or_else(|| CmdError::msg(UNAVAILABLE))?;
    ctx.w
        .module_state_mut::<AttachmentCandidate>()
        .ok_or_else(|| CmdError::msg(UNAVAILABLE))?
        .0 = Some(row.clone());
    let offered = mush_economy::offer_contract(ctx, contract_spec(&row));
    if let Some(candidate) = ctx.w.module_state_mut::<AttachmentCandidate>() {
        candidate.0 = None;
    }
    let contract = offered.map_err(|error| {
        tracing::warn!(%error, "survey commission could not be offered");
        CmdError::msg(UNAVAILABLE)
    })?;
    if contract.0 != row.contract
        || !matches!(
            mush_economy::accept_contract(ctx, contract),
            Ok(AcceptOutcome::Accepted { .. })
        )
    {
        let _ = mush_economy::cancel_contract(ctx, contract);
        ctx.out.pin_deferred();
        return Err(CmdError::msg(UNAVAILABLE));
    }
    let standing_line = row
        .standing
        .as_ref()
        .map(|terms| standing::line(ctx.w, terms));
    if let Err(error) = crate::surface::survey::put_order(ctx, row) {
        tracing::warn!(%error, "survey commission could not be queued");
        let _ = mush_economy::cancel_contract(ctx, contract);
        ctx.out.pin_deferred();
        return Err(CmdError::msg(UNAVAILABLE));
    }
    ctx.submit_authoritative(
        action,
        mush_core::PerceivedRef::SelfActor,
        Vec::new(),
        serde_json::json!({"offer": id, "contract": contract.0}),
        key,
    );
    say(
        ctx,
        format!(
            "Survey commission {id} filed as contract #{}. Occator reserves the reward once the filing lands; +survey {id} shows progress.",
            contract.0
        ),
    );
    if let Some(line) = standing_line {
        say(ctx, line);
    }
    Ok(())
}

fn cancel(ctx: &mut Ctx<'_>, id: &str, go: bool) -> Result<(), CmdError> {
    let row = records(ctx)
        .map_err(CmdError::msg)?
        .into_iter()
        .find(|r| {
            r.offer.key_text() == id
                && r.pilot == ctx.actor
                && r.completed.is_none()
                && r.cancelled.is_none()
        })
        .ok_or_else(|| CmdError::msg(NOT_YOURS))?;
    if !contract_valid(ctx.w, &row) || contract_replaceable(ctx.w, &row) {
        return Err(CmdError::msg(NOT_YOURS));
    }
    if !go {
        say(
            ctx,
            format!(
                "Cancel {id} and return its reserved reward to Occator. +survey/cancel {id} go confirms."
            ),
        );
        return Ok(());
    }
    if contract_active(ctx.w, &row) {
        let action = ctx
            .w
            .module_state::<PortHandles>()
            .and_then(|h| h.survey_cancel.clone())
            .ok_or_else(|| CmdError::msg(UNAVAILABLE))?;
        let key = crate::surface::survey::cancel_key(id, row.contract)
            .ok_or_else(|| CmdError::msg(UNAVAILABLE))?;
        ctx.submit_authoritative(
            action,
            mush_core::PerceivedRef::SelfActor,
            Vec::new(),
            serde_json::json!({"offer": id, "contract": row.contract}),
            key,
        );
        say(
            ctx,
            format!(
                "Cancellation of {id} filed. Its reward returns to Occator once the cancellation lands and settles."
            ),
        );
        return Ok(());
    }
    match mush_economy::cancel_contract(ctx, ContractId(row.contract)) {
        Ok(true) => say(
            ctx,
            format!(
                "Commission {id} cancelled. Any reserved reward returns to Occator when its refund settles."
            ),
        ),
        Ok(false) => say(
            ctx,
            format!("Commission {id} is ending; its refund is still pending."),
        ),
        Err(error) => {
            tracing::warn!(%error, "survey cancellation refused");
            return Err(CmdError::msg(UNAVAILABLE));
        }
    }
    Ok(())
}

pub(crate) fn survey_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if !ctx
        .w
        .module_state::<TacsimStore>()
        .is_some_and(TacsimStore::enabled)
    {
        return Err(CmdError::msg(crate::commands::NOT_ENABLED));
    }
    let raw = ctx.args.raw().trim().to_ascii_lowercase();
    let words: Vec<_> = raw.split_whitespace().collect();
    if ctx.switches.has("accept") || ctx.switches.has("cancel") {
        if ctx.switches.has("accept") && ctx.switches.has("cancel") {
            return Err(CmdError::msg(SURVEY_USAGE));
        }
        let (id, go) = match words.as_slice() {
            [id] => (*id, false),
            [id, "go"] => (*id, true),
            _ => return Err(CmdError::msg(SURVEY_USAGE)),
        };
        if ctx.switches.has("accept") {
            accept(ctx, id, go)
        } else {
            cancel(ctx, id, go)
        }
    } else {
        match words.as_slice() {
            [] | ["board"] => show(ctx, None),
            [id] => show(ctx, Some(id)),
            _ => Err(CmdError::msg(SURVEY_USAGE)),
        }
    }
}

/// Funding follows durable acceptance. An orphaned offer has no survey row and
/// can never debit Occator. Refused funding remains retryable until its deadline.
pub(crate) fn tick(ctx: &mut Ctx<'_>) {
    let Ok(rows) = records(ctx) else {
        return;
    };
    for row in rows {
        crate::surface::survey::forget_order(ctx.w, row.pilot, row.contract);
        if let Err(error) = standing::reconcile(ctx, &row) {
            tracing::warn!(contract = row.contract, %error, "survey standing awaits reconciliation");
        }
        if row.completed.is_some()
            || ctx.w.world_ic_now().at > row.deadline_ic
            || !contract_valid(ctx.w, &row)
        {
            continue;
        }
        let offered = ctx
            .w
            .module_state::<EconomyStore>()
            .and_then(|s| s.snapshot().contract(ContractId(row.contract)))
            .is_some_and(|c| c.state == ContractState::Offered && c.pending.is_none());
        if !offered {
            continue;
        }
        if let Ok(AcceptOutcome::Accepted { .. }) =
            mush_economy::accept_contract_as(ctx, ContractId(row.contract), &row.sponsor)
        {
            let reward = crate::port::coop::money_minor(ctx, row.reward_minor);
            ctx.out.sys_to_actor(
                row.pilot,
                "tacsim",
                format!(
                    "Occator has escrowed {reward} for survey {}.",
                    row.offer.key_text()
                ),
            );
        }
    }
}
