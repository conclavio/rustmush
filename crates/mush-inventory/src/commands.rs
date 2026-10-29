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
//! (D-LBK-5), and — since E7c2 fold B1 — a listing pages to exhaustion under a
//! budget counted in **pages** rather than stopping after one. It used to count
//! `visited` against `INV_PAGE`, which is also the page size, so the loop could
//! run exactly once and a viewer's own stock was displaced by unrelated
//! endpoints and reported as *"(nothing pooled here)"*. A truncated listing now
//! says so and hands back a continuation cursor — an **argument**, not a switch —
//! instead of the old *"narrow the commodity to see more"*, which was false
//! because the commodity filter ran after the single page too.
//!
//! `lot`'s lineage page walks at most [`MAX_LINEAGE_PAGE`] stored parents;
//! coarsening the *projection* deletes no stored parent.

use std::sync::Arc;

use mush_core::{
    ArgShape, CmdError, CommandCompatibility, CommandSpec, Ctx, IdempotencyKeyText, ModError,
    PerceivedRef, Registrar, StateFamilyHandle,
};
use serde_json::json;
use smol_str::SmolStr;

use crate::pool::InventoryFamilies;
use crate::provider::{
    InventoryVerb, action_ref, endpoint_status, endpoint_visible, endpoints_truncated,
};
use crate::record::{EndpointRef, INV_PAGE, LotId, MAX_LOT_PARENTS};
use crate::store::InventoryStore;

/// The one refusal an unviewable and an absent endpoint share, byte for byte.
pub const NO_SUCH_ENDPOINT: &str = "There is no such endpoint.";
/// The one refusal an unviewable and an absent lot share, byte for byte.
pub const NO_SUCH_LOT: &str = "There is no such lot.";
/// Largest stored lineage the `lot` page renders in one call.
pub const MAX_LINEAGE_PAGE: usize = MAX_LOT_PARENTS;
/// Largest number of pages one listing visits before it declares itself
/// truncated. Bounded on records **visited**, never on records matched.
pub const MAX_LIST_PAGES: usize = 128;
/// Largest number of rows one listing renders before it hands back a cursor.
pub const MAX_LIST_ROWS: usize = INV_PAGE;
/// Largest continuation cursor one listing accepts, in bytes.
pub const MAX_LIST_CURSOR_BYTES: usize = 128;

/// The command's idempotency discriminator, parsed **fallibly**.
///
/// It used to be a static per-verb string and an `expect`; since E7c2 fold
/// R1(b) it carries an IC-second sequence, so it is derived from world input and
/// no command path may panic on it.
fn key(text: &str) -> Result<IdempotencyKeyText, CmdError> {
    IdempotencyKeyText::parse_canonical(text)
        .map_err(|_| CmdError::msg("Inventory is not available on this server."))
}

/// What one bounded listing walk produced.
struct Listing {
    rows: Vec<String>,
    /// The cursor a follow-up call resumes after, when the walk stopped early.
    next: Option<SmolStr>,
    /// How many records the walk visited.
    visited: usize,
}

/// Page one typed family to exhaustion under an explicit **page** budget,
/// collecting whatever rows the caller's projection yields.
///
/// This is `store::reconcile`'s own loop shape, which was already right; the
/// listings were the ones that were not.
fn walk<K, V>(
    ctx: &Ctx<'_>,
    handle: &StateFamilyHandle<K, V>,
    start: Option<SmolStr>,
    mut project: impl FnMut(&V) -> Vec<String>,
) -> Result<Listing, CmdError>
where
    K: Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    let mut rows: Vec<String> = Vec::new();
    let mut after = start.clone();
    let mut cursor = start;
    let mut visited = 0_usize;
    let mut pages = 0_usize;
    let mut next = None;
    'walk: loop {
        let page = ctx
            .scan_action_state(handle, after.as_deref(), INV_PAGE)
            .map_err(|_| CmdError::msg(NO_SUCH_ENDPOINT))?;
        for entry in page.entries() {
            if rows.len() >= MAX_LIST_ROWS {
                next = cursor.clone();
                break 'walk;
            }
            visited += 1;
            rows.extend(project(entry.value()));
            cursor = Some(SmolStr::new(entry.encoded_key()));
        }
        pages += 1;
        match page.next_after().cloned() {
            Some(more) => after = Some(more),
            None => break,
        }
        if pages >= MAX_LIST_PAGES {
            next = after.clone();
            break;
        }
    }
    Ok(Listing {
        rows,
        next,
        visited,
    })
}

