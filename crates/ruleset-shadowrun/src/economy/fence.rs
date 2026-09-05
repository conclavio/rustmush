//! The fence (E6b, 08f §4.3, answers 13–15, D-E6-9..D-E6-11).
//!
//! A fence **buys custody, never title**. The deliverable is a `DepositInto` and a
//! fence template carries no `TitleTransfer` at all, so the seller's title record and
//! any `Disputed` encumbrance survive the settlement untouched: the money changing
//! hands is evidence, not guilt (D-E6-10, answer 14).
//!
//! ## The flow, and why it is in that order
//!
//! 1. **`+fence/quote <fence>=<obj> [immediate|consign]`** prices the object
//!    deterministically and stores a [`FenceQuote`] with an expiry on `RealUtc`.
//! 2. **`+fence/accept <quote>`** drafts and offers the contract with the seller as
//!    issuer, and the **fence accepts first** — programmatically, through DL-11's
//!    `with_party_caps` (M15), because a staff-built NPC fence org has no officer to
//!    do it by hand and requiring one would put a staffer in every deal. That accept
//!    is what places the escrow.
//! 3. The seller binds with `@contract/accept`, which activates the contract.
//! 4. **`+fence/deliver <quote>`** hands custody over — and is **refused while the
//!    contract is `Offered`** (D-E6-9, answer 15): E4b2 custody-locks a `DepositInto`
//!    object only for an `Active`/`ReadyToSettle` contract, so a seller delivering
//!    early would hand over an unlocked, unpaid object.
//!
//! An immediate sale settles on the deposit alone; a consignment adds an
//! `After { condition: <deposit>, delay, clock: RealUtc }`, with the delay clamped to
//! answer 15's 24–72 hours and **shown in the offer**.
//!
//! ## Deterministic pricing, strict visibility (D-E6-11)
//!
//! The quote is computed from **three declared inputs and nothing else**: the
//! source-controlled `[shadowrun.economy.fence_baseline]` reference price for the
//! asset's own `CatalogRef.code`, the settlement rate in permille
//! (`fence_rate_immediate` / `fence_rate_consign`), and the fence's own declared
//! demand adjustment. An object whose asset record carries no catalog reference
//! **cannot be quoted** — refused, never guessed — and the price is never derived
//! from the `AcquisitionRecord`, which is provenance the fence may not see.
//!
//! Nothing gated is read at all. Not "read and ignored": the pricing function never
//! looks at `encumbrances`, at `stolen`, at `acquisition`, or at the title owner. So
//! two worlds differing only in hidden state produce byte-identical quotes, and a low
//! quote is never an oracle for a covert theft report.
//!
//! `observed` binds the two revisions the quote was priced against — the asset's and
//! the fence's — and a moved one refuses **deterministically** rather than re-pricing
//! (D-E6-13, scenario 29).

use mush_core::{Ctx, ObjId};
use mush_economy::{
    AccountId, Amount, ConsiderationSpec, ContractError, ContractId, ContractRoles, ContractSpec,
    EconomicDeadline, EconomicVisibility, EconomyStore, EvidenceKind, Expiry, NamedResource,
    OutcomeTerms, PayoutShare, Share, TypedCondition, VaultId,
};
use mush_org::{OrgId, PrincipalId};
use smol_str::SmolStr;

use crate::economy::config::{FENCE_QUOTE_TTL_SECS, MAX_OPEN_QUOTES_PER_SELLER, SrEconomyConfig};
use crate::economy::fence_record::{
    FenceId, FenceQuote, FenceSettlement, PriceFactor, QuoteId, QuoteObserved, SrFence,
};
use crate::economy::store::{SrEconomyStore, del_quote, put_quote};

/// The one refusal a fence or quote the actor may not use ever gets — identical for
/// nonexistent, expired and unviewable (08a decision 13).
pub const NO_SUCH_QUOTE: &str = "No such quote.";
/// The same, for a fence.
pub const NO_SUCH_FENCE: &str = "No such fence.";
/// The same, for the object a quote is about.
pub const NO_SUCH_ASSET: &str = "No such asset.";

