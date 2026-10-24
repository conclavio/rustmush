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
    PrincipalId, Registrar, Role, World, match_object, output::display_name, perm,
};
use smol_str::SmolStr;

use crate::NS;
use crate::pass::{DeliveryReport, delivery_pass_with_metric};
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
///
/// **An incomparable stamp renders `stale`, never `0s`** (fold D5(a)(i)). Before
/// the fold a world time-rate change made every live contact render as observed
/// this second — *fresher than the truth*, on the one player-facing view the
/// whole leak argument rests on.
///
/// **A reported provenance names no dbref** (fold K15). `viewer_source` is the
/// opaque contact id the viewer itself holds on the reporting platform, or
/// `None`; the `ObjId` stays in the record for audit and never in the line.
#[must_use]
pub fn render_contact(
    row: &ContactLedgerRow,
    provenance: Option<&ContactProvenance>,
    at: &ClockStamp,
    viewer_source: Option<ContactId>,
) -> String {
    let c = contact_cells(row, provenance, at, viewer_source);
    format!(
        "{}  gen {} rev {}  {}  {}  {}  {}",
        c.id, c.generation, c.rev, c.state, c.age, c.band, c.source
    )
}

/// The cells of one contact line — what [`render_contact`] formats and what
/// the `tactics.contacts` panel (interface plan I8) ships as a row. One
/// derivation, two renderings: the text and the table cannot disagree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContactCells {
    /// The opaque id's 12-hex prefix — the only identity a viewer ever sees.
    pub id: String,
    /// The contact generation.
    pub generation: u64,
    /// The contact revision.
    pub rev: u64,
    /// `held` or `lost`.
    pub state: &'static str,
    /// `"<n>s"` on the viewer's own clock generation, else `stale` (fold
    /// D5(a)(i): an incomparable stamp never renders fresher than the truth).
    pub age: String,
    /// The provenance band, or `-`.
    pub band: String,
    /// `local`, `unattributed`, or the reported form — never a dbref (fold
    /// K15).
    pub source: String,
}

/// Build the cells of one contact line. See [`render_contact`] for the
/// knowledge rule every cell obeys.
#[must_use]
pub fn contact_cells(
    row: &ContactLedgerRow,
    provenance: Option<&ContactProvenance>,
    at: &ClockStamp,
    viewer_source: Option<ContactId>,
) -> ContactCells {
    let age = if row.observed_at.clock == at.clock {
        format!("{}s", at.at.saturating_sub(row.observed_at.at).max(0))
    } else {
        "stale".to_owned()
    };
    let state = if row.active { "held" } else { "lost" };
    let source = match provenance.map(ContactProvenance::provenance) {
        Some(Provenance::Reported { age, via, .. }) => match viewer_source {
            Some(contact) => format!(
                "reported by {} {age}s old via {via}",
                &contact.to_hex()[..12]
            ),
            None => format!("reported, {age}s old"),
        },
        Some(Provenance::Local) => "local".to_owned(),
        None => "unattributed".to_owned(),
    };
    let band = provenance.map_or("-", |row| row.band().as_str()).to_owned();
    ContactCells {
        id: row.contact.to_hex()[..12].to_owned(),
        generation: row.generation,
        rev: row.revision,
        state,
        age,
        band,
        source,
    }
}

/// The one envelope view. Renders state, revision, hop count, age **and the
/// party it came from**; the payload is rendered **only to the issuer and the
/// recipient**, because an order's text is the order.
///
/// `attribution` is the issuer as the viewer may see them, through the same fog
/// gate `+order` uses — `None` renders "an unidentified sender" rather than a
/// name or a dbref. Before the fold neither this view nor the standing-order
/// line named an issuer at all, so a recipient could not attribute an order to
/// anybody (fold K8).
#[must_use]
pub fn render_envelope(
    envelope: &Envelope,
    at: &ClockStamp,
    party: bool,
    attribution: Option<&str>,
) -> String {
    let body = if party {
        format!("  \"{}\"", envelope.payload())
    } else {
        String::new()
    };
    let age = envelope
        .checked_age(at)
        .map_or_else(|| "stale".to_owned(), |age| format!("{age}s"));
    format!(
        "#{} rev {} {:?} hops {}/{} age {} from {}{body}",
        envelope.id(),
        envelope.revision(),
        envelope.state(),
        envelope.path().len(),
        crate::record::MAX_ENVELOPE_HOPS,
        age,
        attribution.unwrap_or(UNIDENTIFIED_SENDER)
    )
}

