//! The run board (08f §3.2 "Board", §3.4, D-E6-17, D-E6-19..22).
//!
//! A run is **a contract with an audience notice**, not a listing of goods, so the
//! board lives here and `mush-market` never learns about it (D-E6-17). Everything
//! the board holds is a projection: the money, the consent, the authority and the
//! settlement are E4b2's, reached through the four programmatic seams
//! ([`mush_economy::offer_contract`], [`accept_contract`](mush_economy::accept_contract),
//! [`amend_contract`](mush_economy::amend_contract) and
//! [`cancel_contract`](mush_economy::cancel_contract)) — no `@contract` command is
//! ever dispatched.
//!
//! ## The flow, end to end
//!
//! 1. **`+run/post`** — the payer drafts as **issuer** (D-E6-19), offers, and accepts
//!    in one command. The accept is what places the escrow, so **no listing exists
//!    before its escrow does** (answer 24); a refused accept lists nothing *and
//!    cancels the draft*, so no unfunded `Offered` contract counts against
//!    `MAX_OPEN_CONTRACTS_PER_ISSUER` (M17).
//! 2. **`+run/apply <id>=<party>`** — the party's leader offers their crew. Every
//!    member needs an `sr.nuyen` account (M6).
//! 3. **`+run/crew <id>=accept`** — the payer's answer, and an **A′ amendment**: the
//!    crew are added as parties, every outcome row is rewritten to *their* accounts,
//!    and the surface is re-offered. The payer authored the amendment, so their own
//!    acceptance and escrow stand (DL-01 A′).
//! 4. Each runner accepts through `@contract/accept`; the first acceptance locks the
//!    roster, the last activates the contract, and the board mirrors `Crewed`.
//! 5. The **named adjudicator** attests the row that happened
//!    (`@contract/attest`), and E4b2 settles it. E6a has no other route
//!    (D-E6-20).
//!
//! ## Why the payer is the issuer (D-E6-19, M1)
//!
//! `amend_contract` is issuer-only and A′ keeps only the *amending party's*
//! acceptance. A fixer-issuer would therefore refund the payer's escrow at every
//! `+run/crew accept` and leave a `Listed` entry standing on nothing. A fixer is
//! `roles.author`/`representative` with a fee row, and never the issuer.
//!
//! ## Why every share is named (D-E6-6, answer 7)
//!
//! `remainder_to` is **inert** whenever `outcomes` is non-empty, and every row's
//! payouts must sum to the escrowed total exactly. That is the only way to make "no
//! unspecified remainder to the fixer" structural rather than a promise. Payout
//! accounts at the crew are **derived from the roster's own profiles**, never from
//! input (M6).

use std::collections::BTreeMap;

use mush_core::{CmdError, Ctx, ObjId, World};
use mush_economy::{
    AccountId, AccountStatus, Amount, AttestorSet, ConsiderationSpec, ContractAmendment,
    ContractError, ContractId, ContractRoles, ContractSpec, EconomicDeadline, EconomicVisibility,
    EconomyStore, EvidenceKind, Expiry, MAX_SHARES, NamedResource, OutcomeTerms, PayoutShare,
    Share, TypedCondition,
};
use mush_faction::{FactionId, FactionState};
use mush_org::{OrgScopeId, PrincipalId};
use smol_str::SmolStr;

use crate::economy::boot::personal_account;
use crate::economy::config::{BOARD_PAGE_VISIT, MAX_RUN_APPLICANTS, SrEconomyConfig};
use crate::economy::record::{BoardVisibility, RunId, RunNotice, RunState, RunTemplate, clip};
use crate::economy::store::{SrEconomyStore, persist_meta, put_run};

/// The one refusal a run the actor may not view ever gets (08a decision 13):
/// identical for nonexistent and unviewable.
pub const NO_SUCH_RUN: &str = "No such run.";

/// What one row of the outcome table pays.
///
/// The five frozen slots of answer 7 — runner payout, fixer fee, expense advance,
/// reimbursable reserve, bonuses/penalties — are **named payout shares inside each
/// row**, not new economy fields. E6a ships three shapes of row and the reserve is a
/// share back to the payer's own account, named explicitly, because
/// `ExpenseTreatment::Returned` is declarative and read by nothing.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RowShape {
    /// The first row (`success` by convention): the crew take everything but the fee.
    Success,
    /// A middle row (`partial`): the crew take half of what is left after the fee,
    /// floored; the rest returns to the payer.
    Partial,
    /// The last row of a multi-row table (`failure`): the crew keep the advance and
    /// the **reserve** returns to the payer under `ExpenseTreatment::Returned`.
    Failure,
}

impl RowShape {
    /// Which shape row `i` of an `n`-row table is.
    #[must_use]
    pub fn of(i: usize, n: usize) -> RowShape {
        if i == 0 {
            RowShape::Success
        } else if i + 1 == n {
            RowShape::Failure
        } else {
            RowShape::Partial
        }
    }

    /// The declarative expense treatment this shape records.
    #[must_use]
    pub fn treatment(self) -> mush_economy::ExpenseTreatment {
        match self {
            RowShape::Success | RowShape::Partial => mush_economy::ExpenseTreatment::Retained,
            RowShape::Failure => mush_economy::ExpenseTreatment::Returned,
        }
    }
}

/// Split `total` across `accounts` exactly: the floor each, and the remainder handed
/// out one minor unit at a time in roster order.
///
/// Deterministic and **exact** — the sum of the returned amounts is `total` — which
/// is what makes "no remainder reaches the fixer" structural rather than a hope.
#[must_use]
pub fn even_split(total: Amount, accounts: &[AccountId]) -> Vec<(AccountId, Amount)> {
    if accounts.is_empty() || total.minor() <= 0 {
        return Vec::new();
    }
    let n = i64::try_from(accounts.len()).unwrap_or(1);
    let base = total.minor() / n;
    let rem = total.minor() % n;
    accounts
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let extra = i64::try_from(i).map(|i| i64::from(i < rem)).unwrap_or(0);
            (*a, Amount::from_minor(base.saturating_add(extra)))
        })
        .collect()
}

/// Merge shares that name the same account, preserving first-seen order, and drop
/// the zeroes.
///
/// Two shares on one account are arithmetically fine (`apply_outcome` nets them),
/// but the merged form is what `check_offerable` measures against `MAX_SHARES`, and
/// a zero share would name an account for nothing.
#[must_use]
fn merge(shares: Vec<(AccountId, Amount)>) -> Vec<PayoutShare> {
    let mut order: Vec<AccountId> = Vec::with_capacity(shares.len());
    let mut sums: BTreeMap<AccountId, i64> = BTreeMap::new();
    for (account, amount) in shares {
        if amount.minor() == 0 {
            continue;
        }
        if !sums.contains_key(&account) {
            order.push(account);
        }
        let slot = sums.entry(account).or_insert(0);
        *slot = slot.saturating_add(amount.minor());
    }
    order
        .into_iter()
        .filter_map(|a| {
            sums.get(&a).map(|n| PayoutShare {
                to: a,
                share: Share::Exact(Amount::from_minor(*n)),
            })
        })
        .collect()
}

