//! E7 cut 7 B4/B7 — the player- and staff-facing surface, and the **one
//! internal view** every projection is built from (§4 rule 4).
//!
//! ## The knowledge rule, stated once
//!
//! A contact is **one actor's knowledge, never truth**. Everything a viewer can
//! see here is derived from [`render_contact`] and [`render_envelope`], which
//! read the leak-safe `ContactLedgerState` and this crate's own books — never
//! `ContactState::truth_subject`, which this crate cannot reach at all. A
//! `False` (phantom) contact renders through **exactly** the same code path,
//! the same allocations and the same number of book reads as a real one, so
//! neither the text nor the timing is an oracle.
//!
//! Two refusals a viewer may not distinguish are **byte-identical**: an
//! envelope on a subject the actor may not view and one that does not exist both
//! render [`NO_SUCH_ENVELOPE`], and a contact id nobody holds and one held by
//! somebody else both render [`NO_SUCH_CONTACT`].

use std::sync::Arc;

use mush_core::{
    ArgShape, CanonicalId, ClockStamp, CmdError, CommandSpec, ContactId, ContactLedgerRow,
    ContactLedgerState, Ctx, IdempotencyKeyText, ModError, ModuleOp, ModuleWrite, ObjId, Powers,
    PrincipalId, Registrar, Role, match_object, output::display_name, perm,
};
use smol_str::SmolStr;

use crate::NS;
use crate::pass::{DeliveryReport, active_order, delivery_pass};
use crate::record::{
    ALLOCATOR_KEY, ContactProvenance, Envelope, EnvelopeState, FalseCause, Phantom,
    PropagationStore, Provenance, envelope_key, phantom_key, provenance_key, sighting_key,
};

/// The one line an unknown, unheld or unviewable contact renders (rule 4).
pub const NO_SUCH_CONTACT: &str = "No such contact.";
/// The one line an unknown or unviewable envelope renders (rule 4).
pub const NO_SUCH_ENVELOPE: &str = "No such traffic.";
/// The line every propagation verb answers with when the layer is unwired.
pub const UNAVAILABLE: &str = "Propagation is not available on this server.";

fn info(ctx: &mut Ctx<'_>, msg: impl Into<String>) {
    let actor = ctx.actor;
    ctx.out.sys_to_actor(actor, NS, msg.into());
}

fn is_admin(ctx: &Ctx<'_>) -> bool {
    perm::has_power(ctx.w, ctx.actor, Powers::SIM_ADMIN)
        || perm::has_role(ctx.w, ctx.actor, Role::Royalty)
}

fn store<'a>(ctx: &'a Ctx<'_>) -> Option<&'a PropagationStore> {
    ctx.w.module_state::<PropagationStore>()
}

fn ledger<'a>(ctx: &'a Ctx<'_>) -> Option<&'a ContactLedgerState> {
    ctx.w.module_state::<ContactLedgerState>()
}

fn now(ctx: &Ctx<'_>) -> ClockStamp {
    ctx.w.world_ic_now()
}

fn rhs(ctx: &Ctx<'_>, usage: &str) -> Result<String, CmdError> {
    ctx.args
        .rhs()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| CmdError::msg(format!("Usage: {usage}")))
}

/// **The one internal view.** Every contact projection is this function's
/// output — the list, the detail and any later panel — so a leak has one place
/// to be, and it is not there.
///
/// It renders the **opaque id**, the counters the holder already has a
/// `PerceivedRef` for, the age of the observation, and the provenance side
/// projection. It does **not** render a subject, an object id or a name: a real
/// unit and a phantom are byte-identical apart from the id itself.
#[must_use]
pub fn render_contact(
    row: &ContactLedgerRow,
    provenance: Option<&ContactProvenance>,
    at: &ClockStamp,
) -> String {
    let age = if row.observed_at.clock == at.clock {
        at.at.saturating_sub(row.observed_at.at).max(0)
    } else {
        0
    };
    let state = if row.active { "held" } else { "lost" };
    let source = match provenance.map(ContactProvenance::provenance) {
        Some(Provenance::Reported { source, age, via }) => {
            format!("reported by #{} {age}s old via {via}", source.0)
        }
        Some(Provenance::Local) => "local".to_owned(),
        None => "unattributed".to_owned(),
    };
    let band = provenance.map_or("-", |row| row.band().as_str());
    format!(
        "{}  gen {} rev {}  {state}  {age}s  {band}  {source}",
        &row.contact.to_hex()[..12],
        row.generation,
        row.revision
    )
}

