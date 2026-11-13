#![cfg(test)]

//! Doc 46 CT.0: the House renewal service's private identity strings and
//! sentences, for the twin's walk (`tests::ct0_snapshot`). A child of the
//! service, so it reads the service's own private names (`service_lines!`, in
//! `tests::ct0_snapshot`, is shared with the personal twin's service).
use super::*;

crate::mind_house_renewal::tests::ct0_snapshot::service_lines!();
