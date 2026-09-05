//! Chargen as one live cash authority, and the idempotent migration (08f §3.5,
//! answers 17 and 18, M12, M13).
//!
//! ## Chargen (answer 17)
//!
//! Until E6c's catalog exists there is nothing to shop for, so E6a ships the **cash
//! authority** and keeps today's behaviour otherwise. The approval handler, after
//! its own validation has passed and the sheet is stamped `Approved`, opens the
//! character's `sr.nuyen` account and mints the priority allotment
//! (`Sr5Config::resource_nuyen`) from `system:sr.chargen` as a **one-shot attributed
//! post** under the deterministic key `sr.chargen/{obj}`, with the **approver** as
//! actor (nobody acts for a `System` principal — E4b1's A-lens rule).
//!
//! Two consequences, both deliberate:
//!
//! - **The `[economy]` mint cap does not bind it** (M12). That cap bounds
//!   `create_system_obligation` — a *standing mint the scheduler executes forever* —
//!   and this is one attributed post somebody signed for. 450,000¥ exceeds the
//!   50,000 major default, so `MintCapExceeded` is unreachable here; the bound is the
//!   priority table itself.
//! - **A refused mint never fails the approval** (08d deviation 11). A sheet that has
//!   been signed off is not un-signed by an economic hiccup: the character is
//!   approved with an empty account, and `@sr/migrate commit` adopts them (M13).
//!
//! The **5,000¥ carry-over cap is parsed and not applied** — answer 17: "until
//! shopping exists, keep current behaviour and do not apply the cap."
//!
//! ## Migration (answer 18)
//!
//! `@sr/migrate dry` lists every approved character with a non-zero sheet `nuyen`
//! and the exact mint it would make. `commit` posts once per character under
//! `sr.migration/{obj}`, from `system:sr.migration` — **not** the chargen faucet —
//! binding the sheet value and the approval stamp, and writes `SrProfile.migrated`.
//! A re-run is an idempotency HIT: it re-reports the first mint and mints nothing. A
//! sheet edited afterwards keeps the bound value and stamp and is reported as a
//! mismatch, never re-minted (`SheetDoc` carries no revision).

use mush_core::{Ctx, ObjId};
use mush_economy::{
    Amount, EconomicCause, EconomicVisibility, EconomyPlan, IdempotencyKey, PostOutcome,
    RequestedLine,
};
use mush_org::PrincipalId;
use mush_ruleset::{ApprovalState, SheetDoc};
use smol_str::SmolStr;

use crate::Sr5Cmd;
use crate::chargen::persisted_priorities;
use crate::data::Sr5Config;
use crate::economy::boot::{open_personal_account, system_account};
use crate::economy::config::{FAUCET_CHARGEN, FAUCET_MIGRATION, SrEconomyConfig};
use crate::economy::record::{Migration, Mint, SrProfile};
use crate::economy::store::{SrEconomyStore, put_profile};

/// The idempotency namespace the chargen allotment is minted under.
pub const CHARGEN_KEY_NS: &str = "sr.chargen";
/// The idempotency namespace the migration is minted under.
pub const MIGRATION_KEY_NS: &str = "sr.migration";

/// The priority allotment `who`'s persisted priorities entitle them to, in major
/// units. `None` when they never assigned a Resources priority.
#[must_use]
pub fn allotment_major(config: &Sr5Config, doc: &SheetDoc) -> Option<u64> {
    let priority = persisted_priorities(doc).get("resources")?;
    Some(u64::from(config.resource_nuyen(priority)))
}

