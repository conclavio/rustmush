//! Contracts, live (E4b2, 08c §5) — the `@contract/*` surface and the machinery
//! the sweep shares with it.
//!
//! E4a built the pure consent carrier: a hashed consent surface, acceptances bound
//! to it, live re-verification, [`ContractAuth`], and the custody lock. This module
//! is what makes those things happen to real money, and four rules shape every line
//! of it:
//!
//! - **The acceptance is the authority, and it is collected under the accepting
//!   party's own live rights.** `@contract/accept` places that party's consideration
//!   in the same step, through the ordinary `post` seam under `WorldAuth`, so debit
//!   authority is proved **independently** of the right to accept (D-E4-4). An org
//!   member with `contract.accept` and no `economy.account.post` escrows nothing.
//! - **Escrow belongs to the contract.** A hold placed here has
//!   `HoldPurpose::Escrow { contract }` and a `System` beneficiary nobody acts for,
//!   and `WorldAuth::authorize_hold` refuses every op on such a hold. The only
//!   authority that can capture or release it is [`ContractAuth`], built from the
//!   live record at the moment of use.
//! - **The state change comes first, then the money.** A refund runs *after* the
//!   contract has moved to `Cancelled`/`Expired`, because that is what lifts the
//!   custody lock (E4a verify fold, finding 2); a settlement is a single plan that
//!   captures, pays out, moves title and releases pledges, or does none of it.
//! - **Nothing here panics.** The sweep drives the same functions, and a panic in a
//!   tick handler aborts the world — so there is no `unwrap`, no `expect`, and no
//!   indexing that can panic on any path the sweep can reach.

use std::sync::Arc;

use mush_core::module::{ModError, Registrar};
use mush_core::{ArgShape, CmdError, CommandSpec, Ctx, ObjId, World};
use mush_org::{CapabilityId, OrgScopeId, PrincipalId};
use smol_str::SmolStr;

use crate::api::{self, PostAuth, PostOutcome};
use crate::clock::{EconomicClock, EconomicDeadline, EconomicSchedule};
use crate::commands::{
    asset_target, cmd_key, emit_list, info, is_admin, may_view_account, parse_account, parse_amount,
    parse_deadline, parse_opts, parse_principal, parse_scope, parse_vault_id, refuse,
    refuse_if_frozen, render_principal,
};
use crate::contract::{
    Acceptance, Attestation, AttestorRef, AttestorSet, ConsentSurface, Consideration, ContractAuth,
    ContractError, ContractRoles, ContractState, ContractTerms, DRAFT_TTL_SECS,
    Deliverable, DepositRecord, EconomicContract, EvidenceKind, Expiry, MAX_AGREEMENTS,
    MAX_AMENDMENTS, MAX_CONDITIONS, MAX_CONTRACT_NAME_BYTES, MAX_DELIVERABLES, MAX_DEPOSITS,
    MAX_LIVE_CONTRACTS_PER_PARTY, MAX_OPEN_CONTRACTS_PER_ISSUER, MAX_OUTCOMES, MAX_PARTIES,
    MAX_SETTLE_ATTEMPTS, MAX_SHARES, ObligationTemplate, OutcomeTerms, PERMILLE, PartyAgreement,
    PayoutShare, Readiness, Resolution, Share, TypedCondition, activate_key, check_offerable,
    consent_hash, forfeit_plan, principal_key, readiness, refund_plan, resolve_outcome,
    settlement_plan,
};
use crate::custody::WorldCustody;
use crate::engine::{NoFacts, ValidateEnv};
use crate::escalation::{EscalationKind, EscalationSubject};
use crate::ids::{AccountId, ContractId, HoldId};
use crate::module::EconomyStore;
use crate::obligation::{GracePolicy, Obligation, ObligationCause, ObligationStatus};
use crate::plan::{EconomyPlan, HoldOp, NamedResource};
use crate::record::{
    AccountStatus, EconomicCause, EconomicVisibility, ExternalRef, HoldEffectKind, HoldPurpose,
    MAX_AUDIENCE,
};

/// The refusal text every unviewable contract renders as (§4.5's existence hiding):
/// "denied" and "there is no such contract" must be one message.
const NO_SUCH: &str = "No such contract.";

// ---- view standing --------------------------------------------------------

/// §4.5 view standing over a contract (doc 10 §15 / T6): staff, the issuer, a party,
/// a member of the declared **audience**, or anyone at all when the surface says
/// `Public`.
///
/// Party and audience membership both ask [`api::WorldAuth::acts_for`] rather than
/// identity, for the reason E4b1's fold gave for holds (A2): a contract with an
/// organization as a party would otherwise be invisible to every person in it,
/// including the ones who will have to accept and fund it.
///
/// The renderer treats a contract id exactly like an account id for everyone else,
/// so the nonexistent and the unviewable produce identical text.
#[must_use]
pub fn may_view_contract(w: &World, actor: ObjId, id: ContractId) -> bool {
    if api::is_staff(w, actor) {
        return true;
    }
    let Some(store) = w.module_state::<EconomyStore>() else { return false };
    let Some(c) = store.snapshot().contract(id) else { return false };
    if c.consent.visibility.is_public() {
        return true;
    }
    let issuer = c.issuer.clone();
    let parties = c.consent.parties.clone();
    let audience = c.consent.visibility.members().to_vec();
    let scope = c.consent.scope;
    let me = PrincipalId::Object(actor);
    let auth = api::WorldAuth { w };
    if auth.acts_for(&me, &issuer) {
        return true;
    }
    if parties.iter().chain(audience.iter()).any(|p| auth.acts_for(&me, p)) {
        return true;
    }
    // **Every named attestor sees what they attest** — the adjudicator, and whoever
    // each `Attestation` condition names. Naming an authority in the consent surface
    // and then hiding the contract from it is the same shape the E4b1 fold fixed for
    // holds (A2): a right you cannot see is a right you cannot exercise, and here the
    // right is the one thing that can release the escrow.
    let attestors = c
        .consent
        .adjudicator
        .iter()
        .chain(c.consent.conditions.iter().filter_map(|cond| match cond {
            TypedCondition::Attestation { by } => Some(by),
            _ => None,
        }));
    if attestors.into_iter().any(|set| attestor_can_view(w, actor, set)) {
        return true;
    }
    // An org contract is visible to the people the org authorized to act on it —
    // without this an org member holding `contract.accept` could not read the
    // contract they are supposed to accept.
    scope.is_some_and(|s| {
        ["contract.draft", "contract.accept", "contract.attest", "contract.dispute",
         "economy.account.view"]
            .iter()
            .any(|cap| api::holds_capability(w, actor, cap, s))
    })
}

/// Whether `actor` is somebody the attestor set names — the view half of "the
/// adjudicator sees what they adjudicate". Staff are already through the gate above;
/// a canon set names no person at all.
fn attestor_can_view(w: &World, actor: ObjId, set: &AttestorSet) -> bool {
    match set {
        AttestorSet::Party(p) => {
            api::WorldAuth { w }.acts_for(&PrincipalId::Object(actor), p)
        }
        AttestorSet::Capability { scope, capability } => scope
            .is_some_and(|s| api::holds_capability(w, actor, capability.as_str(), s)),
        AttestorSet::Staff => api::is_staff(w, actor),
        AttestorSet::Canon { .. } => false,
    }
}

/// Fetch a contract the actor may see, or the nonexistent-id text.
fn contract_of(ctx: &Ctx<'_>, s: &str) -> Result<EconomicContract, CmdError> {
    let unknown = || CmdError::msg(NO_SUCH);
    let id = parse_contract_id(s).ok_or_else(unknown)?;
    if !may_view_contract(ctx.w, ctx.actor, id) {
        return Err(unknown());
    }
    let store = ctx.w.module_state::<EconomyStore>().ok_or_else(unknown)?;
    store.snapshot().contract(id).cloned().ok_or_else(unknown)
}

fn parse_contract_id(s: &str) -> Option<ContractId> {
    s.trim().trim_start_matches('#').parse::<u64>().ok().map(ContractId)
}

// ---- registration ---------------------------------------------------------

/// Register `@contract` (§5's verb list plus the four the E6 extensions need).
pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.command(
        CommandSpec {
            name: "@contract".into(),
            shape: ArgShape::EqPair,
            switches: [
                "draft", "party", "pay", "deliver", "split", "remainder", "condition", "deadline",
                "visibility", "offer", "accept", "attest", "dispute", "agree", "cancel", "resolve",
                "show", "list", "roster", "amend", "adjudicator", "evidence", "outcome", "role",
            ]
            .iter()
            .copied()
            .map(SmolStr::new)
            .collect(),
            help: "@contract/draft <name> [scope=<id>]        start a draft\n\
                   Builders (Draft only, or an Offered contract whose roster is open):\n\
                   @contract/party <id>=<principal>\n\
                   @contract/pay <id>=<from acct> <amount>    what that account escrows\n\
                   @contract/deliver <id>=title <obj> <from>-><to> | deposit <obj> <vault>\n\
                   \x20                    | collateral <obj> <from> | obligation <debit> <credit> <amt> every=<secs>\n\
                   @contract/split <id>=<acct> <exact|N‰> [<acct> <share> …]\n\
                   @contract/remainder <id>=<acct>\n\
                   @contract/condition <id>=attest <who> | deposit <obj> <vault> | title <obj> <who>\n\
                   \x20                    | fact <provider> <kind> <id> | before <secs|ic:secs> | after <n> <delay>\n\
                   \x20  <who> = party:<principal> | cap:<capability>[:<scope>] | staff | canon:<kind>[:<source>]\n\
                   @contract/deadline <id>=<secs|ic:secs> [refund|forfeit=<acct>|dispute]\n\
                   @contract/visibility <id>=parties|staff|public|audience <principal> …\n\
                   @contract/adjudicator <id>=<who>|none      who may attest (consented)\n\
                   @contract/evidence <id>=attest,deposit,title,fact,before,after|any\n\
                   @contract/outcome <id>=<name> when=<n,…> pay=<acct>:<share>,… [titles=<n,…>]\n\
                   @contract/role <id>=author|payer|johnson|facilitator <principal>|none\n\
                   @contract/roster <id>=lock|open            freeze (or re-open) the party list\n\
                   Lifecycle:\n\
                   @contract/offer <id>                       Draft → Offered; freezes the consent hash\n\
                   @contract/accept <id>                      a party; places their escrow in the same step\n\
                   @contract/amend <id>                       Offered → Draft: refunds and voids every acceptance\n\
                   @contract/attest <id>=<n>                  an attestor; may settle in the same step\n\
                   @contract/dispute <id>                     a party; freezes the escrow\n\
                   @contract/agree <id>=settle|refund         unanimous party agreement ends a dispute\n\
                   @contract/cancel <id>                      the issuer (Draft/Offered) or staff\n\
                   @contract/resolve <id>=settle|refund       staff\n\
                   @contract/show <id>   @contract/list\n\
                   Accepting is TWO rights, checked separately: the right to accept, and debit\n\
                   authority on the account your consideration names. Escrow placed here belongs\n\
                   to the contract — nobody can release or capture it by hand."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_contract),
    )?;
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn cmd_contract(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let sw = &ctx.switches;
    if sw.has("show") {
        return contract_show(ctx);
    }
    if sw.has("list") {
        return contract_list(ctx);
    }
    refuse_if_frozen(ctx)?;
    if sw.has("draft") {
        contract_draft(ctx)
    } else if sw.has("party") {
        build_party(ctx)
    } else if sw.has("pay") {
        build_pay(ctx)
    } else if sw.has("deliver") {
        build_deliver(ctx)
    } else if sw.has("split") {
        build_split(ctx)
    } else if sw.has("remainder") {
        build_remainder(ctx)
    } else if sw.has("condition") {
        build_condition(ctx)
    } else if sw.has("deadline") {
        build_deadline(ctx)
    } else if sw.has("visibility") {
        build_visibility(ctx)
    } else if sw.has("adjudicator") {
        build_adjudicator(ctx)
    } else if sw.has("evidence") {
        build_evidence(ctx)
    } else if sw.has("outcome") {
        build_outcome(ctx)
    } else if sw.has("role") {
        build_role(ctx)
    } else if sw.has("roster") {
        build_roster(ctx)
    } else if sw.has("offer") {
        contract_offer(ctx)
    } else if sw.has("accept") {
        contract_accept(ctx)
    } else if sw.has("amend") {
        contract_amend(ctx)
    } else if sw.has("attest") {
        contract_attest(ctx)
    } else if sw.has("dispute") {
        contract_dispute(ctx)
    } else if sw.has("agree") {
        contract_agree(ctx)
    } else if sw.has("cancel") {
        contract_cancel(ctx)
    } else if sw.has("resolve") {
        contract_resolve(ctx)
    } else {
        Err(CmdError::msg(
            "Usage: @contract/draft|party|pay|deliver|split|remainder|condition|deadline|\
             visibility|adjudicator|evidence|outcome|role|roster|offer|accept|amend|attest|\
             dispute|agree|cancel|resolve|show|list",
        ))
    }
}