/// Split a trailing `after=<cursor>` argument off a listing's raw arguments.
///
/// A cursor is an **argument**, not a switch: `@inv` stays one primary verb with
/// one allowlist stanza (rule 1 / K8.5).
fn split_cursor(raw: &str) -> Result<(String, Option<SmolStr>), CmdError> {
    let mut head = raw.trim().to_owned();
    let mut cursor = None;
    if let Some(at) = head.rfind("after=") {
        let (before, token) = head.split_at(at);
        let value = token.trim_start_matches("after=").trim();
        if value.len() > MAX_LIST_CURSOR_BYTES {
            return Err(CmdError::msg("That continuation is not one of ours."));
        }
        if !value.is_empty() {
            cursor = Some(SmolStr::new(value));
        }
        head = before.trim().to_owned();
    }
    Ok((head, cursor))
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
    // E7c2 fold C12: an actor holding more live endpoints than one offer may
    // publish is TOLD so. The offer silently omitted the rest, and the parameter
    // refusal that followed read as "that endpoint is not live".
    if endpoints_truncated(ctx.w, ctx.actor) {
        ctx.out.info(
            ctx.actor,
            format!(
                "You hold more than {} live endpoints; only the first {} by id can be \
                 named right now.",
                crate::provider::MAX_ENDPOINT_CHOICES,
                crate::provider::MAX_ENDPOINT_CHOICES
            ),
        );
    }
    // E7c2 fold R1(b) / owner decision D1: the discriminator carries the IC
    // second and an ordinal inside it, so a deliberately retyped intent is a
    // NEW request rather than a silent preflight HIT that moves nothing —
    // while driving the same `AuthoritativeSubmit` value twice is still one
    // commit, and the marker (fold R1(a)) is what makes that true across a
    // reboot. It is a sequence, never a random: nothing here draws.
    let at = ctx.w.world_ic_now().at;
    let attempt = ctx
        .w
        .module_state_mut::<InventoryStore>()
        .map_or(0, |store| store.next_attempt(at));
    let discriminator = format!("{discriminator}.{at:x}.{attempt:x}");
    // The handler performs NO mutation of its own and emits no success line:
    // the mechanic is the provider's and the confirmation is the receipt's.
    ctx.submit_authoritative(
        action,
        PerceivedRef::SelfActor,
        Vec::new(),
        parameters,
        key(&discriminator)?,
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
                "quarantine".into(),
            ],
            shape: ArgShape::EqPair,
            help: "@inv/show <endpoint> [after=<cursor>] · \
                   @inv/lots <endpoint> [commodity] [after=<cursor>] · \
                   @inv/lot <id> · @inv/move <endpoint>/<commodity>:<n>=<endpoint> · \
                   @inv/split <lot>=<n> · @inv/merge <lot>,<lot>… · \
                   @inv/dissolve <lot> · @inv/crystallize <endpoint>/<commodity>=<n>[/<grade>] · \
                   @inv/reservations <endpoint> [after=<cursor>] · @inv/quarantine (staff)"
                .to_owned(),
            ..CommandSpec::default()
        },
        Arc::new(move |ctx: &mut Ctx<'_>| dispatch(ctx, &families)),
    )?;
    // ---- E7r2: P2 — `@salvage`, in the SAME `Authoritative` group ----
    //
    // A **new primary verb**, so it enters the pinned allowlist in this same commit
    // with a stanza of its own — non-baseline, so `PINNED_BASELINE` does not move.
    // It registers after `classify_commands(Authoritative)` above and before any
    // re-classification, which is K8.5's rule: `/recover` stages exactly one
    // `Ctx::submit_authoritative` of the salvage action and `/status` reads, so the
    // ceiling class is `Authoritative`.
    //
    // Recovery is a **command**, never an observer and never a tick arm: the actor is
    // the wreck's own custodian and the authority is theirs, so there is nobody for a
    // GOD tick to act as.
    reg.command(
        CommandSpec {
            name: "@salvage".into(),
            switches: vec!["status".into(), "recover".into()],
            shape: ArgShape::EqPair,
            help: "@salvage/status [<wreck>]              what you may recover, and from \
                   where.\n\
                   @salvage/recover <wreck>/<endpoint>/<lot>=<endpoint>\n\
                   \x20                                    move one stranded lot out of a \
                   wreck you may draw from and into an endpoint you hold. The lot is the SAME \
                   lot: its provenance, grade and quantity are untouched, and nothing is \
                   created. Capture changes custody, never title, so a captor sees no wreck \
                   here at all."
                .to_owned(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_salvage),
    )
}