/// The one envelope view. Renders state, revision, hop count and age; the
/// payload is rendered **only to the issuer and the recipient**, because an
/// order's text is the order.
#[must_use]
pub fn render_envelope(envelope: &Envelope, at: &ClockStamp, party: bool) -> String {
    let body = if party {
        format!("  \"{}\"", envelope.payload())
    } else {
        String::new()
    };
    format!(
        "#{} rev {} {:?} hops {}/{} age {}s{body}",
        envelope.id(),
        envelope.revision(),
        envelope.state(),
        envelope.path().len(),
        crate::record::MAX_ENVELOPE_HOPS,
        envelope.age(at)
    )
}

/// Every principal whose picture this actor may read.
///
/// **Q16A, exactly**: a contact belongs to a platform or a station, so a crew
/// member reads their own platform's picture and nobody else's. The roster is
/// the actor itself plus every unit it holds a control station on (cut 5's
/// `assignments_by_actor`, re-used rather than re-derived). There is no argument
/// anywhere that widens it, and no side or world arm at all.
fn viewable_holders(ctx: &Ctx<'_>, actor: ObjId) -> Vec<PrincipalId> {
    let mut holders = vec![PrincipalId::Object { id: actor }];
    if let Some(state) = ctx.w.module_state::<mush_sim::SpatialState>()
        && let Some(units) = state.derived.assignments_by_actor.get(&actor)
    {
        holders.extend(
            units
                .iter()
                .take(mush_core::MAX_CONTACTS_PER_OBSERVER)
                .map(|unit| PrincipalId::Object { id: *unit }),
        );
    }
    holders.sort();
    holders.dedup();
    holders
}

fn holder_rows<'a>(
    ledger: &'a ContactLedgerState,
    holders: &[PrincipalId],
) -> Vec<&'a ContactLedgerRow> {
    ledger
        .rows()
        .iter()
        .filter(|row| holders.contains(&row.holder))
        .take(mush_core::MAX_CONTACTS_PER_OBSERVER)
        .collect()
}

/// `+contacts` — the contacts this actor's own platform holds.
///
/// **One observer's picture is invisible to another**, on the same side or not:
/// the filter is the holder principal, and there is no argument that widens it.
fn cmd_contacts(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let (Some(ledger), Some(store)) = (ledger(ctx), store(ctx)) else {
        info(ctx, UNAVAILABLE);
        return Ok(());
    };
    let at = now(ctx);
    let holders = viewable_holders(ctx, ctx.actor);
    let rows = holder_rows(ledger, &holders);
    let lines: Vec<String> = rows
        .iter()
        .map(|row| render_contact(row, store.provenance_of(row.contact), &at))
        .collect();
    if lines.is_empty() {
        info(ctx, "You hold no contacts.");
        return Ok(());
    }
    info(ctx, format!("{} contact(s):", lines.len()));
    for line in lines {
        info(ctx, line);
    }
    Ok(())
}

/// `+contact <id>` — one contact, by the opaque id prefix the list printed.
///
/// A contact somebody else holds and a contact that does not exist are the
/// **same refusal**, byte for byte.
fn cmd_contact(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let wanted = ctx.args.raw().trim().to_lowercase();
    let (Some(ledger), Some(store)) = (ledger(ctx), store(ctx)) else {
        info(ctx, NO_SUCH_CONTACT);
        return Ok(());
    };
    let at = now(ctx);
    let holders = viewable_holders(ctx, ctx.actor);
    let found = holder_rows(ledger, &holders)
        .into_iter()
        .find(|row| row.contact.to_hex().starts_with(&wanted) && !wanted.is_empty())
        .map(|row| render_contact(row, store.provenance_of(row.contact), &at));
    match found {
        Some(line) => info(ctx, line),
        None => info(ctx, NO_SUCH_CONTACT),
    }
    Ok(())
}

fn next_revision(store: &PropagationStore, issuer: ObjId, recipient: ObjId) -> u32 {
    store
        .envelopes()
        .filter(|envelope| {
            envelope.issuer() == issuer
                && envelope.recipient() == recipient
                && envelope.ack_of().is_none()
        })
        .map(Envelope::revision)
        .max()
        .unwrap_or(0)
        .saturating_add(1)
}