// ---- who may act ----------------------------------------------------------

/// Whether `actor` holds a `contract.*` capability on the contract's own
/// compartment. An unscoped contract has no compartment, so the answer is `no` and
/// the party clause is the only way in.
fn holds_contract_capability(ctx: &Ctx<'_>, scope: Option<OrgScopeId>, capability: &str) -> bool {
    let Some(scope) = scope else { return false };
    api::holds_capability(ctx.w, ctx.actor, capability, scope)
}

/// Which party this actor speaks for on this contract, if any.
///
/// Two routes, and the order matters only for determinism: the actor **is** a party,
/// or the actor holds `capability` on the contract's compartment and one of the
/// parties is that compartment's organization. An org contract therefore lets the
/// people an org authorized act for it without anyone storing a delegation.
fn acting_party(
    ctx: &Ctx<'_>,
    c: &EconomicContract,
    capability: &str,
) -> Option<PrincipalId> {
    let me = PrincipalId::Object(ctx.actor);
    if c.consent.parties.contains(&me) {
        return Some(me);
    }
    if !holds_contract_capability(ctx, c.consent.scope, capability) {
        return None;
    }
    let scope = c.consent.scope?;
    let store = ctx.w.module_state::<mush_org::OrgStore>()?;
    let org = store.state().scope(scope)?.org;
    let wanted = PrincipalId::Organization(org);
    // Canonical order, so "which party did I accept for" is never iteration-dependent.
    let mut parties: Vec<&PrincipalId> = c.consent.parties.iter().collect();
    parties.sort_by(|a, b| principal_key(a).cmp(&principal_key(b)));
    parties.into_iter().find(|p| **p == wanted).cloned()
}

/// Whether the actor is the contract's issuer (or acts for it), or staff.
fn is_issuer(ctx: &Ctx<'_>, c: &EconomicContract) -> bool {
    if is_admin(ctx.w, ctx.actor) {
        return true;
    }
    api::WorldAuth { w: ctx.w }.acts_for(&PrincipalId::Object(ctx.actor), &c.issuer)
}

// ---- draft-time editing ---------------------------------------------------

/// Fetch a contract whose consent surface the actor may still edit.
///
/// `Draft` is always editable by the issuer. An `Offered` contract is editable only
/// while its **roster is open** and nobody has accepted yet — 08e answers 5/7: the
/// first acceptance freezes the roster snapshot and the exact shares, because a
/// runner who funded escrow agreed to a specific set of counterparties and a
/// specific split, and adding one afterwards dilutes what they signed. After that
/// the sanctioned route is `@contract/amend`, which refunds and re-collects.
fn editable(ctx: &Ctx<'_>, s: &str) -> Result<EconomicContract, CmdError> {
    let c = contract_of(ctx, s)?;
    if !is_issuer(ctx, &c) {
        return Err(CmdError::msg(NO_SUCH));
    }
    match c.state {
        ContractState::Draft => Ok(c),
        ContractState::Offered if !c.roster_locked && c.acceptances.is_empty() => Ok(c),
        ContractState::Offered => Err(CmdError::msg(format!(
            "Contract #{}'s roster is locked: {} party/parties have already accepted these exact \
             terms. Amend it (@contract/amend #{}), which refunds their escrow and re-collects \
             every acceptance.",
            c.id,
            c.acceptances.len(),
            c.id
        ))),
        _ => Err(CmdError::msg(format!("Contract #{} is no longer being drafted.", c.id))),
    }
}

/// Persist an edited contract, re-freezing its consent hash while it is still open
/// for editing so `consent_intact()` never reads a draft as tampered-with.
fn save_open(ctx: &mut Ctx<'_>, mut c: EconomicContract) -> ContractId {
    c.consent_hash = consent_hash(&c.issuer, &c.consent);
    c.revision = c.revision.saturating_add(1);
    let id = c.id;
    api::put_contract(ctx, c);
    ctx.gate_on_durable_ack();
    id
}

/// Persist a contract whose consent surface must NOT be re-hashed (anything from
/// `Offer` onwards: the hash is what the acceptances bind to).
fn save(ctx: &mut Ctx<'_>, mut c: EconomicContract) -> ContractId {
    c.revision = c.revision.saturating_add(1);
    let id = c.id;
    api::put_contract(ctx, c);
    id
}

// ---- @contract/draft ------------------------------------------------------

fn contract_draft(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: @contract/draft <name> [scope=<id>]";
    let raw = ctx.args.raw().to_string();
    let (rest, opts) = parse_opts(&raw, &["scope"]);
    let name = rest.trim().to_string();
    if name.is_empty() {
        return Err(CmdError::msg(USAGE));
    }
    if name.len() > MAX_CONTRACT_NAME_BYTES {
        return Err(CmdError::msg("That name is too long."));
    }
    let scope = match opts.get("scope") {
        None => None,
        Some(s) => Some(parse_scope(s).ok_or_else(|| CmdError::msg("No such scope."))?),
    };
    // An org contract is issued BY the organization, and drafting one needs
    // `contract.draft` on that compartment — the org's own authorization, not the
    // drafter's personal standing.
    let issuer = match scope {
        None => PrincipalId::Object(ctx.actor),
        Some(s) => {
            let org = ctx
                .w
                .module_state::<mush_org::OrgStore>()
                .and_then(|st| st.state().scope(s).map(|sc| sc.org))
                .ok_or_else(|| CmdError::msg("No such scope."))?;
            if !is_admin(ctx.w, ctx.actor)
                && !api::holds_capability(ctx.w, ctx.actor, "contract.draft", s)
            {
                return Err(CmdError::msg("No such scope."));
            }
            PrincipalId::Organization(org)
        }
    };
    // §9's cap, counted over NON-TERMINAL contracts: a terminal one costs nothing to
    // keep, and an issuer who cannot open a seventeenth draft can still read the
    // sixteen they have.
    let open = ctx.w.module_state::<EconomyStore>().map_or(0, |st| {
        st.snapshot()
            .contracts()
            .filter(|c| c.issuer == issuer && !c.state.is_terminal())
            .count()
    });
    if open >= MAX_OPEN_CONTRACTS_PER_ISSUER {
        return Err(CmdError::msg(format!(
            "That issuer already has {open} contracts open (the cap is \
             {MAX_OPEN_CONTRACTS_PER_ISSUER}). Settle, cancel or let some expire first."
        )));
    }
    let now = ctx.w.now();
    let id = {
        let store = ctx.w.module_state_mut::<EconomyStore>().ok_or_else(|| CmdError::msg("No economy."))?;
        store.alloc_contract().ok_or_else(|| {
            CmdError::msg("The contract-id allocator is exhausted; nothing was created.")
        })?
    };
    let consent = ConsentSurface {
        parties: vec![issuer.clone()],
        terms: ContractTerms {
            consideration: Vec::new(),
            deliverables: Vec::new(),
            split: Vec::new(),
            // Until `@contract/remainder` says otherwise, a floored remainder has
            // nowhere to go. `check_offerable` refuses the offer, which is the point:
            // a contract must say where every minor unit lands before it is signed.
            remainder_to: AccountId(u64::MAX),
            on_expiry: Expiry::Refund,
            outcomes: Vec::new(),
        },
        conditions: Vec::new(),
        deadline: None,
        visibility: EconomicVisibility::Parties,
        scope,
        adjudicator: None,
        evidence: Vec::new(),
    };
    let c = EconomicContract {
        id,
        issuer: issuer.clone(),
        name: SmolStr::new(&name),
        consent_hash: consent_hash(&issuer, &consent),
        consent,
        acceptances: Vec::new(),
        satisfied: Vec::new(),
        escrow: Vec::new(),
        state: ContractState::Draft,
        settle_attempts: 0,
        created_at: now,
        amendments: 0,
        disputed_at: None,
        roster_locked: false,
        deposits: Vec::new(),
        agreements: Vec::new(),
        roles: ContractRoles { author: Some(PrincipalId::Object(ctx.actor)), ..ContractRoles::default() },
        revision: 1,
    };
    api::put_contract(ctx, c);
    let meta = ctx.w.module_state::<EconomyStore>().map(EconomyStore::meta_doc);
    if let Some(meta) = meta {
        api::persist_meta(ctx.out, meta);
    }
    info(ctx, format!("Contract #{id} drafted: \"{name}\"."));
    info(
        ctx,
        format!(
            "Build it with @contract/party, /pay, /split, /remainder, /condition, then \
             @contract/offer #{id}. A draft is collected after {} days.",
            DRAFT_TTL_SECS / 86_400
        ),
    );
    ctx.gate_on_durable_ack();
    Ok(())
}

// ---- the consent-surface builders -----------------------------------------

fn build_party(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: @contract/party <id>=<principal>";
    let mut c = editable(ctx, ctx.args.lhs())?;
    let rhs = ctx.args.rhs().ok_or_else(|| CmdError::msg(USAGE))?.to_string();
    let who = parse_principal(ctx, &rhs).ok_or_else(|| CmdError::msg("No such principal."))?;
    if c.consent.parties.iter().any(|p| principal_key(p) == principal_key(&who)) {
        return Err(CmdError::msg("That principal is already a party."));
    }
    if c.consent.parties.len() >= MAX_PARTIES {
        return Err(CmdError::msg(format!("A contract may name at most {MAX_PARTIES} parties.")));
    }
    c.consent.parties.push(who.clone());
    let rendered = render_principal(ctx, &who);
    let id = save_open(ctx, c);
    info(ctx, format!("Contract #{id}: {rendered} is now a party."));
    Ok(())
}

fn build_pay(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: @contract/pay <id>=<from account> <amount>";
    let mut c = editable(ctx, ctx.args.lhs())?;
    let rhs = ctx.args.rhs().ok_or_else(|| CmdError::msg(USAGE))?.to_string();
    let mut words = rhs.split_whitespace();
    let account = words.next().and_then(parse_account).ok_or_else(|| CmdError::msg(USAGE))?;
    let amount = words.next().and_then(parse_amount).ok_or_else(|| CmdError::msg(USAGE))?;
    if amount.is_zero() || amount.is_negative() {
        return Err(CmdError::msg("A consideration must be positive."));
    }
    // The payer is whoever OWNS the account: the accountable escrow principal is a
    // fact about the wallet, never a claim the drafter makes (08e answer 4).
    let owner = {
        let store = ctx.w.module_state::<EconomyStore>().ok_or_else(|| CmdError::msg("No economy."))?;
        let a = store.snapshot().account(account).ok_or_else(|| CmdError::msg("No such account."))?;
        if a.status != AccountStatus::Active {
            return Err(CmdError::msg("No such account."));
        }
        a.owner.clone()
    };
    if c.consent.terms.consideration.len() >= MAX_SHARES {
        return Err(CmdError::msg("Too many considerations."));
    }
    c.consent.terms.consideration.retain(|x| x.account != account);
    c.consent.terms.consideration.push(Consideration { from: owner.clone(), account, amount });
    // The payer role defaults to the first consideration's owner, and stays whatever
    // `@contract/role` last said.
    if c.roles.payer.is_none() {
        c.roles.payer = Some(owner);
    }
    let id = save_open(ctx, c);
    info(ctx, format!("Contract #{id}: account #{account} escrows {} at acceptance.", amount.minor()));
    Ok(())
}

