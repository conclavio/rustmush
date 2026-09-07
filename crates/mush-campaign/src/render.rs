//! The one internal view every projection reads (E7 cut 9; §4 rule 4, lens A).
//!
//! T6's rule: **"declared" means every party entitled to defend receives sufficient
//! notice and knows the window — not that the attacker's plan or identity is
//! published.** `EconomicVisibility::Audience { members, announce_issuer }` with
//! `announce_issuer` defaulting to `false` is already exactly that class, so cut 9
//! carries the economy's own type rather than inventing a second visibility model.
//!
//! ## Existence-hiding, byte for byte
//!
//! The existence-hiding renderer treats an operation id **like an account id**
//! (E3b2's D-E3-6 rule): a viewer who may not see an operation gets
//! [`NO_SUCH_OPERATION`], and so does a viewer asking about an id that never
//! existed — the same bytes, **at every verbosity**. There is one refusal constant
//! and one lens function, so the two answers cannot drift apart.

use mush_core::{ObjId, Role, World};
use mush_economy::EconomicVisibility;
use mush_org::PrincipalId;

use crate::record::{OperationDoc, OperationState};
use crate::store::CampaignStore;

/// The one refusal. A viewer who may not see an operation and a viewer naming an id
/// that never existed receive **these exact bytes**, at every verbosity.
pub const NO_SUCH_OPERATION: &str = "No such operation.";

/// The one refusal for a formation.
pub const NO_SUCH_FORMATION: &str = "No such formation.";

/// How much of an operation a viewer may see. Computed **once**, by
/// [`lens_for`], and handed to every renderer — there is no second place that
/// decides who sees what.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Lens {
    /// Nothing at all: the operation is indistinguishable from one that does not
    /// exist.
    Hidden,
    /// T6's "notice of the window, not the plan": the window and the notice, the
    /// issuer only when `announce_issuer` is set.
    Notice,
    /// Everything: staff, a side's principal, or a committed unit's controller.
    Plan,
}

/// How verbose a render is. The refusal bytes are identical across all three,
/// which is the point of enumerating them.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Verbosity {
    /// One line.
    Brief,
    /// The default listing.
    Normal,
    /// Everything the lens admits.
    Full,
}

impl Verbosity {
    /// Every arm, so a leak test can enumerate them by name.
    pub const ALL: [Verbosity; 3] = [Verbosity::Brief, Verbosity::Normal, Verbosity::Full];
}

/// Staff: `SIM_ADMIN` or `Royalty` — deliberately the same predicate
/// `mush-economy` and `mush-housing` use, so "staff" means one thing.
#[must_use]
pub fn is_staff(w: &World, who: ObjId) -> bool {
    mush_core::perm::has_power(w, who, mush_core::Powers::SIM_ADMIN)
        || mush_core::perm::has_role(w, who, Role::Royalty)
}

/// Whether `who` acts for `principal` — the object itself, an object it controls,
/// or staff. Deliberately narrower than the economy's `acts_for`: this decides
/// disclosure, not money.
#[must_use]
fn acts_for(w: &World, who: ObjId, principal: &PrincipalId) -> bool {
    if is_staff(w, who) {
        return true;
    }
    match principal {
        PrincipalId::Object(o) => *o == who || mush_core::perm::controls(w, who, *o),
        // An organization's disclosure rights are its officers', and asking that
        // question needs `mush_org::authorize` with a capability nobody has yet
        // declared for *reading an operation*. Fail closed rather than invent one.
        PrincipalId::Organization(_) | PrincipalId::System(_) => false,
    }
}

/// **The one lens function.** Every projection — `@op/show`, `@op/consumers`, the
/// listing, and any future feed — asks this and nothing else.
#[must_use]
pub fn lens_for(w: &World, doc: &OperationDoc, who: ObjId) -> Lens {
    if is_staff(w, who) {
        return Lens::Plan;
    }
    if acts_for(w, who, &doc.terms.attacker) {
        return Lens::Plan;
    }
    if doc
        .terms
        .sides
        .iter()
        .any(|side| acts_for(w, who, &side.principal))
    {
        return Lens::Plan;
    }
    // A committed unit's controller is inside the operation and sees the plan.
    if doc
        .runtime
        .participants
        .iter()
        .any(|p| p.unit.0 == who || mush_core::perm::controls(w, who, p.unit.0))
    {
        return Lens::Plan;
    }
    let audience_member = doc
        .terms
        .visibility
        .members()
        .iter()
        .chain(doc.terms.eligible_defenders.members.iter())
        .any(|p| acts_for(w, who, p));
    match (&doc.terms.visibility, audience_member) {
        // Public is a notice to everyone. It is deliberately NOT the plan.
        (EconomicVisibility::Public, _) => Lens::Notice,
        (EconomicVisibility::Audience { .. } | EconomicVisibility::Parties, true) => Lens::Notice,
        // Staff-only visibility is T6's "covert existence": nobody outside the
        // operation learns it exists.
        _ => Lens::Hidden,
    }
}

/// Whether the audience's notice may name the issuer (E4b2 verify fold A7): the
/// default is `false`, and who is running an operation is exactly the thing a
/// defender learning about it must not get for free.
#[must_use]
fn announces_issuer(visibility: &EconomicVisibility) -> bool {
    match visibility {
        EconomicVisibility::Audience {
            announce_issuer, ..
        } => *announce_issuer,
        // A public operation names its issuer; the other classes do not disclose
        // to anyone who is not already inside.
        EconomicVisibility::Public => true,
        _ => false,
    }
}

