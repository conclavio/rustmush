//! E7c3 B3/B7 — the five command lanes.
//!
//! # K8.5 classification, per registration group
//!
//! `reg.classify_commands(..)` is called **before the first `reg.command` in
//! each group**, which the production composition refuses otherwise. Each group
//! takes the **ceiling class of its own switches**, and conservative
//! over-declaration is permitted (08j §7 "Accept"):
//!
//! | group | class | why |
//! |---|---|---|
//! | `@deposit` | `Authoritative` | `open` and `access` ride cut 2's B1 ingress seam; `list`/`show` never submit. **A deviation from the brief's `ReadOnly`, with the reason**: a body's reserve *is* cut 2's pool, and a pool can only be credited inside an authoritative bundle, so opening a body is an authoritative act. Cut 2 made the identical argument for `@inv`'s four read switches |
//! | `@licence` | `ReadOnly` | it reads the acting principal's own entitlement account and writes nothing at all |
//! | `@survey` | `LegacyPostImage` | it writes the one conventional document this cut owns, and **no legality check reads a band**, which is exactly what keeps it exempt under §4.1's closing rule |
//! | `@claim` | `Authoritative` | every switch either rides the prepared path or posts an E4 asset op |
//! | `@extract` | `Authoritative` | every switch rides the prepared path |
//!
//! # Disclosure (§4 rule 4)
//!
//! `@deposit/show` on an unknown id and on a known unsurveyed one are
//! **byte-identical**, and so is a claim refused over a body the actor may not
//! know exists. One internal view feeds every projection: [`render_deposit`] is
//! the only place a body is rendered, and it takes the viewer.

use std::sync::Arc;

use std::collections::BTreeSet;

use mush_core::{
    ArgShape, CanonicalId, CmdError, CommandCompatibility, CommandSpec, Ctx, IdempotencyKeyText,
    ModError, ObjId, PerceivedRef, Registrar,
};
use mush_economy::{CatalogRef, EconomyStore};
use mush_org::PrincipalId;
use serde_json::json;
use smol_str::SmolStr;

use crate::facility::{FacilityRecord, facility_capacity_key};
use crate::families::IndustryFamilies;
use crate::licence::licence_accounts;
use crate::provider::{IndustryVerb, action_ref};
use crate::record::{
    ClaimId, ClaimSubject, ContestState, INDUSTRY_PAGE, MAX_LIST_PAGES, MAX_LIST_ROWS, OreDeposit,
    OreDepositId, SurveyBands, SurveyDoc,
};
use crate::store::IndustryStore;
use crate::workorder::{
    MAX_WORK_ORDERS_PER_REQUESTER, RecipeAuthority, WorkOrder, WorkOrderId, quote_duration,
    quote_price,
};

/// The one line an unknown, unreadable or unsurveyed ore body renders.
///
/// **Byte-identical** for all three, deliberately: a claim refusal over a body
/// the actor may not know exists must say nothing about its existence.
pub const NO_SUCH_DEPOSIT: &str = "No such ore body.";
/// The one line an unknown or unreadable claim renders.
pub const NO_SUCH_CLAIM: &str = "No such claim.";

fn key(text: &str) -> Result<IdempotencyKeyText, CmdError> {
    IdempotencyKeyText::parse_canonical(text)
        .map_err(|_| CmdError::msg("Industry is not available on this server."))
}

/// Submit one authoritative act on cut 2's B1 ingress seam.
///
/// The handler performs **no** mutation of its own and emits no success line:
/// the mechanic is the provider's and the confirmation is the receipt's, held
/// behind the durable ack.
fn submit(
    ctx: &mut Ctx<'_>,
    verb: IndustryVerb,
    parameters: serde_json::Value,
    discriminator: &str,
) -> Result<(), CmdError> {
    let Some(action) = action_ref(ctx.reg, verb) else {
        return Err(CmdError::msg("Industry is not available on this server."));
    };
    // Cut 2's fold R1(b): the discriminator carries the IC second and an
    // ordinal inside it, so a deliberately retyped intent is a NEW request
    // while driving the same submission twice is still one commit. It is a
    // sequence, never a random: nothing here draws.
    let at = ctx.w.world_ic_now().at;
    let attempt = ctx
        .w
        .module_state_mut::<IndustryStore>()
        .map_or(0, |store| store.next_attempt(at));
    let discriminator = format!("{discriminator}.{at:x}.{attempt:x}");
    ctx.submit_authoritative(
        action,
        PerceivedRef::SelfActor,
        Vec::new(),
        parameters,
        key(&discriminator)?,
    );
    Ok(())
}

/// The one internal view every deposit projection is built from.
///
/// **The reserve, the curve and another principal's band never leave it.** A
/// surveyed principal sees their own two recorded bands; staff see the exact
/// figures; everyone else sees the body's place and commodity and nothing
/// numeric at all.
#[must_use]
pub fn render_deposit(
    ctx: &Ctx<'_>,
    deposit: &OreDeposit,
    survey: Option<&SurveyBands>,
    staff: bool,
) -> Vec<String> {
    let mut rows = vec![format!(
        "{} - {} at #{}",
        deposit.id(),
        deposit.commodity().as_str(),
        deposit.place().0
    )];
    if let Some(bands) = survey {
        rows.push(format!(
            "  grade band {} | reserve band {}",
            bands.grade_class, bands.reserve_class
        ));
    }
    if staff {
        rows.push(format!(
            "  [staff] authored reserve {} | accessibility {}% | grade {} | curve {} points",
            deposit.initial_reserve(),
            deposit.accessibility().get(),
            deposit.grade().get(),
            deposit.curve().points().len()
        ));
        if let Some(why) = ctx
            .w
            .module_state::<IndustryStore>()
            .and_then(|store| store.quarantine(deposit.id()))
        {
            rows.push(format!("  [staff] QUARANTINED: {}", why.runbook()));
        }
    }
    rows
}

/// The economic principal a character acts as by default.
#[must_use]
pub fn actor_principal(actor: ObjId) -> PrincipalId {
    PrincipalId::Object(actor)
}

fn deposit_by_id(
    ctx: &Ctx<'_>,
    families: &IndustryFamilies,
    id: OreDepositId,
) -> Option<OreDeposit> {
    ctx.action_state(&families.deposits, &id)
        .ok()
        .flatten()
        .cloned()
}

fn claim_by_id(
    ctx: &Ctx<'_>,
    families: &IndustryFamilies,
    id: ClaimId,
) -> Option<crate::ClaimRecord> {
    ctx.action_state(&families.claims, &id)
        .ok()
        .flatten()
        .cloned()
}