fn build_deliver(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: @contract/deliver <id>=title <obj> <from>-><to> | deposit <obj> \
                         <vault> | collateral <obj> <from> | obligation <debit> <credit> <amount> \
                         every=<secs>";
    let mut c = editable(ctx, ctx.args.lhs())?;
    let rhs = ctx.args.rhs().ok_or_else(|| CmdError::msg(USAGE))?.to_string();
    let (rest, opts) = parse_opts(&rhs, &["every", "grace"]);
    let mut words = rest.split_whitespace();
    let kind = words.next().unwrap_or("").to_ascii_lowercase();
    let deliverable = match kind.as_str() {
        "title" => {
            let object = asset_target(ctx, words.next().unwrap_or(""))?;
            let pair: String = words.collect::<Vec<_>>().join(" ");
            let (from, to) = split_arrow(&pair).ok_or_else(|| CmdError::msg(USAGE))?;
            let from = parse_principal(ctx, &from).ok_or_else(|| CmdError::msg("No such principal."))?;
            let to = parse_principal(ctx, &to).ok_or_else(|| CmdError::msg("No such principal."))?;
            Deliverable::TitleTransfer { object, from, to }
        }
        "deposit" => {
            let object = asset_target(ctx, words.next().unwrap_or(""))?;
            let vault = words
                .next()
                .and_then(parse_vault_id)
                .ok_or_else(|| CmdError::msg("No such vault."))?;
            Deliverable::DepositInto { object, vault }
        }
        "collateral" => {
            let object = asset_target(ctx, words.next().unwrap_or(""))?;
            let from = words
                .next()
                .and_then(|s| parse_principal(ctx, s))
                .ok_or_else(|| CmdError::msg("No such principal."))?;
            Deliverable::Collateral { object, from }
        }
        "obligation" => {
            let debit = words.next().and_then(parse_account).ok_or_else(|| CmdError::msg(USAGE))?;
            let credit = words.next().and_then(parse_account).ok_or_else(|| CmdError::msg(USAGE))?;
            let amount = words.next().and_then(parse_amount).ok_or_else(|| CmdError::msg(USAGE))?;
            if amount.is_zero() || amount.is_negative() {
                return Err(CmdError::msg("An obligation amount must be positive."));
            }
            let every = opts
                .get("every")
                .and_then(|s| s.parse::<i64>().ok())
                .filter(|n| *n > 0)
                .ok_or_else(|| CmdError::msg("every= must be a positive number of seconds."))?;
            let grace = opts.get("grace").and_then(|s| s.parse::<u8>().ok()).unwrap_or(crate::DEFAULT_GRACE_PERIODS);
            let (debtor, creditor) = {
                let store = ctx.w.module_state::<EconomyStore>().ok_or_else(|| CmdError::msg("No economy."))?;
                let snap = store.snapshot();
                let d = snap.account(debit).ok_or_else(|| CmdError::msg("No such account."))?;
                let cr = snap.account(credit).ok_or_else(|| CmdError::msg("No such account."))?;
                if matches!(d.owner, PrincipalId::System(_)) {
                    return Err(CmdError::msg(
                        "A system source/sink cannot be the debit side of an obligation: that would \
                         be a standing mint.",
                    ));
                }
                (d.owner.clone(), cr.owner.clone())
            };
            let now = ctx.w.now();
            Deliverable::Obligation {
                template: Box::new(ObligationTemplate {
                    debtor,
                    creditor,
                    debit_account: debit,
                    credit_account: credit,
                    amount,
                    schedule: EconomicSchedule::every(now.saturating_add(every), every),
                    grace: GracePolicy::new(grace),
                    notify_on_success: false,
                }),
            }
        }
        _ => return Err(CmdError::msg(USAGE)),
    };
    if c.consent.terms.deliverables.len() >= MAX_DELIVERABLES {
        return Err(CmdError::msg(format!("At most {MAX_DELIVERABLES} deliverables.")));
    }
    c.consent.terms.deliverables.push(deliverable);
    let n = c.consent.terms.deliverables.len() - 1;
    let id = save_open(ctx, c);
    info(ctx, format!("Contract #{id}: deliverable {n} recorded ({kind})."));
    Ok(())
}

/// Split `<a>-><b>` or `<a>→<b>` — the arrow §5 writes with, and the ASCII one a
/// terminal can actually type.
fn split_arrow(s: &str) -> Option<(String, String)> {
    for sep in ["->", "→"] {
        if let Some((a, b)) = s.split_once(sep) {
            return Some((a.trim().to_string(), b.trim().to_string()));
        }
    }
    None
}

/// `<exact>` (a bare number) or `<N>‰` / `<N>pm` (a permille).
fn parse_share(s: &str) -> Option<Share> {
    let t = s.trim();
    for suffix in ["‰", "pm", "permille"] {
        if let Some(n) = t.strip_suffix(suffix) {
            return n.trim().parse::<u32>().ok().filter(|p| *p <= PERMILLE).map(Share::Permille);
        }
    }
    parse_amount(t).filter(|a| !a.is_negative()).map(Share::Exact)
}

fn build_split(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: @contract/split <id>=<acct> <exact|N‰> [<acct> <share> …]";
    let mut c = editable(ctx, ctx.args.lhs())?;
    let rhs = ctx.args.rhs().ok_or_else(|| CmdError::msg(USAGE))?.to_string();
    let shares = parse_shares(&rhs).ok_or_else(|| CmdError::msg(USAGE))?;
    if shares.is_empty() || shares.len() > MAX_SHARES {
        return Err(CmdError::msg(USAGE));
    }
    c.consent.terms.split = shares;
    let n = c.consent.terms.split.len();
    let id = save_open(ctx, c);
    info(ctx, format!("Contract #{id}: the split now has {n} share(s)."));
    Ok(())
}

/// `<acct> <share> [<acct> <share> …]`, or the comma form `<acct>:<share>,…`.
fn parse_shares(s: &str) -> Option<Vec<PayoutShare>> {
    let mut out: Vec<PayoutShare> = Vec::new();
    if s.contains(':') {
        for part in s.split(',') {
            let (a, sh) = part.split_once(':')?;
            out.push(PayoutShare { to: parse_account(a)?, share: parse_share(sh)? });
        }
        return Some(out);
    }
    let words: Vec<&str> = s.split_whitespace().collect();
    if words.is_empty() || !words.len().is_multiple_of(2) {
        return None;
    }
    for pair in words.chunks(2) {
        let (a, sh) = (pair.first()?, pair.get(1)?);
        out.push(PayoutShare { to: parse_account(a)?, share: parse_share(sh)? });
    }
    Some(out)
}

fn build_remainder(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let mut c = editable(ctx, ctx.args.lhs())?;
    let rhs = ctx.args.rhs().unwrap_or("").to_string();
    let account = parse_account(&rhs).ok_or_else(|| CmdError::msg("No such account."))?;
    c.consent.terms.remainder_to = account;
    let id = save_open(ctx, c);
    info(ctx, format!("Contract #{id}: the floored remainder goes to account #{account}."));
    Ok(())
}

/// `party:<principal>` | `cap:<capability>[:<scope>]` | `staff` | `canon:<kind>[:<source>]`
fn parse_attestor(ctx: &Ctx<'_>, s: &str) -> Option<AttestorSet> {
    let t = s.trim();
    if t.eq_ignore_ascii_case("staff") {
        return Some(AttestorSet::Staff);
    }
    if let Some(rest) = t.strip_prefix("party:") {
        return parse_principal(ctx, rest).map(AttestorSet::Party);
    }
    if let Some(rest) = t.strip_prefix("cap:") {
        let mut parts = rest.splitn(2, ':');
        let capability = CapabilityId::new(parts.next()?)?;
        let scope = parts.next().and_then(parse_scope);
        return Some(AttestorSet::Capability { scope, capability });
    }
    if let Some(rest) = t.strip_prefix("canon:") {
        let mut parts = rest.splitn(2, ':');
        let kind = SmolStr::new(parts.next()?);
        let source = parts.next().map(SmolStr::new);
        return Some(AttestorSet::Canon { kind, source });
    }
    None
}

fn build_condition(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: @contract/condition <id>=attest <who> | deposit <obj> <vault> | \
                         title <obj> <who> | fact <provider> <kind> <id> | before <secs|ic:secs> | \
                         after <n> <delay>";
    let mut c = editable(ctx, ctx.args.lhs())?;
    let rhs = ctx.args.rhs().ok_or_else(|| CmdError::msg(USAGE))?.to_string();
    let mut words = rhs.split_whitespace();
    let kind = words.next().unwrap_or("").to_ascii_lowercase();
    let condition = match kind.as_str() {
        "attest" => {
            let spec: String = words.collect::<Vec<_>>().join(" ");
            TypedCondition::Attestation {
                by: parse_attestor(ctx, &spec).ok_or_else(|| CmdError::msg(USAGE))?,
            }
        }
        "deposit" => {
            let object = asset_target(ctx, words.next().unwrap_or(""))?;
            let vault = words
                .next()
                .and_then(parse_vault_id)
                .ok_or_else(|| CmdError::msg("No such vault."))?;
            TypedCondition::DepositInto { object, vault }
        }
        "title" => {
            let object = asset_target(ctx, words.next().unwrap_or(""))?;
            let by = words
                .next()
                .and_then(|s| parse_principal(ctx, s))
                .ok_or_else(|| CmdError::msg("No such principal."))?;
            TypedCondition::TitleHeld { object, by }
        }
        "fact" => {
            let provider = words.next().ok_or_else(|| CmdError::msg(USAGE))?.to_string();
            let fkind = words.next().ok_or_else(|| CmdError::msg(USAGE))?.to_string();
            let fid = words.next().ok_or_else(|| CmdError::msg(USAGE))?.to_string();
            TypedCondition::Fact {
                fact: ExternalRef {
                    provider: SmolStr::new(provider),
                    kind: SmolStr::new(fkind),
                    id: SmolStr::new(fid),
                    revision: None,
                },
            }
        }
        "before" => {
            let d = words
                .next()
                .and_then(|s| parse_deadline(ctx, s))
                .ok_or_else(|| CmdError::msg("Bad deadline (a POSITIVE number of seconds, or ic:<seconds>)."))?;
            TypedCondition::Before { deadline: d }
        }
        "after" => {
            let anchor = words.next().and_then(|s| s.parse::<usize>().ok()).ok_or_else(|| CmdError::msg(USAGE))?;
            let raw = words.next().unwrap_or("");
            let (clock, secs) = match raw.strip_prefix("ic:") {
                Some(rest) => (EconomicClock::WorldIc, rest),
                None => (EconomicClock::RealUtc, raw),
            };
            let delay = secs.parse::<i64>().ok().filter(|n| *n > 0).ok_or_else(|| CmdError::msg(USAGE))?;
            TypedCondition::After { condition: anchor, delay, clock }
        }
        _ => return Err(CmdError::msg(USAGE)),
    };
    if c.consent.conditions.len() >= MAX_CONDITIONS {
        return Err(CmdError::msg(format!("At most {MAX_CONDITIONS} conditions.")));
    }
    c.consent.conditions.push(condition);
    // `satisfied` is parallel to `conditions` by construction, everywhere.
    c.satisfied.push(None);
    let n = c.consent.conditions.len() - 1;
    let id = save_open(ctx, c);
    info(ctx, format!("Contract #{id}: condition {n} recorded ({kind})."));
    Ok(())
}

