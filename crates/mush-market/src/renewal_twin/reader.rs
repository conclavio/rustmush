//! The Roll's two renewal readers, as the generic service reads them.
//!
//! The readers are the Roll's and still two (CT.5 left them unmerged), with the same methods
//! over each family's own types. [`reader_face!`] is the one text of their [`Inventory`] and
//! [`Entry`] implementations, expanded in each family's `mod.rs` over its reader module.
//!
//! [`Inventory`]: super::Inventory
//! [`Entry`]: super::Entry

/// The [`Inventory`](super::Inventory) and [`Entry`](super::Entry) of the reader module
/// `$reader`, each method its inherent method of the same name.
macro_rules! reader_face {
    ($reader:path) => {
        use $reader as reader;
        impl $crate::renewal_twin::Inventory for reader::RenewalInventory {
            type Entry = reader::RenewalInventoryEntry;
            fn entries(&self) -> &[reader::RenewalInventoryEntry] {
                Self::entries(self)
            }
            fn into_single(
                self,
            ) -> Result<reader::RenewalInventoryEntry, mush_core::read_service::OwnerReadError>
            {
                Self::into_single(self)
            }
            fn next_after(&self) -> Option<&str> {
                Self::next_after(self)
            }
        }
        impl $crate::renewal_twin::Entry for reader::RenewalInventoryEntry {
            fn stop(&self) -> Option<mush_roll::commerce::renewal::reader::RenewalStop> {
                Self::stop(self)
            }
            fn licence(&self) -> mush_roll::commerce::LicenceId {
                Self::licence(self)
            }
            fn holder(&self) -> mush_economy::Holder {
                Self::holder(self)
            }
            fn generation(&self) -> mush_roll::commerce::TermGeneration {
                Self::generation(self)
            }
            fn term_end(&self) -> mush_roll::commerce::WorldIcSeconds {
                Self::term_end(self)
            }
            fn last_operation(&self) -> Option<mush_core::CommitToken> {
                Self::last_operation(self)
            }
            fn holder_generation(&self) -> u64 {
                Self::holder_generation(self)
            }
            fn attempted(&self) -> bool {
                Self::attempted(self)
            }
        }
    };
}
pub(crate) use reader_face;