fn render_party_envelope(
    envelope: &Envelope,
    viewer: ObjId,
    at: &ClockStamp,
    source: Option<&str>,
) -> String {
    if envelope.issuer() != viewer {
        return render_envelope(envelope, at, true, source);
    }
    // A sender's view contains local dispatch knowledge and a received ACK, not
    // the recipient's arrival state, supersession or remote hop progress.
    let state = match envelope.state() {
        EnvelopeState::Queued => EnvelopeState::Queued,
        EnvelopeState::Acknowledged => EnvelopeState::Acknowledged,
        EnvelopeState::Expired if envelope.age(at) >= crate::record::ENVELOPE_TTL_SECS => {
            EnvelopeState::Expired
        }
        _ => EnvelopeState::Transmitted,
    };
    let age = envelope
        .checked_age(at)
        .map_or_else(|| "stale".into(), |n| format!("{n}s"));
    format!(
        "#{} rev {} {state:?} age {age} from {}  \"{}\"",
        envelope.id(),
        envelope.revision(),
        source.unwrap_or(UNIDENTIFIED_SENDER),
        envelope.payload()
    )
}

/// What an unattributable issuer renders as. Never a name, never a dbref.
pub const UNIDENTIFIED_SENDER: &str = "an unidentified sender";

/// Every principal whose picture this actor may read.
///
/// **Q16A, exactly**: a contact belongs to a platform or a station, so a crew
/// member reads their own platform's picture and nobody else's. The roster is
/// the actor itself plus every unit it holds a control station on (cut 5's
/// `assignments_by_actor`, re-used rather than re-derived). There is no argument
/// anywhere that widens it, and no side or world arm at all.
fn viewable_holders(ctx: &Ctx<'_>, actor: ObjId) -> Vec<PrincipalId> {
    viewable_holders_of(ctx.w, actor)
}

/// [`viewable_holders`] over a bare `World` — the same roster for a reader
/// with no `Ctx` (the `tactics.contacts` panel, interface plan I8).
pub(crate) fn viewable_holders_of(w: &World, actor: ObjId) -> Vec<PrincipalId> {
    // ---- E7c11: B0 (the proxy DRIVING ingress; 10b §3.10 deviation (h)) ----
    // **A proxy driver reads the NPC's picture and ONLY the NPC's** (T12:
    // "exposes only that NPC station's observer projection"). The substitution
    // is total — the driver's own character drops out of the roster while the
    // seat is live — and it is NARROWED to the seat's own unit rather than every
    // unit the NPC is billeted on, which is the "not a control channel but
    // reachable through a station" act T11 names. Cut 10's own refutation of
    // K12 ("a proxy driver today perceives nothing of the unit") is what this
    // closes; without it a driver's `+contacts` renders nothing of the hull.
    if let Some(drive) = mush_sim::proxy_drive_of(w, actor) {
        let mut holders = vec![
            PrincipalId::Object { id: drive.npc },
            PrincipalId::Object { id: drive.unit },
        ];
        holders.sort();
        holders.dedup();
        return holders;
    }
    let mut holders = vec![PrincipalId::Object { id: actor }];
    if let Some(state) = w.module_state::<mush_sim::SpatialState>()
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

/// This actor's rows, **[`mush_core::MAX_CONTACTS_PER_OBSERVER`] per holder**.
///
/// Fold O6: the cap used to be taken across the COMBINED holder set, so an
/// actor stationed on three platforms saw 128 rows in total rather than 128 per
/// observer, and the first holder's volume hid every later one's picture. The
/// per-observer accessor the projection already carries is what answers now.
pub(crate) fn holder_rows<'a>(
    ledger: &'a ContactLedgerState,
    holders: &[PrincipalId],
) -> Vec<&'a ContactLedgerRow> {
    let mut rows: Vec<&ContactLedgerRow> = Vec::new();
    for holder in holders {
        let PrincipalId::Object { id } = holder else {
            continue;
        };
        rows.extend(ledger.for_observer(&mush_core::ObserverRef::Platform { id: *id }));
    }
    rows
}

