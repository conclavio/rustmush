//! Optional report-only accepted-shot counters. Kept out of snapshots/hashes;
//! an ordinary arena allocates nothing and performs no additional odds query.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Acceptance {
    buckets: [u32; 10],
}
impl Acceptance {
    pub fn record(&mut self, permille: u32) {
        let bucket = (permille.min(1000) / 100).min(9) as usize;
        self.buckets[bucket] = self.buckets[bucket].saturating_add(1);
    }
    /// Ten shot-count buckets: 0..99,100..199,...,900..1000 per mille.
    pub fn buckets(&self) -> [u32; 10] {
        self.buckets
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn endpoints_and_saturation_have_fixed_storage() {
        let mut a = Acceptance::default();
        for p in [0, 99, 100, 899, 900, 1000] {
            a.record(p);
        }
        assert_eq!(a.buckets(), [2, 1, 0, 0, 0, 0, 0, 0, 1, 2]);
        a.buckets[9] = u32::MAX;
        a.record(1000);
        assert_eq!(a.buckets[9], u32::MAX);
    }
}
