//! Formations and task forces (E7 cut 9 B4; 10b §3.9, Q9A/D-E7-19, doc 10 §19 T8,
//! §16 items 5 and 11's R1/R2).
//!
//! Two layers, deliberately. A **persistent order of battle** — [`FormationNode`]s
//! in an administrative tree — and, beside it, an operation-scoped command overlay
//! ([`crate::record::TaskForce`]). **Administrative parentage is unchanged by
//! attachment**, and a unit has exactly one operational command parent inside an
//! operation.
//!
//! ## `composition` is a constraint set, and `exact` is `minimum == maximum`
//!
//! Never a separate mode. A four-unit lance and a five-unit star therefore share an
//! echelon and stay distinct kinds, because what differs is the constraint set, not
//! an enum arm.
//!
//! ## Strictness is scoped
//!
//! [`Strictness`] names *when* a constraint set is enforced — at creation, at
//! doctrine qualification ("is this still a full squadron?"), or as an operation's
//! requirement — and **never** to a formation's existence or its ability to deploy.
//! **An understrength permanent formation is normal, not an error**, and
//! [`FormationNode::may_deploy`] answers `true` for one.
//!
//! ## Authority
//!
//! Each node names **exactly one** owning `OrgScopeId` (R2) — a node without one is
//! unconstructible, because the type has no `Option` there — and `force.*`
//! capabilities resolve through `mush_org::authorize`, so R2 needs **no core
//! change**. The ids must be in `org_capability_vocabulary()` or `@org/grant` fails
//! loudly (`crates/mush-org/src/wiring.rs`'s typo catcher).
//!
//! ## Rule 1 category
//!
//! **Exempt** — workflow, legacy-post-image, family `campaign.formation`, key
//! `formation:<id>`.

use std::collections::BTreeSet;

use mush_core::World;
use mush_org::OrgScopeId;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::ids::FormationId;
use crate::record::{MAX_LABEL_BYTES, UnitRef};

/// The deepest administrative tree a formation may sit in (10b §7).
pub const MAX_FORMATION_DEPTH: usize = 5;
/// The largest direct membership one node may carry (10b §7).
pub const MAX_FORMATION_MEMBERS: usize = 64;
/// The largest role list a composition may declare.
pub const MAX_COMMAND_ROLES: usize = 16;
/// The largest allowed-category set a composition may declare.
pub const MAX_ALLOWED_CATEGORIES: usize = 32;
/// The largest number of formations one operation may attach.
pub const MAX_FORMATIONS_PER_OPERATION: usize = 32;

/// A unit category a composition admits. Ruleset vocabulary, opaque here — the
/// engine stores generic kinds and `tacsim` supplies the names (T8).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CategoryId(pub SmolStr);

/// A command or support role a composition requires.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleSpec {
    /// The role's id (≤ [`MAX_LABEL_BYTES`]).
    pub id: SmolStr,
    /// How many of it the composition wants.
    pub minimum: u8,
}

/// The constraint set (Q9A). **`exact` is `minimum == maximum`** and there is no
/// third field for it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Composition {
    /// The fewest members the kind wants.
    pub minimum: u8,
    /// The most it admits.
    pub maximum: u8,
    /// What it is normally fielded at.
    pub preferred: u8,
    /// Which unit categories may fill it (empty admits any).
    pub allowed: BTreeSet<CategoryId>,
    /// The command roles it requires.
    pub required_command: Vec<RoleSpec>,
    /// The support roles it requires.
    pub required_support: Vec<RoleSpec>,
}

impl Composition {
    /// **`exact` ≡ `minimum == maximum`** — a derived predicate, never a mode.
    #[must_use]
    pub fn is_exact(&self) -> bool {
        self.minimum == self.maximum
    }

    /// Re-run the set's own bounds.
    pub fn check(&self) -> Result<(), FormationError> {
        if self.maximum == 0 || self.minimum > self.maximum {
            return Err(FormationError::CompositionRange);
        }
        if self.preferred < self.minimum || self.preferred > self.maximum {
            return Err(FormationError::CompositionRange);
        }
        if usize::from(self.maximum) > MAX_FORMATION_MEMBERS {
            return Err(FormationError::TooMany {
                what: "the composition maximum",
                actual: usize::from(self.maximum),
                max: MAX_FORMATION_MEMBERS,
            });
        }
        if self.allowed.len() > MAX_ALLOWED_CATEGORIES {
            return Err(FormationError::TooMany {
                what: "allowed categories",
                actual: self.allowed.len(),
                max: MAX_ALLOWED_CATEGORIES,
            });
        }
        if self.required_command.len() > MAX_COMMAND_ROLES
            || self.required_support.len() > MAX_COMMAND_ROLES
        {
            return Err(FormationError::TooMany {
                what: "required roles",
                actual: self.required_command.len().max(self.required_support.len()),
                max: MAX_COMMAND_ROLES,
            });
        }
        for role in self.required_command.iter().chain(&self.required_support) {
            if role.id.trim().is_empty() || role.id.len() > MAX_LABEL_BYTES {
                return Err(FormationError::BadLabel);
            }
        }
        for category in &self.allowed {
            if category.0.trim().is_empty() || category.0.len() > MAX_LABEL_BYTES {
                return Err(FormationError::BadLabel);
            }
        }
        Ok(())
    }
}

