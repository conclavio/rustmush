//! **The `@tacsim` surface** (B4, B6; §3.1's command list).
//!
//! One primary verb, entered in the pinned rule-1 allowlist in the same commit
//! (08i §4). Three switches: `pack`, `seed`, `pin`.
//!
//! # Disclosure
//!
//! *A rules pack is public; a campaign-seed pack is staff's.* A non-staff viewer
//! asking about a seed pack gets [`NO_SUCH_PACK`] — **byte-identical** to the answer
//! for an id that was never published, at every switch — so `@tacsim/pack show` is
//! not a pack-existence oracle (§3.1's review item, 08c §6.2 D1). Every refusal in
//! this module is **one message** through the ordinary error renderer; none of them
//! branches on how much the viewer already knows.
//!
//! # Bounds
//!
//! `@tacsim/pack list` renders at most [`crate::store::PACK_PAGE`] rows and the page
//! is taken on records **visited**, not matched (D-LBK-5), so a viewer who may see
//! none of them still pays exactly one page of work.

use std::sync::Arc;

use mush_core::module::{ModError, Registrar};
use mush_core::{ArgShape, CanonicalId, CmdError, CommandSpec, Ctx, ObjId, World};
use smol_str::SmolStr;

use crate::pack::{PackEntry, PackKind};
use crate::record::PinRecord;
use crate::seed::{self, SeedError};
use crate::store::{PACK_PAGE, TacsimStore, persist_meta, put_pin};

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
                   @tacsim/pack show <id>[@<version>]  one pack: its digest, its dependencies \
                   and its entries. A campaign-seed pack is staff-only and answers \"No such \
                   pack.\" to everyone else — the same answer an unknown id gets.\n\
                   @tacsim/pack verify [<id>[@<version>]]\n\
                   \x20                                 re-verify the loaded bytes' digests. It \
                   re-reads nothing: a pack whose declared digest did not match its content was \
                   refused at boot, naming the file.\n\
                   @tacsim/seed <pack> [dry|commit]    (staff) apply a campaign-seed pack ONCE. \
                   `dry` writes nothing and lists every planned object; `commit` applies it, and \
                   a second commit is a HIT that writes nothing.\n\
                   @tacsim/pin <subject>=<pack>@<version>\n\
                   \x20                                 (staff) pin one subject to one exact pack \
                   version, so publishing a newer version cannot move it."
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
        "show" => pack_show(ctx, words.next().unwrap_or_default()),
        "verify" => pack_verify(ctx, words.next().unwrap_or_default()),
        _ => Err(CmdError::msg(
            "@tacsim/pack list | show <id>[@<version>] | verify [<id>[@<version>]]",
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
    // Bounded on records VISITED, not matched: the loop stops after PACK_PAGE
    // candidates whether or not this viewer may see them.
    for (visited, loaded) in store.packs().enumerate() {
        if visited >= PACK_PAGE {
            rows.push(format!(
                "(more than {PACK_PAGE} packs are loaded; name one with @tacsim/pack show)"
            ));
            break;
        }
        let manifest = loaded.pack.manifest();
        if !may_view(manifest.kind(), staff) {
            continue;
        }
        rows.push(format!(
            "{}@{} [{}] {} entries, digest {}",
            manifest.id(),
            manifest.version(),
            manifest.kind().label(),
            loaded.pack.entries().len(),
            &manifest.digest().to_hex()[..12]
        ));
    }
    if rows.is_empty() {
        rows.push("No packs are loaded.".to_string());
    }
    lines(ctx, rows);
    Ok(())
}

fn pack_show(ctx: &mut Ctx<'_>, arg: &str) -> Result<(), CmdError> {
    let staff = is_staff(ctx.w, ctx.actor);
    let Some((id, version)) = parse_ref(arg) else {
        return Err(CmdError::msg(NO_SUCH_PACK));
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
    for (entry_id, entry) in loaded.pack.entries() {
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
    lines(ctx, rows);
    Ok(())
}

fn pack_verify(ctx: &mut Ctx<'_>, arg: &str) -> Result<(), CmdError> {
    let staff = is_staff(ctx.w, ctx.actor);
    let wanted = (!arg.trim().is_empty()).then(|| parse_ref(arg)).flatten();
    if !arg.trim().is_empty() && wanted.is_none() {
        return Err(CmdError::msg(NO_SUCH_PACK));
    }
    let store = store(ctx)?;
    let mut rows: Vec<String> = Vec::new();
    let mut matched = false;
    for loaded in store.packs() {
        let manifest = loaded.pack.manifest();
        if let Some((id, version)) = &wanted {
            if manifest.id() != id {
                continue;
            }
            if version.is_some_and(|v| v != manifest.version()) {
                continue;
            }
        }
        if !may_view(manifest.kind(), staff) {
            continue;
        }
        matched = true;
        // Re-verify the LOADED bytes' digest. It re-reads nothing: a file whose
        // declared digest did not match its content was refused at boot, naming the
        // file, so this is a check on what is actually in memory.
        let outcome = match loaded.pack.computed_digest() {
            Ok(computed) if computed == manifest.digest() => "ok".to_string(),
            Ok(computed) => format!("MISMATCH (content hashes to {})", computed.to_hex()),
            Err(error) => format!("UNVERIFIABLE ({error})"),
        };
        rows.push(format!(
            "{}@{} {outcome}",
            manifest.id(),
            manifest.version()
        ));
    }
    if !matched {
        return Err(CmdError::msg(NO_SUCH_PACK));
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

fn pin_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    require_staff(ctx)?;
    require_enabled(ctx)?;
    let subject = ctx.args.lhs().trim().to_string();
    let target = ctx.args.rhs().unwrap_or_default().trim().to_string();
    if subject.is_empty() || target.is_empty() {
        return Err(CmdError::msg("@tacsim/pin <subject>=<pack>@<version>"));
    }
    let Ok(subject) = CanonicalId::parse_canonical(&subject) else {
        return Err(CmdError::msg("@tacsim/pin <subject>=<pack>@<version>"));
    };
    let Some((pack, Some(version))) = parse_ref(&target) else {
        return Err(CmdError::msg(
            "A pin names one exact version: @tacsim/pin <subject>=<pack>@<version>",
        ));
    };
    // A pin to a version this server does not load would be a pin to nothing, which
    // is worse than no pin: resolution would refuse for ever with no way to tell
    // whether the operator meant it. Refuse at the command instead.
    if store(ctx)?.pack(&pack, version).is_none() {
        return Err(CmdError::msg(NO_SUCH_PACK));
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
