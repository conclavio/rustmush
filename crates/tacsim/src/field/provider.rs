//! **`tacsim_field`** — the field host's writes to the site record (doc 32 cut CF.2), one
//! authoritative action with three operations.
//!
//! - **`open`** (Q-CF-14, Q-CF-30, Q-CF-31): the promotion. In one bundle the site's next field
//!   opens on the record, `PromotedBy::Mark` or `PromotedBy::Fire`, and every surface act in
//!   flight on the site that can be settled where it stands is settled: a drive from the ground
//!   (a recover's drive home too) stops at the point its leg had reached, pays for the share of
//!   the leg it drove and is told `the field opened`; a scan, a sounding or a dump is cancelled
//!   and pays for the share of its time that ran; a mining run is told to stop after the cycle
//!   in flight, which keeps the ore it extracts (no duplicate yield, Q-CF-30). The settled rigs'
//!   poses ride the promotion ([`crate::surface::site_record::SettledRig`]) for the host to place
//!   before it admits anyone. A launch, a recover from beside the ramp, a drive from the bay and
//!   an unload are ramp handoffs (Q-H4): they finish as they would, and the host admits once they
//!   have — a departure on the ramp completes into the field at the ramp pose (§4a).
//! - **`exit`** (doc 32 cut CF.5; Q-CF-18): a unit that left the running field at its carrier's
//!   ramp, recorded on the open promotion so its recover into the bay is the one surface act the
//!   promoted site takes (`SiteRecord::exited`).
//! - **`close`** (Q-CF-20, Q-CF-35): the demotion. The record's unit index becomes every unit the
//!   field leaves on the ground at its field pose; the flag clears; the instant is kept. **Doc 36
//!   cut GC.3** (Q-36-18, Q-CF-25 c): each irrecoverable hull the host names drops a remains
//!   object with its mass, and a rig's open epoch closes `Destroyed` in the same bundle, its
//!   `sortie_closed` line riding the black box's participant.
//!
//! **Who may write.** `open` is offered to anybody and refused at the prepare on the record's own
//! state; the verbs (`+field/mark`, `+field/fire`) decide who leads. `close` closes only what the
//! host owes: the offer fixes the host's owed closes — each site's ordinal and the units it
//! leaves, from the host's RAM — into the invocation, so no player can demote a field, or write
//! a unit index, by submitting it. `exit` writes only the host's owed exits, fixed the same way.
//!
//! **Doc 23 cut D-23-18** (§17.7): the minds the host checked in on open epochs are fixed the same
//! way (`carried`, by site), and every `exit` or `close` on the site lays each on its rig record's
//! `OpenSortie.mind` in the write's own bundle, while that record's epoch is still the one the
//! mind flew on.

use std::collections::BTreeSet;

use mush_core::{
    ActionError, ActionErrorCode, ActionInvocation, ActionProvider, ActionTxn,
    AuthoritativeActionOutput, CanonicalId, DefinitionRef, ObjId, ProjectedParameterKind,
    ProviderOfferDraft, RulesValue, StateFamilyHandle, World,
};
use serde_json::{Map, Value, json};

use crate::port_tick::port_error;
use crate::surface::excursion::{
    ActKind, ActResult, Outcome, RefusedCode, RigKey, RigRecord, settle_spent_out_of_bay,
};
use crate::surface::site_record::{
    LeftRemains, LeftUnit, PromotedBy, SettledRig, SiteKey, SitePose, SiteRecord,
};

/// The action.
pub const FIELD_ACTION: &str = "tacsim_field";
/// Its outcome.
pub const FIELD_OUTCOME: &str = "tacsim_field_result";
/// Its idempotency namespace: `tacsim.field/{actor}/{site}/{op}/{ordinal}/{now_ic}`.
pub const FIELD_KEY_PREFIX: &str = "tacsim.field/";
/// The most bytes an `open`'s rig list may carry (the offer's text bound).
pub const MAX_LIST_BYTES: usize = mush_core::MAX_PROJECTED_ACTION_TEXT_BYTES;
/// **The most canonical JSON bytes `FieldProvider::project_offer`'s `fixed_parameters` may take**
/// (security hardening CRIT-18): strictly `<= 48 KiB`, leaving >= 16 KiB headroom below
/// [`mush_core::MAX_ACTION_PARAMETER_BYTES`] (64 KiB) so `InternalOffer::new` can never fail
/// globally and lock out `SelfActor` legal actions.
pub const MAX_OFFER_FIXED_BYTES: usize = 48 * 1024;
/// **The most bytes the offer's carried minds may take** (doc 23 cut D-23-18; bounded for
/// CRIT-18): half of [`MAX_OFFER_FIXED_BYTES`] (24 KiB), so carried minds never starve or
/// overflow the 48 KiB fixed-parameter budget. Extra minds ride `op = "carry"` chunks.
pub const MAX_CARRIED_BYTES: usize = MAX_OFFER_FIXED_BYTES / 2;

/// **`tacsim_field`**'s provider.
pub(crate) struct FieldProvider {
    pub action: DefinitionRef,
    pub outcome: DefinitionRef,
    pub sites: StateFamilyHandle<SiteKey, SiteRecord>,
    pub excursions: StateFamilyHandle<RigKey, RigRecord>,
    /// Doc 36 cut GC.3: the black box's participant, for a close-destroyed epoch's line.
    pub black_box: Option<
        mush_core::participant::ParticipantHandle<crate::surface::black_box::BlackBoxParticipant>,
    >,
}

fn invalid(message: &'static str) -> ActionError {
    port_error(ActionErrorCode::InvalidRequest, message)
}

fn rejected(message: &'static str) -> ActionError {
    port_error(ActionErrorCode::ProviderRejected, message)
}

fn integrity(message: &'static str) -> ActionError {
    port_error(ActionErrorCode::Integrity, message)
}

fn parameters(invocation: &ActionInvocation) -> Result<&Map<String, Value>, ActionError> {
    if invocation.subject() != &mush_core::PerceivedRef::SelfActor
        || !invocation.targets().is_empty()
    {
        return Err(invalid("a field write is self-scoped and takes no targets"));
    }
    invocation
        .parameters()
        .as_object()
        .ok_or_else(|| invalid("a field write names a site and an operation"))
}

fn int(parameters: &Map<String, Value>, name: &str) -> Option<i64> {
    parameters.get(name).and_then(Value::as_i64)
}

fn obj(parameters: &Map<String, Value>, name: &str) -> Result<ObjId, ActionError> {
    int(parameters, name)
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| *n != 0)
        .map(ObjId)
        .ok_or_else(|| invalid("a field write names its units by number"))
}

/// The idempotency key one write takes.
#[must_use]
pub fn field_key(
    actor: ObjId,
    site: &CanonicalId,
    op: &str,
    ordinal: u32,
    now_ic: i64,
) -> Option<mush_core::IdempotencyKeyText> {
    mush_core::IdempotencyKeyText::parse_canonical(format!(
        "{FIELD_KEY_PREFIX}{}/{site}/{op}/{ordinal}/{now_ic}",
        actor.0
    ))
    .ok()
}