/// **When** a constraint set is enforced (D-E7-19). Never to existence, never to
/// the ability to deploy.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Strictness {
    /// At `@force/create`: the node must be constructible under the kind.
    Creation,
    /// At doctrine qualification: "is this still a full squadron?"
    DoctrineQualification,
    /// As an operation's own requirement at `@op/join`.
    OperationRequirement,
}

impl Strictness {
    /// Every scope, so a test can enumerate the arms by name.
    pub const ALL: [Strictness; 3] = [
        Strictness::Creation,
        Strictness::DoctrineQualification,
        Strictness::OperationRequirement,
    ];

    /// The staff-facing word.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Strictness::Creation => "creation",
            Strictness::DoctrineQualification => "doctrine",
            Strictness::OperationRequirement => "operation",
        }
    }
}

/// A ruleset-authored formation kind (T8). The engine stores the generic shape;
/// `tacsim` supplies the names and doctrine through the
/// [`FormationAuthority`] composition seam.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "FormationKindWire", deny_unknown_fields)]
pub struct FormationKind {
    /// The kind's id (`lance`, `star`, `squadron`).
    pub id: SmolStr,
    /// Its branch/domain (`mech`, `aero`, `naval`).
    pub branch: SmolStr,
    /// Its echelon. A lance and a star **share** one and stay distinct kinds.
    pub echelon: u8,
    /// The constraint set.
    pub composition: Composition,
    /// The command roles the kind names.
    pub command_roles: Vec<RoleSpec>,
}

/// One node of a persistent order of battle.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "FormationNodeWire", deny_unknown_fields)]
pub struct FormationNode {
    /// Identity.
    pub id: FormationId,
    /// Which [`FormationKind`] it instantiates.
    pub kind: SmolStr,
    /// **Exactly one** owning org compartment (R2). Not an `Option`: a node with
    /// no scope is unconstructible, which is the invariant rather than a check.
    pub scope: OrgScopeId,
    /// Its **administrative** parent. Attachment to an operation never changes it.
    pub parent: Option<FormationId>,
    /// Its direct members (≤ [`MAX_FORMATION_MEMBERS`]).
    pub members: Vec<UnitRef>,
    /// Staleness stamp.
    pub revision: u64,
}

/// What a formation failed on.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FormationError {
    /// A set is over its cap.
    #[error("{what} is over its cap ({actual} > {max})")]
    TooMany {
        /// Which set.
        what: &'static str,
        /// How many.
        actual: usize,
        /// The cap.
        max: usize,
    },
    /// A label is empty or too long.
    #[error("a formation label is empty or longer than {MAX_LABEL_BYTES} bytes")]
    BadLabel,
    /// `minimum`/`maximum`/`preferred` do not describe a range.
    #[error("the composition's minimum, maximum and preferred do not describe a range")]
    CompositionRange,
    /// The same unit is listed twice.
    #[error("a unit is listed twice in one formation")]
    DuplicateMember,
    /// A node names itself as its parent.
    #[error("a formation cannot be its own parent")]
    SelfParent,
    /// The id is zero.
    #[error("a formation id is never zero")]
    ZeroId,
    /// The tree is deeper than [`MAX_FORMATION_DEPTH`], or cycles.
    #[error("the administrative tree is deeper than {MAX_FORMATION_DEPTH}, or cycles")]
    TooDeep,
}

/// Which constraints a node fails, under one scope of strictness.
///
/// **A failure is never a refusal to exist or to deploy.** `Creation` is the only
/// scope `@force/create` consults; the other two are questions a caller asks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Qualification {
    /// Fewer members than `minimum`.
    pub understrength: bool,
    /// More members than `maximum`.
    pub overstrength: bool,
    /// A required command role is unfilled (the roles a node fills are cut 10's;
    /// cut 9 reports the *requirement*, never invents a filled role).
    pub missing_command: Vec<SmolStr>,
    /// A required support role is unfilled.
    pub missing_support: Vec<SmolStr>,
}