/// Every ore body **this viewer may see**, bounded on records **visited**
/// (D-LBK-5), with an honest continuation cursor rather than a silent
/// truncation.
///
/// The visibility filter runs **inside** the page walk, not after it. That is
/// cut 2's fold B1 finding, and shipping it again would be the same bug: a
/// viewer with one surveyed body among a thousand would have their own row
/// displaced by unrelated ones and be told `(nothing here)`. The **page**
/// budget counts records visited; the **row** budget counts rows the viewer
/// actually gets.
fn list_deposits(
    ctx: &Ctx<'_>,
    families: &IndustryFamilies,
    place: Option<ObjId>,
    after: Option<&str>,
    visible: &dyn Fn(&OreDeposit) -> bool,
) -> (Vec<OreDeposit>, Option<String>) {
    let mut out = Vec::new();
    let mut cursor = after.map(str::to_owned);
    let mut pages = 0;
    loop {
        if pages >= MAX_LIST_PAGES || out.len() >= MAX_LIST_ROWS {
            break;
        }
        pages += 1;
        let Ok(page) = ctx.scan_action_state(&families.deposits, cursor.as_deref(), INDUSTRY_PAGE)
        else {
            break;
        };
        for entry in page.entries() {
            if out.len() >= MAX_LIST_ROWS {
                break;
            }
            let deposit = entry.value().clone();
            if place.is_none_or(|place| deposit.place() == place) && visible(&deposit) {
                out.push(deposit);
            }
        }
        let next = page.next_after().map(ToString::to_string);
        match next {
            Some(next) => cursor = Some(next),
            None => return (out, None),
        }
    }
    (out, cursor)
}

// ---- @deposit ----

fn cmd_deposit(ctx: &mut Ctx<'_>, families: &IndustryFamilies) -> Result<(), CmdError> {
    let staff = crate::store::is_staff(ctx.w, ctx.actor);
    if ctx.switches.has("open") {
        if !staff {
            return Err(CmdError::msg(NO_SUCH_DEPOSIT));
        }
        // `<place>=<commodity>/<grade>/<accessibility>/<grade band>/<reserve band>/
        //  <curve>/<endpoint>/<reserve>`
        let lhs = ctx.args.lhs().trim().to_owned();
        let rhs = ctx.args.rhs().unwrap_or_default().trim().to_owned();
        let place: u32 = lhs
            .trim_start_matches('#')
            .parse()
            .map_err(|_| CmdError::msg("Usage: @deposit/open <place>=<spec>"))?;
        let parts: Vec<&str> = rhs.split('/').collect();
        if parts.len() != 8 {
            return Err(CmdError::msg(
                "Usage: @deposit/open <place>=<commodity>/<grade>/<accessibility>/\
                 <grade band>/<reserve band>/<curve>/<endpoint>/<reserve>",
            ));
        }
        let parameters = json!({
            "place": place,
            "commodity": parts[0].trim(),
            "grade": parts[1].trim().parse::<u64>().unwrap_or(u64::MAX),
            "accessibility": parts[2].trim().parse::<u64>().unwrap_or(u64::MAX),
            "grade_class": parts[3].trim(),
            "reserve_class": parts[4].trim(),
            "curve": parts[5].trim(),
            "endpoint": parts[6].trim(),
            "reserve": parts[7].trim().parse::<u64>().unwrap_or(0),
        });
        return submit(ctx, IndustryVerb::DepositOpen, parameters, "deposit.open");
    }
    if ctx.switches.has("access") {
        if !staff {
            return Err(CmdError::msg(NO_SUCH_DEPOSIT));
        }
        let lhs = ctx.args.lhs().trim().to_owned();
        let rhs = ctx.args.rhs().unwrap_or_default().trim().to_owned();
        let (accessibility, event) = rhs
            .split_once('/')
            .ok_or_else(|| CmdError::msg("Usage: @deposit/access <body>=<0-100>/<event>"))?;
        let parameters = json!({
            "deposit": lhs,
            "accessibility": accessibility.trim().parse::<u64>().unwrap_or(u64::MAX),
            "event": event.trim(),
        });
        return submit(
            ctx,
            IndustryVerb::DepositAccess,
            parameters,
            "deposit.access",
        );
    }
    if ctx.switches.has("show") {
        let raw = ctx.args.lhs().trim().to_owned();
        // §4 rule 4: an unknown id and a known-but-unsurveyed one are the SAME
        // bytes. The only difference a viewer may observe is their own band.
        let Some(id) = OreDepositId::parse(&raw) else {
            ctx.out.info(ctx.actor, NO_SUCH_DEPOSIT);
            return Ok(());
        };
        let Some(deposit) = deposit_by_id(ctx, families, id) else {
            ctx.out.info(ctx.actor, NO_SUCH_DEPOSIT);
            return Ok(());
        };
        let principal = actor_principal(ctx.actor);
        let bands = ctx.w.module_state::<IndustryStore>().and_then(|store| {
            store
                .survey(id)
                .and_then(|doc| doc.bands_for(&principal))
                .cloned()
        });
        if bands.is_none() && !staff {
            // Unsurveyed and not staff: the SAME line an unknown id renders.
            ctx.out.info(ctx.actor, NO_SUCH_DEPOSIT);
            return Ok(());
        }
        for row in render_deposit(ctx, &deposit, bands.as_ref(), staff) {
            ctx.out.info(ctx.actor, row);
        }
        return Ok(());
    }
    // `list`
    let raw = ctx.args.lhs().trim().to_owned();
    let place = raw
        .trim_start_matches('#')
        .parse::<u32>()
        .ok()
        .map(ObjId)
        .filter(|_| !raw.is_empty());
    let after = ctx.args.rhs().map(|rhs| rhs.trim().to_owned());
    let principal = actor_principal(ctx.actor);
    let store = ctx.w.module_state::<IndustryStore>();
    // A body you have not surveyed is not listed at all: listing it without a
    // band would still say "it exists here", which is the one thing an
    // unsurveyed body must not say.
    let visible = |deposit: &OreDeposit| {
        staff
            || store.is_some_and(|store| {
                store
                    .survey(deposit.id())
                    .is_some_and(|doc| doc.bands_for(&principal).is_some())
            })
    };
    let (rows, cursor) = list_deposits(ctx, families, place, after.as_deref(), &visible);
    let mut shown = 0;
    for deposit in rows {
        let bands = ctx.w.module_state::<IndustryStore>().and_then(|store| {
            store
                .survey(deposit.id())
                .and_then(|doc| doc.bands_for(&principal))
                .cloned()
        });
        for row in render_deposit(ctx, &deposit, bands.as_ref(), staff) {
            ctx.out.info(ctx.actor, row);
        }
        shown += 1;
    }
    if shown == 0 {
        ctx.out.info(ctx.actor, "(nothing here)");
    }
    if let Some(cursor) = cursor {
        ctx.out
            .info(ctx.actor, format!("(more: @deposit/list <place>={cursor})"));
    }
    Ok(())
}

// ---- @survey ----