fn persist_envelope(ctx: &mut Ctx<'_>, envelope: &Envelope) {
    let Ok(value) = serde_json::to_value(envelope) else {
        // A record that will not serialize stops only itself and pages with the
        // command a person would run (08c §4.6).
        info(
            ctx,
            format!(
                "Envelope {} could not be written; run @prop/envelopes #{} and re-issue it.",
                envelope.id(),
                envelope.recipient().0
            ),
        );
        return;
    };
    ctx.persist(ModuleOp {
        ns: SmolStr::new_static(NS),
        op: ModuleWrite::JsonSet {
            key: envelope_key(envelope.id()),
            value,
        },
    });
}

fn persist_allocator(ctx: &mut Ctx<'_>) {
    let Some(value) = store(ctx).map(PropagationStore::allocator) else {
        return;
    };
    ctx.persist(ModuleOp {
        ns: SmolStr::new_static(NS),
        op: ModuleWrite::JsonSet {
            key: SmolStr::new_static(ALLOCATOR_KEY),
            value,
        },
    });
}

/// `+order <recipient>=<order>` and `+order/report <recipient>=<contact>`.
///
/// Issuing revision N does **not** make it the recipient's active order: it
/// creates a `Queued` envelope, and only delivery makes it active (T15).
fn cmd_order(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let report_switch = ctx.switches.has("report");
    let recipient = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    if recipient == ctx.actor {
        return Err(CmdError::msg("You cannot send yourself an order."));
    }
    let body = rhs(ctx, "+order <recipient>=<order>")?;
    let at = now(ctx);
    let issuer = ctx.actor;
    let contact = if report_switch {
        let wanted = body.trim().to_lowercase();
        let holders = viewable_holders(ctx, issuer);
        let Some(row) = ledger(ctx).and_then(|ledger| {
            holder_rows(ledger, &holders)
                .into_iter()
                .find(|row| !wanted.is_empty() && row.contact.to_hex().starts_with(&wanted))
                .cloned()
        }) else {
            // A contact the actor does not hold and one that does not exist are
            // the same refusal.
            info(ctx, NO_SUCH_CONTACT);
            return Ok(());
        };
        Some(row.contact)
    } else {
        None
    };
    let Some(slot) = ctx.w.module_state_mut::<PropagationStore>() else {
        info(ctx, UNAVAILABLE);
        return Ok(());
    };
    let revision = next_revision(slot, issuer, recipient);
    let Some(id) = slot.alloc_envelope() else {
        return Err(CmdError::msg("The envelope allocator is exhausted."));
    };
    let envelope = match contact {
        Some(contact) => Envelope::report(id, issuer, recipient, contact, revision, at),
        None => Envelope::issue(id, issuer, recipient, body, revision, at),
    }
    .map_err(|error| CmdError::msg(error.to_string()))?;
    slot.put_envelope(envelope.clone())
        .map_err(|error| CmdError::msg(error.to_string()))?;
    persist_envelope(ctx, &envelope);
    persist_allocator(ctx);
    info(
        ctx,
        format!(
            "Queued revision {revision} to {} as envelope #{id}. It is not their order until it \
             arrives.",
            display_name(ctx.w, recipient)
        ),
    );
    Ok(())
}

/// `+orders` — the traffic this actor issued or received, and `/ack <id>`.
///
/// **The issuer sees `Transmitted` until the acknowledgement itself arrives**,
/// which is the whole point of an ack being a delivery.
fn cmd_orders(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.has("ack") {
        return order_ack(ctx);
    }
    let Some(store) = store(ctx) else {
        info(ctx, UNAVAILABLE);
        return Ok(());
    };
    let at = now(ctx);
    let actor = ctx.actor;
    let mut lines: Vec<String> = Vec::new();
    for envelope in store.envelopes() {
        if envelope.issuer() != actor && envelope.recipient() != actor {
            continue;
        }
        let direction = if envelope.issuer() == actor {
            "->"
        } else {
            "<-"
        };
        lines.push(format!(
            "{direction} {}",
            render_envelope(envelope, &at, true)
        ));
    }
    if let Some(order) = active_order(store, actor) {
        lines.push(format!(
            "Standing order: revision {} — \"{}\"",
            order.revision(),
            order.payload()
        ));
    }
    if lines.is_empty() {
        info(ctx, "No traffic.");
        return Ok(());
    }
    for line in lines {
        info(ctx, line);
    }
    Ok(())
}

