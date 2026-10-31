//! **`tacsim_field`** — the field host's writes to the site record (doc 32 cut CF.2), one
//! authoritative action with two operations.
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
//! - **`close`** (Q-CF-20, Q-CF-35): the demotion. The record's unit index becomes every unit the
//!   field leaves on the ground at its field pose; the flag clears; the instant is kept.
//!
//! **Who may write.** `open` is offered to anybody and refused at the prepare on the record's own
//! state; the verbs (`+field/mark`, `+field/fire`) decide who leads. `close` closes only what the
//! host owes: the offer fixes the host's owed closes — each site's ordinal and the units it
//! leaves, from the host's RAM — into the invocation, so no player can demote a field, or write
//! a unit index, by submitting it.

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
    LeftUnit, PromotedBy, SettledRig, SiteKey, SitePose, SiteRecord,
};

/// The action.
pub const FIELD_ACTION: &str = "tacsim_field";
/// Its outcome.
pub const FIELD_OUTCOME: &str = "tacsim_field_result";
/// Its idempotency namespace: `tacsim.field/{actor}/{site}/{op}/{ordinal}/{now_ic}`.
pub const FIELD_KEY_PREFIX: &str = "tacsim.field/";
/// The most bytes an `open`'s rig list may carry (the offer's text bound).
pub const MAX_LIST_BYTES: usize = mush_core::MAX_PROJECTED_ACTION_TEXT_BYTES;

/// **`tacsim_field`**'s provider.
pub(crate) struct FieldProvider {
    pub action: DefinitionRef,
    pub outcome: DefinitionRef,
    pub sites: StateFamilyHandle<SiteKey, SiteRecord>,
    pub excursions: StateFamilyHandle<RigKey, RigRecord>,
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
            .filter(|op| matches!(*op, "open" | "close"))
            .ok_or_else(|| invalid("a field write opens or closes"))?;
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
        let sites: Vec<String> = crate::surface::chart::lane_sites(w)
            .iter()
            .map(|site| site.entry.to_string())
            .collect();
        if sites.is_empty() {
            return Ok(None);
        }
        // The host's owed closes, fixed into the invocation: only these may be closed, and only
        // with these units.
        let closing: Map<String, Value> = w
            .module_state::<super::admit::OwedCloses>()
            .map(|owed| {
                owed.0
                    .iter()
                    .map(|(site, close)| {
                        (
                            site.to_string(),
                            json!({ "ordinal": close.ordinal, "units": close.units, "known": close.known }),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let integer =
            |minimum: i64, maximum: i64| ProjectedParameterKind::Integer { minimum, maximum };
        let text = ProjectedParameterKind::Text {
            maximum_bytes: MAX_LIST_BYTES,
        };
        let parameters = vec![
            crate::surface::act::parameter(
                "site",
                true,
                ProjectedParameterKind::Choice { choices: sites },
            )?,
            crate::surface::act::parameter(
                "op",
                true,
                ProjectedParameterKind::Choice {
                    choices: vec!["open".to_owned(), "close".to_owned()],
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
        ];
        crate::surface::act::offer_draft(
            action,
            engine,
            json!({ "now": w.world_ic_now().at, "closing": closing }),
            parameters,
            (
                "tacsim-field",
                "Open or close a field",
                "The IC field host: promote a site by a leader's mark or a fire order, or demote it.",
            ),
        )
        .map(Some)
    }

    fn prepare_authoritative(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
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
        let found = txn
            .get(&self.sites, &key)
            .map_err(|_| integrity("the site records are unavailable"))?
            .cloned();
        if found.as_ref().is_some_and(|record| record.key() != key) {
            return Err(integrity("a site record disagrees with its own key"));
        }
        let existed = found.is_some();
        let mut record = found.unwrap_or_else(|| SiteRecord::new(key.0.clone()));
        let said = match op {
            "open" => self.open(txn, &mut record, parameters, (invocation.actor(), now))?,
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
        assert!(record.act.is_none(), "the remainder is cancelled");
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
}
