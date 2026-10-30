//! Record-only accepted order attribution for snapshot recovery. Never command
//! authority, a public mind view, or a combat state-hash input. Empty latest
//! groups remain as tombstones, so superseded held orders cannot regain a watch.
use super::{Arena, Controller, MindObjective};
use crate::deliberation::order::{Objective, Order, RangeBand};
use crate::deliberation::{CharacterRef, SquadId};
use mush_core::ObjId;
use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{DeserializeSeed, Error as _, SeqAccess, Visitor},
};
use sha2::{Digest as _, Sha256};
use std::{collections::BTreeMap, fmt, marker::PhantomData};

const GROUPS: usize = 2 * crate::deliberation::squad::MAX_SQUADS_PER_SIDE;
const MEMBERS: usize = crate::deliberation::SQUAD_SIZE_MAX as usize;
/// Complete worst-width JSON record allowance, including nested member pairs.
/// No source, string chosen by a caller, or unbounded payload is retained.
pub(super) const SNAPSHOT_RESERVE: u64 = (GROUPS as u64) * (1024 + MEMBERS as u64 * 64) + 2;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Pairs<K, V, const N: usize>(BTreeMap<K, V>);
impl<K, V, const N: usize> Default for Pairs<K, V, N> {
    fn default() -> Self {
        Self(BTreeMap::new())
    }
}
impl<K: Serialize, V: Serialize, const N: usize> Serialize for Pairs<K, V, N> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(self.0.iter())
    }
}
impl<'de, K: Deserialize<'de> + Ord, V: Deserialize<'de>, const N: usize> Deserialize<'de>
    for Pairs<K, V, N>
{
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Bounded<K, V, const N: usize>(PhantomData<(K, V)>);
        struct Overflow;
        impl<'de> DeserializeSeed<'de> for Overflow {
            type Value = ();
            fn deserialize<D: Deserializer<'de>>(self, _: D) -> Result<(), D::Error> {
                Err(D::Error::custom("order watch pair limit"))
            }
        }
        impl<'de, K: Deserialize<'de> + Ord, V: Deserialize<'de>, const N: usize> Visitor<'de>
            for Bounded<K, V, N>
        {
            type Value = Pairs<K, V, N>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("bounded unique order watch pairs")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                if seq.size_hint().is_some_and(|n| n > N) {
                    return Err(A::Error::custom("order watch pair limit"));
                }
                let mut rows = BTreeMap::new();
                while rows.len() < N {
                    let Some((key, value)) = seq.next_element::<(K, V)>()? else {
                        return Ok(Pairs(rows));
                    };
                    if rows.insert(key, value).is_some() {
                        return Err(A::Error::custom("duplicate order watch pair"));
                    }
                }
                // Reject the next element before deserializing or allocating it.
                let _ = seq.next_element_seed(Overflow)?;
                Ok(Pairs(rows))
            }
        }
        d.deserialize_seq(Bounded::<K, V, N>(PhantomData))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Word {
    Hold,
    Advance,
    Withdraw,
    Engage,
    Screen,
    Support,
}
impl Word {
    fn of(o: &Objective) -> Self {
        match o {
            Objective::Hold => Self::Hold,
            Objective::Advance { .. } => Self::Advance,
            Objective::Withdraw => Self::Withdraw,
            Objective::Engage { .. } => Self::Engage,
            Objective::Screen { .. } => Self::Screen,
            Objective::Support { .. } => Self::Support,
        }
    }
    pub(crate) fn text(self) -> &'static str {
        match self {
            Self::Hold => "hold",
            Self::Advance => "advance",
            Self::Withdraw => "withdraw",
            Self::Engage => "engage",
            Self::Screen => "screen",
            Self::Support => "support",
        }
    }
    fn matches(self, held: &MindObjective, charge: Option<ObjId>) -> bool {
        self.text() == held.as_str()
            || (self == Self::Support
                && matches!(held, MindObjective::Engage { .. })
                && charge.is_none())
    }
}
fn required_option<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(d)
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Group {
    request: [u8; 32],
    by: CharacterRef,
    #[serde(deserialize_with = "required_option")]
    issuer: Option<ObjId>,
    objective: Word,
    members: Pairs<ObjId, i64, MEMBERS>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct OrderWatches(Pairs<SquadId, Group, GROUPS>);

/// Complete versioned request identity without a temporary serialized payload.
fn request(order: &Order) -> [u8; 32] {
    let Order {
        squad,
        objective,
        target,
        horizon_ms,
        constraints,
        ttl_ms,
        issued_ms,
        issuer,
        by,
        spot,
    } = order;
    let mut h = Sha256::new();
    h.update(b"tacsim.accepted-order-watch.v1\0");
    fn hash_squad(h: &mut Sha256, s: SquadId) {
        h.update([match s.team {
            super::PartyId::RED => 0,
            super::PartyId::BLUE => 1,
            other => other.code(),
        }]);
        h.update(s.index.to_le_bytes());
    }
    fn optional(h: &mut Sha256, n: Option<u64>) {
        h.update([u8::from(n.is_some())]);
        if let Some(n) = n {
            h.update(n.to_le_bytes());
        }
    }
    hash_squad(&mut h, *squad);
    h.update([objective.code()]);
    match objective {
        Objective::Advance { to } => {
            h.update(to.0.to_le_bytes());
            h.update(to.1.to_le_bytes());
        }
        Objective::Engage { target_hint } => optional(&mut h, target_hint.map(|id| id.0)),
        Objective::Screen { facing_mdeg } => h.update(facing_mdeg.to_le_bytes()),
        Objective::Support { of } => h.update(of.0.to_le_bytes()),
        Objective::Hold | Objective::Withdraw => {}
    }
    optional(&mut h, target.map(|id| id.0));
    h.update(horizon_ms.to_le_bytes());
    h.update([
        u8::from(constraints.hold_fire),
        match constraints.max_range_band {
            None => 0,
            Some(RangeBand::Close) => 1,
            Some(RangeBand::Effective) => 2,
            Some(RangeBand::Far) => 3,
        },
    ]);
    h.update(ttl_ms.to_le_bytes());
    h.update(issued_ms.to_le_bytes());
    optional(&mut h, issuer.map(|id| u64::from(id.0)));
    match by {
        CharacterRef::Character { id } => {
            h.update([0]);
            h.update(id.0.to_le_bytes());
        }
        CharacterRef::Persona { squad: s } => {
            h.update([1]);
            hash_squad(&mut h, *s);
        }
    }
    h.update([u8::from(spot.is_some())]);
    if let Some(spot) = spot {
        h.update((spot.len() as u64).to_le_bytes());
        h.update(spot.as_bytes());
    }
    h.finalize().into()
}

impl Arena {
    /// Called only after the native Order gates and held-row write succeeded.
    pub(super) fn note_accepted_order(&mut self, unit: ObjId, order: &Order, expires_ms: i64) {
        // The actual held row was replaced even when no squad watch applies.
        self.clear_order_watch_member(unit);
        let Some(roster) = self
            .squads
            .iter()
            .find(|s| s.id == order.squad && s.holds(unit))
        else {
            return;
        };
        if roster.units.len() > MEMBERS
            || (!self.order_watches.0.0.contains_key(&order.squad)
                && self.order_watches.0.0.len() >= GROUPS)
        {
            return;
        }
        let identity = request(order);
        let group = self
            .order_watches
            .0
            .0
            .entry(order.squad)
            .or_insert_with(|| Group {
                request: identity,
                by: order.by.clone(),
                issuer: order.issuer,
                objective: Word::of(&order.objective),
                members: Pairs::default(),
            });
        if group.request != identity {
            *group = Group {
                request: identity,
                by: order.by.clone(),
                issuer: order.issuer,
                objective: Word::of(&order.objective),
                members: Pairs::default(),
            };
        }
        group.members.0.insert(unit, expires_ms);
    }
    pub(super) fn clear_order_watch_member(&mut self, unit: ObjId) {
        for group in self.order_watches.0.0.values_mut() {
            group.members.0.remove(&unit);
        }
    }
    pub(super) fn validate_order_watches(&self) -> Result<(), super::SnapshotError> {
        let bad = || {
            super::SnapshotError::Malformed(
                "accepted order watch does not match held roster".into(),
            )
        };
        for (squad, group) in &self.order_watches.0.0 {
            let roster = self
                .squads
                .iter()
                .find(|s| s.id == *squad)
                .ok_or_else(bad)?;
            if roster.units.len() > MEMBERS
                || squad.index == 0
                || usize::from(squad.index) > crate::deliberation::squad::MAX_SQUADS_PER_SIDE
            {
                return Err(bad());
            }
            match &group.by {
                CharacterRef::Persona { squad: by } if by != squad || group.issuer.is_some() => {
                    return Err(bad());
                }
                CharacterRef::Character { id } if id.0 == 0 => return Err(bad()),
                _ => {}
            }
            if let Some(issuer) = group.issuer
                && !self.unit(issuer).is_some_and(|u| u.team == squad.team)
            {
                return Err(bad());
            }
            for (unit, expiry) in &group.members.0 {
                let own = self.unit(*unit).ok_or_else(bad)?;
                let mind = self.minds.get(unit).ok_or_else(bad)?;
                let held = mind.order.as_ref().ok_or_else(bad)?;
                if !roster.holds(*unit)
                    || own.team != squad.team
                    || own.controller != Controller::Mind
                    || held.squad != squad.to_string()
                    || held.expires_ms != *expiry
                    || mind.order_issuer != group.issuer
                    || !group.objective.matches(&held.objective, mind.order_charge)
                {
                    return Err(bad());
                }
            }
        }
        Ok(())
    }
    /// Actual latest live accepted groups only. Never searches superseded rows.
    pub(crate) fn recovered_order_watches(
        &self,
    ) -> Vec<(SquadId, CharacterRef, Option<ObjId>, &'static str, i64)> {
        self.order_watches
            .0
            .0
            .iter()
            .filter_map(|(squad, group)| {
                let expiry = group
                    .members
                    .0
                    .values()
                    .copied()
                    .filter(|expiry| *expiry > self.now_ms)
                    .max()?;
                Some((
                    *squad,
                    group.by.clone(),
                    group.issuer,
                    group.objective.text(),
                    expiry,
                ))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        Intent, IntentKind, IntentOutcome, IntentSource, MachineState, Snapshot, PartyId,
        apply_intent, state_hash,
        test_support::{arena_at_epoch, pose, tank},
    };
    use super::*;
    use crate::deliberation::{SquadRoster, order::Constraints};
    use serde_json::json;
    fn squad() -> SquadId {
        SquadId {
            team: PartyId::RED,
            index: 1,
        }
    }
    pub(crate) fn field() -> Arena {
        let mut arena = arena_at_epoch(
            vec![
                tank(ObjId(1), PartyId::RED, pose(1500, 1500)),
                tank(ObjId(2), PartyId::RED, pose(2500, 1500)),
                tank(ObjId(3), PartyId::BLUE, pose(6500, 6500)),
            ],
            1,
        );
        for id in [ObjId(1), ObjId(2)] {
            arena.set_controller(id, Controller::Mind, MachineState::Patrol);
        }
        arena.squads = vec![SquadRoster {
            id: squad(),
            commander: ObjId(1),
            units: vec![ObjId(1), ObjId(2)],
        }];
        arena
    }
    fn order(ttl_ms: u32) -> Order {
        Order {
            squad: squad(),
            objective: Objective::Hold,
            target: None,
            horizon_ms: ttl_ms,
            constraints: Constraints::default(),
            ttl_ms,
            issued_ms: 77,
            issuer: Some(ObjId(1)),
            by: CharacterRef::Character { id: ObjId(90) },
            spot: None,
        }
    }
    fn issue(arena: &mut Arena, unit: ObjId, order: &Order, seq: u64) -> IntentOutcome {
        let intent = Intent {
            unit,
            channel: super::super::Channel::Helm,
            epoch: arena.epochs.current(unit, super::super::Channel::Helm),
            seq,
            kind: IntentKind::Order(Box::new(order.clone())),
        };
        apply_intent(arena, &intent, IntentSource::Seat)
    }
    fn release(arena: &mut Arena, unit: ObjId) {
        let i = Intent {
            unit,
            channel: super::super::Channel::Helm,
            epoch: arena.epochs.current(unit, super::super::Channel::Helm),
            seq: 20,
            kind: IntentKind::ReleaseOrder {
                squad: squad(),
                issuer: Some(ObjId(1)),
            },
        };
        assert_eq!(
            apply_intent(arena, &i, IntentSource::Seat),
            IntentOutcome::AppliedContinuous
        );
    }
    fn resumed(arena: &Arena) -> Arena {
        let snapshot = Snapshot::encode(arena).expect("settled snapshot");
        Snapshot::decode(&snapshot.document, arena.definition.as_ref().clone())
            .expect("actual snapshot decode")
    }
    #[test]
    fn r4_native_fanout_latest_group_and_clear_tombstone_survive_snapshot() {
        let mut arena = field();
        let long = order(60_000);
        for id in [ObjId(1), ObjId(2)] {
            assert_eq!(
                issue(&mut arena, id, &long, 1),
                IntentOutcome::AppliedContinuous
            );
        }
        assert_eq!(arena.recovered_order_watches().len(), 1);
        assert_eq!(
            arena
                .order_watches
                .0
                .0
                .get(&squad())
                .unwrap()
                .members
                .0
                .len(),
            2
        );
        let mut short = order(1000);
        short.by = CharacterRef::Character { id: ObjId(91) };
        assert_eq!(
            issue(&mut arena, ObjId(2), &short, 2),
            IntentOutcome::AppliedContinuous
        );
        let again = resumed(&arena);
        assert_eq!(again.recovered_order_watches()[0].1, short.by);
        assert_eq!(again.recovered_order_watches()[0].4, 1000);
        release(&mut arena, ObjId(2));
        assert!(
            arena.minds.get(&ObjId(1)).unwrap().order.is_some(),
            "older native order remains"
        );
        let again = resumed(&arena);
        assert!(
            again.recovered_order_watches().is_empty(),
            "never resurrect older author"
        );
        assert_eq!(
            again.order_watches.0.0.len(),
            1,
            "empty latest tombstone retained"
        );
    }
    #[test]
    fn r4_partial_release_possess_and_refusal_preserve_exact_members() {
        let mut arena = field();
        let original = order(10000);
        for id in [ObjId(1), ObjId(2)] {
            assert_eq!(
                issue(&mut arena, id, &original, 1),
                IntentOutcome::AppliedContinuous
            );
        }
        release(&mut arena, ObjId(2));
        assert_eq!(resumed(&arena).recovered_order_watches().len(), 1);
        arena.set_controller(ObjId(1), Controller::Pilot, MachineState::Patrol);
        assert!(resumed(&arena).recovered_order_watches().is_empty());
        let before = arena.order_watches.clone();
        assert!(matches!(
            issue(&mut arena, ObjId(1), &order(60000), 2),
            IntentOutcome::Refused(_)
        ));
        assert_eq!(
            before, arena.order_watches,
            "refused offers never change attribution"
        );
    }
    #[test]
    fn r4_persona_and_self_support_record_original_word_without_combat_hash_changes() {
        let mut arena = field();
        let mut told = order(10000);
        told.by = CharacterRef::Persona { squad: squad() };
        told.issuer = None;
        told.objective = Objective::Support { of: ObjId(1) };
        assert_eq!(
            issue(&mut arena, ObjId(1), &told, 1),
            IntentOutcome::AppliedContinuous
        );
        assert!(matches!(
            arena
                .minds
                .get(&ObjId(1))
                .unwrap()
                .order
                .as_ref()
                .unwrap()
                .objective,
            MindObjective::Engage { .. }
        ));
        let before = state_hash(&arena);
        let mut decoded = resumed(&arena);
        assert_eq!(before, state_hash(&decoded));
        assert_eq!(decoded.recovered_order_watches()[0].3, "support");
        decoded.order_watches.0.0.get_mut(&squad()).unwrap().by =
            CharacterRef::Character { id: ObjId(99) };
        assert_eq!(before, state_hash(&decoded), "attribution is record-only");
        // Actual settled continuation: the carried record never changes a runner answer.
        for wall in (100..=2000).step_by(100) {
            let a = arena.step_settled(wall, &[], &[]);
            let b = decoded.step_settled(wall, &[], &[]);
            assert_eq!(a.ended, b.ended);
            assert_eq!(state_hash(&arena), state_hash(&decoded));
            assert_eq!(arena.ledger.digest(), decoded.ledger.digest());
        }
    }
    #[test]
    fn r4_native_expiry_clears_latest_but_never_resurrects_older_held_order() {
        let mut arena = field();
        let long = order(60000);
        let short = order(1000);
        assert_eq!(
            issue(&mut arena, ObjId(1), &long, 1),
            IntentOutcome::AppliedContinuous
        );
        assert_eq!(
            issue(&mut arena, ObjId(2), &short, 1),
            IntentOutcome::AppliedContinuous
        );
        for wall in (100..=6000).step_by(100) {
            arena.step_settled(wall, &[], &[]);
        }
        assert!(arena.now_ms >= 1000);
        assert!(arena.minds.get(&ObjId(1)).unwrap().order.is_some());
        assert!(arena.minds.get(&ObjId(2)).unwrap().order.is_none());
        assert!(resumed(&arena).recovered_order_watches().is_empty());
    }
    #[test]
    fn r4_strict_pairs_and_crossed_snapshot_members_refuse() {
        let mut arena = field();
        assert_eq!(
            issue(&mut arena, ObjId(1), &order(10000), 1),
            IntentOutcome::AppliedContinuous
        );
        let good = serde_json::to_value(&arena.order_watches).unwrap();
        for change in 0..9 {
            let mut bad = good.clone();
            match change {
                0 => {
                    let first = bad[0].clone();
                    bad.as_array_mut().unwrap().push(first);
                }
                1 => {
                    let member = bad[0][1]["members"][0].clone();
                    bad[0][1]["members"].as_array_mut().unwrap().push(member);
                }
                2 => bad[0][1]["objective"] = json!("unknown"),
                3 => {
                    bad[0][1].as_object_mut().unwrap().remove("issuer");
                }
                4 => bad[0][1]["members"][0][0] = json!(3),
                5 => bad[0][1]["members"][0][1] = json!(10001),
                6 => bad[0][0]["index"] = json!(2),
                7 => bad[0][1]["by"] = json!({"kind":"persona","squad":{"team":"blue","index":1}}),
                _ => bad[0][1]["issuer"] = json!(3),
            }
            // Recompute only the snapshot integrity digest: the held-row join
            // must reject coherent bytes too, not merely a stale checksum.
            let mut document = Snapshot::encode(&arena).unwrap().document;
            let mut state: serde_json::Value =
                serde_json::from_str(document["state"].as_str().unwrap()).unwrap();
            state["order_watches"] = bad.clone();
            let raw = serde_json::to_string(&state).unwrap();
            document["state_digest"] = json!(super::super::snapshot::digest_text(
                &Sha256::digest(raw.as_bytes()).into()
            ));
            document["state"] = json!(raw);
            assert!(
                Snapshot::decode(&document, arena.definition.as_ref().clone()).is_err(),
                "restamped crossed watch {change}"
            );
            if let Ok(rows) = serde_json::from_value::<OrderWatches>(bad) {
                let mut changed = resumed(&arena);
                changed.order_watches = rows;
                assert!(
                    Snapshot::encode(&changed).is_err(),
                    "crossed held metadata {change}"
                );
            }
        }
        let mut too_many = good[0][1].clone();
        too_many["members"] = json!((0..=MEMBERS).map(|n| (n, 10000)).collect::<Vec<_>>());
        assert!(serde_json::from_value::<OrderWatches>(json!([[squad(), too_many]])).is_err());
        let groups = (0..=GROUPS)
            .map(|n| json!([{"team":"red","index":n},good[0][1].clone()]))
            .collect::<Vec<_>>();
        assert!(serde_json::from_value::<OrderWatches>(json!(groups)).is_err());
        let mut missing = Snapshot::encode(&arena).unwrap().document;
        let mut state: serde_json::Value =
            serde_json::from_str(missing["state"].as_str().unwrap()).unwrap();
        state.as_object_mut().unwrap().remove("order_watches");
        let raw = serde_json::to_string(&state).unwrap();
        missing["state_digest"] = json!(super::super::snapshot::digest_text(
            &Sha256::digest(raw.as_bytes()).into()
        ));
        missing["state"] = json!(raw);
        assert!(Snapshot::decode(&missing, arena.definition.as_ref().clone()).is_err());
        let mut snapshot = Snapshot::encode(&arena).unwrap().document;
        snapshot["format"] = json!(15);
        assert!(matches!(
            Snapshot::decode(&snapshot, arena.definition.as_ref().clone()),
            Err(super::super::SnapshotError::Format { .. })
        ));
    }
    #[test]
    fn r4_support_charge_departure_and_native_authority_refusals_keep_tombstone() {
        let mut arena = field();
        let original = order(60000);
        assert_eq!(
            issue(&mut arena, ObjId(1), &original, 1),
            IntentOutcome::AppliedContinuous
        );
        let before = arena.order_watches.clone();
        let wrong_epoch = Intent {
            unit: ObjId(1),
            channel: super::super::Channel::Helm,
            epoch: 0,
            seq: 2,
            kind: IntentKind::Order(Box::new(order(1000))),
        };
        assert_eq!(
            apply_intent(&mut arena, &wrong_epoch, IntentSource::Seat),
            IntentOutcome::Refused(super::super::Refusal::Epoch)
        );
        let mut persona = order(1000);
        persona.by = CharacterRef::Persona { squad: squad() };
        persona.issuer = None;
        arena.unit_mut(ObjId(1)).unwrap().limits.wreck = true;
        assert_eq!(
            issue(&mut arena, ObjId(2), &persona, 3),
            IntentOutcome::Refused(super::super::Refusal::NoCommander)
        );
        assert_eq!(arena.order_watches, before);
        arena.unit_mut(ObjId(1)).unwrap().limits.wreck = false;

        let mut support = order(60000);
        support.objective = Objective::Support { of: ObjId(2) };
        assert_eq!(
            issue(&mut arena, ObjId(1), &support, 4),
            IntentOutcome::AppliedContinuous
        );
        assert_eq!(
            arena.minds.get(&ObjId(1)).unwrap().order_charge,
            Some(ObjId(2))
        );
        // There is no production unit-removal path. Exercise the existing
        // native latent departure predicate; do not invent a removal service.
        arena.units.retain(|unit| unit.id != ObjId(2));
        for wall in (100..=2000).step_by(100) {
            arena.step_settled(wall, &[], &[]);
        }
        assert!(arena.minds.get(&ObjId(1)).unwrap().order.is_none());
        assert!(arena.recovered_order_watches().is_empty());
        assert!(
            arena
                .order_watches
                .0
                .0
                .get(&squad())
                .unwrap()
                .members
                .0
                .is_empty()
        );
    }

    #[test]
    fn r4_actual_300_stock_hull_snapshot_reserves_and_contains_accepted_watches() {
        use super::super::{
            snapshot_capacity,
            state::ArenaTerrain,
            test_support::{custody, ground},
        };
        let units = (1..=300)
            .map(|id| {
                tank(
                    ObjId(id),
                    if id <= 150 { PartyId::RED } else { PartyId::BLUE },
                    pose(
                        i64::from((id - 1) % 20) * 1000 + 500,
                        i64::from((id - 1) / 20) * 1000 + 500,
                    ),
                )
            })
            .collect();
        // Native checkout validates one hull per cell; the ordinary 8x8 test
        // ground cannot contain this actual 300-hull capacity fixture.
        let mut checked_out = custody(units, Vec::new());
        checked_out.definition.terrain =
            ArenaTerrain::uniform(20, 15, 1000, ground()).expect("300 distinct ground cells");
        let mut arena = Arena::new(
            checked_out,
            &crate::config::ArenaConfig::default(),
            &crate::config::MatchConfig::default(),
            &crate::config::SensorsConfig::default(),
        )
        .expect("actual nonoverlapping 300-hull checkout");
        let cohort: Vec<_> = arena.units.iter().map(|u| (u.id, u.team, true)).collect();
        for (id, _, _) in &cohort {
            arena.set_controller(*id, Controller::Mind, MachineState::Patrol);
        }
        arena.squads = crate::deliberation::squads_of(&cohort, MEMBERS as u32);
        let opening = snapshot_capacity::admit(&arena).expect("actual stock cohort admitted");
        assert!(opening.objective_events >= SNAPSHOT_RESERVE);
        for roster in arena.squads.clone() {
            let mut told = order(60000);
            told.squad = roster.id;
            told.issuer = Some(roster.commander);
            for id in roster.units {
                assert_eq!(
                    issue(&mut arena, id, &told, 1),
                    IntentOutcome::AppliedContinuous
                );
            }
        }
        let snapshot = Snapshot::encode(&arena).expect("complete accepted-order snapshot");
        let bytes = serde_json::to_vec(&snapshot.document).unwrap().len() as u64;
        assert!(bytes <= opening.total && opening.total <= opening.limit);
        let decoded =
            Snapshot::decode(&snapshot.document, arena.definition.as_ref().clone()).unwrap();
        assert_eq!(decoded.order_watches, arena.order_watches);
        assert_eq!(decoded.recovered_order_watches().len(), arena.squads.len());
        assert_eq!(
            decoded
                .order_watches
                .0
                .0
                .values()
                .map(|g| g.members.0.len())
                .sum::<usize>(),
            300
        );
    }

    #[test]
    fn r4_complete_maximum_record_is_bounded_and_digest_covers_request_fields() {
        let mut rows = OrderWatches::default();
        for team in [PartyId::RED, PartyId::BLUE] {
            for index in 1..=crate::deliberation::squad::MAX_SQUADS_PER_SIDE {
                rows.0.0.insert(
                    SquadId {
                        team,
                        index: index as u16,
                    },
                    Group {
                        request: [255; 32],
                        by: CharacterRef::Character {
                            id: ObjId(u32::MAX),
                        },
                        issuer: Some(ObjId(u32::MAX)),
                        objective: Word::Withdraw,
                        members: Pairs(
                            (0..MEMBERS)
                                .map(|n| (ObjId(u32::MAX - n as u32), i64::MIN))
                                .collect(),
                        ),
                    },
                );
            }
        }
        let raw = serde_json::to_string(&rows).unwrap();
        // Snapshot embeds JSON as a string; include that full extra layer.
        assert!(serde_json::to_vec(&raw).unwrap().len() as u64 <= SNAPSHOT_RESERVE);
        assert_eq!(serde_json::from_str::<OrderWatches>(&raw).unwrap(), rows);
        let original = order(1000);
        let identity = request(&original);
        for changed in 0..11 {
            let mut o = original.clone();
            match changed {
                0 => o.squad.index = 2,
                1 => o.by = CharacterRef::Character { id: ObjId(91) },
                2 => o.issuer = None,
                3 => o.ttl_ms += 1,
                4 => o.issued_ms += 1,
                5 => o.horizon_ms += 1,
                6 => o.constraints.hold_fire = true,
                7 => o.constraints.max_range_band = Some(RangeBand::Far),
                8 => o.target = Some(super::super::ContactId(1)),
                9 => o.spot = Some("A".into()),
                _ => {
                    o.objective = Objective::Engage {
                        target_hint: Some(super::super::ContactId(2)),
                    }
                }
            }
            assert_ne!(request(&o), identity, "request field {changed}");
        }
    }
}