fn order_ack(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let wanted: u64 = ctx
        .args
        .raw()
        .trim()
        .trim_start_matches('#')
        .parse()
        .map_err(|_| CmdError::msg("Usage: +orders/ack <envelope id>"))?;
    let at = now(ctx);
    let actor = ctx.actor;
    let Some(read) = store(ctx) else {
        info(ctx, UNAVAILABLE);
        return Ok(());
    };
    let Some(original) = read.envelope(wanted).cloned() else {
        info(ctx, NO_SUCH_ENVELOPE);
        return Ok(());
    };
    let duplicate = read
        .envelopes()
        .find(|envelope| envelope.ack_of() == Some(wanted))
        .map(Envelope::id);
    // An order the actor did not receive, and one that does not exist, are the
    // same refusal.
    if original.recipient() != actor || original.state() != EnvelopeState::Delivered {
        info(ctx, NO_SUCH_ENVELOPE);
        return Ok(());
    }
    // A duplicate ack is a HIT: the existing acknowledgement envelope stands and
    // nothing is written twice.
    if let Some(existing) = duplicate {
        info(
            ctx,
            format!("Already acknowledged; envelope #{existing} carries it."),
        );
        return Ok(());
    }
    let Some(slot) = ctx.w.module_state_mut::<PropagationStore>() else {
        info(ctx, UNAVAILABLE);
        return Ok(());
    };
    let Some(id) = slot.alloc_envelope() else {
        return Err(CmdError::msg("The envelope allocator is exhausted."));
    };
    let ack = Envelope::acknowledge(id, &original, at)
        .map_err(|error| CmdError::msg(error.to_string()))?;
    slot.put_envelope(ack.clone())
        .map_err(|error| CmdError::msg(error.to_string()))?;
    persist_envelope(ctx, &ack);
    persist_allocator(ctx);
    info(
        ctx,
        format!(
            "Acknowledgement queued as envelope #{id}. Your commander learns nothing until it \
             arrives."
        ),
    );
    Ok(())
}

/// `@prop/envelopes <subject>` (read), `@prop/pass` (staff),
/// `@prop/phantom <observer>=<cause>` (staff) and `@prop/provenance` (read).
fn cmd_prop(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    // Every arm is enumerated: an unknown switch REFUSES, it is never silently
    // treated as the read (the fail-open shape this cut is not allowed to have).
    if ctx.switches.has("pass") {
        prop_pass(ctx)
    } else if ctx.switches.has("phantom") {
        prop_phantom(ctx)
    } else if ctx.switches.has("jam") {
        prop_jam(ctx)
    } else if ctx.switches.has("envelopes") || ctx.switches.is_empty() {
        prop_envelopes(ctx)
    } else {
        Err(CmdError::msg(
            "Usage: @prop/envelopes <subject> | @prop/pass | @prop/phantom <observer>=<cause> |              @prop/jam <place>=<level>",
        ))
    }
}

fn prop_envelopes(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    // A subject that does not exist and one the actor may not view are the same
    // refusal, byte for byte — so the resolution failure is folded into the
    // same line rather than raised.
    let subject = match_object(ctx.w, ctx.actor, ctx.args.lhs()).ok();
    let staff = is_admin(ctx);
    let actor = ctx.actor;
    let at = now(ctx);
    let Some(store) = store(ctx) else {
        info(ctx, NO_SUCH_ENVELOPE);
        return Ok(());
    };
    let Some(subject) = subject.filter(|subject| staff || *subject == actor) else {
        info(ctx, NO_SUCH_ENVELOPE);
        return Ok(());
    };
    let lines: Vec<String> = store
        .envelopes()
        .filter(|envelope| envelope.issuer() == subject || envelope.recipient() == subject)
        .map(|envelope| render_envelope(envelope, &at, staff || subject == actor))
        .collect();
    if lines.is_empty() {
        info(ctx, NO_SUCH_ENVELOPE);
        return Ok(());
    }
    for line in lines {
        info(ctx, line);
    }
    Ok(())
}