/// **Settle one rig's work in flight on `site` at `now`** (this module's head), answering the
/// pose a settled drive stopped at. `None` when nothing moved: nothing in flight there, a ramp
/// handoff the host waits for, or a cancelled act that stands where it stood.
#[must_use]
pub(crate) fn settle(record: &mut RigRecord, site: &CanonicalId, now: i64) -> Option<SitePose> {
    if let Some(run) = record.run.as_mut()
        && &run.site == site
    {
        run.stop = true;
    }
    let act = record.act.clone()?;
    if &act.site != site {
        return None;
    }
    let elapsed = now.saturating_sub(act.started_ic).max(0);
    let total = act.due_ic.saturating_sub(act.started_ic).max(1);
    let share = |cu: u32| -> u32 {
        let paid = i128::from(cu) * i128::from(elapsed.min(total)) / i128::from(total);
        u32::try_from(paid).unwrap_or(cu)
    };
    let pose = match act.kind {
        ActKind::Drive | ActKind::Recover if !act.params.launches() => {
            let (waypoints, _) = act.params.leg()?;
            let (x_cm, y_cm) = crate::port::site_map::leg_position(waypoints, elapsed)?;
            let heading_mdeg = match &act.params {
                crate::surface::excursion::ActParams::Drive { heading_mdeg, .. } => *heading_mdeg,
                _ => 0,
            };
            Some(SitePose {
                x_cm,
                y_cm,
                heading_mdeg,
            })
        }
        ActKind::Scan | ActKind::Sound | ActKind::Dump if !act.params.launches() => None,
        // A launch, a recover from beside the ramp, a drive or a reading from the bay, an
        // unload: ramp handoffs, left to finish.
        _ => return None,
    };
    record.charge = settle_spent_out_of_bay(&record.charge, share(act.reserved_cu), now)
        .unwrap_or(record.charge);
    record.act = None;
    record.last = Some(Outcome {
        ordinal: act.ordinal,
        kind: act.kind,
        operator: act.operator,
        slot: act.slot,
        site: act.site.clone(),
        at_ic: now,
        result: ActResult::Refused {
            code: RefusedCode::FieldOpened,
        },
        moved: std::collections::BTreeMap::new(),
        span: None,
        arrived: pose.map(|pose| {
            let (x, y) = pose.cell();
            [x, y]
        }),
        mined: None,
        stays: std::collections::BTreeMap::new(),
        by: act.by,
    });
    pose
}

/// **Whether `record` holds a ramp handoff on `site`**: an act there [`settle`] leaves to finish.
#[must_use]
pub(crate) fn handoff_pending(record: &RigRecord, site: &CanonicalId) -> bool {
    record
        .act
        .as_ref()
        .is_some_and(|act| &act.site == site && !settleable(act))
}

/// Whether [`settle`] stops `act` where it stands.
fn settleable(act: &crate::surface::excursion::Act) -> bool {
    !act.params.launches()
        && match act.kind {
            ActKind::Drive | ActKind::Recover => act.params.leg().is_some(),
            ActKind::Scan | ActKind::Sound | ActKind::Dump => true,
            ActKind::Launch | ActKind::Mine | ActKind::Unload => false,
        }
}

impl FieldProvider {
    fn open(
        &self,
        txn: &mut ActionTxn<'_>,
        record: &mut SiteRecord,
        parameters: &Map<String, Value>,
        (actor, now): (ObjId, i64),
    ) -> Result<String, ActionError> {
        let by = match parameters.get("by").and_then(Value::as_str) {
            Some("mark") => PromotedBy::Mark {
                leader: actor,
                by: obj(parameters, "from")?,
                of: obj(parameters, "at")?,
            },
            Some("fire") => PromotedBy::Fire {
                pilot: actor,
                shooter: obj(parameters, "from")?,
                target: obj(parameters, "at")?,
            },
            _ => return Err(invalid("a field opens by a mark or a fire order")),
        };
        let field = record
            .promote(now, by)
            .map_err(|why| rejected(why.sentence()))?;
        let rigs: BTreeSet<ObjId> = parameters
            .get("rigs")
            .and_then(Value::as_str)
            .unwrap_or("")
            .split(',')
            .filter(|word| !word.is_empty())
            .map(|word| {
                crate::port::families::strict_decimal_u32(word)
                    .map(ObjId)
                    .ok_or_else(|| invalid("a field's rig list is numbers"))
            })
            .collect::<Result<_, _>>()?;
        let mut settled = Vec::new();
        for tv in rigs {
            let key = RigKey(tv);
            let Some(mut rig) = txn
                .get(&self.excursions, &key)
                .map_err(|_| integrity("the rig records are unavailable"))?
                .cloned()
            else {
                continue;
            };
            let before = rig.clone();
            if let Some(pose) = settle(&mut rig, &record.site, now) {
                settled.push(SettledRig { tv, pose });
            }
            if rig != before {
                rig.check()
                    .map_err(|_| integrity("a settled rig record is out of bounds"))?;
                txn.replace(&self.excursions, &key, rig)
                    .map_err(|_| integrity("a settled rig record did not stage"))?;
            }
        }
        if let Some(open) = record.promoted.as_mut() {
            open.settled = settled;
        }
        Ok(format!("field {} opened", field.ordinal))
    }

    /// **Close-destroyed** (doc 36 cut GC.3; Q-36-18, Q-CF-25 c): `remains` lies on the ground,
    /// and the rig's epoch it names, when the rig's own record holds it open, closes `Destroyed`
    /// at `now`, its line appended in this bundle. An epoch held elsewhere (a DT flight's, which
    /// its own family keeps) is left open, and logged.
    fn close_destroyed(
        &self,
        txn: &mut ActionTxn<'_>,
        context: &mut mush_core::PrepareCtx<'_>,
        record: &mut SiteRecord,
        (remains, now): (&LeftRemains, i64),
    ) -> Result<(), ActionError> {
        record
            .leave_remains(remains)
            .map_err(|why| rejected(why.sentence()))?;
        let Some(epoch) = remains.epoch else {
            return Ok(());
        };
        let key = RigKey(remains.unit);
        let found = txn
            .get(&self.excursions, &key)
            .map_err(|_| integrity("the rig records are unavailable"))?
            .cloned();
        let Some(mut rig) =
            found.filter(|rig| rig.sortie.as_ref().is_some_and(|open| open.id == epoch))
        else {
            tracing::warn!(
                unit = remains.unit.0,
                "tacsim: an irrecoverable hull's epoch is not its rig record's; it stays open"
            );
            return Ok(());
        };
        let Some(open) = rig.sortie.take() else {
            return Ok(());
        };
        // The hull is gone from the ground: nothing of its surface work stands.
        rig.excursion = None;
        rig.act = None;
        rig.run = None;
        rig.queued = None;
        rig.plan = None;
        // Doc 23 cut S.4b-2: the claim's Close is owed with the epoch's (`Destroyed`).
        rig.owe_claim_close(mush_roll::commerce::ic_claims::IcCloseKind::Destroyed);
        rig.check()
            .map_err(|_| integrity("a destroyed rig's record is out of bounds"))?;
        txn.replace(&self.excursions, &key, rig)
            .map_err(|_| integrity("a destroyed rig's record did not stage"))?;
        let line = crate::surface::black_box::BlackBoxLine::sortie_closed(
            remains.unit,
            &open,
            now,
            mush_roll::commerce::ic_claims::IcCloseKind::Destroyed,
        );
        crate::surface::black_box::append(context, self.black_box.as_ref(), remains.unit, line)?;
        Ok(())
    }
}