/// Why a fence act was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FenceError {
    /// The layer is off, in the off-mode words.
    Disabled(&'static str),
    /// Nothing the actor may act on — the existence-hiding answer.
    NoStanding(&'static str),
    /// A shaped refusal that says something true about the command.
    Bad(String),
    /// The economy refused the offer or the accept.
    Contract(ContractError),
    /// The economy refused the custody post.
    Custody(mush_economy::EconomyError),
}

/// Render a [`FenceError`] under the existence-hiding discipline.
#[must_use]
pub fn refuse_fence(
    ctx: &mut Ctx<'_>,
    e: &FenceError,
    named: &[NamedResource],
) -> mush_core::CmdError {
    // E6b verify fold (D8): C16's per-actor tally exists so staff can see the shape
    // of an id-space walk, and `FenceId`/`QuoteId` are two brand-new enumerable id
    // spaces. A refusal built straight into a `CmdError` never reached it.
    match e {
        FenceError::Disabled(why) => mush_core::CmdError::msg((*why).to_string()),
        FenceError::NoStanding(why) => {
            mush_economy::note_refusal(ctx, true);
            mush_core::CmdError::msg((*why).to_string())
        }
        FenceError::Bad(why) => {
            mush_economy::note_refusal(ctx, false);
            mush_core::CmdError::msg(why.clone())
        }
        FenceError::Contract(e) => mush_economy::refuse_contract_offer(ctx, e, named.to_vec()),
        FenceError::Custody(e) => mush_economy::refuse(
            ctx,
            named
                .first()
                .copied()
                .unwrap_or(NamedResource::Asset(ObjId(0))),
            &[],
            e,
        ),
    }
}

// ---- pricing --------------------------------------------------------------

/// The deterministic price of one object at one fence (D-E6-11).
///
/// `None` when the object cannot be quoted at all: no title record, no
/// `CatalogRef`, or a code the source-controlled baseline does not name (OPEN 1).
/// **Refused, never guessed** — an object with no reference price has no price a
/// fence could defend, and inventing one from the `AcquisitionRecord` would publish
/// what somebody paid for it.
#[must_use]
pub fn price(
    cfg: &SrEconomyConfig,
    fence: &SrFence,
    catalog_code: &str,
    settlement: FenceSettlement,
) -> Option<(Amount, Vec<PriceFactor>)> {
    let baseline = cfg.baseline_minor(catalog_code)?;
    let rate = i64::from(match settlement {
        FenceSettlement::Immediate => cfg.fence_rate_immediate,
        FenceSettlement::Consign { .. } => cfg.fence_rate_consign,
    });
    // The fence's own declared demand adjustment, in permille, staff-set at
    // registration and part of its public listing. Everything here is declared and
    // observable; nothing gated is read.
    let effective = rate.saturating_add(fence.demand).clamp(0, 1_000);
    let offer = baseline
        .checked_mul(effective)
        .map(|n| n / 1_000)
        .unwrap_or(0);
    if offer <= 0 {
        return None;
    }
    let basis = vec![
        PriceFactor {
            label: SmolStr::new_static("baseline"),
            permille: 1_000,
        },
        PriceFactor {
            label: match settlement {
                FenceSettlement::Immediate => SmolStr::new_static("immediate"),
                FenceSettlement::Consign { .. } => SmolStr::new_static("consign"),
            },
            permille: rate,
        },
        PriceFactor {
            label: SmolStr::new_static("demand"),
            permille: fence.demand,
        },
    ];
    Some((Amount::from_minor(offer), basis))
}

/// The **catalog code** an object may be quoted against, and nothing else about it.
///
/// Deliberately narrow: it reads the asset's `catalog` field and returns its `code`.
/// It does not read the encumbrances, the theft report, the acquisition or the title
/// owner, which is what makes two worlds differing only in hidden state produce
/// identical quotes (D-E6-11).
#[must_use]
pub fn quotable_code(ctx: &Ctx<'_>, object: ObjId) -> Option<SmolStr> {
    ctx.w
        .module_state::<EconomyStore>()?
        .snapshot()
        .asset(object)?
        .catalog
        .as_ref()
        .map(|c| c.code.clone())
}

/// The **declared** state a quote is bound to (E6b verify fold, D1).
///
/// It was `EconomicAsset.revision`, and that was an oracle: `AssetOp::ReportStolen`
/// bumps the revision like every other asset op, so filing a **covert** theft report
/// flipped the holder's outstanding quote from `open` to `lapsed` and
/// `+fence/accept` said so out loud. The price was identical in both worlds and the
/// *rendered state* was not, which is the same leak D-E6-11 forbids one step later.
///
/// So the stamp is the declared half and nothing else: a stable hash of the asset's
/// own `CatalogRef.code`, which is exactly what the quote was priced from. A hidden
/// fact moves it not at all; re-cataloguing the object — a declared change to what
/// the thing *is* — moves it, and the quote then refuses deterministically rather
/// than being re-priced (D-E6-13, scenario 29).
#[must_use]
fn declared_stamp(ctx: &Ctx<'_>, object: ObjId) -> Option<u64> {
    quotable_code(ctx, object).map(|code| {
        let mut h: u64 = 1_469_598_103_934_665_603;
        for b in code.as_bytes() {
            h ^= u64::from(*b);
            h = h.wrapping_mul(1_099_511_628_211);
        }
        h
    })
}

/// A principal's own unscoped, `Active` account in the configured instrument — the
/// same derivation `+run/post` uses, so a fence's wallet is never named by anybody.
#[must_use]
fn principal_account(ctx: &Ctx<'_>, cfg: &SrEconomyConfig, who: &PrincipalId) -> Option<AccountId> {
    let instrument = mush_economy::InstrumentId::new(cfg.instrument.as_str())?;
    // An **organization's** account is always scoped — `@account/open org:X` opens it
    // on the org's root compartment — so the "unscoped" rule that names a person's
    // own wallet would name none of a fence's. The rule is the same either way: the
    // principal's own live account in the configured instrument, lowest id first, so
    // nobody ever names a wallet.
    let personal = matches!(who, PrincipalId::Object(_));
    ctx.w
        .module_state::<EconomyStore>()?
        .snapshot()
        .accounts_of(who)
        .find(|a| {
            a.instrument == instrument
                && (!personal || a.scope.is_none())
                && a.status == mush_economy::AccountStatus::Active
        })
        .map(|a| a.id)
}

// ---- the verbs ------------------------------------------------------------

/// `@sr/fence <org>=<vault>` (staff) — register a vendor organization.
///
/// The fence's account is derived from the org's own unscoped `sr.nuyen` account, so
/// nobody names a wallet: the escrow posts under **that principal's** debit authority
/// (DL-11 grants no money authority whatever), and a fence that runs out of money
/// simply cannot accept, which is how "a fence that runs out stops quoting" is
/// enforced without a line of code for it (M19, D-E6-15).
///
/// # Errors
///
/// [`FenceError`]; render it with [`refuse_fence`].
pub fn register_fence(
    ctx: &mut Ctx<'_>,
    cfg: &SrEconomyConfig,
    org: OrgId,
    vault: VaultId,
    name: SmolStr,
    demand: i64,
) -> Result<FenceId, FenceError> {
    let principal = PrincipalId::Organization(org);
    let account =
        principal_account(ctx, cfg, &principal).ok_or(FenceError::NoStanding(NO_SUCH_FENCE))?;
    // The vault must exist, be active, and belong to the same organization: a fence
    // that took custody into somebody else's vault would be handing the goods to a
    // third party under the seller's nose.
    let ok = ctx
        .w
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().vault(vault).cloned())
        .is_some_and(|v| v.owner == principal && v.status == mush_economy::VaultStatus::Active);
    if !ok {
        return Err(FenceError::NoStanding(NO_SUCH_FENCE));
    }
    if ctx
        .w
        .module_state::<SrEconomyStore>()
        .is_some_and(|s| s.fences().count() >= crate::economy::config::MAX_FENCES)
    {
        return Err(FenceError::Bad(
            "This server already registers as many fences as it allows.".to_string(),
        ));
    }
    // One fence per organization: two would make "which one did I sell to" a
    // question about ids rather than about the world.
    if let Some(existing) = ctx
        .w
        .module_state::<SrEconomyStore>()
        .and_then(|s| s.fence_of_org(org).map(|f| f.id))
    {
        let mut f = ctx
            .w
            .module_state::<SrEconomyStore>()
            .and_then(|s| s.fence(existing).cloned())
            .ok_or(FenceError::NoStanding(NO_SUCH_FENCE))?;
        f.vault = vault;
        f.account = account;
        f.name = name;
        f.demand = demand;
        f.revision = f.revision.saturating_add(1);
        put_quote_free(ctx, &f);
        return Ok(existing);
    }
    let Some(id) = ctx
        .w
        .module_state_mut::<SrEconomyStore>()
        .and_then(SrEconomyStore::alloc_fence)
    else {
        return Err(FenceError::Bad(
            "The fence-id allocator is exhausted.".to_string(),
        ));
    };
    let fence = SrFence {
        id,
        org,
        vault,
        account,
        name,
        demand,
        revision: 1,
    };
    put_quote_free(ctx, &fence);
    Ok(id)
}