fn build_deadline(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str =
        "Usage: @contract/deadline <id>=<secs|ic:secs> [refund|forfeit=<acct>|dispute]";
    let mut c = editable(ctx, ctx.args.lhs())?;
    let rhs = ctx.args.rhs().ok_or_else(|| CmdError::msg(USAGE))?.to_string();
    let (rest, opts) = parse_opts(&rhs, &["forfeit"]);
    let mut words = rest.split_whitespace();
    let deadline: EconomicDeadline = words
        .next()
        .and_then(|s| parse_deadline(ctx, s))
        .ok_or_else(|| CmdError::msg("Bad deadline (a POSITIVE number of seconds, or ic:<seconds>)."))?;
    let on_expiry = match (words.next().map(str::to_ascii_lowercase).as_deref(), opts.get("forfeit")) {
        (_, Some(acct)) => {
            let to = parse_account(acct).ok_or_else(|| CmdError::msg("No such account."))?;
            Expiry::Forfeit { to }
        }
        (Some("dispute"), _) => Expiry::Dispute,
        (Some("refund") | None, _) => Expiry::Refund,
        (Some(_), _) => return Err(CmdError::msg(USAGE)),
    };
    c.consent.deadline = Some(deadline);
    c.consent.terms.on_expiry = on_expiry;
    let id = save_open(ctx, c);
    info(ctx, format!("Contract #{id}: it lapses at {} on {:?}.", deadline.at, deadline.clock));
    Ok(())
}

fn build_visibility(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: @contract/visibility <id>=parties|staff|public|audience <principal> …";
    let mut c = editable(ctx, ctx.args.lhs())?;
    let rhs = ctx.args.rhs().ok_or_else(|| CmdError::msg(USAGE))?.to_string();
    let mut words = rhs.split_whitespace();
    let word = words.next().unwrap_or("").to_ascii_lowercase();
    let visibility = match word.as_str() {
        "parties" => EconomicVisibility::Parties,
        "staff" => EconomicVisibility::Staff,
        "public" => EconomicVisibility::Public,
        "audience" => {
            let mut members: Vec<PrincipalId> = Vec::new();
            for w in words {
                let p = parse_principal(ctx, w).ok_or_else(|| CmdError::msg("No such principal."))?;
                if members.iter().all(|m| principal_key(m) != principal_key(&p)) {
                    members.push(p);
                }
            }
            if members.is_empty() || members.len() > MAX_AUDIENCE {
                return Err(CmdError::msg(format!(
                    "An audience names 1-{MAX_AUDIENCE} principals besides the parties."
                )));
            }
            EconomicVisibility::Audience { members }
        }
        _ => return Err(CmdError::msg(USAGE)),
    };
    let label = visibility.label();
    c.consent.visibility = visibility;
    let id = save_open(ctx, c);
    info(ctx, format!("Contract #{id}: visibility is now {label}."));
    if label == "public" {
        info(
            ctx,
            "Every party has to accept that disclosure: it is inside the consent surface, and a \
             public contract is the only kind that ever reaches canon."
                .to_string(),
        );
    }
    Ok(())
}

fn build_adjudicator(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: @contract/adjudicator <id>=<who>|none";
    let mut c = editable(ctx, ctx.args.lhs())?;
    let rhs = ctx.args.rhs().ok_or_else(|| CmdError::msg(USAGE))?.trim().to_string();
    c.consent.adjudicator = if rhs.eq_ignore_ascii_case("none") {
        None
    } else {
        Some(parse_attestor(ctx, &rhs).ok_or_else(|| CmdError::msg(USAGE))?)
    };
    let named = c.consent.adjudicator.is_some();
    let id = save_open(ctx, c);
    if named {
        info(ctx, format!("Contract #{id}: the adjudicator is named, and only they may attest."));
    } else {
        info(ctx, format!("Contract #{id}: no adjudicator; each condition's own attestor set applies."));
    }
    Ok(())
}

fn build_evidence(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str =
        "Usage: @contract/evidence <id>=attest,deposit,title,fact,before,after   (or: any)";
    let mut c = editable(ctx, ctx.args.lhs())?;
    let rhs = ctx.args.rhs().ok_or_else(|| CmdError::msg(USAGE))?.trim().to_ascii_lowercase();
    let mut kinds: Vec<EvidenceKind> = Vec::new();
    if rhs != "any" {
        for word in rhs.split([',', ' ']).filter(|w| !w.is_empty()) {
            let k = match word {
                "attest" | "attestation" => EvidenceKind::Attestation,
                "deposit" => EvidenceKind::DepositInto,
                "title" => EvidenceKind::TitleHeld,
                "fact" => EvidenceKind::Fact,
                "before" => EvidenceKind::Before,
                "after" => EvidenceKind::After,
                _ => return Err(CmdError::msg(USAGE)),
            };
            if !kinds.contains(&k) {
                kinds.push(k);
            }
        }
        if kinds.is_empty() {
            return Err(CmdError::msg(USAGE));
        }
    }
    c.consent.evidence = kinds;
    let n = c.consent.evidence.len();
    let id = save_open(ctx, c);
    if n == 0 {
        info(ctx, format!("Contract #{id}: any kind of evidence is permitted."));
    } else {
        info(ctx, format!("Contract #{id}: {n} permitted kind(s) of evidence; anything else is refused."));
    }
    Ok(())
}

fn build_outcome(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: @contract/outcome <id>=<name> when=<n[,n]> pay=<acct>:<share>[,…] \
                         [titles=<n[,n]>] [expense=none|retained|returned|reimbursed]";
    let mut c = editable(ctx, ctx.args.lhs())?;
    let rhs = ctx.args.rhs().ok_or_else(|| CmdError::msg(USAGE))?.to_string();
    let (rest, opts) = parse_opts(&rhs, &["when", "pay", "titles", "expense"]);
    let name = rest.trim().to_string();
    if name.is_empty() || name.len() > MAX_CONTRACT_NAME_BYTES {
        return Err(CmdError::msg(USAGE));
    }
    let when = parse_indexes(opts.get("when").map(String::as_str).unwrap_or(""))
        .ok_or_else(|| CmdError::msg(USAGE))?;
    let payouts = parse_shares(opts.get("pay").map(String::as_str).unwrap_or(""))
        .ok_or_else(|| CmdError::msg(USAGE))?;
    let asset_consequences = match opts.get("titles") {
        None => Vec::new(),
        Some(s) => parse_indexes(s).ok_or_else(|| CmdError::msg(USAGE))?,
    };
    let expense_treatment = match opts.get("expense").map(String::as_str) {
        None | Some("none") => crate::contract::ExpenseTreatment::None,
        Some("retained") => crate::contract::ExpenseTreatment::Retained,
        Some("returned") => crate::contract::ExpenseTreatment::Returned,
        Some("reimbursed") => crate::contract::ExpenseTreatment::Reimbursed,
        Some(_) => return Err(CmdError::msg(USAGE)),
    };
    if c.consent.terms.outcomes.len() >= MAX_OUTCOMES {
        return Err(CmdError::msg(format!("At most {MAX_OUTCOMES} outcomes.")));
    }
    c.consent.terms.outcomes.retain(|o| o.name != name);
    c.consent.terms.outcomes.push(OutcomeTerms {
        name: SmolStr::new(&name),
        when,
        payouts,
        expense_treatment,
        asset_consequences,
    });
    let n = c.consent.terms.outcomes.len();
    let id = save_open(ctx, c);
    info(ctx, format!("Contract #{id}: outcome \"{name}\" recorded ({n} in the table)."));
    info(
        ctx,
        "An outcome's shares must sum to the escrowed total exactly — there is no implicit \
         remainder — and exactly one outcome may be satisfied at settlement."
            .to_string(),
    );
    Ok(())
}

fn parse_indexes(s: &str) -> Option<Vec<usize>> {
    let mut out = Vec::new();
    for w in s.split([',', ' ']).filter(|w| !w.is_empty()) {
        let n = w.trim().parse::<usize>().ok()?;
        if n > MAX_CONDITIONS {
            return None;
        }
        if !out.contains(&n) {
            out.push(n);
        }
    }
    Some(out)
}

fn build_role(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str =
        "Usage: @contract/role <id>=author|payer|johnson|facilitator <principal>|none";
    let mut c = editable(ctx, ctx.args.lhs())?;
    let rhs = ctx.args.rhs().ok_or_else(|| CmdError::msg(USAGE))?.to_string();
    let mut words = rhs.split_whitespace();
    let role = words.next().unwrap_or("").to_ascii_lowercase();
    let rest: String = words.collect::<Vec<_>>().join(" ");
    let who = if rest.trim().eq_ignore_ascii_case("none") || rest.trim().is_empty() {
        None
    } else {
        Some(parse_principal(ctx, &rest).ok_or_else(|| CmdError::msg("No such principal."))?)
    };
    match role.as_str() {
        "author" => c.roles.author = who,
        "payer" => c.roles.payer = who,
        "johnson" | "representative" => c.roles.representative = who,
        "facilitator" => c.roles.facilitator = who,
        _ => return Err(CmdError::msg(USAGE)),
    }
    let id = save_open(ctx, c);
    info(ctx, format!("Contract #{id}: the {role} role is recorded. Roles are attribution, never authority."));
    Ok(())
}

fn build_roster(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: @contract/roster <id>=lock|open";
    let c = contract_of(ctx, ctx.args.lhs())?;
    if !is_issuer(ctx, &c) {
        return Err(CmdError::msg(NO_SUCH));
    }
    let rhs = ctx.args.rhs().unwrap_or("").trim().to_ascii_lowercase();
    let mut c = c;
    match rhs.as_str() {
        "lock" => c.roster_locked = true,
        "open" => {
            if !c.acceptances.is_empty() {
                return Err(CmdError::msg(format!(
                    "Contract #{}'s roster cannot re-open: {} party/parties have accepted it. \
                     @contract/amend #{} refunds their escrow and re-collects every acceptance.",
                    c.id,
                    c.acceptances.len(),
                    c.id
                )));
            }
            c.roster_locked = false;
        }
        _ => return Err(CmdError::msg(USAGE)),
    }
    let locked = c.roster_locked;
    let id = save(ctx, c);
    info(ctx, format!("Contract #{id}: the roster is {}.", if locked { "locked" } else { "open" }));
    ctx.gate_on_durable_ack();
    Ok(())
}

// ---- @contract/offer ------------------------------------------------------

fn contract_offer(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let mut c = contract_of(ctx, ctx.args.lhs())?;
    if !is_issuer(ctx, &c) {
        return Err(CmdError::msg(NO_SUCH));
    }
    if c.state != ContractState::Draft {
        return Err(CmdError::msg(format!("Contract #{} is not a draft.", c.id)));
    }
    // The satisfaction vector is parallel to the conditions, always.
    c.satisfied.resize(c.consent.conditions.len(), None);
    {
        let store = ctx.w.module_state::<EconomyStore>().ok_or_else(|| CmdError::msg("No economy."))?;
        check_offerable(&c, store.snapshot()).map_err(|e| contract_refusal(&c, &e))?;
        // §9's per-party cap, counted over the parties this offer would bind.
        for p in &c.consent.parties {
            let live = store
                .snapshot()
                .contracts()
                .filter(|x| x.id != c.id && x.state.is_live())
                .filter(|x| x.consent.parties.contains(p))
                .count();
            if live >= MAX_LIVE_CONTRACTS_PER_PARTY {
                return Err(CmdError::msg(format!(
                    "A party is already on {live} live contracts (the cap is \
                     {MAX_LIVE_CONTRACTS_PER_PARTY})."
                )));
            }
        }
    }
    // Freeze the surface. From here on `save` never re-hashes: the hash IS what the
    // acceptances bind to, and a mutation after Offer must show up as a mismatch.
    c.consent_hash = consent_hash(&c.issuer, &c.consent);
    c.state = ContractState::Offered;
    // §5's builder block is "Draft only". The E6 extension (08e answers 5/7) that
    // lets an `Offered` contract still carry an unfrozen party is therefore an
    // explicit act — `@contract/roster <id>=open` — and never the default, so
    // "the surface changed after Offer" stays the refusal it has to be.
    c.roster_locked = true;
    let (id, parties) = (c.id, c.consent.parties.clone());
    save(ctx, c);
    info(ctx, format!("Contract #{id} is offered; its consent surface is frozen."));
    for p in &parties {
        notify(ctx, p, &format!("Contract #{id} has been offered to you (@contract/show #{id})."));
    }
    ctx.gate_on_durable_ack();
    Ok(())
}

