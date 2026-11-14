#![cfg(test)]

//! Doc 46 CT.0: the personal renewal service's private identity strings and
//! sentences, for the twin's walk (`tests::ct0_snapshot`): the shared
//! `service_lines!` (`mind_house_renewal::tests::ct0_snapshot`), read against
//! this service's own names.
use super::*;

/// Doc 46 CT.6: the personal issuer's command, under the name the shared lines read.
const COMMAND: &str = <mush_economy::renewal_twin::Personal as Issuer>::COMMAND;

crate::mind_house_renewal::tests::ct0_snapshot::service_lines!();
