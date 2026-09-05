//! **The `@tacsim` surface** (B4, B6; §3.1's command list).
//!
//! One primary verb, entered in the pinned rule-1 allowlist in the same commit
//! (08i §4). Three switches: `pack`, `seed`, `pin`.
//!
//! # Disclosure
//!
//! *A rules pack is public; a campaign-seed pack is staff's.* A non-staff viewer
//! asking about a seed pack gets [`NO_SUCH_PACK`] — **byte-identical** to the answer
//! for an id that was never published — so `@tacsim/pack show` is not a pack-existence
//! oracle (§3.1's review item, 08c §6.2 D1). Every refusal in this module is **one
//! message** through the ordinary error renderer.
//!
//! **`list` is the switch that needed the care** (independent verify B2, correcting
//! this block's earlier "at every switch … none of them branches on how much the viewer
//! already knows"). `list` renders *rows*, not a refusal, so hiding is not enough: the
//! page marker used to be pushed on records **visited**, ahead of the visibility
//! filter, and it was an oracle twice over. A server loading 33 campaign-seed packs and
//! no rules pack answered a non-staff viewer *"more than 32 packs are loaded"* where an
//! empty server answered *"No packs are loaded."*; and because the visit budget is a
//! fixed [`crate::store::PACK_PAGE`], a viewer who saw the marker could subtract the
//! rendered rows from it and learn exactly how many packs inside the window were
//! hidden. The **work** is still bounded on records visited (D-LBK-5 is right); the
//! **marker** is now bounded on records the viewer may *see*, so a full window of
//! invisible packs is byte-identical to an empty server and a rendered row count is
//! indistinguishable from a smaller server's.
//!
//! # Bounds
//!
//! Every switch that can render more than one row is paged at
//! [`crate::store::PACK_PAGE`], and the page is taken on records **visited**, not
//! matched (D-LBK-5), so a viewer who may see none of them still pays exactly one page
//! of work:
//!
//! | switch | bound |
//! |---|---|
//! | `@tacsim/pack list` | `PACK_PAGE` records visited; the marker on records viewable |
//! | `@tacsim/pack show` | three header rows plus `PACK_PAGE` entries, with a cursor to continue (B3 — it used to render all `MAX_PACK_ENTRIES` of them, 515 messages per non-staff command) |
//! | `@tacsim/pack verify` | **staff-only** and `PACK_PAGE` records visited (B1 — it used to be `require_enabled` alone and unpaged, so any player could make the world's single writer canonically re-encode and SHA-256 up to `MAX_CONFIGURED_PACKS` × `MAX_PACK_BYTES` = 32 MiB, synchronously, at command rate) |
//! | `@tacsim/pin list` | `PACK_PAGE` records visited |
//!
//! `mush-core`'s `Outbox` carries no per-command message cap to fall back on, so an
//! unpaged loop here is the whole bound.

use std::sync::Arc;

use mush_core::module::{ModError, Registrar};
use mush_core::{ArgShape, CanonicalId, CmdError, CommandSpec, Ctx, ObjId, World};
use smol_str::SmolStr;

use crate::pack::{PackEntry, PackKind};
use crate::record::PinRecord;
use crate::seed::{self, SeedError};
use crate::store::{MAX_PINS, PACK_PAGE, TacsimStore, del_pin, persist_meta, put_pin};

/// The one existence-hiding refusal. Identical for "never published", "not loaded"
/// and "a seed pack you may not see".
pub const NO_SUCH_PACK: &str = "No such pack.";

/// What an `enabled = false` server answers. The verbs still register (E5c
/// deviation 1): a disabled layer refuses out loud rather than hiding a verb.
pub const NOT_ENABLED: &str = "TacSim is not enabled on this server.";