/// Write a fence record and its allocator document.
fn put_quote_free(ctx: &mut Ctx<'_>, f: &SrFence) {
    crate::economy::store::put_fence(ctx, f.clone());
    crate::economy::store::persist_meta(ctx);
}

/// `+fence/quote <fence>=<obj> [immediate|consign]` — a deterministic offer.
///
/// # Errors
///
/// [`FenceError`]; render it with [`refuse_fence`].
pub fn quote(
    ctx: &mut Ctx<'_>,
    cfg: &SrEconomyConfig,
    fence: FenceId,
    object: ObjId,
    consign: bool,
) -> Result<QuoteId, FenceError> {
    let seller = ctx.actor;
    let Some(f) = ctx
        .w
        .module_state::<SrEconomyStore>()
        .and_then(|s| s.fence(fence).cloned())
    else {
        return Err(FenceError::NoStanding(NO_SUCH_FENCE));
    };
    // G3 / scenario 12: a hidden fence's listings are **absent** for a non-member,
    // and so is its quote counter. The existence-hiding answer is the same one an
    // unknown fence gets.
    if !mush_economy::may_list_org(ctx, f.org) {
        return Err(FenceError::NoStanding(NO_SUCH_FENCE));
    }
    // The seller must actually be holding it. This is a world fact, not a gated one:
    // a fence buys custody from whoever has custody, and asking `may_view_asset`
    // instead would have made a stolen object unsellable *and* answered a probe.
    if ctx.w.get(object).and_then(|o| o.location) != Some(seller) {
        return Err(FenceError::NoStanding(NO_SUCH_ASSET));
    }
    let Some(code) = quotable_code(ctx, object) else {
        return Err(FenceError::Bad(
            "That has no catalogue entry, so no fence can put a price on it.".to_string(),
        ));
    };
    let settlement = if consign {
        FenceSettlement::Consign {
            delay: cfg.fence_delay(),
        }
    } else {
        FenceSettlement::Immediate
    };
    let Some((offer, basis)) = price(cfg, &f, &code, settlement) else {
        return Err(FenceError::Bad(
            "That has no catalogue entry, so no fence can put a price on it.".to_string(),
        ));
    };
    let Some(stamp) = declared_stamp(ctx, object) else {
        return Err(FenceError::NoStanding(NO_SUCH_ASSET));
    };
    let open = ctx
        .w
        .module_state::<SrEconomyStore>()
        .map_or(0, |s| s.quotes().filter(|q| q.seller == seller).count());
    if open >= MAX_OPEN_QUOTES_PER_SELLER {
        return Err(FenceError::Bad(
            "You have as many open quotes as the board allows; accept or let one lapse."
                .to_string(),
        ));
    }
    let Some(id) = ctx
        .w
        .module_state_mut::<SrEconomyStore>()
        .and_then(SrEconomyStore::alloc_quote)
    else {
        return Err(FenceError::Bad(
            "The quote-id allocator is exhausted.".to_string(),
        ));
    };
    let now = ctx.w.now();
    let q = FenceQuote {
        id,
        fence,
        seller,
        object,
        catalog: code,
        offer,
        basis,
        settlement,
        expires: EconomicDeadline {
            clock: mush_economy::EconomicClock::RealUtc,
            at: now.saturating_add(FENCE_QUOTE_TTL_SECS),
        },
        observed: QuoteObserved {
            declared: stamp,
            fence: f.revision,
        },
        contract: None,
        revision: 1,
    };
    put_quote(ctx, q);
    crate::economy::store::persist_meta(ctx);
    Ok(id)
}

