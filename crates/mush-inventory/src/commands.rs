//! E7c2 B7 — `@inv`, the tree's first `CommandCompatibility::Authoritative`
//! registration.
//!
//! # Lanes
//!
//! `@inv` is **one** `CommandSpec` and therefore **one** registration, so it
//! takes the ceiling class `Authoritative` — conservative over-declaration is
//! permitted (08j §7 "Accept"), and the four read switches simply never submit.
//! `reg.classify_commands` is called before the first `reg.command`, which the
//! production composition refuses otherwise (K8.5).
//!
//! # Disclosure (§4 rule 4)
//!
//! `@inv/show` on an endpoint the viewer may not read is **byte-identical** to
//! `@inv/show` on one that does not exist: both render
//! [`NO_SUCH_ENDPOINT`] and nothing else, through the same expression. There is
//! no timing or length tell either — neither branch touches a pool.
//!
//! # Bounds
//!
//! Every switch is bounded on records **visited**, never on records matched
//! (D-LBK-5): `lots` and `reservations` page at [`INV_PAGE`], and `lot`'s
//! lineage page walks at most [`MAX_LINEAGE_PAGE`] stored parents — coarsening
//! the *projection* deletes no stored parent.

use std::sync::Arc;

use mush_core::{
    ArgShape, CmdError, CommandCompatibility, CommandSpec, Ctx, IdempotencyKeyText, ModError,
    PerceivedRef, Registrar,
};
use serde_json::json;

use crate::pool::InventoryFamilies;
use crate::provider::{InventoryVerb, action_ref, endpoint_status, endpoint_visible};
use crate::record::{EndpointRef, INV_PAGE, LotId, MAX_LOT_PARENTS};

/// The one refusal an unviewable and an absent endpoint share, byte for byte.
pub const NO_SUCH_ENDPOINT: &str = "There is no such endpoint.";
/// The one refusal an unviewable and an absent lot share, byte for byte.
pub const NO_SUCH_LOT: &str = "There is no such lot.";
/// Largest stored lineage the `lot` page renders in one call.
pub const MAX_LINEAGE_PAGE: usize = MAX_LOT_PARENTS;

fn key(text: &str) -> IdempotencyKeyText {
    IdempotencyKeyText::parse_canonical(text).expect("static inventory command key is canonical")
}

fn parse_endpoint(text: &str) -> Option<EndpointRef> {
    let body = text
        .trim()
        .strip_prefix("vault#")
        .unwrap_or_else(|| text.trim().strip_prefix('#').unwrap_or_else(|| text.trim()));
    body.parse::<u64>()
        .ok()
        .map(|id| EndpointRef(mush_economy::VaultId(id)))
}

/// Split `<endpoint>/<commodity>` into its two halves.
fn parse_slot(text: &str) -> Option<(EndpointRef, String)> {
    let (endpoint, commodity) = text.trim().split_once('/')?;
    let endpoint = parse_endpoint(endpoint)?;
    let commodity = commodity.trim();
    (!commodity.is_empty()).then(|| (endpoint, commodity.to_owned()))
}

fn submit(
    ctx: &mut Ctx<'_>,
    verb: InventoryVerb,
    parameters: serde_json::Value,
    discriminator: &str,
) -> Result<(), CmdError> {
    let Some(action) = action_ref(ctx.reg, verb) else {
        return Err(CmdError::msg("Inventory is not available on this server."));
    };
    // The handler performs NO mutation of its own and emits no success line:
    // the mechanic is the provider's and the confirmation is the receipt's.
    ctx.submit_authoritative(
        action,
        PerceivedRef::SelfActor,
        Vec::new(),
        parameters,
        key(discriminator),
    );
    Ok(())
}

