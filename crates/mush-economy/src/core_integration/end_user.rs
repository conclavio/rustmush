//! **The Class III end-user register** (doc 44 unit MW.23, Q-44-40; doc 17 §16; 17f M7.11).
//!
//! A Class III core is integrated for, and delivered to, only a principal who holds the
//! end-user entitlement: an `Active` unscoped account of their own, with a balance, in the
//! instrument [`CORE3_ENTITLEMENT`]. Integration's acceptance and completion and the delivery
//! ask [`entitlement`], the one rule. Until this unit nothing but a test could make such an
//! account, so no launch world could hold a Class III core.
//!
//! [`register_end_user`] is the route: the holder's own paid registration with the Registry. In
//! **one** posted transaction the fee moves from the holder's account to the Registry's and one
//! unit of the entitlement moves from the issuer's source account to the holder's. The fee and
//! the right are one journal entry, so neither exists without the other, and a restart keeps
//! both or neither.
//!
//! The issuer is a **registered title issuer** ([`crate::EconomyModule::with_title_issuers`]),
//! the same wiring-frozen table [`crate::issue_title`] asks: a system principal a command, a
//! document or a grant cannot add. The post runs under an authority that admits exactly these
//! four lines and nothing else (the title issuer's pattern), because the live org bridge
//! refuses any debit of a system account that staff did not type.
//!
//! The first registration on a world registers the instrument and opens the issuer's source
//! account; each holder's first opens their own account. None of those is money. A caller
//! decides who may ask and what the fee is; this module decides what a registration is.
use super::{CoreIntegrationError as Error, subject::CORE3_ENTITLEMENT};
use crate::api::{self, PostAuth, PostOutcome};
use crate::engine::AuthEval;
use crate::{
    Account, AccountId, AccountStatus, Amount, CreditPolicy, EconomicCause, EconomyError,
    EconomyPlan, EconomySnapshot, EconomyStore, IdempotencyKey, InstrumentDef, InstrumentId,
    InstrumentKind, InstrumentStatus, NegativePolicy, RequestedLine, TransactionId,
};
use mush_core::{Ctx, Flags, ObjId, ObjKind, World};
use mush_org::{PrincipalId, SystemPrincipalId};
use smol_str::SmolStr;
use std::any::TypeId;

/// The journal cause and the idempotency namespace of a registration.
const REGISTRATION: &str = "core3.end_user";
/// The name of a holder's entitlement account and of the issuer's source account.
const ACCOUNT_NAME: &str = "Class III end-user registration";

/// The entitlement's instrument id.
fn instrument_id() -> Result<InstrumentId, Error> {
    InstrumentId::new(CORE3_ENTITLEMENT).ok_or(Error::Invalid)
}

/// Whether `def` has the shape the entitlement is checked by: an active, indivisible,
/// untransferable entitlement.
fn shaped(def: &InstrumentDef) -> bool {
    def.kind == InstrumentKind::Entitlement
        && def.scale == 0
        && !def.transferable
        && def.status == InstrumentStatus::Active
}

/// **The one rule** (17f M7.11): the registered entitlement instrument, when `requester` holds
/// it now. Integration's acceptance and completion and the delivery all ask this.
///
/// # Errors
///
/// [`Error::Unavailable`] when the instrument is not registered or not in its shape, or
/// `requester` has no `Active` unscoped account in it with a balance.
pub(super) fn entitlement(
    snapshot: &EconomySnapshot,
    requester: ObjId,
) -> Result<&InstrumentDef, Error> {
    let id = instrument_id()?;
    let def = snapshot.instrument(&id).ok_or(Error::Unavailable)?;
    if !shaped(def) {
        return Err(Error::Unavailable);
    }
    let principal = PrincipalId::Object(requester);
    if !snapshot.accounts_of(&principal).any(|a| {
        a.instrument == id
            && a.scope.is_none()
            && a.status == AccountStatus::Active
            && a.posted_balance.minor() > 0
    }) {
        return Err(Error::Unavailable);
    }
    Ok(def)
}

/// Where a character stands on the end-user register.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EndUserStanding {
    /// No registration: a Class III core is refused them.
    Unlisted,
    /// Registered: the account that says so.
    Registered {
        /// Their entitlement account.
        account: AccountId,
    },
    /// An account of theirs in the entitlement is not `Active` (the Registry or staff froze
    /// it): a Class III core is refused them, and so is a new registration.
    Suspended {
        /// The account that is not `Active`.
        account: AccountId,
    },
}

