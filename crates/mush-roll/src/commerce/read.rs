//! The two real owner-stamped read contexts share validation, not authority.
//! No outside implementation, callback, writable handle or synthetic healthy
//! view can implement this private adapter.

use mush_core::read_service::OwnerReadContext;
use mush_core::{ModuleCtx, StateFamilyHandle, World};

mod sealed {
    pub trait Sealed {}
    impl Sealed for mush_core::ModuleCtx<'_> {}
    impl Sealed for mush_core::read_service::OwnerReadContext<'_> {}
}

#[derive(Clone, Copy)]
pub(crate) struct ReadUnavailable;

pub(crate) trait CommerceRead: sealed::Sealed {
    fn world(&self) -> &World;
    fn get<K, V>(
        &self,
        handle: &StateFamilyHandle<K, V>,
        key: &K,
    ) -> Result<Option<&V>, ReadUnavailable>
    where
        K: Send + Sync + 'static,
        V: Clone + Send + Sync + 'static;
    fn get_many<K, V>(
        &self,
        handle: &StateFamilyHandle<K, V>,
        keys: &[K],
    ) -> Result<Vec<Option<&V>>, ReadUnavailable>
    where
        K: Send + Sync + 'static,
        V: Clone + Send + Sync + 'static;
}
impl CommerceRead for ModuleCtx<'_> {
    fn world(&self) -> &World {
        self.w
    }
    fn get<K, V>(
        &self,
        handle: &StateFamilyHandle<K, V>,
        key: &K,
    ) -> Result<Option<&V>, ReadUnavailable>
    where
        K: Send + Sync + 'static,
        V: Clone + Send + Sync + 'static,
    {
        self.action_state(handle, key).map_err(|_| ReadUnavailable)
    }
    fn get_many<K, V>(
        &self,
        handle: &StateFamilyHandle<K, V>,
        keys: &[K],
    ) -> Result<Vec<Option<&V>>, ReadUnavailable>
    where
        K: Send + Sync + 'static,
        V: Clone + Send + Sync + 'static,
    {
        if keys.is_empty() || keys.len() > mush_core::read_service::MAX_OWNER_READ_PAGE {
            return Err(ReadUnavailable);
        }
        keys.iter().map(|key| self.get(handle, key)).collect()
    }
}
impl CommerceRead for OwnerReadContext<'_> {
    fn world(&self) -> &World {
        OwnerReadContext::world(self)
    }
    fn get<K, V>(
        &self,
        handle: &StateFamilyHandle<K, V>,
        key: &K,
    ) -> Result<Option<&V>, ReadUnavailable>
    where
        K: Send + Sync + 'static,
        V: Clone + Send + Sync + 'static,
    {
        self.action_state(handle, key).map_err(|_| ReadUnavailable)
    }
    fn get_many<K, V>(
        &self,
        handle: &StateFamilyHandle<K, V>,
        keys: &[K],
    ) -> Result<Vec<Option<&V>>, ReadUnavailable>
    where
        K: Send + Sync + 'static,
        V: Clone + Send + Sync + 'static,
    {
        self.action_states(handle, keys)
            .map_err(|_| ReadUnavailable)
    }
}