/// The composed wreck oracle, or `None` on a server that wired none.
fn salvage_source(ctx: &Ctx<'_>) -> Option<Arc<dyn crate::salvage::SalvageSource>> {
    ctx.w
        .module_state::<crate::salvage::SalvageHandle>()
        .and_then(|handle| handle.0.clone())
}

fn cmd_salvage(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.has("recover") {
        return salvage_recover(ctx);
    }
    salvage_status(ctx)
}

/// **What this actor may recover, and from where.**
///
/// It discloses **no other principal's wreck**: the list is `salvage::sites`, which is
/// `may_take_from_vault` over the wreck's own endpoints and nothing else, so a captor
/// and a stranger both read `(nothing to recover)` — the same line an actor reads on a
/// world with no wrecks in it at all.
fn salvage_status(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let Some(source) = salvage_source(ctx) else {
        // Fail-closed and byte-identical to a world with no wrecks: an unwired
        // composition is not announced, because announcing it would tell a player
        // something about the server rather than about the world.
        ctx.out.info(ctx.actor, "(nothing to recover)");
        return Ok(());
    };
    let (sites, truncated) = crate::salvage::sites(ctx.w, source.as_ref(), ctx.actor);
    if sites.is_empty() {
        ctx.out.info(ctx.actor, "(nothing to recover)");
        return Ok(());
    }
    for (root, endpoint) in &sites {
        let site = source
            .wreck(ctx.w, *root)
            .map_or(0, |report| report.site().0);
        ctx.out.info(
            ctx.actor,
            format!(
                "Wreck #{} (in #{site}): endpoint {} — @salvage/recover {}/<lot>=<endpoint>",
                root.0,
                endpoint.vault().0,
                crate::salvage::site_token(*root, *endpoint)
            ),
        );
    }
    if truncated {
        ctx.out.info(
            ctx.actor,
            format!(
                "(more than {} recoverable sites; recover some to see the rest)",
                crate::salvage::MAX_SALVAGE_SITES
            ),
        );
    }
    Ok(())
}