/// The approval artifacts (§3.5). Runs inside the `shadowrun.chargen` payload
/// handler, after the sheet is stamped `Approved`.
///
/// Returns lines to show the approving staffer. It **never fails the approval**.
pub fn on_approved(
    ctx: &mut Ctx<'_>,
    cfg: &SrEconomyConfig,
    config: &Sr5Config,
    doc: &SheetDoc,
    who: ObjId,
) -> Vec<String> {
    if !cfg.enabled {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut profile = ctx
        .w
        .module_state::<SrEconomyStore>()
        .and_then(|s| s.profile(who).cloned())
        .unwrap_or_else(|| SrProfile::new(who));
    let Some(instrument) = ctx
        .w
        .module_state::<SrEconomyStore>()
        .and_then(|s| s.instrument.clone())
    else {
        out.push("SR economy: sr.nuyen is not registered; no account was opened.".to_string());
        return out;
    };
    let Some(account) = open_personal_account(ctx, who, &instrument) else {
        out.push(
            "SR economy: no account could be opened (the allocator is exhausted).".to_string(),
        );
        return out;
    };
    profile.account = Some(account);
    profile.revision = profile.revision.saturating_add(1);
    put_profile(ctx, profile.clone());
    out.push(format!("SR economy: nuyen account #{account} opened."));

    if profile.minted.is_some() {
        out.push("SR economy: the priority allotment was already minted.".to_string());
        return out;
    }
    let Some(major) = allotment_major(config, doc) else {
        out.push("SR economy: no Resources priority is recorded; nothing was minted.".to_string());
        return out;
    };
    match mint(
        ctx,
        &MintSpec {
            who,
            account,
            major,
            faucet: FAUCET_CHARGEN,
            namespace: CHARGEN_KEY_NS,
            correlation: "chargen.allotment",
            instrument: &instrument,
        },
    ) {
        Ok(MintDone {
            amount,
            transaction,
            key,
        }) => {
            profile.minted = Some(Mint {
                key,
                transaction,
                amount,
                at: ctx.w.now(),
            });
            profile.revision = profile.revision.saturating_add(1);
            put_profile(ctx, profile);
            out.push(format!(
                "SR economy: minted {} from system:{FAUCET_CHARGEN}.",
                mush_economy::money_on(ctx, account, amount)
            ));
        }
        // 08d deviation 11: a refused mint never fails the approval.
        Err(why) => out.push(format!(
            "SR economy: the allotment could not be minted ({why}); the character is approved \
             with an empty account. Staff: @sr/migrate commit adopts them."
        )),
    }
    out
}

/// What one one-shot attributed mint needs.
#[derive(Copy, Clone, Debug)]
struct MintSpec<'a> {
    /// Whose mint it is — the deterministic key's subject.
    who: ObjId,
    /// Where it lands.
    account: mush_economy::AccountId,
    /// How much, in major units.
    major: u64,
    /// The faucet it comes from.
    faucet: &'a str,
    /// The idempotency namespace.
    namespace: &'a str,
    /// The cause correlation token.
    correlation: &'a str,
    /// The instrument.
    instrument: &'a mush_economy::InstrumentId,
}

/// What one landed mint left behind.
#[derive(Clone, Debug)]
struct MintDone {
    /// How much was minted, in minor units.
    amount: Amount,
    /// The transaction it landed on, when the post reported one.
    transaction: Option<mush_economy::TransactionId>,
    /// The deterministic key it was posted under.
    key: SmolStr,
}