/// Render a [`ContractError`] behind the existence-hiding rule: a viewer sees the
/// real reason, everyone else sees the nonexistent-contract text. The caller has
/// already proved viewability for every path that reaches this, so it is the message
/// that varies, never the disclosure.
fn contract_refusal(c: &EconomicContract, e: &ContractError) -> CmdError {
    CmdError::msg(format!("Contract #{}: {e}", c.id))
}

// ---- @contract/accept -----------------------------------------------------

/// `@contract/accept <id>` — a party, under **their own live authority**.
///
/// The step does two independent things, and D-E4-4 requires them to be independent:
/// it records an acceptance bound to the exact consent hash, and it places that
/// party's consideration as an escrow hold through the ordinary `post` seam. The
/// hold is a debit-authority plan (invariant 25), so the account's own rules apply —
/// including its [`crate::ApprovalRule`], which returns `NeedsApproval` and records
/// **no acceptance at all** (D-E4-11).
fn contract_accept(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let c = contract_of(ctx, ctx.args.lhs())?;
    if c.state != ContractState::Offered {
        return Err(CmdError::msg(format!("Contract #{} is not open for acceptance.", c.id)));
    }
    if !c.consent_intact() {
        return Err(CmdError::msg(format!(
            "Contract #{}'s consent surface has changed since it was offered; it must be re-offered.",
            c.id
        )));
    }
    let party = acting_party(ctx, &c, "contract.accept")
        .ok_or_else(|| CmdError::msg(NO_SUCH))?;
    if c.accepted_by(&party) {
        return Err(CmdError::msg(format!("Contract #{} is already accepted on your behalf.", c.id)));
    }
    // Everything this party owes, escrowed in one plan.
    let considerations: Vec<Consideration> = c
        .consent
        .terms
        .consideration
        .iter()
        .filter(|x| principal_key(&x.from) == principal_key(&party))
        .cloned()
        .collect();
    let placed: Vec<HoldId> = if considerations.is_empty() {
        Vec::new()
    } else {
        let mut plan = EconomyPlan::new(
            cmd_key(ctx),
            PrincipalId::Object(ctx.actor),
            EconomicCause::Command { serial: 0 },
            Vec::new(),
        );
        plan.visibility = c.consent.visibility.clone();
        plan.description = format!("escrow for contract {}", c.id);
        plan.hold_ops = considerations
            .iter()
            .map(|x| HoldOp::Place {
                account: x.account,
                amount: x.amount,
                // A beneficiary NOBODY acts for (`WorldAuth::acts_for` refuses every
                // `System` principal, staff included), so this escrow is capturable
                // and releasable only through `ContractAuth`.
                beneficiary: Some(api::contract_principal()),
                purpose: HoldPurpose::Escrow { contract: c.id },
                // No deadline: the CONTRACT's deadline governs, and a hold that aged
                // out on its own would break an agreement that was still standing.
                expires: None,
            })
            .collect();
        let first = considerations.first().map_or(AccountId(0), |x| x.account);
        match api::post(ctx, &plan) {
            Ok(PostOutcome::Posted(txid)) => ctx
                .w
                .module_state::<EconomyStore>()
                .and_then(|s| s.transaction(txid).cloned())
                .map(|t| {
                    t.hold_effects
                        .iter()
                        .filter(|e| e.kind == HoldEffectKind::Placed)
                        .map(|e| e.hold.id)
                        .collect::<Vec<HoldId>>()
                })
                .unwrap_or_default(),
            Ok(PostOutcome::AlreadyPosted(_)) => {
                return Err(CmdError::msg("That escrow was already placed; try again."));
            }
            // D-E4-11: no acceptance is recorded, the contract stays `Offered`, and
            // the party re-runs accept once the approval is collected.
            Ok(PostOutcome::NeedsApproval(request)) => {
                info(
                    ctx,
                    format!(
                        "Approval required before you can escrow that: request #{request}. Nothing \
                         was accepted — run @contract/accept #{} again once it is approved.",
                        c.id
                    ),
                );
                return Ok(());
            }
            Err(e) => {
                return Err(refuse(
                    ctx,
                    NamedResource::Contract(c.id),
                    &[NamedResource::Account(first)],
                    &e,
                ));
            }
        }
    };
    // The money is durable (or there was none to move); record the acceptance.
    let now = ctx.w.now();
    let mut c = c;
    c.acceptances.push(Acceptance { party: party.clone(), at: now, consent_hash: c.consent_hash });
    c.escrow.extend(placed.iter().copied());
    // 08e answers 5/7: the first acceptance freezes the roster snapshot and the
    // exact shares. From here a substitution is an amendment.
    c.roster_locked = true;
    let id = c.id;
    let full = c.fully_accepted();
    let escrowed: Vec<String> = placed.iter().map(|h| format!("#{h}")).collect();
    save(ctx, c);
    if escrowed.is_empty() {
        info(ctx, format!("Contract #{id}: accepted."));
    } else {
        info(ctx, format!("Contract #{id}: accepted; escrow {} placed.", escrowed.join(", ")));
        info(
            ctx,
            "That reservation belongs to the contract now: it is released by its refund and \
             captured by its settlement, and by nothing else."
                .to_string(),
        );
    }
    if full {
        activate(ctx, id);
    }
    ctx.gate_on_durable_ack();
    Ok(())
}

// ---- activation -----------------------------------------------------------

/// Every party has accepted: bring the contract into force (§5).
///
/// Three things happen, in this order and for this reason:
///
/// 1. the state moves to `Active`, which is what makes the `DepositInto` custody lock
///    real and what makes `ContractAuth` answer at all;
/// 2. the deposits already sitting in the named vaults are recorded with **the
///    depositor the journal names**, so a later refund can put them back;
/// 3. the pledges are encumbered and the `Obligation` deliverables are created.
///
/// Step 3 is a second post, and it can fail (a frozen asset, an encumbrance cap, an
/// exhausted allocator). It must not be allowed to undo step 1 — the escrow is
/// already placed and durable — so a failure here **disputes** the contract and pages
/// a human rather than returning an error that would discard the deferred writes of
/// a post that has already happened.
pub(crate) fn activate(ctx: &mut Ctx<'_>, id: ContractId) {
    let Some(mut c) = live_contract(ctx, id) else { return };
    if c.state != ContractState::Offered || !c.fully_accepted() {
        return;
    }
    c.state = ContractState::Active;
    c.deposits = record_deposits(ctx, &c);
    save(ctx, c.clone());
    notify_parties(ctx, &c, &format!("Contract #{id} is active: every party has accepted."));

    let ops = crate::contract::activation_asset_ops(&c);
    if !ops.is_empty() {
        let mut plan = EconomyPlan::new(
            activate_key(&c),
            c.issuer.clone(),
            EconomicCause::Internal { kind: SmolStr::new_static("contract"), id: id.0 },
            Vec::new(),
        );
        plan.description = format!("activation of contract {id}");
        plan.visibility = c.consent.visibility.clone();
        plan.asset_ops = ops;
        let owned = c.clone();
        let outcome = {
            let auth = {
                let Some(store) = ctx.w.module_state::<EconomyStore>() else { return };
                ContractAuth::settling(&owned, store.snapshot(), None)
            };
            api::post_with(ctx, &plan, PostAuth::Injected(&auth))
        };
        match outcome {
            Ok(PostOutcome::Posted(_) | PostOutcome::AlreadyPosted(_)) => {}
            Ok(PostOutcome::NeedsApproval(_)) | Err(_) => {
                let detail = match outcome {
                    Err(e) => e.to_string(),
                    _ => "the activation asked for an approval nothing can collect".to_string(),
                };
                tracing::error!(contract = id.0, %detail, "economy: a contract could not place its pledges");
                dispute_with(
                    ctx,
                    id,
                    format!(
                        "contract #{id} could not place the claims its terms pledge: {detail}. Its \
                         escrow is frozen until a human decides"
                    ),
                );
                return;
            }
        }
    }
    create_lease_obligations(ctx, id);
}

/// Record what is already deposited into this contract's named vaults, and who put
/// it there — from the **journal's** last-custody map, never from a claim (§5).
fn record_deposits(ctx: &Ctx<'_>, c: &EconomicContract) -> Vec<DepositRecord> {
    let Some(store) = ctx.w.module_state::<EconomyStore>() else { return Vec::new() };
    let mut out: Vec<DepositRecord> = Vec::new();
    for condition in &c.consent.conditions {
        let TypedCondition::DepositInto { object, vault } = condition else { continue };
        let Some(mark) = store.last_custody(*object) else { continue };
        if mark.vault != *vault || mark.direction != crate::record::CustodyDirection::Deposit {
            continue;
        }
        if out.len() >= MAX_DEPOSITS {
            break;
        }
        out.push(DepositRecord {
            object: *object,
            vault: *vault,
            deposited_by: mark.by.clone(),
            at: mark.at,
        });
    }
    out
}

/// Create the obligations this contract's `Obligation` deliverables describe, with
/// `cause: Contract(id)` — so E4b1's boot already suspends them when the contract
/// stops being live, and this cut's unwind cancels them outright.
fn create_lease_obligations(ctx: &mut Ctx<'_>, id: ContractId) {
    let Some(c) = live_contract(ctx, id) else { return };
    let templates: Vec<ObligationTemplate> = c
        .consent
        .terms
        .deliverables
        .iter()
        .filter_map(|d| match d {
            Deliverable::Obligation { template } => Some((**template).clone()),
            _ => None,
        })
        .collect();
    for t in templates {
        // Already created (a re-run after a restart)? The cause is the key.
        let exists = ctx.w.module_state::<EconomyStore>().is_some_and(|s| {
            s.snapshot().obligations().any(|o| {
                o.cause == ObligationCause::Contract(id)
                    && o.debit_account == t.debit_account
                    && o.credit_account == t.credit_account
            })
        });
        if exists {
            continue;
        }
        let minted = ctx.w.module_state_mut::<EconomyStore>().and_then(EconomyStore::alloc_obligation);
        let Some(oid) = minted else {
            tracing::error!(contract = id.0, "economy: the obligation allocator is exhausted; a lease has no rent record");
            continue;
        };
        let o = Obligation {
            id: oid,
            debtor: t.debtor.clone(),
            creditor: t.creditor.clone(),
            debit_account: t.debit_account,
            credit_account: t.credit_account,
            amount: t.amount,
            schedule: t.schedule,
            grace: t.grace,
            notify_on_success: t.notify_on_success,
            status: ObligationStatus::Active,
            cause: ObligationCause::Contract(id),
            minted_by: None,
            periods_settled: 0,
            history: Vec::new(),
            settled_total: 0,
            missed_total: 0,
            forgiven_total: 0,
            revision: 1,
        };
        api::put_obligation(ctx, o);
        let meta = ctx.w.module_state::<EconomyStore>().map(EconomyStore::meta_doc);
        if let Some(meta) = meta {
            api::persist_meta(ctx.out, meta);
        }
    }
}