fn cmd_survey(ctx: &mut Ctx<'_>, families: &IndustryFamilies) -> Result<(), CmdError> {
    let raw = ctx.args.lhs().trim().to_owned();
    let staff = crate::store::is_staff(ctx.w, ctx.actor);
    let Some(id) = OreDepositId::parse(&raw) else {
        ctx.out.info(ctx.actor, NO_SUCH_DEPOSIT);
        return Ok(());
    };
    let Some(deposit) = deposit_by_id(ctx, families, id) else {
        ctx.out.info(ctx.actor, NO_SUCH_DEPOSIT);
        return Ok(());
    };
    if crate::store::is_quarantined(ctx.w, id) {
        ctx.out.info(ctx.actor, NO_SUCH_DEPOSIT);
        return Ok(());
    }
    // **You survey what you are standing on.** Without this, `@survey <id>` is
    // an existence oracle over the whole id space for anybody who guesses: it
    // is the one verb that turns "I have never heard of this body" into "here
    // are its two bands". The refusal is the SAME line an unknown id renders,
    // so being in the wrong place and naming nothing are indistinguishable.
    let here = ctx.w.get(ctx.actor).and_then(|actor| actor.location);
    if here != Some(deposit.place()) && !staff {
        ctx.out.info(ctx.actor, NO_SUCH_DEPOSIT);
        return Ok(());
    }
    let principal = actor_principal(ctx.actor);
    let now = ctx.w.world_ic_now();
    // **The survey copies the deposit's RECORDED classes verbatim.** It is
    // never `reserve` re-quantised per viewer, which is exactly what makes a
    // repeated survey unable to narrow the band and two principals' bands the
    // same two `BandId`s. It draws nothing: a replay is a HIT.
    let bands = SurveyBands {
        grade_class: deposit.grade_class().clone(),
        reserve_class: deposit.reserve_class().clone(),
        at: now,
        fact: mush_economy::ExternalRef {
            provider: SmolStr::new_static(crate::record::NS),
            kind: SmolStr::new_static("survey"),
            id: SmolStr::new(format!("survey/{id}")),
            revision: Some(1),
        },
    };
    let existing = ctx
        .w
        .module_state::<IndustryStore>()
        .and_then(|store| store.survey(id).cloned())
        .unwrap_or_else(|| SurveyDoc::empty(id));
    let (next, fresh) = match existing.disclosed(&principal, bands.clone()) {
        Ok(result) => result,
        Err(_) => {
            // E7c3 verify fold B6: past `MAX_SURVEY_BANDS` a new principal's
            // disclosure refuses. Collapse it to the **same** line an unknown
            // body renders rather than surfacing "a deposit discloses to at most
            // 8 principals", which minor-leaked that the surveyor roster is full
            // to a co-located actor. The cap is a lifetime one and stands (rows
            // are never evicted); only the disclosure of it changes.
            ctx.out.info(ctx.actor, NO_SUCH_DEPOSIT);
            return Ok(());
        }
    };
    if fresh {
        if let Some(store) = ctx.w.module_state_mut::<IndustryStore>() {
            store.put_survey(next.clone());
        }
        crate::store::persist_survey(ctx.out, &next);
    }
    let shown = next.bands_for(&principal).cloned().unwrap_or(bands);
    ctx.out.info(
        ctx.actor,
        format!(
            "{id}: grade band {} | reserve band {}",
            shown.grade_class, shown.reserve_class
        ),
    );
    if !fresh {
        // A re-survey tells you the SAME two bands. There is no narrowing and
        // no second roll, because there was never a roll.
        ctx.out
            .info(ctx.actor, "(you had already surveyed this body)");
    }
    Ok(())
}

// ---- @licence ----

fn cmd_licence(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let raw = ctx.args.lhs().trim().to_owned();
    let staff = crate::store::is_staff(ctx.w, ctx.actor);
    let who = if raw.is_empty() {
        actor_principal(ctx.actor)
    } else if staff {
        crate::provider::parse_principal(&raw)
            .ok_or_else(|| CmdError::msg("Usage: @licence/show [obj:<n>|org:<n>|sys:<name>]"))?
    } else {
        // A principal may read their own standing and nobody else's.
        actor_principal(ctx.actor)
    };
    let Some(instrument) = ctx
        .w
        .module_state::<IndustryStore>()
        .and_then(|store| store.licence().cloned())
    else {
        ctx.out
            .info(ctx.actor, "This server issues no extraction licences.");
        return Ok(());
    };
    if ctx.w.module_state::<EconomyStore>().is_none() {
        ctx.out
            .info(ctx.actor, "This server issues no extraction licences.");
        return Ok(());
    }
    match licence_accounts(ctx.w, &instrument, &who) {
        Some(status) => ctx.out.info(
            ctx.actor,
            format!("Extraction licence in {}: {status:?}.", instrument.as_str()),
        ),
        None => ctx.out.info(
            ctx.actor,
            "No extraction licence stands for that principal.",
        ),
    }
    Ok(())
}

// ---- @claim ----

