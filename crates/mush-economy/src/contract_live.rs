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
//! - **The plan lands first, then the state change** (E4b2 verify fold, D31 — the
//!   terminal-transition rule). A terminal transition is a post under the contract's
//!   deterministic key (`settle_key`/`refund_key`/`forfeit_key`) followed by the state
//!   write, with `pending: PendingTerminal` recorded so the sweep and boot can finish
//!   a half-done one; a refund's release runs against a contract already marked
//!   `Cancelled`/`Expired` in the same bundle, because that is what lifts the custody
//!   lock (E4a verify fold, finding 2). A settlement is a single plan that captures,
//!   pays out, moves title and releases pledges, or does none of it.
//! - **Nothing here panics.** The sweep drives the same functions, and a panic in a
//!   tick handler aborts the world — so there is no `unwrap`, no `expect`, and no
//!   indexing that can panic on any path the sweep can reach.

use std::sync::Arc;

use mush_core::module::{ModError, Registrar};
use mush_core::{ArgShape, CmdError, CommandSpec, Ctx, ObjId, ObjKind, World};
use mush_org::{CapabilityId, OrgScopeId, PrincipalId};
use smol_str::SmolStr;

use crate::amount::Amount;
use crate::api::{self, PostAuth, PostOutcome};
use crate::clock::{EconomicClock, EconomicDeadline, EconomicSchedule};
use crate::commands::{
    asset_target, cmd_key, emit_list, info, is_admin, is_hidden, mask_account, may_view_account,
    may_view_vault, money_on, money_on_or, parse_account, parse_amount, parse_deadline, parse_opts,
    parse_principal, parse_scope, parse_vault_id, refuse, refuse_if_frozen, refuse_named,
    render_principal,
};
use crate::contract::{
    Acceptance, Attestation, AttestorRef, AttestorSet, ConsentSurface, Consideration, ContractAuth,
    ContractError, ContractRoles, ContractState, ContractTerms, DRAFT_TTL_SECS, Deliverable,
    DepositRecord, EconomicContract, EvidenceKind, Expiry, MAX_AGREEMENTS, MAX_AMENDMENTS,
    MAX_CONDITIONS, MAX_CONTRACT_NAME_BYTES, MAX_DELIVERABLES, MAX_DEPOSITS,
    MAX_LIVE_CONTRACTS_PER_PARTY, MAX_OPEN_CONTRACTS_PER_ISSUER, MAX_OUTCOMES, MAX_PARTIES,
    MAX_SETTLE_ATTEMPTS, MAX_SHARES, ObligationTemplate, OutcomeTerms, PERMILLE, PartyAgreement,
    PayoutShare, PendingKind, PendingTerminal, Readiness, Resolution, Share, TypedCondition,
    activate_key, check_offerable, consent_hash, forfeit_plan, principal_key, qualifying_escrow,
    readiness, refund_plan, resolve_outcome, settlement_plan,
};
use crate::custody::WorldCustody;
use crate::engine::ValidateEnv;
use crate::escalation::{EscalationKind, EscalationSubject};
use crate::facts::WorldFacts;
use crate::ids::{AccountId, ContractId, HoldId, InstrumentId};
use crate::module::EconomyStore;
use crate::obligation::{GracePolicy, Obligation, ObligationCause, ObligationStatus};
use crate::plan::{EconomyError, EconomyPlan, HoldOp, NamedResource};
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
/// Party and audience membership both ask `api::WorldAuth::acts_for` rather than
/// identity, for the reason E4b1's fold gave for holds (A2): a contract with an
/// organization as a party would otherwise be invisible to every person in it,
/// including the ones who will have to accept and fund it.
///
/// The renderer treats a contract id exactly like an account id for everyone else,
/// so the nonexistent and the unviewable produce identical text.
/// Whether `who` **stands on one side of** contract `id` (E6b independent verify
/// fold, M4).
///
/// True when they are a party (directly, or through `acts_for` — an org's officer
/// stands in their org's contract) or hold one of its named roles: author, payer,
/// representative or facilitator. An unknown contract answers `false`.
///
/// This is the seam a *producer* asks so it can keep 08e answer 22's rule — the
/// authority that ends a contract never stands inside it — on an authority the
/// economy does not own. `mush-scenes` wires it in as its `SealerGate` and never
/// learns what a contract is; the run id it holds stays opaque there.
#[must_use]
pub fn stands_in_contract(w: &World, id: ContractId, who: ObjId) -> bool {
    let Some(store) = w.module_state::<EconomyStore>() else {
        return false;
    };
    let Some(c) = store.snapshot().contract(id) else {
        return false;
    };
    let me = PrincipalId::Object(who);
    let auth = api::WorldAuth { w };
    let stands = |p: &PrincipalId| p == &me || auth.acts_for(&me, p);
    c.consent.parties.iter().any(&stands)
        || [
            c.roles.author.as_ref(),
            c.roles.payer.as_ref(),
            c.roles.representative.as_ref(),
            c.roles.facilitator.as_ref(),
        ]
        .into_iter()
        .flatten()
        .any(stands)
}