/// Everything the outcome table is built from.
#[derive(Clone, Debug)]
pub struct RunSplit {
    /// The escrowed total.
    pub pay: Amount,
    /// The fixer's fee, paid in every row (a fixer is paid for arranging, win or
    /// lose). Zero when there is no fixer.
    pub fee: Amount,
    /// The expense advance the crew keep even on failure.
    pub advance: Amount,
    /// The payer's own account — where the reserve returns, and where the crew's
    /// share sits provisionally before the roster exists (M6).
    pub payer_account: AccountId,
    /// The fixer's account, when a fee row exists.
    pub fixer_account: Option<AccountId>,
    /// The row names, in table order.
    pub names: Vec<SmolStr>,
    /// E6b (B8): the **gating** condition indexes every row must also satisfy — a
    /// composite. They sit after the per-row conditions in the consent surface, so
    /// row `i` is decided by condition `i` **plus** all of these.
    pub gates: Vec<usize>,
}

impl RunSplit {
    /// Whether the arithmetic closes: every row must be payable out of `pay`.
    #[must_use]
    pub fn is_feasible(&self) -> bool {
        let pay = self.pay.minor();
        let fee = self.fee.minor();
        let advance = self.advance.minor();
        pay > 0
            && fee >= 0
            && advance >= 0
            && fee.saturating_add(advance) <= pay
            && !self.names.is_empty()
    }

    /// Build the outcome table against a roster.
    ///
    /// **An empty roster is a table that pays nobody but the payer** (E6a verify fold,
    /// M1). A `Listed` run's contract is `Active` — a one-party contract activates at
    /// its author's own accept (S1 deviation 2) — so the named adjudicator could
    /// attest row 0 before a single runner existed and E4 would settle it. When the
    /// rows carried a fixer fee and an advance, that settlement moved the payer's
    /// escrow to the fixer for a run nobody had been crewed onto. So: until a roster
    /// exists there is **no fee row and no advance row**; every row returns the whole
    /// `pay` to the payer's own account, and a pre-crew settlement is arithmetically a
    /// no-op that closes the run. The fee and the advance appear at
    /// `+run/crew accept`, with the roster, which is the moment a fixer has actually
    /// arranged anything.
    #[must_use]
    pub fn table(&self, crew: &[AccountId]) -> Vec<OutcomeTerms> {
        if crew.is_empty() {
            return self.pre_crew_table();
        }
        let after_fee = Amount::from_minor(self.pay.minor().saturating_sub(self.fee.minor()));
        self.names
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let shape = RowShape::of(i, self.names.len());
                let mut shares: Vec<(AccountId, Amount)> = Vec::new();
                if let Some(fixer) = self.fixer_account
                    && self.fee.minor() > 0
                {
                    shares.push((fixer, self.fee));
                }
                match shape {
                    RowShape::Success => shares.extend(even_split(after_fee, crew)),
                    RowShape::Partial => {
                        let half = Amount::from_minor(after_fee.minor() / 2);
                        shares.extend(even_split(half, crew));
                        shares.push((
                            self.payer_account,
                            Amount::from_minor(after_fee.minor().saturating_sub(half.minor())),
                        ));
                    }
                    RowShape::Failure => {
                        shares.extend(even_split(self.advance, crew));
                        shares.push((
                            self.payer_account,
                            Amount::from_minor(
                                after_fee.minor().saturating_sub(self.advance.minor()),
                            ),
                        ));
                    }
                }
                OutcomeTerms {
                    name: name.clone(),
                    // Row `i` is decided by condition `i` — one condition per row —
                    // **plus** every gating condition (E6b's composite route).
                    when: self.when(i),
                    payouts: merge(shares),
                    expense_treatment: shape.treatment(),
                    asset_consequences: Vec::new(),
                }
            })
            .collect()
    }

    /// The table a listing with no roster carries (M1): one row per outcome, the
    /// whole escrowed total back to the payer, and nothing else named at all.
    fn pre_crew_table(&self) -> Vec<OutcomeTerms> {
        self.names
            .iter()
            .enumerate()
            .map(|(i, name)| OutcomeTerms {
                name: name.clone(),
                when: self.when(i),
                payouts: merge(vec![(self.payer_account, self.pay)]),
                // Nothing was advanced, so nothing is retained or returned.
                expense_treatment: mush_economy::ExpenseTreatment::None,
                asset_consequences: Vec::new(),
            })
            .collect()
    }

    /// The condition indexes row `i` needs: its own, then every gate (E6b, B8).
    #[must_use]
    fn when(&self, i: usize) -> Vec<usize> {
        let mut when = vec![i];
        when.extend(self.gates.iter().copied());
        when
    }

    /// Whether the table it would build fits E4's caps.
    #[must_use]
    pub fn fits(&self, crew_len: usize) -> bool {
        // fixer + crew + payer, per row.
        crew_len.saturating_add(2) <= MAX_SHARES
    }
}

// ---- E6b: B8 — the other three settlement routes --------------------------

/// How one outcome row (or one gate) is decided (E6b, §4.2's four routes).
///
/// E6a settled by attestation and nothing else (D-E6-20) because no canon kind was
/// registered, there was no `sr.*` fact provider, and a bare `Before` row is `Ready`
/// the moment the last runner accepts. With `scene.outcome` registered all four
/// routes are available, and this is the ruleset's word for which one a row uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunRoute {
    /// The named adjudicator attests it (`@contract/attest`) — E6a's route.
    Attest,
    /// A **sealed scene outcome**: a canon record of `kind` naming `source`. The
    /// source is mandatory (`contract.rs`'s `check_canon_sets`) and the kind must be
    /// a producer-declared outcome kind, or the offer is refused.
    Canon {
        /// The canon kind.
        kind: SmolStr,
        /// The exact source it must name.
        source: SmolStr,
    },
    /// Satisfied **until** an absolute instant passes.
    Before {
        /// The instant, on `RealUtc`.
        at: i64,
    },
    /// Satisfied once condition `anchor` has been satisfied for `delay` seconds.
    After {
        /// The anchor condition's index.
        anchor: usize,
        /// How long after it, in seconds on `RealUtc`.
        delay: i64,
    },
    /// An external provider's fact.
    Fact {
        /// The provider namespace (`"jobs"`, `"sim"`, `"tactics"`).
        provider: SmolStr,
        /// The record kind within it.
        kind: SmolStr,
        /// The record id.
        id: SmolStr,
    },
}

impl RunRoute {
    /// The typed condition this route becomes. `adjudicator` is the set an
    /// `Attest` route names — belt-and-braces, since `ConsentSurface.adjudicator`
    /// replaces every condition's own set anyway (D-E6-8).
    #[must_use]
    pub fn condition(&self, adjudicator: &AttestorSet) -> TypedCondition {
        match self {
            RunRoute::Attest => TypedCondition::Attestation {
                by: adjudicator.clone(),
            },
            RunRoute::Canon { kind, source } => TypedCondition::Attestation {
                by: AttestorSet::Canon {
                    kind: kind.clone(),
                    source: Some(source.clone()),
                },
            },
            RunRoute::Before { at } => TypedCondition::Before {
                deadline: EconomicDeadline {
                    clock: mush_economy::EconomicClock::RealUtc,
                    at: *at,
                },
            },
            RunRoute::After { anchor, delay } => TypedCondition::After {
                condition: *anchor,
                delay: *delay,
                clock: mush_economy::EconomicClock::RealUtc,
            },
            RunRoute::Fact { provider, kind, id } => TypedCondition::Fact {
                fact: mush_economy::ExternalRef {
                    provider: provider.clone(),
                    kind: kind.clone(),
                    id: id.clone(),
                    revision: None,
                },
            },
        }
    }