#[allow(clippy::too_many_lines)]
fn cmd_claim(
    ctx: &mut Ctx<'_>,
    families: &IndustryFamilies,
    catalog: &(SmolStr, SmolStr),
) -> Result<(), CmdError> {
    let staff = crate::store::is_staff(ctx.w, ctx.actor);
    if ctx.switches.has("file") {
        // `<subject>=<issuer>/<holder>/<exclusivity>/<priority>/<term seconds>/
        //  <transferability>`
        let lhs = ctx.args.lhs().trim().to_owned();
        let rhs = ctx.args.rhs().unwrap_or_default().trim().to_owned();
        let parts: Vec<&str> = rhs.split('/').collect();
        if parts.len() != 6 {
            return Err(CmdError::msg(
                "Usage: @claim/file <subject>=<issuer>/<holder>/<exclusive|shared>/\
                 <priority>/<term seconds>/<transferable|bound>",
            ));
        }
        let holder = crate::provider::parse_principal(parts[1].trim())
            .ok_or_else(|| CmdError::msg(NO_SUCH_CLAIM))?;
        // The claim's **titled world object**: created and titled here, on E4's
        // own journaled path, before the record half is submitted. Cut 3
        // defines no second title record and no second transfer path.
        let Ok(object) = ctx.w.create(
            mush_core::ObjKind::Thing,
            &format!("Claim over {lhs}"),
            ctx.actor,
            None,
        ) else {
            return Err(CmdError::msg(NO_SUCH_CLAIM));
        };
        let issued = mush_economy::issue_title(
            ctx,
            object,
            holder,
            None,
            crate::module::industry_title_issuer(),
            Some(CatalogRef {
                ruleset: catalog.0.clone(),
                code: catalog.1.clone(),
            }),
        );
        if let Err(error) = issued {
            // `issue_title` refuses a non-live object and an unregistered
            // issuer; both are wiring, and both are said out loud to staff.
            if staff {
                ctx.out
                    .info(ctx.actor, format!("[staff] title refused: {error}"));
            }
            return Err(CmdError::msg(NO_SUCH_CLAIM));
        }
        let term = ctx.w.world_ic_now();
        let term_secs: i64 = parts[4].trim().parse().unwrap_or(0);
        let term = mush_core::ClockStamp {
            clock: term.clock,
            at: term.at.saturating_add(term_secs),
        };
        let parameters = json!({
            "subject": lhs,
            "issuer": parts[0].trim(),
            "holder": parts[1].trim(),
            "object": object.0,
            "exclusivity": parts[2].trim(),
            "priority": parts[3].trim().parse::<u64>().unwrap_or(u64::MAX),
            "term": term,
            "transferability": parts[5].trim(),
        });
        return submit(ctx, IndustryVerb::ClaimFile, parameters, "claim.file");
    }
    if ctx.switches.has("transfer") {
        let lhs = ctx.args.lhs().trim().to_owned();
        let rhs = ctx.args.rhs().unwrap_or_default().trim().to_owned();
        let Some(id) = ClaimId::parse(&lhs) else {
            return Err(CmdError::msg(NO_SUCH_CLAIM));
        };
        let Some(claim) = claim_by_id(ctx, families, id) else {
            return Err(CmdError::msg(NO_SUCH_CLAIM));
        };
        let Some(to) = crate::provider::parse_principal(&rhs) else {
            return Err(CmdError::msg(NO_SUCH_CLAIM));
        };
        // E4's own rule, reused verbatim: `AssetOp::TransferTitle` refuses with
        // `Encumbered` while ANY encumbrance stands, so a **pledged** claim
        // cannot be transferred and cut 3 writes no second check for it.
        let plan = mush_economy::EconomyPlan {
            asset_ops: vec![mush_economy::AssetOp::TransferTitle {
                object: claim.object(),
                to: to.clone(),
                to_scope: None,
            }],
            ..mush_economy::EconomyPlan::new(
                mush_economy::IdempotencyKey::new("industry.claim.transfer", format!("{id}")),
                actor_principal(ctx.actor),
                mush_economy::EconomicCause::Internal {
                    kind: SmolStr::new_static("industry.claim.transfer"),
                    id: claim.object().0.into(),
                },
                Vec::new(),
            )
        };
        if let Err(error) = mush_economy::post(ctx, &plan) {
            if staff {
                ctx.out
                    .info(ctx.actor, format!("[staff] title refused: {error}"));
            }
            return Err(CmdError::msg(NO_SUCH_CLAIM));
        }
        let parameters = json!({
            "claim": lhs,
            "holder": rhs,
            "revision": claim.revision(),
        });
        return submit(ctx, IndustryVerb::ClaimMove, parameters, "claim.transfer");
    }
    for (switch, state) in [
        ("contest", "contested"),
        ("suspend", "suspended"),
        ("seize", "seized"),
        ("release", "uncontested"),
    ] {
        if !ctx.switches.has(switch) {
            continue;
        }
        let lhs = ctx.args.lhs().trim().to_owned();
        let rhs = ctx.args.rhs().unwrap_or_default().trim().to_owned();
        let Some(id) = ClaimId::parse(&lhs) else {
            return Err(CmdError::msg(NO_SUCH_CLAIM));
        };
        let Some(claim) = claim_by_id(ctx, families, id) else {
            return Err(CmdError::msg(NO_SUCH_CLAIM));
        };
        if switch == "seize" && pledged(ctx, claim.object()) {
            // **Refuse and page level 2 with the command filled in**: seizure
            // would silently destroy a lender's collateral, and that is a
            // genuine decision, not a chore. Only the subject is stopped.
            tracing::error!(
                target: "industry",
                claim = %id,
                object = claim.object().0,
                runbook = %format!(
                    "a pledged claim cannot be seized while its collateral stands; settle or \
                     release the contract, then `@claim/seize {id}`"
                ),
                "industry: seizure refused on a pledged claim (level 2)"
            );
            ctx.out.info(
                ctx.actor,
                "That claim is pledged as collateral; seizing it would destroy a lender's \
                 security. Staff have been paged.",
            );
            return Ok(());
        }
        if switch == "contest" && disputed(ctx, claim.object()) {
            // A contest whose adjudicator contract is itself `Disputed`
            // **suspends the claim rather than resolving it**.
            tracing::error!(
                target: "industry",
                claim = %id,
                runbook = %format!(
                    "the adjudicating contract for {id} is itself disputed; resolve the dispute, \
                     then `@claim/contest {id}`"
                ),
                "industry: a contest over a disputed adjudicator suspends the claim (level 2)"
            );
            let parameters =
                json!({ "claim": lhs, "state": "suspended", "revision": claim.revision() });
            return submit(ctx, IndustryVerb::ClaimContest, parameters, "claim.suspend");
        }
        let contract = rhs.parse::<u64>().unwrap_or(0);
        let parameters = json!({
            "claim": lhs,
            "state": state,
            "revision": claim.revision(),
            "contract": contract,
        });
        return submit(
            ctx,
            IndustryVerb::ClaimContest,
            parameters,
            &format!("claim.{switch}"),
        );
    }
    if ctx.switches.has("pledge") {
        // Pledging is E4's `Encumbrance::Collateral` on the claim's titled
        // object, posted through the economy's own path. Cut 3 defines no
        // encumbrance model of its own.
        let lhs = ctx.args.lhs().trim().to_owned();
        let rhs = ctx.args.rhs().unwrap_or_default().trim().to_owned();
        let Some(id) = ClaimId::parse(&lhs) else {
            return Err(CmdError::msg(NO_SUCH_CLAIM));
        };
        let Some(claim) = claim_by_id(ctx, families, id) else {
            return Err(CmdError::msg(NO_SUCH_CLAIM));
        };
        let contract = rhs
            .parse::<u64>()
            .map_err(|_| CmdError::msg("Usage: @claim/pledge <claim>=<contract>"))?;
        let plan = mush_economy::EconomyPlan {
            asset_ops: vec![mush_economy::AssetOp::Encumber {
                object: claim.object(),
                encumbrance: mush_economy::Encumbrance::Collateral {
                    contract: mush_economy::ContractId(contract),
                },
            }],
            ..mush_economy::EconomyPlan::new(
                mush_economy::IdempotencyKey::new("industry.claim.pledge", format!("{id}")),
                actor_principal(ctx.actor),
                mush_economy::EconomicCause::Internal {
                    kind: SmolStr::new_static("industry.claim.pledge"),
                    id: claim.object().0.into(),
                },
                Vec::new(),
            )
        };
        if let Err(error) = mush_economy::post(ctx, &plan) {
            if staff {
                ctx.out
                    .info(ctx.actor, format!("[staff] pledge refused: {error}"));
            }
            return Err(CmdError::msg(NO_SUCH_CLAIM));
        }
        ctx.out.info(ctx.actor, format!("{id} is pledged."));
        return Ok(());
    }
    // `show`
    let lhs = ctx.args.lhs().trim().to_owned();
    let Some(id) = ClaimId::parse(&lhs) else {
        ctx.out.info(ctx.actor, NO_SUCH_CLAIM);
        return Ok(());
    };
    let Some(claim) = claim_by_id(ctx, families, id) else {
        ctx.out.info(ctx.actor, NO_SUCH_CLAIM);
        return Ok(());
    };
    // E7c3 verify fold B3: `@claim/show` is gated to staff, the holder, the
    // issuer, or an actor standing where the claim's subject is. Otherwise it
    // renders `NO_SUCH_CLAIM` byte-identically to an unknown id — without this
    // gate, anyone who learned a claim id read the **fog-gated deposit id** and
    // both claimant principals out of a bare family read (the same existence
    // oracle `@survey`'s locality gate and `@deposit/show`'s byte-identity close).
    let here = ctx.w.get(ctx.actor).and_then(|actor| actor.location);
    let subject_place = match claim.subject() {
        ClaimSubject::Deposit { deposit } => {
            deposit_by_id(ctx, families, deposit).map(|body| body.place())
        }
        ClaimSubject::Place { place } => Some(place),
    };
    let authorized = staff
        || mush_economy::acts_for(ctx.w, ctx.actor, claim.holder())
        || mush_economy::acts_for(ctx.w, ctx.actor, claim.issuer())
        || (subject_place.is_some() && here == subject_place);
    if !authorized {
        ctx.out.info(ctx.actor, NO_SUCH_CLAIM);
        return Ok(());
    }
    // Explicit formatting, never `{:?}` (which leaked the raw `OreDepositId`):
    // a non-staff viewer never sees the fog-gated deposit id, and the contest
    // state renders as a word rather than a Debug dump.
    ctx.out.info(
        ctx.actor,
        format!(
            "{id} - {} | issuer {} | holder {} | priority {} | {}",
            claim_subject_label(claim.subject(), staff),
            mush_economy::principal_token(claim.issuer()),
            mush_economy::principal_token(claim.holder()),
            claim.priority(),
            contest_state_label(claim.contest_state()),
        ),
    );
    Ok(())
}

