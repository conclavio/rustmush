//! E7c9 B4 (OPEN 4) — the `FormationAuthority` implementation, hosted at the
//! composition root so **`mush-campaign` gains no `ruleset-tacsim` edge and
//! `ruleset-tacsim` gains no `mush-campaign` edge**. It is
//! `crate::industry_recipes::TacsimRecipeAuthority`'s shape exactly: the impl reads
//! the ruleset's published packs through `TacsimStore` and hands the campaign layer
//! the generic kinds it stores.
//!
//! Unwired — a server with no TacSim layer, or with no published pack authoring a
//! formation — the campaign layer knows no kinds at all and `@force/create`
//! refuses. That is the fail-closed default, not an oversight.
//!
//! Every kind crosses this boundary through `mush_campaign::FormationKind`'s own
//! `TryFrom` wire, so a pack-authored composition that the engine would refuse on
//! load is **dropped here with a warning** rather than installed: the pack's own
//! `PackEntry::check` already refused the same shapes on the way in, and this is the
//! second, independent check the engine's constructor performs on data it did not
//! author.

use mush_campaign::{
    Composition, FormationAuthority, FormationKind, MAX_ALLOWED_CATEGORIES, MAX_COMMAND_ROLES,
    RoleSpec,
};
use mush_core::World;
use ruleset_tacsim::{FormationKindDef, FormationRoleDef, TacsimStore};
use smol_str::SmolStr;

/// The one formation authority, handed to `CampaignModule::with_formation_kinds`
/// at composition. Holds nothing: it reads the per-world `TacsimStore`.
#[derive(Clone, Copy, Debug, Default)]
pub struct TacsimFormationAuthority;

fn roles(defs: &[FormationRoleDef]) -> Vec<RoleSpec> {
    defs.iter()
        .take(MAX_COMMAND_ROLES)
        .map(|r| RoleSpec {
            id: SmolStr::new(r.role.as_str()),
            minimum: r.minimum,
        })
        .collect()
}

/// Convert one pack-authored definition into the engine's generic kind, **through
/// the engine's own `TryFrom` wire**. `None` for a definition the engine would
/// refuse on load.
#[must_use]
pub fn kind_from(id: &str, def: &FormationKindDef) -> Option<FormationKind> {
    let candidate = FormationKind {
        id: SmolStr::new(id),
        branch: SmolStr::new(def.branch.as_str()),
        echelon: def.echelon,
        composition: Composition {
            minimum: def.minimum,
            maximum: def.maximum,
            preferred: def.preferred,
            allowed: def
                .allowed
                .iter()
                .take(MAX_ALLOWED_CATEGORIES)
                .map(|c| mush_campaign::CategoryId(SmolStr::new(c.as_str())))
                .collect(),
            required_command: roles(&def.command_roles),
            required_support: roles(&def.required_support),
        },
        command_roles: roles(&def.command_roles),
    };
    // Round-trip through the wire so the engine's constructor — not this file —
    // decides what is a legal kind.
    let value = serde_json::to_value(&candidate).ok()?;
    serde_json::from_value::<FormationKind>(value).ok()
}

impl FormationAuthority for TacsimFormationAuthority {
    fn kinds(&self, world: &World) -> Vec<FormationKind> {
        let Some(store) = world.module_state::<TacsimStore>() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for (id, def) in store.formation_kinds() {
            match kind_from(id.as_str(), &def) {
                Some(kind) => out.push(kind),
                None => tracing::warn!(
                    target: "campaign",
                    kind = %id,
                    "campaign: a pack-authored formation kind was refused by the engine's own \
                     constructor and is not installed"
                ),
            }
        }
        out
    }
}