/// `@salvage/recover <wreck>/<endpoint>/<lot>=<endpoint>`.
///
/// The pre-check is a *good refusal*, never the gate: the gate is the offer's own
/// regenerated `Choice` list, which `Engine::resolve_legal_action` rebuilds
/// immediately before preparation, so custody that moved between the command and the
/// commit drops the site out of the list and the submission is refused.
fn salvage_recover(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let lhs = ctx.args.lhs().trim().to_owned();
    let mut parts = lhs.rsplitn(2, '/');
    let Some(lot_text) = parts.next() else {
        return Err(CmdError::msg(crate::salvage::NO_SUCH_WRECK));
    };
    let Some(site_text) = parts.next() else {
        return Err(CmdError::msg(
            "Usage: @salvage/recover <wreck>/<endpoint>/<lot>=<endpoint>",
        ));
    };
    let Some(lot) = LotId::parse(lot_text) else {
        return Err(CmdError::msg(crate::salvage::NO_SUCH_WRECK));
    };
    let Some((root, from)) = crate::salvage::parse_site(site_text) else {
        return Err(CmdError::msg(crate::salvage::NO_SUCH_WRECK));
    };
    let Some(to) = ctx.args.rhs().and_then(|rhs| parse_endpoint(rhs.trim())) else {
        return Err(CmdError::msg(
            "Name the endpoint this is recovered into: @salvage/recover \
             <wreck>/<endpoint>/<lot>=<endpoint>",
        ));
    };
    // Existence-hiding, through the SAME view the offer gates on: an actor who may not
    // take from that wreck endpoint reads exactly what an actor naming a wreck that
    // never existed reads, and a captor reads it too — before and after the capture,
    // because capture does not change `may_take_from_vault`'s answer for anybody.
    let permitted = salvage_source(ctx).is_some_and(|source| {
        crate::salvage::sites(ctx.w, source.as_ref(), ctx.actor)
            .0
            .contains(&(root, from))
    });
    if !permitted {
        return Err(CmdError::msg(crate::salvage::NO_SUCH_WRECK));
    }
    if !crate::provider::may_take(ctx.w, ctx.actor, to) || to == from {
        return Err(CmdError::msg(crate::salvage::NO_SUCH_WRECK));
    }
    let Some(action) = crate::salvage::action_ref(ctx.reg) else {
        return Err(CmdError::msg(crate::salvage::NO_SUCH_WRECK));
    };
    let at = ctx.w.world_ic_now().at;
    let attempt = ctx
        .w
        .module_state_mut::<InventoryStore>()
        .map_or(0, |store| store.next_attempt(at));
    ctx.submit_authoritative(
        action,
        PerceivedRef::SelfActor,
        Vec::new(),
        json!({
            "site": crate::salvage::site_token(root, from),
            "to": to.vault().0.to_string(),
            "lot": lot.to_string(),
        }),
        key(&format!("inv.salvage.{}.{at:x}.{attempt:x}", root.0))?,
    );
    Ok(())
}

fn dispatch(ctx: &mut Ctx<'_>, families: &InventoryFamilies) -> Result<(), CmdError> {
    // E7c2 fold B4: NO per-command `store::reconcile`. Rebuilding three whole
    // typed families on the world single-writer for every read switch cost
    // ~12 k record visits per command and grew with the tree's pool count
    // rather than with the caller's. 08i answer 15 grants typed reads in ticks
    // and boot reconciliation, which is what the 60 s sweep and
    // `on_world_loaded` are; the mirror's own contract already tolerates them.
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
    if ctx.switches.has("quarantine") {
        return quarantine(ctx);
    }
    if ctx.switches.has("move") {
        return move_stock(ctx);
    }
    if ctx.switches.has("split") {
        return split(ctx, families);
    }
    if ctx.switches.has("merge") {
        return merge(ctx, families);
    }
    if ctx.switches.has("dissolve") {
        return dissolve(ctx, families);
    }
    if ctx.switches.has("crystallize") {
        return crystallize(ctx);
    }
    Err(CmdError::msg(
        "Use @inv/show, /lots, /lot, /move, /split, /merge, /dissolve, /crystallize or /reservations.",
    ))
}