/// The opaque contact the viewer itself holds on `source`, if any — the fog-safe
/// way to name a reporting platform (fold K15).
pub(crate) fn viewer_contact_on(
    ledger: &ContactLedgerState,
    holders: &[PrincipalId],
    source: ObjId,
) -> Option<ContactId> {
    let _ = source;
    // Propagation cannot resolve a platform back to the viewer's contact on it:
    // the leak-safe ledger deliberately carries no object id, which is the very
    // property that makes it leak-safe. Until cut 8's relay pass supplies the
    // reporting platform's own contact id alongside the provenance row, the
    // honest answer is "unattributed", which is what `None` renders.
    let _ = (ledger, holders);
    None
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
        .map(|row| {
            let provenance = store.provenance_of(row.contact);
            let source = match provenance.map(ContactProvenance::provenance) {
                Some(Provenance::Reported { source, .. }) => {
                    viewer_contact_on(ledger, &holders, *source)
                }
                _ => None,
            };
            render_contact(row, provenance, &at, source)
        })
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
        .map(|row| {
            let provenance = store.provenance_of(row.contact);
            let source = match provenance.map(ContactProvenance::provenance) {
                Some(Provenance::Reported { source, .. }) => {
                    viewer_contact_on(ledger, &holders, *source)
                }
                _ => None,
            };
            render_contact(row, provenance, &at, source)
        });
    match found {
        Some(line) => info(ctx, line),
        None => info(ctx, NO_SUCH_CONTACT),
    }
    Ok(())
}

/// Whether `actor` may address `recipient` at all (fold D6(a)).
///
/// One of: staff; a unit the actor holds a control station on (cut 5's
/// `assignments_by_actor`, re-used through [`viewable_holders`]); or an object
/// standing in the same place the actor does, read from the placement graph the
/// rest of the crate reads.
///
/// Cut 7 records **no authority relation**: an order is a message, and who a
/// unit *obeys* is cut 9's command relation, one predicate away from here.
fn may_address(ctx: &Ctx<'_>, actor: ObjId, recipient: ObjId) -> bool {
    // **E7c11 B0, deviation (h) — the order-authority half of the ingress.**
    // `actor` is the character that TYPED; `acting` is the actor whose reach
    // answers. For a proxy driver they differ, and the narrowing is real in both
    // directions: the roster is the seat's own `[npc, unit]` (never the NPC's
    // other billets — `viewable_holders`' proxy arm), and locality is the NPC's
    // place, not the absent character's room. Without the ingress `+order` from
    // a seat was inert; with it and without this narrowing it would be exactly
    // the unrestricted "reachable through a station" act T11 forbids.
    //
    // **The independent verify fold, D7.** `is_admin` is computed on the
    // **typing** actor (`SIM_ADMIN`/`Royalty`) and used to short-circuit, so a
    // staff account holding a proxy seat addressed any object anywhere in the
    // world while `cmd_order` still attributed the envelope to the NPC — the
    // widened act laundered onto a character that could neither perceive nor
    // reach the recipient, recoverable only through `@seat/audit`. That inverts
    // the rule the four `seat_restriction` sites are deliberately placed *ahead*
    // of their staff gates for: "a staff account can hold a proxy seat like
    // anyone else; T11 forbids the act **from the seat**, not from the person."
    // A live seat therefore suppresses the short-circuit; staff who want the
    // wider reach stand down first, which is what every seat refusal already
    // tells them. This narrows an existing arm's precondition — it adds no
    // fourth arm and is byte-unchanged for every non-proxy actor.
    let drive = mush_sim::proxy_drive_of(ctx.w, actor);
    let acting = drive.as_ref().map_or(actor, |drive| drive.npc);
    if actor == recipient || acting == recipient || (drive.is_none() && is_admin(ctx)) {
        return true;
    }
    if viewable_holders(ctx, actor).contains(&PrincipalId::Object { id: recipient }) {
        return true;
    }
    let Some(state) = ctx.w.module_state::<mush_sim::SpatialState>() else {
        return false;
    };
    match (
        crate::context::place_of(state, ctx.w, acting),
        crate::context::place_of(state, ctx.w, recipient),
    ) {
        (Some(here), Some(there)) => here == there,
        _ => false,
    }
}

/// How an issuer renders to a viewer, through the same gate `+order` uses.
fn attribution(ctx: &Ctx<'_>, viewer: ObjId, issuer: ObjId) -> Option<String> {
    may_address(ctx, viewer, issuer).then(|| display_name(ctx.w, issuer))
}