/// **A character's standing on the register**, read from the economy alone. `Registered`
/// exactly when the integration's rule admits them.
#[must_use]
pub fn end_user_standing(world: &World, holder: ObjId) -> EndUserStanding {
    let Some(store) = world.module_state::<EconomyStore>() else {
        return EndUserStanding::Unlisted;
    };
    standing(store.snapshot(), holder)
}

fn standing(snapshot: &EconomySnapshot, holder: ObjId) -> EndUserStanding {
    let Ok(id) = instrument_id() else {
        return EndUserStanding::Unlisted;
    };
    let principal = PrincipalId::Object(holder);
    let own = |a: &&Account| a.instrument == id && a.scope.is_none();
    if entitlement(snapshot, holder).is_ok()
        && let Some(account) = snapshot
            .accounts_of(&principal)
            .filter(own)
            .filter(|a| a.status == AccountStatus::Active && a.posted_balance.minor() > 0)
            .map(|a| a.id)
            .min()
    {
        return EndUserStanding::Registered { account };
    }
    match snapshot
        .accounts_of(&principal)
        .filter(own)
        .filter(|a| a.status != AccountStatus::Active)
        .map(|a| a.id)
        .min()
    {
        Some(account) => EndUserStanding::Suspended { account },
        None => EndUserStanding::Unlisted,
    }
}

/// One registration, as its caller asks for it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EndUserRegistration {
    /// The registered title issuer the entitlement is issued by.
    pub issuer: SystemPrincipalId,
    /// The character registering: a live player. The registration is theirs alone.
    pub holder: ObjId,
    /// The holder's own unscoped account the fee is paid from.
    pub payer: AccountId,
    /// The Registry's account the fee is paid into, in the payer's instrument.
    pub registry: AccountId,
    /// The fee, above zero.
    pub fee: Amount,
}

/// A registration that landed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EndUserRegistered {
    /// The holder's entitlement account.
    pub account: AccountId,
    /// The one transaction that paid the fee and issued the entitlement.
    pub transaction: TransactionId,
}

/// Why a registration was refused. Nothing was paid and nothing was issued.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum EndUserError {
    /// The economy, the issuer, the holder or one of the named accounts cannot take a
    /// registration now.
    #[error("the end-user register is unavailable")]
    Unavailable,
    /// The holder is registered already.
    #[error("already registered as an end user")]
    Registered {
        /// Their entitlement account.
        account: AccountId,
    },
    /// The holder's entitlement account is not `Active`.
    #[error("the end-user registration is suspended")]
    Suspended {
        /// The account that is not `Active`.
        account: AccountId,
    },
    /// The payer cannot cover the fee.
    #[error("the fee is not covered")]
    Funds {
        /// What the payer has available.
        available: Amount,
    },
    /// The post refused. Any account this call opened stays, empty, and entitles nobody.
    #[error("the registration did not post: {0}")]
    Refused(EconomyError),
}

/// What a registration will write, decided before the first write.
struct Plan {
    instrument: InstrumentId,
    /// Whether the instrument is registered already.
    registered: bool,
    /// The holder's empty `Active` entitlement account, when one exists.
    entitlement: Option<AccountId>,
    /// The issuer's source account, when it exists.
    source: Option<AccountId>,
    /// The holder's entitlement account's last journal sequence: the idempotency generation.
    generation: u64,
}

