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

use mush_core::{
    ArgShape, CmdError, CommandCompatibility, CommandSpec, Ctx, IdempotencyKeyText, ModError,
    ObjId, PerceivedRef, Registrar,
};
use mush_economy::{CatalogRef, EconomyStore};
use mush_org::PrincipalId;
use serde_json::json;
use smol_str::SmolStr;

use crate::families::IndustryFamilies;
use crate::licence::licence_accounts;
use crate::provider::{IndustryVerb, action_ref};
use crate::record::{
    ClaimId, INDUSTRY_PAGE, MAX_LIST_PAGES, MAX_LIST_ROWS, OreDeposit, OreDepositId, SurveyBands,
    SurveyDoc,
};
use crate::store::IndustryStore;

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
    let (next, fresh) = existing
        .disclosed(&principal, bands.clone())
        .map_err(|source| CmdError::msg(source.to_string()))?;
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
    ctx.out.info(
        ctx.actor,
        format!(
            "{id} - {:?} | issuer {} | holder {} | priority {} | {:?}",
            claim.subject(),
            mush_economy::principal_token(claim.issuer()),
            mush_economy::principal_token(claim.holder()),
            claim.priority(),
            claim.contest_state()
        ),
    );
    Ok(())
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

/// Register the five lanes.
///
/// # Errors
/// [`ModError`] if registration collides.
pub fn register(
    reg: &mut Registrar<'_>,
    families: &IndustryFamilies,
    catalog: (SmolStr, SmolStr),
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

    // K8.5: a **new registration group**, so the class is declared again. The
    // survey writes the one conventional document this cut owns.
    reg.classify_commands(CommandCompatibility::LegacyPostImage);
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