/// Register the one TacSim verb.
///
/// # Errors
///
/// [`ModError`] if the name is already taken.
pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.command(
        CommandSpec {
            name: "@tacsim".into(),
            shape: ArgShape::EqPair,
            switches: ["pack", "seed", "pin"]
                .iter()
                .copied()
                .map(SmolStr::new)
                .collect(),
            help: "@tacsim/pack list                   every rules pack this server loads.\n\
                   @tacsim/pack show <id>[@<version>] [<from-entry>]\n\
                   \x20                                 one pack: its digest, its dependencies \
                   and a page of its entries, continuing from <from-entry> when the last page \
                   said there were more. A campaign-seed pack is staff-only and answers \"No such \
                   pack.\" to everyone else — the same answer an unknown id gets.\n\
                   @tacsim/pack verify [<id>[@<version>]]\n\
                   \x20                                 (staff) re-verify the loaded bytes' \
                   digests against the ledger row each version was published under. It re-reads \
                   no file: a pack whose declared digest did not match its content was refused at \
                   boot, naming the file.\n\
                   @tacsim/seed <pack> [dry|commit]    (staff) apply a campaign-seed pack ONCE. \
                   `dry` writes nothing and lists every planned object; `commit` applies it, and \
                   a second commit is a HIT that writes nothing.\n\
                   @tacsim/pin <subject>=<pack>@<version>\n\
                   \x20                                 (staff) pin one subject to one exact pack \
                   version, so publishing a newer version cannot move it. The pack must declare \
                   the subject.\n\
                   @tacsim/pin <subject>=              (staff) remove that subject's pin.\n\
                   @tacsim/pin list                    (staff) every pin this world carries."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(tacsim_cmd),
    )?;
    Ok(())
}

fn info(ctx: &mut Ctx<'_>, msg: impl Into<String>) {
    let actor = ctx.actor;
    ctx.out.sys_to_actor(actor, "tacsim", msg.into());
}

fn lines(ctx: &mut Ctx<'_>, rows: Vec<String>) {
    for row in rows {
        info(ctx, row);
    }
}

/// Staff, on `mush-market`'s exact test.
fn is_staff(w: &World, who: ObjId) -> bool {
    mush_core::perm::has_power(w, who, mush_core::Powers::SIM_ADMIN)
        || mush_core::perm::has_role(w, who, mush_core::Role::Royalty)
}

fn require_staff(ctx: &Ctx<'_>) -> Result<(), CmdError> {
    if is_staff(ctx.w, ctx.actor) {
        Ok(())
    } else {
        Err(CmdError::msg("Permission denied."))
    }
}

fn store<'a>(ctx: &'a Ctx<'_>) -> Result<&'a TacsimStore, CmdError> {
    ctx.w
        .module_state::<TacsimStore>()
        .ok_or_else(|| CmdError::msg(NOT_ENABLED))
}

fn require_enabled(ctx: &Ctx<'_>) -> Result<(), CmdError> {
    if store(ctx)?.enabled() {
        Ok(())
    } else {
        Err(CmdError::msg(NOT_ENABLED))
    }
}

/// Parse `<id>` or `<id>@<version>`.
fn parse_ref(text: &str) -> Option<(CanonicalId, Option<u32>)> {
    let text = text.trim();
    match text.rsplit_once('@') {
        Some((id, version)) => {
            let version: u32 = version.trim().parse().ok()?;
            (version > 0)
                .then(|| CanonicalId::parse_canonical(id.trim()).ok())
                .flatten()
                .map(|id| (id, Some(version)))
        }
        None => CanonicalId::parse_canonical(text).ok().map(|id| (id, None)),
    }
}

fn tacsim_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.has("seed") {
        seed_cmd(ctx)
    } else if ctx.switches.has("pin") {
        pin_cmd(ctx)
    } else {
        pack_cmd(ctx)
    }
}

// ---- @tacsim/pack ----------------------------------------------------------

fn pack_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    require_enabled(ctx)?;
    let arg = ctx.args.raw().trim().to_string();
    let mut words = arg.split_whitespace();
    match words.next().unwrap_or("list") {
        "list" => pack_list(ctx),
        "show" => {
            let named = words.next().unwrap_or_default().to_string();
            let cursor = words.next().unwrap_or_default().to_string();
            pack_show(ctx, &named, &cursor)
        }
        "verify" => {
            let named = words.next().unwrap_or_default().to_string();
            pack_verify(ctx, &named)
        }
        _ => Err(CmdError::msg(
            "@tacsim/pack list | show <id>[@<version>] [<from-entry>] | verify [<id>[@<version>]]",
        )),
    }
}

/// Whether this viewer may see this pack at all.
///
/// A rules pack is the rules and is public; a campaign-seed pack names what a world
/// was built out of and is staff's. Nothing else discriminates.
fn may_view(kind: PackKind, staff: bool) -> bool {
    matches!(kind, PackKind::RulesDefinition) || staff
}