/// Every check of a registration, read-only.
fn plan(world: &World, request: &EndUserRegistration) -> Result<Plan, EndUserError> {
    if world
        .module_state_owner(TypeId::of::<EconomyStore>())
        .is_none_or(|owner| owner.as_str() != "economy")
    {
        return Err(EndUserError::Unavailable);
    }
    let store = world
        .module_state::<EconomyStore>()
        .filter(|store| !store.is_frozen())
        .ok_or(EndUserError::Unavailable)?;
    if !store.is_title_issuer(&request.issuer)
        || world
            .get(request.holder)
            .is_none_or(|o| o.kind != ObjKind::Player || o.flags.contains(Flags::GOING))
        || request.fee.minor() <= 0
        || request.payer == request.registry
    {
        return Err(EndUserError::Unavailable);
    }
    let snapshot = store.snapshot();
    let instrument = instrument_id().map_err(|_| EndUserError::Unavailable)?;
    let registered = match snapshot.instrument(&instrument) {
        Some(def) if shaped(def) => true,
        Some(_) => return Err(EndUserError::Unavailable),
        None => false,
    };
    let holder = PrincipalId::Object(request.holder);
    let entitlement = match standing(snapshot, request.holder) {
        EndUserStanding::Registered { account } => {
            return Err(EndUserError::Registered { account });
        }
        EndUserStanding::Suspended { account } => {
            return Err(EndUserError::Suspended { account });
        }
        // Every account of theirs in the instrument is `Active` and empty: the lowest is
        // reused (one a refused registration left, or one staff opened).
        EndUserStanding::Unlisted => snapshot
            .accounts_of(&holder)
            .filter(|a| a.instrument == instrument && a.scope.is_none())
            .min_by_key(|a| a.id),
    };
    let issuer = PrincipalId::System(request.issuer.clone());
    let source = snapshot
        .accounts_of(&issuer)
        .filter(|a| a.instrument == instrument && a.scope.is_none())
        .min_by_key(|a| a.id);
    if source.is_some_and(|a| a.status != AccountStatus::Active) {
        return Err(EndUserError::Unavailable);
    }
    // The fee: the holder's own live unscoped account, with no approval rule to wait on, in
    // an active currency; the Registry's live account in the same currency.
    let payer = snapshot
        .account(request.payer)
        .ok_or(EndUserError::Unavailable)?;
    let registry = snapshot
        .account(request.registry)
        .ok_or(EndUserError::Unavailable)?;
    let currency = snapshot
        .instrument(&payer.instrument)
        .ok_or(EndUserError::Unavailable)?;
    if payer.owner != holder
        || payer.scope.is_some()
        || payer.status != AccountStatus::Active
        || payer.approval.is_some()
        || currency.kind != InstrumentKind::Currency
        || currency.status != InstrumentStatus::Active
        || registry.instrument != payer.instrument
        || registry.status != AccountStatus::Active
        || !api::credit_allowed(world, registry)
    {
        return Err(EndUserError::Unavailable);
    }
    let available = payer.available().unwrap_or(Amount::ZERO);
    if available.minor() < request.fee.minor() {
        return Err(EndUserError::Funds { available });
    }
    Ok(Plan {
        instrument,
        registered,
        entitlement: entitlement.map(|a| a.id),
        source: source.map(|a| a.id),
        generation: entitlement.map_or(0, |a| a.last_journal_seq),
    })
}

/// **Whether a registration would be taken now**: every check [`register_end_user`] makes,
/// with nothing written. A preview reads this.
///
/// # Errors
///
/// The refusal the registration would meet.
pub fn check_end_user_registration(
    world: &World,
    request: &EndUserRegistration,
) -> Result<(), EndUserError> {
    plan(world, request).map(|_| ())
}

/// The authority of one registration's post: exactly its four lines, by the holder.
struct EndUserAuth {
    holder: PrincipalId,
    issuer: PrincipalId,
    fee: Amount,
    payer: AccountId,
    registry: AccountId,
    source: AccountId,
    entitlement: AccountId,
}

impl AuthEval for EndUserAuth {
    fn authorize(&self, actor: &PrincipalId, account: &Account, delta: Amount) -> bool {
        if *actor != self.holder {
            return false;
        }
        let own = account.owner == self.holder && account.scope.is_none();
        if account.id == self.payer {
            own && delta.checked_neg() == Some(self.fee)
        } else if account.id == self.registry {
            delta == self.fee
        } else if account.id == self.source {
            account.owner == self.issuer && delta.minor() == -1
        } else if account.id == self.entitlement {
            own && delta.minor() == 1
        } else {
            false
        }
    }
}

/// A fresh zero-balance account.
fn account(
    id: AccountId,
    owner: PrincipalId,
    instrument: InstrumentId,
    credit: CreditPolicy,
) -> Account {
    Account {
        id,
        owner,
        instrument,
        scope: None,
        name: SmolStr::new_static(ACCOUNT_NAME),
        status: AccountStatus::Active,
        posted_balance: Amount::ZERO,
        held_amount: Amount::ZERO,
        last_hold_seq: 0,
        credit,
        last_journal_seq: 0,
        revision: 1,
        approval: None,
    }
}

