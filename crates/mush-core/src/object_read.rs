//! Bounded, private dependency reads for inert Core admission checks.
//!
//! A trace is a full object preimage (including absence), not an authority
//! grant or a substitute for final module hooks and typed-state guards.

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::fmt;
use std::io::Write;

use crate::attr::AttrValue;
use crate::{Obj, ObjId, World};

/// Shared semantic readers use this interface; only the tracked reader has
/// limits. References borrow the World, so recursive reads can charge the
/// same budget without cloning the expression or attribute being evaluated.
pub(crate) trait ObjectRead<'w> {
    type Error;

    fn object(&mut self, id: ObjId) -> Result<Option<&'w Obj>, Self::Error>;
    fn charge_work(&mut self, units: usize) -> Result<(), Self::Error>;
    fn object_slots(&mut self) -> Result<usize, Self::Error>;
}

pub(crate) struct PlainRead<'w> {
    world: &'w World,
}

impl<'w> PlainRead<'w> {
    pub(crate) fn new(world: &'w World) -> Self {
        Self { world }
    }
}

impl<'w> ObjectRead<'w> for PlainRead<'w> {
    type Error = Infallible;

    fn object(&mut self, id: ObjId) -> Result<Option<&'w Obj>, Infallible> {
        Ok(self.world.get(id))
    }

    fn charge_work(&mut self, _units: usize) -> Result<(), Infallible> {
        Ok(())
    }

    fn object_slots(&mut self) -> Result<usize, Infallible> {
        Ok(self.world.object_slot_count())
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ReadLimits {
    pub(crate) max_objects: usize,
    pub(crate) max_bytes: usize,
    pub(crate) max_visits: usize,
    pub(crate) max_work: usize,
}

impl Default for ReadLimits {
    fn default() -> Self {
        Self {
            max_objects: 512,
            max_bytes: 1_048_576,
            max_visits: 4096,
            max_work: 4_194_304,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum ReadLimit {
    #[cfg(test)]
    #[error("object read limits must be positive")]
    InvalidLimits,
    #[error("object read dependency limit exceeded")]
    Objects,
    #[error("object read byte limit exceeded")]
    Bytes,
    #[error("object read visit limit exceeded")]
    Visits,
    #[error("object read work limit exceeded")]
    Work,
    #[error("object read nesting limit exceeded")]
    Nesting,
    #[error("object read preimage could not be encoded")]
    Encoding,
}

struct Budget {
    limits: ReadLimits,
    visits: usize,
    work: usize,
    bytes: usize,
    failed: Option<ReadLimit>,
}

impl Budget {
    fn new(limits: ReadLimits) -> Self {
        Self {
            limits,
            visits: 0,
            work: 0,
            bytes: 0,
            failed: None,
        }
    }

    fn fail(&mut self, error: ReadLimit) -> ReadLimit {
        *self.failed.get_or_insert(error)
    }

    fn check(&self) -> Result<(), ReadLimit> {
        self.failed.map_or(Ok(()), Err)
    }

    fn work(&mut self, units: usize) -> Result<(), ReadLimit> {
        self.check()?;
        let Some(next) = self.work.checked_add(units) else {
            return Err(self.fail(ReadLimit::Work));
        };
        if next > self.limits.max_work {
            return Err(self.fail(ReadLimit::Work));
        }
        self.work = next;
        Ok(())
    }

    fn visit(&mut self) -> Result<(), ReadLimit> {
        self.work(1)?;
        let Some(next) = self.visits.checked_add(1) else {
            return Err(self.fail(ReadLimit::Visits));
        };
        if next > self.limits.max_visits {
            return Err(self.fail(ReadLimit::Visits));
        }
        self.visits = next;
        Ok(())
    }

    fn bytes(&mut self, amount: usize) -> Result<(), ReadLimit> {
        self.check()?;
        let Some(next) = self.bytes.checked_add(amount) else {
            return Err(self.fail(ReadLimit::Bytes));
        };
        if next > self.limits.max_bytes {
            return Err(self.fail(ReadLimit::Bytes));
        }
        self.work(amount)?;
        self.bytes = next;
        Ok(())
    }
}

pub(crate) struct TrackedRead<'w> {
    world: &'w World,
    rows: BTreeMap<ObjId, Option<Obj>>,
    budget: Budget,
}

impl<'w> TrackedRead<'w> {
    pub(crate) fn new(world: &'w World) -> Self {
        Self {
            world,
            rows: BTreeMap::new(),
            budget: Budget::new(ReadLimits::default()),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_limits(world: &'w World, limits: ReadLimits) -> Result<Self, ReadLimit> {
        if limits.max_objects == 0
            || limits.max_bytes == 0
            || limits.max_visits == 0
            || limits.max_work == 0
        {
            return Err(ReadLimit::InvalidLimits);
        }
        Ok(Self {
            world,
            rows: BTreeMap::new(),
            budget: Budget::new(limits),
        })
    }

    pub(crate) fn finish(self) -> Result<ReadTrace, ReadLimit> {
        self.budget.check()?;
        Ok(ReadTrace {
            rows: self.rows,
            limits: self.budget.limits,
        })
    }
}

impl<'w> ObjectRead<'w> for TrackedRead<'w> {
    type Error = ReadLimit;

    fn object(&mut self, id: ObjId) -> Result<Option<&'w Obj>, ReadLimit> {
        self.budget.visit()?;
        if !self.rows.contains_key(&id) {
            if self.rows.len() >= self.budget.limits.max_objects {
                return Err(self.budget.fail(ReadLimit::Objects));
            }
            let object = self.world.get(id);
            preflight_entry(&mut self.budget, id, object)?;
            // Neither a recursive clone nor an unbounded value serialization
            // runs until both the shape and streaming byte checks pass.
            self.rows.insert(id, object.cloned());
        }
        Ok(self.world.get(id))
    }

    fn charge_work(&mut self, units: usize) -> Result<(), ReadLimit> {
        self.budget.work(units)
    }

    fn object_slots(&mut self) -> Result<usize, ReadLimit> {
        self.budget.visit()?;
        // This is only the legacy containment walk's termination bound.
        // All semantic edges are traced; unrelated allocation does not stale
        // a route whose full chain was proved.
        Ok(self.world.object_slot_count())
    }
}

pub(crate) struct ReadTrace {
    rows: BTreeMap<ObjId, Option<Obj>>,
    limits: ReadLimits,
}

impl fmt::Debug for ReadTrace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReadTrace")
            .field("objects", &self.rows.len())
            .finish_non_exhaustive()
    }
}

impl ReadTrace {
    pub(crate) fn matches(&self, world: &World) -> bool {
        self.matches_with(|id| world.get(id))
    }

    /// The same bounded comparison under Core's exclusive installation borrow.
    /// No callback or object cloning occurs while binding an image.
    pub(crate) fn matches_objects(&self, objects: &[Option<Obj>]) -> bool {
        self.matches_with(|id| objects.get(id.0 as usize).and_then(Option::as_ref))
    }

    fn matches_with<'a>(&self, mut get: impl FnMut(ObjId) -> Option<&'a Obj>) -> bool {
        let mut budget = Budget::new(self.limits);
        for (id, expected) in &self.rows {
            if budget.visit().is_err() {
                return false;
            }
            let live = get(*id);
            // A changed candidate may now contain a malformed deep tree.
            // Bound it before the derived, recursive Obj equality walk.
            if preflight_entry(&mut budget, *id, live).is_err() || live != expected.as_ref() {
                return false;
            }
        }
        true
    }
}