/// `@prop/pass` — single-step the coarse pass (staff).
///
/// It runs the same [`delivery_pass`] the coarse tick runs and then arms the
/// detection pass's intents through **`mush-tactics`' own ingress**: one
/// `Ctx::submit_authoritative` of the tactics-owned `sensor_arm` action, never a
/// contact write of its own. Propagation writes **no `ContactState`** — it could
/// not, and this is the verb where that is easiest to check.
fn prop_pass(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if !is_admin(ctx) {
        return Err(CmdError::msg("Permission denied."));
    }
    let at = now(ctx);
    let Some(state) = ctx.w.module_state::<mush_sim::SpatialState>().cloned() else {
        info(ctx, UNAVAILABLE);
        return Ok(());
    };
    let ledger_snapshot = ledger(ctx).cloned().unwrap_or_default();
    let Some(mut book) = store(ctx).cloned() else {
        info(ctx, UNAVAILABLE);
        return Ok(());
    };
    let report = delivery_pass(&mut book, &state, ctx.w, &at);
    let intents = crate::pass::detection_pass(&mut book, &state, ctx.w, &ledger_snapshot, &at);
    // Persist the delivery post-images and the sighting book the pass advanced.
    let touched = report.touched();
    let sightings: Vec<crate::record::Sighting> = book.sightings().cloned().collect();
    if let Some(slot) = ctx.w.module_state_mut::<PropagationStore>() {
        *slot = book;
    }
    let envelopes: Vec<Envelope> = touched
        .iter()
        .filter_map(|id| store(ctx).and_then(|book| book.envelope(*id)).cloned())
        .collect();
    for envelope in &envelopes {
        persist_envelope(ctx, envelope);
    }
    for sighting in &sightings {
        if let Ok(value) = serde_json::to_value(sighting) {
            ctx.persist(ModuleOp {
                ns: SmolStr::new_static(NS),
                op: ModuleWrite::JsonSet {
                    key: sighting_key(&sighting.key()),
                    value,
                },
            });
        }
    }
    report_pass(ctx, &report);
    arm_intents(ctx, intents);
    Ok(())
}

fn report_pass(ctx: &mut Ctx<'_>, report: &DeliveryReport) {
    info(
        ctx,
        format!(
            "Delivery pass: {} relayed, {} delivered, {} acknowledged, {} superseded, {} expired, \
             {} waiting on a jammed link ({} visited).",
            report.relayed.len(),
            report.delivered.len(),
            report.acknowledged.len(),
            report.superseded.len(),
            report.expired.len(),
            report.jammed.len(),
            report.visited
        ),
    );
    if report.bound_hit {
        // The bound is PAGED, not logged (E4c V6): the line names the command
        // that finishes the work.
        info(
            ctx,
            format!(
                "The pass stopped at its bound of {} envelopes; run @prop/pass again to continue.",
                crate::pass::MAX_ENVELOPES_PER_PASS
            ),
        );
    }
    for anomaly in &report.anomalies {
        info(ctx, anomaly.clone());
    }
}

/// Arm the tactics-owned ingress. **One submission per command** — calling
/// `submit_authoritative` twice is a refusal by design (E7c2 fold C4), so a pass
/// with more due work than one arm carries says so and staff run it again.
fn arm_intents(ctx: &mut Ctx<'_>, intents: Vec<mush_core::ObservationIntent>) {
    if intents.is_empty() {
        info(ctx, "Detection pass: nothing due.");
        return;
    }
    let armed = intents.len().min(mush_tactics_max_intents_per_arm());
    let remaining = intents.len().saturating_sub(armed);
    let batch: Vec<mush_core::ObservationIntent> = intents.into_iter().take(armed).collect();
    let Some(action) = sensor_arm_action(ctx) else {
        info(
            ctx,
            "The sensor ingress is not composed on this server; nothing was armed.",
        );
        return;
    };
    let Ok(document) = serde_json::to_string(&batch) else {
        info(ctx, "That pass could not be encoded; nothing was armed.");
        return;
    };
    let Ok(key) = IdempotencyKeyText::parse_canonical(format!(
        "prop/arm/{}/{:x}",
        ctx.actor.0,
        ctx.w.world_ic_now().at
    )) else {
        info(ctx, "That pass could not be keyed; nothing was armed.");
        return;
    };
    ctx.submit_authoritative(
        action,
        mush_core::PerceivedRef::SelfActor,
        Vec::new(),
        serde_json::json!({ "intents": document }),
        key,
    );
    info(
        ctx,
        format!(
            "Detection pass: {armed} observation(s) armed through the tactics ingress\
             {}.",
            if remaining > 0 {
                format!(", {remaining} still due — run @prop/pass again")
            } else {
                String::new()
            }
        ),
    );
}

