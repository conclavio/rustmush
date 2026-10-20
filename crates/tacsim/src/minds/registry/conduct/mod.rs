//! Private deterministic role work. None of these values is an acknowledged
//! qualification: the owner publication and replay join mint that separately.
mod cargo;
pub(crate) mod composition;
pub(crate) mod factory;
pub(crate) mod history;
mod manifest;
mod observation;
mod run;
#[cfg(test)]
mod tests;
mod trial;
mod worker;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Role {
    Escort,
    Transit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Case {
    Convoy,
    EscortThreat,
    MissingCharge,
    ForeignCharge,
    HoldFire,
    ThreatStopResume,
    BlockedRoute,
    Withdraw,
}
impl Case {
    pub(super) const ALL: [Self; 8] = [
        Self::Convoy,
        Self::EscortThreat,
        Self::MissingCharge,
        Self::ForeignCharge,
        Self::HoldFire,
        Self::ThreatStopResume,
        Self::BlockedRoute,
        Self::Withdraw,
    ];
    pub(super) fn for_role(role: Role) -> &'static [Self] {
        match role {
            Role::Escort => &[
                Self::Convoy,
                Self::EscortThreat,
                Self::MissingCharge,
                Self::ForeignCharge,
                Self::HoldFire,
                Self::Withdraw,
            ],
            Role::Transit => &[
                Self::Convoy,
                Self::MissingCharge,
                Self::HoldFire,
                Self::ThreatStopResume,
                Self::BlockedRoute,
                Self::Withdraw,
            ],
        }
    }
}
