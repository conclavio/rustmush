//! 17g C2a (C2 plan §2.1): the Roll actions the line-mode mind commands
//! wrap, captured from the live definition registry. TacSim never hard-codes
//! a definition: each method returns the reference Roll actually registered
//! under its own private id and version, or `None` when that action is not
//! composed on this server (the command then refuses `NOT_OPEN`).
//!
//! A captured reference is a selector only. `Ctx::submit_authoritative` still
//! resolves it against the actor's own projected offers, and the provider
//! rechecks every authority at preparation; capturing adds none.

use mush_core::DefinitionRef;
use mush_core::mechanics::DefinitionRegistry;

/// `(id, version)` of each wrapped Roll action, all of kind `action`.
const RESERVE: (&str, u32) = (mush_core::reserved_shell::RESERVE_ACTION, 1);
const FILE_ORIGINAL: (&str, u32) = ("initial_filing", 1);
const FILE_DEPENDENT: (&str, u32) = (super::dependent_filing::ACTION, 1);
const UPDATE_ORIGINAL: (&str, u32) = (super::line_update::ACTION, 1);
const UPDATE_DERIVED: (&str, u32) = (super::line_update::DERIVED_ACTION, 1);
const WITHDRAW: (&str, u32) = ("withdraw_authored_mind", 1);
const ATTEST: (&str, u32) = (super::owner_attestation::protocol::ACTION, 1);
const FORK: (&str, u32) = ("fork_paid_mind", 1);

/// The registered Roll actions behind `@mind/file`, `/update`, `/withdraw`,
/// `/attest` and `/fork`, as this composition registered them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RollCommandActions {
    reserve: Option<DefinitionRef>,
    file_original: Option<DefinitionRef>,
    file_dependent: Option<DefinitionRef>,
    update_original: Option<DefinitionRef>,
    update_derived: Option<DefinitionRef>,
    withdraw: Option<DefinitionRef>,
    attest: Option<DefinitionRef>,
    fork: Option<DefinitionRef>,
}

impl RollCommandActions {
    /// Read each wrapped action from the registry. A key Roll did not
    /// register, or registered more than once under that version, is `None`.
    #[must_use]
    pub fn capture(definitions: &DefinitionRegistry) -> Self {
        let find = |(id, version): (&str, u32)| {
            let mut found = definitions
                .iter()
                .map(|entry| entry.reference())
                .filter(|r| {
                    let key = r.key();
                    key.owner.as_str() == crate::NS
                        && key.kind.as_str() == "action"
                        && key.id.as_str() == id
                        && r.version() == version
                });
            match (found.next(), found.next()) {
                (Some(reference), None) => Some(reference.clone()),
                _ => None,
            }
        };
        Self {
            reserve: find(RESERVE),
            file_original: find(FILE_ORIGINAL),
            file_dependent: find(FILE_DEPENDENT),
            update_original: find(UPDATE_ORIGINAL),
            update_derived: find(UPDATE_DERIVED),
            withdraw: find(WITHDRAW),
            attest: find(ATTEST),
            fork: find(FORK),
        }
    }
    /// `roll-reserve-mind-charter`: reserve the filing's inert charter shell.
    #[must_use]
    pub fn reserve(&self) -> Option<DefinitionRef> {
        self.reserve.clone()
    }
    /// `roll-file-original-mind`, or `roll-file-dependent-mind` for a marked
    /// derivative.
    #[must_use]
    pub fn file(&self, dependent: bool) -> Option<DefinitionRef> {
        if dependent {
            self.file_dependent.clone()
        } else {
            self.file_original.clone()
        }
    }
    /// `roll-update-original-mind`, or `roll-update-derived-mind` on a line
    /// born dependent.
    #[must_use]
    pub fn update(&self, derived: bool) -> Option<DefinitionRef> {
        if derived {
            self.update_derived.clone()
        } else {
            self.update_original.clone()
        }
    }
    /// `roll-withdraw-authored-mind`.
    #[must_use]
    pub fn withdraw(&self) -> Option<DefinitionRef> {
        self.withdraw.clone()
    }
    /// `roll-attest-owner-mind`.
    #[must_use]
    pub fn attest(&self) -> Option<DefinitionRef> {
        self.attest.clone()
    }
    /// `roll-fork-paid-mind` (Roll's own action; Market's same-named
    /// definition is not this one).
    #[must_use]
    pub fn fork(&self) -> Option<DefinitionRef> {
        self.fork.clone()
    }
}

#[cfg(test)]
mod tests;