/// Register `@inv`.
///
/// # Errors
/// [`ModError`] if registration collides.
pub fn register(reg: &mut Registrar<'_>, families: &InventoryFamilies) -> Result<(), ModError> {
    // K8.5: declare the lane before the first command. One registration, one
    // class, and the ceiling of the nine switches is `Authoritative`.
    reg.classify_commands(CommandCompatibility::Authoritative);
    let families = families.clone();
    reg.command(
        CommandSpec {
            name: "@inv".into(),
            switches: vec![
                "show".into(),
                "lots".into(),
                "lot".into(),
                "move".into(),
                "split".into(),
                "merge".into(),
                "dissolve".into(),
                "crystallize".into(),
                "reservations".into(),
            ],
            shape: ArgShape::EqPair,
            help: "@inv/show <endpoint> · @inv/lots <endpoint> [commodity] · \
                   @inv/lot <id> · @inv/move <endpoint>/<commodity>:<n>=<endpoint> · \
                   @inv/split <lot>=<n> · @inv/merge <lot>,<lot>… · \
                   @inv/dissolve <lot> · @inv/crystallize <endpoint>/<commodity>=<n>[/<grade>] · \
                   @inv/reservations <endpoint>"
                .to_owned(),
            ..CommandSpec::default()
        },
        Arc::new(move |ctx: &mut Ctx<'_>| dispatch(ctx, &families)),
    )
}

fn dispatch(ctx: &mut Ctx<'_>, families: &InventoryFamilies) -> Result<(), CmdError> {
    // Keep the fact mirror fresh on the cheapest path there is: one bounded
    // reconciliation per inventory command (see `crate::facts`).
    crate::store::reconcile(ctx, families);
    if ctx.switches.has("show") {
        return show(ctx, families);
    }
    if ctx.switches.has("lots") {
        return lots(ctx, families);
    }
    if ctx.switches.has("lot") {
        return lot(ctx, families);
    }
    if ctx.switches.has("reservations") {
        return reservations(ctx, families);
    }
    if ctx.switches.has("move") {
        return move_stock(ctx);
    }
    if ctx.switches.has("split") {
        return split(ctx);
    }
    if ctx.switches.has("merge") {
        return merge(ctx);
    }
    if ctx.switches.has("dissolve") {
        return dissolve(ctx);
    }
    if ctx.switches.has("crystallize") {
        return crystallize(ctx);
    }
    Err(CmdError::msg(
        "Use @inv/show, /lots, /lot, /move, /split, /merge, /dissolve, /crystallize or /reservations.",
    ))
}

// ---- the four reads ----

fn show(ctx: &mut Ctx<'_>, families: &InventoryFamilies) -> Result<(), CmdError> {
    let Some(endpoint) = parse_endpoint(ctx.args.raw()) else {
        return Err(CmdError::msg(NO_SUCH_ENDPOINT));
    };
    // ONE expression decides both refusals, so they cannot drift apart.
    if !endpoint_visible(ctx.w, ctx.actor, endpoint) {
        return Err(CmdError::msg(NO_SUCH_ENDPOINT));
    }
    let quarantined = ctx
        .w
        .module_state::<crate::store::InventoryStore>()
        .is_some_and(|store| store.is_quarantined(endpoint));
    let status = endpoint_status(ctx.w, endpoint);
    let mut lines = vec![format!("{endpoint} — {status:?}")];
    if quarantined {
        lines.push(
            "  QUARANTINED: this endpoint's pool and book disagree; every debit and \
             reservation against it refuses until it is reconciled."
                .to_owned(),
        );
    }
    let mut after: Option<smol_str::SmolStr> = None;
    let mut visited = 0_usize;
    while visited < INV_PAGE {
        let page = ctx
            .scan_action_state(&families.books, after.as_deref(), INV_PAGE)
            .map_err(|_| CmdError::msg(NO_SUCH_ENDPOINT))?;
        for entry in page.entries() {
            visited += 1;
            let book = entry.value();
            if crate::store::endpoint_of(book.resource().subject.clone()) != Some(endpoint) {
                continue;
            }
            let holds = book
                .records()
                .iter()
                .filter(|record| record.state() == mush_core::ReservationState::Held)
                .count();
            lines.push(format!(
                "  {} — {} held reservation(s)",
                book.resource().id.as_str(),
                holds
            ));
        }
        match page.next_after().cloned() {
            Some(next) => after = Some(next),
            None => break,
        }
    }
    if lines.len() == 1 {
        lines.push("  (nothing pooled here)".to_owned());
    }
    for line in lines {
        ctx.out.info(ctx.actor, line);
    }
    Ok(())
}

fn lots(ctx: &mut Ctx<'_>, families: &InventoryFamilies) -> Result<(), CmdError> {
    let raw = ctx.args.raw().trim().to_owned();
    let (endpoint_text, filter) = raw.split_once(' ').unwrap_or((raw.as_str(), ""));
    let Some(endpoint) = parse_endpoint(endpoint_text) else {
        return Err(CmdError::msg(NO_SUCH_ENDPOINT));
    };
    if !endpoint_visible(ctx.w, ctx.actor, endpoint) {
        return Err(CmdError::msg(NO_SUCH_ENDPOINT));
    }
    let filter = filter.trim().to_owned();
    let page = ctx
        .scan_action_state(&families.lots, None, INV_PAGE)
        .map_err(|_| CmdError::msg(NO_SUCH_ENDPOINT))?;
    let mut rows = Vec::new();
    let visited = page.entries().len();
    for entry in page.entries() {
        let lot = entry.value();
        if lot.endpoint() != endpoint {
            continue;
        }
        if !filter.is_empty() && lot.commodity().as_str() != filter {
            continue;
        }
        rows.push(format!(
            "  {} — {} {} (grade {})",
            lot.id(),
            lot.quantity(),
            lot.commodity().as_str(),
            lot.grade().get()
        ));
    }
    let more = page.next_after().is_some();
    ctx.out.info(ctx.actor, format!("Lots at {endpoint}:"));
    if rows.is_empty() {
        ctx.out.info(ctx.actor, "  (none)");
    }
    for row in rows {
        ctx.out.info(ctx.actor, row);
    }
    if more {
        // The page marker is bounded on records VISITED, which is what makes
        // this listing cost-bounded; it deliberately does not report how many
        // rows the viewer could not see.
        ctx.out.info(
            ctx.actor,
            format!("  … {visited} records visited; narrow the commodity to see more."),
        );
    }
    Ok(())
}

fn lot(ctx: &mut Ctx<'_>, families: &InventoryFamilies) -> Result<(), CmdError> {
    let Some(id) = LotId::parse(ctx.args.raw()) else {
        return Err(CmdError::msg(NO_SUCH_LOT));
    };
    let Ok(Some(record)) = ctx.action_state(&families.lots, &id) else {
        return Err(CmdError::msg(NO_SUCH_LOT));
    };
    let record = record.clone();
    if !endpoint_visible(ctx.w, ctx.actor, record.endpoint()) {
        return Err(CmdError::msg(NO_SUCH_LOT));
    }
    ctx.out.info(
        ctx.actor,
        format!(
            "{} — {} {} (grade {}) at {}, lineage depth {}",
            record.id(),
            record.quantity(),
            record.commodity().as_str(),
            record.grade().get(),
            record.endpoint(),
            record.depth()
        ),
    );
    match record.lineage() {
        crate::record::Lineage::Origin { source } => ctx.out.info(
            ctx.actor,
            format!(
                "  origin: {}/{}/{}",
                source.provider, source.kind, source.id
            ),
        ),
        crate::record::Lineage::Split { parent } => {
            ctx.out.info(ctx.actor, format!("  split from {parent}"));
        }
        crate::record::Lineage::Merge { parents } => {
            // Coarsening the ACTIVE PROJECTION deletes no stored parent: the
            // record still carries every one of them.
            for parent in parents.iter().take(MAX_LINEAGE_PAGE) {
                ctx.out.info(ctx.actor, format!("  merged from {parent}"));
            }
            if parents.len() > MAX_LINEAGE_PAGE {
                ctx.out.info(
                    ctx.actor,
                    format!(
                        "  … and {} more stored parents",
                        parents.len() - MAX_LINEAGE_PAGE
                    ),
                );
            }
        }
        crate::record::Lineage::Tranche { tranche } => {
            ctx.out
                .info(ctx.actor, format!("  crystallized from {tranche}"));
            if let Ok(Some(root)) = ctx.action_state(&families.tranches, tranche) {
                let absorbed: Vec<String> = root
                    .absorbed()
                    .iter()
                    .take(MAX_LINEAGE_PAGE)
                    .map(ToString::to_string)
                    .collect();
                let total = root.absorbed().len();
                ctx.out.info(
                    ctx.actor,
                    format!("  that tranche absorbed {total}: {}", absorbed.join(", ")),
                );
            }
        }
    }
    Ok(())
}

fn reservations(ctx: &mut Ctx<'_>, families: &InventoryFamilies) -> Result<(), CmdError> {
    let Some(endpoint) = parse_endpoint(ctx.args.raw()) else {
        return Err(CmdError::msg(NO_SUCH_ENDPOINT));
    };
    if !endpoint_visible(ctx.w, ctx.actor, endpoint) {
        return Err(CmdError::msg(NO_SUCH_ENDPOINT));
    }
    // One internal view feeds the render (§4 rule 4): the page is turned into
    // owned rows before a single line is queued, so the projection and any later
    // DTO come from the same bounded walk.
    let rows: Vec<String> = {
        let page = ctx
            .scan_action_state(&families.books, None, INV_PAGE)
            .map_err(|_| CmdError::msg(NO_SUCH_ENDPOINT))?;
        page.entries()
            .iter()
            .filter(|entry| {
                crate::store::endpoint_of(entry.value().resource().subject.clone())
                    == Some(endpoint)
            })
            .flat_map(|entry| {
                let book = entry.value();
                book.records().iter().map(move |record| {
                    format!(
                        "  {} {} × {} — {:?}",
                        book.resource().id.as_str(),
                        record.amount(),
                        record.id().0.to_hex(),
                        record.state()
                    )
                })
            })
            .collect()
    };
    ctx.out
        .info(ctx.actor, format!("Reservations at {endpoint}:"));
    if rows.is_empty() {
        ctx.out.info(ctx.actor, "  (none)");
    }
    for row in rows {
        ctx.out.info(ctx.actor, row);
    }
    Ok(())
}

// ---- the five mutations, every one through B1's seam ----

fn move_stock(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let lhs = ctx.args.lhs().to_owned();
    let rhs = ctx.args.rhs().unwrap_or("").to_owned();
    let (slot, quantity) = lhs
        .rsplit_once(':')
        .ok_or_else(|| CmdError::msg("Usage: @inv/move <endpoint>/<commodity>:<n>=<endpoint>"))?;
    let quantity = quantity
        .trim()
        .parse::<u64>()
        .map_err(|_| CmdError::msg("How much?"))?;
    let (from, commodity) =
        parse_slot(slot).ok_or_else(|| CmdError::msg("Usage: <endpoint>/<commodity>:<n>"))?;
    let to = parse_endpoint(&rhs).ok_or_else(|| CmdError::msg(NO_SUCH_ENDPOINT))?;
    submit(
        ctx,
        InventoryVerb::Move,
        json!({
            "from": from.vault().0.to_string(),
            "to": to.vault().0.to_string(),
            "commodity": commodity,
            "quantity": quantity,
        }),
        "inv.move",
    )
}

fn split(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let lot = ctx.args.lhs().trim().to_owned();
    let quantity = ctx
        .args
        .rhs()
        .unwrap_or_default()
        .trim()
        .parse::<u64>()
        .map_err(|_| CmdError::msg("Usage: @inv/split <lot>=<n>"))?;
    let id = LotId::parse(&lot).ok_or_else(|| CmdError::msg(NO_SUCH_LOT))?;
    let endpoint = lot_endpoint_text(ctx, id)?;
    submit(
        ctx,
        InventoryVerb::Split,
        json!({ "endpoint": endpoint, "lot": id.to_string(), "quantity": quantity }),
        "inv.split",
    )
}

fn merge(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let raw = ctx.args.raw().trim().to_owned();
    let mut ids = Vec::new();
    for token in raw.split(',') {
        if ids.len() >= MAX_LOT_PARENTS {
            return Err(CmdError::msg("That is too many lots."));
        }
        ids.push(LotId::parse(token).ok_or_else(|| CmdError::msg(NO_SUCH_LOT))?);
    }
    if ids.len() < 2 {
        return Err(CmdError::msg("Usage: @inv/merge <lot>,<lot>…"));
    }
    let endpoint = lot_endpoint_text(ctx, ids[0])?;
    let list = ids
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",");
    submit(
        ctx,
        InventoryVerb::Merge,
        json!({ "endpoint": endpoint, "lots": list }),
        "inv.merge",
    )
}