/// Preflight support shared with the lock AST's sole structural reader.
/// Raw text is bounded before serde can scan a huge unescaped string.
pub(crate) struct SnapshotPreflight<'a> {
    budget: &'a mut Budget,
    raw_remaining: usize,
}

impl SnapshotPreflight<'_> {
    pub(crate) fn node(&mut self) -> Result<(), ReadLimit> {
        self.budget.work(1)?;
        self.raw_remaining = self
            .raw_remaining
            .checked_sub(1)
            .ok_or_else(|| self.budget.fail(ReadLimit::Bytes))?;
        Ok(())
    }

    pub(crate) fn container(&mut self, depth: usize) -> Result<(), ReadLimit> {
        self.node()?;
        if depth > crate::depth::MAX_DOC_DEPTH {
            return Err(self.budget.fail(ReadLimit::Nesting));
        }
        Ok(())
    }

    pub(crate) fn text(&mut self, text: &str) -> Result<(), ReadLimit> {
        self.node()?;
        self.raw_remaining = self
            .raw_remaining
            .checked_sub(text.len())
            .ok_or_else(|| self.budget.fail(ReadLimit::Bytes))?;
        self.budget.work(text.len())
    }
}

fn preflight_entry(budget: &mut Budget, id: ObjId, object: Option<&Obj>) -> Result<(), ReadLimit> {
    budget.check()?;
    if let Some(object) = object {
        let raw_remaining = budget.limits.max_bytes.saturating_sub(budget.bytes);
        let mut shape = SnapshotPreflight {
            budget,
            raw_remaining,
        };
        shape.container(1)?;
        shape.text(&object.name)?;
        for alias in &object.aliases {
            shape.text(alias)?;
        }
        for marker in &object.markers {
            shape.text(marker)?;
        }
        for _ in object.contents.iter().chain(&object.exits) {
            shape.node()?;
        }
        for (name, attr) in object.attrs.iter() {
            shape.text(name.as_str())?;
            shape.container(3)?;
            match &attr.value {
                AttrValue::Text(text) => shape.text(text)?,
                // Obj -> attrs -> Attr -> AttrValue::Json -> JSON value.
                AttrValue::Json(value) => preflight_json(&mut shape, value, 5)?,
            }
        }
        for lock in object.locks.values() {
            crate::lock::preflight_snapshot_lock(lock, &mut shape, 2)?;
        }
        for id in object.behaviors.keys() {
            shape.text(id.as_str())?;
        }
    }
    // Tuple framing accounts for the key, absence, and delimiters as well as
    // the full object. This counter never allocates the serialized value.
    let encoded = serde_json::to_writer(
        Counter {
            budget: &mut *budget,
        },
        &(id, object),
    );
    if encoded.is_err() {
        return Err(budget.fail(ReadLimit::Encoding));
    }
    Ok(())
}