/// Render a claim's subject for `@claim/show`. A non-staff viewer never sees the
/// fog-gated `OreDepositId`; a place is a world location and is shown plainly.
fn claim_subject_label(subject: ClaimSubject, staff: bool) -> String {
    match subject {
        ClaimSubject::Deposit { deposit } => {
            if staff {
                deposit.to_string()
            } else {
                "an ore body".to_owned()
            }
        }
        ClaimSubject::Place { place } => format!("place #{}", place.0),
    }
}

/// The one-word contest state a `@claim/show` line carries.
const fn contest_state_label(state: &ContestState) -> &'static str {
    match state {
        ContestState::Uncontested => "uncontested",
        ContestState::Contested { .. } => "contested",
        ContestState::Suspended { .. } => "suspended",
        ContestState::Seized { .. } => "seized",
    }
}

/// Whether the claim's titled object carries an `Encumbrance::Collateral`.
fn pledged(ctx: &Ctx<'_>, object: ObjId) -> bool {
    ctx.w
        .module_state::<EconomyStore>()
        .and_then(|store| store.snapshot().asset(object).cloned())
        .is_some_and(|asset| {
            asset
                .encumbrances
                .iter()
                .any(|claim| matches!(claim, mush_economy::Encumbrance::Collateral { .. }))
        })
}

/// Whether the claim's titled object carries an `Encumbrance::Disputed`.
fn disputed(ctx: &Ctx<'_>, object: ObjId) -> bool {
    ctx.w
        .module_state::<EconomyStore>()
        .and_then(|store| store.snapshot().asset(object).cloned())
        .is_some_and(|asset| {
            asset
                .encumbrances
                .iter()
                .any(|claim| matches!(claim, mush_economy::Encumbrance::Disputed { .. }))
        })
}

// ---- @extract ----

fn cmd_extract(ctx: &mut Ctx<'_>, _families: &IndustryFamilies) -> Result<(), CmdError> {
    if ctx.switches.has("start") {
        let lhs = ctx.args.lhs().trim().to_owned();
        let rhs = ctx.args.rhs().unwrap_or_default().trim().to_owned();
        let parameters = json!({ "deposit": lhs, "endpoint": rhs });
        return submit(ctx, IndustryVerb::ExtractStart, parameters, "extract.start");
    }
    if ctx.switches.has("stop") {
        let lhs = ctx.args.lhs().trim().to_owned();
        return submit(
            ctx,
            IndustryVerb::ExtractStop,
            json!({ "deposit": lhs }),
            "extract.stop",
        );
    }
    // `cycle`
    let lhs = ctx.args.lhs().trim().to_owned();
    submit(
        ctx,
        IndustryVerb::ExtractCycle,
        json!({ "deposit": lhs }),
        "extract.cycle",
    )
}

/// The line an unknown, unreadable or unviewable facility renders.
pub const NO_SUCH_FACILITY: &str = "No such facility.";
/// The line an unknown or unviewable work order renders — byte-identical to the
/// nonexistent-id line (the P lens).
pub const NO_SUCH_WORK_ORDER: &str = "No such work order.";

// ---- E7c4 B8: @facility (LegacyPostImage — open writes a document) ----

fn facility_available(
    ctx: &Ctx<'_>,
    families: &IndustryFamilies,
    object: ObjId,
    maximum_slots: u32,
) -> u64 {
    // R4 (independent verify fold): a quarantined facility reads 0 — a live order
    // holds a slot whose capacity book is gone, so availability must not rise.
    if ctx
        .w
        .module_state::<IndustryStore>()
        .is_some_and(|store| store.is_facility_quarantined(object))
    {
        return 0;
    }
    let Ok(key) = facility_capacity_key(object) else {
        return u64::from(maximum_slots);
    };
    let scalar = ctx
        .action_state(&families.facility_slots, &key)
        .ok()
        .flatten()
        .copied();
    let book = ctx
        .action_state(&families.facility_books, &key)
        .ok()
        .flatten();
    match (scalar, book) {
        (Some(scalar), Some(book)) => book.checked_available(scalar).unwrap_or(0),
        _ => u64::from(maximum_slots),
    }
}