fn pack_list(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let staff = is_staff(ctx.w, ctx.actor);
    let store = store(ctx)?;
    let mut rows: Vec<String> = Vec::new();
    // Two different budgets, on purpose (B2). The WORK is bounded on records VISITED,
    // not matched: the loop stops after PACK_PAGE candidates whether or not this viewer
    // may see them (D-LBK-5). The MARKER is bounded on records this viewer may SEE, so
    // it never reports the existence of packs the viewer may not know about — a window
    // full of hidden packs is byte-identical to an empty server.
    let mut viewable = 0_usize;
    for (visited, loaded) in store.packs().enumerate() {
        if visited >= PACK_PAGE {
            break;
        }
        let manifest = loaded.pack.manifest();
        if !may_view(manifest.kind(), staff) {
            continue;
        }
        viewable += 1;
        rows.push(format!(
            "{}@{} [{}] {} entries, digest {}",
            manifest.id(),
            manifest.version(),
            manifest.kind().label(),
            loaded.pack.entries().len(),
            &manifest.digest().to_hex()[..12]
        ));
    }
    // The marker fires on a full page of VIEWABLE rows and on nothing else, so it
    // reports only what this viewer has already been shown.
    if viewable >= PACK_PAGE {
        rows.push(format!(
            "(more than {PACK_PAGE} packs are loaded; name one with @tacsim/pack show)"
        ));
    }
    if rows.is_empty() {
        rows.push("No packs are loaded.".to_string());
    }
    lines(ctx, rows);
    Ok(())
}

