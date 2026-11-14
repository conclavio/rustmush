//! The House renewal's service-close tests: the bodies are
//! `renewal_twin/tests/service.rs`, run here for [`House`].
use super::*;

twin::service::instantiate!(House);

/// Doc 46 CT.0: the committed snapshot of this twin's walk.
mod ct0_snapshot;