/// One one-shot attributed post from a faucet into `account`.
///
/// The **approver's** name is on it, not the faucet's: nobody acts for a `System`
/// principal (staff included), so a plan whose actor is the faucet is refused at
/// validation. This is the same attribution `@econ/mint` carries — a seed somebody
/// signed for.
fn mint(ctx: &mut Ctx<'_>, spec: &MintSpec<'_>) -> Result<MintDone, String> {
    let MintSpec {
        who,
        account,
        major,
        faucet,
        namespace,
        correlation,
        instrument,
    } = *spec;
    if major == 0 {
        return Err("the amount is zero".to_string());
    }
    let scale = ctx
        .w
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|s| s.snapshot().instrument(instrument).map(|i| i.scale))
        .ok_or_else(|| "the instrument is not registered".to_string())?;
    let factor = 10_i128
        .checked_pow(u32::from(scale))
        .ok_or_else(|| "the instrument scale does not fit".to_string())?;
    let amount = i128::from(major)
        .checked_mul(factor)
        .and_then(Amount::from_widened)
        .ok_or_else(|| "the amount does not fit".to_string())?;
    let negative = amount
        .checked_neg()
        .ok_or_else(|| "the amount does not negate".to_string())?;
    let debit = system_account(ctx, faucet, instrument)
        .ok_or_else(|| format!("system:{faucet} has no account"))?;
    let key = SmolStr::new(format!("{namespace}/{}", who.0));
    let mut plan = EconomyPlan::new(
        IdempotencyKey::new(SmolStr::new(namespace), SmolStr::new(who.0.to_string())),
        PrincipalId::Object(ctx.actor),
        EconomicCause::Ruleset {
            namespace: SmolStr::new_static("shadowrun"),
            correlation: SmolStr::new(correlation),
        },
        vec![
            RequestedLine {
                account: debit,
                delta: negative,
                memo: SmolStr::new(correlation),
            },
            RequestedLine {
                account,
                delta: amount,
                memo: SmolStr::new(correlation),
            },
        ],
    );
    plan.description = "Shadowrun nuyen mint.".to_string();
    plan.visibility = EconomicVisibility::Parties;
    match mush_economy::post(ctx, &plan) {
        Ok(PostOutcome::Posted(id) | PostOutcome::AlreadyPosted(id)) => Ok(MintDone {
            amount,
            transaction: Some(id),
            key,
        }),
        Ok(PostOutcome::NeedsApproval(id)) => Err(format!("it needs approval (request #{})", id.0)),
        Err(e) => Err(e.to_string()),
    }
}

/// One line of the migration plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigrationLine {
    /// Whose sheet.
    pub who: ObjId,
    /// The sheet's `nuyen`.
    pub sheet_value: i64,
    /// What the commit would do (or did).
    pub verdict: MigrationVerdict,
}

/// What `@sr/migrate` says about one character.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MigrationVerdict {
    /// Would mint this many major units from `system:sr.migration`.
    WouldMint(u64),
    /// Already migrated; the original result stands (an idempotency HIT).
    Hit {
        /// What was minted then.
        amount: Amount,
        /// The sheet value that was bound.
        bound: i64,
    },
    /// Already migrated and the sheet has been edited since — reported, never
    /// re-minted (answer 18).
    Mismatch {
        /// The value the commit bound.
        bound: i64,
    },
    /// Approved with no chargen mint recorded; `commit` **adopts** them — opens the
    /// account, writes the profile, and mints the priority allotment if their
    /// priorities entitle them to one (M13). `0` adopts with an empty account.
    Adopt(u64),
    /// Nothing to do.
    Skip(&'static str),
}

/// Build the migration plan over every approved character with a sheet.
#[must_use]
pub(crate) fn plan(
    ctx: &Ctx<'_>,
    cmd: &Sr5Cmd,
    config: &Sr5Config,
    only: Option<ObjId>,
) -> Vec<MigrationLine> {
    let mut out = Vec::new();
    let candidates: Vec<ObjId> = match only {
        Some(who) => vec![who],
        None => ctx
            .w
            .iter()
            .filter(|o| o.kind == mush_core::ObjKind::Player)
            .map(|o| o.id)
            .collect(),
    };
    for who in candidates {
        let Some(doc) = cmd.read(ctx.w, who) else {
            continue;
        };
        let ApprovalState::Approved { .. } = doc.approved else {
            continue;
        };
        let sheet_value = doc.number("nuyen").unwrap_or(0);
        let profile = ctx
            .w
            .module_state::<SrEconomyStore>()
            .and_then(|s| s.profile(who).cloned());
        let verdict = match profile.as_ref().and_then(|p| p.migrated.as_ref()) {
            Some(m) if m.from_sheet_value == sheet_value => MigrationVerdict::Hit {
                amount: profile
                    .as_ref()
                    .and_then(|p| p.minted.as_ref().map(|x| x.amount))
                    .unwrap_or(Amount::ZERO),
                bound: m.from_sheet_value,
            },
            Some(m) => MigrationVerdict::Mismatch {
                bound: m.from_sheet_value,
            },
            None if profile.as_ref().is_some_and(|p| p.minted.is_some()) => {
                MigrationVerdict::Skip("already funded at chargen approval")
            }
            None if sheet_value > 0 => {
                MigrationVerdict::WouldMint(u64::try_from(sheet_value).unwrap_or(0))
            }
            // M13: approved, no mint, no sheet money. `commit` still **adopts** them —
            // an approved character with no account is exactly the shape a refused
            // chargen mint leaves behind — and mints whatever their priorities
            // entitle them to, which may be nothing at all.
            None => MigrationVerdict::Adopt(allotment_major(config, &doc).unwrap_or(0)),
        };
        out.push(MigrationLine {
            who,
            sheet_value,
            verdict,
        });
    }
    out
}