    /// The board notice's word for this route — what `notice.kinds` carries, so a
    /// reader of the board knows what will end the run before they apply. It stopped
    /// being the constant `["attest"]` with E6b.
    #[must_use]
    pub fn label(&self) -> SmolStr {
        match self {
            RunRoute::Attest => SmolStr::new_static("attest"),
            RunRoute::Canon { .. } => SmolStr::new_static("canon"),
            RunRoute::Before { .. } => SmolStr::new_static("before"),
            RunRoute::After { .. } => SmolStr::new_static("after"),
            RunRoute::Fact { .. } => SmolStr::new_static("fact"),
        }
    }

    /// Whether this route is anything but an attestation — the `deterministic`
    /// switch `offer_contract` gates the non-`Attestation` alphabet behind.
    #[must_use]
    pub fn is_deterministic(&self) -> bool {
        !matches!(self, RunRoute::Attest)
    }
}

/// The consent surface's conditions: **one per outcome row, then every gate**.
///
/// The row order is load-bearing — row `i` is decided by condition `i` — and the
/// gates follow, so their indexes are `rows..rows + gates.len()`, which is exactly
/// what [`RunSplit::gates`] carries into every row's `when`.
///
/// `ConsentSurface.adjudicator` **replaces** each `Attestation` condition's own
/// attestor set (D-E6-8), so naming it here is belt-and-braces for an `Attest`
/// route: it is what a reader of the frozen surface sees, and what
/// `check_structure` measures for satisfiability. A `Canon` route names its own set
/// and is only replaced when the run also names a person adjudicator — which is the
/// documented rule, and why `+run/post` does not default one onto a routed run.
#[must_use]
pub fn conditions(
    adjudicator: &AttestorSet,
    routes: &[RunRoute],
    gates: &[RunRoute],
) -> Vec<TypedCondition> {
    routes
        .iter()
        .chain(gates.iter())
        .map(|r| r.condition(adjudicator))
        .collect()
}

/// The board notice's condition kinds: every route's label, row order then gate
/// order, deduplicated and bounded by the notice's own cap (E6b, B8).
#[must_use]
pub fn notice_kinds(routes: &[RunRoute], gates: &[RunRoute]) -> Vec<SmolStr> {
    let mut out: Vec<SmolStr> = Vec::new();
    for r in routes.iter().chain(gates.iter()) {
        let label = r.label();
        if !out.contains(&label) {
            out.push(label);
        }
    }
    if out.is_empty() {
        out.push(SmolStr::new_static("attest"));
    }
    out
}

/// The `EvidenceKind`s a routed surface permits — exactly the kinds it uses.
///
/// The permitted-evidence list is exhaustive when non-empty (08e answer 6), so this
/// is what keeps `evidence` an honest statement of what may release the money rather
/// than E6a's forced `[Attestation]`. Note there is **no** `EvidenceKind::Canon`:
/// a canon row's evidence is `Attestation`, and `TypedCondition::evidence_kind` is
/// what fills this in.
#[must_use]
pub fn evidence_of(conditions: &[TypedCondition]) -> Vec<EvidenceKind> {
    let mut out: Vec<EvidenceKind> = Vec::new();
    for c in conditions {
        let kind = c.evidence_kind();
        if !out.contains(&kind) {
            out.push(kind);
        }
    }
    out
}

/// Who may see a listing (M5). Existence hiding is the caller's: this answers the
/// question, [`NO_SUCH_RUN`] is the answer to a `false`.
#[must_use]
pub fn may_view(w: &World, viewer: ObjId, r: &RunTemplate) -> bool {
    if crate::economy::is_staff(w, viewer) {
        return true;
    }
    // The payer always sees their own listing.
    if mush_economy::acts_for(w, viewer, &r.principal) {
        return true;
    }
    match &r.board {
        // A profiled character: the board is for people who have a wallet in it.
        BoardVisibility::Public => w
            .module_state::<SrEconomyStore>()
            .is_some_and(|s| s.profile(viewer).is_some()),
        // Faction membership through `mush_faction`'s own predicates, never the
        // contract's `Audience` (M5): a hidden membership is not a board pass.
        //
        // D9: `membership_visible(viewer, false, m)` was also asked here and was
        // dead — it answers "may *this* viewer see this membership", and the viewer
        // is the member, who always may. A clause that is always true reads like a
        // second gate and is not one; `is_member` is the whole rule.
        BoardVisibility::Faction(f) => w
            .module_state::<FactionState>()
            .is_some_and(|st| st.is_member(viewer, *f)),
        // The invited party's members and invitees.
        BoardVisibility::Invitation { party } => {
            mush_party::may_view(w, viewer, mush_party::PartyId(*party))
        }
    }
}

/// Whether `who` is a member of `faction` at all (the `+run/list faction` filter).
#[must_use]
pub fn is_faction_member(w: &World, who: ObjId, faction: FactionId) -> bool {
    w.module_state::<FactionState>()
        .is_some_and(|st| st.is_member(who, faction))
}

/// One page of the board (E6a verify fold, D1).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BoardPage {
    /// The listings this page shows, newest first.
    pub ids: Vec<RunId>,
    /// Where the next page starts (`+run/list … after=<id>`), when the walk stopped
    /// before the end of the board.
    pub next: Option<RunId>,
    /// How many live listings this walk actually **touched** — what
    /// [`BOARD_PAGE_VISIT`] bounds, and what the D-LBK-5 rule is about.
    pub visited: usize,
}

/// The listings `viewer` may see, newest first, from `after` (exclusive).
///
/// **Bounded on records VISITED, not records matched** (D-LBK-5). The old shape
/// filtered the whole board and truncated the survivors, so a selective filter — one
/// faction's runs, or `mine` on a board of a hundred — walked every listing ever
/// posted while claiming a page cap. It now walks at most [`BOARD_PAGE_VISIT`] live
/// listings from the cursor, returns at most `page` matches, and hands back a
/// continuation cursor when either bound stopped it. Descending id order keeps
/// "newest first" and makes the cursor a plain `range(..after).rev()`.
#[must_use]
pub fn visible(
    w: &World,
    viewer: ObjId,
    filter: Option<&str>,
    page: usize,
    after: Option<RunId>,
) -> BoardPage {
    let Some(store) = w.module_state::<SrEconomyStore>() else {
        return BoardPage::default();
    };
    let mut out = BoardPage::default();
    let mut last: Option<RunId> = None;
    let walk: Box<dyn Iterator<Item = &RunTemplate>> = match after {
        Some(cursor) => Box::new(
            store
                .live
                .range(..cursor)
                .rev()
                .filter_map(|id| store.run(*id)),
        ),
        None => Box::new(store.live_ids_desc().filter_map(|id| store.run(id))),
    };
    for r in walk {
        if out.visited >= BOARD_PAGE_VISIT || out.ids.len() >= page {
            out.next = last;
            return out;
        }
        out.visited = out.visited.saturating_add(1);
        last = Some(r.id);
        let matches = match filter {
            Some("public") => matches!(r.board, BoardVisibility::Public),
            Some("faction") => matches!(r.board, BoardVisibility::Faction(_)),
            Some("mine") => mush_economy::acts_for(w, viewer, &r.principal),
            _ => true,
        };
        if matches && may_view(w, viewer, r) {
            out.ids.push(r.id);
        }
    }
    out
}