/// The staff surface half of the quarantine (E7c2 fold B14).
///
/// `orphaned_pool_keys` — a pool scalar that survived without its book — can be
/// marked only **by key**, because no endpoint is recoverable from a scalar
/// alone. `@inv/show` is keyed by endpoint, so that half of the quarantine had
/// no operator surface at all: nothing named it, and §4.0's staff ladder forbids
/// parking state on "wait for staff" without the order filled in.
fn quarantine(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if !mush_core::perm::has_role(ctx.w, ctx.actor, mush_core::Role::Royalty) {
        // A non-staff actor sees exactly what a staffer with nothing quarantined
        // would: the switch is not theirs, and it says nothing about the world.
        return Err(CmdError::msg(
            "Use @inv/show, /lots, /lot or /reservations.",
        ));
    }
    let Some(store) = ctx.w.module_state::<InventoryStore>() else {
        return Err(CmdError::msg("Inventory is not available on this server."));
    };
    let mut lines = vec!["Inventory quarantine:".to_owned()];
    for (endpoint, why) in store.quarantined() {
        lines.push(format!("  {endpoint} — {}", why.runbook()));
    }
    for key in store.orphaned_keys() {
        lines.push(format!(
            "  pool key {key} — {}",
            crate::store::Quarantine::HalfPair.runbook()
        ));
    }
    if !store.is_complete() {
        lines.push(
            "  the applied-marker mirror is truncated; every inventory fact answers \
             unsatisfied until the family is pruned"
                .to_owned(),
        );
    }
    if lines.len() == 1 {
        lines.push("  (nothing quarantined)".to_owned());
    }
    for line in lines {
        ctx.out.info(ctx.actor, line);
    }
    Ok(())
}

// ---- the four reads ----

fn show(ctx: &mut Ctx<'_>, families: &InventoryFamilies) -> Result<(), CmdError> {
    let (head, cursor) = split_cursor(ctx.args.raw())?;
    let Some(endpoint) = parse_endpoint(&head) else {
        return Err(CmdError::msg(NO_SUCH_ENDPOINT));
    };
    // ONE expression decides both refusals, so they cannot drift apart.
    if !endpoint_visible(ctx.w, ctx.actor, endpoint) {
        return Err(CmdError::msg(NO_SUCH_ENDPOINT));
    }
    let quarantine = ctx
        .w
        .module_state::<InventoryStore>()
        .and_then(|store| store.quarantine_reason(endpoint));
    let status = endpoint_status(ctx.w, endpoint);
    let mut lines = vec![format!("{endpoint} — {status}")];
    if let Some(why) = quarantine {
        lines.push(format!("  QUARANTINED: {}", why.runbook()));
    }
    let listing = walk(ctx, &families.books, cursor, |book| {
        if crate::store::endpoint_of(book.resource().subject.clone()) != Some(endpoint) {
            return Vec::new();
        }
        let holds = book
            .records()
            .iter()
            .filter(|record| record.state() == mush_core::ReservationState::Held)
            .count();
        vec![format!(
            "  {} — {holds} held reservation(s)",
            book.resource().id.as_str()
        )]
    })?;
    let empty = listing.rows.is_empty();
    let listing_meta = Listing {
        rows: Vec::new(),
        next: listing.next.clone(),
        visited: listing.visited,
    };
    lines.extend(listing.rows);
    if empty {
        lines.push("  (nothing pooled here)".to_owned());
    }
    if let Some(next) = listing_meta.next.clone() {
        lines.push(continuation(
            "show",
            &format!("{}", endpoint.vault().0),
            &listing_meta,
            &next,
        ));
    }
    for line in lines {
        ctx.out.info(ctx.actor, line);
    }
    Ok(())
}

/// The honest continuation line. It names the exact follow-up, because the
/// advice it replaced — *"narrow the commodity to see more"* — was false: the
/// commodity filter ran after the single page too, so narrowing surfaced fewer
/// rows, never more.
fn continuation(switch: &str, subject: &str, listing: &Listing, cursor: &str) -> String {
    format!(
        "  … {} records visited and more remain; continue with @inv/{switch} {subject} after={cursor}",
        listing.visited
    )
}