impl FieldProvider {
    /// **A bay rig its wrecked carrier took down owes its claim's Close** (doc 38a F-23.1):
    /// `Destroyed`, its queued launch cancelled; the claim arm lands the Close, and the host's
    /// tick then wrecks the rig. A rig with no open claim is left alone.
    fn close_wrecked_claim(&self, txn: &mut ActionTxn<'_>, rig: ObjId) -> Result<(), ActionError> {
        let key = RigKey(rig);
        let Some(mut record) = txn
            .get(&self.excursions, &key)
            .map_err(|_| integrity("the rig records are unavailable"))?
            .cloned()
        else {
            return Ok(());
        };
        if !wreck_claim(&mut record) {
            return Ok(());
        }
        record
            .check()
            .map_err(|_| integrity("a wrecked bay rig's record is out of bounds"))?;
        txn.replace(&self.excursions, &key, record)
            .map_err(|_| integrity("a wrecked bay rig's record did not stage"))?;
        Ok(())
    }

    /// **Lay the site's carried minds on their epoch records** (doc 23 cut D-23-18): each row
    /// the offer fixed for `site` whose rig record still has that epoch open becomes its
    /// `OpenSortie.mind`, staged in this write's bundle. A record whose epoch closed or moved on
    /// is left alone: the mind lasted its epoch.
    fn lay_minds(
        &self,
        txn: &mut ActionTxn<'_>,
        parameters: &Map<String, Value>,
        site: &SiteKey,
    ) -> Result<(), ActionError> {
        let rows: Vec<super::record::CarriedRow> = parameters
            .get("carried")
            .and_then(|carried| carried.get(site.0.as_str()))
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|_| invalid("a field write lists the minds it carries"))?
            .unwrap_or_default();
        let mut seen = BTreeSet::new();
        for row in rows {
            if !seen.insert(row.unit) {
                continue;
            }
            let key = RigKey(row.unit);
            let found = txn
                .get(&self.excursions, &key)
                .map_err(|_| integrity("the rig records are unavailable"))?
                .cloned();
            let Some(mut rig) = found else {
                continue;
            };
            let Some(open) = rig.sortie.as_mut().filter(|open| open.id == row.epoch) else {
                continue;
            };
            if open.mind.as_deref() == Some(&row.mind) {
                continue;
            }
            open.mind = Some(Box::new(row.mind));
            rig.check()
                .map_err(|_| integrity("a carried mind's rig record is out of bounds"))?;
            txn.replace(&self.excursions, &key, rig)
                .map_err(|_| integrity("a carried mind's rig record did not stage"))?;
        }
        Ok(())
    }
}

/// **A wrecked bay rig's record, its claim owed `Destroyed`** (doc 38a F-23.1): an open claim
/// becomes `Closing { Destroyed }` and its queued launch is cancelled; anything else is left
/// alone (`false`).
fn wreck_claim(record: &mut RigRecord) -> bool {
    if !matches!(
        record.claim.as_deref(),
        Some(crate::surface::excursion::ClaimCursor::Open { .. })
    ) {
        return false;
    }
    record.owe_claim_close(mush_roll::commerce::ic_claims::IcCloseKind::Destroyed);
    record.queued = None;
    true
}

/// Whether `value`'s canonical JSON fits within `limit` bytes.
fn fits_budget(value: &Value, limit: usize) -> bool {
    mush_core::mechanics::canonical_json_bytes_with_limit(value, limit).is_ok()
}

/// Canonical JSON byte length of `value`, or `usize::MAX` if it exceeds `limit` or fails to encode.
fn canonical_len(value: &Value, limit: usize) -> usize {
    mush_core::mechanics::canonical_json_bytes_with_limit(value, limit)
        .map_or(usize::MAX, |bytes| bytes.len())
}

/// Whether a single carried-mind row fits inside [`MAX_CARRIED_BYTES`] on its own.
#[must_use]
pub(crate) fn carried_row_fits(row: &super::record::CarriedRow) -> bool {
    serde_json::to_value(row)
        .ok()
        .is_some_and(|value| fits_budget(&value, MAX_CARRIED_BYTES))
}

/// Serialize one site's close entry with the given `known`, `units`, `remains`, and `wrecked`.
fn close_entry_value(
    ordinal: u32,
    units: &[LeftUnit],
    known: &[super::known::Learned],
    remains: &[LeftRemains],
    wrecked: &[ObjId],
) -> Value {
    json!({
        "ordinal": ordinal,
        "units": units,
        "known": known,
        "remains": remains,
        "wrecked": wrecked,
    })
}

/// Insert `(site, entry)` into `fixed[section]` iff the resulting canonical JSON stays within
/// [`MAX_OFFER_FIXED_BYTES`]. Reverts on overflow.
fn try_insert_map_entry(fixed: &mut Value, section: &str, site: &str, entry: Value) -> bool {
    let prior = fixed
        .as_object_mut()
        .and_then(|obj| obj.get_mut(section))
        .and_then(Value::as_object_mut)
        .map(|map| map.insert(site.to_owned(), entry));
    if prior.is_none() {
        return false;
    }
    if fits_budget(fixed, MAX_OFFER_FIXED_BYTES) {
        return true;
    }
    if let Some(map) = fixed
        .as_object_mut()
        .and_then(|obj| obj.get_mut(section))
        .and_then(Value::as_object_mut)
    {
        match prior.flatten() {
            Some(old) => {
                map.insert(site.to_owned(), old);
            }
            None => {
                map.remove(site);
            }
        }
    }
    false
}

