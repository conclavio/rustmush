//! Volatile authenticated reference selection. Mutable timelines and their
//! retained state belong to the private worker and connection service.
//! Replay engines and bulk stepping live in the worker adapters, not this owner.
use super::{
    BulkError, invalid,
    model::{Artifact, BulkConfig, RunId},
};
use mush_core::{AccountId, ObjId};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct SessionId(u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Connection {
    pub account: AccountId,
    pub character: ObjId,
    pub lease: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReferenceId {
    Proving(RunId),
    Match(crate::arena_task::MatchId),
}
/// The tape/proving owner constructs this after current-character authorization
/// and complete artifact verification. Request text supplies neither ACL set.
pub(crate) struct Reference {
    id: ReferenceId,
    artifact: Artifact,
    inspect_units: BTreeSet<ObjId>,
    private_units: BTreeSet<ObjId>,
}
impl Reference {
    pub(super) fn verified(
        id: ReferenceId,
        artifact: Artifact,
        inspect_units: BTreeSet<ObjId>,
        memory_units: BTreeSet<ObjId>,
        private_units: BTreeSet<ObjId>,
    ) -> Result<Self, BulkError> {
        artifact.validate()?;
        if inspect_units.is_empty()
            || inspect_units.len() > 300
            || !memory_units.is_subset(&inspect_units)
            || !private_units.is_subset(&inspect_units)
            || (matches!(id, ReferenceId::Match(_)) && !private_units.is_empty())
        {
            return Err(invalid("bench projection authority"));
        }
        Ok(Self {
            id,
            artifact,
            inspect_units,
            private_units,
        })
    }
}
struct Session {
    id: SessionId,
    connection: Connection,
    last_seen_ms: i64,
    reference: Reference,
    selected: ObjId,
}
pub(crate) struct PreparedOpen {
    expected: u64,
    session: Session,
}
pub(crate) struct Benches {
    cfg: BulkConfig,
    generation: u64,
    next: u64,
    sessions: BTreeMap<AccountId, Session>,
}
impl Benches {
    pub fn new(cfg: BulkConfig) -> Result<Self, BulkError> {
        cfg.validate()?;
        Ok(Self {
            cfg,
            generation: 0,
            next: 1,
            sessions: BTreeMap::new(),
        })
    }
    pub fn prepare_open(
        &self,
        connection: Connection,
        reference: Reference,
        selected: ObjId,
        now: i64,
    ) -> Result<PreparedOpen, BulkError> {
        if self.sessions.contains_key(&connection.account) {
            return Err(BulkError::Busy);
        }
        if self.sessions.len() >= self.cfg.bench_sessions
            || self
                .sessions
                .values()
                .map(|s| s.reference.artifact.bytes)
                .sum::<u64>()
                .saturating_add(reference.artifact.bytes)
                > self.cfg.bench_state_bytes
        {
            return Err(BulkError::Retention);
        }
        if !reference.inspect_units.contains(&selected) {
            return Err(BulkError::Missing);
        }
        self.next
            .checked_add(1)
            .ok_or_else(|| invalid("session allocator exhausted"))?;
        Ok(PreparedOpen {
            expected: self.generation,
            session: Session {
                id: SessionId(self.next),
                connection,
                last_seen_ms: now,
                reference,
                selected,
            },
        })
    }
    pub fn check_open(&self, p: &PreparedOpen) -> Result<(), BulkError> {
        if p.expected != self.generation {
            Err(BulkError::Stale)
        } else {
            Ok(())
        }
    }
    /// Publish after checking this token and the matching durable run/tape pin.
    pub fn apply_open(&mut self, p: PreparedOpen) -> SessionId {
        let id = p.session.id;
        self.next = self.next.saturating_add(1);
        self.generation = self.generation.saturating_add(1);
        self.sessions
            .insert(p.session.connection.account, p.session);
        id
    }
    fn current(&mut self, connection: Connection, now: i64) -> Result<&mut Session, BulkError> {
        let row = self
            .sessions
            .get_mut(&connection.account)
            .filter(|s| s.connection == connection)
            .ok_or(BulkError::Missing)?;
        if now < row.last_seen_ms
            || now.saturating_sub(row.last_seen_ms) >= self.cfg.bench_idle_ms as i64
        {
            return Err(BulkError::Missing);
        }
        row.last_seen_ms = now;
        Ok(row)
    }
    /// Pure cached reads are free; the exact authenticated connection still
    /// scopes the selected unit. Whole-world replay internals never escape.
    pub fn selected(&mut self, connection: Connection, now: i64) -> Result<ObjId, BulkError> {
        Ok(self.current(connection, now)?.selected)
    }
    pub fn act_as(
        &mut self,
        connection: Connection,
        unit: ObjId,
        now: i64,
    ) -> Result<(), BulkError> {
        let row = self.current(connection, now)?;
        if !row.reference.private_units.contains(&unit) {
            return Err(BulkError::Missing);
        }
        row.selected = unit;
        Ok(())
    }
    pub fn close(&mut self, connection: Connection) -> Option<ReferenceId> {
        if !self
            .sessions
            .get(&connection.account)
            .is_some_and(|row| row.connection == connection)
        {
            return None;
        }
        let row = self.sessions.remove(&connection.account)?;
        self.generation = self.generation.saturating_add(1);
        Some(row.reference.id)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn connection() -> Connection {
        Connection {
            account: AccountId::mint("owner", 1).expect("account"),
            character: ObjId(2),
            lease: 7,
        }
    }
    fn reference(private: bool) -> Reference {
        Reference::verified(
            ReferenceId::Proving(RunId(1)),
            Artifact {
                sha256: [1; 32],
                bytes: 100,
            },
            BTreeSet::from([ObjId(3)]),
            BTreeSet::from([ObjId(3)]),
            if private {
                BTreeSet::from([ObjId(3)])
            } else {
                BTreeSet::new()
            },
        )
        .expect("reference")
    }
    fn open(b: &mut Benches, c: Connection) {
        let p = b
            .prepare_open(c, reference(true), ObjId(3), 0)
            .expect("prepare");
        b.check_open(&p).expect("current");
        b.apply_open(p);
    }
    #[test]
    fn one_account_disconnect_and_idle_release_pin_only() {
        let mut b = Benches::new(BulkConfig::default()).expect("store");
        let c = connection();
        open(&mut b, c);
        assert!(matches!(
            b.prepare_open(c, reference(true), ObjId(3), 1),
            Err(BulkError::Busy)
        ));
        assert!(b.close(Connection { lease: 8, ..c }).is_none());
        assert!(b.selected(c, 30 * 60 * 1000).is_err());
        assert_eq!(b.close(c), Some(ReferenceId::Proving(RunId(1))));
        assert!(b.selected(c, 30 * 60 * 1000).is_err());
    }
    #[test]
    fn unauthorized_units_characters_and_non_private_units_refuse() {
        let mut b = Benches::new(BulkConfig::default()).unwrap();
        let c = connection();
        open(&mut b, c);
        assert_eq!(b.act_as(c, ObjId(99), 1), Err(BulkError::Missing));
        assert_eq!(
            b.selected(
                Connection {
                    character: ObjId(77),
                    ..c
                },
                1
            ),
            Err(BulkError::Missing)
        );
        b.close(c);
        let p = b.prepare_open(c, reference(false), ObjId(3), 2).unwrap();
        b.check_open(&p).unwrap();
        b.apply_open(p);
        assert_eq!(b.act_as(c, ObjId(3), 3), Err(BulkError::Missing));
    }
}