#[must_use]
pub fn may_view_contract(w: &World, actor: ObjId, id: ContractId) -> bool {
    if api::is_staff(w, actor) {
        return true;
    }
    let Some(store) = w.module_state::<EconomyStore>() else {
        return false;
    };
    let Some(c) = store.snapshot().contract(id) else {
        return false;
    };
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
    if parties
        .iter()
        .chain(audience.iter())
        .any(|p| auth.acts_for(&me, p))
    {
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
    if attestors
        .into_iter()
        .any(|set| attestor_can_view(w, actor, set))
    {
        return true;
    }
    // An org contract is visible to the people the org authorized to act on it —
    // without this an org member holding `contract.accept` could not read the
    // contract they are supposed to accept.
    scope.is_some_and(|s| {
        [
            "contract.draft",
            "contract.accept",
            "contract.attest",
            "contract.dispute",
            "economy.account.view",
        ]
        .iter()
        .any(|cap| api::holds_capability(w, actor, cap, s))
    })
}

/// Whether `actor` is somebody the attestor set names — the view half of "the
/// adjudicator sees what they adjudicate". Staff are already through the gate above;
/// a canon set names no person at all.
fn attestor_can_view(w: &World, actor: ObjId, set: &AttestorSet) -> bool {
    match set {
        AttestorSet::Party(p) => api::WorldAuth { w }.acts_for(&PrincipalId::Object(actor), p),
        AttestorSet::Capability { scope, capability } => {
            scope.is_some_and(|s| api::holds_capability(w, actor, capability.as_str(), s))
        }
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
    s.trim()
        .trim_start_matches('#')
        .parse::<u64>()
        .ok()
        .map(ContractId)
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
                   Builders — the ISSUER (or staff), on a Draft, or on an Offered contract whose\n\
                   roster is still open and which nobody has accepted:\n\
                   @contract/party <id>=<principal>\n\
                   @contract/pay <id>=<acct> <amount>         what that account escrows\n\
                   \x20  the issuer may name only an account they may view; a listed PARTY may add\n\
                   \x20  their own account to a Draft (@contract/pay <id>=<your acct> <amount>)\n\
                   @contract/deliver <id>=title <obj> <from>-><to> | deposit <obj> <vault>\n\
                   \x20                    | collateral <obj> <from> | obligation <debit> <credit> <amt> every=<secs>\n\
                   \x20  a collateral is pledged BY A PARTY who holds its title\n\
                   @contract/split <id>=<acct> <exact|N‰> [<acct> <share> …]\n\
                   @contract/remainder <id>=<acct>\n\
                   @contract/condition <id>=attest <who> | deposit <obj> <vault> | title <obj> <principal>\n\
                   \x20                    | fact <provider> <kind> <id> | before <secs|ic:secs> | after <n> <delay>\n\
                   \x20  <who> = party:<principal> | cap:<capability>:<scope> | staff | canon:<kind>:<source>\n\
                   \x20  `title` takes a bare <principal> (#123, org:Name) — no party: prefix\n\
                   @contract/deadline <id>=<secs|ic:secs> [refund|forfeit=<acct>|dispute]\n\
                   @contract/visibility <id>=parties|staff|public|audience <principal> … [announce=yes]\n\
                   \x20  an audience sees a NOTICE (name, state, window, condition kinds) and no money\n\
                   @contract/adjudicator <id>=<who>|none      who may attest (consented)\n\
                   @contract/evidence <id>=attest,deposit,title,fact,before,after|any\n\
                   @contract/outcome <id>=<name> when=<n,…> pay=<acct>:<share>,… [titles=<n,…>]\n\
                   @contract/role <id>=author|payer|johnson|facilitator <principal>|none\n\
                   @contract/roster <id>=lock|open            issuer or staff: freeze/re-open the parties\n\
                   Lifecycle:\n\
                   @contract/offer <id>                       ISSUER or staff: Draft → Offered; freezes the hash\n\
                   @contract/accept <id>                      a PARTY; places their escrow in the same step\n\
                   @contract/amend <id>                       ISSUER or staff: Offered → Draft; voids and refunds every\n\
                   \x20  acceptance but the AUTHOR's own (the party you act for, else the issuer), whose escrow stands\n\
                   @contract/attest <id>=<n>                  an ATTESTOR; may settle in the same step (once per condition)\n\
                   @contract/dispute <id>                     a PARTY (or staff); freezes the escrow\n\
                   @contract/agree <id>=settle|refund         a PARTY; unanimity ends a dispute\n\
                   @contract/cancel <id>                      the ISSUER (Draft/Offered) or staff\n\
                   @contract/resolve <id>=settle|refund [reason]   staff; /settle attests
\
                      every unmet attestation condition (deterministic ones are still checked)
\
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
/// Three routes, tried in a fixed order so "which party did I accept for" is never
/// iteration-dependent: the actor **is** a party; the actor holds `capability` on the
/// contract's own compartment and one of the parties is that compartment's
/// organization; or the actor simply [`api::WorldAuth::acts_for`] one of the parties.
///
/// The third is the E4b2 verify fold's A4. The capability route needs a compartment,
/// and an **unscoped** contract has none — so an `org:X` party on a contract drafted
/// without `scope=` could be accepted, disputed and agreed by *nobody*: its escrow
/// would sit until the deadline with the org's own officers locked out. `acts_for` is
/// the same predicate that decides who may spend an organization's money and who may
/// see its holds (E4b1's A2), so seeing a contract, funding it and acting on it stay
/// one question.
fn acting_party(ctx: &Ctx<'_>, c: &EconomicContract, capability: &str) -> Option<PrincipalId> {
    let me = PrincipalId::Object(ctx.actor);
    if c.consent.parties.contains(&me) {
        return Some(me);
    }
    // Canonical order, so a contract naming two principals one actor speaks for
    // resolves to the same party every time.
    let mut parties: Vec<&PrincipalId> = c.consent.parties.iter().collect();
    parties.sort_by(|a, b| principal_key(a).cmp(&principal_key(b)));
    if holds_contract_capability(ctx, c.consent.scope, capability)
        && let Some(scope) = c.consent.scope
        && let Some(store) = ctx.w.module_state::<mush_org::OrgStore>()
        && let Some(org) = store.state().scope(scope).map(|s| s.org)
    {
        let wanted = PrincipalId::Organization(org);
        if let Some(p) = parties.iter().find(|p| ***p == wanted) {
            return Some((*p).clone());
        }
    }
    let auth = api::WorldAuth { w: ctx.w };
    parties
        .into_iter()
        // Only for a principal that cannot act for itself. A **person** party is
        // still an identity match: staff and MUSH controllers get no silent power to
        // accept, dispute or agree in somebody else's name (staff already have
        // `@contract/cancel` and `@contract/resolve`, which say whose act it was).
        .find(|p| matches!(p, PrincipalId::Organization(_)) && auth.acts_for(&me, p))
        .cloned()
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
        _ => Err(CmdError::msg(format!(
            "Contract #{} is no longer being drafted.",
            c.id
        ))),
    }
}

/// Persist an edited contract, re-freezing its consent hash while it is still open
/// for editing so `consent_intact()` never reads a draft as tampered-with.
///
/// **An `Offered` contract is re-validated here** (E4b2 verify fold, M7).
/// `@contract/roster <id>=open` puts an offered contract back under the builders, and
/// every builder then re-signed the surface with no feasibility check at all: an
/// offer that had passed `check_offerable` could be edited into one that could never
/// settle — an infeasible split, an account that had since closed, a `Manual`
/// deadline — and the parties would be looking at a live offer that would refuse
/// every acceptance or strand its escrow. Re-validating is the same gate `/offer`
/// runs, and the parties are told their offer moved, because it did.
fn save_open(ctx: &mut Ctx<'_>, mut c: EconomicContract) -> Result<ContractId, CmdError> {
    c.consent_hash = consent_hash(&c.issuer, &c.consent);
    c.revision = c.revision.saturating_add(1);
    let id = c.id;
    if c.state == ContractState::Offered {
        let verdict = {
            let store = ctx
                .w
                .module_state::<EconomyStore>()
                .ok_or_else(|| CmdError::msg("No economy."))?;
            let outcomes = store.canon_outcome_kinds();
            crate::contract::check_structure(&c, &outcomes)
                .and_then(|()| check_offerable(&c, store.snapshot()))
        };
        if let Err(e) = verdict {
            return Err(contract_refusal(ctx, &c, &e));
        }
    }
    let offered = c.state == ContractState::Offered;
    let parties = c.consent.parties.clone();
    api::put_contract(ctx, c);
    if offered {
        for p in &parties {
            notify(
                ctx,
                p,
                &format!("Contract #{id}'s offered terms have changed (@contract/show #{id})."),
            );
        }
    }
    ctx.gate_on_durable_ack();
    Ok(id)
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
        let store = ctx
            .w
            .module_state_mut::<EconomyStore>()
            .ok_or_else(|| CmdError::msg("No economy."))?;
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
        generation_started_at: now,
        disputed_at: None,
        canon_published: None,
        roster_locked: false,
        deposits: Vec::new(),
        agreements: Vec::new(),
        pending: None,
        roles: ContractRoles {
            author: Some(PrincipalId::Object(ctx.actor)),
            ..ContractRoles::default()
        },
        revision: 1,
    };
    api::put_contract(ctx, c);
    let meta = ctx
        .w
        .module_state::<EconomyStore>()
        .map(EconomyStore::meta_doc);
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
    let rhs = ctx
        .args
        .rhs()
        .ok_or_else(|| CmdError::msg(USAGE))?
        .to_string();
    let who = parse_principal(ctx, &rhs).ok_or_else(|| CmdError::msg("No such principal."))?;
    // E6a step 1 (M4): a `System` principal is never a party (`check_structure`
    // refuses it at offer too); the text is the unresolvable-principal one.
    if matches!(who, PrincipalId::System(_)) {
        return Err(CmdError::msg("No such principal."));
    }
    if c.consent
        .parties
        .iter()
        .any(|p| principal_key(p) == principal_key(&who))
    {
        return Err(CmdError::msg("That principal is already a party."));
    }
    if c.consent.parties.len() >= MAX_PARTIES {
        return Err(CmdError::msg(format!(
            "A contract may name at most {MAX_PARTIES} parties."
        )));
    }
    c.consent.parties.push(who.clone());
    let rendered = render_principal(ctx, &who);
    let id = save_open(ctx, c)?;
    info(ctx, format!("Contract #{id}: {rendered} is now a party."));
    Ok(())
}

/// `@contract/pay <id>=<account> <amount>` — what that account escrows at acceptance.
///
/// **Two rules, both from the E4b2 verify fold's A1.**
///
/// (a) The issuer may name only an account they [`may_view_account`]. `/pay` used to
/// answer three different ways for an id the drafter had no standing over — "No such
/// account." for a missing one, the same text for a closed one, and *success* for a
/// live one, whose owner it then printed — which made it an existence-and-owner oracle
/// over every wallet in the game, driveable from a throwaway draft.
///
/// (b) A **listed party** may add their own consideration to a `Draft`, naming an
/// account they may view. That is the other half: with (a) alone, a payer whose wallet
/// the Johnson cannot see could never be written into the contract at all, and the
/// answer to "who pays" would have quietly become "only people whose finances the
/// drafter can read". Self-consideration keeps the disclosure where it belongs — the
/// payer discloses their own account, to a contract they are already named on.
fn build_pay(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: @contract/pay <id>=<account> <amount>";
    const HIDDEN: &str = "No such account.";
    let seen = contract_of(ctx, ctx.args.lhs())?;
    let rhs = ctx
        .args
        .rhs()
        .ok_or_else(|| CmdError::msg(USAGE))?
        .to_string();
    let mut words = rhs.split_whitespace();
    let account = words
        .next()
        .and_then(parse_account)
        .ok_or_else(|| CmdError::msg(USAGE))?;
    let amount = words
        .next()
        .and_then(parse_amount)
        .ok_or_else(|| CmdError::msg(USAGE))?;
    if amount.is_zero() || amount.is_negative() {
        return Err(CmdError::msg("A consideration must be positive."));
    }
    // Rule (a)/(b) share one gate: an account the actor may not VIEW renders exactly
    // like one that does not exist, before anything else is decided.
    if !may_view_account(ctx.w, ctx.actor, account) {
        return Err(CmdError::msg(HIDDEN));
    }
    // The payer is whoever OWNS the account: the accountable escrow principal is a
    // fact about the wallet, never a claim the drafter makes (08e answer 4).
    let owner = {
        let store = ctx
            .w
            .module_state::<EconomyStore>()
            .ok_or_else(|| CmdError::msg("No economy."))?;
        let a = store
            .snapshot()
            .account(account)
            .ok_or_else(|| CmdError::msg(HIDDEN))?;
        if a.status != AccountStatus::Active {
            return Err(CmdError::msg(HIDDEN));
        }
        // E6a step 1 (M4 / D-E6-4): a `System`-owned account is never a payer — a
        // faucet is not a party, and nobody could accept for it. Decided on the
        // account's OWNER before the surface is touched, and rendered as the
        // account's own existence-hiding text, never a faucet-shaped message: a
        // faucet's existence is exactly what a probe would be after.
        if matches!(a.owner, PrincipalId::System(_)) {
            return Err(CmdError::msg(HIDDEN));
        }
        a.owner.clone()
    };
    let mut c = if is_issuer(ctx, &seen) {
        editable(ctx, ctx.args.lhs())?
    } else {
        // A party's self-consideration: a `Draft` only, and only their own account —
        // adding somebody else's wallet is the issuer's job, and adding one to an
        // offer people have already read would change what they are accepting.
        acting_party(ctx, &seen, "contract.accept")
            .filter(|p| principal_key(p) == principal_key(&owner))
            .ok_or_else(|| CmdError::msg(NO_SUCH))?;
        if seen.state != ContractState::Draft {
            return Err(CmdError::msg(format!(
                "Contract #{} is no longer being drafted; ask its issuer to amend it.",
                seen.id
            )));
        }
        seen
    };
    if c.consent.terms.consideration.len() >= MAX_SHARES {
        return Err(CmdError::msg("Too many considerations."));
    }
    // E6a step 1 (A′): a consideration a `Held` escrow hold backs stands under a
    // surviving acceptance, and its author consented to exactly that amount. Changing
    // it would leave the acceptance binding money the surface no longer names; the
    // route out is a cancellation, which releases it.
    let backed = ctx.w.module_state::<EconomyStore>().is_some_and(|store| {
        let (claims, _) = crate::contract::qualifying_claims(&c, store.snapshot());
        c.consent
            .terms
            .consideration
            .iter()
            .enumerate()
            .any(|(i, x)| {
                x.account == account && x.amount != amount && claims.iter().any(|(_, k)| *k == i)
            })
    });
    if backed {
        return Err(CmdError::msg(format!(
            "Contract #{}: account #{account}'s consideration is backed by escrow that stands \
             under an acceptance and cannot be changed; @contract/cancel #{} releases it.",
            c.id, c.id
        )));
    }
    c.consent
        .terms
        .consideration
        .retain(|x| x.account != account);
    c.consent.terms.consideration.push(Consideration {
        from: owner.clone(),
        account,
        amount,
    });
    // The payer role defaults to the first consideration's owner, and stays whatever
    // `@contract/role` last said.
    if c.roles.payer.is_none() {
        c.roles.payer = Some(owner);
    }
    let id = save_open(ctx, c)?;
    let rendered = money_on(ctx, account, amount);
    info(
        ctx,
        format!("Contract #{id}: account #{account} escrows {rendered} at acceptance."),
    );
    Ok(())
}

fn build_deliver(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: @contract/deliver <id>=title <obj> <from>-><to> | deposit <obj> \
                         <vault> | collateral <obj> <from> | obligation <debit> <credit> <amount> \
                         every=<secs>";
    let mut c = editable(ctx, ctx.args.lhs())?;
    let rhs = ctx
        .args
        .rhs()
        .ok_or_else(|| CmdError::msg(USAGE))?
        .to_string();
    let (rest, opts) = parse_opts(&rhs, &["every", "grace"]);
    let mut words = rest.split_whitespace();
    let kind = words.next().unwrap_or("").to_ascii_lowercase();
    let deliverable = match kind.as_str() {
        "title" => {
            let object = asset_target(ctx, words.next().unwrap_or(""))?;
            let pair: String = words.collect::<Vec<_>>().join(" ");
            let (from, to) = split_arrow(&pair).ok_or_else(|| CmdError::msg(USAGE))?;
            let from =
                parse_principal(ctx, &from).ok_or_else(|| CmdError::msg("No such principal."))?;
            let to =
                parse_principal(ctx, &to).ok_or_else(|| CmdError::msg("No such principal."))?;
            Deliverable::TitleTransfer { object, from, to }
        }
        "deposit" => {
            let object = asset_target(ctx, words.next().unwrap_or(""))?;
            // The same draft-time viewability gate `/condition deposit` runs (A2).
            let vault = words
                .next()
                .and_then(parse_vault_id)
                .filter(|v| may_view_vault(ctx.w, ctx.actor, *v))
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
            let debit = words
                .next()
                .and_then(parse_account)
                .ok_or_else(|| CmdError::msg(USAGE))?;
            let credit = words
                .next()
                .and_then(parse_account)
                .ok_or_else(|| CmdError::msg(USAGE))?;
            let amount = words
                .next()
                .and_then(parse_amount)
                .ok_or_else(|| CmdError::msg(USAGE))?;
            if amount.is_zero() || amount.is_negative() {
                return Err(CmdError::msg("An obligation amount must be positive."));
            }
            let every = opts
                .get("every")
                .and_then(|s| s.parse::<i64>().ok())
                .filter(|n| *n > 0)
                .ok_or_else(|| CmdError::msg("every= must be a positive number of seconds."))?;
            let grace = opts
                .get("grace")
                .and_then(|s| s.parse::<u8>().ok())
                .unwrap_or(crate::DEFAULT_GRACE_PERIODS);
            // **Standing first, through the same `hidden_text` loop the escrow
            // builder runs** (E4c verify fold, D1).
            //
            // Before the fold this branch answered with three distinguishable
            // messages — "No such account.", the system source/sink text, and
            // silence on success — so naming an id and reading the reply told you
            // whether the account existed and whether it was a faucet, one command
            // at a time, from a draft nobody else ever sees.
            //
            // The standing an obligation deliverable needs is `@contract/pay`'s plus
            // one clause: **an account owned by a party of this contract**. A lease's
            // rent debits the tenant's wallet, which the landlord may not view, and
            // credits the landlord's, which the tenant may not view — so a bare
            // viewability gate would make the canonical lease undraftable by anybody
            // but staff. A party's name is already on the surface; a stranger's is
            // not, and a stranger's account is exactly what the oracle was probing
            // for.
            let known = |ctx: &Ctx<'_>, a: AccountId| -> bool {
                if may_view_account(ctx.w, ctx.actor, a) {
                    return true;
                }
                let Some(store) = ctx.w.module_state::<EconomyStore>() else {
                    return false;
                };
                let Some(acct) = store.snapshot().account(a) else {
                    return false;
                };
                c.consent
                    .parties
                    .iter()
                    .chain(std::iter::once(&c.issuer))
                    .any(|p| principal_key(p) == principal_key(&acct.owner))
            };
            let hidden = |ctx: &mut Ctx<'_>, detail: &str| -> CmdError {
                refuse_named(
                    ctx,
                    NamedResource::Account(debit),
                    [NamedResource::Account(credit)],
                    detail.to_string(),
                )
            };
            if !known(ctx, debit) || !known(ctx, credit) {
                return Err(CmdError::msg("No such account."));
            }
            let (debtor, creditor) = {
                let store = ctx
                    .w
                    .module_state::<EconomyStore>()
                    .ok_or_else(|| CmdError::msg("No economy."))?;
                let snap = store.snapshot();
                let (Some(d), Some(cr)) =
                    (snap.account(debit).cloned(), snap.account(credit).cloned())
                else {
                    return Err(hidden(ctx, "No such account."));
                };
                if matches!(d.owner, PrincipalId::System(_)) {
                    return Err(hidden(
                        ctx,
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
        return Err(CmdError::msg(format!(
            "At most {MAX_DELIVERABLES} deliverables."
        )));
    }
    c.consent.terms.deliverables.push(deliverable);
    let n = c.consent.terms.deliverables.len() - 1;
    let id = save_open(ctx, c)?;
    info(
        ctx,
        format!("Contract #{id}: deliverable {n} recorded ({kind})."),
    );
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
            return n
                .trim()
                .parse::<u32>()
                .ok()
                .filter(|p| *p <= PERMILLE)
                .map(Share::Permille);
        }
    }
    parse_amount(t)
        .filter(|a| !a.is_negative())
        .map(Share::Exact)
}

fn build_split(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: @contract/split <id>=<acct> <exact|N‰> [<acct> <share> …]";
    let mut c = editable(ctx, ctx.args.lhs())?;
    let rhs = ctx
        .args
        .rhs()
        .ok_or_else(|| CmdError::msg(USAGE))?
        .to_string();
    let shares = parse_shares(&rhs).ok_or_else(|| CmdError::msg(USAGE))?;
    if shares.is_empty() || shares.len() > MAX_SHARES {
        return Err(CmdError::msg(USAGE));
    }
    c.consent.terms.split = shares;
    let n = c.consent.terms.split.len();
    let id = save_open(ctx, c)?;
    info(
        ctx,
        format!("Contract #{id}: the split now has {n} share(s)."),
    );
    Ok(())
}

/// `<acct> <share> [<acct> <share> …]`, or the comma form `<acct>:<share>,…`.
fn parse_shares(s: &str) -> Option<Vec<PayoutShare>> {
    let mut out: Vec<PayoutShare> = Vec::new();
    if s.contains(':') {
        for part in s.split(',') {
            let (a, sh) = part.split_once(':')?;
            out.push(PayoutShare {
                to: parse_account(a)?,
                share: parse_share(sh)?,
            });
        }
        return Some(out);
    }
    let words: Vec<&str> = s.split_whitespace().collect();
    if words.is_empty() || !words.len().is_multiple_of(2) {
        return None;
    }
    for pair in words.chunks(2) {
        let (a, sh) = (pair.first()?, pair.get(1)?);
        out.push(PayoutShare {
            to: parse_account(a)?,
            share: parse_share(sh)?,
        });
    }
    Some(out)
}

fn build_remainder(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let mut c = editable(ctx, ctx.args.lhs())?;
    let rhs = ctx.args.rhs().unwrap_or("").to_string();
    let account = parse_account(&rhs).ok_or_else(|| CmdError::msg("No such account."))?;
    c.consent.terms.remainder_to = account;
    let id = save_open(ctx, c)?;
    info(
        ctx,
        format!("Contract #{id}: the floored remainder goes to account #{account}."),
    );
    Ok(())
}

/// `party:<principal>` | `cap:<capability>[:<scope>]` | `staff` | `canon:<kind>:<source>`
///
/// The canon source is **mandatory** (E4c verify fold, E2): `canon:scene.outcome`
/// with no source accepted every record of that kind, which is the broad match E5's
/// owner answer 20 exists to refuse — one front's outcome would have discharged
/// another front's contract.
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
        let (kind, source) = rest.split_once(':')?;
        if kind.is_empty() || source.is_empty() {
            return None;
        }
        return Some(AttestorSet::Canon {
            kind: SmolStr::new(kind),
            source: Some(SmolStr::new(source)),
        });
    }
    None
}

fn build_condition(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: @contract/condition <id>=attest <who> | deposit <obj> <vault> | \
                         title <obj> <principal> | fact <provider> <kind> <id> | \
                         before <secs|ic:secs> | after <n> <delay>   \
                         (<who> = party:<principal> | cap:<capability>:<scope> | staff | \
                         canon:<kind>:<source>; `title` takes a bare <principal>)";
    let mut c = editable(ctx, ctx.args.lhs())?;
    let rhs = ctx
        .args
        .rhs()
        .ok_or_else(|| CmdError::msg(USAGE))?
        .to_string();
    let mut words = rhs.split_whitespace();
    let kind = words.next().unwrap_or("").to_ascii_lowercase();
    let condition = match kind.as_str() {
        "attest" => {
            let spec: String = words.collect::<Vec<_>>().join(" ");
            TypedCondition::Attestation {
                by: parse_attestor(ctx, &spec).ok_or_else(|| CmdError::msg(USAGE))?,
            }
        }
        // E4b2 verify fold, A2: the vault and the object are checked for
        // **viewability at draft**, so an unviewable one and a nonexistent one render
        // identically here and not only at `/offer`. Without it a draft was a probe
        // that took two commands instead of one.
        "deposit" => {
            let object = asset_target(ctx, words.next().unwrap_or(""))?;
            let vault = words
                .next()
                .and_then(parse_vault_id)
                .filter(|v| may_view_vault(ctx.w, ctx.actor, *v))
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
            let provider = words
                .next()
                .ok_or_else(|| CmdError::msg(USAGE))?
                .to_string();
            let fkind = words
                .next()
                .ok_or_else(|| CmdError::msg(USAGE))?
                .to_string();
            let fid = words
                .next()
                .ok_or_else(|| CmdError::msg(USAGE))?
                .to_string();
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
                .ok_or_else(|| {
                    CmdError::msg("Bad deadline (a POSITIVE number of seconds, or ic:<seconds>).")
                })?;
            TypedCondition::Before { deadline: d }
        }
        "after" => {
            let anchor = words
                .next()
                .and_then(|s| s.parse::<usize>().ok())
                .ok_or_else(|| CmdError::msg(USAGE))?;
            let raw = words.next().unwrap_or("");
            let (clock, secs) = match raw.strip_prefix("ic:") {
                Some(rest) => (EconomicClock::WorldIc, rest),
                None => (EconomicClock::RealUtc, raw),
            };
            let delay = secs
                .parse::<i64>()
                .ok()
                .filter(|n| *n > 0)
                .ok_or_else(|| CmdError::msg(USAGE))?;
            TypedCondition::After {
                condition: anchor,
                delay,
                clock,
            }
        }
        _ => return Err(CmdError::msg(USAGE)),
    };
    if c.consent.conditions.len() >= MAX_CONDITIONS {
        return Err(CmdError::msg(format!(
            "At most {MAX_CONDITIONS} conditions."
        )));
    }
    c.consent.conditions.push(condition);
    // `satisfied` is parallel to `conditions` by construction, everywhere.
    c.satisfied.push(None);
    let n = c.consent.conditions.len() - 1;
    let id = save_open(ctx, c)?;
    info(
        ctx,
        format!("Contract #{id}: condition {n} recorded ({kind})."),
    );
    Ok(())
}

fn build_deadline(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str =
        "Usage: @contract/deadline <id>=<secs|ic:secs> [refund|forfeit=<acct>|dispute]";
    let mut c = editable(ctx, ctx.args.lhs())?;
    let rhs = ctx
        .args
        .rhs()
        .ok_or_else(|| CmdError::msg(USAGE))?
        .to_string();
    let (rest, opts) = parse_opts(&rhs, &["forfeit"]);
    let mut words = rest.split_whitespace();
    let deadline: EconomicDeadline = words
        .next()
        .and_then(|s| parse_deadline(ctx, s))
        .ok_or_else(|| {
            CmdError::msg("Bad deadline (a POSITIVE number of seconds, or ic:<seconds>).")
        })?;
    let on_expiry = match (
        words.next().map(str::to_ascii_lowercase).as_deref(),
        opts.get("forfeit"),
    ) {
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
    let id = save_open(ctx, c)?;
    info(
        ctx,
        format!(
            "Contract #{id}: it lapses at {} on {:?}.",
            deadline.at, deadline.clock
        ),
    );
    Ok(())
}

fn build_visibility(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: @contract/visibility <id>=parties|staff|public|audience <principal> … [announce=yes]";
    let mut c = editable(ctx, ctx.args.lhs())?;
    let raw = ctx
        .args
        .rhs()
        .ok_or_else(|| CmdError::msg(USAGE))?
        .to_string();
    let (rhs, opts) = parse_opts(&raw, &["announce"]);
    let announce_issuer = opts
        .get("announce")
        .is_some_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "yes" | "true" | "1"));
    let mut words = rhs.split_whitespace();
    let word = words.next().unwrap_or("").to_ascii_lowercase();
    let visibility = match word.as_str() {
        "parties" => EconomicVisibility::Parties,
        "staff" => EconomicVisibility::Staff,
        "public" => EconomicVisibility::Public,
        "audience" => {
            let mut members: Vec<PrincipalId> = Vec::new();
            for w in words {
                let p =
                    parse_principal(ctx, w).ok_or_else(|| CmdError::msg("No such principal."))?;
                if members
                    .iter()
                    .all(|m| principal_key(m) != principal_key(&p))
                {
                    members.push(p);
                }
            }
            if members.is_empty() || members.len() > MAX_AUDIENCE {
                return Err(CmdError::msg(format!(
                    "An audience names 1-{MAX_AUDIENCE} principals besides the parties."
                )));
            }
            EconomicVisibility::Audience {
                members,
                announce_issuer,
            }
        }
        _ => return Err(CmdError::msg(USAGE)),
    };
    let label = visibility.label();
    let audience = matches!(visibility, EconomicVisibility::Audience { .. });
    c.consent.visibility = visibility;
    let id = save_open(ctx, c)?;
    info(ctx, format!("Contract #{id}: visibility is now {label}."));
    if audience {
        info(
            ctx,
            format!(
                "The audience sees a notice — name, state, window and the kinds of condition — \
                 and nothing about anyone's money or roles. The issuer is {}named on it.",
                if announce_issuer { "" } else { "NOT " }
            ),
        );
    }
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

/// The refusal an NPC adjudicator gets, so the surface says why.
const ADJUDICATOR_IS_NOT_A_PRINCIPAL: &str = "The adjudicator's word settles this contract, so it is never a process: name a player, a \
     capability, staff, or a canon source.";

/// **Rule 2 (08i §4), the adjudicator half** — the mirror of `mush-scenes`'
/// `is_sealable_principal`.
///
/// A named adjudicator *replaces* every condition's own attestor set
/// (08e answer 6), so naming one is the single largest authority transfer the
/// `@contract/*` surface can make. An `ObjKind::Npc` is driven by
/// `NpcModule::on_complete` — and, in a later cut, by an LLM controller — so
/// conferring that authority on one would hand a settlement to a process rather
/// than a principal.
///
/// E6b closed the sealer half; this was the open half (08j S5):
/// `parse_principal`'s object arm is a bare `match_object(..).map(Object)` with
/// no kind test, and `attestor_satisfiable` answers `true` for every
/// `AttestorSet::Party(_)`, so nothing anywhere refused it. Only the object arm
/// is constrained — `Staff`, `Capability` and `Canon` name no object, and an
/// organization is a principal with members, not a process.
///
/// Game-neutral: `ObjKind` is `mush-core`'s, and the rule is about who may hold
/// settlement authority, not about any ruleset's content.
fn is_adjudicable_principal(w: &World, set: &AttestorSet) -> bool {
    match set {
        AttestorSet::Party(PrincipalId::Object(id)) => {
            w.get(*id).is_some_and(|o| o.kind == ObjKind::Player)
        }
        AttestorSet::Party(_)
        | AttestorSet::Staff
        | AttestorSet::Capability { .. }
        | AttestorSet::Canon { .. } => true,
    }
}

fn build_adjudicator(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: @contract/adjudicator <id>=<who>|none";
    let mut c = editable(ctx, ctx.args.lhs())?;
    let rhs = ctx
        .args
        .rhs()
        .ok_or_else(|| CmdError::msg(USAGE))?
        .trim()
        .to_string();
    c.consent.adjudicator = if rhs.eq_ignore_ascii_case("none") {
        None
    } else {
        let set = parse_attestor(ctx, &rhs).ok_or_else(|| CmdError::msg(USAGE))?;
        if !is_adjudicable_principal(ctx.w, &set) {
            return Err(CmdError::msg(ADJUDICATOR_IS_NOT_A_PRINCIPAL));
        }
        Some(set)
    };
    let named = c.consent.adjudicator.is_some();
    let id = save_open(ctx, c)?;
    if named {
        info(
            ctx,
            format!("Contract #{id}: the adjudicator is named, and only they may attest."),
        );
    } else {
        info(
            ctx,
            format!("Contract #{id}: no adjudicator; each condition's own attestor set applies."),
        );
    }
    Ok(())
}

fn build_evidence(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str =
        "Usage: @contract/evidence <id>=attest,deposit,title,fact,before,after   (or: any)";
    let mut c = editable(ctx, ctx.args.lhs())?;
    let rhs = ctx
        .args
        .rhs()
        .ok_or_else(|| CmdError::msg(USAGE))?
        .trim()
        .to_ascii_lowercase();
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
    let id = save_open(ctx, c)?;
    if n == 0 {
        info(
            ctx,
            format!("Contract #{id}: any kind of evidence is permitted."),
        );
    } else {
        info(
            ctx,
            format!("Contract #{id}: {n} permitted kind(s) of evidence; anything else is refused."),
        );
    }
    Ok(())
}

fn build_outcome(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: @contract/outcome <id>=<name> when=<n[,n]> pay=<acct>:<share>[,…] \
                         [titles=<n[,n]>] [expense=none|retained|returned|reimbursed]";
    let mut c = editable(ctx, ctx.args.lhs())?;
    let rhs = ctx
        .args
        .rhs()
        .ok_or_else(|| CmdError::msg(USAGE))?
        .to_string();
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
    let id = save_open(ctx, c)?;
    info(
        ctx,
        format!("Contract #{id}: outcome \"{name}\" recorded ({n} in the table)."),
    );
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
    let rhs = ctx
        .args
        .rhs()
        .ok_or_else(|| CmdError::msg(USAGE))?
        .to_string();
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
    let id = save_open(ctx, c)?;
    info(
        ctx,
        format!(
            "Contract #{id}: the {role} role is recorded. Roles are attribution, never authority."
        ),
    );
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
    info(
        ctx,
        format!(
            "Contract #{id}: the roster is {}.",
            if locked { "locked" } else { "open" }
        ),
    );
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
    if let Err(e) = check_offer(ctx, &c) {
        return Err(offer_refusal(ctx, &c, &e));
    }
    let id = freeze_and_offer(ctx, c);
    info(
        ctx,
        format!("Contract #{id} is offered; its consent surface is frozen."),
    );
    ctx.gate_on_durable_ack();
    Ok(())
}

/// How many live contracts a party may be on at once, as a refusal the seam can
/// carry (the command renders it with §9's own words).
const PARTY_CAP_REFUSAL: &str = "a party is already on the most live contracts allowed";

/// Everything that must be true of a draft before it is offered — E4b2's gate plus
/// E6a step 1's re-stamp rule — against the live store, with no side effect. The
/// shared core of `@contract/offer` and the programmatic `offer_contract` seam.
///
/// E4c verify fold, E1 + E2: the canon-outcome table is checked HERE and not inside
/// `check_offerable`, because a live contract must never be stranded by a change to
/// composition-root data. A draft, by contrast, can still be fixed.
///
/// **The re-stamp rule (A′).** An acceptance that survived an amendment is bound to
/// the surface its author wrote; it may be carried onto the amended surface only
/// while every consideration that party owes is still claimed by a `Held` escrow
/// hold ([`crate::contract::party_escrow_backed`]). Otherwise the offer is refused
/// ([`ContractError::AcceptanceUnbacked`]): a new own consideration, or one whose
/// hold was broken, is money the acceptance never covered, and `@contract/cancel`
/// is the route that releases what still stands.
pub(crate) fn check_offer(ctx: &Ctx<'_>, c: &EconomicContract) -> Result<(), ContractError> {
    let store = ctx
        .w
        .module_state::<EconomyStore>()
        .ok_or(ContractError::Infeasible("no economy"))?;
    let outcomes = store.canon_outcome_kinds();
    crate::contract::check_structure(c, &outcomes)?;
    check_offerable(c, store.snapshot())?;
    // §9's per-party cap, counted over the parties this offer would bind.
    for p in &c.consent.parties {
        let live = store
            .snapshot()
            .contracts()
            .filter(|x| x.id != c.id && x.state.is_live())
            .filter(|x| x.consent.parties.contains(p))
            .count();
        if live >= MAX_LIVE_CONTRACTS_PER_PARTY {
            return Err(ContractError::TooLarge(PARTY_CAP_REFUSAL));
        }
    }
    if c.acceptances
        .iter()
        .any(|a| !crate::contract::party_escrow_backed(c, store.snapshot(), &a.party))
    {
        return Err(ContractError::AcceptanceUnbacked);
    }
    Ok(())
}

/// Render an offer refusal for the command path (the §9 cap keeps its own words).
fn offer_refusal(ctx: &mut Ctx<'_>, c: &EconomicContract, e: &ContractError) -> CmdError {
    match e {
        ContractError::TooLarge(PARTY_CAP_REFUSAL) => CmdError::msg(format!(
            "A party is already on {MAX_LIVE_CONTRACTS_PER_PARTY} live contracts (the cap is \
             {MAX_LIVE_CONTRACTS_PER_PARTY})."
        )),
        ContractError::AcceptanceUnbacked => CmdError::msg(format!(
            "Contract #{}: a surviving acceptance is no longer backed by escrow for every \
             consideration its party owes, so it cannot be re-offered as it stands. \
             @contract/cancel #{} releases what is still reserved.",
            c.id, c.id
        )),
        other => contract_refusal(ctx, c, other),
    }
}

/// Freeze the surface and offer it: the hash, the **re-stamped** survivors, the
/// state, the roster lock, the doc, the notices. The shared core of
/// `@contract/offer` and the programmatic seam; [`check_offer`] has already said
/// yes.
///
/// From here on `save` never re-hashes: the hash IS what the acceptances bind to,
/// and a mutation after Offer must show up as a mismatch. A surviving acceptance is
/// re-stamped onto the new hash — consent by authorship (A′): its party wrote the
/// terms, and `check_offer` proved their escrow still backs every consideration they
/// owe.
///
/// §5's builder block is "Draft only". The E6 extension (08e answers 5/7) that lets
/// an `Offered` contract still carry an unfrozen party is therefore an explicit act —
/// `@contract/roster <id>=open` — and never the default, so "the surface changed
/// after Offer" stays the refusal it has to be.
pub(crate) fn freeze_and_offer(ctx: &mut Ctx<'_>, mut c: EconomicContract) -> ContractId {
    c.consent_hash = consent_hash(&c.issuer, &c.consent);
    let hash = c.consent_hash;
    for a in &mut c.acceptances {
        a.consent_hash = hash;
    }
    c.state = ContractState::Offered;
    c.roster_locked = true;
    let (id, parties) = (c.id, c.consent.parties.clone());
    let standing: Vec<PrincipalId> = c.acceptances.iter().map(|a| a.party.clone()).collect();
    // E6a step 1 verify fold (M5): a re-offer can be **already unanimous**.
    //
    // The last acceptance activates (D-E4-10), and A′ deliberately keeps the author's:
    // a one-party live contract — a run board's own listing — amended and re-offered
    // comes back `Offered` with `fully_accepted()` already true, and nothing else in
    // the world promotes it. It sat `Offered` forever, its escrow reserved, waiting
    // for an acceptance that had already happened. Activation belongs wherever a
    // surface becomes fully accepted, and this is one of those places.
    let unanimous = c.fully_accepted();
    save(ctx, c);
    for p in &parties {
        let already = standing
            .iter()
            .any(|s| principal_key(s) == principal_key(p));
        let msg = if already {
            format!(
                "Contract #{id} has been re-offered; your acceptance stands (@contract/show #{id})."
            )
        } else {
            format!("Contract #{id} has been offered to you (@contract/show #{id}).")
        };
        notify(ctx, p, &msg);
    }
    if unanimous {
        activate(ctx, id);
    }
    id
}

/// Render a [`ContractError`] behind the existence-hiding rule (§4.5, E4b2 verify
/// fold, A2).
///
/// The caller has already proved the *contract* is viewable, but the error names
/// other people's resources: `no such vault: 7` and `no such account: 12` are answers
/// about a vault or a wallet the actor may have no standing over at all, and rendering
/// them raw made `@contract/offer` a probe — build a draft naming any id, offer it,
/// and read the reply. So every refusal goes through the same
/// [`crate::commands::refuse`] the money verbs use, against the resource the error
/// names *and* (for a refusal that is a policy answer about them) every resource the
/// surface names.
fn contract_refusal(ctx: &mut Ctx<'_>, c: &EconomicContract, e: &ContractError) -> CmdError {
    match e {
        // One answer for "no such", "not yours to see", "not a party", "not the
        // issuer" (§4.5).
        ContractError::NoStanding => return CmdError::msg(NO_SUCH),
        // M4 (ii): a `System` payer or party renders as the ACCOUNT's own
        // existence-hiding text, never a faucet-shaped message — a faucet's existence
        // is exactly what a probe would be after.
        ContractError::SystemParty => return CmdError::msg("No such account."),
        _ => {}
    }
    let named = e.named_resources();
    let surface = if e.concerns_named_subjects() {
        ContractError::surface_resources(c)
    } else {
        Vec::new()
    };
    refuse_named(
        ctx,
        NamedResource::Contract(c.id),
        named.iter().chain(surface.iter()).copied(),
        format!("Contract #{}: {e}", c.id),
    )
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
    if c.state != ContractState::Offered || c.pending.is_some() {
        return Err(CmdError::msg(format!(
            "Contract #{} is not open for acceptance.",
            c.id
        )));
    }
    if !c.consent_intact() {
        return Err(CmdError::msg(format!(
            "Contract #{}'s consent surface has changed since it was offered; it must be re-offered.",
            c.id
        )));
    }
    let party = acting_party(ctx, &c, "contract.accept").ok_or_else(|| CmdError::msg(NO_SUCH))?;
    let id = c.id;
    match accept_core(ctx, c.clone(), party) {
        Ok(AcceptOutcome::Accepted { escrow, .. }) => {
            let escrowed: Vec<String> = escrow.iter().map(|h| format!("#{h}")).collect();
            if escrowed.is_empty() {
                info(ctx, format!("Contract #{id}: accepted."));
            } else {
                info(
                    ctx,
                    format!(
                        "Contract #{id}: accepted; escrow {} placed.",
                        escrowed.join(", ")
                    ),
                );
                info(
                    ctx,
                    "That reservation belongs to the contract now: it is released by its refund \
                     and captured by its settlement, and by nothing else."
                        .to_string(),
                );
            }
            Ok(())
        }
        // D-E4-11: no acceptance is recorded, the contract stays `Offered`, and the
        // party re-runs accept once the approval is collected.
        Ok(AcceptOutcome::NeedsApproval(request)) => {
            info(
                ctx,
                format!(
                    "Approval required before you can escrow that: request #{request}. Nothing \
                     was accepted — run @contract/accept #{id} again once it is approved."
                ),
            );
            Ok(())
        }
        Err(ContractError::AlreadyAccepted) => Err(CmdError::msg(format!(
            "Contract #{id} is already accepted on your behalf."
        ))),
        Err(ContractError::Refused(e)) => {
            let first = c
                .consent
                .terms
                .consideration
                .iter()
                .find(|x| principal_key(&x.from) == principal_key(&acting_party_or_self(ctx, &c)))
                .map_or(AccountId(0), |x| x.account);
            Err(refuse(
                ctx,
                NamedResource::Contract(id),
                &[NamedResource::Account(first)],
                &e,
            ))
        }
        Err(e) => Err(contract_refusal(ctx, &c, &e)),
    }
}

/// The party the actor speaks for on `c`, or the actor themself when nobody — the
/// refusal renderer's `also` list needs *some* account to hide behind.
fn acting_party_or_self(ctx: &Ctx<'_>, c: &EconomicContract) -> PrincipalId {
    acting_party(ctx, c, "contract.accept").unwrap_or(PrincipalId::Object(ctx.actor))
}

/// What one acceptance did (E6a step 1's seam, and the command behind it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AcceptOutcome {
    /// The acceptance is recorded, its escrow placed, and the contract activated when
    /// it was the last one.
    Accepted {
        /// The escrow holds this acceptance placed (none for a party owing nothing).
        escrow: Vec<HoldId>,
        /// Whether this was the last acceptance, so the contract came into force.
        activated: bool,
    },
    /// The payer's account rule fired: **no acceptance is recorded** and nothing is
    /// placed (D-E4-11). Re-run the accept once the request is approved.
    NeedsApproval(crate::ids::ApprovalRequestId),
}