/// Fit one site's `OwedClose` into `fixed["closing"]`, degrading `known` (and only as a last
/// resort `units`/`remains`/`wrecked`) when `allow_degrade` is true (i.e. this is the primary
/// close in the offer) so even an oversized tombstone can still close without exceeding
/// [`MAX_OFFER_FIXED_BYTES`].
fn fit_close_entry(
    fixed: &mut Value,
    site: &str,
    close: &super::admit::OwedClose,
    allow_degrade: bool,
) -> bool {
    let mut known: Vec<super::known::Learned> = close
        .known
        .iter()
        .cloned()
        .map(|mut row| {
            row.clamp();
            row
        })
        .collect();
    let full = close_entry_value(
        close.ordinal,
        &close.units,
        &known,
        &close.remains,
        &close.wrecked,
    );
    if try_insert_map_entry(fixed, "closing", site, full) {
        return true;
    }
    if !allow_degrade {
        return false;
    }
    // Step 1: drop `eyes` from `known` rows while keeping learned items.
    for row in &mut known {
        row.eyes.clear();
    }
    let no_eyes = close_entry_value(
        close.ordinal,
        &close.units,
        &known,
        &close.remains,
        &close.wrecked,
    );
    if try_insert_map_entry(fixed, "closing", site, no_eyes) {
        tracing::warn!(
            site,
            ordinal = close.ordinal,
            "tacsim: field close offer dropped learned eye cells to stay within 48 KiB"
        );
        return true;
    }
    // Step 2: binary-search the largest prefix of `known` that fits.
    let mut lo = 0_usize;
    let mut hi = known.len();
    while lo < hi {
        let mid = lo.saturating_add(hi.saturating_sub(lo).div_ceil(2));
        let prefix = known.get(..mid).unwrap_or(&[]);
        let mut probe = fixed.clone();
        let candidate = close_entry_value(
            close.ordinal,
            &close.units,
            prefix,
            &close.remains,
            &close.wrecked,
        );
        if try_insert_map_entry(&mut probe, "closing", site, candidate) {
            lo = mid;
        } else {
            hi = mid.saturating_sub(1);
        }
    }
    let prefix = known.get(..lo).unwrap_or(&[]);
    let trimmed_known = close_entry_value(
        close.ordinal,
        &close.units,
        prefix,
        &close.remains,
        &close.wrecked,
    );
    if try_insert_map_entry(fixed, "closing", site, trimmed_known) {
        tracing::warn!(
            site,
            ordinal = close.ordinal,
            kept_known = lo,
            total_known = known.len(),
            "tacsim: field close offer trimmed learned rows to stay within 48 KiB"
        );
        return true;
    }
    // Step 3: last-resort degradation for a corrupt/oversized tombstone — empty non-essential
    // payload so the site can still demote cleanly instead of wedging globally.
    let minimal = close_entry_value(close.ordinal, &[], &[], &[], &[]);
    if try_insert_map_entry(fixed, "closing", site, minimal) {
        tracing::error!(
            site,
            ordinal = close.ordinal,
            "tacsim: field close offer degraded to minimal demotion to stay within 48 KiB"
        );
        return true;
    }
    false
}

/// Append as many `CarriedRow`s for `site` into `fixed["carried"][site]` as fit within both
/// `carried_bytes <= MAX_CARRIED_BYTES` and `fixed <= MAX_OFFER_FIXED_BYTES`. Returns the count
/// of rows inserted.
fn pack_carried_for_site<'a>(
    fixed: &mut Value,
    carried_bytes: &mut usize,
    site: &str,
    rows: impl IntoIterator<Item = &'a super::record::CarriedRow>,
) -> usize {
    let mut packed = 0_usize;
    for row in rows {
        let Ok(value) = serde_json::to_value(row) else {
            continue;
        };
        let size = canonical_len(&value, MAX_CARRIED_BYTES);
        if carried_bytes.saturating_add(size) > MAX_CARRIED_BYTES {
            continue;
        }
        let mut probe = fixed.clone();
        let Some(carried_map) = probe
            .as_object_mut()
            .and_then(|obj| obj.get_mut("carried"))
            .and_then(Value::as_object_mut)
        else {
            break;
        };
        if let Value::Array(list) = carried_map
            .entry(site.to_owned())
            .or_insert_with(|| Value::Array(Vec::new()))
        {
            list.push(value);
        }
        if !fits_budget(&probe, MAX_OFFER_FIXED_BYTES) {
            continue;
        }
        *fixed = probe;
        *carried_bytes = carried_bytes.saturating_add(size);
        packed = packed.saturating_add(1);
    }
    packed
}

/// Borrowed state views for [`build_fixed_parameters_with`].
struct FixedInputs<'a> {
    exits: Option<&'a super::exit::OwedExits>,
    rejoins: Option<&'a super::exit::OwedRejoins>,
    set_downs: Option<&'a super::close::OwedSetDowns>,
    closes: Option<&'a super::admit::OwedCloses>,
    book: Option<&'a super::FieldBook>,
}

/// **Build actor-scoped, strictly bounded `fixed_parameters` (`<= 48 KiB`)** (CRIT-18):
/// includes ONLY exits, rejoins, set-downs, closes, and carried minds owed by `actor`.
/// When an owed close's carried minds do not all fit beside its close entry, emits a carry-only
/// chunk for that site (`carried[site]` populated, `closing[site]` withheld) so `op = "carry"`
/// lays minds in chunks before `op = "close"` demotes the site.
fn build_fixed_parameters(w: &World, actor: ObjId) -> (Value, BTreeSet<String>) {
    build_fixed_parameters_with(
        w,
        actor,
        FixedInputs {
            exits: w.module_state::<super::exit::OwedExits>(),
            rejoins: w.module_state::<super::exit::OwedRejoins>(),
            set_downs: w.module_state::<super::close::OwedSetDowns>(),
            closes: w.module_state::<super::admit::OwedCloses>(),
            book: w.module_state::<super::FieldBook>(),
        },
    )
}