struct Counter<'a> {
    budget: &'a mut Budget,
}

impl Write for Counter<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.budget
            .bytes(bytes.len())
            .map_err(std::io::Error::other)?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn preflight_json(
    shape: &mut SnapshotPreflight<'_>,
    root: &serde_json::Value,
    depth: usize,
) -> Result<(), ReadLimit> {
    use serde_json::Value;
    enum Frame<'a> {
        Value(&'a Value, usize),
        Array(std::slice::Iter<'a, Value>, usize),
        Object(serde_json::map::Iter<'a>, usize),
    }
    let mut stack = vec![Frame::Value(root, depth)];
    while let Some(frame) = stack.pop() {
        match frame {
            Frame::Value(value, depth) => match value {
                Value::Array(values) => {
                    shape.container(depth)?;
                    stack.push(Frame::Array(values.iter(), depth));
                }
                Value::Object(values) => {
                    shape.container(depth)?;
                    stack.push(Frame::Object(values.iter(), depth));
                }
                Value::String(text) => shape.text(text)?,
                _ => shape.node()?,
            },
            Frame::Array(mut values, depth) => {
                if let Some(value) = values.next() {
                    shape.budget.work(1)?;
                    stack.push(Frame::Array(values, depth));
                    stack.push(Frame::Value(value, depth + 1));
                }
            }
            Frame::Object(mut values, depth) => {
                if let Some((key, value)) = values.next() {
                    shape.text(key)?;
                    stack.push(Frame::Object(values, depth));
                    stack.push(Frame::Value(value, depth + 1));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
