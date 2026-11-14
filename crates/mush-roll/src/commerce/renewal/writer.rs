//! Closed paired Roll publication. Plain records never establish authority:
//! publication consumes exact opaque Economy output and same-preparation owner
//! facts, then retains the actual Root participant fence through custody.
//!
//! Doc 46 CT.5: written once in `renewal_twin::writer`; these are its types at the
//! personal issuer.
use super::super::renewal_twin::Personal;

/// Shared trusted composition cell. Only Roll installs its real families and
/// fixed grants; no public state/image or current-rights constructor exists.
pub type RenewalWriter = super::super::renewal_twin::writer::RenewalWriter<Personal>;
/// No Clone/Deserialize/public constructor. An observation is inseparable from
/// its actual preparation action+commit; serialized facts are only data.
pub type PreparedRenewal = super::super::renewal_twin::writer::PreparedRenewal<Personal>;
pub type RegisteredRenewalWriter =
    super::super::renewal_twin::writer::RegisteredRenewalWriter<Personal>;