/// Render one operation under a lens. **[`Lens::Hidden`] renders the refusal
/// bytes, at every verbosity**, so a covert operation and an id that never existed
/// are one answer.
#[must_use]
pub fn render(doc: &OperationDoc, lens: Lens, verbosity: Verbosity) -> Vec<String> {
    if lens == Lens::Hidden {
        return vec![NO_SUCH_OPERATION.to_owned()];
    }
    let id = doc.id();
    let mut out = Vec::new();
    let issuer = if announces_issuer(&doc.terms.visibility) || lens == Lens::Plan {
        format!(" declared by {}", principal_label(&doc.terms.attacker))
    } else {
        String::new()
    };
    out.push(format!(
        "Operation {id}{issuer}: window {}-{}, notice {}.",
        doc.terms.window.0.at, doc.terms.window.1.at, doc.terms.notice.at
    ));
    if verbosity == Verbosity::Brief {
        return out;
    }
    if lens == Lens::Notice {
        // The window and the notice, never the plan: no objective, no legal
        // targets, no stakes, no roster, no bond, no state machine.
        out.push(
            "You are entitled to defend within that window. No further detail is disclosed."
                .to_owned(),
        );
        return out;
    }
    out.push(format!(
        "State: {}{}.",
        doc.runtime.state.label(),
        doc.runtime
            .paused
            .as_ref()
            .map(|p| format!(" (paused: {})", p.label()))
            .unwrap_or_default()
    ));
    out.push(format!("Objective: {}.", doc.terms.objective.summary));
    out.push(format!(
        "Participants: {} of {}.",
        doc.runtime.participants.len(),
        crate::record::MAX_OPERATION_PARTICIPANTS
    ));
    if verbosity == Verbosity::Normal {
        return out;
    }
    out.push(format!(
        "Legal targets: {}.",
        if doc.terms.legal_targets.is_empty() {
            "none declared".to_owned()
        } else {
            doc.terms
                .legal_targets
                .iter()
                .map(smol_str::SmolStr::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        }
    ));
    out.push(format!("Stakes: {}.", doc.terms.stakes.len()));
    match &doc.terms.bond {
        Some(bond) => out.push(format!(
            "Bond: reserved on account #{}{}.",
            bond.account.0,
            bond.hold
                .map(|h| format!(" as hold #{}", h.0))
                .unwrap_or_else(|| " (not yet placed)".to_owned())
        )),
        None => out.push("Bond: none.".to_owned()),
    }
    if let OperationState::Resolving { pending } = &doc.runtime.state {
        out.push(format!(
            "Resolving: {} consumer(s) outstanding.",
            pending.len()
        ));
    }
    out
}

/// A staff-safe principal label. Never an account id.
#[must_use]
pub fn principal_label(p: &PrincipalId) -> String {
    match p {
        PrincipalId::Object(o) => format!("#{}", o.0),
        PrincipalId::Organization(org) => format!("org:{}", org.0),
        PrincipalId::System(s) => format!("system:{}", s.as_str()),
    }
}

/// Look one operation up **through the lens**: `None` is the caller's cue to print
/// [`NO_SUCH_OPERATION`], and it is returned identically for "hidden" and "absent".
#[must_use]
pub fn visible_operation(
    w: &World,
    id: crate::ids::OperationId,
    who: ObjId,
) -> Option<(OperationDoc, Lens)> {
    let doc = w.module_state::<CampaignStore>()?.operation(id)?.clone();
    match lens_for(w, &doc, who) {
        Lens::Hidden => None,
        lens => Some((doc, lens)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::OperationId;
    use crate::record::OperationRuntime;
    use crate::terms::operation_consent_hash;
    use crate::testfix::terms_fixture;

    fn covert(id: OperationId) -> OperationDoc {
        let mut terms = terms_fixture(id);
        terms.visibility = EconomicVisibility::Staff;
        terms.consent_hash = operation_consent_hash(&terms);
        OperationDoc {
            terms,
            runtime: OperationRuntime::declared(id),
        }
    }

    #[test]
    fn e7c9_b8_a_hidden_lens_renders_the_refusal_bytes_at_every_verbosity() {
        let doc = covert(OperationId(1));
        for verbosity in Verbosity::ALL {
            assert_eq!(
                render(&doc, Lens::Hidden, verbosity),
                vec![NO_SUCH_OPERATION.to_owned()],
                "{verbosity:?}"
            );
        }
    }

    #[test]
    fn e7c9_b8_a_notice_lens_carries_the_window_and_never_the_plan() {
        let mut doc = covert(OperationId(1));
        doc.terms.visibility = EconomicVisibility::Audience {
            members: vec![PrincipalId::Object(ObjId(200))],
            announce_issuer: false,
        };
        doc.terms.consent_hash = operation_consent_hash(&doc.terms);
        for verbosity in Verbosity::ALL {
            let lines = render(&doc, Lens::Notice, verbosity).join("\n");
            assert!(lines.contains("window"), "{verbosity:?}: no window");
            // The plan: the objective, the targets, the stakes, the roster, the
            // issuer, the bond — none of it, at any verbosity.
            for leak in [
                doc.terms.objective.summary.as_str(),
                "structure",
                "Bond",
                "Participants",
                "declared by",
                "State:",
            ] {
                assert!(
                    !lines.contains(leak),
                    "{verbosity:?}: the notice leaked {leak:?}"
                );
            }
        }
        // …and with `announce_issuer` set, the issuer — and only the issuer —
        // joins it.
        doc.terms.visibility = EconomicVisibility::Audience {
            members: vec![PrincipalId::Object(ObjId(200))],
            announce_issuer: true,
        };
        let lines = render(&doc, Lens::Notice, Verbosity::Full).join("\n");
        assert!(lines.contains("declared by #100"));
        assert!(!lines.contains("Bond"));
    }
}