const fn mush_tactics_max_intents_per_arm() -> usize {
    // The tactics ingress's own per-arm bound, restated rather than imported:
    // `mush-propagation` has no `mush-tactics` edge, and a mismatch is caught by
    // the composition pin in `mush-server`.
    16
}

/// Resolve the tactics-owned `sensor_arm` action by its registered identity.
///
/// A **string lookup, not an edge**: the definition registry is keyed by
/// `(owner, kind, id)`, so naming `tactics`' action costs no dependency and
/// gains no authority — the action is still tactics' provider, still writes only
/// tactics' families, and still refuses a non-staff actor at `project_offer`.
fn sensor_arm_action(ctx: &Ctx<'_>) -> Option<mush_core::DefinitionRef> {
    let definitions = ctx.reg.definitions()?;
    definitions
        .current(&mush_core::DefinitionKey {
            owner: mush_core::OwnerId::new("tactics").ok()?,
            kind: mush_core::DefinitionKind::new("action").ok()?,
            id: mush_core::DefinitionId::new("sensor_arm").ok()?,
        })
        .cloned()
}

/// `@prop/jam <place>=<level>` (staff) — declare interference at a place.
///
/// **Delivery-affecting only** (R4): `resolve_detection` never reads the term,
/// so a jammed unit is still seen and still takes the order it already has. The
/// level is world state and is persisted with the allocator document, so a
/// jammed link survives a restart rather than clearing itself.
fn prop_jam(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if !is_admin(ctx) {
        return Err(CmdError::msg("Permission denied."));
    }
    let place = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    let level: u32 = rhs(ctx, "@prop/jam <place>=<level>")?
        .parse()
        .map_err(|_| CmdError::msg("A jamming level is a whole number (0 clears it)."))?;
    let Some(slot) = ctx.w.module_state_mut::<PropagationStore>() else {
        info(ctx, UNAVAILABLE);
        return Ok(());
    };
    slot.set_jamming(place, level);
    persist_allocator(ctx);
    info(
        ctx,
        format!(
            "Interference at {} is now {level}. Orders already delivered still stand.",
            display_name(ctx.w, place)
        ),
    );
    Ok(())
}

/// `@prop/phantom <observer>=<cause>[,<ttl seconds>]` (staff) — mint a false
/// contact's subject.
fn prop_phantom(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if !is_admin(ctx) {
        return Err(CmdError::msg("Permission denied."));
    }
    let observer = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    let spec = rhs(
        ctx,
        "@prop/phantom <observer>=artefact|decoy|spoof[,<ttl secs>]",
    )?;
    let (cause_text, ttl) = match spec.split_once(',') {
        Some((cause, ttl)) => (
            cause.trim().to_owned(),
            ttl.trim().parse::<i64>().unwrap_or(300),
        ),
        None => (spec.trim().to_owned(), 300),
    };
    let cause = match cause_text.as_str() {
        "artefact" => FalseCause::Artefact,
        "decoy" => FalseCause::Decoy,
        "spoof" => FalseCause::Spoof,
        other => {
            return Err(CmdError::msg(format!(
                "`{other}` is not a false-contact cause (artefact|decoy|spoof)."
            )));
        }
    };
    if ttl <= 0 {
        return Err(CmdError::msg("A phantom must expire after it is minted."));
    }
    let at = now(ctx);
    let expires = ClockStamp {
        clock: at.clock.clone(),
        at: at.at.saturating_add(ttl),
    };
    let Some(slot) = ctx.w.module_state_mut::<PropagationStore>() else {
        info(ctx, UNAVAILABLE);
        return Ok(());
    };
    let Some(ordinal) = slot.alloc_phantom() else {
        return Err(CmdError::msg("The phantom allocator is exhausted."));
    };
    let id = IdempotencyKeyText::parse_canonical(format!("{}/{ordinal:x}", observer.0))
        .map_err(|error| CmdError::msg(error.to_string()))?;
    let phantom = Phantom::new(
        id.clone(),
        cause,
        at,
        expires,
        vec![PrincipalId::Object { id: observer }],
    )
    .map_err(|error| CmdError::msg(error.to_string()))?;
    slot.put_phantom(phantom.clone())
        .map_err(|error| CmdError::msg(error.to_string()))?;
    let value = serde_json::to_value(&phantom).map_err(|error| CmdError::msg(error.to_string()))?;
    ctx.persist(ModuleOp {
        ns: SmolStr::new_static(NS),
        op: ModuleWrite::JsonSet {
            key: phantom_key(&id),
            value,
        },
    });
    persist_allocator(ctx);
    info(
        ctx,
        format!(
            "Minted phantom {} for {} ({:?}); it is a scoped subject and resolves to no unit.",
            id.as_str(),
            display_name(ctx.w, observer),
            cause
        ),
    );
    Ok(())
}