fn lots(ctx: &mut Ctx<'_>, families: &InventoryFamilies) -> Result<(), CmdError> {
    let (head, cursor) = split_cursor(ctx.args.raw())?;
    let (endpoint_text, filter) = head.split_once(' ').unwrap_or((head.as_str(), ""));
    let Some(endpoint) = parse_endpoint(endpoint_text) else {
        return Err(CmdError::msg(NO_SUCH_ENDPOINT));
    };
    if !endpoint_visible(ctx.w, ctx.actor, endpoint) {
        return Err(CmdError::msg(NO_SUCH_ENDPOINT));
    }
    let filter = filter.trim().to_owned();
    let listing = walk(ctx, &families.lots, cursor, |lot| {
        if lot.endpoint() != endpoint {
            return Vec::new();
        }
        if !filter.is_empty() && lot.commodity().as_str() != filter {
            return Vec::new();
        }
        vec![format!(
            "  {} — {} {} (grade {})",
            lot.id(),
            lot.quantity(),
            lot.commodity().as_str(),
            lot.grade().get()
        )]
    })?;
    let meta = Listing {
        rows: Vec::new(),
        next: listing.next.clone(),
        visited: listing.visited,
    };
    ctx.out.info(ctx.actor, format!("Lots at {endpoint}:"));
    if listing.rows.is_empty() {
        ctx.out.info(ctx.actor, "  (none)");
    }
    for row in listing.rows {
        ctx.out.info(ctx.actor, row);
    }
    if let Some(next) = meta.next.clone() {
        let subject = if filter.is_empty() {
            format!("{}", endpoint.vault().0)
        } else {
            format!("{} {filter}", endpoint.vault().0)
        };
        ctx.out
            .info(ctx.actor, continuation("lots", &subject, &meta, &next));
    }
    Ok(())
}