fn dissolve(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let id = LotId::parse(ctx.args.raw()).ok_or_else(|| CmdError::msg(NO_SUCH_LOT))?;
    let endpoint = lot_endpoint_text(ctx, id)?;
    submit(
        ctx,
        InventoryVerb::Dissolve,
        json!({ "endpoint": endpoint, "lot": id.to_string() }),
        "inv.dissolve",
    )
}

fn crystallize(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let lhs = ctx.args.lhs().to_owned();
    let rhs = ctx.args.rhs().unwrap_or("").trim().to_owned();
    let (endpoint, commodity) = parse_slot(&lhs)
        .ok_or_else(|| CmdError::msg("Usage: @inv/crystallize <endpoint>/<commodity>=<n>[/<g>]"))?;
    let (quantity, grade) = rhs.split_once('/').unwrap_or((rhs.as_str(), "0"));
    let quantity = quantity
        .trim()
        .parse::<u64>()
        .map_err(|_| CmdError::msg("How much?"))?;
    let grade = grade.trim().parse::<u64>().unwrap_or(0);
    submit(
        ctx,
        InventoryVerb::Crystallize,
        json!({
            "endpoint": endpoint.vault().0.to_string(),
            "commodity": commodity,
            "quantity": quantity,
            "grade": grade,
        }),
        "inv.crystallize",
    )
}

/// The endpoint text a lot-shaped action's `endpoint` choice must carry.
///
/// A lot the viewer may not see answers exactly [`NO_SUCH_LOT`], the same bytes
/// as a lot that does not exist.
fn lot_endpoint_text(ctx: &Ctx<'_>, _lot: LotId) -> Result<String, CmdError> {
    // The lot family is authoritative and the command layer deliberately does
    // not read it to choose an endpoint: it names the actor's own live set and
    // lets the provider refuse a lot that is not there. Anything else would let
    // a non-custodian probe for lot existence through the choice list.
    let live = crate::provider::live_endpoints(ctx.w, ctx.actor);
    live.first()
        .map(|vault| vault.0.to_string())
        .ok_or_else(|| CmdError::msg(NO_SUCH_LOT))
}