fn build_fixed_parameters_with(
    w: &World,
    actor: ObjId,
    inputs: FixedInputs<'_>,
) -> (Value, BTreeSet<String>) {
    let now = w.world_ic_now().at;
    let base = || {
        json!({
            "now": now,
            "closing": {},
            "exiting": {},
            "rejoining": {},
            "setting_down": {},
            "carried": {},
        })
    };
    let mut fixed = base();
    let mut priority_sites = BTreeSet::new();
    {
        let mut with_sides = fixed.clone();
        crate::surface::site_record::fix_sides(w, &mut with_sides);
        if fits_budget(&with_sides, MAX_OFFER_FIXED_BYTES) {
            fixed = with_sides;
        }
    }
    // 1. Owed exits for `actor` only.
    let mut exit_sites: Vec<CanonicalId> = Vec::new();
    if let Some(owed) = inputs.exits {
        for (site, units) in &owed.0 {
            let mine: Vec<u32> = units
                .iter()
                .filter(|(_, owner)| **owner == actor)
                .map(|(unit, _)| unit.0)
                .collect();
            if mine.is_empty() {
                continue;
            }
            let site_text = site.to_string();
            if try_insert_map_entry(&mut fixed, "exiting", &site_text, json!(mine)) {
                priority_sites.insert(site_text);
                exit_sites.push(site.clone());
            }
        }
    }
    // 2. Owed rejoins for `actor` only.
    if let Some(owed) = inputs.rejoins {
        for (site, units) in &owed.0 {
            let mine: Vec<u32> = units
                .iter()
                .filter(|(_, owner)| **owner == actor)
                .map(|(unit, _)| unit.0)
                .collect();
            if mine.is_empty() {
                continue;
            }
            let site_text = site.to_string();
            if try_insert_map_entry(&mut fixed, "rejoining", &site_text, json!(mine)) {
                priority_sites.insert(site_text);
            }
        }
    }
    // 3. Owed set-downs for `actor` only.
    if let Some(owed) = inputs.set_downs {
        for (site, rows) in &owed.0 {
            let mine: Vec<&LeftUnit> = rows
                .values()
                .filter(|(owner, _)| *owner == actor)
                .map(|(_, row)| row)
                .collect();
            if mine.is_empty() {
                continue;
            }
            let site_text = site.to_string();
            if try_insert_map_entry(&mut fixed, "setting_down", &site_text, json!(mine)) {
                priority_sites.insert(site_text);
            }
        }
    }
    // 4. Owed closes for `actor` only, with carried-mind chunking when needed.
    let mut carried_bytes = 0_usize;
    let mut carried_sites = BTreeSet::new();
    let mut closing_count = 0_usize;
    if let Some(owed) = inputs.closes {
        for (site, close) in &owed.0 {
            if close.actor != actor {
                continue;
            }
            let site_text = site.to_string();
            let allow_degrade = closing_count == 0;
            let live_carried: &[super::record::CarriedRow] = inputs
                .book
                .and_then(|book| book.hosted(site))
                .map_or(&[], |hosted| hosted.live.carried.as_slice());
            let total_minds = live_carried.len().saturating_add(close.minds.len());
            if total_minds == 0 {
                if fit_close_entry(&mut fixed, &site_text, close, allow_degrade) {
                    closing_count = closing_count.saturating_add(1);
                    priority_sites.insert(site_text);
                }
                continue;
            }
            // Try fitting both `closing[site]` and ALL carried minds for `site` together.
            let mut together = fixed.clone();
            let mut together_carried = carried_bytes;
            let close_ok = fit_close_entry(&mut together, &site_text, close, allow_degrade);
            let packed_together = if close_ok {
                pack_carried_for_site(
                    &mut together,
                    &mut together_carried,
                    &site_text,
                    live_carried.iter().chain(close.minds.iter()),
                )
            } else {
                0
            };
            if close_ok && packed_together == total_minds {
                fixed = together;
                carried_bytes = together_carried;
                closing_count = closing_count.saturating_add(1);
                carried_sites.insert(site_text.clone());
                priority_sites.insert(site_text);
                continue;
            }
            // Not all carried minds fit beside `closing[site]`: emit a carry-only chunk for this
            // site (without `closing[site]`) so `op = "carry"` drains minds before `op = "close"`.
            let packed_chunk = pack_carried_for_site(
                &mut fixed,
                &mut carried_bytes,
                &site_text,
                live_carried.iter().chain(close.minds.iter()),
            );
            if packed_chunk > 0 {
                carried_sites.insert(site_text.clone());
                priority_sites.insert(site_text);
            } else if close_ok {
                // No carried mind could fit even alone (e.g., all rows oversized); still close.
                fixed = together;
                carried_bytes = together_carried;
                closing_count = closing_count.saturating_add(1);
                priority_sites.insert(site_text);
            }
        }
    }
    // 5. Carried minds for `actor`'s owed exits on live running fields.
    if let Some(book) = inputs.book {
        for site in &exit_sites {
            let site_text = site.to_string();
            if carried_sites.contains(&site_text) {
                continue;
            }
            let Some(hosted) = book.hosted(site) else {
                continue;
            };
            if hosted.live.carried.is_empty() {
                continue;
            }
            let _ = pack_carried_for_site(
                &mut fixed,
                &mut carried_bytes,
                &site_text,
                hosted.live.carried.iter(),
            );
        }
    }
    // Defensive backstop: never return a value exceeding `MAX_OFFER_FIXED_BYTES`.
    if !fits_budget(&fixed, MAX_OFFER_FIXED_BYTES) {
        tracing::error!(
            actor = actor.0,
            "tacsim: field offer fixed parameters exceeded 48 KiB after packing; degrading to open-only offer"
        );
        return (base(), BTreeSet::new());
    }
    (fixed, priority_sites)
}

/// **Whether `close` on `site` must run an `op = "carry"` chunk before `op = "close"`**
/// (CRIT-18): true iff `build_fixed_parameters` withheld `closing[site]` and packed a non-empty
/// `carried[site]` chunk instead.
#[must_use]
pub(crate) fn close_needs_carry_chunk(
    w: &World,
    site: &CanonicalId,
    close: &super::admit::OwedClose,
) -> bool {
    if close.minds.is_empty() {
        return false;
    }
    let (fixed, _) = build_fixed_parameters(w, close.actor);
    fixed_needs_carry_chunk(&fixed, site)
}

fn fixed_needs_carry_chunk(fixed: &Value, site: &CanonicalId) -> bool {
    let site_text = site.as_str();
    let has_close = fixed
        .get("closing")
        .and_then(Value::as_object)
        .is_some_and(|map| map.contains_key(site_text));
    let has_carried = fixed
        .get("carried")
        .and_then(Value::as_object)
        .and_then(|map| map.get(site_text))
        .and_then(Value::as_array)
        .is_some_and(|rows| !rows.is_empty());
    !has_close && has_carried
}