/// Whether a quote is still good **right now**: not lapsed, and priced against the
/// revisions that still stand (D-E6-13, scenario 29).
///
/// A stale quote refuses **deterministically** rather than re-pricing, which is the
/// whole point: a seller who watched the price and waited gets the same answer as one
/// who never looked.
#[must_use]
pub fn is_current(ctx: &Ctx<'_>, q: &FenceQuote) -> bool {
    let now = ctx.w.now();
    if now >= q.expires.at {
        return false;
    }
    let fence_rev = ctx
        .w
        .module_state::<SrEconomyStore>()
        .and_then(|s| s.fence(q.fence).map(|f| f.revision));
    fence_rev == Some(q.observed.fence)
        && declared_stamp(ctx, q.object) == Some(q.observed.declared)
}

/// `+fence/accept <quote>` — draft, offer, and let the **fence** accept first.
///
/// Returns the contract. The seller binds afterwards with `@contract/accept`, and
/// only then may they deliver (D-E6-9): the custody lock exists only for a live
/// contract, so escrow strictly precedes custody (answer 15).
///
/// # Errors
///
/// [`FenceError`]; render it with [`refuse_fence`].
pub fn accept(
    ctx: &mut Ctx<'_>,
    cfg: &SrEconomyConfig,
    id: QuoteId,
) -> Result<ContractId, FenceError> {
    let Some(mut q) = ctx
        .w
        .module_state::<SrEconomyStore>()
        .and_then(|s| s.quote(id).cloned())
    else {
        return Err(FenceError::NoStanding(NO_SUCH_QUOTE));
    };
    if q.seller != ctx.actor {
        return Err(FenceError::NoStanding(NO_SUCH_QUOTE));
    }
    if q.contract.is_some() {
        return Err(FenceError::Bad(
            "That quote has already been taken up; deliver it or let it lapse.".to_string(),
        ));
    }
    if !is_current(ctx, &q) {
        return Err(FenceError::Bad(
            "That quote has lapsed or the goods have changed since it was priced. Ask for a new \
             one."
                .to_string(),
        ));
    }
    let Some(f) = ctx
        .w
        .module_state::<SrEconomyStore>()
        .and_then(|s| s.fence(q.fence).cloned())
    else {
        return Err(FenceError::NoStanding(NO_SUCH_QUOTE));
    };
    let Some(seller_account) = principal_account(ctx, cfg, &PrincipalId::Object(q.seller)) else {
        return Err(FenceError::Bad(
            "You need a nuyen account before a fence can pay you.".to_string(),
        ));
    };
    let spec = fence_spec(cfg, &q, &f, seller_account, ctx.w.now());
    let contract = mush_economy::offer_contract(ctx, spec).map_err(FenceError::Contract)?;
    // **The fence accepts first** — programmatically (DL-11, M15), because a
    // staff-built NPC fence has no officer at all. The escrow posts under the org
    // account's own debit authority, so a fence that has run out is refused here and
    // stops being able to take anything.
    let fence_principal = PrincipalId::Organization(f.org);
    match mush_economy::accept_contract_as(ctx, contract, &fence_principal) {
        Ok(mush_economy::AcceptOutcome::Accepted { .. }) => {}
        Ok(mush_economy::AcceptOutcome::NeedsApproval(_)) | Err(_) => {
            let _ = mush_economy::cancel_contract(ctx, contract);
            return Err(FenceError::Bad(
                "That fence cannot pay for it right now. Nothing was agreed.".to_string(),
            ));
        }
    }
    q.contract = Some(contract);
    q.revision = q.revision.saturating_add(1);
    put_quote(ctx, q);
    Ok(contract)
}