/// **Register `request.holder` as a Class III end user**: pay the fee to the Registry and
/// issue one entitlement, in one transaction.
///
/// A holder is registered once. A second call for a registered holder is refused
/// ([`EndUserError::Registered`]) and writes nothing; two calls over the same state resolve
/// to one transaction (the key carries the holder and their entitlement account's journal
/// sequence), so a retried registration can never issue twice.
///
/// Like [`crate::post`], **this is the last fallible call a handler may make**. When the post
/// refuses after this call registered the instrument or opened an account, those writes are
/// pinned so RAM and the store agree; what is left is an empty account that entitles nobody
/// and that the next registration reuses.
///
/// # Errors
///
/// [`EndUserError`]; nothing was paid and nothing was issued.
pub fn register_end_user(
    ctx: &mut Ctx<'_>,
    request: &EndUserRegistration,
) -> Result<EndUserRegistered, EndUserError> {
    let plan = plan(ctx.w, request)?;
    let charge = request.fee.checked_neg().ok_or(EndUserError::Unavailable)?;
    let holder = PrincipalId::Object(request.holder);
    let issuer = PrincipalId::System(request.issuer.clone());
    // Both ids before the first write: a spent allocator refuses with nothing written.
    let mut fresh = || {
        ctx.w
            .module_state_mut::<EconomyStore>()
            .and_then(EconomyStore::alloc_account)
            .ok_or(EndUserError::Unavailable)
    };
    let (source, open_source) = match plan.source {
        Some(id) => (id, false),
        None => (fresh()?, true),
    };
    let (entitlement, open_entitlement) = match plan.entitlement {
        Some(id) => (id, false),
        None => (fresh()?, true),
    };
    let wrote = !plan.registered || open_source || open_entitlement;
    if !plan.registered {
        api::register_instrument(
            ctx,
            InstrumentDef {
                id: plan.instrument.clone(),
                ruleset: SmolStr::new_static("tacsim"),
                code: SmolStr::new_static("licence.core3_end_user"),
                kind: InstrumentKind::Entitlement,
                scale: 0,
                transferable: false,
                // The issuer's source runs negative by one a registration; no holder's may.
                negative_policy: NegativePolicy::SystemOnly,
                status: InstrumentStatus::Active,
                display: None,
                revision: 1,
            },
        )
        .map_err(|_| EndUserError::Unavailable)?;
    }
    if open_source {
        api::open_account(
            ctx,
            account(
                source,
                issuer.clone(),
                plan.instrument.clone(),
                CreditPolicy::unbounded(),
            ),
        );
    }
    if open_entitlement {
        api::open_account(
            ctx,
            account(
                entitlement,
                holder.clone(),
                plan.instrument.clone(),
                CreditPolicy::forbid(),
            ),
        );
    }
    let line = |account, delta| RequestedLine {
        account,
        delta,
        memo: SmolStr::new_static(REGISTRATION),
    };
    let refused = |ctx: &mut Ctx<'_>, error| {
        if wrote {
            ctx.out.pin_deferred();
        }
        EndUserError::Refused(error)
    };
    let mut post = EconomyPlan::new(
        IdempotencyKey::new(
            SmolStr::new_static(REGISTRATION),
            SmolStr::new(format!("{}/{}", request.holder.0, plan.generation)),
        ),
        holder.clone(),
        EconomicCause::Internal {
            kind: SmolStr::new_static(REGISTRATION),
            id: u64::from(request.holder.0),
        },
        vec![
            line(request.payer, charge),
            line(request.registry, request.fee),
            line(source, Amount::from_minor(-1)),
            line(entitlement, Amount::from_minor(1)),
        ],
    );
    post.description = format!("Class III end-user registration for #{}", request.holder.0);
    let auth = EndUserAuth {
        holder,
        issuer,
        fee: request.fee,
        payer: request.payer,
        registry: request.registry,
        source,
        entitlement,
    };
    match api::post_with(ctx, &post, PostAuth::Injected(&auth)) {
        Ok(PostOutcome::Posted(transaction) | PostOutcome::AlreadyPosted(transaction)) => {
            Ok(EndUserRegistered {
                account: entitlement,
                transaction,
            })
        }
        // The plan checked the payer has no approval rule; one set since refuses.
        Ok(PostOutcome::NeedsApproval(_)) => Err(refused(ctx, EconomyError::Unauthorized)),
        Err(error) => Err(refused(ctx, error)),
    }
}

#[cfg(test)]
mod tests;