impl Qualification {
    /// Whether the node satisfies the constraint set completely.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        !self.understrength
            && !self.overstrength
            && self.missing_command.is_empty()
            && self.missing_support.is_empty()
    }
}

impl FormationNode {
    /// Re-run every documented invariant (the wire calls this).
    pub fn check(&self) -> Result<(), FormationError> {
        if self.id.0 == 0 {
            return Err(FormationError::ZeroId);
        }
        if self.kind.trim().is_empty() || self.kind.len() > MAX_LABEL_BYTES {
            return Err(FormationError::BadLabel);
        }
        if self.members.len() > MAX_FORMATION_MEMBERS {
            return Err(FormationError::TooMany {
                what: "formation members",
                actual: self.members.len(),
                max: MAX_FORMATION_MEMBERS,
            });
        }
        if self.parent == Some(self.id) {
            return Err(FormationError::SelfParent);
        }
        let mut seen: BTreeSet<UnitRef> = BTreeSet::new();
        for unit in &self.members {
            if !seen.insert(*unit) {
                return Err(FormationError::DuplicateMember);
            }
        }
        Ok(())
    }

    /// **An understrength permanent formation exists and deploys.** This is a
    /// method rather than an absence so the claim is testable in one place.
    #[must_use]
    pub fn may_deploy(&self) -> bool {
        true
    }

