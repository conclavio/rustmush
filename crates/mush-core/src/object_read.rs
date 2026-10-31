//! The object-read seam shared by lock evaluation and movement checks.

use std::convert::Infallible;

use crate::{Obj, ObjId, World};

/// Shared semantic readers use this interface. References borrow the World, so recursive reads can charge the
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