/// The contract a fence deal is: **custody for money, and no title anywhere**.
fn fence_spec(
    cfg: &SrEconomyConfig,
    q: &FenceQuote,
    f: &SrFence,
    seller_account: AccountId,
    now: i64,
) -> ContractSpec {
    let deposit = TypedCondition::DepositInto {
        object: q.object,
        vault: f.vault,
    };
    let (conditions, evidence, when) = match q.settlement {
        FenceSettlement::Immediate => {
            (vec![deposit], vec![EvidenceKind::DepositInto], vec![0usize])
        }
        FenceSettlement::Consign { delay } => (
            vec![
                deposit,
                TypedCondition::After {
                    condition: 0,
                    delay,
                    clock: mush_economy::EconomicClock::RealUtc,
                },
            ],
            vec![EvidenceKind::DepositInto, EvidenceKind::After],
            vec![0usize, 1usize],
        ),
    };
    ContractSpec {
        name: SmolStr::new(format!("fence sale #{}", q.id.0)),
        scope: None,
        parties: vec![PrincipalId::Organization(f.org)],
        considerations: vec![ConsiderationSpec {
            account: f.account,
            amount: q.offer,
        }],
        outcomes: vec![OutcomeTerms {
            name: SmolStr::new_static("sold"),
            when,
            payouts: vec![PayoutShare {
                to: seller_account,
                share: Share::Exact(q.offer),
            }],
            expense_treatment: mush_economy::ExpenseTreatment::None,
            asset_consequences: Vec::new(),
        }],
        remainder_to: f.account,
        conditions,
        // **No adjudicator.** Nobody's word settles a fence sale: the journal's own
        // custody effect does, which is exactly what makes the deal deterministic.
        adjudicator: None,
        evidence,
        deadline: Some(EconomicDeadline {
            clock: mush_economy::EconomicClock::RealUtc,
            at: now.saturating_add(deal_window(cfg, q.settlement)),
        }),
        // An undelivered deal simply refunds: nothing was taken, so there is nothing
        // to forfeit, and a fence keeping the money for goods it never received is
        // the confiscation D-E6-9 exists to prevent.
        on_expiry: Expiry::Refund,
        visibility: EconomicVisibility::Parties,
        roles: ContractRoles {
            author: Some(PrincipalId::Object(q.seller)),
            payer: Some(PrincipalId::Organization(f.org)),
            representative: None,
            facilitator: None,
        },
        // The deliverable IS the deposit; `TitleTransfer` is never in a fence
        // template (D-E6-10).
        deterministic: true,
    }
}