    /// Evaluate the node against a kind's constraint set under one scope of
    /// strictness. The *same* set is evaluated in every scope — what differs is
    /// who asks and what they do with the answer, which is exactly D-E7-19.
    #[must_use]
    pub fn qualify(&self, kind: &FormationKind, _scope: Strictness) -> Qualification {
        let n = self.members.len();
        Qualification {
            understrength: n < usize::from(kind.composition.minimum),
            overstrength: n > usize::from(kind.composition.maximum),
            missing_command: kind
                .composition
                .required_command
                .iter()
                .filter(|r| r.minimum > 0)
                .map(|r| r.id.clone())
                .collect(),
            missing_support: kind
                .composition
                .required_support
                .iter()
                .filter(|r| r.minimum > 0)
                .map(|r| r.id.clone())
                .collect(),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FormationKindWire {
    id: SmolStr,
    branch: SmolStr,
    echelon: u8,
    composition: Composition,
    #[serde(default)]
    command_roles: Vec<RoleSpec>,
}

impl TryFrom<FormationKindWire> for FormationKind {
    type Error = FormationError;

    fn try_from(w: FormationKindWire) -> Result<Self, Self::Error> {
        let kind = FormationKind {
            id: w.id,
            branch: w.branch,
            echelon: w.echelon,
            composition: w.composition,
            command_roles: w.command_roles,
        };
        if kind.id.trim().is_empty()
            || kind.id.len() > MAX_LABEL_BYTES
            || kind.branch.trim().is_empty()
            || kind.branch.len() > MAX_LABEL_BYTES
        {
            return Err(FormationError::BadLabel);
        }
        if kind.command_roles.len() > MAX_COMMAND_ROLES {
            return Err(FormationError::TooMany {
                what: "command roles",
                actual: kind.command_roles.len(),
                max: MAX_COMMAND_ROLES,
            });
        }
        for role in &kind.command_roles {
            if role.id.trim().is_empty() || role.id.len() > MAX_LABEL_BYTES {
                return Err(FormationError::BadLabel);
            }
        }
        kind.composition.check()?;
        Ok(kind)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FormationNodeWire {
    id: FormationId,
    kind: SmolStr,
    scope: OrgScopeId,
    #[serde(default)]
    parent: Option<FormationId>,
    members: Vec<UnitRef>,
    revision: u64,
}

impl TryFrom<FormationNodeWire> for FormationNode {
    type Error = FormationError;

    fn try_from(w: FormationNodeWire) -> Result<Self, Self::Error> {
        let node = FormationNode {
            id: w.id,
            kind: w.kind,
            scope: w.scope,
            parent: w.parent,
            members: w.members,
            revision: w.revision,
        };
        node.check()?;
        Ok(node)
    }
}

/// The ruleset seam formation **kinds** arrive through (OPEN 4), copied from
/// `mush_industry::RecipeAuthority` / `IndustryModule::with_recipes`: the impl
/// lives at the composition root and reads the ruleset's own store, so
/// `mush-campaign` gains no ruleset edge and `tacsim` gains no campaign
/// edge. Unwired, the campaign layer knows no kinds and `@force/create` refuses —
/// the fail-closed default.
pub trait FormationAuthority: Send + Sync + std::fmt::Debug {
    /// Every kind this world's published packs author, in id order. Read at
    /// command time, where a world is available — never at prepare time.
    fn kinds(&self, world: &World) -> Vec<FormationKind>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lance() -> FormationKind {
        FormationKind {
            id: SmolStr::new_static("lance"),
            branch: SmolStr::new_static("mech"),
            echelon: 1,
            composition: Composition {
                minimum: 4,
                maximum: 4,
                preferred: 4,
                allowed: BTreeSet::new(),
                required_command: Vec::new(),
                required_support: Vec::new(),
            },
            command_roles: Vec::new(),
        }
    }

    fn star() -> FormationKind {
        FormationKind {
            id: SmolStr::new_static("star"),
            branch: SmolStr::new_static("mech"),
            echelon: 1,
            composition: Composition {
                minimum: 5,
                maximum: 5,
                preferred: 5,
                allowed: BTreeSet::new(),
                required_command: Vec::new(),
                required_support: Vec::new(),
            },
            command_roles: Vec::new(),
        }
    }

    fn node(members: usize) -> FormationNode {
        FormationNode {
            id: FormationId(1),
            kind: SmolStr::new_static("lance"),
            scope: OrgScopeId(1),
            parent: None,
            members: (0..members)
                .map(|n| UnitRef(mush_core::ObjId(n as u32 + 1)))
                .collect(),
            revision: 1,
        }
    }

    #[test]
    fn e7c9_b4_a_lance_and_a_star_share_an_echelon_and_stay_distinct_kinds() {
        assert_eq!(lance().echelon, star().echelon);
        assert_ne!(lance().id, star().id);
        assert_ne!(lance().composition, star().composition);
        // …and `exact` is the derived predicate, not a mode.
        assert!(lance().composition.is_exact());
        assert!(star().composition.is_exact());
        let loose = Composition {
            minimum: 3,
            maximum: 6,
            preferred: 4,
            ..Composition::default()
        };
        assert!(!loose.is_exact());
    }

    #[test]
    fn e7c9_b4_an_understrength_formation_exists_and_deploys_in_every_strictness_scope() {
        let understrength = node(2);
        assert!(understrength.check().is_ok(), "it exists");
        assert!(understrength.may_deploy(), "and it deploys");
        // The SAME constraint set reports the SAME failure in every scope — the
        // scope decides who asks, never whether the node exists (every arm named).
        for scope in Strictness::ALL {
            let q = understrength.qualify(&lance(), scope);
            assert!(q.understrength, "{}", scope.label());
            assert!(!q.is_clean(), "{}", scope.label());
        }
        let full = node(4);
        for scope in Strictness::ALL {
            assert!(
                full.qualify(&lance(), scope).is_clean(),
                "{}",
                scope.label()
            );
        }
        // Over the maximum is the other end of the same set.
        assert!(node(5).qualify(&lance(), Strictness::Creation).overstrength);
    }

    #[test]
    fn e7c9_b4_a_stored_membership_over_the_cap_refuses_on_load() {
        let over = node(MAX_FORMATION_MEMBERS + 1);
        let value = serde_json::to_value(&over).expect("serializes");
        let error = serde_json::from_value::<FormationNode>(value).expect_err("refuses");
        assert!(error.to_string().contains("over its cap"), "{error}");
        // And a duplicate member is refused on load too.
        let mut dupe = node(2);
        dupe.members[1] = dupe.members[0];
        let value = serde_json::to_value(&dupe).expect("serializes");
        assert!(serde_json::from_value::<FormationNode>(value).is_err());
    }

    #[test]
    fn e7c9_b4_a_node_with_no_scope_is_unconstructible() {
        // The type carries `OrgScopeId`, not `Option<OrgScopeId>`, so the only way
        // to express "no scope" is to omit the field — which the strict wire
        // refuses. That is the invariant, stated as a type and pinned here.
        let mut value = serde_json::to_value(node(1)).expect("serializes");
        value.as_object_mut().expect("object").remove("scope");
        assert!(serde_json::from_value::<FormationNode>(value).is_err());
    }

    #[test]
    fn e7c9_b4_a_kind_whose_composition_is_not_a_range_refuses_on_load() {
        let mut kind = lance();
        kind.composition.minimum = 5;
        kind.composition.maximum = 4;
        let value = serde_json::to_value(&kind).expect("serializes");
        assert!(serde_json::from_value::<FormationKind>(value).is_err());
    }
}