impl ActionProvider for FieldProvider {
    fn validate_invocation(&self, invocation: &ActionInvocation) -> Result<(), ActionError> {
        let parameters = parameters(invocation)?;
        parameters
            .get("site")
            .and_then(Value::as_str)
            .and_then(SiteKey::parse)
            .ok_or_else(|| invalid("a field write names a site"))?;
        parameters
            .get("op")
            .and_then(Value::as_str)
            .filter(|op| {
                matches!(
                    *op,
                    "open" | "close" | "exit" | "rejoin" | "set_down" | "carry"
                )
            })
            .ok_or_else(|| {
                invalid("a field write opens, closes, exits, rejoins, sets down or carries minds")
            })?;
        Ok(())
    }

    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &mush_core::LegalActionQuery,
        engine: &mush_core::Engine,
    ) -> Result<Option<ProviderOfferDraft>, ActionError> {
        if !crate::surface::act::self_query(action, &self.action, query) {
            return Ok(None);
        }
        let w: &World = &engine.world;
        if !w
            .module_state::<crate::TacsimStore>()
            .is_some_and(crate::TacsimStore::enabled)
        {
            return Ok(None);
        }
        let all_sites: Vec<String> = crate::surface::chart::lane_sites(w)
            .iter()
            .map(|site| site.entry.to_string())
            .collect();
        if all_sites.is_empty() {
            return Ok(None);
        }
        let (fixed, priority_sites) = build_fixed_parameters(w, query.actor());
        // Bound choice list to `MAX_PROJECTED_ACTION_CHOICES` (prioritizing active owed sites)
        // so schema construction never fails globally.
        let mut sites: Vec<String> = Vec::new();
        for site in &all_sites {
            if priority_sites.contains(site)
                && sites.len() < mush_core::MAX_PROJECTED_ACTION_CHOICES
            {
                sites.push(site.clone());
            }
        }
        for site in all_sites {
            if sites.len() >= mush_core::MAX_PROJECTED_ACTION_CHOICES {
                break;
            }
            if !priority_sites.contains(&site) {
                sites.push(site);
            }
        }
        let integer =
            |minimum: i64, maximum: i64| ProjectedParameterKind::Integer { minimum, maximum };
        let text = ProjectedParameterKind::Text {
            maximum_bytes: MAX_LIST_BYTES,
        };
        let Ok(parameters) = (|| -> Result<Vec<mush_core::ProjectedParameter>, ActionError> {
            Ok(vec![
                crate::surface::act::parameter(
                    "site",
                    true,
                    ProjectedParameterKind::Choice { choices: sites },
                )?,
                crate::surface::act::parameter(
                    "op",
                    true,
                    ProjectedParameterKind::Choice {
                        choices: vec![
                            "open".to_owned(),
                            "close".to_owned(),
                            "exit".to_owned(),
                            "rejoin".to_owned(),
                            "set_down".to_owned(),
                            "carry".to_owned(),
                        ],
                    },
                )?,
                crate::surface::act::parameter(
                    "by",
                    false,
                    ProjectedParameterKind::Choice {
                        choices: vec!["mark".to_owned(), "fire".to_owned()],
                    },
                )?,
                crate::surface::act::parameter("from", false, integer(1, i64::from(u32::MAX)))?,
                crate::surface::act::parameter("at", false, integer(1, i64::from(u32::MAX)))?,
                crate::surface::act::parameter("rigs", false, text)?,
                crate::surface::act::parameter("unit", false, integer(1, i64::from(u32::MAX)))?,
            ])
        })() else {
            tracing::error!(
                "tacsim: field offer parameter schema failed to build; degrading gracefully"
            );
            return Ok(None);
        };
        match crate::surface::act::offer_draft(
            action,
            engine,
            fixed,
            parameters,
            (
                "tacsim-field",
                "Open, close or leave a field",
                "The IC field host: promote a site by a leader's mark or a fire order, record a unit's exit at its carrier's ramp, or demote it.",
            ),
        ) {
            Ok(draft) => Ok(Some(draft)),
            Err(error) => {
                tracing::error!(%error, "tacsim: field offer draft failed; degrading gracefully to avoid SelfActor lockout");
                Ok(None)
            }
        }
    }

    /// Doc 36 cut GC.3: the close takes its preparation context, through which a close-destroyed
    /// epoch's line is appended in this act's own bundle.
    fn prepare_authoritative_with_context(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        context: &mut mush_core::PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        self.validate_invocation(invocation)?;
        let parameters = parameters(invocation)?;
        let now = int(parameters, "now")
            .ok_or_else(|| invalid("a field write's offer fixes the instant"))?;
        let key = parameters
            .get("site")
            .and_then(Value::as_str)
            .and_then(SiteKey::parse)
            .ok_or_else(|| invalid("a field write names a site"))?;
        let op = parameters
            .get("op")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("a field write opens or closes"))?;
        if op == "carry" {
            let has_rows = parameters
                .get("carried")
                .and_then(|carried| carried.get(key.0.as_str()))
                .and_then(Value::as_array)
                .is_some_and(|rows| !rows.is_empty());
            if !has_rows {
                return Err(rejected(
                    "only the field host carries minds to their epoch records",
                ));
            }
            self.lay_minds(txn, parameters, &key)?;
            let outcome = RulesValue::new(
                self.outcome.clone(),
                json!({
                    "actor": invocation.actor().0,
                    "site": key.key_text(),
                    "said": "carried minds laid on their epoch records",
                }),
            )
            .map_err(|_| {
                port_error(
                    ActionErrorCode::BoundsExceeded,
                    "the field's outcome exceeds canonical bounds",
                )
            })?;
            return Ok(AuthoritativeActionOutput::new(outcome)
                .with_eligible_witnesses(vec![invocation.principal().clone()]));
        }
        let found = txn
            .get(&self.sites, &key)
            .map_err(|_| integrity("the site records are unavailable"))?
            .cloned();
        if found.as_ref().is_some_and(|record| record.key() != key) {
            return Err(integrity("a site record disagrees with its own key"));
        }
        let existed = found.is_some();
        let mut record =
            found.unwrap_or_else(|| crate::surface::site_record::fixed_record(parameters, &key.0));
        let said = match op {
            "open" => self.open(txn, &mut record, parameters, (invocation.actor(), now))?,
            "exit" => {
                let unit = obj(parameters, "unit")?;
                let owed = parameters
                    .get("exiting")
                    .and_then(|exiting| exiting.get(key.0.as_str()))
                    .and_then(Value::as_array)
                    .is_some_and(|units| {
                        units.iter().any(|n| n.as_u64() == Some(u64::from(unit.0)))
                    });
                if !owed {
                    return Err(rejected("only the field host writes a unit out of a field"));
                }
                record.exit(unit).map_err(|why| rejected(why.sentence()))?;
                self.lay_minds(txn, parameters, &key)?;
                format!("#{} left field {} at the ramp", unit.0, record.ordinal)
            }
            // Doc 40 (the ramp exit's boarding window; the 38b review's F-15).
            "rejoin" => {
                let unit = obj(parameters, "unit")?;
                let owed = parameters
                    .get("rejoining")
                    .and_then(|rejoining| rejoining.get(key.0.as_str()))
                    .and_then(Value::as_array)
                    .is_some_and(|units| {
                        units.iter().any(|n| n.as_u64() == Some(u64::from(unit.0)))
                    });
                if !owed {
                    return Err(rejected(
                        "only the field host writes a unit back into a field",
                    ));
                }
                record.rejoin(unit);
                format!("#{} is back in field {}", unit.0, record.ordinal)
            }
            // Doc 41's RS.9a follow-up: a carrier wrecked while docked, outside a field, indexed
            // where the host set her down on her pad.
            "set_down" => {
                let unit = obj(parameters, "unit")?;
                let row: LeftUnit = parameters
                    .get("setting_down")
                    .and_then(|owed| owed.get(key.0.as_str()))
                    .and_then(Value::as_array)
                    .and_then(|rows| {
                        rows.iter().find(|row| {
                            row.get("unit").and_then(Value::as_u64) == Some(u64::from(unit.0))
                        })
                    })
                    .cloned()
                    .map(serde_json::from_value)
                    .transpose()
                    .map_err(|_| invalid("a set-down's row is a left unit"))?
                    .ok_or_else(|| rejected("only the field host sets a wrecked carrier down"))?;
                if record.open_field().is_some() {
                    return Err(rejected(
                        "a field is open on the site; its close indexes her",
                    ));
                }
                record
                    .leave_unit(row)
                    .map_err(|why| rejected(why.sentence()))?;
                format!("#{} is set down on her pad, a wreck", unit.0)
            }
            _ => {
                let owed = parameters
                    .get("closing")
                    .and_then(|closing| closing.get(key.0.as_str()))
                    .ok_or_else(|| rejected("only the field host closes a field"))?;
                let ordinal = owed
                    .get("ordinal")
                    .and_then(Value::as_u64)
                    .and_then(|n| u32::try_from(n).ok())
                    .ok_or_else(|| invalid("a field's close names its ordinal"))?;
                if record.open_field().map(|field| field.ordinal) != Some(ordinal) {
                    return Err(rejected("that field is not the one open on the site"));
                }
                let units: Vec<LeftUnit> = owed
                    .get("units")
                    .cloned()
                    .map(serde_json::from_value)
                    .transpose()
                    .map_err(|_| invalid("a field's close lists the units it leaves"))?
                    .unwrap_or_default();
                let field = record
                    .demote(now, units)
                    .map_err(|why| rejected(why.sentence()))?;
                // Doc 32 cut CF.4 (Q-CF-28): each principal learns what their party saw, from the
                // host's own rows fixed into the offer.
                let known: Vec<super::known::Learned> = owed
                    .get("known")
                    .cloned()
                    .map(serde_json::from_value)
                    .transpose()
                    .map_err(|_| invalid("a field's close lists what its parties learned"))?
                    .unwrap_or_default();
                // Doc 36 cut GC.3 (Q-36-18): the remains, before what was learned of them.
                let remains: Vec<LeftRemains> = owed
                    .get("remains")
                    .cloned()
                    .map(serde_json::from_value)
                    .transpose()
                    .map_err(|_| invalid("a field's close lists the remains it leaves"))?
                    .unwrap_or_default();
                self.lay_minds(txn, parameters, &key)?;
                // Doc 38a F-23.1: a bay rig a wrecked carrier took down under its claim.
                let wrecked: Vec<ObjId> = owed
                    .get("wrecked")
                    .cloned()
                    .map(serde_json::from_value)
                    .transpose()
                    .map_err(|_| invalid("a field's close lists the bay rigs it wrecked"))?
                    .unwrap_or_default();
                for rig in wrecked {
                    self.close_wrecked_claim(txn, rig)?;
                }
                for left in &remains {
                    self.close_destroyed(txn, context, &mut record, (left, now))?;
                }
                for row in &known {
                    record.learn(row.principal, &row.items, |cell| row.swept(cell), now);
                }
                format!("field {} closed", field.ordinal)
            }
        };
        record
            .check()
            .map_err(|_| integrity("the site record is out of bounds"))?;
        if existed {
            txn.replace(&self.sites, &key, record)
                .map_err(|_| integrity("the site record did not stage"))?;
        } else {
            txn.insert(&self.sites, &key, record)
                .map_err(|_| integrity("the site record did not stage"))?;
        }
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({ "actor": invocation.actor().0, "site": key.key_text(), "said": said }),
        )
        .map_err(|_| {
            port_error(
                ActionErrorCode::BoundsExceeded,
                "the field's outcome exceeds canonical bounds",
            )
        })?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::excursion::{Act, ActParams, Charge, LegWaypoint};

    fn cid(text: &str) -> CanonicalId {
        CanonicalId::new(text).unwrap_or_else(|_| panic!("{text} is canonical"))
    }

    /// A rig out on the ground, 100 s into a 200 s drive east from (10, 10) to (30, 10) that
    /// reserved 2,000 cu.
    /// **Doc 38a F-23.1: a bay rig its wrecked carrier took down owes its claim `Destroyed`.**
    /// An open claim closes `Destroyed`, its queued launch cancelled; an opening claim, or none,
    /// is left alone.
    #[test]
    fn f23_1_a_wrecked_bay_rig_owes_its_claim_destroyed() {
        use crate::surface::excursion::{ClaimCloseKind, ClaimCursor};
        let open = mush_core::CommitToken::from_bytes([7; 16]);
        let mut record = RigRecord::fresh_unread(ObjId(7));
        record.claim = Some(Box::new(ClaimCursor::Open {
            open,
            operator: ObjId(42),
        }));
        assert!(wreck_claim(&mut record));
        assert_eq!(
            record.claim.map(|claim| *claim),
            Some(ClaimCursor::Closing {
                open,
                kind: ClaimCloseKind::Destroyed,
                operator: ObjId(42),
            })
        );
        let mut bare = RigRecord::fresh_unread(ObjId(8));
        assert!(!wreck_claim(&mut bare));
        assert!(bare.claim.is_none());
    }

    fn driving() -> RigRecord {
        let mut record = RigRecord::fresh_unread(ObjId(7));
        record.charge = Charge {
            value_cu: 50_000,
            at_ic: 900,
            in_bay: false,
        };
        record.act = Some(Act {
            ordinal: 3,
            kind: ActKind::Drive,
            operator: ObjId(42),
            hull: ObjId(500),
            slot: 1,
            site: cid("ahuna_port"),
            started_ic: 1_000,
            due_ic: 1_200,
            reserved_cu: 2_000,
            params: ActParams::Drive {
                from_bay: false,
                target: "30,10".to_owned(),
                to: [30, 10],
                waypoints: vec![
                    LegWaypoint {
                        x_cm: 10_500,
                        y_cm: 10_500,
                        at_s: 0,
                    },
                    LegWaypoint {
                        x_cm: 30_500,
                        y_cm: 10_500,
                        at_s: 200,
                    },
                ],
                secs: 200,
                cost: 2_000,
                metres: 200,
                heading_mdeg: 90_000,
                side: crate::surface::ground::SITE_CELLS,
            },
            by: None,
            seal: None,
        });
        record
    }

    #[test]
    fn a_drive_stops_where_its_leg_reached_and_pays_for_what_it_drove() {
        let mut record = driving();
        let pose = settle(&mut record, &cid("ahuna_port"), 1_100).expect("the drive stops");
        assert_eq!((pose.x_cm, pose.y_cm), (20_500, 10_500), "half way");
        assert_eq!(pose.heading_mdeg, 90_000);
        assert!(record.act.is_none(), "the remainder is canceled");
        assert_eq!(
            record.charge.value_cu, 49_000,
            "half the reservation is spent"
        );
        assert!(matches!(
            record.last.as_ref().map(|last| last.result),
            Some(ActResult::Refused {
                code: RefusedCode::FieldOpened
            })
        ));
        assert!(!handoff_pending(&record, &cid("ahuna_port")));
    }

    #[test]
    fn another_site_and_a_ramp_handoff_are_left_alone() {
        let mut record = driving();
        let before = record.clone();
        assert_eq!(settle(&mut record, &cid("vesta_port"), 1_100), None);
        assert_eq!(
            record, before,
            "a drive on another site is not this field's"
        );
        let mut launching = driving();
        if let Some(act) = launching.act.as_mut() {
            act.kind = ActKind::Launch;
            act.params = ActParams::None {};
        }
        let kept = launching.clone();
        assert_eq!(settle(&mut launching, &cid("ahuna_port"), 1_100), None);
        assert_eq!(
            launching, kept,
            "a launch on the ramp finishes into the field"
        );
        assert!(handoff_pending(&launching, &cid("ahuna_port")));
    }

    /// **CRIT-18**: `Learned::clamp` caps `eyes` at `MAX_LEARNED_EYES` and `items` at
    /// `MAX_KNOWN_ITEMS` so a large battle cannot produce unbounded eye lists.
    #[test]
    fn crit18_learned_clamp_caps_eyes_and_items() {
        let mut row = super::super::known::Learned {
            principal: ObjId(7),
            items: (0..100)
                .map(|i| {
                    crate::surface::site_record::KnownItem(
                        crate::surface::site_record::KnownWhat::Wreck,
                        i,
                        0,
                    )
                })
                .collect(),
            eyes: (0..500).map(|i| (i / 2, i % 17)).collect(),
        };
        row.clamp();
        assert_eq!(
            row.items.len(),
            crate::surface::site_record::MAX_KNOWN_ITEMS
        );
        assert_eq!(row.eyes.len(), super::super::known::MAX_LEARNED_EYES);
    }

    /// **CRIT-18**: `build_fixed_parameters` scopes owed closes/carried minds to the owning actor,
    /// keeps canonical JSON strictly `<= 48 KiB`, chunks carried minds via `close_needs_carry_chunk`,
    /// and degrades oversized `known` payloads cleanly without overflowing 48 KiB.
    #[test]
    fn crit18_offer_is_actor_scoped_bounded_and_chunks_carried_minds() {
        let w = World::new_minimal();
        let site = cid("ahuna_port");
        let owner = ObjId(10);
        let bystander = ObjId(99);

        // Build 400 `Learned` rows (well over 64 KiB if emitted verbatim) plus 12 carried minds
        // totaling > 36 KiB.
        let huge_known: Vec<super::super::known::Learned> = (1..=400_u32)
            .map(|p| super::super::known::Learned {
                principal: ObjId(p),
                items: (0..20)
                    .map(|i| {
                        crate::surface::site_record::KnownItem(
                            crate::surface::site_record::KnownWhat::Casualty,
                            i,
                            i64::from(p),
                        )
                    })
                    .collect(),
                eyes: (0..120).map(|i| (i, i64::from(p))).collect(),
            })
            .collect();
        let mem_payload = format!("\"{}\"", "a".repeat(3_500)).into_bytes();
        let many_minds: Vec<super::super::record::CarriedRow> = (1..=12_u32)
            .map(|u| super::super::record::CarriedRow {
                unit: ObjId(100 + u),
                epoch: crate::arena::mind::SortieId::Packed(u64::from(u)),
                mind: crate::arena::mind::CarriedMind {
                    seal_digest: [7; 32],
                    think_seq: u64::from(u),
                    memory: mem_payload.clone(),
                    dropped: None,
                },
            })
            .collect();
        let close = super::super::admit::OwedClose {
            actor: owner,
            ordinal: 3,
            units: Vec::new(),
            known: huge_known,
            remains: Vec::new(),
            minds: many_minds,
            wrecked: Vec::new(),
        };
        let mut closes = super::super::admit::OwedCloses::default();
        closes.0.insert(site.clone(), close);

        let build = |actor: ObjId, closes: &super::super::admit::OwedCloses| {
            build_fixed_parameters_with(
                &w,
                actor,
                FixedInputs {
                    exits: None,
                    rejoins: None,
                    set_downs: None,
                    closes: Some(closes),
                    book: None,
                },
            )
        };

        // 1. Bystander sees empty closing/carried and tiny canonical JSON (no global lockout).
        let (bystander_fixed, _) = build(bystander, &closes);
        let bystander_bytes = mush_core::mechanics::canonical_json_bytes_with_limit(
            &bystander_fixed,
            MAX_OFFER_FIXED_BYTES,
        )
        .expect("bystander fits 48 KiB");
        assert!(
            bystander_bytes.len() < 1024,
            "bystander offer should be tiny"
        );
        assert!(
            bystander_fixed["closing"]
                .as_object()
                .expect("closing")
                .is_empty()
        );
        assert!(
            bystander_fixed["carried"]
                .as_object()
                .expect("carried")
                .is_empty()
        );

        // 2. Owner with > 24 KiB of carried minds gets a carry-only chunk strictly <= 48 KiB.
        let (owner_carry, _) = build(owner, &closes);
        assert!(fixed_needs_carry_chunk(&owner_carry, &site));
        let carry_bytes = mush_core::mechanics::canonical_json_bytes_with_limit(
            &owner_carry,
            MAX_OFFER_FIXED_BYTES,
        )
        .expect("carry chunk fits 48 KiB");
        assert!(carry_bytes.len() <= MAX_OFFER_FIXED_BYTES);
        assert!(
            !owner_carry["closing"]
                .as_object()
                .expect("closing")
                .contains_key("ahuna_port")
        );
        let carried_chunk = owner_carry["carried"]["ahuna_port"]
            .as_array()
            .expect("carried chunk");
        assert!(!carried_chunk.is_empty() && carried_chunk.len() < 12);

        // 3. Once carried minds are drained, the close offer degrades `known` cleanly to stay
        // strictly <= 48 KiB while still closing the site.
        closes.0.get_mut(&site).expect("site").minds.clear();
        let (owner_close, _) = build(owner, &closes);
        assert!(!fixed_needs_carry_chunk(&owner_close, &site));
        let close_bytes = mush_core::mechanics::canonical_json_bytes_with_limit(
            &owner_close,
            MAX_OFFER_FIXED_BYTES,
        )
        .expect("degraded close fits 48 KiB");
        assert!(close_bytes.len() <= MAX_OFFER_FIXED_BYTES);
        assert_eq!(owner_close["closing"]["ahuna_port"]["ordinal"], 3);
    }

    /// **CRIT-18**: `sanitize_close` clamps restored tombstones and drops single oversized minds
    /// so a restart can never wedge `project_legal_actions`.
    #[test]
    fn crit18_sanitize_close_clamps_eyes_and_drops_oversized_mind() {
        let mut close = super::super::admit::OwedClose {
            actor: ObjId(10),
            ordinal: 1,
            units: Vec::new(),
            known: vec![super::super::known::Learned {
                principal: ObjId(1),
                items: Vec::new(),
                eyes: (0..300).map(|i| (i, 0)).collect(),
            }],
            remains: Vec::new(),
            minds: vec![super::super::record::CarriedRow {
                unit: ObjId(77),
                epoch: crate::arena::mind::SortieId::Packed(1),
                mind: crate::arena::mind::CarriedMind {
                    seal_digest: [1; 32],
                    think_seq: 1,
                    memory: format!("\"{}\"", "z".repeat(MAX_CARRIED_BYTES + 1024)).into_bytes(),
                    dropped: None,
                },
            }],
            wrecked: Vec::new(),
        };
        super::super::admit::sanitize_close(&mut close);
        assert_eq!(
            close.known[0].eyes.len(),
            super::super::known::MAX_LEARNED_EYES
        );
        assert!(close.minds.is_empty(), "single oversized mind is dropped");
    }
}