/// Run `commit` for one planned line. Returns the report line.
pub fn commit_one(
    ctx: &mut Ctx<'_>,
    cfg: &SrEconomyConfig,
    line: &MigrationLine,
    approved_by: ObjId,
    approved_at: i64,
) -> String {
    let who = line.who;
    let major = match line.verdict {
        MigrationVerdict::WouldMint(n) | MigrationVerdict::Adopt(n) => n,
        MigrationVerdict::Hit { amount, bound } => {
            return format!(
                "#{}: already migrated ({} bound at sheet value {bound}); nothing minted.",
                who.0,
                amount.minor()
            );
        }
        MigrationVerdict::Mismatch { bound } => {
            return format!(
                "#{}: MISMATCH — the migration bound sheet value {bound}, the sheet now says {}. \
                 Nothing was re-minted.",
                who.0, line.sheet_value
            );
        }
        MigrationVerdict::Skip(why) => return format!("#{}: skipped ({why}).", who.0),
    };
    let Some(instrument) = ctx
        .w
        .module_state::<SrEconomyStore>()
        .and_then(|s| s.instrument.clone())
    else {
        return format!("#{}: sr.nuyen is not registered.", who.0);
    };
    let Some(account) = open_personal_account(ctx, who, &instrument) else {
        return format!("#{}: no account could be opened.", who.0);
    };
    let mut profile = ctx
        .w
        .module_state::<SrEconomyStore>()
        .and_then(|s| s.profile(who).cloned())
        .unwrap_or_else(|| SrProfile::new(who));
    profile.account = Some(account);
    let carryover = cfg.chargen_carryover_cap;
    let _ = carryover; // answer 17: parsed, deliberately not applied.
    if major == 0 {
        // Adopted with nothing to mint: the account and the profile are the point.
        profile.revision = profile.revision.saturating_add(1);
        put_profile(ctx, profile);
        return format!(
            "#{}: adopted (account #{account} opened); nothing to mint.",
            who.0
        );
    }
    match mint(
        ctx,
        &MintSpec {
            who,
            account,
            major,
            faucet: FAUCET_MIGRATION,
            namespace: MIGRATION_KEY_NS,
            correlation: "migration",
            instrument: &instrument,
        },
    ) {
        Ok(MintDone {
            amount,
            transaction,
            key,
        }) => {
            profile.migrated = Some(Migration {
                from_sheet_value: line.sheet_value,
                approved_at,
                approved_by,
                transaction,
                at: ctx.w.now(),
            });
            if profile.minted.is_none() {
                profile.minted = Some(Mint {
                    key,
                    transaction,
                    amount,
                    at: ctx.w.now(),
                });
            }
            profile.revision = profile.revision.saturating_add(1);
            put_profile(ctx, profile);
            format!(
                "#{}: minted {} from system:{FAUCET_MIGRATION} (sheet value {} bound).",
                who.0,
                amount.minor(),
                line.sheet_value
            )
        }
        Err(why) => {
            profile.revision = profile.revision.saturating_add(1);
            put_profile(ctx, profile);
            format!("#{}: not minted ({why}).", who.0)
        }
    }
}

/// The approval stamp on a sheet, when it has one.
#[must_use]
pub fn approval_stamp(doc: &SheetDoc) -> Option<(ObjId, i64)> {
    match doc.approved {
        ApprovalState::Approved { by, at } => Some((by, at)),
        ApprovalState::Unapproved | ApprovalState::Pending(_) => None,
    }
}