/// Record one contact's provenance (the side projection; no lifecycle
/// authority). Exposed for the pass and for tests; there is no player verb that
/// writes it, because nothing may gate on it.
pub fn record_provenance(
    ctx: &mut Ctx<'_>,
    contact: ContactId,
    band: &str,
    provenance: Provenance,
    generation: u64,
) -> Result<(), CmdError> {
    let at = now(ctx);
    let band = CanonicalId::new(band).map_err(|error| CmdError::msg(error.to_string()))?;
    let row = ContactProvenance::new(contact, band, provenance, generation, at)
        .map_err(|error| CmdError::msg(error.to_string()))?;
    let Some(slot) = ctx.w.module_state_mut::<PropagationStore>() else {
        return Err(CmdError::msg(UNAVAILABLE));
    };
    slot.put_provenance(row.clone())
        .map_err(|error| CmdError::msg(error.to_string()))?;
    let value = serde_json::to_value(&row).map_err(|error| CmdError::msg(error.to_string()))?;
    ctx.persist(ModuleOp {
        ns: SmolStr::new_static(NS),
        op: ModuleWrite::JsonSet {
            key: provenance_key(contact),
            value,
        },
    });
    Ok(())
}

/// Register every verb, with `classify_commands` **per group, before the first
/// `reg.command`** (K8.5), and `LegacyPostImage` restored after.
pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    // Group 1 — the reads. Nothing here writes a document or stages an action.
    reg.classify_commands(mush_core::CommandCompatibility::ReadOnly);
    reg.command(
        CommandSpec {
            name: "+contacts".into(),
            shape: ArgShape::Raw,
            help: "+contacts  --  The contacts your platform holds. One observer's picture is \
                   never another's."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_contacts),
    )?;
    reg.command(
        CommandSpec {
            name: "+contact".into(),
            shape: ArgShape::Raw,
            help: "+contact <id>  --  One contact you hold, by the id prefix +contacts printed."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_contact),
    )?;
    // Group 2 — `+orders` writes an acknowledgement envelope on `/ack`, so the
    // spec takes the CEILING class of its switches.
    reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
    reg.command(
        CommandSpec {
            name: "+orders".into(),
            shape: ArgShape::Raw,
            switches: ["ack"].iter().copied().map(SmolStr::new).collect(),
            help: "+orders  --  Traffic you issued or received, and your standing order.\n\
                   +orders/ack <id>  --  Acknowledge a delivered order. The acknowledgement is \
                   itself a delivery: your commander learns nothing until it arrives."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_orders),
    )?;
    reg.command(
        CommandSpec {
            name: "+order".into(),
            shape: ArgShape::EqPair,
            switches: ["report"].iter().copied().map(SmolStr::new).collect(),
            help: "+order <recipient>=<order>  --  Issue an order. Issuing it does not make it \
                   theirs; it becomes their order when it arrives.\n\
                   +order/report <recipient>=<contact id>  --  Send a contact report. The share \
                   happens on delivery, never before."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_order),
    )?;
    reg.command(
        CommandSpec {
            name: "@prop".into(),
            shape: ArgShape::EqPair,
            switches: ["envelopes", "pass", "phantom", "jam"]
                .iter()
                .copied()
                .map(SmolStr::new)
                .collect(),
            help: "@prop/envelopes <subject>  --  Traffic on a subject (yours; staff see all).\n\
                   @prop/pass  --  (staff) Single-step the coarse delivery pass and arm the \
                   detection pass through the tactics sensor ingress.\n\
                   @prop/phantom <observer>=artefact|decoy|spoof[,<ttl>]  --  (staff) Mint a \
                   false contact's scoped subject."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_prop),
    )?;
    // K8.5: restore the module default after the last group.
    reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
    Ok(())
}