/// One pack, and **one page** of its entries (independent verify B3).
///
/// The entry loop used to be unpaged, so `@tacsim/pack show <public rules pack>` from
/// any player emitted three header rows plus one message per entry — 515 output events
/// at [`crate::MAX_PACK_ENTRIES`], on a tree where every other listing surface pages and
/// `mush-core`'s `Outbox` has no per-command message cap to fall back on. `cursor` is an
/// **argument**, not a switch: it names the entry id to resume from, which the trailing
/// row prints for you.
fn pack_show(ctx: &mut Ctx<'_>, arg: &str, cursor: &str) -> Result<(), CmdError> {
    let staff = is_staff(ctx.w, ctx.actor);
    let Some((id, version)) = parse_ref(arg) else {
        return Err(CmdError::msg(NO_SUCH_PACK));
    };
    let cursor = cursor.trim();
    let cursor = if cursor.is_empty() {
        None
    } else {
        match CanonicalId::parse_canonical(cursor) {
            Ok(cursor) => Some(cursor),
            // An unreadable cursor is an unreadable reference, and answers the way
            // every other unreadable reference here does.
            Err(_) => return Err(CmdError::msg(NO_SUCH_PACK)),
        }
    };
    let store = store(ctx)?;
    let version = match version.or_else(|| store.newest_loaded(&id)) {
        Some(version) => version,
        None => return Err(CmdError::msg(NO_SUCH_PACK)),
    };
    let Some(loaded) = store.pack(&id, version) else {
        return Err(CmdError::msg(NO_SUCH_PACK));
    };
    let manifest = loaded.pack.manifest();
    if !may_view(manifest.kind(), staff) {
        // Byte-identical to an id that was never published.
        return Err(CmdError::msg(NO_SUCH_PACK));
    }
    let mut rows = vec![
        format!(
            "{}@{} [{}]",
            manifest.id(),
            manifest.version(),
            manifest.kind().label()
        ),
        format!("digest {}", manifest.digest().to_hex()),
    ];
    if manifest.requires().is_empty() {
        rows.push("requires nothing".to_string());
    } else {
        rows.push(format!(
            "requires {}",
            manifest
                .requires()
                .iter()
                .map(|r| format!("{}@{}", r.id, r.version))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let mut rendered = 0_usize;
    let mut remaining = 0_usize;
    let mut resume: Option<&CanonicalId> = None;
    for (entry_id, entry) in loaded.pack.entries() {
        if cursor.as_ref().is_some_and(|from| entry_id < from) {
            continue;
        }
        if rendered >= PACK_PAGE {
            remaining += 1;
            if resume.is_none() {
                resume = Some(entry_id);
            }
            continue;
        }
        rendered += 1;
        let detail = match entry {
            PackEntry::Commodity(def) => {
                let used = if def.in_use { " (in use)" } else { "" };
                format!("{}{used}", def.label)
            }
            PackEntry::Recipe(def) => format!(
                "{} — {} in, {} out",
                def.label,
                def.inputs.len(),
                def.outputs.len()
            ),
            PackEntry::Location(def) => format!("{} [{}]", def.label, def.role),
            PackEntry::Route(route) => format!(
                "{} → {}, {} legs, drive {}",
                route.from,
                route.to,
                route.legs.len(),
                route.drive_class
            ),
        };
        rows.push(format!("  {entry_id} ({}) {detail}", entry.label()));
    }
    if let Some(resume) = resume {
        rows.push(format!(
            "({remaining} more entries; @tacsim/pack show {}@{} {resume})",
            manifest.id(),
            manifest.version()
        ));
    }
    lines(ctx, rows);
    Ok(())
}

/// Re-hash the loaded packs and compare them with **the ledger**.
///
/// Two independent findings meet here. **B1:** this loop was gated by `require_enabled`
/// alone and had no page bound at all, so one invocation from any player canonically
/// re-encoded and SHA-256'd every loaded pack — up to `MAX_CONFIGURED_PACKS` ×
/// `MAX_PACK_BYTES` = 32 MiB — synchronously on the world's single writer task, at
/// command rate, emitting one message per pack. It is now **staff-only** (the owner's
/// decision of 2026-09-05) and paged on records **visited**, exactly as `pack_list` is.
/// **P1:** it compared `loaded.pack.computed_digest()` with `manifest.digest()`, which
/// is the equality `load_pack` already proved at boot over the same in-memory value
/// that nothing mutates — a tautology that could only ever print `ok`, paying 32 MiB to
/// prove nothing. The comparison that can actually say something is against the
/// **`tacsim:pack:` ledger row**: the digest this world published that version under.
/// `boot_reconcile` refuses a boot where those disagree, so a `MISMATCH` here is the
/// same fact seen from the other side rather than a second policy.
fn pack_verify(ctx: &mut Ctx<'_>, arg: &str) -> Result<(), CmdError> {
    require_staff(ctx)?;
    let staff = is_staff(ctx.w, ctx.actor);
    let wanted = (!arg.trim().is_empty()).then(|| parse_ref(arg)).flatten();
    if !arg.trim().is_empty() && wanted.is_none() {
        return Err(CmdError::msg(NO_SUCH_PACK));
    }
    let store = store(ctx)?;
    let mut rows: Vec<String> = Vec::new();
    let mut more = false;
    // Re-hash the LOADED bytes and compare them with the digest this world PUBLISHED
    // that version under. It re-reads no file: a file whose declared digest did not
    // match its content was refused at boot, naming the file. Without a ledger row
    // there is nothing but the declaration to compare with, and the answer says so
    // rather than pretending to more.
    let verdict = |loaded: &crate::LoadedPack| -> String {
        let manifest = loaded.pack.manifest();
        let published = store.published_digest(manifest.id(), manifest.version());
        let outcome = match loaded.pack.computed_digest() {
            Ok(computed) => match published {
                Some(row) if row == computed => "ok".to_string(),
                Some(row) => format!(
                    "MISMATCH (published as {}, content hashes to {})",
                    row.to_hex(),
                    computed.to_hex()
                ),
                None if computed == manifest.digest() => "ok (not in the ledger)".to_string(),
                None => format!(
                    "MISMATCH (declared {}, content hashes to {})",
                    manifest.digest().to_hex(),
                    computed.to_hex()
                ),
            },
            Err(error) => format!("UNVERIFIABLE ({error})"),
        };
        format!("{}@{} {outcome}", manifest.id(), manifest.version())
    };
    // A named pack is a **lookup**, not a scan: it is exactly one re-hash, and it stays
    // reachable however many packs a server loads.
    if let Some((id, wanted_version)) = wanted {
        let Some(version) = wanted_version.or_else(|| store.newest_loaded(&id)) else {
            return Err(CmdError::msg(NO_SUCH_PACK));
        };
        let Some(loaded) = store.pack(&id, version) else {
            return Err(CmdError::msg(NO_SUCH_PACK));
        };
        if !may_view(loaded.pack.manifest().kind(), staff) {
            return Err(CmdError::msg(NO_SUCH_PACK));
        }
        let row = verdict(loaded);
        lines(ctx, vec![row]);
        return Ok(());
    }
    let mut matched = false;
    for (visited, loaded) in store.packs().enumerate() {
        if visited >= PACK_PAGE {
            more = true;
            break;
        }
        if !may_view(loaded.pack.manifest().kind(), staff) {
            continue;
        }
        matched = true;
        rows.push(verdict(loaded));
    }
    if !matched {
        return Err(CmdError::msg(NO_SUCH_PACK));
    }
    if more {
        rows.push(format!(
            "(more than {PACK_PAGE} packs are loaded; name one with @tacsim/pack verify <id>)"
        ));
    }
    lines(ctx, rows);
    Ok(())
}

// ---- @tacsim/seed ----------------------------------------------------------

fn refuse_seed(error: &SeedError) -> CmdError {
    match error {
        SeedError::NoSuchPack => CmdError::msg(NO_SUCH_PACK),
        other => CmdError::msg(other.to_string()),
    }
}

fn seed_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    require_staff(ctx)?;
    require_enabled(ctx)?;
    let arg = ctx.args.raw().trim().to_string();
    let mut words = arg.split_whitespace();
    let named = words.next().unwrap_or_default().to_string();
    let mode = words.next().unwrap_or("dry").to_lowercase();
    let default_corridor = store(ctx)?.config().corridor.clone();
    let target = if named.is_empty() {
        default_corridor
    } else {
        named
    };
    let Ok(pack) = CanonicalId::parse_canonical(target.trim()) else {
        return Err(CmdError::msg(NO_SUCH_PACK));
    };
    let plan = match seed::plan(store(ctx)?, &pack) {
        Ok(plan) => plan,
        // **The HIT.** This exact `(pack, version)` is already applied, so the honest
        // answer is "done, and nothing was written" — not a refusal. A *different*
        // version is a refusal naming the applied one, and falls through below.
        Err(SeedError::AlreadyApplied { pack, version }) => {
            info(
                ctx,
                format!(
                    "{pack}@{version} has already been applied to this world; nothing was written."
                ),
            );
            return Ok(());
        }
        Err(other) => return Err(refuse_seed(&other)),
    };
    match mode.as_str() {
        "dry" => {
            let mut rows = vec![format!(
                "{}@{} would create {} object(s). Nothing has been written.",
                plan.pack,
                plan.version,
                plan.objects.len()
            )];
            for object in &plan.objects {
                rows.push(format!(
                    "  {} — {} [{}]",
                    object.id, object.label, object.role
                ));
            }
            lines(ctx, rows);
            Ok(())
        }
        "commit" => {
            let owner = ctx.actor;
            let record = seed::apply(ctx, &plan, owner).map_err(|e| refuse_seed(&e))?;
            // The module's one irreversible command, so its confirmation waits on the
            // durable ack the way every other irreversible staff command on this tree
            // does (independent verify S7; `Ctx::gate_on_durable_ack`). Not strict: the
            // objects and the latch are lost together (the one-batch argument), so
            // re-running after a dropped write is correct and the staffer is better off
            // being told which command may not have survived.
            ctx.gate_on_durable_ack();
            info(
                ctx,
                format!(
                    "{}@{} applied: {} object(s) created, and the latch was written in the same \
                     batch. A second commit is a HIT.",
                    record.pack(),
                    record.version(),
                    record.objects()
                ),
            );
            Ok(())
        }
        _ => Err(CmdError::msg("@tacsim/seed <pack> [dry|commit]")),
    }
}

// ---- @tacsim/pin -----------------------------------------------------------

/// The usage line every malformed `@tacsim/pin` answers with.
const PIN_USAGE: &str = "@tacsim/pin <subject>=<pack>@<version> | <subject>= | list";

fn pin_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    require_staff(ctx)?;
    require_enabled(ctx)?;
    let subject = ctx.args.lhs().trim().to_string();
    let target = ctx.args.rhs().map(|rhs| rhs.trim().to_string());
    // `@tacsim/pin list` — the family was uncapped, undeletable and invisible from the
    // game until B4 (`grep ModuleWrite::Del` over the crate found none), so it now has
    // a way to be read and a way to be drained, both paged like every other listing.
    if target.is_none() && subject.eq_ignore_ascii_case("list") {
        return pin_list(ctx);
    }
    let Some(target) = target else {
        return Err(CmdError::msg(PIN_USAGE));
    };
    if subject.is_empty() {
        return Err(CmdError::msg(PIN_USAGE));
    }
    let Ok(subject) = CanonicalId::parse_canonical(&subject) else {
        return Err(CmdError::msg(PIN_USAGE));
    };
    // `@tacsim/pin <subject>=` — the unpin path, the crate's one `ModuleWrite::Del`.
    if target.is_empty() {
        return pin_clear(ctx, &subject);
    }
    let Some((pack, Some(version))) = parse_ref(&target) else {
        return Err(CmdError::msg(
            "A pin names one exact version: @tacsim/pin <subject>=<pack>@<version>",
        ));
    };
    // A pin to a version this server does not load would be a pin to nothing, which
    // is worse than no pin: resolution would refuse for ever with no way to tell
    // whether the operator meant it. Refuse at the command instead.
    //
    // And the pinned pack must **declare the subject** (B4/P4). `resolve` gives a pin
    // absolute precedence, ahead of the `candidates` scan that would otherwise have
    // answered `Unknown`, so a typo'd or stale subject name became a durable pin
    // answering `Ok` for a definition no pack declares — contradicting the store's own
    // "a pin wins and is exact" and handing cut 2 onward a resolvable id with nothing
    // behind it. The refusal reuses the existing existence-hiding message, so no new
    // oracle appears.
    let store = store(ctx)?;
    let Some(loaded) = store.pack(&pack, version) else {
        return Err(CmdError::msg(NO_SUCH_PACK));
    };
    if loaded.pack.entry(&subject).is_none() {
        return Err(CmdError::msg(NO_SUCH_PACK));
    }
    // Bounded like every other document family in the programme (10b §7). Re-pinning a
    // subject that already has a pin replaces it and is never past the cap.
    if store.pin(&subject).is_none() && store.pins().count() >= MAX_PINS {
        return Err(CmdError::msg(format!(
            "This world already carries {MAX_PINS} pins, which is the maximum; remove one with \
             @tacsim/pin <subject>= first."
        )));
    }
    let pinned_at = ctx.world_ic_now();
    let record = match PinRecord::new(subject.clone(), pack.clone(), version, pinned_at) {
        Ok(record) => record,
        Err(error) => return Err(CmdError::msg(error.to_string())),
    };
    put_pin(ctx, record);
    persist_meta(ctx);
    info(
        ctx,
        format!(
            "{subject} is pinned to {pack}@{version}; publishing a newer version will not move it."
        ),
    );
    Ok(())
}

/// Every pin this world carries, paged on records **visited**.
fn pin_list(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let store = store(ctx)?;
    let mut rows: Vec<String> = Vec::new();
    let mut more = false;
    for (visited, pin) in store.pins().enumerate() {
        if visited >= PACK_PAGE {
            more = true;
            break;
        }
        rows.push(format!(
            "{} -> {}@{}",
            pin.subject(),
            pin.pack(),
            pin.version()
        ));
    }
    if rows.is_empty() {
        rows.push("No subjects are pinned.".to_string());
    } else if more {
        rows.push(format!(
            "(more than {PACK_PAGE} subjects are pinned; the cap is {MAX_PINS})"
        ));
    }
    lines(ctx, rows);
    Ok(())
}

/// Remove one subject's pin.
///
/// Answering "there was no such pin" out loud is right here and is not an oracle: this
/// switch is staff-only, and a pin is this world's own bookkeeping rather than a fact
/// about a pack.
fn pin_clear(ctx: &mut Ctx<'_>, subject: &CanonicalId) -> Result<(), CmdError> {
    if store(ctx)?.pin(subject).is_none() {
        return Err(CmdError::msg(format!("{subject} is not pinned.")));
    }
    del_pin(ctx, subject);
    persist_meta(ctx);
    info(
        ctx,
        format!("{subject} is no longer pinned; it resolves by the ordinary rule again."),
    );
    Ok(())
}
