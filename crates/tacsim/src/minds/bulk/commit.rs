//! Opaque durable-writer latch. It is never released by RAM publication.
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
#[derive(Clone, Debug, Default)]
pub(crate) struct CommitLatch(Arc<AtomicBool>);
impl CommitLatch {
    pub fn committed(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
    /// Only ArenaTasks' existing stamped writer high-water loop calls this.
    pub(crate) fn acknowledge(&self) {
        self.0.store(true, Ordering::Release);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cloning_and_ram_publication_do_not_acknowledge_a_lease() {
        let latch = CommitLatch::default();
        let pending = latch.clone();
        assert!(!pending.committed());
        latch.acknowledge();
        assert!(pending.committed());
    }
}