/// Why `+run/post` refused before any contract existed.
#[derive(Clone, Debug)]
pub enum PostError {
    /// A switch in `[shadowrun.economy]` is off.
    Disabled(&'static str),
    /// A shape or arithmetic complaint the poster can fix.
    Bad(String),
    /// A cap.
    Full(&'static str),
    /// The account named is not one the poster may use (rendered under hiding).
    Account(AccountId),
    /// The economy refused the offer or the accept.
    Contract(ContractError),
}

/// Render a [`PostError`] under the existence-hiding discipline.
#[must_use]
pub fn refuse_post(ctx: &mut Ctx<'_>, e: &PostError, named: &[NamedResource]) -> CmdError {
    match e {
        PostError::Disabled(why) => CmdError::msg((*why).to_string()),
        PostError::Bad(why) => CmdError::msg(why.clone()),
        PostError::Full(why) => CmdError::msg((*why).to_string()),
        PostError::Account(id) => {
            mush_economy::refuse_named(ctx, NamedResource::Account(*id), [], {
                format!("Account #{} cannot fund that run.", id.0)
            })
        }
        PostError::Contract(e) => mush_economy::refuse_contract_offer(ctx, e, named.to_vec()),
    }
}

/// Everything `+run/post` resolved before it touched the economy.
#[derive(Clone, Debug)]
pub struct PostRequest {
    /// The listing's name.
    pub name: SmolStr,
    /// `Some(scope)` makes the organization the issuer (D-E6-19).
    pub scope: Option<OrgScopeId>,
    /// The payer's account.
    pub account: AccountId,
    /// The payer principal, derived from the account's own owner.
    pub principal: PrincipalId,
    /// The split.
    pub split: RunSplit,
    /// The named adjudicator (defaults to `party:<payer>` — D-E6-20).
    pub adjudicator: AttestorSet,
    /// Whether the adjudicator was named rather than defaulted.
    pub adjudicator_named: bool,
    /// The visible representative, when announced.
    pub johnson: Option<PrincipalId>,
    /// The fixer, when one takes a fee row.
    pub fixer: Option<PrincipalId>,
    /// How many runners it seats.
    pub slots: u8,
    /// Who may see it.
    pub board: BoardVisibility,
    /// The deadline instant on `RealUtc`.
    pub expires_at: i64,
    /// E6b (B8): how each outcome row is decided, in table order.
    pub routes: Vec<RunRoute>,
    /// E6b (B8): the gating conditions every row must also satisfy.
    pub gates: Vec<RunRoute>,
}

/// **Draft, offer and accept in one call** (§3.2 "Board", answer 24).
///
/// Returns the listing. Nothing is written unless the escrow was placed: a refused
/// accept cancels the draft and reports the economy's own refusal (M17).
///
/// # Errors
///
/// Every [`PostError`]; render it with [`refuse_post`].
pub fn post(
    ctx: &mut Ctx<'_>,
    cfg: &SrEconomyConfig,
    req: PostRequest,
) -> Result<RunId, (PostError, Vec<NamedResource>)> {
    let named = vec![NamedResource::Account(req.account)];
    if !req.split.is_feasible() {
        return Err((
            PostError::Bad(
                "The pay must be positive and cover the fee and the advance together.".to_string(),
            ),
            named,
        ));
    }
    if !req.split.fits(usize::from(req.slots)) {
        return Err((
            PostError::Full("That crew is too large for one outcome row."),
            named,
        ));
    }
    // The caps, before anything exists.
    let (open_all, open_mine) = ctx
        .w
        .module_state::<SrEconomyStore>()
        .map(|s| (s.open_runs(), s.open_runs_of(&req.principal)))
        .unwrap_or((0, 0));
    if open_all >= cfg.board_cap {
        return Err((PostError::Full("The run board is full."), named));
    }
    if open_mine >= cfg.open_runs_cap() {
        return Err((
            PostError::Full("You already have as many runs listed as the board allows."),
            named,
        ));
    }
    let rows = req.split.names.len();
    // E6b (B8): `run:self` names **this** run — the id the offer below is about to
    // mint. The seal's source is `run:{contract id}` and a `scene:{uid}` is
    // unknowable at offer, so a canon-routed listing has to be able to write its own
    // id into a condition that freezes before the id exists.
    let next = ctx
        .w
        .module_state::<EconomyStore>()
        .map_or(0, EconomyStore::peek_next_contract);
    let mut req = req;
    // E6b verify fold (M5): a canon source names the **row**, not just the run.
    // `CanonRecorded` carries no `detail`, so the economy matches on `(kind, source)`
    // alone — with one source per run, one seal filled *every* canon row's slot, and
    // `resolve_outcome` then read `Undecidable` and disputed. Worse, with a single
    // canon row the row name the sealer typed was decoration: `+scene/outcome
    // failure=…` paid the success row. `run:{id}#{row}` is what makes the seal
    // select the outcome it names.
    let names = req.split.names.clone();
    for (i, route) in req.routes.iter_mut().enumerate() {
        if let RunRoute::Canon { source, .. } = route
            && source.as_str() == "run:self"
        {
            let row = names.get(i).cloned().unwrap_or_default();
            *source = SmolStr::new(format!("run:{next}#{row}"));
        }
    }
    // A **gate** is not a row, so it names the run alone.
    for route in req.gates.iter_mut() {
        if let RunRoute::Canon { source, .. } = route
            && source.as_str() == "run:self"
        {
            *source = SmolStr::new(format!("run:{next}"));
        }
    }
    // E6b verify fold (M11), **inverted to an allowlist by the independent fold
    // (D1, and it bounds the source for D6)**. The refusal used to name the one shape
    // it knew was wrong — a `run:<n>` that is not this run — so every *other* prefix
    // passed: `route=success:canon:scene.outcome:scene:41` listed happily, and a
    // scene bound to no run at all seals under exactly `scene:{uid}`. Staff's
    // "chronicle only" conferral was therefore a live settlement authority for any
    // run that named the uid, and a listing could cite a seal that was never meant to
    // settle anything.
    //
    // So: after the `run:self` substitution above, a canon source is **exactly**
    // `run:{next}` or `run:{next}#{row}`, and nothing else lists. `run:self` is the
    // only spelling a poster can write, which is the point — the source of a seal is
    // not a free-text field on a run board.
    let allowed_prefix = format!("run:{next}");
    let source_ok = |source: &SmolStr| -> bool {
        match source.strip_prefix(&allowed_prefix) {
            Some("") => true,
            Some(rest) => rest
                .strip_prefix('#')
                .is_some_and(|row| names.iter().any(|n| n.as_str() == row)),
            None => false,
        }
    };
    if req
        .routes
        .iter()
        .chain(req.gates.iter())
        .any(|r| matches!(r, RunRoute::Canon { source, .. } if !source_ok(source)))
    {
        return Err((
            PostError::Bad(
                "A run's canon route names its own run and one of its own outcome rows \
                 (route=<row>:canon:<kind>:run:self)."
                    .to_string(),
            ),
            named,
        ));
    }
    let req = req;
    // E6b (B8): the four routes, one condition per row plus the gates. The
    // permitted-evidence list is exactly the kinds the surface uses, and
    // `deterministic` is set from them rather than hard-`false`.
    let conditions = conditions(&req.adjudicator, &req.routes, &req.gates);
    let evidence = evidence_of(&conditions);
    let deterministic = req
        .routes
        .iter()
        .chain(req.gates.iter())
        .any(RunRoute::is_deterministic);
    let _ = rows;
    let spec = ContractSpec {
        name: req.name.clone(),
        scope: req.scope,
        parties: Vec::new(),
        considerations: vec![ConsiderationSpec {
            account: req.account,
            amount: req.split.pay,
        }],
        outcomes: req.split.table(&[]),
        // Inert under a table, but it must still resolve to a live account in the
        // instrument (M18): the payer's own.
        remainder_to: req.account,
        conditions,
        // E6b: a run with any non-`Attest` route names **no person adjudicator**
        // unless one was asked for by name — a named adjudicator replaces every
        // condition's own set (D-E6-8), which would make a canon row settle on that
        // person's word instead of on the seal. `adjudicator_named` is exactly that
        // question.
        adjudicator: (req.adjudicator_named || !deterministic).then(|| req.adjudicator.clone()),
        evidence,
        deadline: Some(EconomicDeadline {
            clock: mush_economy::EconomicClock::RealUtc,
            at: req.expires_at,
        }),
        // D-E6-22: a crewed run's deadline disputes; an unbound one refunds
        // regardless, because `expire`'s forfeit needs a fully-accepted contract with
        // more than one party.
        on_expiry: Expiry::Dispute,
        // The crew are parties, so `Parties` discloses to exactly the right people
        // and the board's own notice does the rest (G7).
        visibility: EconomicVisibility::Parties,
        roles: ContractRoles {
            author: Some(PrincipalId::Object(ctx.actor)),
            payer: Some(req.principal.clone()),
            representative: req.johnson.clone().or_else(|| req.fixer.clone()),
            facilitator: None,
        },
        deterministic,
        deliverables: Vec::new(),
    };
    let contract = mush_economy::offer_contract(ctx, spec)
        .map_err(|e| (PostError::Contract(e), named.clone()))?;
    // The payer's own accept is what places the escrow (answer 24).
    match mush_economy::accept_contract(ctx, contract) {
        Ok(mush_economy::AcceptOutcome::Accepted { .. }) => {}
        Ok(mush_economy::AcceptOutcome::NeedsApproval(id)) => {
            let _ = mush_economy::cancel_contract(ctx, contract);
            return Err((
                PostError::Bad(format!(
                    "That run needs approval on the funding account first (request #{}). Nothing \
                     was listed.",
                    id.0
                )),
                named,
            ));
        }
        Err(e) => {
            // M17: a refused accept lists nothing AND cancels the draft, so no
            // unfunded `Offered` contract counts against the issuer's cap.
            let _ = mush_economy::cancel_contract(ctx, contract);
            return Err((PostError::Contract(e), named));
        }
    }
    let Some(id) = ctx
        .w
        .module_state_mut::<SrEconomyStore>()
        .and_then(SrEconomyStore::alloc_run)
    else {
        let _ = mush_economy::cancel_contract(ctx, contract);
        return Err((PostError::Full("The run-id allocator is exhausted."), named));
    };
    let faction = match &req.board {
        BoardVisibility::Faction(f) => Some(*f),
        BoardVisibility::Public | BoardVisibility::Invitation { .. } => None,
    };
    let now = ctx.w.now();
    let run = RunTemplate {
        id,
        contract,
        principal: req.principal,
        account: req.account,
        notice: RunNotice {
            name: SmolStr::new(clip(&req.name, crate::economy::config::MAX_RUN_NAME_BYTES)),
            window: req.expires_at.saturating_sub(now),
            // E6b (B8): the notice says how the run actually settles, in row order
            // then gate order, deduplicated — no longer the constant `["attest"]`.
            kinds: notice_kinds(&req.routes, &req.gates),
            johnson: req.johnson,
        },
        board: req.board,
        faction,
        slots: req.slots,
        pay: req.split.pay,
        fee: req.split.fee,
        advance: req.split.advance,
        fixer_account: req.split.fixer_account,
        outcomes: req.split.names.clone(),
        gates: req.split.gates.clone(),
        posted_at: now,
        deadline: EconomicDeadline {
            clock: mush_economy::EconomicClock::RealUtc,
            at: req.expires_at,
        },
        expires_at: req.expires_at,
        applied: Vec::new(),
        party: None,
        state: RunState::Listed,
        revision: 1,
    };
    put_run(ctx, run);
    persist_meta(ctx);
    Ok(id)
}

/// Why a board action after the post was refused.
#[derive(Clone, Debug)]
pub enum BoardError {
    /// No such run, or none this actor may see.
    NoStanding,
    /// The listing is not in a state that admits this.
    State(&'static str),
    /// A shape complaint the actor can fix.
    Bad(String),
    /// The economy refused.
    Contract(ContractError),
    /// The party subsystem refused.
    Party(&'static str),
}

/// Render a [`BoardError`] under the existence-hiding discipline.
#[must_use]
pub fn refuse_board(ctx: &mut Ctx<'_>, contract: Option<ContractId>, e: &BoardError) -> CmdError {
    match e {
        // E6b verify fold (D8): the board's own existence-hiding refusal is counted
        // too — `RunId` is as enumerable as any other id space.
        BoardError::NoStanding => {
            mush_economy::note_refusal(ctx, true);
            CmdError::msg(NO_SUCH_RUN)
        }
        BoardError::State(why) | BoardError::Party(why) => CmdError::msg((*why).to_string()),
        BoardError::Bad(why) => CmdError::msg(why.clone()),
        BoardError::Contract(e) => match contract.and_then(|id| {
            ctx.w
                .module_state::<EconomyStore>()
                .and_then(|s| s.snapshot().contract(id).cloned())
        }) {
            Some(c) => mush_economy::refuse_contract(ctx, &c, e),
            None => mush_economy::refuse_contract_offer(ctx, e, []),
        },
    }
}

/// `+run/apply <id>=<party>` — the leader offers their party.
///
/// Refuses a member with no `sr.nuyen` account (M6): a runner who cannot be paid
/// cannot be named in a payout row, and discovering that at the amendment would
/// refund the payer's escrow for a reason nobody could see at apply time.
///
/// # Errors
///
/// [`BoardError`]; render it with [`refuse_board`].
pub fn apply(ctx: &mut Ctx<'_>, id: RunId, party: u64) -> Result<String, BoardError> {
    let Some(mut run) = live_run(ctx, id) else {
        return Err(BoardError::NoStanding);
    };
    if !run.state.is_open() {
        return Err(BoardError::State(
            "That run is no longer taking applications.",
        ));
    }
    let Some(members) = party_members(ctx, party) else {
        return Err(BoardError::Party(mush_party::NO_SUCH_PARTY));
    };
    if members.is_empty() {
        return Err(BoardError::Party("That party has no members."));
    }
    if members.len() > usize::from(run.slots) {
        return Err(BoardError::Bad(format!(
            "That run seats {}; your party has {}.",
            run.slots,
            members.len()
        )));
    }
    if ctx
        .w
        .module_state::<SrEconomyStore>()
        .is_some_and(|s| s.party_has_live_run(party))
    {
        return Err(BoardError::State("That party already has a live run."));
    }
    if crew_accounts(ctx, &members).is_none() {
        return Err(BoardError::Bad(
            "Every member needs a nuyen account before the party can apply.".to_string(),
        ));
    }
    if run.applied.contains(&party) {
        return Ok("Your party has already applied.".to_string());
    }
    if run.applied.len() >= MAX_RUN_APPLICANTS {
        return Err(BoardError::State(
            "That run has all the applications it can hold.",
        ));
    }
    run.applied.push(party);
    run.revision = run.revision.saturating_add(1);
    put_run(ctx, run);
    Ok("Your party has applied.".to_string())
}

/// `+run/crew <id>=accept` — the payer's answer, and the **A′ amendment** (M6,
/// DL-01 A′).
///
/// The crew are added as parties, every outcome row is rewritten to the members'
/// own `SrProfile.account`s — derived from the roster, never from input — and the
/// surface is re-offered. The payer authored the amendment, so their acceptance and
/// escrow stand.
///
/// # Errors
///
/// [`BoardError`]; render it with [`refuse_board`].
pub fn crew_accept(ctx: &mut Ctx<'_>, id: RunId, party: Option<u64>) -> Result<String, BoardError> {
    let Some(mut run) = live_run(ctx, id) else {
        return Err(BoardError::NoStanding);
    };
    if !is_payer(ctx, &run) {
        return Err(BoardError::NoStanding);
    }
    if !run.state.is_open() {
        return Err(BoardError::State("That run is no longer taking a crew."));
    }
    let Some(party) = party.or_else(|| run.applied.first().copied()) else {
        return Err(BoardError::State("Nobody has applied to that run."));
    };
    if !run.applied.contains(&party) {
        return Err(BoardError::Party(mush_party::NO_SUCH_PARTY));
    }
    let Some(members) = party_members(ctx, party) else {
        return Err(BoardError::Party(mush_party::NO_SUCH_PARTY));
    };
    if members.is_empty() {
        return Err(BoardError::Party("That party has no members."));
    }
    // M9 + M8: the adjudicator is never a party, and the check **fails closed**. The
    // old `adjudicator_is` matched `AttestorSet::Party` alone, so a contract staff had
    // amended to a `Capability`, `Staff` or `Canon` attestor set answered "not one of
    // them" for every roster — a set that can include a runner passing a check
    // designed to exclude one. Anything but a single named party refuses the crewing
    // and says so; the payer can re-amend it back.
    match effective_attestors(ctx, run.contract) {
        AdjudicatorShape::Party(p) => {
            if members
                .iter()
                .any(|m| adjudicator_is(&AttestorSet::Party(p.clone()), PrincipalId::Object(*m)))
            {
                return Err(BoardError::Bad(
                    "That party includes the run's adjudicator; they cannot be paid by the run \
                     they settle."
                        .to_string(),
                ));
            }
        }
        // E6b: an impersonal settlement route has no adjudicator a runner could be.
        AdjudicatorShape::Impersonal => {}
        AdjudicatorShape::Other => {
            return Err(BoardError::Bad(
                "That run no longer settles on one named adjudicator, so who would be paid by \
                 whom cannot be established. Nothing was crewed."
                    .to_string(),
            ));
        }
    }
    let Some(accounts) = crew_accounts(ctx, &members) else {
        return Err(BoardError::Bad(
            "A member of that party has no nuyen account.".to_string(),
        ));
    };
    let split = split_of(&run);
    if !split.fits(accounts.len()) {
        return Err(BoardError::Bad(
            "That crew is too large for one outcome row.".to_string(),
        ));
    }
    // **Every party check, and the lock itself, happen BEFORE the amendment**
    // (E6a verify fold, M2 + R-M2). `party_has_live_run` was asked at `+run/apply`
    // and never again, and `lock` came *after* `amend_contract` with its failure
    // downgraded to a `warn!` — so one party could be crewed onto two runs and paid
    // by both, and a party disbanded between the apply and the accept was crewed
    // anyway. `lock` is now the gate: it refuses anything that is not `Forming`
    // (already locked, already disbanded), and a refusal here posts nothing at all.
    if ctx
        .w
        .module_state::<SrEconomyStore>()
        .is_some_and(|s| s.party_has_live_run(party))
    {
        return Err(BoardError::Party("That party already has a live run."));
    }
    if let Err(e) = mush_party::lock(
        ctx,
        mush_party::PartyId(party),
        mush_party::ContractRef(run.contract.0),
    ) {
        tracing::info!(
            run = id.0, party, error = %e,
            "sr.economy: a crew accept was refused because the party would not lock; nothing was \
             amended"
        );
        return Err(BoardError::Party(
            "That party is no longer available to crew.",
        ));
    }
    let amendment = ContractAmendment {
        add_parties: members.iter().map(|m| PrincipalId::Object(*m)).collect(),
        outcomes: Some(split.table(&accounts)),
        on_expiry: None,
        reoffer: true,
    };
    let amended = match mush_economy::amend_contract(ctx, run.contract, amendment) {
        Ok(mush_economy::AmendReoffer::Amended(_)) => Ok(()),
        // The amendment stands as a `Draft`; the payer retries or withdraws
        // (M10 — the sweep expires an escrowed draft on its deadline).
        Ok(mush_economy::AmendReoffer::NotReoffered(_, e)) | Err(e) => Err(e),
    };
    if let Err(e) = amended {
        // The lock landed and the amendment did not: give the party back rather than
        // leaving it frozen against a contract it was never added to.
        if let Err(unlocked) = mush_party::unlock(ctx, mush_party::PartyId(party)) {
            tracing::error!(
                run = id.0, party, error = %unlocked,
                "sr.economy: a refused crew amendment could not release the party it had just \
                 locked (staff: the party is frozen against a contract it never joined)"
            );
        }
        return Err(BoardError::Contract(e));
    }
    run.party = Some(party);
    run.applied.clear();
    run.state = RunState::Crewing;
    run.revision = run.revision.saturating_add(1);
    put_run(ctx, run);
    Ok("Crew accepted. Each runner must now accept the contract (@contract/accept).".to_string())
}

/// `+run/crew <id>=decline` — the payer turns the applications away.
///
/// # Errors
///
/// [`BoardError`]; render it with [`refuse_board`].
pub fn crew_decline(ctx: &mut Ctx<'_>, id: RunId) -> Result<String, BoardError> {
    let Some(mut run) = live_run(ctx, id) else {
        return Err(BoardError::NoStanding);
    };
    if !is_payer(ctx, &run) {
        return Err(BoardError::NoStanding);
    }
    if run.applied.is_empty() {
        return Ok("Nobody has applied to that run.".to_string());
    }
    run.applied.clear();
    run.revision = run.revision.saturating_add(1);
    put_run(ctx, run);
    Ok("Applications declined.".to_string())
}

/// Which slot `+run/split` edits.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SplitSlot {
    /// The fixer's fee.
    Fee,
    /// The expense advance.
    Advance,
}

/// `+run/split <id>=<slot> <exact|N‰>` — edit a slot while still `Listed`.
///
/// An **A′ amendment**: the payer authored it, so their acceptance and escrow stand
/// and the amendment generation bumps only when a refund actually posts (M16) — so a
/// run can be re-split without exhausting `MAX_AMENDMENTS`.
///
/// # Errors
///
/// [`BoardError`]; render it with [`refuse_board`].
pub fn split(
    ctx: &mut Ctx<'_>,
    id: RunId,
    slot: SplitSlot,
    amount: Amount,
) -> Result<String, BoardError> {
    let Some(mut run) = live_run(ctx, id) else {
        return Err(BoardError::NoStanding);
    };
    if !is_payer(ctx, &run) {
        return Err(BoardError::NoStanding);
    }
    if !run.state.is_open() {
        return Err(BoardError::State(
            "That run has a crew; its shares are frozen.",
        ));
    }
    if amount.minor() < 0 {
        return Err(BoardError::Bad("A share is never negative.".to_string()));
    }
    if slot == SplitSlot::Fee && run.fixer_account.is_none() && amount.minor() > 0 {
        return Err(BoardError::Bad(
            "That run has no fixer to pay a fee to.".to_string(),
        ));
    }
    let mut next = split_of(&run);
    match slot {
        SplitSlot::Fee => next.fee = amount,
        SplitSlot::Advance => next.advance = amount,
    }
    if !next.is_feasible() {
        return Err(BoardError::Bad(
            "The fee and the advance together must fit inside the pay.".to_string(),
        ));
    }
    let amendment = ContractAmendment {
        add_parties: Vec::new(),
        outcomes: Some(next.table(&[])),
        on_expiry: None,
        reoffer: true,
    };
    match mush_economy::amend_contract(ctx, run.contract, amendment) {
        Ok(mush_economy::AmendReoffer::Amended(_)) => {}
        Ok(mush_economy::AmendReoffer::NotReoffered(_, e)) | Err(e) => {
            return Err(BoardError::Contract(e));
        }
    }
    run.fee = next.fee;
    run.advance = next.advance;
    run.revision = run.revision.saturating_add(1);
    put_run(ctx, run);
    Ok("The split is amended.".to_string())
}

/// `+run/withdraw <id>` — the payer takes a listing down; the escrow refunds through
/// the contract's own cancellation.
///
/// **Only while nobody else is bound** (E6a verify fold, M5). `cancel_contract`
/// admits A′'s other face — a live contract only its own author has accepted — and
/// that is exactly a `Listed` run, so `withdraw` had no state check at all and the
/// payer could tear up a run runners had already accepted the moment the amendment
/// re-offer had not yet collected every signature. "A live agreement is not one
/// party's to tear up" is a rule about *other people's* consent, and a `Crewing` run
/// with an acceptance on it has some. The routes after that are the ones the design
/// names: `+run/crew decline` (which unlocks the party and re-lists), the deadline,
/// or a dispute.
///
/// # Errors
///
/// [`BoardError`]; render it with [`refuse_board`].
pub fn withdraw(ctx: &mut Ctx<'_>, id: RunId) -> Result<String, BoardError> {
    let Some(mut run) = live_run(ctx, id) else {
        return Err(BoardError::NoStanding);
    };
    if !is_payer(ctx, &run) {
        return Err(BoardError::NoStanding);
    }
    if !withdrawable(ctx, &run) {
        return Err(BoardError::State(
            "Runners have already bound themselves to that run; it is not yours to withdraw. Use \
             +run/crew decline while it is still listed, or let the deadline settle it.",
        ));
    }
    let done = mush_economy::cancel_contract(ctx, run.contract).map_err(BoardError::Contract)?;
    if !done {
        return Ok(format!(
            "Run #{id} is ending, but its refund was refused: the escrow stays reserved while the \
             sweep retries, and staff have been paged."
        ));
    }
    run.state = RunState::Closed;
    run.applied.clear();
    run.revision = run.revision.saturating_add(1);
    let party = run.party;
    put_run(ctx, run);
    release_party(ctx, party, mush_party::DisbandCause::Released);
    Ok(format!("Run #{id} is withdrawn and the escrow refunded."))
}

// ---- the pieces every verb shares -----------------------------------------

/// The listing, if it exists, is not closed, and the actor may see it.
#[must_use]
pub fn live_run(ctx: &Ctx<'_>, id: RunId) -> Option<RunTemplate> {
    let run = ctx
        .w
        .module_state::<SrEconomyStore>()
        .and_then(|s| s.run(id).cloned())?;
    if run.state.is_closed() || !may_view(ctx.w, ctx.actor, &run) {
        return None;
    }
    Some(run)
}

/// Whether a listing is still the payer's alone to take down (M5).
///
/// `Listed` always is: the only acceptance on the contract is the payer's own, which
/// is what placed the escrow. `Crewing` is only while **no runner has accepted yet** —
/// asked of the contract's own `acceptances`, never of the board's mirror, because the
/// board hears about an acceptance one `ContractChanged` later. `Crewed` and `Closed`
/// never are.
#[must_use]
pub fn withdrawable(ctx: &Ctx<'_>, run: &RunTemplate) -> bool {
    match run.state {
        RunState::Listed => true,
        RunState::Crewed | RunState::Closed => false,
        RunState::Crewing => ctx
            .w
            .module_state::<EconomyStore>()
            .and_then(|s| s.snapshot().contract(run.contract).cloned())
            .is_some_and(|c| {
                c.acceptances.iter().all(|a| {
                    mush_economy::principal_key(&a.party) == mush_economy::principal_key(&c.issuer)
                })
            }),
    }
}

/// Whether the actor speaks for the listing's payer.
#[must_use]
pub fn is_payer(ctx: &Ctx<'_>, run: &RunTemplate) -> bool {
    crate::economy::is_staff(ctx.w, ctx.actor)
        || mush_economy::acts_for(ctx.w, ctx.actor, &run.principal)
}

/// The split a listing carries, rebuilt from its own fields.
#[must_use]
pub fn split_of(run: &RunTemplate) -> RunSplit {
    RunSplit {
        pay: run.pay,
        fee: run.fee,
        advance: run.advance,
        payer_account: run.account,
        fixer_account: run.fixer_account,
        names: run.outcomes.clone(),
        gates: run.gates.clone(),
    }
}

/// The party's accepted members, in roster order.
#[must_use]
pub fn party_members(ctx: &Ctx<'_>, party: u64) -> Option<Vec<ObjId>> {
    let store = ctx.w.module_state::<mush_party::PartyStore>()?;
    let p = store.party(mush_party::PartyId(party))?;
    Some(p.members.clone())
}

/// Every member's `sr.nuyen` account, in roster order — `None` if any of them has
/// none (M6: payout accounts are derived from the roster, never from input).
#[must_use]
pub fn crew_accounts(ctx: &Ctx<'_>, members: &[ObjId]) -> Option<Vec<AccountId>> {
    let instrument = ctx
        .w
        .module_state::<SrEconomyStore>()
        .and_then(|s| s.instrument.clone())?;
    let snapshot_ok = |id: AccountId| {
        ctx.w
            .module_state::<EconomyStore>()
            .and_then(|s| {
                s.snapshot()
                    .account(id)
                    .map(|a| a.status == AccountStatus::Active)
            })
            .unwrap_or(false)
    };
    members
        .iter()
        .map(|m| {
            ctx.w
                .module_state::<SrEconomyStore>()
                .and_then(|s| s.profile(*m).and_then(|p| p.account))
                .or_else(|| personal_account(ctx, *m, &instrument))
                .filter(|id| snapshot_ok(*id))
        })
        .collect()
}

/// The contract's named adjudicator.
#[must_use]
pub fn adjudicator_of(ctx: &Ctx<'_>, contract: ContractId) -> Option<AttestorSet> {
    ctx.w
        .module_state::<EconomyStore>()?
        .snapshot()
        .contract(contract)?
        .consent
        .adjudicator
        .clone()
}

/// Whether an attestor set names exactly this principal.
#[must_use]
pub fn adjudicator_is(set: &AttestorSet, who: PrincipalId) -> bool {
    matches!(set, AttestorSet::Party(p) if *p == who)
}

/// What shape the authority that will settle a run has (E6a verify fold, M8).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdjudicatorShape {
    /// E6b: **nobody decides** — every condition is deterministic or canon, so no
    /// person could be paid by the run they settle and the crew check has nothing
    /// to find. Crewable.
    Impersonal,
    /// Exactly one named principal decides — what `+run/post` builds and the only
    /// shape the "no adjudicator on the roster" rule can be evaluated against.
    Party(PrincipalId),
    /// Anything else: a `Capability`/`Staff`/`Canon` set, several different sets
    /// across the conditions, or no attestation conditions at all. The crew check
    /// **fails closed** on this.
    Other,
}

/// The authority that will settle `contract`, reduced to [`AdjudicatorShape`].
///
/// `ConsentSurface.adjudicator` replaces each condition's own attestor set when it is
/// set, so it wins; with none set, every condition's own set must agree and every one
/// must be a single named party.
#[must_use]
pub fn effective_attestors(ctx: &Ctx<'_>, contract: ContractId) -> AdjudicatorShape {
    let Some(c) = ctx
        .w
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().contract(contract).cloned())
    else {
        return AdjudicatorShape::Other;
    };
    adjudicator_shape(c.consent.adjudicator.as_ref(), &c.consent.conditions)
}

/// [`effective_attestors`] as a pure function of the consent surface's two attestor
/// sources — the half worth asserting directly (M8).
#[must_use]
pub fn adjudicator_shape(
    adjudicator: Option<&AttestorSet>,
    conditions: &[TypedCondition],
) -> AdjudicatorShape {
    if let Some(set) = adjudicator {
        return match set {
            AttestorSet::Party(p) => AdjudicatorShape::Party(p.clone()),
            // E6b: a canon set names **no person** — the producer's own gate decides
            // who may seal, and no runner can be it. There is therefore no conflict
            // of interest for the crew check to find.
            AttestorSet::Canon { .. } => AdjudicatorShape::Impersonal,
            AttestorSet::Capability { .. } | AttestorSet::Staff => AdjudicatorShape::Other,
        };
    }
    // E6b (B8): with no adjudicator named, every condition's own set governs. A
    // deterministic or canon condition names nobody; a `Party` one names one person,
    // and there must be exactly one of them across the whole surface. A `Capability`
    // or `Staff` set is still refused — it can silently include a runner.
    let mut only: Option<PrincipalId> = None;
    let mut impersonal = false;
    for con in conditions {
        match con {
            TypedCondition::Attestation {
                by: AttestorSet::Party(p),
            } => match &only {
                None => only = Some(p.clone()),
                Some(seen) if seen == p => {}
                Some(_) => return AdjudicatorShape::Other,
            },
            TypedCondition::Attestation {
                by: AttestorSet::Canon { .. },
            }
            | TypedCondition::Before { .. }
            | TypedCondition::After { .. }
            | TypedCondition::Fact { .. }
            | TypedCondition::DepositInto { .. }
            | TypedCondition::TitleHeld { .. } => impersonal = true,
            TypedCondition::Attestation {
                by: AttestorSet::Capability { .. } | AttestorSet::Staff,
            } => return AdjudicatorShape::Other,
        }
    }
    match (only, impersonal) {
        (Some(p), _) => AdjudicatorShape::Party(p),
        (None, true) => AdjudicatorShape::Impersonal,
        (None, false) => AdjudicatorShape::Other,
    }
}

/// Release a locked party, if there is one, and never let its refusal matter: the
/// contract already ended, and a party that cannot be released is a board note, not
/// a money fault.
pub fn release_party(ctx: &mut Ctx<'_>, party: Option<u64>, how: mush_party::DisbandCause) {
    let Some(party) = party else {
        return;
    };
    if let Err(e) = mush_party::release(ctx, mush_party::PartyId(party), how) {
        tracing::info!(party, error = %e, "sr.economy: a crewed party was already released");
    }
}

#[cfg(test)]
mod tests {
    use super::{AdjudicatorShape, adjudicator_shape};
    use mush_economy::{AttestorSet, TypedCondition};
    use mush_org::{CapabilityId, PrincipalId};