/// Cancel every obligation this contract created (a lease's rent stops when the
/// lease does).
fn cancel_lease_obligations(ctx: &mut Ctx<'_>, id: ContractId) {
    let doomed: Vec<Obligation> = ctx
        .w
        .module_state::<EconomyStore>()
        .map(|s| {
            s.snapshot()
                .obligations()
                .filter(|o| o.cause == ObligationCause::Contract(id))
                .filter(|o| {
                    !matches!(o.status, ObligationStatus::Cancelled | ObligationStatus::Satisfied)
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    for mut o in doomed {
        o.status = ObligationStatus::Cancelled;
        o.revision = o.revision.saturating_add(1);
        api::put_obligation(ctx, o);
    }
}

fn live_contract(ctx: &Ctx<'_>, id: ContractId) -> Option<EconomicContract> {
    ctx.w.module_state::<EconomyStore>()?.snapshot().contract(id).cloned()
}

// ---- @contract/attest -----------------------------------------------------

fn contract_attest(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: @contract/attest <id>=<condition number>";
    let c = contract_of(ctx, ctx.args.lhs())?;
    // `Disputed` is deliberately included: evidence often arrives *because* somebody
    // disputed, and recording it is what lets `@contract/agree`/`resolve` settle
    // afterwards. Attesting a disputed contract records the attestation and settles
    // nothing — `promote_and_settle` only ever moves an `Active`/`ReadyToSettle` one,
    // so a disputed contract's escrow stays frozen exactly as §5 requires.
    if !matches!(
        c.state,
        ContractState::Active | ContractState::ReadyToSettle | ContractState::Disputed
    ) {
        return Err(CmdError::msg(format!("Contract #{} is not live.", c.id)));
    }
    let n: usize = ctx
        .args
        .rhs()
        .and_then(|s| s.trim().parse().ok())
        .ok_or_else(|| CmdError::msg(USAGE))?;
    let Some(TypedCondition::Attestation { by }) = c.consent.conditions.get(n).cloned() else {
        return Err(CmdError::msg(format!("Contract #{} has no attestable condition {n}.", c.id)));
    };
    let me = PrincipalId::Object(ctx.actor);
    let auth = api::WorldAuth { w: ctx.w };
    let qualifies = |set: &AttestorSet| -> bool {
        use crate::engine::AuthEval;
        auth.authorize_attestor(&me, set)
    };
    // 08e answer 6: when the surface NAMES an adjudicator, that authority **replaces**
    // each condition's own attestor set. Runners consent to the adjudicator when they
    // accept, and the whole point of consenting to an authority is that an interested
    // party asserting the same fact is not the same thing. `condition_holds` honours
    // it symmetrically, so the adjudicator's word re-verifies at settlement.
    match c.consent.adjudicator.clone() {
        Some(adjudicator) => {
            if !qualifies(&adjudicator) {
                return Err(CmdError::msg(format!(
                    "Contract #{} names an adjudicator, and only they may attest its conditions.",
                    c.id
                )));
            }
        }
        None => {
            if !qualifies(&by) {
                return Err(CmdError::msg(format!("Condition {n} is not yours to attest.")));
            }
        }
    }
    let now = ctx.w.now();
    let mut c = c;
    if c.satisfied.len() < c.consent.conditions.len() {
        c.satisfied.resize(c.consent.conditions.len(), None);
    }
    if let Some(slot) = c.satisfied.get_mut(n) {
        *slot = Some(Attestation {
            by: AttestorRef::Principal(me),
            at: now,
            clock: EconomicClock::RealUtc,
            source: SmolStr::new(format!("actor:{}", ctx.actor.0)),
        });
    }
    let id = c.id;
    save(ctx, c);
    info(ctx, format!("Contract #{id}: condition {n} attested."));
    // §3.5: the step that satisfies the last condition settles in the same step if
    // every re-verification passes.
    promote_and_settle(ctx, id, true);
    ctx.gate_on_durable_ack();
    Ok(())
}

// ---- settlement -----------------------------------------------------------

/// What one settlement attempt did.
pub(crate) enum Settled {
    /// Posted (or already posted) as this transaction.
    Posted(#[allow(dead_code)] crate::ids::TransactionId),
    /// Re-verification (or the plan) refused; nothing moved.
    Refused(String),
    /// Nothing to do: the contract is not ready, or the outcome table says nothing
    /// happened yet.
    NotReady,
}

/// Move a live contract to `ReadyToSettle` if its conditions now hold, and settle it
/// if `settle_now`.
pub(crate) fn promote_and_settle(ctx: &mut Ctx<'_>, id: ContractId, settle_now: bool) -> Settled {
    let Some(c) = live_contract(ctx, id) else { return Settled::NotReady };
    if !matches!(c.state, ContractState::Active | ContractState::ReadyToSettle) {
        return Settled::NotReady;
    }
    let verdict = {
        let Some(store) = ctx.w.module_state::<EconomyStore>() else { return Settled::NotReady };
        let providers = store.providers();
        let custody = WorldCustody { w: ctx.w, providers };
        let world = api::WorldAuth { w: ctx.w };
        let env = ValidateEnv {
            auth: &world,
            custody: &custody,
            facts: &NoFacts,
            now: ctx.w.now(),
            ic_now: ctx.w.game_now_unix(),
            next_hold_id: store.peek_next_hold(),
        };
        readiness(&c, store.snapshot(), &env)
    };
    match verdict {
        Readiness::NotYet => return Settled::NotReady,
        // A contract that cannot settle at all as it stands is a decision, not a wait
        // (§3.5: an ambiguous outcome is `Disputed`). Retrying it every thirty seconds
        // would freeze the escrow indefinitely and tell nobody.
        Readiness::Undecidable(e) => return note_settle_failure(ctx, id, &e.to_string(), true),
        Readiness::Ready(_) => {}
    }
    if c.state == ContractState::Active {
        let mut next = c.clone();
        next.state = ContractState::ReadyToSettle;
        save(ctx, next);
    }
    if !settle_now {
        return Settled::NotReady;
    }
    settle(ctx, id)
}

/// Attempt one settlement of a `ReadyToSettle` (or `Active`) contract.
///
/// The plan is built against the live world and re-verifies every gating condition on
/// the way (E4a verify fold, finding 6), the outcome is resolved from the same world,
/// and the post runs under a [`ContractAuth`] bounded to exactly that outcome. A
/// failure leaves the contract `ReadyToSettle` with a loud log and one more
/// `settle_attempts`; at [`MAX_SETTLE_ATTEMPTS`] it is `Disputed` and paged.
pub(crate) fn settle(ctx: &mut Ctx<'_>, id: ContractId) -> Settled {
    let Some(c) = live_contract(ctx, id) else { return Settled::NotReady };
    if !matches!(c.state, ContractState::Active | ContractState::ReadyToSettle) {
        return Settled::NotReady;
    }
    let built = {
        let Some(store) = ctx.w.module_state::<EconomyStore>() else { return Settled::NotReady };
        let providers = store.providers();
        let custody = WorldCustody { w: ctx.w, providers };
        let world = api::WorldAuth { w: ctx.w };
        let env = ValidateEnv {
            auth: &world,
            custody: &custody,
            facts: &NoFacts,
            now: ctx.w.now(),
            ic_now: ctx.w.game_now_unix(),
            next_hold_id: store.peek_next_hold(),
        };
        let snap = store.snapshot();
        match resolve_outcome(&c, snap, &env) {
            Err(e) => Err(e),
            Ok(outcome) => settlement_plan(&c, snap, &env).map(|p| (p, outcome)),
        }
    };
    let (plan, outcome) = match built {
        Ok(v) => v,
        Err(e) => return note_settle_failure(ctx, id, &e.to_string(), matches!(e, ContractError::AmbiguousOutcome)),
    };
    let owned = c.clone();
    let posted = {
        let auth = {
            let Some(store) = ctx.w.module_state::<EconomyStore>() else { return Settled::NotReady };
            ContractAuth::settling(&owned, store.snapshot(), outcome)
        };
        api::post_with(ctx, &plan, PostAuth::Injected(&auth))
    };
    match posted {
        Ok(PostOutcome::Posted(tx) | PostOutcome::AlreadyPosted(tx)) => {
            let mut next = owned;
            next.state = ContractState::Settled { transaction: tx };
            next.settle_attempts = 0;
            save(ctx, next.clone());
            notify_parties(ctx, &next, &format!("Contract #{id} has settled."));
            Settled::Posted(tx)
        }
        Ok(PostOutcome::NeedsApproval(request)) => note_settle_failure(
            ctx,
            id,
            &format!("the settlement asked for approval (request #{request}), which nothing can collect"),
            true,
        ),
        Err(e) => note_settle_failure(ctx, id, &e.to_string(), false),
    }
}

/// Record a failed settlement attempt: loud log, `ReadyToSettle` kept, and
/// `Disputed` + a level-2 page once the attempts are spent (or at once, for a
/// failure no retry can fix — an ambiguous outcome is a decision, not a wait).
fn note_settle_failure(ctx: &mut Ctx<'_>, id: ContractId, detail: &str, terminal: bool) -> Settled {
    let Some(mut c) = live_contract(ctx, id) else { return Settled::Refused(detail.to_string()) };
    if !matches!(c.state, ContractState::Active | ContractState::ReadyToSettle) {
        return Settled::Refused(detail.to_string());
    }
    tracing::warn!(contract = id.0, detail, "economy: a contract settlement was refused; nothing moved");
    c.settle_attempts = c.settle_attempts.saturating_add(1);
    let spent = terminal || c.settle_attempts >= MAX_SETTLE_ATTEMPTS;
    if c.state == ContractState::Active {
        c.state = ContractState::ReadyToSettle;
    }
    save(ctx, c);
    if spent {
        dispute_with(
            ctx,
            id,
            format!(
                "contract #{id} could not settle after {MAX_SETTLE_ATTEMPTS} attempts: {detail}. \
                 Its escrow is frozen until a human decides"
            ),
        );
    }
    Settled::Refused(detail.to_string())
}

// ---- dispute, agreement, resolution ---------------------------------------

/// Move a contract to `Disputed` and open the level-2 record for it, through the
/// **ladder's** own `page` (§4.8): one open record per subject, an acknowledged one
/// reopened rather than re-minted.
pub(crate) fn dispute_with(ctx: &mut Ctx<'_>, id: ContractId, context: String) {
    let Some(mut c) = live_contract(ctx, id) else { return };
    if c.state.is_terminal() {
        return;
    }
    if c.state != ContractState::Disputed {
        c.state = ContractState::Disputed;
        c.disputed_at = Some(ctx.w.now());
        save(ctx, c.clone());
        notify_parties(
            ctx,
            &c,
            &format!("Contract #{id} is disputed: its escrow is frozen until it is resolved."),
        );
    }
    let now = ctx.w.now();
    crate::sweep::page(
        ctx,
        EscalationKind::SettlementRefused,
        EscalationSubject::Contract(id),
        None,
        context,
        vec![
            format!("@contract/show #{id}"),
            format!("@contract/resolve #{id}=settle"),
            format!("@contract/resolve #{id}=refund"),
        ],
        now,
    );
}

fn contract_dispute(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let c = contract_of(ctx, ctx.args.lhs())?;
    if acting_party(ctx, &c, "contract.dispute").is_none() && !is_admin(ctx.w, ctx.actor) {
        return Err(CmdError::msg(NO_SUCH));
    }
    if !matches!(c.state, ContractState::Active | ContractState::ReadyToSettle) {
        return Err(CmdError::msg(format!("Contract #{} is not live.", c.id)));
    }
    let id = c.id;
    dispute_with(ctx, id, format!("contract #{id} was disputed by a party"));
    info(
        ctx,
        format!(
            "Contract #{id} is disputed. Its escrow is frozen: nothing is released and nothing is \
             captured until every party agrees (@contract/agree #{id}=settle|refund) or staff \
             resolve it."
        ),
    );
    ctx.gate_on_durable_ack();
    Ok(())
}

fn contract_agree(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: @contract/agree <id>=settle|refund";
    let c = contract_of(ctx, ctx.args.lhs())?;
    if c.state != ContractState::Disputed {
        return Err(CmdError::msg(format!("Contract #{} is not disputed.", c.id)));
    }
    let party = acting_party(ctx, &c, "contract.dispute").ok_or_else(|| CmdError::msg(NO_SUCH))?;
    let resolution = match ctx.args.rhs().map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        Some("settle") => Resolution::Settle,
        Some("refund") => Resolution::Refund,
        _ => return Err(CmdError::msg(USAGE)),
    };
    let now = ctx.w.now();
    let mut c = c;
    c.agreements.retain(|a| principal_key(&a.party) != principal_key(&party));
    if c.agreements.len() >= MAX_AGREEMENTS {
        return Err(CmdError::msg("Too many recorded agreements."));
    }
    c.agreements.push(PartyAgreement { party, resolution, at: now });
    // Level 0 (§4.6): unanimity among the parties resolves a dispute with no staff at
    // all — the owner's operating constraint is that a game may have one staffer who
    // is hours away, so the economy resolves whatever it can determine itself.
    let unanimous = c.consent.parties.iter().all(|p| {
        c.agreements
            .iter()
            .any(|a| principal_key(&a.party) == principal_key(p) && a.resolution == resolution)
    });
    let id = c.id;
    save(ctx, c);
    info(ctx, format!("Contract #{id}: your agreement to {resolution:?} is recorded."));
    if unanimous {
        info(ctx, format!("Every party agrees; contract #{id} is being resolved."));
        apply_resolution(ctx, id, resolution);
    }
    ctx.gate_on_durable_ack();
    Ok(())
}

fn contract_resolve(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: @contract/resolve <id>=settle|refund   (staff)";
    if !is_admin(ctx.w, ctx.actor) {
        return Err(CmdError::msg(NO_SUCH));
    }
    let c = contract_of(ctx, ctx.args.lhs())?;
    if c.state != ContractState::Disputed {
        return Err(CmdError::msg(format!("Contract #{} is not disputed.", c.id)));
    }
    let resolution = match ctx.args.rhs().map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        Some("settle") => Resolution::Settle,
        Some("refund") => Resolution::Refund,
        _ => return Err(CmdError::msg(USAGE)),
    };
    let id = c.id;
    apply_resolution(ctx, id, resolution);
    ctx.gate_on_durable_ack();
    Ok(())
}

/// Apply a resolution to a disputed contract.
///
/// `Settle` puts it back to `ReadyToSettle` and attempts a settlement — which
/// **still re-verifies** (D-E4-4): unanimity is consent to settle, not evidence that
/// the goods are in the vault, and the one thing a contract may never do is pay out
/// on a condition that is not true.
fn apply_resolution(ctx: &mut Ctx<'_>, id: ContractId, resolution: Resolution) {
    let Some(mut c) = live_contract(ctx, id) else { return };
    if c.state != ContractState::Disputed {
        return;
    }
    match resolution {
        Resolution::Settle => {
            c.state = ContractState::ReadyToSettle;
            c.settle_attempts = 0;
            save(ctx, c);
            match settle(ctx, id) {
                Settled::Posted(_) => {}
                Settled::Refused(detail) => {
                    info(
                        ctx,
                        format!(
                            "Contract #{id} could not settle even so: {detail}. Nothing moved; it \
                             stays where it was."
                        ),
                    );
                }
                Settled::NotReady => {
                    info(ctx, format!("Contract #{id} is not in a state that can settle."));
                }
            }
        }
        Resolution::Refund => {
            unwind(ctx, id, ContractState::Cancelled);
        }
    }
}

// ---- cancellation, amendment, refunds --------------------------------------

fn contract_cancel(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let c = contract_of(ctx, ctx.args.lhs())?;
    let staff = is_admin(ctx.w, ctx.actor);
    if !is_issuer(ctx, &c) {
        return Err(CmdError::msg(NO_SUCH));
    }
    match c.state {
        ContractState::Draft | ContractState::Offered => {}
        // A live agreement is not one party's to tear up: the routes are a dispute
        // plus unanimous agreement, the deadline, or staff.
        ContractState::Active | ContractState::ReadyToSettle | ContractState::Disputed if staff => {}
        ContractState::Active | ContractState::ReadyToSettle => {
            return Err(CmdError::msg(format!(
                "Contract #{} is in force. Dispute it (@contract/dispute #{}) and agree a refund, \
                 or wait for its deadline.",
                c.id, c.id
            )));
        }
        ContractState::Disputed => {
            return Err(CmdError::msg(format!(
                "Contract #{} is disputed; @contract/agree #{}=refund ends it by agreement.",
                c.id, c.id
            )));
        }
        _ => return Err(CmdError::msg(format!("Contract #{} is already finished.", c.id))),
    }
    let id = c.id;
    unwind(ctx, id, ContractState::Cancelled);
    info(ctx, format!("Contract #{id} is cancelled."));
    ctx.gate_on_durable_ack();
    Ok(())
}

/// `@contract/amend <id>` — the sanctioned way to change an `Offered` contract's
/// terms after somebody has accepted them (08e answer 5).
///
/// An acceptance binds to the whole consent surface's hash, which is the money-safety
/// spine: changing anything voids every acceptance. So an amendment is explicit about
/// it — it refunds every escrow placed so far, clears the acceptances, re-opens the
/// roster, and returns the contract to `Draft`. The affected parties then re-accept
/// the amended terms, which is exactly the consent the answer asks for.
fn contract_amend(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let c = contract_of(ctx, ctx.args.lhs())?;
    if !is_issuer(ctx, &c) {
        return Err(CmdError::msg(NO_SUCH));
    }
    if c.state != ContractState::Offered {
        return Err(CmdError::msg(format!(
            "Only an offered contract can be amended; #{} is not.",
            c.id
        )));
    }
    if c.amendments >= MAX_AMENDMENTS {
        return Err(CmdError::msg(format!(
            "Contract #{} has been amended {MAX_AMENDMENTS} times; draft a new one.",
            c.id
        )));
    }
    let id = c.id;
    let had = c.acceptances.len();
    // Refund through the ordinary unwind (the state change first, then the money),
    // then re-open the surface.
    unwind(ctx, id, ContractState::Cancelled);
    let Some(mut back) = live_contract(ctx, id) else { return Ok(()) };
    back.state = ContractState::Draft;
    // The generation every deterministic key carries. It moves BEFORE anything else,
    // because the refund above posted under the old one: without it a second
    // cancellation would reuse `contract/{id}/refund`, resolve to that first refund
    // as an idempotency HIT, and hand nothing back.
    back.amendments = back.amendments.saturating_add(1);
    back.acceptances.clear();
    back.agreements.clear();
    back.escrow.clear();
    back.deposits.clear();
    back.roster_locked = false;
    back.settle_attempts = 0;
    save_open(ctx, back.clone());
    info(ctx, format!("Contract #{id} is back to draft; {had} acceptance(s) were voided and refunded."));
    notify_parties(
        ctx,
        &back,
        &format!("Contract #{id} has been amended: your acceptance is void and your escrow released."),
    );
    Ok(())
}

/// Take a contract out of force and give everything back (§5).
///
/// The order is not negotiable: the **state change comes first**, because that is
/// what lifts the `DepositInto` custody lock (E4a verify fold, finding 2), and only
/// then does the refund plan run — releasing the escrow, releasing the pledges, and
/// returning the deposit to its depositor, all in the same durable bundle under
/// [`ContractAuth::unwinding`].
pub(crate) fn unwind(ctx: &mut Ctx<'_>, id: ContractId, state: ContractState) {
    let Some(mut c) = live_contract(ctx, id) else { return };
    if c.state.is_terminal() {
        return;
    }
    c.state = state;
    save(ctx, c.clone());
    cancel_lease_obligations(ctx, id);

    // Only the returns this world can actually perform ride the plan: an object that
    // is no longer in the vault, or a depositor who is not a legal destination, must
    // not make the whole refund fail and strand the escrow.
    let mut working = c.clone();
    working.deposits.retain(|d| returnable(ctx, d));
    let plan = {
        let Some(store) = ctx.w.module_state::<EconomyStore>() else { return };
        refund_plan(&working, store.snapshot())
    };
    let plan = match plan {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(contract = id.0, error = %e, "economy: a contract's refund plan could not be built");
            return;
        }
    };
    if plan.hold_ops.is_empty() && plan.asset_ops.is_empty() && plan.custody_ops.is_empty() {
        return;
    }
    let posted = {
        let auth = {
            let Some(store) = ctx.w.module_state::<EconomyStore>() else { return };
            ContractAuth::unwinding(&working, store.snapshot())
        };
        api::post_with(ctx, &plan, PostAuth::Injected(&auth))
    };
    match posted {
        Ok(PostOutcome::Posted(_) | PostOutcome::AlreadyPosted(_)) => {
            notify_parties(ctx, &c, &format!("Contract #{id} ended; its escrow was released."));
        }
        Ok(PostOutcome::NeedsApproval(request)) => {
            tracing::error!(contract = id.0, request = request.0, "economy: a contract refund asked for approval");
        }
        Err(e) => {
            tracing::error!(contract = id.0, error = %e, "economy: a contract refund was refused; the escrow is still held");
            let now = ctx.w.now();
            crate::sweep::page(
                ctx,
                EscalationKind::SettlementRefused,
                EscalationSubject::Contract(id),
                None,
                format!(
                    "contract #{id} ended but its refund was refused ({e}); the escrow is still \
                     reserved and its parties cannot spend it"
                ),
                vec![format!("@contract/show #{id}"), "@econ/audit".to_string()],
                now,
            );
        }
    }
}

/// Whether a recorded deposit can actually be handed back right now: the object is
/// still in the vault, and the depositor is a live object the withdrawal may name as
/// a destination.
fn returnable(ctx: &Ctx<'_>, d: &DepositRecord) -> bool {
    let PrincipalId::Object(to) = &d.deposited_by else { return false };
    let Some(store) = ctx.w.module_state::<EconomyStore>() else { return false };
    let Some(vault) = store.snapshot().vault(d.vault) else { return false };
    if vault.status != crate::record::VaultStatus::Active {
        return false;
    }
    let custody = WorldCustody { w: ctx.w, providers: store.providers() };
    use crate::engine::CustodyEval;
    custody.holds(vault, d.object) && custody.destination_ok(&d.deposited_by, *to)
}

// ---- expiry (the sweep's arm, and the shared logic) ------------------------

/// Apply `on_expiry` to a contract whose deadline has passed (§5).
///
/// An `Offered` contract **always refunds**, whatever `on_expiry` says: it never came
/// into force, and taking the money someone escrowed toward an agreement that never
/// bound anybody is a confiscation rather than a forfeit (§5.1's deviation).
pub(crate) fn expire(ctx: &mut Ctx<'_>, id: ContractId) {
    let Some(c) = live_contract(ctx, id) else { return };
    let bound = c.fully_accepted();
    match c.consent.terms.on_expiry {
        Expiry::Refund => {
            unwind(ctx, id, ContractState::Expired);
        }
        Expiry::Dispute if bound => {
            dispute_with(
                ctx,
                id,
                format!("contract #{id} reached its deadline unsettled and its terms say to dispute"),
            );
        }
        Expiry::Dispute => {
            unwind(ctx, id, ContractState::Expired);
        }
        Expiry::Forfeit { .. } if bound => forfeit(ctx, id),
        Expiry::Forfeit { .. } => {
            tracing::warn!(
                contract = id.0,
                "economy: an offered contract lapsed before it bound anybody; refunding rather \
                 than forfeiting"
            );
            unwind(ctx, id, ContractState::Expired);
        }
    }
}

/// `Expiry::Forfeit { to }`: the escrow pays out to the account the consent surface
/// named, under a [`ContractAuth::forfeiting`] that can credit exactly that account
/// and nothing else.
fn forfeit(ctx: &mut Ctx<'_>, id: ContractId) {
    let Some(mut c) = live_contract(ctx, id) else { return };
    let Expiry::Forfeit { to } = c.consent.terms.on_expiry else { return };
    if c.state.is_terminal() {
        return;
    }
    c.state = ContractState::Expired;
    save(ctx, c.clone());
    cancel_lease_obligations(ctx, id);
    let plan = {
        let Some(store) = ctx.w.module_state::<EconomyStore>() else { return };
        forfeit_plan(&c, store.snapshot())
    };
    let plan = match plan {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(contract = id.0, error = %e, "economy: a forfeit plan could not be built");
            return;
        }
    };
    let posted = {
        let auth = {
            let Some(store) = ctx.w.module_state::<EconomyStore>() else { return };
            ContractAuth::forfeiting(&c, store.snapshot(), to)
        };
        api::post_with(ctx, &plan, PostAuth::Injected(&auth))
    };
    match posted {
        Ok(PostOutcome::Posted(_) | PostOutcome::AlreadyPosted(_)) => {
            notify_parties(ctx, &c, &format!("Contract #{id} lapsed; its escrow was forfeit."));
        }
        Ok(PostOutcome::NeedsApproval(_)) | Err(_) => {
            let detail = match posted {
                Err(e) => e.to_string(),
                _ => "the forfeit asked for an approval nothing can collect".to_string(),
            };
            tracing::error!(contract = id.0, %detail, "economy: a contract forfeit was refused");
            dispute_with(
                ctx,
                id,
                format!("contract #{id} lapsed but its forfeit was refused ({detail}); the escrow is frozen"),
            );
        }
    }
}

// ---- notifications --------------------------------------------------------

/// Tell one principal something, if it is a principal that can be told (E4c's
/// observer seam is where an org or system party gets reached).
fn notify(ctx: &mut Ctx<'_>, who: &PrincipalId, msg: &str) {
    if let PrincipalId::Object(obj) = who {
        ctx.out.sys_to_actor(*obj, "economy", msg.to_string());
    }
}

/// Tell every party once. **No message here claims money durably moved** (D-E4-6):
/// the sweep drives the same paths and does not take the whole-step durable gate.
fn notify_parties(ctx: &mut Ctx<'_>, c: &EconomicContract, msg: &str) {
    let parties = c.consent.parties.clone();
    let mut told: Vec<ObjId> = Vec::new();
    for p in &parties {
        if let PrincipalId::Object(o) = p {
            if told.contains(o) {
                continue;
            }
            told.push(*o);
        }
        notify(ctx, p, msg);
    }
}

// ---- @contract/show | list ------------------------------------------------

fn contract_show(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let c = contract_of(ctx, ctx.args.lhs())?;
    let staff = is_admin(ctx.w, ctx.actor);
    let scale = ctx.w.module_state::<EconomyStore>().map_or(0, |s| {
        let snap = s.snapshot();
        c.consent
            .terms
            .consideration
            .first()
            .and_then(|x| snap.account(x.account))
            .and_then(|a| snap.instrument(&a.instrument))
            .map_or(0, |i| i.scale)
    });
    let mut lines = vec![
        format!("contract #{} \"{}\" — {}", c.id, c.name, render_state(&c.state)),
        format!("  issued by {}", render_principal(ctx, &c.issuer)),
    ];
    let parties: Vec<String> = c.consent.parties.iter().map(|p| render_principal(ctx, p)).collect();
    lines.push(format!("  parties: {}", parties.join(", ")));
    // A party sees the other parties as the consent surface names them, and nothing
    // more: no balances, no account contents, only what they agreed to.
    let accepted: Vec<String> = c
        .acceptances
        .iter()
        .filter(|a| a.consent_hash == c.consent_hash)
        .map(|a| render_principal(ctx, &a.party))
        .collect();
    lines.push(format!(
        "  accepted by: {}",
        if accepted.is_empty() { "nobody yet".to_string() } else { accepted.join(", ") }
    ));
    for x in &c.consent.terms.consideration {
        let acct = if may_view_account(ctx.w, ctx.actor, x.account) {
            format!("account #{}", x.account)
        } else {
            "an account".to_string()
        };
        lines.push(format!(
            "  {} escrows {} from {acct}",
            render_principal(ctx, &x.from),
            x.amount.to_decimal_string(scale)
        ));
    }
    if c.consent.terms.outcomes.is_empty() {
        for s in &c.consent.terms.split {
            lines.push(format!("  pays #{} {}", s.to, render_share(s.share, scale)));
        }
        lines.push(format!("  remainder to #{}", c.consent.terms.remainder_to));
    } else {
        for o in &c.consent.terms.outcomes {
            let pays: Vec<String> = o
                .payouts
                .iter()
                .map(|s| format!("#{} {}", s.to, render_share(s.share, scale)))
                .collect();
            lines.push(format!(
                "  outcome \"{}\" when {:?} pays {}",
                o.name,
                o.when,
                pays.join(", ")
            ));
        }
    }
    for (i, d) in c.consent.terms.deliverables.iter().enumerate() {
        lines.push(format!("  deliverable {i}: {}", render_deliverable(ctx, d)));
    }
    for (i, cond) in c.consent.conditions.iter().enumerate() {
        let mark = match c.satisfied.get(i).and_then(|s| s.as_ref()) {
            Some(a) => format!(" [attested by {} at {}]", render_attestor(ctx, &a.by), a.at),
            None => String::new(),
        };
        lines.push(format!("  condition {i}: {}{mark}", render_condition(ctx, cond)));
    }
    if let Some(a) = &c.consent.adjudicator {
        lines.push(format!("  adjudicator: {}", render_attestor_set(ctx, a)));
    }
    if !c.consent.evidence.is_empty() {
        lines.push(format!("  permitted evidence: {:?}", c.consent.evidence));
    }
    if let Some(d) = c.consent.deadline {
        lines.push(format!(
            "  deadline {} on {:?}; on expiry {}",
            d.at,
            d.clock,
            render_expiry(c.consent.terms.on_expiry)
        ));
    }
    lines.push(format!("  visibility: {}", c.consent.visibility.label()));
    if let EconomicVisibility::Audience { members } = &c.consent.visibility {
        let who: Vec<String> = members.iter().map(|p| render_principal(ctx, p)).collect();
        lines.push(format!("  audience: {}", who.join(", ")));
    }
    for (label, who) in [
        ("author", &c.roles.author),
        ("payer", &c.roles.payer),
        ("johnson", &c.roles.representative),
        ("facilitator", &c.roles.facilitator),
    ] {
        if let Some(p) = who {
            lines.push(format!("  {label}: {}", render_principal(ctx, p)));
        }
    }
    if !c.escrow.is_empty() {
        let ids: Vec<String> = c.escrow.iter().map(|h| format!("#{h}")).collect();
        lines.push(format!("  escrow holds: {}", ids.join(", ")));
    }
    if staff {
        lines.push(format!(
            "  roster {}, settle attempts {}, revision {}",
            if c.roster_locked { "locked" } else { "open" },
            c.settle_attempts,
            c.revision
        ));
        for d in &c.deposits {
            lines.push(format!(
                "  deposit #{} in vault #{} by {}",
                d.object.0,
                d.vault,
                render_principal(ctx, &d.deposited_by)
            ));
        }
    }
    for l in lines {
        info(ctx, l);
    }
    Ok(())
}

fn render_state(s: &ContractState) -> String {
    match s {
        ContractState::Draft => "draft".to_string(),
        ContractState::Offered => "offered".to_string(),
        ContractState::Active => "active".to_string(),
        ContractState::ReadyToSettle => "ready to settle".to_string(),
        ContractState::Settled { transaction } => format!("settled (transaction #{transaction})"),
        ContractState::Disputed => "disputed".to_string(),
        ContractState::Cancelled => "cancelled".to_string(),
        ContractState::Expired => "expired".to_string(),
    }
}

fn render_share(s: Share, scale: u8) -> String {
    match s {
        Share::Exact(a) => a.to_decimal_string(scale),
        Share::Permille(p) => format!("{p}‰"),
    }
}

fn render_expiry(e: Expiry) -> String {
    match e {
        Expiry::Refund => "refund".to_string(),
        Expiry::Forfeit { to } => format!("forfeit to #{to}"),
        Expiry::Dispute => "dispute".to_string(),
    }
}

fn render_deliverable(ctx: &Ctx<'_>, d: &Deliverable) -> String {
    match d {
        Deliverable::TitleTransfer { object, from, to } => format!(
            "title of #{} from {} to {}",
            object.0,
            render_principal(ctx, from),
            render_principal(ctx, to)
        ),
        Deliverable::DepositInto { object, vault } => {
            format!("#{} deposited into vault #{vault}", object.0)
        }
        Deliverable::Collateral { object, from } => {
            format!("#{} pledged by {}", object.0, render_principal(ctx, from))
        }
        Deliverable::Obligation { template } => format!(
            "an obligation: account #{} owes #{} {} each period",
            template.debit_account,
            template.credit_account,
            template.amount.minor()
        ),
    }
}

fn render_condition(ctx: &Ctx<'_>, c: &TypedCondition) -> String {
    match c {
        TypedCondition::Attestation { by } => format!("attested by {}", render_attestor_set(ctx, by)),
        TypedCondition::DepositInto { object, vault } => {
            format!("#{} is in vault #{vault}", object.0)
        }
        TypedCondition::TitleHeld { object, by } => {
            format!("{} holds title to #{}", render_principal(ctx, by), object.0)
        }
        TypedCondition::Fact { fact } => {
            format!("the fact {}/{}/{}", fact.provider, fact.kind, fact.id)
        }
        TypedCondition::Before { deadline } => format!("before {} on {:?}", deadline.at, deadline.clock),
        TypedCondition::After { condition, delay, clock } => {
            format!("{delay} after condition {condition} on {clock:?}")
        }
    }
}

fn render_attestor_set(ctx: &Ctx<'_>, set: &AttestorSet) -> String {
    match set {
        AttestorSet::Party(p) => render_principal(ctx, p),
        AttestorSet::Capability { scope, capability } => match scope {
            Some(s) => format!("anyone with {capability} on scope #{s}"),
            None => format!("anyone with {capability} (no scope: nobody)"),
        },
        AttestorSet::Staff => "staff".to_string(),
        AttestorSet::Canon { kind, source } => match source {
            Some(s) => format!("a canon {kind} naming {s}"),
            None => format!("any canon {kind}"),
        },
    }
}

fn render_attestor(ctx: &Ctx<'_>, by: &AttestorRef) -> String {
    match by {
        AttestorRef::Principal(p) => render_principal(ctx, p),
        AttestorRef::Canon { canon, .. } => canon.to_string(),
        AttestorRef::Fact(f) => format!("{}/{}", f.provider, f.kind),
        AttestorRef::Clock => "the clock".to_string(),
    }
}

fn contract_list(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let ids: Vec<ContractId> = {
        let store = ctx.w.module_state::<EconomyStore>().ok_or_else(|| CmdError::msg("No economy."))?;
        store.snapshot().contracts().map(|c| c.id).collect()
    };
    let rows: Vec<String> = ids
        .into_iter()
        .filter(|id| may_view_contract(ctx.w, ctx.actor, *id))
        .filter_map(|id| {
            let store = ctx.w.module_state::<EconomyStore>()?;
            let c = store.snapshot().contract(id)?;
            Some(format!(
                "#{} \"{}\" — {} ({} part{})",
                c.id,
                c.name,
                render_state(&c.state),
                c.consent.parties.len(),
                if c.consent.parties.len() == 1 { "y" } else { "ies" }
            ))
        })
        .collect();
    emit_list(ctx, rows, "No contracts.");
    Ok(())
}
