//! Doc 46 CT.6: the market's renewal composition, written once for both issuers.
//!
//! `mind_renewal` (the personal Pin) and `mind_house_renewal` (the House-issued Pin) were two
//! copies of one composition: the providers, the paired boot audit, the retained service and
//! its command, the capacity bound and the records. Their differences were the family's
//! identity strings, the Economy and kernel entries only the family can name, its reader, and
//! three rules: the offer's payee and parent royalties, the House purchase template a renewal
//! must name, and the status line. This module is the one implementation, generic over the
//! sealed [`Issuer`] the two families implement; each family's `mod.rs` holds its
//! implementation, its records (one macro text, [`records::records!`]) and every path its
//! callers name.
//!
//! There is no generic engine, no `dyn`, no callback and no function pointer: the owner is
//! resolved at compile time.

mod issuer;
pub(crate) use issuer::{Entry, Inventory, Issuer, Offer, OfferParts, Read, RollOperation};

pub(crate) mod actions;
pub(crate) mod audit;
pub(crate) mod capacity;
pub(crate) mod config;
pub(crate) mod families;
pub(crate) mod offer;
pub(crate) mod provider;
pub(crate) mod reader;
pub(crate) mod records;
pub(crate) mod registration;
pub(crate) mod service;
pub(crate) mod service_provider;