    fn party(n: u32) -> PrincipalId {
        PrincipalId::Object(mush_core::ObjId(n))
    }
    fn attest(by: AttestorSet) -> TypedCondition {
        TypedCondition::Attestation { by }
    }

    // ---- E6a verify fold ----

    /// M8: the crew check reads **one named party or nothing**. `adjudicator_is`
    /// matched `AttestorSet::Party` alone and answered "not on the roster" for every
    /// other shape, so a contract amended to `Staff` or a `Capability` set — either of
    /// which can include a runner — passed a check written to exclude one.
    #[test]
    fn vf_m8_only_a_single_named_party_is_a_usable_adjudicator() {
        let one = AttestorSet::Party(party(7));
        assert_eq!(
            adjudicator_shape(Some(&one), &[]),
            AdjudicatorShape::Party(party(7))
        );
        for other in [
            AttestorSet::Staff,
            AttestorSet::Capability {
                scope: None,
                capability: CapabilityId::new("economy.account.view").expect("well-formed"),
            },
        ] {
            assert_eq!(
                adjudicator_shape(Some(&other), &[]),
                AdjudicatorShape::Other,
                "{other:?} was read as a named adjudicator"
            );
        }
        // E6b (B8): a canon adjudicator names no **person** at all — who may seal
        // is the producer's gate, and no runner can be it — so it is not "a named
        // adjudicator" and it is not the fail-closed `Other` either. The crew check
        // has nothing to find, and the run is crewable.
        assert_eq!(
            adjudicator_shape(
                Some(&AttestorSet::Canon {
                    kind: smol_str::SmolStr::new("scene.outcome"),
                    source: Some(smol_str::SmolStr::new("front:1")),
                }),
                &[]
            ),
            AdjudicatorShape::Impersonal
        );
    }

    /// M8, the no-adjudicator branch: with no `ConsentSurface.adjudicator` every
    /// condition's own set governs, so they must **all** name the same single party.
    /// An empty condition list, a mixed list and a non-attestation row all fail closed.
    #[test]
    fn vf_m8_conditions_must_agree_on_one_party_when_no_adjudicator_is_named() {
        let same = vec![
            attest(AttestorSet::Party(party(7))),
            attest(AttestorSet::Party(party(7))),
        ];
        assert_eq!(
            adjudicator_shape(None, &same),
            AdjudicatorShape::Party(party(7))
        );
        let mixed = vec![
            attest(AttestorSet::Party(party(7))),
            attest(AttestorSet::Party(party(8))),
        ];
        assert_eq!(adjudicator_shape(None, &mixed), AdjudicatorShape::Other);
        let staffed = vec![
            attest(AttestorSet::Party(party(7))),
            attest(AttestorSet::Staff),
        ];
        assert_eq!(adjudicator_shape(None, &staffed), AdjudicatorShape::Other);
        assert_eq!(adjudicator_shape(None, &[]), AdjudicatorShape::Other);
    }
}