fn cmd_facility(ctx: &mut Ctx<'_>, families: &IndustryFamilies) -> Result<(), CmdError> {
    if ctx.switches.has("open") {
        // Instantiating a facility binds a pack `FacilityDef` to a seeded object;
        // it is a staff act, seeded as authored data (cut 3's `@deposit/open`
        // precedent). `@facility/open <object>=<slots> [cap...]`.
        if !crate::store::is_staff(ctx.w, ctx.actor) {
            return Err(CmdError::msg("Only staff open facilities."));
        }
        let object = ObjId(ctx.args.lhs().trim().parse::<u32>().map_err(|_| {
            CmdError::msg("Usage: @facility/open <object>=<slots> [capability...]")
        })?);
        let rhs = ctx.args.rhs().unwrap_or_default();
        let mut parts = rhs.split_whitespace();
        let slots: u32 = parts
            .next()
            .and_then(|token| token.parse().ok())
            .ok_or_else(|| {
                CmdError::msg("Usage: @facility/open <object>=<slots> [capability...]")
            })?;
        let mut capabilities = BTreeSet::new();
        for token in parts {
            capabilities.insert(
                CanonicalId::new(token)
                    .map_err(|_| CmdError::msg("a capability is a canonical id"))?,
            );
        }
        let facility =
            FacilityRecord::new(object, PrincipalId::Object(ctx.actor), capabilities, slots)
                .map_err(|error| CmdError::msg(error.to_string()))?;
        crate::store::persist_facility(ctx.out, &facility);
        if let Some(store) = ctx.w.module_state_mut::<IndustryStore>() {
            store.put_facility(facility);
        }
        ctx.out.info(
            ctx.actor,
            format!("Facility #{} opened with {slots} slots.", object.0),
        );
        return Ok(());
    }
    if ctx.switches.has("show") {
        let Ok(id) = ctx.args.lhs().trim().parse::<u32>() else {
            ctx.out.info(ctx.actor, NO_SUCH_FACILITY);
            return Ok(());
        };
        let object = ObjId(id);
        let Some(facility) = ctx
            .w
            .module_state::<IndustryStore>()
            .and_then(|store| store.facility(object).cloned())
        else {
            ctx.out.info(ctx.actor, NO_SUCH_FACILITY);
            return Ok(());
        };
        let available = facility_available(ctx, families, object, facility.maximum_slots());
        ctx.out.info(
            ctx.actor,
            format!(
                "Facility #{}: {} slots ({available} available), capabilities [{}]",
                object.0,
                facility.maximum_slots(),
                facility
                    .capabilities()
                    .iter()
                    .map(CanonicalId::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        );
        return Ok(());
    }
    // list — the store view is byte-ordered by object id (a `BTreeMap`), so it
    // pages with an `after` cursor and an honest `(more:)` line rather than a
    // silent `.take(MAX_LIST_ROWS)` (B3, independent verify fold): past 64
    // facilities the remainder was dropped with no signal. `@facility/list [=<after>]`.
    let after: Option<u32> = ctx
        .args
        .rhs()
        .and_then(|rhs| rhs.trim().trim_start_matches('#').parse::<u32>().ok());
    let facilities: Vec<FacilityRecord> = ctx
        .w
        .module_state::<IndustryStore>()
        .map(|store| {
            store
                .facilities()
                .into_iter()
                .filter(|facility| after.is_none_or(|after| facility.object().0 > after))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    if facilities.is_empty() {
        ctx.out.info(ctx.actor, "(no facilities)");
        return Ok(());
    }
    let mut more: Option<u32> = None;
    for facility in facilities.iter().take(MAX_LIST_ROWS) {
        let available =
            facility_available(ctx, families, facility.object(), facility.maximum_slots());
        ctx.out.info(
            ctx.actor,
            format!(
                "Facility #{}: {} slots ({available} available)",
                facility.object().0,
                facility.maximum_slots()
            ),
        );
    }
    if facilities.len() > MAX_LIST_ROWS {
        more = facilities
            .get(MAX_LIST_ROWS - 1)
            .map(|last| last.object().0);
    }
    if let Some(cursor) = more {
        ctx.out
            .info(ctx.actor, format!("(more: @facility/list ={cursor})"));
    }
    Ok(())
}

// ---- E7c4 B8: @wo (Authoritative — draft/submit/... ride the B1 seam) ----

fn cmd_wo(
    ctx: &mut Ctx<'_>,
    families: &IndustryFamilies,
    recipes: Option<&std::sync::Arc<dyn RecipeAuthority>>,
) -> Result<(), CmdError> {
    if ctx.switches.has("draft") {
        let Some(recipes) = recipes else {
            return Err(CmdError::msg(
                "Work orders are not available on this server.",
            ));
        };
        // `@wo/draft <pack>/<recipe>/<version>=<facility> <qty> <account> <source> <destination> [beneficiary]`
        let lhs = ctx.args.lhs().trim();
        let mut parts = lhs.split('/');
        let pack = CanonicalId::new(parts.next().unwrap_or_default().trim())
            .map_err(|_| CmdError::msg("Usage: @wo/draft <pack>/<recipe>/<version>=..."))?;
        let recipe_id = CanonicalId::new(parts.next().unwrap_or_default().trim())
            .map_err(|_| CmdError::msg("Usage: @wo/draft <pack>/<recipe>/<version>=..."))?;
        let version: u32 = parts
            .next()
            .and_then(|token| token.trim().parse().ok())
            .unwrap_or(1);
        let rhs = ctx.args.rhs().unwrap_or_default();
        let mut rp = rhs.split_whitespace();
        let facility = ObjId(
            rp.next()
                .and_then(|token| token.parse().ok())
                .ok_or_else(|| {
                    CmdError::msg(
                        "Usage: @wo/draft ...=<facility> <qty> <account> <source> <destination>",
                    )
                })?,
        );
        let quantity: u32 = rp.next().and_then(|token| token.parse().ok()).unwrap_or(1);
        let account: u64 = rp.next().and_then(|token| token.parse().ok()).unwrap_or(0);
        let source: u64 = rp.next().and_then(|token| token.parse().ok()).unwrap_or(0);
        let destination: u64 = rp
            .next()
            .and_then(|token| token.parse().ok())
            .ok_or_else(|| CmdError::msg("a work order needs a destination endpoint"))?;
        let beneficiary = rp
            .next()
            .map(str::to_owned)
            .unwrap_or_else(|| format!("obj:{}", ctx.actor.0));
        let Some((recipe_ref, pinned)) = recipes.resolve(ctx.w, &pack, version, &recipe_id) else {
            // Fog + fail-closed: an unknown or retired recipe is indistinguishable.
            return Err(CmdError::msg("No such recipe."));
        };
        let Some(facility_rec) = ctx
            .w
            .module_state::<IndustryStore>()
            .and_then(|store| store.facility(facility).cloned())
        else {
            return Err(CmdError::msg(NO_SUCH_FACILITY));
        };
        // C2/D2 (independent verify fold): only staff, the facility's owner, or an
        // org/role the actor `acts_for` may draft here — the facility record's own
        // "org/role scope" language, and cut 3's `extraction_authorized` shape. A
        // stranger gets NO_SUCH_FACILITY byte-for-byte: a facility they may not run
        // reveals neither its existence nor its capabilities (the fog rule). This
        // closes the durable-DoS where any actor held a competitor's finite slots.
        if !crate::store::is_staff(ctx.w, ctx.actor)
            && !mush_economy::acts_for(ctx.w, ctx.actor, facility_rec.owner())
        {
            return Err(CmdError::msg(NO_SUCH_FACILITY));
        }
        // R4: a facility whose capacity book was lost is quarantined; refuse until
        // reconciled rather than letting a fresh order overbook a phantom slot.
        if ctx
            .w
            .module_state::<IndustryStore>()
            .is_some_and(|store| store.is_facility_quarantined(facility))
        {
            return Err(CmdError::msg("That facility is unavailable."));
        }
        if !facility_rec.offers(pinned.requires()) {
            return Err(CmdError::msg("That facility cannot run that recipe."));
        }
        // C2/D2 (m2): a per-requester cap. Terminal orders are never pruned, so
        // without this a single actor could grow the family (and hold a rival's
        // slots) without bound with cheap drafts.
        if active_work_orders(ctx, families, ctx.actor) >= MAX_WORK_ORDERS_PER_REQUESTER {
            return Err(CmdError::msg(
                "you already hold the maximum number of open work orders",
            ));
        }
        let pinned_json = serde_json::to_string(&pinned)
            .map_err(|_| CmdError::msg("that recipe could not be pinned"))?;
        let params = json!({
            "facility": facility.0,
            "facility_slots": facility_rec.maximum_slots(),
            "capacity_revision": facility_rec.capacity_revision(),
            "policy_revision": facility_rec.policy_revision(),
            "pack": pack.as_str(),
            "version": version,
            "recipe": recipe_id.as_str(),
            "digest": recipe_ref.digest().to_hex(),
            "quantity": quantity,
            "account": account,
            "source": source,
            "destination": destination,
            "beneficiary": beneficiary,
            "pinned": pinned_json,
        });
        return submit(ctx, IndustryVerb::WoDraft, params, "wo.draft");
    }
    if ctx.switches.has("submit") {
        let order_arg = ctx.args.lhs().trim().to_owned();
        // B4: revalidate the pinned recipe digest and the facility revisions
        // against the live world before submitting — a drift makes the draft
        // stale and refuses (fail-closed). The reservations themselves happen in
        // the provider's atomic bundle.
        if let Some(order) = wo_by_id(ctx, families, &order_arg) {
            // C2/D2: re-check facility authorization at submit — the drafter's
            // authority may have lapsed since draft (`acts_for` is re-evaluated),
            // and the direct-offer path never ran the draft command's gate.
            let facility_rec = ctx
                .w
                .module_state::<IndustryStore>()
                .and_then(|store| store.facility(order.facility()).cloned());
            let authorized = facility_rec.as_ref().is_some_and(|facility| {
                crate::store::is_staff(ctx.w, ctx.actor)
                    || mush_economy::acts_for(ctx.w, ctx.actor, facility.owner())
            });
            if !authorized {
                return Err(CmdError::msg(NO_SUCH_FACILITY));
            }
            if ctx
                .w
                .module_state::<IndustryStore>()
                .is_some_and(|store| store.is_facility_quarantined(order.facility()))
            {
                return Err(CmdError::msg("That facility is unavailable."));
            }
            let facility_ok = facility_rec.is_some_and(|facility| {
                facility.capacity_revision() == order.capacity_revision()
                    && facility.policy_revision() == order.policy_revision()
            });
            if !facility_ok {
                return Err(CmdError::msg(
                    "that work order is stale — its facility changed; redraft it.",
                ));
            }
            let digest_ok = recipes.is_some_and(|authority| {
                authority.digest_of(ctx.w, order.recipe()) == Some(order.recipe().digest())
            });
            if !digest_ok {
                return Err(CmdError::msg(
                    "that work order's recipe has drifted; redraft it.",
                ));
            }
        }
        return submit(
            ctx,
            IndustryVerb::WoSubmit,
            json!({ "order": order_arg }),
            "wo.submit",
        );
    }
    if ctx.switches.has("deliver") {
        let order = ctx.args.lhs().trim().to_owned();
        return submit(
            ctx,
            IndustryVerb::WoDeliver,
            json!({ "order": order }),
            "wo.deliver",
        );
    }
    if ctx.switches.has("cancel") {
        let order = ctx.args.lhs().trim().to_owned();
        return submit(
            ctx,
            IndustryVerb::WoCancel,
            json!({ "order": order }),
            "wo.cancel",
        );
    }
    if ctx.switches.has("restamp") {
        let order = ctx.args.lhs().trim().to_owned();
        let policy = ctx.args.rhs().unwrap_or_default().trim().to_owned();
        return submit(
            ctx,
            IndustryVerb::WoRestamp,
            json!({ "order": order, "policy": policy }),
            "wo.restamp",
        );
    }
    if ctx.switches.has("quote") {
        // ReadOnly, a **pure function of recorded inputs**, reserving nothing.
        let Some(order) = wo_by_id(ctx, families, ctx.args.lhs().trim()) else {
            ctx.out.info(ctx.actor, NO_SUCH_WORK_ORDER);
            return Ok(());
        };
        if !wo_viewable(ctx, &order) {
            ctx.out.info(ctx.actor, NO_SUCH_WORK_ORDER);
            return Ok(());
        }
        let price = quote_price(order.pinned(), order.quantity());
        let duration = quote_duration(order.pinned(), order.quantity());
        ctx.out.info(
            ctx.actor,
            format!(
                "{}: price {price} input units, duration {duration} work units.",
                order.id()
            ),
        );
        return Ok(());
    }
    if ctx.switches.has("show") {
        let Some(order) = wo_by_id(ctx, families, ctx.args.lhs().trim()) else {
            ctx.out.info(ctx.actor, NO_SUCH_WORK_ORDER);
            return Ok(());
        };
        if !wo_viewable(ctx, &order) {
            // Byte-identical to the nonexistent-id line (the P lens): a projected
            // order carries no hidden org, account or endpoint fact.
            ctx.out.info(ctx.actor, NO_SUCH_WORK_ORDER);
            return Ok(());
        }
        ctx.out.info(
            ctx.actor,
            format!(
                "{}: {:?} at facility #{}, quantity {}.",
                order.id(),
                order.state(),
                order.facility().0,
                order.quantity()
            ),
        );
        return Ok(());
    }
    if ctx.switches.has("set") {
        // B5 (independent verify fold): `@wo/set` was a declared switch with no
        // handler, so it silently ran the list. Per-field mutation is a later
        // cut's; a declared switch must never mean "list", so it errors explicitly.
        return Err(CmdError::msg(
            "@wo/set is not available in this cut (per-field work-order mutation is later cut's).",
        ));
    }
    // list — the actor's own orders, bounded on records visited, with an honest
    // continuation cursor (B2, independent verify fold). Without it, a viewer
    // whose own orders sort beyond `MAX_LIST_PAGES × INDUSTRY_PAGE` saw
    // `(no work orders)` with no way to page on — exactly the displacement bug
    // `@deposit/list` already fixes with an `after` cursor.
    let after = ctx.args.rhs().map(|rhs| rhs.trim().to_owned());
    let mut cursor: Option<String> = after.filter(|value| !value.is_empty());
    let mut pages = 0;
    let mut rows = 0;
    let mut more: Option<String> = None;
    loop {
        if pages >= MAX_LIST_PAGES || rows >= MAX_LIST_ROWS {
            break;
        }
        pages += 1;
        let page =
            match ctx.scan_action_state(&families.work_orders, cursor.as_deref(), INDUSTRY_PAGE) {
                Ok(page) => page,
                Err(_) => break,
            };
        let mut batch: Vec<WorkOrder> = Vec::new();
        for entry in page.entries() {
            batch.push(entry.value().clone());
        }
        let next = page.next_after().map(ToString::to_string);
        for order in batch {
            if wo_viewable(ctx, &order) && rows < MAX_LIST_ROWS {
                rows += 1;
                ctx.out
                    .info(ctx.actor, format!("{}: {:?}", order.id(), order.state()));
            }
        }
        match next {
            // The page budget or the row budget tripped with records still to
            // visit: hand back a cursor so the viewer can page to their own rows.
            Some(next) if rows >= MAX_LIST_ROWS || pages >= MAX_LIST_PAGES => {
                more = Some(next);
                break;
            }
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    if rows == 0 {
        ctx.out.info(ctx.actor, "(no work orders)");
    }
    if let Some(cursor) = more {
        ctx.out
            .info(ctx.actor, format!("(more: @wo/list ={cursor})"));
    }
    Ok(())
}

/// Count one requester's non-terminal work orders, bounded on records visited
/// (D-LBK-5), and stopping as soon as the cap is reached (C2/D2 m2). Never a
/// completeness claim: a truncated walk simply under-counts, which fails closed
/// for the cap (it refuses sooner, never later).
fn active_work_orders(ctx: &Ctx<'_>, families: &IndustryFamilies, actor: ObjId) -> usize {
    let principal = PrincipalId::Object(actor);
    let mut cursor: Option<String> = None;
    let mut pages = 0;
    let mut count = 0;
    loop {
        if pages >= MAX_LIST_PAGES || count >= MAX_WORK_ORDERS_PER_REQUESTER {
            break;
        }
        pages += 1;
        let Ok(page) =
            ctx.scan_action_state(&families.work_orders, cursor.as_deref(), INDUSTRY_PAGE)
        else {
            break;
        };
        let next = page.next_after().map(ToString::to_string);
        for entry in page.entries() {
            let order = entry.value();
            if order.requester() == &principal && !order.state().is_terminal() {
                count += 1;
            }
        }
        match next {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    count
}

fn wo_by_id(ctx: &Ctx<'_>, families: &IndustryFamilies, raw: &str) -> Option<WorkOrder> {
    let id = WorkOrderId::parse(raw)?;
    ctx.action_state(&families.work_orders, &id)
        .ok()
        .flatten()
        .cloned()
}

/// Whether the viewer may see this order — its requester, its beneficiary, or
/// **staff** (B4, independent verify fold: the doc-comment claimed "or staff" but
/// `is_staff` was consulted nowhere, so staff could inspect no work order and
/// could not diagnose a stuck one). One internal view, so an unviewable order and
/// a nonexistent one still render the byte-identical `NO_SUCH_WORK_ORDER` for a
/// genuine non-viewer.
fn wo_viewable(ctx: &Ctx<'_>, order: &WorkOrder) -> bool {
    let principal = PrincipalId::Object(ctx.actor);
    order.requester() == &principal
        || order.beneficiary() == &principal
        || crate::store::is_staff(ctx.w, ctx.actor)
}

/// Register the lanes (cut 3's five plus cut 4's `@facility` and `@wo`).
///
/// # Errors
/// [`ModError`] if registration collides.
pub fn register(
    reg: &mut Registrar<'_>,
    families: &IndustryFamilies,
    catalog: (SmolStr, SmolStr),
    recipes: Option<std::sync::Arc<dyn RecipeAuthority>>,
) -> Result<(), ModError> {
    // K8.5: the ceiling class of `@deposit`'s four switches, declared before
    // the first `reg.command` of this group.
    reg.classify_commands(CommandCompatibility::Authoritative);
    let deposit_families = families.clone();
    reg.command(
        CommandSpec {
            name: "@deposit".into(),
            switches: vec!["open".into(), "access".into(), "list".into(), "show".into()],
            shape: ArgShape::EqPair,
            ..CommandSpec::default()
        },
        Arc::new(move |ctx: &mut Ctx<'_>| cmd_deposit(ctx, &deposit_families)),
    )?;
    let claim_families = families.clone();
    let claim_catalog = catalog.clone();
    reg.command(
        CommandSpec {
            name: "@claim".into(),
            switches: vec![
                "file".into(),
                "show".into(),
                "transfer".into(),
                "pledge".into(),
                "release".into(),
                "contest".into(),
                "suspend".into(),
                "seize".into(),
            ],
            shape: ArgShape::EqPair,
            ..CommandSpec::default()
        },
        Arc::new(move |ctx: &mut Ctx<'_>| cmd_claim(ctx, &claim_families, &claim_catalog)),
    )?;
    let extract_families = families.clone();
    reg.command(
        CommandSpec {
            name: "@extract".into(),
            switches: vec!["start".into(), "cycle".into(), "stop".into()],
            shape: ArgShape::EqPair,
            ..CommandSpec::default()
        },
        Arc::new(move |ctx: &mut Ctx<'_>| cmd_extract(ctx, &extract_families)),
    )?;
    // E7c4 B8: `@wo` rides the same Authoritative group. Its ceiling class is
    // Authoritative (draft/submit/deliver/cancel/restamp ride the B1 seam); the
    // read switches (quote/show/list) never submit, a conservative
    // over-declaration 08j §7 "Accept" permits.
    let wo_families = families.clone();
    let wo_recipes = recipes.clone();
    reg.command(
        CommandSpec {
            name: "@wo".into(),
            switches: vec![
                "draft".into(),
                "set".into(),
                "submit".into(),
                "cancel".into(),
                "deliver".into(),
                "restamp".into(),
                "quote".into(),
                "show".into(),
                "list".into(),
            ],
            shape: ArgShape::EqPair,
            ..CommandSpec::default()
        },
        Arc::new(move |ctx: &mut Ctx<'_>| cmd_wo(ctx, &wo_families, wo_recipes.as_ref())),
    )?;

    // K8.5: a **new registration group**, so the class is declared again. The
    // survey and the facility document are the conventional documents this cut
    // owns (a facility's `open` writes one; `list`/`show` read).
    reg.classify_commands(CommandCompatibility::LegacyPostImage);
    let facility_families = families.clone();
    reg.command(
        CommandSpec {
            name: "@facility".into(),
            switches: vec!["open".into(), "list".into(), "show".into()],
            shape: ArgShape::EqPair,
            ..CommandSpec::default()
        },
        Arc::new(move |ctx: &mut Ctx<'_>| cmd_facility(ctx, &facility_families)),
    )?;
    let survey_families = families.clone();
    reg.command(
        CommandSpec {
            name: "@survey".into(),
            switches: Vec::new(),
            shape: ArgShape::EqPair,
            ..CommandSpec::default()
        },
        Arc::new(move |ctx: &mut Ctx<'_>| cmd_survey(ctx, &survey_families)),
    )?;

    // K8.5: and again — `@licence` writes nothing at all.
    reg.classify_commands(CommandCompatibility::ReadOnly);
    reg.command(
        CommandSpec {
            name: "@licence".into(),
            switches: vec!["show".into()],
            shape: ArgShape::EqPair,
            ..CommandSpec::default()
        },
        Arc::new(cmd_licence),
    )?;
    Ok(())
}