/// The acceptance itself — the shared core of `@contract/accept` and the
/// programmatic `accept_contract` seam, under the accepting party's **own live
/// authority** (D-E4-4).
///
/// The step does two independent things, and D-E4-4 requires them to be independent:
/// it records an acceptance bound to the exact consent hash, and it places that
/// party's consideration as an escrow hold through the ordinary `post` seam. The
/// hold is a debit-authority plan (invariant 25), so the account's own rules apply —
/// including its [`crate::ApprovalRule`], which returns `NeedsApproval` and records
/// **no acceptance at all** (D-E4-11).
///
/// **The acceptance rides the escrow post's own batch** (E4b2 verify fold, B2). The
/// record is built BEFORE the post, with the hold ids the engine is about to mint
/// (`peek_next_hold` is the very floor `validate` allocates from, one per `Place`),
/// and handed to `post_with_companions` — so the journal chunk, the hold projections
/// and the contract doc that names them are one durable write. A separate later
/// write could drop, leaving escrow reserved to a contract whose record does not
/// list it: money nothing in the world could release.
pub(crate) fn accept_core(
    ctx: &mut Ctx<'_>,
    c: EconomicContract,
    party: PrincipalId,
) -> Result<AcceptOutcome, ContractError> {
    let actor = PrincipalId::Object(ctx.actor);
    accept_core_as(ctx, c, party, actor)
}