fn next_revision(store: &PropagationStore, issuer: ObjId, recipient: ObjId) -> u32 {
    store
        .envelopes()
        .filter(|envelope| {
            envelope.issuer() == issuer
                && envelope.recipient() == recipient
                && envelope.kind() == crate::record::DispatchKind::Order
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

fn persist_transport(ctx: &mut Ctx<'_>) {
    let Some(book) = store(ctx) else {
        return;
    };
    let value = book.transport.to_value();
    ctx.persist(ModuleOp {
        ns: NS.into(),
        op: ModuleWrite::JsonSet {
            key: crate::routing::STATE_KEY.into(),
            value,
        },
    });
}

fn route_submission(ctx: &mut Ctx<'_>, envelope: Envelope) -> Result<Envelope, CmdError> {
    if !crate::routing::enabled(ctx.w) {
        return Ok(envelope);
    }
    let envelope = if envelope.kind() == crate::record::DispatchKind::Acknowledgement {
        envelope
    } else {
        let r = crate::access::selected(ctx.w, envelope.issuer())
            .map_err(|e| CmdError::msg(e.message()))?;
        envelope.with_representation(r)
    };
    crate::admission::check(
        ctx.w,
        envelope.issuer(),
        envelope.representation(),
        crate::admission::producer(envelope.kind(), None),
        1,
    )
    .map_err(|e| CmdError::msg(e.message()))?;
    let metric = crate::routing::snapshot_in(ctx, envelope.issued());
    let route = ctx
        .w
        .module_state::<mush_sim::SpatialState>()
        .and_then(|state| {
            crate::routing::prepare_with(
                ctx.w,
                state,
                &envelope,
                envelope.issued(),
                metric.as_deref()?,
            )
        });
    if let Some(book) = store(ctx) {
        crate::routing::admission(book, route.as_ref(), None).map_err(CmdError::msg)?;
    }
    persist_transport(ctx);
    ctx.gate_on_durable_ack();
    Ok(route.map_or_else(|| envelope.clone(), |r| envelope.with_transit(r)))
}

/// `+order <recipient>=<order>` and `+order/report <recipient>=<contact>`.
///
/// Issuing revision N does **not** make it the recipient's active order: it
/// creates a `Queued` envelope, and only delivery makes it active (T15).
fn cmd_order(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let report_switch = ctx.switches.has("report");
    // **The recipient must be one the actor can actually address** (fold D6/K5).
    // `match_object`'s `#N` branch is `w.get(id).is_some()` with no locality,
    // visibility or contact gate, and the success line echoed the display name —
    // so `+order #412=go` answered "Queued revision 1 to Black Prince…" for any
    // object in the world and a match error for one that did not exist: an
    // existence-and-identity oracle over the whole dbref space, on a tactical
    // verb. Both arms now render the SAME line, byte for byte.
    // **E7c11 B0 (deviation (h)).** An order issued from a proxy seat is the
    // NPC's: it is the NPC that is identified to the recipient, the NPC's
    // reachability that gates the address, and the NPC's revision series the
    // envelope joins. The account is not IC and never appears in the envelope
    // (T12/T13); its attribution lives in cut 10's `SeatAudit`.
    let issuer = mush_sim::driving_actor(ctx.w, ctx.actor);
    let resolved = match_object(ctx.w, ctx.actor, ctx.args.lhs()).ok();
    let Some(recipient) = resolved.filter(|id| may_address(ctx, ctx.actor, *id)) else {
        info(ctx, NO_SUCH_CONTACT);
        return Ok(());
    };
    if recipient == issuer || recipient == ctx.actor {
        return Err(CmdError::msg("You cannot send yourself an order."));
    }
    let body = rhs(ctx, "+order <recipient>=<order>")?;
    let at = now(ctx);
    let contact = if report_switch {
        let wanted = body.trim().to_lowercase();
        // **E7c11 B0.** Keyed on the TYPING actor, so a proxy driver's `/report`
        // picks from exactly the seat's own narrowed picture (`viewable_holders`'
        // proxy arm) — the same set `+contacts` renders them. Passing `issuer`
        // (the NPC) here would widen it to every unit the NPC is billeted on,
        // which is wider than the seat and wider than what the driver can see.
        let holders = viewable_holders(ctx, ctx.actor);
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
    let envelope = route_submission(ctx, envelope)?;
    let slot = ctx
        .w
        .module_state_mut::<PropagationStore>()
        .ok_or_else(|| CmdError::msg(UNAVAILABLE))?;
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
    // **Bounded and paged** (fold B14): a long-lived pair used to render its
    // entire history in one command's output, with the only ceiling being
    // `MAX_ENVELOPES` and no compaction behind it.
    let mut traffic: Vec<&Envelope> = store
        .envelopes()
        .filter(|envelope| envelope.dispatch_ref().is_none())
        .filter(|envelope| {
            envelope.issuer() == actor
                || (envelope.recipient() == actor
                    && matches!(
                        envelope.state(),
                        EnvelopeState::Delivered
                            | EnvelopeState::Acknowledged
                            | EnvelopeState::Superseded
                    ))
        })
        .collect();
    let total = traffic.len();
    traffic.sort_unstable_by_key(|envelope| std::cmp::Reverse(envelope.id()));
    traffic.truncate(MAX_ORDERS_LISTED);
    traffic.sort_unstable_by_key(|envelope| envelope.id());
    let standing: Vec<(u32, String, ObjId)> = crate::pass::standing_orders(store, actor)
        .into_iter()
        .map(|order| (order.revision(), order.payload().to_owned(), order.issuer()))
        .collect();
    let mut lines: Vec<String> = Vec::new();
    for envelope in traffic {
        let source = attribution(ctx, actor, envelope.issuer());
        let direction = if envelope.issuer() == actor {
            "->"
        } else {
            "<-"
        };
        lines.push(format!(
            "{direction} {}",
            render_party_envelope(envelope, actor, &at, source.as_deref())
        ));
    }
    if total > MAX_ORDERS_LISTED {
        lines.push(format!(
            "{} older record(s) not shown; run +orders again after they compact.",
            total.saturating_sub(MAX_ORDERS_LISTED)
        ));
    }
    // Every standing order, **with its source** — one per issuer, because
    // supersession is per issuer and cut 7 ranks no issuer above another.
    for (revision, payload, issuer) in standing {
        let source = attribution(ctx, actor, issuer);
        lines.push(format!(
            "Standing order: revision {revision} — \"{payload}\"  (from {})",
            source.as_deref().unwrap_or(UNIDENTIFIED_SENDER)
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

/// The most envelopes `+orders` and `@prop/envelopes` render in one page.
pub const MAX_ORDERS_LISTED: usize = 32;

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
    if original.recipient() != actor
        || original.dispatch_ref().is_some()
        || original.state() != EnvelopeState::Delivered
        || original.kind() == crate::record::DispatchKind::Acknowledgement
    {
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
    let ack = route_submission(ctx, ack)?;
    let slot = ctx
        .w
        .module_state_mut::<PropagationStore>()
        .ok_or_else(|| CmdError::msg(UNAVAILABLE))?;
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
    if ctx.switches.has("route") {
        prop_route(ctx)
    } else if ctx.switches.has("pass") {
        prop_pass(ctx)
    } else if ctx.switches.has("phantom") {
        prop_phantom(ctx)
    } else if ctx.switches.has("jam") {
        prop_jam(ctx)
    } else if ctx.switches.has("held") {
        prop_held(ctx)
    } else if ctx.switches.has("envelopes") || ctx.switches.is_empty() {
        prop_envelopes(ctx)
    } else {
        Err(CmdError::msg(
            "Usage: @prop/envelopes <subject> | @prop/pass | @prop/phantom <observer>=<cause> |              @prop/jam <place>=<level> | @prop/held",
        ))
    }
}

/// Staff-only route truth. Ordinary readers never learn a hidden route or ETA.
fn prop_route(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if !is_admin(ctx) {
        return Err(CmdError::msg("Permission denied."));
    }
    let id: u64 = ctx
        .args
        .raw()
        .trim()
        .trim_start_matches('#')
        .parse()
        .map_err(|_| CmdError::msg("Usage: @prop/route <envelope id>"))?;
    let Some(envelope) = store(ctx).and_then(|s| s.envelope(id)).cloned() else {
        info(ctx, NO_SUCH_ENVELOPE);
        return Ok(());
    };
    let Some(route) = envelope.transit() else {
        info(
            ctx,
            format!("Envelope #{id}: no route; waiting for a known endpoint."),
        );
        return Ok(());
    };
    info(
        ctx,
        format!(
            "Envelope #{id} {:?} {:?}: route v{} registry {} at #{} to #{}; leg {}/{}; metric reads {}, edge visits {}.",
            envelope.kind(),
            envelope.state(),
            route.version,
            route.registry_revision,
            route.position.0,
            route.destination.0,
            route.next,
            route.legs.len(),
            route.last_work.metric_reads,
            route.last_work.edge_visits
        ),
    );
    for (i, leg) in route.legs.iter().enumerate() {
        info(
            ctx,
            format!(
                "leg {i}: #{} -> #{} {}",
                leg.from.0,
                leg.to.0,
                leg.pair.as_ref().map_or("conventional", |p| p.as_str())
            ),
        );
    }
    if let Some(flight) = &route.flight {
        info(
            ctx,
            format!(
                "Frozen leg: started {} due {} lag {} IC seconds; {}.",
                flight.started.at, flight.due.at, flight.seconds, flight.basis
            ),
        );
    }
    if let Some(progress) = &route.serialization {
        info(
            ctx,
            format!(
                "Serialization: {}/{} units, first IC {}, last IC {}; no recipient visibility until complete.",
                progress.sent, progress.total, progress.started, progress.last
            ),
        );
    }
    if let Some(queued) = &route.queued_at {
        info(
            ctx,
            format!(
                "FIFO queued at {} IC; {} bytes, {} units.",
                queued.at,
                envelope.payload().len(),
                envelope
                    .payload()
                    .len()
                    .div_ceil(crate::routing::BYTES_PER_UNIT)
            ),
        );
    }
    Ok(())
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
    let mut matching: Vec<&Envelope> = store
        .envelopes()
        .filter(|envelope| {
            (envelope.issuer() == subject || envelope.recipient() == subject)
                && (staff
                    || envelope.issuer() == actor
                    || matches!(
                        envelope.state(),
                        EnvelopeState::Delivered
                            | EnvelopeState::Acknowledged
                            | EnvelopeState::Superseded
                    ))
        })
        .collect();
    let total = matching.len();
    matching.sort_unstable_by_key(|envelope| std::cmp::Reverse(envelope.id()));
    matching.truncate(MAX_ORDERS_LISTED);
    matching.sort_unstable_by_key(|envelope| envelope.id());
    let party = staff || subject == actor;
    let mut lines: Vec<String> = matching
        .into_iter()
        .map(|envelope| {
            let source = attribution(ctx, actor, envelope.issuer());
            if staff {
                render_envelope(envelope, &at, party, source.as_deref())
            } else {
                render_party_envelope(envelope, actor, &at, source.as_deref())
            }
        })
        .collect();
    if lines.is_empty() {
        info(ctx, NO_SUCH_ENVELOPE);
        return Ok(());
    }
    if total > MAX_ORDERS_LISTED {
        lines.push(format!(
            "{} older record(s) not shown.",
            total.saturating_sub(MAX_ORDERS_LISTED)
        ));
    }
    for line in lines {
        info(ctx, line);
    }
    Ok(())
}

/// `@prop/held` — the verb that can actually show a held document (fold B7).
///
/// The boot notice used to name `@prop/envelopes`, which reads `store.envelopes()`
/// — a held document is in no iterator any verb consulted, so nobody could see
/// which subject's traffic was missing.
fn prop_held(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if !is_admin(ctx) {
        return Err(CmdError::msg("Permission denied."));
    }
    let Some(store) = store(ctx) else {
        info(ctx, UNAVAILABLE);
        return Ok(());
    };
    let mut lines: Vec<String> = store
        .held()
        .iter()
        .map(|(key, value)| {
            let kind = key.split(':').next().unwrap_or("unknown");
            format!(
                "{key}  [{kind}]  {}",
                decode_error_of(key, value).unwrap_or_else(|| "over the census bound".to_owned())
            )
        })
        .collect();
    let overflow = store.held_overflow();
    if lines.is_empty() && overflow == 0 {
        info(ctx, "Nothing is held.");
        return Ok(());
    }
    if overflow > 0 {
        lines.push(format!(
            "{overflow} further unreadable document(s) were past the RAM cap of {} and are still \
             in the store, unmirrored.",
            crate::record::MAX_HELD_DOCUMENTS
        ));
    }
    for line in lines {
        info(ctx, line);
    }
    Ok(())
}

/// Re-run the decode of a held document so the verb can name the actual error.
fn decode_error_of(key: &str, value: &serde_json::Value) -> Option<String> {
    if key.starts_with("envelope:") {
        serde_json::from_value::<Envelope>(value.clone())
            .err()
            .map(|e| e.to_string())
    } else if key.starts_with("phantom:") {
        serde_json::from_value::<Phantom>(value.clone())
            .err()
            .map(|e| e.to_string())
    } else if key.starts_with("provenance:") {
        serde_json::from_value::<ContactProvenance>(value.clone())
            .err()
            .map(|e| e.to_string())
    } else if key.starts_with("sighting:") {
        serde_json::from_value::<crate::record::Sighting>(value.clone())
            .err()
            .map(|e| e.to_string())
    } else {
        None
    }
}

/// `@prop/pass` — single-step the coarse pass (staff).
///
/// It runs the same [`delivery_pass_with_metric`] the coarse tick runs and then arms the
/// detection pass's intents through **`mush-tactics`' own ingress**: one
/// `Ctx::submit_authoritative` of the tactics-owned `sensor_arm` action, never a
/// contact write of its own. Propagation writes **no `ContactState`** — it could
/// not, and this is the verb where that is easiest to check.
fn prop_pass(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if !is_admin(ctx) {
        return Err(CmdError::msg("Permission denied."));
    }
    crate::commerce::sweep(ctx);
    crate::billing::maintain(ctx);
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
    let metric = crate::routing::snapshot_in(ctx, &at);
    let report = delivery_pass_with_metric(
        &mut book,
        &state,
        ctx.w,
        &ledger_snapshot,
        &at,
        None,
        metric,
    );
    if !crate::commerce::settle_pass(ctx, &mut book) {
        info(
            ctx,
            "Paid relay transmission waits for settlement; routing pass not applied.",
        );
        return Ok(());
    }
    // The pass is given the caller's OWN arm bound, so it books exactly what it
    // emits and the page it reports is true (fold D4(a)).
    let detection = crate::pass::detection_pass(
        &mut book,
        &state,
        ctx.w,
        &ledger_snapshot,
        &at,
        None,
        mush_tactics_max_intents_per_arm(),
    );
    // Persist the delivery post-images and only the sighting rows the pass
    // actually touched (fold B12: the whole book was re-written on every call,
    // up to 4_096 persistence ops per invocation even when nothing changed).
    let touched = report.touched();
    let written: Vec<crate::record::Sighting> = detection
        .sightings
        .iter()
        .filter_map(|key| book.sighting(key).cloned())
        .collect();
    let dropped_sightings = detection.dropped_sightings.clone();
    let reaped = report.reaped.clone();
    let notices = book.take_notices();
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
    for sighting in &written {
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
    for key in dropped_sightings.iter().chain(&reaped.sightings) {
        ctx.persist(ModuleOp {
            ns: SmolStr::new_static(NS),
            op: ModuleWrite::Del {
                key: sighting_key(key),
            },
        });
    }
    for id in &reaped.envelopes {
        ctx.persist(ModuleOp {
            ns: SmolStr::new_static(NS),
            op: ModuleWrite::Del {
                key: envelope_key(*id),
            },
        });
    }
    for contact in &reaped.provenance {
        ctx.persist(ModuleOp {
            ns: SmolStr::new_static(NS),
            op: ModuleWrite::Del {
                key: provenance_key(*contact),
            },
        });
    }
    for id in &reaped.phantoms {
        ctx.persist(ModuleOp {
            ns: SmolStr::new_static(NS),
            op: ModuleWrite::Del {
                key: phantom_key(id),
            },
        });
    }
    persist_allocator(ctx);
    // Anything the TICK paged since the last staff read (fold B3/B7).
    if !report.routed.is_empty() {
        persist_transport(ctx);
    }
    for notice in notices {
        info(ctx, notice);
    }
    report_pass(ctx, &report);
    arm_intents(ctx, detection);
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
    if !report.reaped.is_empty() {
        info(
            ctx,
            format!(
                "Compacted {} finished record(s): {} envelope(s), {} provenance row(s), {} \
                 phantom(s).",
                report.reaped.len(),
                report.reaped.envelopes.len(),
                report.reaped.provenance.len(),
                report.reaped.phantoms.len()
            ),
        );
    }
    if report.bound_hit {
        // The bound is PAGED, not logged (E4c V6), and the page is now TRUE:
        // the scan cursor is durable, so running again resumes at the envelope
        // the line names rather than repeating the same work (fold B3).
        info(
            ctx,
            format!(
                "The pass stopped at its bound of {} envelopes and resumes at #{}; run @prop/pass \
                 again to continue.",
                crate::pass::MAX_ENVELOPES_PER_PASS,
                report.resume.unwrap_or(0)
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
fn arm_intents(ctx: &mut Ctx<'_>, detection: crate::pass::DetectionReport) {
    for refusal in &detection.refused {
        info(ctx, refusal.clone());
    }
    if detection.stranded > 0 {
        info(
            ctx,
            format!(
                "{} contact(s) were stranded on an older world time generation and are being \
                 retired; their holders will see them go lost.",
                detection.stranded
            ),
        );
    }
    let resume_line = detection.resume.map(|(observer, subject)| {
        format!(
            " — the pass stopped at its bound and resumes at observer #{}, subject #{}; run \
             @prop/pass again",
            observer.0, subject.0
        )
    });
    let intents = detection.intents;
    if intents.is_empty() {
        info(ctx, "Detection pass: nothing due.");
        return;
    }
    let armed = intents.len().min(mush_tactics_max_intents_per_arm());
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
            "Detection pass: {armed} observation(s) armed through the tactics ingress{}.",
            resume_line.unwrap_or_default()
        ),
    );
}

const fn mush_tactics_max_intents_per_arm() -> usize {
    // The tactics ingress's own per-arm bound, restated rather than imported:
    // `mush-propagation` has no `mush-tactics` edge, and a mismatch is caught by
    // the composition pin in `mush-server`.
    16
}

/// The same bound, for the `propagation.detection` tick (fold D8(a)).
#[must_use]
pub const fn max_intents_per_arm() -> usize {
    mush_tactics_max_intents_per_arm()
}

/// The tactics-owned `sensor_arm` action, for the detection tick.
#[must_use]
pub fn sensor_arm_action_ref(reg: &mush_core::Registry) -> Option<mush_core::DefinitionRef> {
    reg.definitions()?
        .current(&mush_core::DefinitionKey {
            owner: mush_core::OwnerId::new("tactics").ok()?,
            kind: mush_core::DefinitionKind::new("action").ok()?,
            id: mush_core::DefinitionId::new("sensor_arm").ok()?,
        })
        .cloned()
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
    // A digest of `(observer, ordinal, created)`, never the observer's dbref
    // (fold K13): a phantom id appears in its document key, in the sighting book
    // and in `ContactLedgerRow.phantom`, so a readable one told anybody who
    // could see it which platform the false contact was minted for.
    let id = Phantom::mint_id(observer, ordinal, &at)
        .map_err(|error| CmdError::msg(error.to_string()))?;
    let phantom = Phantom::new(
        id.clone(),
        cause,
        at,
        expires,
        vec![PrincipalId::Object { id: observer }],
    )
    .map_err(|error| CmdError::msg(error.to_string()))?
    .with_ordinal(ordinal);
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
    // Interface plan I8 (D-I8-1): the contacts table — the cells `+contacts`
    // prints, as a panel keyframe the unit screens render. A query handler is
    // not a command, so it sits outside the K8.5 classification groups.
    let _contacts = reg.panel_feed(crate::panel::CONTACTS_FEED)?;
    reg.query_handler(
        crate::panel::CONTACTS_FEED,
        Arc::new(crate::panel::ContactsQuery::new(
            crate::panel::CONTACTS_FEED,
        )),
    )?;
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
    // Group 3 — `@prop` takes the CEILING class of its switches, and `/pass`
    // stages the tactics-owned `sensor_arm` through `Ctx::submit_authoritative`:
    // it is an ingress adapter for the authoritative action coordinator, which
    // is `Authoritative`'s own definition. The precedent is explicit for
    // `@station` in `mush-sim` (fold B8).
    reg.classify_commands(mush_core::CommandCompatibility::Authoritative);
    reg.command(
        CommandSpec {
            name: "@prop".into(),
            shape: ArgShape::EqPair,
            switches: ["envelopes", "pass", "phantom", "jam", "held", "route"]
                .iter()
                .copied()
                .map(SmolStr::new)
                .collect(),
            help: "@prop/envelopes <subject>  --  Traffic on a subject (yours; staff see all).\n\
                   @prop/route <id>  --  (staff) Frozen legs, FIFO state and route-work bounds.\n\
                   @prop/pass  --  (staff) Single-step the coarse delivery pass and arm the \
                   detection pass through the tactics sensor ingress.\n\
                   @prop/phantom <observer>=artefact|decoy|spoof[,<ttl>]  --  (staff) Mint a \
                   false contact's scoped subject.\n\
                   @prop/held  --  (staff) Documents the boot could not decode, with the error."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_prop),
    )?;
    // K8.5: restore the module default after the last group.
    reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
    Ok(())
}