/// How long the deal itself stands: the quote's own TTL for an immediate sale, and
/// the consignment delay plus that window for a consignment, so the `After` can
/// actually mature before the deadline lapses.
#[must_use]
pub fn deal_window(cfg: &SrEconomyConfig, settlement: FenceSettlement) -> i64 {
    match settlement {
        FenceSettlement::Immediate => FENCE_QUOTE_TTL_SECS.saturating_mul(24),
        FenceSettlement::Consign { delay } => delay
            .saturating_add(FENCE_QUOTE_TTL_SECS.saturating_mul(24))
            .max(cfg.fence_delay()),
    }
}

/// `+fence/deliver <quote>` — hand the goods over.
///
/// **Refused while the contract is `Offered`** (D-E6-9): E4b2 custody-locks a
/// `DepositInto` object only for an `Active`/`ReadyToSettle` contract, so a seller
/// who delivered before the deal was in force would hand over an unlocked, unpaid
/// object. Note that `Disputed` does not lock either — a disputed consignment's
/// deposit is *not* protected by the lock, which is why delivery is refused there
/// too and the goods stay with the seller until the dispute resolves.
///
/// # Errors
///
/// [`FenceError`]; render it with [`refuse_fence`].
pub fn deliver(ctx: &mut Ctx<'_>, id: QuoteId) -> Result<String, FenceError> {
    let Some(q) = ctx
        .w
        .module_state::<SrEconomyStore>()
        .and_then(|s| s.quote(id).cloned())
    else {
        return Err(FenceError::NoStanding(NO_SUCH_QUOTE));
    };
    if q.seller != ctx.actor {
        return Err(FenceError::NoStanding(NO_SUCH_QUOTE));
    }
    let Some(contract) = q.contract else {
        return Err(FenceError::Bad(
            "Nobody has agreed to buy that yet (+fence/accept first).".to_string(),
        ));
    };
    let live = ctx
        .w
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().contract(contract).map(|c| c.state))
        .ok_or(FenceError::NoStanding(NO_SUCH_QUOTE))?;
    if !matches!(
        live,
        mush_economy::ContractState::Active | mush_economy::ContractState::ReadyToSettle
    ) {
        return Err(FenceError::Bad(format!(
            "That deal is not in force yet — accept it first (@contract/accept #{}). The fence's \
             payment is already escrowed; the goods stay with you until it is.",
            contract.0
        )));
    }
    let Some(f) = ctx
        .w
        .module_state::<SrEconomyStore>()
        .and_then(|s| s.fence(q.fence).cloned())
    else {
        return Err(FenceError::NoStanding(NO_SUCH_QUOTE));
    };
    // The seller's own custody standing, through the economy's own seam — never a
    // dispatched `@vault/deposit`.
    mush_economy::deposit_into_vault(ctx, q.object, f.vault).map_err(FenceError::Custody)?;
    Ok(format!(
        "Delivered into the fence's vault. Contract #{} settles from here.",
        contract.0
    ))
}

/// Forget a quote whose contract reached a terminal state, or that simply lapsed —
/// the board's `del_run` shape (E6a verify fold, D1): a closed record is deleted, not
/// kept forever, so a quote/lapse loop mints no permanent documents.
pub fn close_quote(ctx: &mut Ctx<'_>, id: QuoteId) {
    del_quote(ctx, id);
}