/// [`accept_core`] with the **escrow plan's actor** named explicitly (E6b, B4).
///
/// For every command path the actor is the person typing, exactly as before. DL-11's
/// programmatic accept passes the accepting principal itself, so the hold posts under
/// that principal's own debit authority rather than under a bystander's.
pub(crate) fn accept_core_as(
    ctx: &mut Ctx<'_>,
    c: EconomicContract,
    party: PrincipalId,
    actor: PrincipalId,
) -> Result<AcceptOutcome, ContractError> {
    if c.state != ContractState::Offered || c.pending.is_some() {
        return Err(ContractError::State(c.id));
    }
    if !c.consent_intact() {
        return Err(ContractError::ConsentMismatch);
    }
    if c.accepted_by(&party) {
        return Err(ContractError::AlreadyAccepted);
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
    let now = ctx.w.now();
    let mut c = c;
    c.acceptances.push(Acceptance {
        party: party.clone(),
        at: now,
        consent_hash: c.consent_hash,
    });
    let predicted: Vec<HoldId> = {
        let floor = ctx
            .w
            .module_state::<EconomyStore>()
            .map_or(0, EconomyStore::peek_next_hold);
        (0..considerations.len())
            .map(|i| HoldId(floor.saturating_add(u64::try_from(i).unwrap_or(u64::MAX))))
            .collect()
    };
    if !considerations.is_empty() {
        c.escrow.extend(predicted.iter().copied());
    }
    // 08e answers 5/7: the first acceptance freezes the roster snapshot and the
    // exact shares. From here a substitution is an amendment.
    c.roster_locked = true;
    let id = c.id;
    let placed: Vec<HoldId> = if considerations.is_empty() {
        // Nothing to post, so nothing to ride: the ordinary container write.
        save(ctx, c.clone());
        Vec::new()
    } else {
        let mut plan = EconomyPlan::new(
            cmd_key(ctx),
            actor,
            EconomicCause::Command {
                serial: ctx.w.next_correlation_id(),
            },
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
        let mut committed = c.clone();
        committed.revision = committed.revision.saturating_add(1);
        match api::post_with_companions(ctx, &plan, PostAuth::World, vec![committed]) {
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
                return Err(ContractError::Infeasible(
                    "that escrow was already placed; try again",
                ));
            }
            // Nothing posted, so the companion was never written either.
            Ok(PostOutcome::NeedsApproval(request)) => {
                return Ok(AcceptOutcome::NeedsApproval(request));
            }
            Err(e) => return Err(ContractError::Refused(e)),
        }
    };
    // Belt and braces: if the engine somehow minted ids other than the predicted run,
    // heal the record rather than leave it naming holds that are not this escrow.
    if !placed.is_empty() && placed != predicted {
        tracing::warn!(
            contract = id.0,
            "economy: escrow hold ids differed from the predicted run; healing the contract record"
        );
        let mut healed = c.clone();
        healed.escrow.retain(|h| !predicted.contains(h));
        healed.escrow.extend(placed.iter().copied());
        c = healed.clone();
        save(ctx, healed);
    }
    let full = c.fully_accepted();
    if full {
        activate(ctx, id);
    }
    ctx.gate_on_durable_ack();
    Ok(AcceptOutcome::Accepted {
        escrow: placed,
        activated: full,
    })
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
    let Some(mut c) = live_contract(ctx, id) else {
        return;
    };
    if c.state != ContractState::Offered || !c.fully_accepted() {
        return;
    }
    // E6b independent verify fold (M1): **the same object cannot be two live
    // contracts' deposit deliverable.** `check_offerable` refuses it at the offer,
    // but two offers can both pass that gate before either activates — the lock is
    // taken at `Active` — so the last gate is here, where the lock is about to be
    // taken. Activating anyway would make one hand-over satisfy both escrows.
    // Nothing is stranded: the contract goes `Disputed` with its escrow frozen, which
    // is exactly what an activation that cannot place its pledges already does.
    let contested: Option<(ObjId, ContractId)> = {
        let Some(store) = ctx.w.module_state::<EconomyStore>() else {
            return;
        };
        let snapshot = store.snapshot();
        c.consent
            .conditions
            .iter()
            .filter_map(|con| match con {
                crate::contract::TypedCondition::DepositInto { object, .. } => Some(*object),
                _ => None,
            })
            .find_map(|object| {
                snapshot
                    .deposit_lock_of(object)
                    .filter(|held| *held != id)
                    .map(|held| (object, held))
            })
    };
    if let Some((object, held)) = contested {
        dispute_with(
            ctx,
            id,
            format!(
                "contract #{id} names object #{} as its deposit, and live contract #{held} already \
                 does: one delivery cannot pay two agreements. Its escrow is frozen until a human \
                 decides",
                object.0
            ),
        );
        return;
    }
    c.state = ContractState::Active;
    c.deposits = record_deposits(ctx, &c);
    save(ctx, c.clone());
    notify_parties(
        ctx,
        &c,
        &format!("Contract #{id} is active: every party has accepted."),
    );

    let ops = crate::contract::activation_asset_ops(&c);
    if !ops.is_empty() {
        let mut plan = EconomyPlan::new(
            activate_key(&c),
            c.issuer.clone(),
            EconomicCause::Internal {
                kind: SmolStr::new_static("contract"),
                id: id.0,
            },
            Vec::new(),
        );
        plan.description = format!("activation of contract {id}");
        plan.visibility = c.consent.visibility.clone();
        plan.asset_ops = ops;
        let owned = c.clone();
        let outcome = {
            let auth = {
                let Some(store) = ctx.w.module_state::<EconomyStore>() else {
                    return;
                };
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
    // E4c verify fold, E4a: canon evidence is **edge-triggered**, and the edge for a
    // record that landed while this contract was still `Offered` is gone. Re-scan
    // what the world has announced since the economy booted, now that there is a live
    // contract to answer. It records nothing twice — a filled slot is skipped — and
    // it may settle in this same step, which is the point.
    crate::attest::rescan_canon_tail(ctx, crate::attest::RescanTrigger::Activation);
}

/// Record what is already deposited into this contract's named vaults, and who put
/// it there — from the **journal's** last-custody map, never from a claim (§5).
fn record_deposits(ctx: &Ctx<'_>, c: &EconomicContract) -> Vec<DepositRecord> {
    let Some(store) = ctx.w.module_state::<EconomyStore>() else {
        return Vec::new();
    };
    let mut out: Vec<DepositRecord> = Vec::new();
    for condition in &c.consent.conditions {
        let TypedCondition::DepositInto { object, vault } = condition else {
            continue;
        };
        let Some(mark) = store.last_custody(*object) else {
            continue;
        };
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
    let Some(c) = live_contract(ctx, id) else {
        return;
    };
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
        let minted = ctx
            .w
            .module_state_mut::<EconomyStore>()
            .and_then(EconomyStore::alloc_obligation);
        let Some(oid) = minted else {
            // H4 (E4b2-D33): an exhausted allocator is not a reason to lose the
            // world — but a lease that is active with no rent record is a fact a
            // person has to hear. Level 2, on the contract, deduped by subject.
            tracing::error!(
                contract = id.0,
                "economy: the obligation allocator is exhausted; a lease has no rent record"
            );
            let now = ctx.w.now();
            let _ = crate::sweep::page(
                ctx,
                crate::escalation::EscalationKind::AllocatorExhausted,
                crate::escalation::EscalationSubject::Contract(id),
                None,
                format!(
                    "contract #{id} activated as a lease but its rent could not be recorded: \
                     the obligation allocator is exhausted, so the lease is running with no \
                     rent record"
                ),
                vec![format!("@contract/show #{id}")],
                now,
            );
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
            suspensions: std::collections::BTreeSet::new(),
            revision: 1,
        };
        api::put_obligation(ctx, o);
        let meta = ctx
            .w
            .module_state::<EconomyStore>()
            .map(EconomyStore::meta_doc);
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
                    !matches!(
                        o.status,
                        ObligationStatus::Cancelled | ObligationStatus::Satisfied
                    )
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
    ctx.w
        .module_state::<EconomyStore>()?
        .snapshot()
        .contract(id)
        .cloned()
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
        return Err(CmdError::msg(format!(
            "Contract #{} has no attestable condition {n}.",
            c.id
        )));
    };
    // **An attestation is written once** (E4b2 verify fold, M5). Overwriting a filled
    // slot re-stamped `at`, which is the anchor an `After` condition counts its delay
    // from — so a party could push a consented settlement delay forward indefinitely
    // by re-attesting, one command at a time, and the contract would never mature.
    // The one exception is an attestor who no longer qualifies live: their record has
    // stopped counting at re-verification anyway, so replacing it is the difference
    // between a contract that can settle and one that is stuck.
    let filled = c.satisfied.get(n).and_then(|s| s.as_ref()).cloned();
    let me = PrincipalId::Object(ctx.actor);
    let auth = api::WorldAuth { w: ctx.w };
    let qualifies = |set: &AttestorSet| -> bool {
        use crate::engine::AuthEval;
        auth.authorize_attestor(&me, set)
    };
    if let Some(recorded) = &filled {
        let set = c.consent.adjudicator.clone().unwrap_or_else(|| by.clone());
        let stands = match &recorded.by {
            AttestorRef::Principal(p) => {
                use crate::engine::AuthEval;
                matches!(&set, AttestorSet::Party(want) if want == p)
                    || auth.authorize_attestor(p, &set)
            }
            // A canon record, a fact, the clock, an observation and a staff
            // resolution are not people; nothing about them stops qualifying, so
            // they stand.
            AttestorRef::Canon { .. }
            | AttestorRef::Fact(_)
            | AttestorRef::Clock
            | AttestorRef::Observed { .. }
            | AttestorRef::StaffOverride { .. } => true,
        };
        if stands {
            return Err(CmdError::msg(format!(
                "Contract #{}: condition {n} is already attested.",
                c.id
            )));
        }
    }
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
                return Err(CmdError::msg(format!(
                    "Condition {n} is not yours to attest."
                )));
            }
        }
    }
    // **Whose attestation is it?** (A3) When the set names a `Party` and the actor
    // merely acts for that party — an org's capability holder speaking for
    // `org:Ironhold` — the record names the **party**, with the human in `source`.
    // Recording the individual instead would make `attestation_matches` (which
    // compares the recorded principal to the named party, and deliberately does not
    // re-qualify — a party attestation stands once given) fail forever, so an
    // org-attested condition could be attested and then never settle.
    let recorded_by = match (&c.consent.adjudicator, &by) {
        (None, AttestorSet::Party(p)) if p != &me => p.clone(),
        _ => me,
    };
    let now = ctx.w.now();
    let mut c = c;
    if c.satisfied.len() < c.consent.conditions.len() {
        c.satisfied.resize(c.consent.conditions.len(), None);
    }
    if let Some(slot) = c.satisfied.get_mut(n) {
        *slot = Some(Attestation {
            by: AttestorRef::Principal(recorded_by),
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
    Posted(crate::ids::TransactionId),
    /// Re-verification (or the plan) refused; nothing moved.
    ///
    /// It carries the resources the detail **names** (E4b2 verify fold, A6) as well
    /// as the text, because the text goes to a party: `no usable account: 12` and
    /// `hold 7 is no longer held` are answers about somebody else's wallet, and a
    /// runner who agrees a settlement should not learn from the refusal that the
    /// Johnson's account has been frozen. `refusal_text` collapses those.
    Refused(String, Vec<NamedResource>),
    /// Nothing to do: the contract is not ready, or the outcome table says nothing
    /// happened yet.
    NotReady,
}

/// Render a settlement refusal for a **party** (A6): the detail when every resource
/// it names is one they may view, and otherwise the fact that it failed and that a
/// human has been told — which is all they are owed and all they need.
fn refusal_text(ctx: &Ctx<'_>, detail: &str, named: &[NamedResource]) -> String {
    if named.iter().any(|r| is_hidden(ctx, *r)) {
        "it could not settle; staff have been paged".to_string()
    } else {
        detail.to_string()
    }
}

/// Move a live contract to `ReadyToSettle` if its conditions now hold, and settle it
/// if `settle_now`.
pub(crate) fn promote_and_settle(ctx: &mut Ctx<'_>, id: ContractId, settle_now: bool) -> Settled {
    promote_and_settle_with(ctx, id, settle_now, Vec::new())
}

/// [`promote_and_settle`] with **companion container docs that ride the settlement
/// post's own batch** (E4b2 verify fold, B2's mechanism; E4c's canon observer is the
/// caller).
///
/// The observer records an attestation and settles on it in one step. Handing the
/// attested record to the post means the journal append that paid the split and the
/// evidence that justified it are one durable write: a batch that carries the money
/// can never fail to carry the reason.
pub(crate) fn promote_and_settle_with(
    ctx: &mut Ctx<'_>,
    id: ContractId,
    settle_now: bool,
    companions: Vec<EconomicContract>,
) -> Settled {
    let Some(c) = live_contract(ctx, id) else {
        return Settled::NotReady;
    };
    if !matches!(
        c.state,
        ContractState::Active | ContractState::ReadyToSettle
    ) {
        return Settled::NotReady;
    }
    // A contract on its way out does not settle on the way (M2+M3): the unwind's plan
    // may already be durable, and paying the split out on top of it would spend the
    // escrow twice.
    if c.pending.is_some() {
        return Settled::NotReady;
    }
    let verdict = {
        let Some(store) = ctx.w.module_state::<EconomyStore>() else {
            return Settled::NotReady;
        };
        let providers = store.providers();
        let custody = WorldCustody {
            w: ctx.w,
            providers,
        };
        let facts = WorldFacts {
            w: ctx.w,
            providers: store.facts(),
        };
        let world = api::WorldAuth { w: ctx.w };
        let gate = store.key_gate();
        let env = ValidateEnv {
            keys: &gate,
            auth: &world,
            custody: &custody,
            facts: &facts,
            now: ctx.w.now(),
            ic_now: ctx.w.game_now_unix(),
            next_hold_id: store.peek_next_hold(),
            spent: std::collections::BTreeMap::new(),
        };
        readiness(&c, store.snapshot(), &env)
    };
    match verdict {
        Readiness::NotYet => return Settled::NotReady,
        // A contract that cannot settle at all as it stands is a decision, not a wait
        // (§3.5: an ambiguous outcome is `Disputed`). Retrying it every thirty seconds
        // would freeze the escrow indefinitely and tell nobody.
        Readiness::Undecidable(e) => {
            let named = e.named_resources();
            return note_settle_failure(ctx, id, &e.to_string(), &named, true);
        }
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
    settle_with(ctx, id, companions)
}

/// Attempt one settlement of a `ReadyToSettle` (or `Active`) contract.
///
/// The plan is built against the live world and re-verifies every gating condition on
/// the way (E4a verify fold, finding 6), the outcome is resolved from the same world,
/// and the post runs under a [`ContractAuth`] bounded to exactly that outcome. A
/// failure leaves the contract `ReadyToSettle` with a loud log and one more
/// `settle_attempts`; at [`MAX_SETTLE_ATTEMPTS`] it is `Disputed` and paged.
pub(crate) fn settle(ctx: &mut Ctx<'_>, id: ContractId) -> Settled {
    settle_with(ctx, id, Vec::new())
}

/// [`settle`], with companion container docs riding the post's batch (E4c).
fn settle_with(ctx: &mut Ctx<'_>, id: ContractId, companions: Vec<EconomicContract>) -> Settled {
    let Some(c) = live_contract(ctx, id) else {
        return Settled::NotReady;
    };
    if !matches!(
        c.state,
        ContractState::Active | ContractState::ReadyToSettle
    ) {
        return Settled::NotReady;
    }
    let built = {
        let Some(store) = ctx.w.module_state::<EconomyStore>() else {
            return Settled::NotReady;
        };
        let providers = store.providers();
        let custody = WorldCustody {
            w: ctx.w,
            providers,
        };
        let facts = WorldFacts {
            w: ctx.w,
            providers: store.facts(),
        };
        let world = api::WorldAuth { w: ctx.w };
        let gate = store.key_gate();
        let env = ValidateEnv {
            keys: &gate,
            auth: &world,
            custody: &custody,
            facts: &facts,
            now: ctx.w.now(),
            ic_now: ctx.w.game_now_unix(),
            next_hold_id: store.peek_next_hold(),
            spent: std::collections::BTreeMap::new(),
        };
        let snap = store.snapshot();
        match resolve_outcome(&c, snap, &env) {
            Err(e) => Err(e),
            Ok(outcome) => settlement_plan(&c, snap, &env).map(|p| (p, outcome)),
        }
    };
    let (plan, outcome) = match built {
        Ok(v) => v,
        Err(e) => {
            let named = e.named_resources();
            return note_settle_failure(
                ctx,
                id,
                &e.to_string(),
                &named,
                matches!(e, ContractError::AmbiguousOutcome),
            );
        }
    };
    let owned = c.clone();
    let posted = {
        let auth = {
            let Some(store) = ctx.w.module_state::<EconomyStore>() else {
                return Settled::NotReady;
            };
            ContractAuth::settling(&owned, store.snapshot(), outcome)
        };
        api::post_with_companions(ctx, &plan, PostAuth::Injected(&auth), companions)
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
            &format!(
                "the settlement asked for approval (request #{request}), which nothing can collect"
            ),
            &[],
            true,
        ),
        // Doc 13 W2 (W-D1): the key is being read from the store; the next fire
        // retries, and no attempt is counted.
        Err(EconomyError::RetryShortly) => {
            tracing::info!(
                contract = id.0,
                "economy: a contract settlement waits on its key document"
            );
            Settled::NotReady
        }
        Err(e) => {
            let named = e.named_resources();
            note_settle_failure(ctx, id, &e.to_string(), &named, false)
        }
    }
}

/// Record a failed settlement attempt: loud log, `ReadyToSettle` kept, and
/// `Disputed` + a level-2 page once the attempts are spent (or at once, for a
/// failure no retry can fix — an ambiguous outcome is a decision, not a wait).
fn note_settle_failure(
    ctx: &mut Ctx<'_>,
    id: ContractId,
    detail: &str,
    named: &[NamedResource],
    terminal: bool,
) -> Settled {
    let refused = || Settled::Refused(detail.to_string(), named.to_vec());
    let Some(mut c) = live_contract(ctx, id) else {
        return refused();
    };
    if !matches!(
        c.state,
        ContractState::Active | ContractState::ReadyToSettle
    ) {
        return refused();
    }
    tracing::warn!(
        contract = id.0,
        detail,
        "economy: a contract settlement was refused; nothing moved"
    );
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
    refused()
}

// ---- dispute, agreement, resolution ---------------------------------------

/// Move a contract to `Disputed` and open the level-2 record for it, through the
/// **ladder's** own `page` (§4.8): one open record per subject, an acknowledged one
/// reopened rather than re-minted.
pub(crate) fn dispute_with(ctx: &mut Ctx<'_>, id: ContractId, context: String) {
    let Some(mut c) = live_contract(ctx, id) else {
        return;
    };
    // A **terminal** contract is not moved to `Disputed` — that would be a state
    // machine going backwards — but it is still **paged** (E4b2 verify fold, M2+M3).
    // The old early return meant every problem discovered about a settled, cancelled
    // or expired contract went nowhere at all: exactly the class of problem that has
    // already touched money and most needs a human.
    if !c.state.is_terminal() && c.state != ContractState::Disputed {
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
    let party = acting_party(ctx, &c, "contract.dispute");
    let admin = is_admin(ctx.w, ctx.actor);
    if party.is_none() && !admin {
        return Err(CmdError::msg(NO_SUCH));
    }
    // Doc 18 cut IC14 row 7 (lead ruling L6): the composition-frozen veto, asked for
    // a non-admin party only, before the state check — a seller may not dispute their
    // own sale into a bid. Staff resolve through `@contract/resolve`, never asked.
    if let (false, Some(party)) = (admin, party.as_ref()) {
        let guard = ctx
            .w
            .module_state::<EconomyStore>()
            .and_then(EconomyStore::dispute_guard);
        if let Some(line) = guard.and_then(|guard| guard.refusal(ctx.w, &c, party)) {
            return Err(CmdError::msg(line));
        }
    }
    if !matches!(
        c.state,
        ContractState::Active | ContractState::ReadyToSettle
    ) {
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
        return Err(CmdError::msg(format!(
            "Contract #{} is not disputed.",
            c.id
        )));
    }
    // E4b2 verify fold, A5: agreeing is an **accept-level** right, not a dispute-level
    // one. `contract.dispute` is the power to freeze an org's escrow and demand a
    // human look at it — a safety valve, deliberately cheap to hand out. Agreeing
    // disposes of the money: a unanimous `settle` pays the whole escrow out, a
    // unanimous `refund` unwinds an agreement the org is bound to. Consenting the org
    // to a disposition is exactly what `contract.accept` is.
    let party = acting_party(ctx, &c, "contract.accept").ok_or_else(|| CmdError::msg(NO_SUCH))?;
    let resolution = match ctx
        .args
        .rhs()
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("settle") => Resolution::Settle,
        Some("refund") => Resolution::Refund,
        _ => return Err(CmdError::msg(USAGE)),
    };
    let now = ctx.w.now();
    let mut c = c;
    c.agreements
        .retain(|a| principal_key(&a.party) != principal_key(&party));
    if c.agreements.len() >= MAX_AGREEMENTS {
        return Err(CmdError::msg("Too many recorded agreements."));
    }
    c.agreements.push(PartyAgreement {
        party,
        resolution,
        at: now,
    });
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
    info(
        ctx,
        format!("Contract #{id}: your agreement to {resolution:?} is recorded."),
    );
    if unanimous {
        info(
            ctx,
            format!("Every party agrees; contract #{id} is being resolved."),
        );
        apply_resolution(ctx, id, resolution);
    }
    ctx.gate_on_durable_ack();
    Ok(())
}

/// `@contract/resolve <id>=settle|refund [reason]` — staff, level 2 of the ladder.
///
/// **`settle` records a staff attestation on every unmet `Attestation` condition**
/// (E4c verify fold, E4b), canon sets included. Canon evidence is edge-triggered: a
/// record that landed while the economy was frozen, before the contract came into
/// force, or before the fact provider it names was ever wired, is an edge nobody can
/// produce again — and without this the escrow sat frozen until its deadline with a
/// staffer looking at a resolve command that refused to do anything. The override is
/// visible on the document forever, naming the staffer and their reason.
///
/// It moves **no deterministic condition**. A `Fact`, a `DepositInto`, a `TitleHeld`,
/// a `Before` or an `After` is re-verified against the live world at settlement
/// exactly as before, so staff can resolve a question of *evidence* and never a
/// question of *fact*: the goods are in the vault or they are not.
fn contract_resolve(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: @contract/resolve <id>=settle|refund [reason]   (staff)";
    if !is_admin(ctx.w, ctx.actor) {
        return Err(CmdError::msg(NO_SUCH));
    }
    let c = contract_of(ctx, ctx.args.lhs())?;
    if c.state != ContractState::Disputed {
        return Err(CmdError::msg(format!(
            "Contract #{} is not disputed.",
            c.id
        )));
    }
    let rhs = ctx.args.rhs().unwrap_or("").trim().to_string();
    let (verb, reason) = match rhs.split_once(char::is_whitespace) {
        Some((v, rest)) => (v.to_ascii_lowercase(), rest.trim().to_string()),
        None => (rhs.to_ascii_lowercase(), String::new()),
    };
    let resolution = match verb.as_str() {
        "settle" => Resolution::Settle,
        "refund" => Resolution::Refund,
        _ => return Err(CmdError::msg(USAGE)),
    };
    let id = c.id;
    if resolution == Resolution::Settle {
        record_staff_override(ctx, id, &reason);
    }
    apply_resolution(ctx, id, resolution);
    ctx.gate_on_durable_ack();
    Ok(())
}

/// Fill every unmet `Attestation` slot with an [`AttestorRef::StaffOverride`].
fn record_staff_override(ctx: &mut Ctx<'_>, id: ContractId, reason: &str) {
    let Some(mut c) = live_contract(ctx, id) else {
        return;
    };
    if c.satisfied.len() < c.consent.conditions.len() {
        c.satisfied.resize(c.consent.conditions.len(), None);
    }
    let now = ctx.w.now();
    let by = PrincipalId::Object(ctx.actor);
    let reason = if reason.is_empty() {
        "resolved by staff"
    } else {
        reason
    };
    let mut wrote = 0usize;
    for i in 0..c.consent.conditions.len() {
        if !matches!(
            c.consent.conditions.get(i),
            Some(TypedCondition::Attestation { .. })
        ) {
            continue;
        }
        let Some(slot) = c.satisfied.get_mut(i) else {
            continue;
        };
        if slot.is_some() {
            continue;
        }
        *slot = Some(Attestation {
            by: AttestorRef::StaffOverride {
                by: by.clone(),
                reason: SmolStr::new(reason),
            },
            at: now,
            clock: EconomicClock::RealUtc,
            source: SmolStr::new(format!("resolve:{}", ctx.actor.0)),
        });
        wrote += 1;
    }
    if wrote == 0 {
        return;
    }
    c.revision = c.revision.saturating_add(1);
    tracing::warn!(
        contract = id.0,
        conditions = wrote,
        actor = ctx.actor.0,
        reason,
        "economy: staff resolved a dispute by attesting unmet conditions"
    );
    info(
        ctx,
        format!(
            "Contract #{id}: {wrote} unmet attestation condition(s) recorded as resolved by you \
             ({reason}). Deterministic conditions are still checked."
        ),
    );
    api::put_contract(ctx, c);
}

/// Apply a resolution to a disputed contract.
///
/// `Settle` puts it back to `ReadyToSettle` and attempts a settlement — which
/// **still re-verifies** (D-E4-4): unanimity is consent to settle, not evidence that
/// the goods are in the vault, and the one thing a contract may never do is pay out
/// on a condition that is not true.
fn apply_resolution(ctx: &mut Ctx<'_>, id: ContractId, resolution: Resolution) {
    let Some(mut c) = live_contract(ctx, id) else {
        return;
    };
    if c.state != ContractState::Disputed {
        return;
    }
    match resolution {
        Resolution::Settle => {
            c.state = ContractState::ReadyToSettle;
            c.settle_attempts = 0;
            save(ctx, c);
            match settle(ctx, id) {
                Settled::Posted(tx) => {
                    info(ctx, format!("Contract #{id} settled as transaction #{tx}."));
                }
                // A6: the detail reaches a **party**, so it goes through the hiding
                // renderer first. Naming another party's frozen account in the reply
                // to `@contract/agree` is a disclosure the surface never made.
                Settled::Refused(detail, named) => {
                    let text = refusal_text(ctx, &detail, &named);
                    info(
                        ctx,
                        format!(
                            "Contract #{id} could not settle even so: {text}. Nothing moved; it \
                             stays where it was."
                        ),
                    );
                }
                Settled::NotReady => {
                    info(
                        ctx,
                        format!("Contract #{id} is not in a state that can settle."),
                    );
                }
            }
        }
        Resolution::Refund => {
            unwind(ctx, id, ContractState::Cancelled);
            if live_contract(ctx, id).is_some_and(|c| !c.state.is_terminal()) {
                info(
                    ctx,
                    format!(
                        "Contract #{id}'s refund was refused; nothing moved. It stays in force with \
                         its escrow reserved, the sweep is retrying, and staff have been paged."
                    ),
                );
            }
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
        ContractState::Active | ContractState::ReadyToSettle | ContractState::Disputed if staff => {
        }
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
        _ => {
            return Err(CmdError::msg(format!(
                "Contract #{} is already finished.",
                c.id
            )));
        }
    }
    let id = c.id;
    unwind(ctx, id, ContractState::Cancelled);
    // M2+M3: the state moves only once the refund posts, so the confirmation says
    // what actually happened rather than what was asked for.
    if live_contract(ctx, id).is_some_and(|c| c.state.is_terminal()) {
        info(ctx, format!("Contract #{id} is cancelled."));
    } else {
        info(
            ctx,
            format!(
                "Contract #{id} is ending, but its refund was refused: it stays in force with its \
                 escrow reserved while the sweep retries, and staff have been paged."
            ),
        );
    }
    ctx.gate_on_durable_ack();
    Ok(())
}

/// `@contract/amend <id>` — the sanctioned way to change an `Offered` contract's
/// terms after somebody has accepted them (08e answer 5).
///
/// An acceptance binds to the whole consent surface's hash, which is the money-safety
/// spine: changing anything voids an acceptance. **Consent by authorship** (E6a step
/// 1, DL-01 Option A′) is the one exception: the amendment's *author* — the party the
/// actor speaks for, or the issuer — wrote the terms, so their own acceptance survives
/// and their escrow stays reserved. Every other acceptance is voided and refunded
/// exactly as E4b2 did for all of them, the roster re-opens, and the contract returns
/// to `Draft`; the affected parties re-accept the amended terms once it is re-offered.
fn contract_amend(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let c = contract_of(ctx, ctx.args.lhs())?;
    if !is_issuer(ctx, &c) {
        return Err(CmdError::msg(NO_SUCH));
    }
    let author = amend_author(ctx, &c);
    let id = c.id;
    match amend_core(ctx, c.clone(), &author) {
        Ok(outcome) => {
            info(
                ctx,
                format!(
                    "Contract #{id} is back to draft; {} acceptance(s) were voided{}, {} (the \
                     author's) stand and their escrow stays reserved.",
                    outcome.voided,
                    if outcome.refunded {
                        " and refunded"
                    } else {
                        ""
                    },
                    outcome.kept
                ),
            );
            Ok(())
        }
        Err(e) => Err(amend_refusal(ctx, &c, &e)),
    }
}

/// Which party an amendment is **authored by** (A′): the party the actor speaks for
/// through the ordinary [`acting_party`] routes, or the issuer.
///
/// Staff are the one exception to the routes: a staffer who is not a party amends
/// *on the issuer's behalf* (their standing here is `is_issuer`'s staff clause), so
/// the issuer's own acceptance is the one that survives — never a party the staffer
/// happens to act for by fiat.
fn amend_author(ctx: &Ctx<'_>, c: &EconomicContract) -> PrincipalId {
    let me = PrincipalId::Object(ctx.actor);
    if c.consent.parties.contains(&me) {
        return me;
    }
    if is_admin(ctx.w, ctx.actor) {
        return c.issuer.clone();
    }
    // E6a step 1 verify fold (M7): **the issuer first.** `acting_party`'s third route
    // returns the first *sorted* org party the actor acts for, which on an unscoped
    // contract is a coin toss — an officer of both the issuer O and a counterparty Q
    // amending as Q would have voided O's own acceptance and refunded O's escrow while
    // keeping Q's. Only the issuer may reach this seam at all (`is_issuer`), so when
    // the actor acts for the issuer, the issuer is who they are amending as.
    if (api::WorldAuth { w: ctx.w }).acts_for(&me, &c.issuer) {
        return c.issuer.clone();
    }
    acting_party(ctx, c, "contract.draft").unwrap_or_else(|| c.issuer.clone())
}

/// What [`amend_contract`] did (E6a step 1 verify fold, R1).
///
/// The re-offer is the one step that runs after the A′ refund has posted, and it must
/// therefore never be an `Err`: a `Result::Err` from a ruleset handler discards the
/// deferred writes of a transaction that has already happened. A refused re-offer is
/// a **state**, not a failure — the amendment stands as an amendable `Draft`, the
/// author's escrow is still reserved, and the caller retries or cancels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AmendReoffer {
    /// The amendment stands. The surface is `Offered` again when `reoffer` was set,
    /// and an amendable `Draft` when it was not.
    Amended(AmendOutcome),
    /// The amendment stands as a `Draft` and the re-offer was refused — nothing was
    /// lost. `@contract/offer` retries it; `@contract/cancel` releases what is left.
    NotReoffered(AmendOutcome, ContractError),
}

impl AmendReoffer {
    /// What the amendment itself did, either way.
    #[must_use]
    pub fn outcome(&self) -> AmendOutcome {
        match self {
            AmendReoffer::Amended(o) | AmendReoffer::NotReoffered(o, _) => *o,
        }
    }

    /// Why the re-offer did not happen, if it did not.
    #[must_use]
    pub fn refusal(&self) -> Option<&ContractError> {
        match self {
            AmendReoffer::Amended(_) => None,
            AmendReoffer::NotReoffered(_, e) => Some(e),
        }
    }
}

/// What one amendment did (E6a step 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AmendOutcome {
    /// Acceptances that survived: the author's.
    pub kept: usize,
    /// Acceptances that were voided: every other party's.
    pub voided: usize,
    /// Whether a refund actually posted — which is the one condition under which the
    /// key generation moved (M16).
    pub refunded: bool,
}

/// **The `Draft` an amendment produces, and the acceptances it voids** — pure, from
/// the snapshot alone (E6a step 1 verify fold, R1).
///
/// It is a separate function precisely so the programmatic seam can build the whole
/// outcome, run every structural and world check against it, and only *then* post the
/// refund: a check that runs after a post is a check that can throw a landed
/// transaction away.
///
/// `refund` is the plan the caller will post (`None` when nothing is voided). The
/// surviving escrow is **every hold the plan does not release** — not just the ones
/// the author's considerations claim (M4). A `Held` hold with this contract's own
/// `Escrow` purpose that matches no consideration is neither released nor kept by
/// the claim rule, and dropping it from `escrow` orphaned real money: nothing would
/// ever release it again but boot's orphan pass. Terminal holds and holds belonging
/// to some other purpose are still dropped — the list is untrusted, and
/// `qualifying_*` filters it anyway.
fn amended_draft(
    c: &EconomicContract,
    snapshot: &crate::snapshot::EconomySnapshot,
    author: &PrincipalId,
    refund: Option<&EconomyPlan>,
    now: i64,
) -> (EconomicContract, Vec<Acceptance>) {
    let key = principal_key(author);
    let (kept, voided): (Vec<Acceptance>, Vec<Acceptance>) = c
        .acceptances
        .iter()
        .cloned()
        .partition(|a| principal_key(&a.party) == key);
    let released: std::collections::BTreeSet<HoldId> = refund
        .map(|p| {
            p.hold_ops
                .iter()
                .filter_map(|op| match op {
                    HoldOp::Release { hold } => Some(*hold),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    let mut seen: std::collections::BTreeSet<HoldId> = std::collections::BTreeSet::new();
    let kept_escrow: Vec<HoldId> = c
        .escrow
        .iter()
        .copied()
        // The list is untrusted, so a duplicate id is possible; two `Release` ops on
        // one hold would refuse the whole later unwind ("≤ 1 per hold").
        .filter(|id| seen.insert(*id))
        .filter(|id| !released.contains(id))
        .filter(|id| {
            snapshot.hold(*id).is_some_and(|h| {
                h.status.is_held() && h.purpose == (HoldPurpose::Escrow { contract: c.id })
            })
        })
        .collect();
    let mut back = c.clone();
    back.state = ContractState::Draft;
    back.pending = None;
    back.acceptances = kept;
    back.agreements.clear();
    back.escrow = kept_escrow;
    back.deposits.clear();
    // An attestation belongs to the surface it attested; a re-opened surface starts
    // with none (only an author-only live contract can carry any here).
    back.satisfied = vec![None; back.consent.conditions.len()];
    // E6b independent verify fold (R4): and so does a **canon** attestation, which
    // clearing `satisfied` alone did not achieve. A seal's source names the run and
    // the row, not the generation, so the next re-scan — from the re-activation,
    // from `@econ/unfreeze`, or from the first sweep after a boot — found the old
    // record again and put the old attestation straight back. The stamp is what makes
    // "evidence for the surface you agreed to" checkable: `record_and_settle` ignores
    // a record older than it.
    back.generation_started_at = now;
    back.roster_locked = false;
    back.settle_attempts = 0;
    back.revision = back.revision.saturating_add(1);
    // A `Draft` is re-hashed on every write; the survivors keep the hash they bound
    // and are re-stamped when the surface is re-offered.
    back.consent_hash = consent_hash(&back.issuer, &back.consent);
    (back, voided)
}

/// The amendment itself — the shared core of `@contract/amend` and the
/// programmatic `amend_contract` seam (E6a step 1, A′).
///
/// The order is the plan-first rule applied to a **non-terminal** transition: the
/// partial refund ([`crate::contract::refund_plan_except`]) posts under the contract's own key at its
/// generation, and the amended `Draft` record **rides that post's own batch** as a
/// companion (`post_with_companions`, B2's mechanism), so the released escrow and the
/// record that no longer lists it are one durable write. A key that resolves
/// `AlreadyPosted` means the refund landed and the doc write was lost: the state
/// write is all that is left, and it is done here. A refused refund refuses the
/// amendment with **nothing changed** — the contract stays `Offered` with its escrow
/// intact; `@contract/cancel` remains the retrying route out.
///
/// The generation moves only when a refund posted (M16): an amendment that released
/// nothing left `contract/{id}/refund/{n}` unused, so a later refund may still post
/// under it, and a run board that re-splits a listing thirty times never exhausts
/// [`MAX_AMENDMENTS`].
pub(crate) fn amend_core(
    ctx: &mut Ctx<'_>,
    c: EconomicContract,
    author: &PrincipalId,
) -> Result<AmendOutcome, ContractError> {
    let key = principal_key(author);
    // An `Offered` contract, or — the other face of A′ — one **in force with nobody
    // but its author bound**. A one-party contract activates at its author's own
    // accept (the last acceptance activates, D-E4-10), which is exactly what a run
    // board's `Listed` entry is: the payer's escrow stands and no counterparty has
    // consented to anything. Amending it unwinds nothing, and there is nothing that
    // activation placed to take back (no deliverables: no pledge, no lease). A live
    // contract anybody else accepted, or one carrying deliverables, is an agreement
    // in force and is not amendable — the routes are a dispute and unanimity, or
    // the deadline.
    let author_only_live = matches!(
        c.state,
        ContractState::Active | ContractState::ReadyToSettle
    ) && c.consent.terms.deliverables.is_empty()
        && c.acceptances.iter().all(|a| principal_key(&a.party) == key);
    if c.state != ContractState::Offered && !author_only_live {
        return Err(ContractError::State(c.id));
    }
    if c.amendments >= MAX_AMENDMENTS {
        return Err(ContractError::TooLarge(
            "the contract has been amended the most times allowed",
        ));
    }
    // **An amendment may not run over a refund that has not landed** (E4b2 verify
    // fold, M2+M3): a transition in flight owns the escrow until it lands.
    if c.pending.is_some() {
        return Err(ContractError::Infeasible(
            "the contract's refund has not landed yet; it is being retried",
        ));
    }
    // A surface that no longer matches its hash is tampered with, and an amendment
    // that re-hashed it would launder the tamper into a fresh draft under the author's
    // surviving acceptance. Nothing here moves it; staff resolve it.
    if !c.consent_intact() {
        return Err(ContractError::ConsentMismatch);
    }
    let now = ctx.w.now();
    let (plan, mut back, voided, refund_landed) = {
        let store = ctx
            .w
            .module_state::<EconomyStore>()
            .ok_or(ContractError::Infeasible("no economy"))?;
        if store.is_frozen() {
            return Err(ContractError::Infeasible("the economy is frozen"));
        }
        let snap = store.snapshot();
        let plan = if c.acceptances.iter().any(|a| principal_key(&a.party) != key) {
            Some(crate::contract::refund_plan_except(&c, snap, author)?)
        } else {
            None
        };
        let (back, voided) = amended_draft(&c, snap, author, plan.as_ref(), now);
        // E6a step 1 verify fold (R2): **the already-posted shape.** A hold this
        // record still lists as its own escrow, for a consideration a voided party
        // owed, that the journal says is already terminal — the refund landed and the
        // companion doc write was lost.
        let refund_landed = !voided.is_empty()
            && c.escrow.iter().any(|h| {
                snap.hold(*h).is_some_and(|hold| {
                    hold.purpose == (HoldPurpose::Escrow { contract: c.id })
                        && !hold.status.is_held()
                        && c.consent.terms.consideration.iter().any(|con| {
                            con.account == hold.account
                                && con.amount == hold.amount
                                && principal_key(&con.from) != key
                        })
                })
            });
        (plan, back, voided, refund_landed)
    };
    let refunding = plan.as_ref().is_some_and(|p| !p.hold_ops.is_empty());
    let id = c.id;
    // R2: the generation moves when a refund posts **or when one already has**.
    //
    // The old rule bumped only inside the refunding arm, and that burned a key. Lose
    // the companion write after a refund lands, and the retry sees the voided parties'
    // holds already `Released`: `qualifying_claims` yields nothing, the plan comes
    // back empty, nothing bumps, and the record is left pointing at
    // `contract/{id}/refund/{n}` — a key that is already spent. Every later unwind then
    // hit `IdempotencyConflict`, `pending` stuck, and the escrow was frozen forever.
    // Detecting the landed shape closes it while leaving M16 exactly as it was: an
    // amendment that releases nothing and never released anything (a run board
    // re-splitting its own listing) still spends no generation.
    if refunding || refund_landed {
        back.amendments = c.amendments.saturating_add(1);
    }
    match plan.filter(|_| refunding) {
        Some(plan) => {
            let owned = c.clone();
            let posted = {
                let auth = {
                    let store = ctx
                        .w
                        .module_state::<EconomyStore>()
                        .ok_or(ContractError::Infeasible("no economy"))?;
                    ContractAuth::unwinding(&owned, store.snapshot())
                };
                api::post_with_companions(ctx, &plan, PostAuth::Injected(&auth), vec![back.clone()])
            };
            match posted {
                Ok(PostOutcome::Posted(_)) => {
                    // The companion bypassed `put_contract`'s transition event; the
                    // transition is real, so say so.
                    crate::observer::emit(
                        ctx,
                        &crate::record::EconomicEvent::ContractChanged {
                            id,
                            state: back.state.kind(),
                            transaction: None,
                        },
                    );
                }
                // The refund landed on an earlier attempt and only the state write
                // was lost: finish it.
                Ok(PostOutcome::AlreadyPosted(_)) => api::put_contract(ctx, back.clone()),
                Ok(PostOutcome::NeedsApproval(_)) => {
                    return Err(ContractError::Refused(EconomyError::ApprovalRequired));
                }
                Err(e) => return Err(ContractError::Refused(e)),
            }
        }
        None => api::put_contract(ctx, back.clone()),
    }
    for p in &back.consent.parties {
        let stands = back
            .acceptances
            .iter()
            .any(|a| principal_key(&a.party) == principal_key(p));
        let msg = if stands {
            format!(
                "Contract #{id} has been amended by its author: your acceptance stands and your \
                 escrow stays reserved; it will be re-offered."
            )
        } else {
            format!(
                "Contract #{id} has been amended: any acceptance of yours is void and your escrow \
                 released."
            )
        };
        notify(ctx, p, &msg);
    }
    ctx.gate_on_durable_ack();
    Ok(AmendOutcome {
        kept: back.acceptances.len(),
        voided: voided.len(),
        refunded: refunding,
    })
}

/// Render an amendment refusal for the command path with the texts E4b2 pinned.
fn amend_refusal(ctx: &mut Ctx<'_>, c: &EconomicContract, e: &ContractError) -> CmdError {
    match e {
        ContractError::State(_) => CmdError::msg(format!(
            "Only an offered contract — or one in force that nobody but its author has accepted \
             and that carries no deliverables — can be amended; #{} is not.",
            c.id
        )),
        ContractError::TooLarge(_) => CmdError::msg(format!(
            "Contract #{} has been amended {MAX_AMENDMENTS} times; draft a new one.",
            c.id
        )),
        ContractError::Infeasible(_) if c.pending.is_some() => CmdError::msg(format!(
            "Contract #{}'s refund has not landed yet. It is being retried; try again shortly, or \
             @contract/resolve #{}=refund.",
            c.id, c.id
        )),
        ContractError::ConsentMismatch => CmdError::msg(format!(
            "Contract #{}'s consent surface has changed since it was offered, so it cannot be \
             amended as it stands: nothing was changed. @contract/cancel #{} refunds it under the \
             retrying path, and staff can @contract/resolve it.",
            c.id, c.id
        )),
        ContractError::Refused(inner) => {
            let first = c
                .consent
                .terms
                .consideration
                .first()
                .map_or(AccountId(0), |x| x.account);
            let hidden = refuse(
                ctx,
                NamedResource::Contract(c.id),
                &[NamedResource::Account(first)],
                inner,
            );
            CmdError::msg(format!(
                "Contract #{}'s escrow could not be released ({}), so it cannot be amended: nothing \
                 was changed. @contract/cancel #{} refunds it under the retrying path.",
                c.id, hidden, c.id
            ))
        }
        other => contract_refusal(ctx, c, other),
    }
}

// ---- terminal transitions: the plan first, then the state (M2+M3) ---------
//
// **The rule this whole section turns on.** A terminal transition is:
//
//   1. write `pending: Some(PendingTerminal { kind, .. })` on the live record;
//   2. post the transition's plan under the contract's deterministic key **at its own
//      generation** (`contract/{id}/refund/{n}`, `contract/{id}/forfeit/{n}`);
//   3. only then write the terminal state and clear `pending`.
//
// E4b2 shipped it the other way round — state first, money after — because that is
// what lifts the `DepositInto` custody lock. But a refused post then left a
// **terminal** contract holding live escrow: `unwind` logged, `forfeit` disputed a
// contract it had already expired (and `dispute_with` returned immediately on a
// terminal state, so even the page never happened), and nothing in the world retried
// either. The escrow was unreachable by any command, because every path that could
// release it refuses a terminal contract.
//
// Inverting the order needs one new thing — a way to withdraw a deposit while the
// contract is still live and the lock is still on — which is
// `AuthEval::authorize_custody_unlock`, answered `true` only by
// `ContractAuth::unwinding` for a deposit that contract recorded.
//
// It buys three properties, and each one is a bug the fold closes: the sweep can
// **retry** a half-done transition (step 3), boot can **finish** one by looking the
// same key up in the idempotency index (`boot.rs`), and a refusal is **paged** rather
// than logged, with `@contract/resolve` already written out.

/// Take a contract out of force and give everything back (§5).
///
/// Sets `pending` and attempts the unwind at once; a refusal leaves the contract live,
/// paged, and retried by the sweep.
pub(crate) fn unwind(ctx: &mut Ctx<'_>, id: ContractId, state: ContractState) {
    let kind = match state {
        ContractState::Cancelled => PendingKind::Cancel,
        _ => PendingKind::Expire,
    };
    if begin_pending(ctx, id, kind) {
        finish_terminal(ctx, id);
    }
}

/// Record that a terminal transition has begun. `false` when there is nothing to
/// begin (no contract, or it is already terminal).
fn begin_pending(ctx: &mut Ctx<'_>, id: ContractId, kind: PendingKind) -> bool {
    let Some(mut c) = live_contract(ctx, id) else {
        return false;
    };
    if c.state.is_terminal() {
        return false;
    }
    if c.pending.is_none() {
        // A transition already in flight WINS: two of them would post under two keys
        // and could each hand back a share of the same escrow.
        c.pending = Some(PendingTerminal {
            kind,
            attempts: 0,
            since: ctx.w.now(),
        });
        save(ctx, c);
    }
    true
}

/// Attempt the pending terminal transition once (the command path's own attempt, the
/// sweep's retry, and the route boot takes when no key resolved).
///
/// Returns `true` when the contract is terminal afterwards.
pub(crate) fn finish_terminal(ctx: &mut Ctx<'_>, id: ContractId) -> bool {
    let Some(c) = live_contract(ctx, id) else {
        return false;
    };
    // A contract that is **already terminal** and still carries `pending` is boot's
    // orphan-escrow recovery (B2): the record ended while money it did not list was
    // still reserved, so boot adopted the hold, advanced the key generation, and
    // asked for the contract's own unwind. Everything below works unchanged —
    // `refund_plan` accepts a `Cancelled`/`Expired` contract by construction, and
    // `land_terminal` leaves a terminal state where it is.
    let Some(pending) = c.pending.clone() else {
        return false;
    };

    // M6: the returns are derived **now**, from the journal's last-custody map, not
    // from what `record_deposits` cached at activation. A `DepositInto` delivered
    // after activation was invisible to the cache, so its object was custody-locked
    // for the contract's whole life and then — because the refund never named it —
    // left in the vault with the lock gone and no record of whose it was.
    let mut working = c.clone();
    working.deposits = record_deposits(ctx, &c);
    // E6b (DL-03): a deposit with nowhere to go is **paged**, never silently
    // stranded in the counterparty's vault. The page names the principal and the
    // command that fixes it, and the unwind still proceeds for everything else.
    for (owner, object) in stranded_deposits(ctx, &working) {
        page_stranded_deposit(ctx, id, &owner, object);
    }
    working.deposits.retain(|d| returnable(ctx, d));

    let built = {
        let Some(store) = ctx.w.module_state::<EconomyStore>() else {
            return false;
        };
        match pending.kind {
            PendingKind::Forfeit => forfeit_plan(&working, store.snapshot()),
            PendingKind::Cancel | PendingKind::Expire => refund_plan(&working, store.snapshot()),
        }
    };
    let plan = match built {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(contract = id.0, error = %e, "economy: a contract's unwind plan could not be built");
            note_terminal_failure(ctx, id, &e.to_string());
            return false;
        }
    };
    // Nothing to post: no escrow to release, no pledge to lift, no deposit to hand
    // back. The transition is complete the moment the state is written.
    if plan.requested_lines.is_empty()
        && plan.hold_ops.is_empty()
        && plan.asset_ops.is_empty()
        && plan.custody_ops.is_empty()
    {
        return land_terminal(ctx, id, pending.kind);
    }
    let posted = {
        let auth = {
            let Some(store) = ctx.w.module_state::<EconomyStore>() else {
                return false;
            };
            match pending.kind {
                PendingKind::Forfeit => match working.consent.terms.on_expiry {
                    Expiry::Forfeit { to } => {
                        ContractAuth::forfeiting(&working, store.snapshot(), to)
                    }
                    _ => return false,
                },
                PendingKind::Cancel | PendingKind::Expire => {
                    ContractAuth::unwinding(&working, store.snapshot())
                }
            }
        };
        api::post_with(ctx, &plan, PostAuth::Injected(&auth))
    };
    match posted {
        Ok(PostOutcome::Posted(_) | PostOutcome::AlreadyPosted(_)) => {
            land_terminal(ctx, id, pending.kind)
        }
        Ok(PostOutcome::NeedsApproval(request)) => {
            note_terminal_failure(
                ctx,
                id,
                &format!("it asked for approval (request #{request}), which nothing can collect"),
            );
            false
        }
        // Doc 13 W2 (W-D1): the key is being read from the store; the next fire
        // retries, and nothing is counted or paged.
        Err(EconomyError::RetryShortly) => {
            tracing::info!(
                contract = id.0,
                "economy: a contract's terminal transition waits on its key document"
            );
            false
        }
        Err(e) => {
            note_terminal_failure(ctx, id, &e.to_string());
            false
        }
    }
}

/// Doc 13 W2 (W-D7): the transitions boot could not finish because their keys
/// were outside the RAM window — resolved through the store, one keyed read
/// per key, and finished exactly as boot would have; a contract none of whose
/// keys resolves once the store has answered is disputed and paged, as boot
/// would have quarantined it.
pub(crate) fn heal_lost_transitions(ctx: &mut Ctx<'_>) {
    let wanted = match ctx.w.module_state::<EconomyStore>() {
        Some(store) if !store.heal_wanted.is_empty() => store.heal_wanted(),
        _ => return,
    };
    for id in wanted {
        let Some(c) = live_contract(ctx, id) else {
            forget_heal(ctx, id);
            continue;
        };
        let mut finished: Option<ContractState> = None;
        let mut unknown: Vec<crate::record::IdempotencyKey> = Vec::new();
        {
            let Some(store) = ctx.w.module_state::<EconomyStore>() else {
                return;
            };
            let gate = store.key_gate();
            for verb in ["settle", "refund", "forfeit"] {
                let key = crate::contract::contract_op_key(c.id, verb, c.amendments);
                match crate::engine::KeyGate::verdict(&gate, store.snapshot(), &key) {
                    crate::engine::KeyVerdict::Known(record) => {
                        if store.id_allocated(record.transaction)
                            && !store.snapshot().is_reversed(record.transaction)
                        {
                            finished = Some(match verb {
                                "settle" => ContractState::Settled {
                                    transaction: record.transaction,
                                },
                                "forfeit" => ContractState::Expired,
                                _ => ContractState::Cancelled,
                            });
                            break;
                        }
                    }
                    crate::engine::KeyVerdict::Fresh => {}
                    crate::engine::KeyVerdict::Unknown => unknown.push(key),
                }
            }
        }
        if let Some(state) = finished {
            tracing::info!(
                contract = id.0,
                state = ?state.kind(),
                "economy: a contract's terminal transition had posted before the RAM window; \
                 finished from its key document"
            );
            let mut c = c;
            c.state = state;
            c.pending = None;
            c.revision = c.revision.saturating_add(1);
            api::put_contract(ctx, c);
            forget_heal(ctx, id);
        } else if unknown.is_empty() {
            forget_heal(ctx, id);
            dispute_with(
                ctx,
                id,
                format!(
                    "contract #{id}'s escrow is not held in the journal, and none of its terminal \
                     keys resolves to a transaction"
                ),
            );
        } else {
            for key in unknown {
                crate::keys::request_key_read(ctx, &key);
            }
        }
    }
}

fn forget_heal(ctx: &mut Ctx<'_>, id: ContractId) {
    if let Some(store) = ctx.w.module_state_mut::<EconomyStore>() {
        store.heal_wanted.remove(&id);
    }
}

/// The transition's plan is durable: write the terminal state, clear `pending`, stop
/// the lease's rent, and tell the parties.
fn land_terminal(ctx: &mut Ctx<'_>, id: ContractId, kind: PendingKind) -> bool {
    let Some(mut c) = live_contract(ctx, id) else {
        return false;
    };
    if !c.state.is_terminal() {
        c.state = kind.target();
        c.pending = None;
        save(ctx, c.clone());
    } else {
        clear_pending(ctx, id);
    }
    cancel_lease_obligations(ctx, id);
    let msg = match kind {
        PendingKind::Forfeit => format!("Contract #{id} lapsed; its escrow was forfeit."),
        PendingKind::Cancel | PendingKind::Expire => {
            format!("Contract #{id} ended; its escrow was released.")
        }
    };
    notify_parties(ctx, &c, &msg);
    true
}

fn clear_pending(ctx: &mut Ctx<'_>, id: ContractId) {
    let Some(mut c) = live_contract(ctx, id) else {
        return;
    };
    if c.pending.is_some() {
        c.pending = None;
        save(ctx, c);
    }
}

/// One refused attempt at a terminal transition: the contract stays **live** with its
/// escrow intact, the attempt is counted, and a human is paged with the two commands
/// that end it. At [`MAX_SETTLE_ATTEMPTS`] it is `Disputed` — still not terminal, so
/// the money is still reachable.
fn note_terminal_failure(ctx: &mut Ctx<'_>, id: ContractId, detail: &str) {
    let Some(mut c) = live_contract(ctx, id) else {
        return;
    };
    let Some(mut pending) = c.pending.clone() else {
        return;
    };
    tracing::error!(
        contract = id.0,
        detail,
        "economy: a contract's terminal transition was refused; it stays live and its escrow is intact"
    );
    pending.attempts = pending.attempts.saturating_add(1);
    let spent = pending.attempts >= MAX_SETTLE_ATTEMPTS;
    c.pending = if spent { None } else { Some(pending) };
    save(ctx, c);
    if spent {
        dispute_with(
            ctx,
            id,
            format!(
                "contract #{id} could not be unwound after {MAX_SETTLE_ATTEMPTS} attempts \
                 ({detail}); it is still live and its escrow is still reserved"
            ),
        );
        return;
    }
    let now = ctx.w.now();
    crate::sweep::page(
        ctx,
        EscalationKind::SettlementRefused,
        EscalationSubject::Contract(id),
        None,
        format!(
            "contract #{id} is ending but its plan was refused ({detail}); it is still live, its \
             escrow is still reserved, and the sweep is retrying"
        ),
        vec![
            format!("@contract/show #{id}"),
            format!("@contract/resolve #{id}=refund"),
            format!("@contract/resolve #{id}=settle"),
        ],
        now,
    );
}

/// Whether a recorded deposit can actually be handed back right now: the object is
/// still in the vault, and the depositor is a live object the withdrawal may name as
/// a destination.
fn returnable(ctx: &Ctx<'_>, d: &DepositRecord) -> bool {
    let Some(store) = ctx.w.module_state::<EconomyStore>() else {
        return false;
    };
    let snapshot = store.snapshot();
    let Some(vault) = snapshot.vault(d.vault) else {
        return false;
    };
    if vault.status != crate::record::VaultStatus::Active {
        return false;
    }
    let custody = WorldCustody {
        w: ctx.w,
        providers: store.providers(),
    };
    use crate::engine::CustodyEval;
    if !custody.holds(vault, d.object) {
        return false;
    }
    match &d.deposited_by {
        PrincipalId::Object(to) => custody.destination_ok(&d.deposited_by, *to),
        // E6b (DL-03): an organization or system depositor goes home to the backing
        // of a vault they own. `deposit_returns` is the one router — this asks it
        // the same question the plan and the authority ask.
        owner @ (PrincipalId::Organization(_) | PrincipalId::System(_)) => {
            crate::contract::owner_home(snapshot, owner)
                .and_then(|(id, _)| snapshot.vault(id))
                .is_some_and(|v| custody.backing_valid(v))
        }
    }
}

/// Page the ladder about a deposit with nowhere to go (E6b, DL-03).
///
/// The subject is the **contract**, so the ladder's per-subject dedupe means one
/// page per stuck unwind rather than one per retry, and the recommended command is
/// the one that fixes it: give the depositor a vault. Nothing is silently dropped.
fn page_stranded_deposit(ctx: &mut Ctx<'_>, id: ContractId, owner: &PrincipalId, object: ObjId) {
    let now = ctx.w.now();
    let who = match owner {
        PrincipalId::Object(o) => format!("#{}", o.0),
        PrincipalId::Organization(o) => format!("org:{o}"),
        PrincipalId::System(s) => format!("sys:{s}"),
    };
    crate::sweep::page(
        ctx,
        EscalationKind::SettlementRefused,
        EscalationSubject::Contract(id),
        None,
        format!(
            "contract #{id} is unwinding but {who}'s deposit of object #{} cannot be returned: \
             they own no active vault to take it back into, or it is no longer where the contract \
             recorded it",
            object.0
        ),
        vec![
            format!("@contract/show #{id}"),
            format!("@vault/create <name>=container:<obj> owner={who}"),
        ],
        now,
    );
}

/// Which recorded deposits of `c` have **nowhere to go** (E6b, DL-03): a depositor
/// with no world object and no `Active` vault of their own. Never silently dropped —
/// [`finish_terminal`] pages with the command to fix it, because a stranded deposit
/// is exactly the shape 08a's staff-scarce rule forbids parking on silence.
fn stranded_deposits(ctx: &Ctx<'_>, c: &EconomicContract) -> Vec<(PrincipalId, ObjId)> {
    let Some(store) = ctx.w.module_state::<EconomyStore>() else {
        return Vec::new();
    };
    // E6b independent verify fold (M6): **the complement of `returnable`, exactly.**
    // The router answers "where would this go", and `returnable` answers "can it
    // actually go there right now" — a deposit whose object has left the vault, or
    // whose home vault's backing was destroyed, has a `to` and is still not
    // returnable, so it fell in the gap between the two: dropped by the plan and
    // reported by nothing. One predicate, and the page names it.
    let routes = crate::contract::deposit_returns(c, store.snapshot());
    routes
        .iter()
        .filter(|r| r.to.is_none() || !returnable(ctx, r.deposit))
        .map(|r| (r.deposit.deposited_by.clone(), r.deposit.object))
        .collect()
}

// ---- expiry (the sweep's arm, and the shared logic) ------------------------

/// Apply `on_expiry` to a contract whose deadline has passed (§5).
///
/// An `Offered` contract **always refunds**, whatever `on_expiry` says: it never came
/// into force, and taking the money someone escrowed toward an agreement that never
/// bound anybody is a confiscation rather than a forfeit (§5.1's deviation).
pub(crate) fn expire(ctx: &mut Ctx<'_>, id: ContractId) {
    let Some(c) = live_contract(ctx, id) else {
        return;
    };
    // Only a contract in force **between parties** is bound. An `Offered` one never
    // came into force; a `Draft` holding escrow (E6a step 1, A′: an author's
    // acceptance survives an amendment) has an acceptance but no agreement; and a
    // one-party contract in force — a run board's `Listed` entry — bound nobody but
    // its author, so its deadline gives the author's own money back (M7: an unbound
    // run refunds regardless). All three refund, whatever `on_expiry` says.
    let bound = c.state.is_live() && c.fully_accepted() && c.consent.parties.len() > 1;
    // A forfeit with nothing to forfeit is a refund: `forfeit_plan` refuses an empty
    // escrow, and there is still a pledge to release and a deposit to hand back.
    let has_escrow = ctx
        .w
        .module_state::<EconomyStore>()
        .is_some_and(|s| !qualifying_escrow(&c, s.snapshot()).holds.is_empty());
    match c.consent.terms.on_expiry {
        Expiry::Refund => {
            unwind(ctx, id, ContractState::Expired);
        }
        Expiry::Dispute if bound => {
            dispute_with(
                ctx,
                id,
                format!(
                    "contract #{id} reached its deadline unsettled and its terms say to dispute"
                ),
            );
        }
        Expiry::Dispute => {
            unwind(ctx, id, ContractState::Expired);
        }
        Expiry::Forfeit { .. } if bound && has_escrow => {
            if begin_pending(ctx, id, PendingKind::Forfeit) {
                finish_terminal(ctx, id);
            }
        }
        Expiry::Forfeit { .. } => {
            tracing::warn!(
                contract = id.0,
                bound,
                "economy: a lapsed contract cannot forfeit (it never bound anybody, or there is \
                 nothing escrowed); refunding instead"
            );
            unwind(ctx, id, ContractState::Expired);
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

/// Which disclosure class this viewer is in (E4b2 verify fold, A7).
///
/// `may_view_contract` answers *whether* you may look; this answers **at what**, and
/// the two were the same question before the fold: an audience member — doc 10 T6's
/// "eligible defenders", people told an operation is happening precisely because they
/// are on the other side of it — got the full surface, with every party, every
/// amount, every share and every escrow hold id. That is the plan, not the notice.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum ShowClass {
    /// A party, the issuer, a named attestor, or staff: the whole surface, with
    /// account ids masked by `may_view_account`.
    Full,
    /// A declared audience member: a **notice** — that it exists, its state, its
    /// window, and the kinds of condition. No money, no roles, no ids.
    Notice,
    /// Anyone else who got through the gate **on a route that consented to the
    /// terms**: a `Public` surface, or an org member holding one of the
    /// `contract.*` capabilities on the contract's own compartment. The consent
    /// surface as consented, with account ids masked.
    ///
    /// It is emphatically *not* everyone `may_view_contract` admits (E4c verify
    /// fold, D5): that gate also lets in an auditor holding `economy.account.view`
    /// on the scope, which is standing over the money and not over the agreement.
    /// A `Staff`-visibility org contract was readable in full by every such auditor,
    /// which undid the omission `@econ/history` was built to make.
    Consented,
}

fn show_class(ctx: &Ctx<'_>, c: &EconomicContract) -> ShowClass {
    if is_admin(ctx.w, ctx.actor) || is_issuer(ctx, c) {
        return ShowClass::Full;
    }
    let me = PrincipalId::Object(ctx.actor);
    let auth = api::WorldAuth { w: ctx.w };
    if c.consent.parties.iter().any(|p| auth.acts_for(&me, p)) {
        return ShowClass::Full;
    }
    // A named attestor or the adjudicator is a consented **authority** over this
    // contract's conditions; they have to read what they are being asked to attest.
    let named_attestor = c
        .consent
        .adjudicator
        .iter()
        .chain(c.consent.conditions.iter().filter_map(|cond| match cond {
            TypedCondition::Attestation { by } => Some(by),
            _ => None,
        }))
        .any(|set| attestor_can_view(ctx.w, ctx.actor, set));
    if named_attestor {
        return ShowClass::Full;
    }
    if c.consent
        .visibility
        .members()
        .iter()
        .any(|p| auth.acts_for(&me, p))
    {
        return ShowClass::Notice;
    }
    // D5: a `Public` surface is consent to be read by anybody. Anything else got here
    // through the org route, and only the `contract.*` half of that route is standing
    // over the agreement — `economy.account.view` is an auditor's standing over the
    // money, and it gets the notice.
    if !c.consent.visibility.is_public() {
        let by_contract_capability = [
            "contract.draft",
            "contract.accept",
            "contract.attest",
            "contract.dispute",
        ]
        .iter()
        .any(|cap| holds_contract_capability(ctx, c.consent.scope, cap));
        if !by_contract_capability {
            return ShowClass::Notice;
        }
    }
    ShowClass::Consented
}

fn contract_show(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let c = contract_of(ctx, ctx.args.lhs())?;
    let class = show_class(ctx, &c);
    let staff = is_admin(ctx.w, ctx.actor);
    if class == ShowClass::Notice {
        for l in notice_lines(ctx, &c) {
            info(ctx, l);
        }
        return Ok(());
    }
    // Every account id on this page goes through the same per-class rule: an id the
    // viewer may see, or the word "an account". E4c's disclosure filter renders
    // through `commands::mask_account` and nothing else — this closure is now that
    // one function, so `@econ/history`, `@hold/show` and this page cannot drift.
    let acct = |ctx: &Ctx<'_>, id: AccountId| -> String { mask_account(ctx, id, "an account") };
    let mut lines = vec![
        format!(
            "contract #{} \"{}\" — {}",
            c.id,
            c.name,
            render_state(&c.state)
        ),
        format!("  issued by {}", render_principal(ctx, &c.issuer)),
    ];
    let parties: Vec<String> = c
        .consent
        .parties
        .iter()
        .map(|p| render_principal(ctx, p))
        .collect();
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
        if accepted.is_empty() {
            "nobody yet".to_string()
        } else {
            accepted.join(", ")
        }
    ));
    // M8: the contract's own instrument, for every row whose account this viewer may
    // not see or that no longer resolves.
    let instrument = contract_instrument(ctx, &c);
    for x in &c.consent.terms.consideration {
        // A1: the payer's NAME comes from the party list, never from the account's
        // owner — otherwise `/pay` plus `/show` was an owner oracle over any wallet
        // the drafter could name. A `from` the surface does not list as a party is
        // rendered as a payer, not as a person.
        let who = if c
            .consent
            .parties
            .iter()
            .any(|p| principal_key(p) == principal_key(&x.from))
        {
            render_principal(ctx, &x.from)
        } else {
            "a payer".to_string()
        };
        lines.push(format!(
            "  {who} escrows {} from {}",
            money_on_or(ctx, x.account, x.amount, instrument.as_ref()),
            acct(ctx, x.account)
        ));
    }
    if c.consent.terms.outcomes.is_empty() {
        for sh in &c.consent.terms.split {
            lines.push(format!(
                "  pays {} {}",
                acct(ctx, sh.to),
                render_share(ctx, sh.to, sh.share, instrument.as_ref())
            ));
        }
        lines.push(format!(
            "  remainder to {}",
            acct(ctx, c.consent.terms.remainder_to)
        ));
    } else {
        for o in &c.consent.terms.outcomes {
            let pays: Vec<String> = o
                .payouts
                .iter()
                .map(|sh| {
                    format!(
                        "{} {}",
                        acct(ctx, sh.to),
                        render_share(ctx, sh.to, sh.share, instrument.as_ref())
                    )
                })
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
        lines.push(format!(
            "  condition {i}: {}{mark}",
            render_condition(ctx, cond)
        ));
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
            render_expiry_masked(ctx, c.consent.terms.on_expiry)
        ));
    }
    lines.push(format!("  visibility: {}", c.consent.visibility.label()));
    if let EconomicVisibility::Audience { members, .. } = &c.consent.visibility {
        let who: Vec<String> = members.iter().map(|p| render_principal(ctx, p)).collect();
        lines.push(format!("  audience: {}", who.join(", ")));
    }
    // The roles and the escrow hold ids are operational detail about the people
    // running this, not part of what a public reader consented to see.
    if class == ShowClass::Full {
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
        if let Some(pending) = &c.pending {
            lines.push(format!(
                "  ending ({:?}) — attempt {} of {MAX_SETTLE_ATTEMPTS}; its escrow is still reserved",
                pending.kind, pending.attempts
            ));
        }
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

/// The **audience notice** (A7, doc 10 T6): notice of the window, not the plan.
///
/// What is here is deliberate and short — that a contract exists, what it is called,
/// where it is in its life, how long it runs, and what *kinds* of thing have to be
/// true for it to settle. What is NOT here is everything a defender would use to act
/// against the parties rather than prepare for the operation: who is on it, what
/// anyone is paying or being paid, which accounts, which objects, which holds, and
/// (unless the surface says `announce=yes`) who is running it.
fn notice_lines(ctx: &Ctx<'_>, c: &EconomicContract) -> Vec<String> {
    let mut lines = vec![format!(
        "contract #{} \"{}\" — {} (you are seeing a notice, not the terms)",
        c.id,
        c.name,
        // D6: the settlement's transaction id is a handle into `@econ/history`;
        // a notice says that it settled and no more.
        render_state_notice(&c.state)
    )];
    if c.consent.visibility.announce_issuer() {
        lines.push(format!("  issued by {}", render_principal(ctx, &c.issuer)));
    }
    if let Some(d) = c.consent.deadline {
        lines.push(format!("  window closes at {} on {:?}", d.at, d.clock));
    }
    let kinds: Vec<String> = c
        .consent
        .conditions
        .iter()
        .enumerate()
        .map(|(i, cond)| {
            let mark = if c.satisfied.get(i).and_then(|s| s.as_ref()).is_some() {
                " (met)"
            } else {
                ""
            };
            format!("{:?}{mark}", cond.evidence_kind())
        })
        .collect();
    if kinds.is_empty() {
        lines.push("  no recorded conditions".to_string());
    } else {
        lines.push(format!("  conditions: {}", kinds.join(", ")));
    }
    lines.push(format!("  visibility: {}", c.consent.visibility.label()));
    lines.push(
        "  This is a notice, not the terms: an audience is told that something is happening and \
         when, never who is paying whom."
            .to_string(),
    );
    lines
}

/// [`render_state`] for a **notice**-class viewer (E4c verify fold, D6): the state
/// word and nothing that hangs off it. A settlement's transaction id is a handle into
/// `@econ/history`, and an audience was told an operation exists, not what it paid.
fn render_state_notice(s: &ContractState) -> String {
    match s {
        ContractState::Settled { .. } => "settled".to_string(),
        other => render_state(other),
    }
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

/// **The instrument a contract is denominated in** (E6a step 1 verify fold, M8): the
/// one its escrow is held in, taken from the first consideration whose account still
/// resolves. `check_offerable` proves every consideration and every exact share share
/// one instrument, so "the first" is "the contract's".
///
/// It is what a payout row renders in when the payee account is unviewable or gone.
/// The viewer is already reading this contract's terms, so its scale and symbol are
/// not a disclosure — and `500` where the world says `¥500` is a number nobody can
/// act on.
fn contract_instrument(ctx: &Ctx<'_>, c: &EconomicContract) -> Option<InstrumentId> {
    let store = ctx.w.module_state::<EconomyStore>()?;
    let snap = store.snapshot();
    c.consent
        .terms
        .consideration
        .iter()
        .find_map(|x| snap.account(x.account).map(|a| a.instrument.clone()))
}

/// A share, an exact one rendered in the instrument the payee account holds — or, for
/// an account this viewer may not see (D2) or that no longer exists, the contract's
/// own instrument (M8).
fn render_share(
    ctx: &Ctx<'_>,
    to: AccountId,
    s: Share,
    instrument: Option<&InstrumentId>,
) -> String {
    match s {
        Share::Exact(a) => money_on_or(ctx, to, a, instrument),
        Share::Permille(p) => format!("{p}‰"),
    }
}

/// `on_expiry`, with the forfeit account behind the same per-class mask as every
/// other account id on the page (A7).
fn render_expiry_masked(ctx: &Ctx<'_>, e: Expiry) -> String {
    match e {
        Expiry::Refund => "refund".to_string(),
        Expiry::Forfeit { to } => {
            if may_view_account(ctx.w, ctx.actor, to) {
                format!("forfeit to account #{to}")
            } else {
                "forfeit to an account".to_string()
            }
        }
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
        Deliverable::Obligation { template } => {
            let side = |id: crate::ids::AccountId| {
                if may_view_account(ctx.w, ctx.actor, id) {
                    format!("account #{id}")
                } else {
                    "an account".to_string()
                }
            };
            format!(
                "an obligation: {} owes {} {} each period",
                side(template.debit_account),
                side(template.credit_account),
                money_on(ctx, template.debit_account, template.amount)
            )
        }
    }
}

fn render_condition(ctx: &Ctx<'_>, c: &TypedCondition) -> String {
    match c {
        TypedCondition::Attestation { by } => {
            format!("attested by {}", render_attestor_set(ctx, by))
        }
        TypedCondition::DepositInto { object, vault } => {
            format!("#{} is in vault #{vault}", object.0)
        }
        TypedCondition::TitleHeld { object, by } => {
            format!("{} holds title to #{}", render_principal(ctx, by), object.0)
        }
        TypedCondition::Fact { fact } => {
            format!("the fact {}/{}/{}", fact.provider, fact.kind, fact.id)
        }
        TypedCondition::Before { deadline } => {
            format!("before {} on {:?}", deadline.at, deadline.clock)
        }
        TypedCondition::After {
            condition,
            delay,
            clock,
        } => {
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
        AttestorRef::Observed { .. } => "observed holding".to_string(),
        AttestorRef::StaffOverride { by, reason } => {
            format!(
                "staff resolution by {} ({reason})",
                render_principal(ctx, by)
            )
        }
    }
}

fn contract_list(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let ids: Vec<ContractId> = {
        let store = ctx
            .w
            .module_state::<EconomyStore>()
            .ok_or_else(|| CmdError::msg("No economy."))?;
        store.snapshot().contracts().map(|c| c.id).collect()
    };
    let rows: Vec<String> = ids
        .into_iter()
        .filter(|id| may_view_contract(ctx.w, ctx.actor, *id))
        .filter_map(|id| {
            // D6: `/list` renders per class too. A notice-class viewer used to read
            // the party count and the settlement transaction id here — the two facts
            // `@contract/show` had just been taught to withhold from them.
            let c = ctx
                .w
                .module_state::<EconomyStore>()?
                .snapshot()
                .contract(id)?
                .clone();
            if show_class(ctx, &c) == ShowClass::Notice {
                return Some(format!(
                    "#{} \"{}\" — {}",
                    c.id,
                    c.name,
                    render_state_notice(&c.state)
                ));
            }
            Some(format!(
                "#{} \"{}\" — {} ({} part{})",
                c.id,
                c.name,
                render_state(&c.state),
                c.consent.parties.len(),
                if c.consent.parties.len() == 1 {
                    "y"
                } else {
                    "ies"
                }
            ))
        })
        .collect();
    emit_list(ctx, rows, "No contracts.");
    Ok(())
}

// ---- the lease seam a layered module offers (E5b, 08d §4) ------------------

/// Everything [`offer_lease`] needs. A struct rather than nine positional
/// arguments, and deliberately **not** an [`EconomicContract`]: the id, the consent
/// hash, the roster lock, the landlord's own acceptance and the state are the
/// economy's to fill in, not the caller's.
#[derive(Clone, Debug)]
pub struct LeaseSpec {
    /// A short human label for `@contract/show`.
    pub name: SmolStr,
    /// The **lessor**: the contract's issuer, its first party, and the principal the
    /// `Lease` encumbrance names.
    pub lessor: PrincipalId,
    /// The org compartment the lease belongs to, when the lessor is an organization.
    pub scope: Option<OrgScopeId>,
    /// The tenant(s). Each becomes a party and each must accept.
    pub tenants: Vec<PrincipalId>,
    /// The room the lease is *of* — pledged as the lessor's collateral, which is
    /// what makes the claim placed at activation an [`crate::Encumbrance::Lease`]
    /// rather than a plain `Collateral` ([`crate::contract_encumbrance`]).
    pub premises: ObjId,
    /// The rent: the obligation created at activation.
    pub rent: ObligationTemplate,
}

/// **Offer a lease** on behalf of a layered module (E5b's housing).
///
/// It is one call because a lease is one act. `@contract/draft` + `/party` + `/deliver
/// collateral` + `/deliver obligation` + `/remainder` + `/condition` + `/offer` is
/// seven commands whose intermediate states — a draft with a pledge and no rent, a
/// lease with rent and no premises — are all things a housing tenancy must never be
/// bound to; and `@contract/deliver` is the **issuer's** verb, so a system landlord
/// (`sys:wod.housing`, which types nothing) could not build one at all.
///
/// What it builds, and why each piece is there:
///
/// - **parties = lessor + tenants**, with the lessor as issuer (`check_structure`
///   requires the issuer to be a party). The lessor's acceptance is recorded here:
///   offering the lease *is* the landlord side's consent, which is exactly the shape
///   `mush_housing::accept(t, landlord_accepted, …)` already has. Every tenant then
///   accepts for themselves through the ordinary `@contract/accept`, under their own
///   live rights — that acceptance is the consent, and it is the last one, so it
///   activates.
/// - **no consideration**: a lease escrows nothing. The rent is a recurring
///   obligation the sweep settles period by period, not a lump the contract holds.
/// - **`Collateral { premises, lessor }` + `Obligation { rent }`**: the pair is what
///   [`crate::contract_encumbrance`] reads as a lease, so activation places
///   `Encumbrance::Lease { lessor, contract }` on the premises and the unwind takes
///   it off again. `check_offerable` proves the lessor really holds that title.
/// - **one `Attestation { by: Staff }` condition**: without it a contract with no
///   conditions is `Ready` the moment it activates, and the very next sweep would
///   settle it — terminal — which cancels the rent obligation it just created. A
///   lease ends when somebody says it ends (D-E5-1 through and through), and this is
///   how that is written in the consent surface the tenant signs.
///
/// # Errors
///
/// Every structural refusal `@contract/offer` would give, plus
/// [`ContractError::TooLarge`] when the contract allocator is exhausted. Nothing is
/// written on any error path.
pub fn offer_lease(ctx: &mut Ctx<'_>, spec: LeaseSpec) -> Result<ContractId, ContractError> {
    if ctx
        .w
        .module_state::<EconomyStore>()
        .is_some_and(EconomyStore::is_frozen)
    {
        return Err(ContractError::Infeasible("the economy is frozen"));
    }
    if spec.tenants.is_empty() {
        return Err(ContractError::Infeasible(
            "a lease needs at least one tenant",
        ));
    }
    let mut parties = vec![spec.lessor.clone()];
    for t in &spec.tenants {
        if parties.iter().any(|p| principal_key(p) == principal_key(t)) {
            return Err(ContractError::Infeasible("a party is named twice"));
        }
        parties.push(t.clone());
    }
    let now = ctx.w.now();
    // Read before `spec.rent` is moved into the deliverable below.
    let rent_credit = spec.rent.credit_account;
    let consent = ConsentSurface {
        parties,
        terms: ContractTerms {
            consideration: Vec::new(),
            deliverables: vec![
                Deliverable::Collateral {
                    object: spec.premises,
                    from: spec.lessor.clone(),
                },
                Deliverable::Obligation {
                    template: Box::new(spec.rent),
                },
            ],
            split: Vec::new(),
            // Nothing is escrowed, so nothing is split and the remainder is zero —
            // but `check_offerable` still proves the account exists and fixes the
            // contract's one instrument, so name the rent's creditor rather than an
            // id that does not resolve.
            remainder_to: rent_credit,
            on_expiry: Expiry::Refund,
            outcomes: Vec::new(),
        },
        conditions: vec![TypedCondition::Attestation {
            by: AttestorSet::Staff,
        }],
        deadline: None,
        visibility: EconomicVisibility::Parties,
        scope: spec.scope,
        adjudicator: None,
        evidence: Vec::new(),
    };
    let mut c = EconomicContract {
        id: ContractId(0),
        issuer: spec.lessor.clone(),
        name: spec.name,
        consent_hash: [0u8; 32],
        consent,
        acceptances: Vec::new(),
        satisfied: vec![None],
        escrow: Vec::new(),
        state: ContractState::Offered,
        settle_attempts: 0,
        created_at: now,
        amendments: 0,
        generation_started_at: now,
        disputed_at: None,
        canon_published: None,
        roster_locked: true,
        deposits: Vec::new(),
        agreements: Vec::new(),
        pending: None,
        roles: ContractRoles {
            payer: Some(spec.lessor.clone()),
            ..ContractRoles::default()
        },
        revision: 1,
    };
    // Validate against a **provisional** id: nothing is allocated and nothing is
    // written until the whole shape is known good, so a refusal costs no id.
    {
        let store = ctx
            .w
            .module_state::<EconomyStore>()
            .ok_or(ContractError::Infeasible("no economy"))?;
        let outcomes = store.canon_outcome_kinds();
        crate::contract::check_structure(&c, &outcomes)?;
        check_offerable(&c, store.snapshot())?;
    }
    let id = {
        let store = ctx
            .w
            .module_state_mut::<EconomyStore>()
            .ok_or(ContractError::Infeasible("no economy"))?;
        store.alloc_contract().ok_or(ContractError::TooLarge(
            "the contract-id allocator is exhausted",
        ))?
    };
    c.id = id;
    // The lessor's acceptance, then the frozen hash — in that order, so the
    // acceptance binds the hash every tenant will also bind.
    c.consent_hash = consent_hash(&c.issuer, &c.consent);
    c.acceptances.push(Acceptance {
        party: spec.lessor.clone(),
        at: now,
        consent_hash: c.consent_hash,
    });
    api::put_contract(ctx, c.clone());
    let meta = ctx
        .w
        .module_state::<EconomyStore>()
        .map(EconomyStore::meta_doc);
    if let Some(meta) = meta {
        api::persist_meta(ctx.out, meta);
    }
    for p in &spec.tenants {
        notify(
            ctx,
            p,
            &format!(
                "Lease #{id} has been offered to you (@contract/show #{id}, @contract/accept #{id})."
            ),
        );
    }
    ctx.gate_on_durable_ack();
    Ok(id)
}

/// What proves a lease is the caller's to end (look-back fold, rent-after-end).
///
/// The economy keeps no record of *which module* offered a contract, and adding one
/// would be a field nothing else reads. What it does keep is the two facts
/// [`LeaseSpec`] fixed when the lease was offered — the lessor, who is the issuer and
/// the `Lease` encumbrance's principal, and the premises pledged as their collateral
/// — so those are the key: a caller that can name both is ending its own lease, and
/// a wrong id (the bug this exists for) fails closed on either.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeaseKey {
    /// The lessor `offer_lease` was called with.
    pub lessor: PrincipalId,
    /// The premises `offer_lease` was called with.
    pub premises: ObjId,
}

/// What [`end_lease`] did.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LeaseEnd {
    /// The unwind posted under the contract's deterministic key and the contract is
    /// `Cancelled`: its pledge is released and every obligation it created is
    /// `Cancelled`.
    Ended,
    /// The contract was already terminal. Nothing was written.
    AlreadyEnded,
    /// The unwind's plan was refused. The contract stays **live** with
    /// `pending: PendingTerminal` recorded, the sweep retries it, and staff have been
    /// paged with the commands that finish it (the E4b2 terminal-transition rule).
    Retrying,
}

/// **End a lease** on behalf of the layered module that offered it (look-back fold,
/// rent-after-end; the counterpart of [`offer_lease`]).
///
/// Housing's tenancy is the shadow of the lease, and the lease is the economy's
/// record: when the tenancy ends — a staff resolution, the last tenant moving out,
/// an eviction decided — the rent has to stop **in the same step**, or a former
/// tenant keeps being debited for a home they no longer have. Before this seam the
/// only routes out of a live lease were `@contract/cancel` (staff, E4b2-D21) and a
/// staff attestation, and a module could call neither.
///
/// It is the ordinary cancellation, through the ordinary machinery, and nothing
/// else: `unwind` writes `pending`, posts the refund plan under
/// `contract/{id}/refund/{generation}` under a [`ContractAuth`] built for
/// [`crate::ContractPurpose::Unwind`] — which for a lease releases the `Lease`
/// encumbrance from the premises (a lease escrows nothing, so there is no hold to
/// give back) — and only then writes `Cancelled`, which is the step that cancels
/// every obligation the contract created (`cause: Contract(id)`). Replayed, the key
/// resolves to the first post (`AlreadyPosted`) and the state write is the only
/// thing left to do; called on a contract that is already terminal it writes
/// nothing at all.
///
/// A lease that is `Offered` (the tenant never accepted) ends the same way with an
/// empty plan, so a later `@contract/accept` cannot activate a lease whose tenancy is
/// gone. A `Disputed` lease ends too: the dispute is a party's, but the home is over,
/// and the one thing a dispute must never do is keep charging rent for it.
///
/// # Errors
///
/// [`ContractError::Infeasible`] when there is no such contract, when it is not a
/// lease this `key` offered (not lease-shaped, a different issuer, or different
/// premises — nothing is written, and the text does not say which), or when the
/// economy is frozen (no write can land; housing's boot pass ends it once the
/// economy is back).
pub fn end_lease(
    ctx: &mut Ctx<'_>,
    contract: ContractId,
    key: &LeaseKey,
) -> Result<LeaseEnd, ContractError> {
    let c = {
        let store = ctx
            .w
            .module_state::<EconomyStore>()
            .ok_or(ContractError::Infeasible("no economy"))?;
        if store.is_frozen() {
            return Err(ContractError::Infeasible("the economy is frozen"));
        }
        store
            .snapshot()
            .contract(contract)
            .cloned()
            .ok_or(ContractError::Infeasible("no such lease"))?
    };
    if !owns_lease(&c, key) {
        return Err(ContractError::Infeasible(
            "not a lease this lessor offered on these premises",
        ));
    }
    if c.state.is_terminal() {
        return Ok(LeaseEnd::AlreadyEnded);
    }
    unwind(ctx, contract, ContractState::Cancelled);
    let terminal = live_contract(ctx, contract).is_some_and(|c| c.state.is_terminal());
    Ok(if terminal {
        LeaseEnd::Ended
    } else {
        LeaseEnd::Retrying
    })
}

/// Whether `c` is the lease `key` describes: issued by the lessor, pledging exactly
/// these premises as the lessor's collateral, and creating an obligation — the shape
/// [`offer_lease`] builds and [`crate::contract_encumbrance`] reads as a lease.
///
/// Doc 18 cut IC6: the shape itself is [`crate::contract::is_lease_shaped`] (one
/// predicate); only the key-specific checks — this lessor, these premises — live here.
fn owns_lease(c: &EconomicContract, key: &LeaseKey) -> bool {
    crate::contract::is_lease_shaped(c)
        && principal_key(&c.issuer) == principal_key(&key.lessor)
        && c.consent.terms.deliverables.iter().any(|d| {
            matches!(
                d,
                Deliverable::Collateral { object, from }
                    if *object == key.premises && principal_key(from) == principal_key(&key.lessor)
            )
        })
}

// ---- the programmatic contract seams (E6a step 1, 08f §2 item 4) -------------

/// One account's escrow in a [`ContractSpec`]: what it puts up at its owner's
/// accept. The payer — the `from` of the resulting [`Consideration`] — is **derived
/// from the account's owner**, never named by the caller (08e answer 4: the
/// accountable escrow principal is a fact about the wallet), and the owner must be a
/// party of the surface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsiderationSpec {
    /// The account escrowed from.
    pub account: AccountId,
    /// How much (> 0).
    pub amount: Amount,
}

/// Everything [`offer_contract`] needs — the [`LeaseSpec`] shape generalized to the
/// whole `@contract` builder surface, so a ruleset never types a command. A struct
/// rather than fifteen positional arguments, and deliberately **not** an
/// [`EconomicContract`]: the id, the issuer, the consent hash, the payers behind the
/// considerations, the satisfaction vector, the roster lock and the state are the
/// economy's to fill in.
#[derive(Clone, Debug)]
pub struct ContractSpec {
    /// A short human label (≤ [`MAX_CONTRACT_NAME_BYTES`]).
    pub name: SmolStr,
    /// `Some(scope)`: the issuer is that compartment's organization, and the actor
    /// needs `contract.draft` on it (or staff) — `@contract/draft … scope=`'s rule.
    /// `None`: the actor is the issuer.
    pub scope: Option<OrgScopeId>,
    /// The parties besides the issuer (the issuer is always a party; naming it here
    /// is harmless).
    pub parties: Vec<PrincipalId>,
    /// What is escrowed, and from where (one or more). The actor must be able to
    /// **view** each account (`@contract/pay`'s rule (a)), its owner must be a
    /// party, and a `System`-owned one is refused outright (M4).
    pub considerations: Vec<ConsiderationSpec>,
    /// The outcome table — **at least one row** (D-E6-6): each row's payouts name
    /// exactly where every minor unit lands and sum to the escrowed total; there is
    /// no implicit remainder. A programmatic offer never carries E4a's single split.
    pub outcomes: Vec<OutcomeTerms>,
    /// Inert under a table, but it must still resolve to a live account in the
    /// contract's instrument (M18) — the payer's own, by convention.
    pub remainder_to: AccountId,
    /// The conditions the outcome rows index. **`Attestation` only** unless
    /// `deterministic` is set: E6a settles by attestation alone (D-E6-20), and a bare
    /// `Before` row would be `Ready` the moment the last party accepted.
    pub conditions: Vec<TypedCondition>,
    /// The named adjudicator (08e answer 6), consented at offer.
    pub adjudicator: Option<AttestorSet>,
    /// The permitted evidence kinds (empty admits every kind).
    pub evidence: Vec<EvidenceKind>,
    /// When the offer lapses, on its own clock.
    pub deadline: Option<EconomicDeadline>,
    /// What a lapsed deadline does once the contract is bound (an unbound one always
    /// refunds).
    pub on_expiry: Expiry,
    /// The disclosure every party consents to.
    pub visibility: EconomicVisibility,
    /// The separate roles (attribution, never authority). `author` defaults to the
    /// actor and `payer` to the first consideration's owner when unset.
    pub roles: ContractRoles,
    /// Admit `DepositInto`/`TitleHeld`/`Fact`/`Before`/`After` conditions — E6b's
    /// settlement routes. Off by default.
    pub deterministic: bool,
    /// What the contract delivers besides money (E6c).
    ///
    /// The field E6a step 1 left out and E6c needs: `offer_contract` hard-coded
    /// `deliverables: Vec::new()`, so a programmatic offer could move money and never
    /// goods — and a market purchase of a titled artefact **is** a
    /// [`Deliverable::TitleTransfer`], which E4b2 already executes at settlement under
    /// D-E3-12's both-parties-accepted rule. Adding the field costs no new settlement
    /// machinery at all: `check_offer` validates it exactly as it validates the
    /// `@contract/deliver` builder's, and each outcome row names the indexes it
    /// causes through `OutcomeTerms::asset_consequences`.
    pub deliverables: Vec<Deliverable>,
}

/// What [`amend_contract`] changes (E6a step 1). Each field is optional so a
/// caller re-splitting a listing touches nothing else.
#[derive(Clone, Debug, Default)]
pub struct ContractAmendment {
    /// Parties to add (each must be new; the cap is [`MAX_PARTIES`]).
    pub add_parties: Vec<PrincipalId>,
    /// Replace the outcome table when `Some` (the same rules as at offer).
    pub outcomes: Option<Vec<OutcomeTerms>>,
    /// Replace what a lapsed deadline does when `Some` — a board posts `Refund`
    /// while nobody but the payer is bound and switches to `Dispute` at crewing
    /// (D-E6-22).
    pub on_expiry: Option<Expiry>,
    /// Re-offer in the same call. When the re-offer is refused the amendment and the
    /// edits **stand** as a `Draft` — the caller retries or cancels (M10: the sweep
    /// expires a draft holding escrow past its deadline).
    pub reoffer: bool,
}

/// Who a programmatic offer is issued by: the actor, or the compartment's
/// organization when `scope` is named and the actor holds `contract.draft` on it.
fn resolve_issuer(ctx: &Ctx<'_>, scope: Option<OrgScopeId>) -> Result<PrincipalId, ContractError> {
    let Some(s) = scope else {
        return Ok(PrincipalId::Object(ctx.actor));
    };
    let org = ctx
        .w
        .module_state::<mush_org::OrgStore>()
        .and_then(|st| st.state().scope(s).map(|sc| sc.org))
        .ok_or(ContractError::Infeasible("no such scope"))?;
    if !is_admin(ctx.w, ctx.actor) && !api::holds_capability(ctx.w, ctx.actor, "contract.draft", s)
    {
        return Err(ContractError::Infeasible("no such scope"));
    }
    Ok(PrincipalId::Organization(org))
}

/// `@contract/pay`'s rules, for a programmatic offer: a positive amount, an
/// account the actor may **view** (a hidden one answers exactly like a missing one),
/// `Active`, and — E6a step 1's M4 fold — never `System`-owned.
fn consideration_for(
    ctx: &Ctx<'_>,
    spec: &ConsiderationSpec,
    parties: &[PrincipalId],
) -> Result<Consideration, ContractError> {
    if spec.amount.is_zero() || spec.amount.is_negative() {
        return Err(ContractError::Infeasible(
            "a consideration must be positive",
        ));
    }
    let store = ctx
        .w
        .module_state::<EconomyStore>()
        .ok_or(ContractError::Infeasible("no economy"))?;
    let account = store
        .snapshot()
        .account(spec.account)
        .ok_or(ContractError::NoSuchAccount(spec.account))?;
    // §11 q7: the caller may not name an account they cannot view — the rule that
    // stops a poster using `account=` as an existence probe.
    //
    // E6b (DL-11) is its one exception, and it is composition-shaped: a
    // **wiring-named vendor's** own account. A fence publishes a price and the deal
    // names that price back to it; the account is derived from the vendor's own
    // record and never typed, no player can add a principal to the table, and the id
    // is still masked in every render (`render_account`). Without it a vendor could
    // quote a number nobody was allowed to name back to it.
    //
    // E6b verify fold (D4): and only for a vendor that is a **party to this very
    // contract**. The first form widened the predicate for every consideration on
    // every offer, which made a vendor's account nameable by anyone building any
    // surface — the next caller of this `pub` seam would have inherited an existence
    // oracle it never asked for.
    //
    // Doc 18 cut IC7 (L8, F3): the order is viewable → party → capability, so the
    // capability predicate (which may now ask a content source) is asked only when
    // it can matter — never about a buyer naming their own account, never about an
    // account whose owner is not a party. Same truth table as before for every
    // table-named vendor.
    let viewable = may_view_account(ctx.w, ctx.actor, spec.account);
    let vendor = !viewable
        && parties
            .iter()
            .any(|p| principal_key(p) == principal_key(&account.owner))
        && crate::module::party_cap(ctx.w, &account.owner, crate::contract::PartyCap::Accept);
    if !viewable && !vendor {
        return Err(ContractError::NoSuchAccount(spec.account));
    }
    if account.status != AccountStatus::Active {
        return Err(ContractError::NoSuchAccount(spec.account));
    }
    if matches!(account.owner, PrincipalId::System(_)) {
        return Err(ContractError::SystemParty);
    }
    Ok(Consideration {
        from: account.owner.clone(),
        account: spec.account,
        amount: spec.amount,
    })
}

/// **Draft and offer a contract in one call** on behalf of a ruleset (E6a step 1,
/// 08f §2 item 4; D-E6-19's `+run/post` is the first caller).
///
/// It is the whole `@contract/draft` → builders → `@contract/offer` sequence with
/// the same gates, and none of the intermediate states: nothing is allocated and
/// nothing is written until the surface has passed `check_offer` — the structural
/// half, the world half, the per-party cap and the re-stamp rule — so a refused
/// offer leaves **no draft behind** (M17) and costs no id. The issuer is the actor
/// or, through `scope`, an organization the actor holds `contract.draft` on
/// (D-E6-19: the payer is the issuer); a `System` payer is refused before anything
/// exists (M4).
///
/// # Errors
///
/// Every structural refusal `@contract/offer` gives, plus
/// [`ContractError::NoSuchAccount`] for a consideration account the actor may not
/// view (rendered as the hiding text), [`ContractError::SystemParty`],
/// [`ContractError::EvidenceNotPermitted`] for a deterministic condition without
/// `deterministic`, [`ContractError::Infeasible`] for a missing table, an unknown
/// scope, or a frozen economy, and [`ContractError::TooLarge`] for the caps
/// (including the allocator at its ceiling). Nothing is written on any error path.
pub fn offer_contract(ctx: &mut Ctx<'_>, spec: ContractSpec) -> Result<ContractId, ContractError> {
    if ctx
        .w
        .module_state::<EconomyStore>()
        .is_none_or(EconomyStore::is_frozen)
    {
        return Err(ContractError::Infeasible("the economy is frozen"));
    }
    if spec.name.is_empty() || spec.name.len() > MAX_CONTRACT_NAME_BYTES {
        return Err(ContractError::TooLarge("the name is empty or too long"));
    }
    if spec.outcomes.is_empty() {
        return Err(ContractError::Infeasible(
            "a programmatic offer carries an outcome table",
        ));
    }
    if spec.considerations.is_empty() {
        return Err(ContractError::Infeasible(
            "a programmatic offer escrows at least one consideration",
        ));
    }
    let issuer = resolve_issuer(ctx, spec.scope)?;
    if spec
        .parties
        .iter()
        .chain(std::iter::once(&issuer))
        .any(|p| matches!(p, PrincipalId::System(_)))
    {
        return Err(ContractError::SystemParty);
    }
    // §9's cap, counted over NON-TERMINAL contracts (as `@contract/draft`).
    let open = ctx.w.module_state::<EconomyStore>().map_or(0, |st| {
        st.snapshot()
            .contracts()
            .filter(|c| c.issuer == issuer && !c.state.is_terminal())
            .count()
    });
    if open >= MAX_OPEN_CONTRACTS_PER_ISSUER {
        return Err(ContractError::TooLarge(
            "that issuer already has the most open contracts allowed",
        ));
    }
    if !spec.deterministic
        && let Some(i) = spec
            .conditions
            .iter()
            .position(|c| !matches!(c, TypedCondition::Attestation { .. }))
    {
        return Err(ContractError::EvidenceNotPermitted(i));
    }
    let mut parties = vec![issuer.clone()];
    for p in &spec.parties {
        // The issuer named as a party is harmless; anyone else twice is refused by
        // `check_structure` below.
        if principal_key(p) == principal_key(&issuer) {
            continue;
        }
        parties.push(p.clone());
    }
    let mut consideration: Vec<Consideration> = Vec::with_capacity(spec.considerations.len());
    for c in &spec.considerations {
        let built = consideration_for(ctx, c, &parties)?;
        consideration.retain(|x| x.account != built.account);
        consideration.push(built);
    }
    let now = ctx.w.now();
    let mut roles = spec.roles;
    if roles.author.is_none() {
        roles.author = Some(PrincipalId::Object(ctx.actor));
    }
    if roles.payer.is_none() {
        roles.payer = consideration.first().map(|x| x.from.clone());
    }
    let satisfied = vec![None; spec.conditions.len()];
    let consent = ConsentSurface {
        parties,
        terms: ContractTerms {
            consideration,
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
    };
    let mut c = EconomicContract {
        // Validated against a PROVISIONAL id nothing else can carry, so the per-party
        // cap never mistakes a real contract for this one.
        id: ContractId(u64::MAX),
        issuer: issuer.clone(),
        name: spec.name,
        consent_hash: [0u8; 32],
        consent,
        acceptances: Vec::new(),
        satisfied,
        escrow: Vec::new(),
        state: ContractState::Draft,
        settle_attempts: 0,
        created_at: now,
        amendments: 0,
        generation_started_at: now,
        disputed_at: None,
        canon_published: None,
        roster_locked: false,
        deposits: Vec::new(),
        agreements: Vec::new(),
        pending: None,
        roles,
        revision: 1,
    };
    check_offer(ctx, &c)?;
    let id = {
        let store = ctx
            .w
            .module_state_mut::<EconomyStore>()
            .ok_or(ContractError::Infeasible("no economy"))?;
        store.alloc_contract().ok_or(ContractError::TooLarge(
            "the contract-id allocator is exhausted",
        ))?
    };
    c.id = id;
    let id = freeze_and_offer(ctx, c);
    let meta = ctx
        .w
        .module_state::<EconomyStore>()
        .map(EconomyStore::meta_doc);
    if let Some(meta) = meta {
        api::persist_meta(ctx.out, meta);
    }
    ctx.gate_on_durable_ack();
    Ok(id)
}

/// **Accept a contract** on behalf of the actor, under their own authority (E6a
/// step 1) — `@contract/accept` without the command: the actor must be a party (or
/// act for one, the `acting_party` routes), and the escrow is placed exactly as the
/// command places it, in one `post_with_companions`. `NeedsApproval` surfaces as
/// [`AcceptOutcome::NeedsApproval`] with **no acceptance recorded** (D-E4-11).
///
/// # Errors
///
/// [`ContractError::NoStanding`] (no such contract, unviewable, or not a party — one
/// answer), [`ContractError::State`] (not open for acceptance),
/// [`ContractError::ConsentMismatch`], [`ContractError::AlreadyAccepted`],
/// [`ContractError::Refused`] (the escrow post was refused: no funds, a suspended
/// compartment, no debit authority), [`ContractError::Infeasible`] (frozen).
pub fn accept_contract(ctx: &mut Ctx<'_>, id: ContractId) -> Result<AcceptOutcome, ContractError> {
    if ctx
        .w
        .module_state::<EconomyStore>()
        .is_none_or(EconomyStore::is_frozen)
    {
        return Err(ContractError::Infeasible("the economy is frozen"));
    }
    if !may_view_contract(ctx.w, ctx.actor, id) {
        return Err(ContractError::NoStanding);
    }
    let c = live_contract(ctx, id).ok_or(ContractError::NoStanding)?;
    let party = acting_party(ctx, &c, "contract.accept").ok_or(ContractError::NoStanding)?;
    accept_core(ctx, c, party)
}

// ---- E6b: B4 — DL-11's vendor accept --------------------------------------

/// **Accept a contract on a wiring-named principal's behalf** (E6b, DL-11 / M15).
///
/// The narrow answer to "a staff-built NPC fence has no officer": `acting_party`
/// takes the `acts_for` route for an organization and nothing else, and a vendor org
/// with no member and no grantee has nobody who can accept, which would put a
/// staffer in every deal.
///
/// Everything about it is bounded:
///
/// - `party` must carry [`crate::contract::PartyCap::Accept`] in the **wiring-frozen** table
///   (`EconomyModule::with_party_caps`) — never a grant, never a document;
/// - `party` must already be a party to `c`; this creates no party and no term;
/// - the acceptance runs the same `accept_core` the command runs, so the roster
///   still locks, the consent hash is still checked, and the escrow is still placed;
/// - the escrow plan's actor is `party` itself, so the hold posts under **that
///   principal's own debit authority** and refuses on `InsufficientAvailable`. A
///   fence that runs out of money stops being able to accept, which is exactly how
///   "a fence that runs out stops quoting" is enforced (M19, D-E6-15).
///
/// # Errors
///
/// [`ContractError::NoStanding`] when the economy is frozen to the caller, the
/// contract is unviewable, `party` is not a party, or the wiring never named it;
/// otherwise everything `accept_core` refuses.
pub fn accept_contract_as(
    ctx: &mut Ctx<'_>,
    id: ContractId,
    party: &PrincipalId,
) -> Result<AcceptOutcome, ContractError> {
    if ctx
        .w
        .module_state::<EconomyStore>()
        .is_none_or(EconomyStore::is_frozen)
    {
        return Err(ContractError::Infeasible("the economy is frozen"));
    }
    if !crate::module::party_cap(ctx.w, party, crate::contract::PartyCap::Accept) {
        return Err(ContractError::NoStanding);
    }
    let c = live_contract(ctx, id).ok_or(ContractError::NoStanding)?;
    if !c.consent.parties.contains(party) {
        return Err(ContractError::NoStanding);
    }
    accept_core_as(ctx, c, party.clone(), party.clone())
}

/// **Deliver the goods leg of a contract that has already settled** (E6c B6) — the
/// seventh programmatic seam, game-neutral, and the reason it exists is arithmetic:
/// an [`EconomicContract`] holds **one instrument** (`check_structure`'s
/// `InstrumentMismatch`; there is no implicit exchange in this economy), so a
/// purchase paid in currency cannot also pay out a commodity or an entitlement.
/// Those move as a second, ordinary journal post afterwards — and until now no
/// caller could make one, because the debit is the **seller's** and every command's
/// actor is `PrincipalId::Object(ctx.actor)`.
///
/// It is bounded exactly as [`accept_contract_as`] is, by the same wiring-frozen
/// table and two facts about the contract:
///
/// 1. `from`'s owner holds [`crate::contract::PartyCap::Accept`] — a composition-named vendor
///    organization, never something a command, a document or a grant can add to;
/// 2. that owner is a **party to `contract`**, so a vendor can only ever deliver
///    against a deal it actually made;
/// 3. `contract` is `Settled`, so the goods move only after the money did.
///
/// It confers **no money authority whatever**: the plan's actor is the seller, so
/// the debit posts under that account's own authority (`WorldAuth`'s organization
/// arm, over its own live compartment), its `ApprovalRule` still binds, and a seller
/// with no stock is refused exactly as one with no money is. The key is
/// `contract.deliver/{contract}`, so a re-entered observer delivers **once**.
///
/// # Errors
///
/// [`ContractError::NoStanding`] when the economy is frozen, the contract is
/// unviewable or unsettled, the wiring never named the seller, or the seller is not a
/// party; [`ContractError::Refused`] carrying the economy's own refusal otherwise.
pub fn deliver_settled(
    ctx: &mut Ctx<'_>,
    contract: ContractId,
    from: AccountId,
    to: AccountId,
    amount: Amount,
) -> Result<PostOutcome, ContractError> {
    if amount.minor() <= 0 {
        return Err(ContractError::Infeasible(
            "a delivery moves a positive amount",
        ));
    }
    if ctx
        .w
        .module_state::<EconomyStore>()
        .is_none_or(EconomyStore::is_frozen)
    {
        return Err(ContractError::Infeasible("the economy is frozen"));
    }
    let (seller, settled) = {
        let store = ctx
            .w
            .module_state::<EconomyStore>()
            .ok_or(ContractError::NoStanding)?;
        let snap = store.snapshot();
        let account = snap.account(from).ok_or(ContractError::NoStanding)?;
        let c = snap.contract(contract).ok_or(ContractError::NoStanding)?;
        (
            account.owner.clone(),
            matches!(c.state, ContractState::Settled { .. })
                && c.consent.parties.contains(&account.owner),
        )
    };
    if !settled {
        return Err(ContractError::NoStanding);
    }
    if !crate::module::party_cap(ctx.w, &seller, crate::contract::PartyCap::Accept) {
        return Err(ContractError::NoStanding);
    }
    let plan = EconomyPlan::new(
        crate::record::IdempotencyKey::new(
            SmolStr::new_static("contract.deliver"),
            SmolStr::new(contract.0.to_string()),
        ),
        seller,
        crate::record::EconomicCause::Internal {
            kind: SmolStr::new_static("contract.deliver"),
            id: contract.0,
        },
        vec![
            crate::plan::RequestedLine {
                account: from,
                delta: Amount::from_minor(-amount.minor()),
                memo: SmolStr::new_static("delivery"),
            },
            crate::plan::RequestedLine {
                account: to,
                delta: amount,
                memo: SmolStr::new_static("delivery"),
            },
        ],
    );
    api::post(ctx, &plan).map_err(ContractError::Refused)
}

/// **Amend an offered contract** on behalf of its issuer (E6a step 1): S1's rules —
/// the author's own acceptance and escrow stand, every other acceptance is voided
/// and refunded — then the roster and the outcome table are rewritten, and the
/// surface is re-offered in the same call when `reoffer` says so. `+run/crew accept`
/// is the first caller: with only the payer accepted, the crew is added and every
/// row rewritten to their accounts without the escrow moving.
///
/// # Errors
///
/// Everything is refused **before anything posts** (E6a step 1 verify fold, R1):
/// [`ContractError::NoStanding`] (no such contract, unviewable, or not the issuer),
/// everything `amend_core` refuses, [`ContractError::SystemParty`],
/// [`ContractError::TooLarge`] for the party cap, [`ContractError::Infeasible`] for a
/// duplicate party, and — when `reoffer` is set — every refusal `check_offer`
/// gives. An `Err` therefore means **nothing changed at all**.
///
/// A re-offer that is refused *after* the amendment lands is not an error: it comes
/// back as [`AmendReoffer::NotReoffered`], with the amendment standing as an
/// amendable `Draft` (retry, or cancel — M10, and the sweep expires an escrowed draft
/// on its deadline or the draft TTL).
pub fn amend_contract(
    ctx: &mut Ctx<'_>,
    id: ContractId,
    amend: ContractAmendment,
) -> Result<AmendReoffer, ContractError> {
    if ctx
        .w
        .module_state::<EconomyStore>()
        .is_none_or(EconomyStore::is_frozen)
    {
        return Err(ContractError::Infeasible("the economy is frozen"));
    }
    if !may_view_contract(ctx.w, ctx.actor, id) {
        return Err(ContractError::NoStanding);
    }
    let c = live_contract(ctx, id).ok_or(ContractError::NoStanding)?;
    if !is_issuer(ctx, &c) {
        return Err(ContractError::NoStanding);
    }
    let author = amend_author(ctx, &c);
    let now = ctx.w.now();
    // ---- R1: every fallible step, BEFORE the refund posts ----
    //
    // `amend_core` posts the A′ refund through `post_with_companions`, and four
    // fallible steps used to follow it: the `System` party test, the duplicate-party
    // test, the party cap, and `check_offer`'s whole battery. Every economy write is
    // `Deferred::Persist`, and dispatch truncates a failed handler's deferred queue —
    // so any one of those refusals threw away the journal chunk, the hold and account
    // projections and the contract doc, while RAM had already released the escrow and
    // moved the contract to `Draft`. After a reboot the escrow was back and the
    // contract was `Offered` again: RAM and disk disagreeing about money.
    //
    // The fix is order, not rescue. Build exactly the surface the amendment would
    // produce — the same pure `amended_draft` the core uses, plus this amendment's own
    // edits — and run every check against it here. Past this point nothing between the
    // post and the return can refuse.
    let probe = {
        let store = ctx
            .w
            .module_state::<EconomyStore>()
            .ok_or(ContractError::Infeasible("no economy"))?;
        let snap = store.snapshot();
        let refund = if c
            .acceptances
            .iter()
            .any(|a| principal_key(&a.party) != principal_key(&author))
        {
            Some(crate::contract::refund_plan_except(&c, snap, &author)?)
        } else {
            None
        };
        let (draft, _) = amended_draft(&c, snap, &author, refund.as_ref(), now);
        draft
    };
    let probe = apply_amendment(probe, &amend)?;
    if amend.reoffer {
        check_offer(ctx, &probe)?;
    }
    // ---- the amendment itself ----
    let outcome = amend_core(ctx, c, &author)?;
    let back = live_contract(ctx, id).ok_or(ContractError::NoStanding)?;
    // The edits were validated against an identical projection above, so this cannot
    // refuse; if a future edit makes it possible, the amendment still stands and the
    // caller is told which step did not run — never an `Err` over a landed post.
    let mut back = match apply_amendment(back, &amend) {
        Ok(back) => back,
        Err(e) => return Ok(AmendReoffer::NotReoffered(outcome, e)),
    };
    back.revision = back.revision.saturating_add(1);
    api::put_contract(ctx, back.clone());
    if amend.reoffer {
        // Re-checked live: the projection was built before the refund posted, and
        // releasing escrow is exactly the kind of change `check_offer` reads.
        if let Err(e) = check_offer(ctx, &back) {
            ctx.gate_on_durable_ack();
            return Ok(AmendReoffer::NotReoffered(outcome, e));
        }
        freeze_and_offer(ctx, back);
    }
    ctx.gate_on_durable_ack();
    Ok(AmendReoffer::Amended(outcome))
}

/// **Cancel a contract the actor issues** on behalf of a ruleset (E6a) — the fourth
/// programmatic seam, and `@contract/cancel` without the command.
///
/// It exists because the other three make cancellation *reachable*: `+run/post`
/// offers and then accepts, and a refused accept must leave no unfunded `Offered`
/// contract standing against the issuer's cap (M17); `+run/withdraw` is the payer
/// taking a listing down. A ruleset that had to type `@contract/cancel` would be
/// dispatching a command from inside a handler, re-running its per-invocation gates
/// against an actor who has already passed them.
///
/// Only a `Draft` or `Offered` contract: a live agreement is not one party's to tear
/// up, and the routes for that stay the dispute, the deadline, and staff.
///
/// Returns whether the contract actually **reached a terminal state**. `Ok(false)`
/// means the refund was refused — the contract stays in force with its escrow
/// reserved while the sweep retries, and staff have been paged — which is exactly
/// what `@contract/cancel` reports, and the caller must not treat it as done.
///
/// # Errors
///
/// [`ContractError::NoStanding`] (no such contract, unviewable, or not the issuer),
/// [`ContractError::State`] (live, disputed or already finished),
/// [`ContractError::Infeasible`] (a frozen economy).
pub fn cancel_contract(ctx: &mut Ctx<'_>, id: ContractId) -> Result<bool, ContractError> {
    if ctx
        .w
        .module_state::<EconomyStore>()
        .is_none_or(EconomyStore::is_frozen)
    {
        return Err(ContractError::Infeasible("the economy is frozen"));
    }
    if !may_view_contract(ctx.w, ctx.actor, id) {
        return Err(ContractError::NoStanding);
    }
    let c = live_contract(ctx, id).ok_or(ContractError::NoStanding)?;
    if !is_issuer(ctx, &c) {
        return Err(ContractError::NoStanding);
    }
    // `@contract/cancel`'s rule, plus **A′'s other face** (S1 deviation 2): a live
    // contract with no deliverables that nobody but its own author has accepted. A
    // one-party contract activates at its author's accept, so that is exactly the
    // shape a run board's `Listed` entry has — the payer's escrow standing with no
    // counterparty consented to anything. "A live agreement is not one party's to
    // tear up" is a rule about *other people's* consent; where there is none to tear
    // up, `+run/withdraw` is the payer taking back their own money.
    let author = amend_author(ctx, &c);
    let key = principal_key(&author);
    let author_only_live = matches!(
        c.state,
        ContractState::Active | ContractState::ReadyToSettle
    ) && c.consent.terms.deliverables.is_empty()
        && c.acceptances.iter().all(|a| principal_key(&a.party) == key);
    match c.state {
        ContractState::Draft | ContractState::Offered => {}
        _ if author_only_live => {}
        ContractState::Active
        | ContractState::ReadyToSettle
        | ContractState::Disputed
        | ContractState::Settled { .. }
        | ContractState::Cancelled
        | ContractState::Expired => return Err(ContractError::State(id)),
    }
    unwind(ctx, id, ContractState::Cancelled);
    let done = live_contract(ctx, id).is_some_and(|c| c.state.is_terminal());
    ctx.gate_on_durable_ack();
    Ok(done)
}

/// Apply one [`ContractAmendment`]'s edits to a surface — pure and fallible, so the
/// seam can run it against a projection before anything posts (R1).
fn apply_amendment(
    mut c: EconomicContract,
    amend: &ContractAmendment,
) -> Result<EconomicContract, ContractError> {
    for p in &amend.add_parties {
        // M4/D-E6-4: nobody acts for a `System` principal, so one named as a party is
        // an offer that can never be accepted and a faucet that could never be
        // controlled.
        if matches!(p, PrincipalId::System(_)) {
            return Err(ContractError::SystemParty);
        }
        if c.consent
            .parties
            .iter()
            .any(|q| principal_key(q) == principal_key(p))
        {
            return Err(ContractError::Infeasible("a party is named twice"));
        }
        if c.consent.parties.len() >= MAX_PARTIES {
            return Err(ContractError::TooLarge("too many parties"));
        }
        c.consent.parties.push(p.clone());
    }
    if let Some(outcomes) = &amend.outcomes {
        c.consent.terms.outcomes = outcomes.clone();
    }
    if let Some(on_expiry) = amend.on_expiry {
        c.consent.terms.on_expiry = on_expiry;
    }
    // A `Draft` is re-hashed on every write; the author's acceptance is re-stamped
    // by the re-offer.
    c.consent_hash = consent_hash(&c.issuer, &c.consent);
    c.satisfied.resize(c.consent.conditions.len(), None);
    Ok(c)
}

// ---- the notice class and the hiding helpers, exported (E6a step 1, E5b-D36/D37) --

/// **The notice a contract shows to somebody outside its terms** (E4b2's audience
/// class, `ShowClass::Notice`): its name, its state (a settlement's transaction id
/// withheld), its window, the *kinds* of condition it settles on, its visibility,
/// and the issuer only when the surface says `announce=yes` — never a party, an
/// amount, an account, a hold or a role.
///
/// Exported for the run board (08f §3.4 `+run/show`) and every later notice surface,
/// so a ruleset renders the notice the economy renders and never makes a third
/// copy of the rule (E5b-D36/D37).
#[must_use]
pub fn contract_notice(ctx: &Ctx<'_>, c: &EconomicContract) -> Vec<String> {
    notice_lines(ctx, c)
}

/// Render a [`ContractError`] about an existing contract for a person, behind the
/// existence-hiding rule — exactly what `@contract/*` shows (E4b2 verify fold, A2;
/// E6a step 1's `NoStanding` and `SystemParty` included). For a ruleset that calls
/// [`accept_contract`] or [`amend_contract`] and has to answer its actor.
#[must_use]
pub fn refuse_contract(ctx: &mut Ctx<'_>, c: &EconomicContract, e: &ContractError) -> CmdError {
    contract_refusal(ctx, c, e)
}

/// Render a [`ContractError`] for a person when there is **no contract yet** — a
/// refused [`offer_contract`] — against the resources the caller's spec named
/// (`named`: every account, vault and asset it mentioned), behind the same
/// existence-hiding rule. A `NoStanding` renders as the nonexistent-contract text, a
/// `SystemParty` as the account's, and anything naming a resource the actor may not
/// view as that resource's "No such …".
#[must_use]
pub fn refuse_contract_offer(
    ctx: &mut Ctx<'_>,
    e: &ContractError,
    named: impl IntoIterator<Item = NamedResource>,
) -> CmdError {
    match e {
        ContractError::NoStanding => return CmdError::msg(NO_SUCH),
        ContractError::SystemParty => return CmdError::msg("No such account."),
        _ => {}
    }
    let own = e.named_resources();
    let also: Vec<NamedResource> = if e.concerns_named_subjects() {
        named.into_iter().collect()
    } else {
        Vec::new()
    };
    let mut hidden: Option<String> = None;
    for r in own.iter().chain(also.iter()) {
        if let Some(text) = crate::commands::hidden_text(ctx, *r) {
            hidden = Some(text.to_string());
            break;
        }
    }
    crate::commands::note_refusal(ctx, hidden.is_some());
    CmdError::msg(hidden.unwrap_or_else(|| e.to_string()))
}