/// A lineage source id as `@inv/lot` prints it: its first eight characters at most (doc 17h
/// cut M8b-2 fix round 1, L1: never a full operation hex on this surface).
fn short_id(id: &str) -> &str {
    id.char_indices().nth(8).map_or(id, |(end, _)| &id[..end])
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
        // Doc 17h cut M8b-2 fix round 1 (L1): the source id is printed to its first eight
        // characters at most, so no full operation hex reaches this surface.
        crate::record::Lineage::Origin { source } => ctx.out.info(
            ctx.actor,
            format!(
                "  origin: {}/{}/{}",
                source.provider,
                source.kind,
                short_id(&source.id)
            ),
        ),
        // Doc 17h cut M8b-2 (H1, H5): the heirloom and grey arms' own lines are staff's
        // (ROYALTY, this module's staff tier). Any other viewer of the vault sees a heirloom
        // on the origin line's existing shape and no lineage line for a grey lot, so no new
        // sentence reaches a player.
        crate::record::Lineage::Heirloom { source } => {
            let word = if mush_core::perm::has_role(ctx.w, ctx.actor, mush_core::Role::Royalty) {
                "heirloom"
            } else {
                "origin"
            };
            ctx.out.info(
                ctx.actor,
                format!(
                    "  {word}: {}/{}/{}",
                    source.provider,
                    source.kind,
                    short_id(&source.id)
                ),
            );
        }
        crate::record::Lineage::Grey {} => {
            if mush_core::perm::has_role(ctx.w, ctx.actor, mush_core::Role::Royalty) {
                ctx.out.info(ctx.actor, "  grey: no origin recorded");
            }
        }
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
    let (head, cursor) = split_cursor(ctx.args.raw())?;
    let Some(endpoint) = parse_endpoint(&head) else {
        return Err(CmdError::msg(NO_SUCH_ENDPOINT));
    };
    if !endpoint_visible(ctx.w, ctx.actor, endpoint) {
        return Err(CmdError::msg(NO_SUCH_ENDPOINT));
    }
    // One internal view feeds the render (§4 rule 4): the walk is turned into
    // owned rows before a single line is queued, so the projection and any later
    // DTO come from the same bounded walk.
    let listing = walk(ctx, &families.books, cursor, |book| {
        if crate::store::endpoint_of(book.resource().subject.clone()) != Some(endpoint) {
            return Vec::new();
        }
        book.records()
            .iter()
            .map(|record| {
                format!(
                    "  {} {} × {} — {:?}",
                    book.resource().id.as_str(),
                    record.amount(),
                    record.id().0.to_hex(),
                    record.state()
                )
            })
            .collect()
    })?;
    let meta = Listing {
        rows: Vec::new(),
        next: listing.next.clone(),
        visited: listing.visited,
    };
    ctx.out
        .info(ctx.actor, format!("Reservations at {endpoint}:"));
    if listing.rows.is_empty() {
        ctx.out.info(ctx.actor, "  (none)");
    }
    for row in listing.rows {
        ctx.out.info(ctx.actor, row);
    }
    if let Some(next) = meta.next.clone() {
        // It used to print no continuation marker at all and silently truncate.
        ctx.out.info(
            ctx.actor,
            continuation(
                "reservations",
                &format!("{}", endpoint.vault().0),
                &meta,
                &next,
            ),
        );
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

fn split(ctx: &mut Ctx<'_>, families: &InventoryFamilies) -> Result<(), CmdError> {
    let lot = ctx.args.lhs().trim().to_owned();
    let quantity = ctx
        .args
        .rhs()
        .unwrap_or_default()
        .trim()
        .parse::<u64>()
        .map_err(|_| CmdError::msg("Usage: @inv/split <lot>=<n>"))?;
    let id = LotId::parse(&lot).ok_or_else(|| CmdError::msg(NO_SUCH_LOT))?;
    let endpoint = lot_endpoint_text(ctx, families, id)?;
    submit(
        ctx,
        InventoryVerb::Split,
        json!({ "endpoint": endpoint, "lot": id.to_string(), "quantity": quantity }),
        "inv.split",
    )
}

fn merge(ctx: &mut Ctx<'_>, families: &InventoryFamilies) -> Result<(), CmdError> {
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
    let endpoint = lot_endpoint_text(ctx, families, ids[0])?;
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

fn dissolve(ctx: &mut Ctx<'_>, families: &InventoryFamilies) -> Result<(), CmdError> {
    let id = LotId::parse(ctx.args.raw()).ok_or_else(|| CmdError::msg(NO_SUCH_LOT))?;
    let endpoint = lot_endpoint_text(ctx, families, id)?;
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
    // E7c2 fold B13: an *omitted* grade still defaults to 0 ("slag is still
    // matter"), but a malformed one refuses instead of being silently coerced.
    // Grade is durable and **unmergeable** — `pool::merge` refuses
    // `MergeMismatch` across grades — so a silent substitution there is a
    // permanent value change the player never asked for, and the quantity beside
    // it already parses strictly.
    let grade = grade
        .trim()
        .parse::<u64>()
        .map_err(|_| CmdError::msg("A grade is 0 to 100."))?;
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

/// The endpoint text a lot-shaped action's `endpoint` choice must carry: **the
/// lot's own**, when it is one the actor may take from.
///
/// E7c2 fold C1. It used to return `live_endpoints(..).first()` — an arbitrary
/// endpoint *of the actor's*, never the lot's — which is what made the `Choice`
/// list decorative for `/split`, `/merge` and `/dissolve`: the offer authorised
/// endpoint A while the mechanic went on to restructure or destroy a lot at
/// endpoint B, anywhere in the world, by id.
///
/// It adds **no oracle**: a lot that does not exist, a lot at an endpoint the
/// actor may not take from, and a lot the actor may not see all answer exactly
/// [`NO_SUCH_LOT`], the same bytes, through this one expression.
fn lot_endpoint_text(
    ctx: &Ctx<'_>,
    families: &InventoryFamilies,
    lot: LotId,
) -> Result<String, CmdError> {
    let Ok(Some(record)) = ctx.action_state(&families.lots, &lot) else {
        return Err(CmdError::msg(NO_SUCH_LOT));
    };
    let endpoint = record.endpoint();
    if !crate::provider::live_endpoints(ctx.w, ctx.actor).contains(&endpoint.vault()) {
        return Err(CmdError::msg(NO_SUCH_LOT));
    }
    Ok(endpoint.vault().0.to_string())
}
