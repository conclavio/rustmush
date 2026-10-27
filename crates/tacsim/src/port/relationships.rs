//! Public desk affiliations for the two authored Ceres service representatives.
//!
//! These roles are public in their authored backgrounds. They are deliberately
//! independent of the informant principal, faction membership, and intel rights.
//! Bind by content identity and the actual port organization, never by a name
//! match or the port definition's currently unimplemented polity label.

use mush_core::{Attr, CanonicalId, Ctx, Flags, ObjId, ObjKind, World};
use mush_faction::FactionId;
use mush_org::OrgId;

const CERES: &str = "neighborhood.ceres_freeport";
const BINDING_ATTR: &str = "tacsim.public_desk_binding";

fn desk(w: &World, entry: &str) -> Option<ObjId> {
    let pack = CanonicalId::new(CERES).ok()?;
    let entry = CanonicalId::new(entry).ok()?;
    let applied = w
        .module_state::<mush_content::ContentState>()?
        .latches
        .applied(&pack)?;
    if applied.is_retired() || applied.anomaly().is_some() {
        return None;
    }
    let npc = applied.entries().get(&entry)?.object()?;
    w.get(npc)
        .filter(|object| object.kind == ObjKind::Npc && !object.flags.contains(Flags::GOING))
        .map(|_| npc)
}

/// Install each public role once. The marker deliberately survives later staff
/// edits to the profile, including clearing the affiliation; a tick must not
/// silently put a former employee back on the payroll. It is not a membership.
pub(crate) fn reconcile(ctx: &mut Ctx<'_>) -> Result<(), String> {
    let Some(port) = super::coop::programme_port(ctx.w) else {
        return Ok(());
    };
    let bindings: Vec<_> = [
        ("npc.oduya", port.programme.org.as_str()),
        ("npc.qureshi", "occator"),
    ]
    .into_iter()
    .filter_map(|(entry, organization)| {
        let npc = desk(ctx.w, entry)?;
        if ctx.w.attr(npc, BINDING_ATTR).is_some() {
            return None;
        }
        let organization = CanonicalId::new(organization).ok()?;
        let row = port.map.orgs.get(&organization)?;
        Some((npc, OrgId(row.org)))
    })
    .collect();
    for (npc, org) in bindings {
        let faction = mush_faction::ensure_organization_faction(ctx, org)?;
        // A prior explicit staff affiliation wins over this one-time default.
        if mush_npc::public_faction_for(ctx.w, npc).is_none() {
            mush_npc::set_public_faction(ctx, npc, Some(faction))?;
        }
        ctx.w
            .set_attr(
                npc,
                BINDING_ATTR,
                Attr::json(serde_json::json!({"schema": 1, "org": org}), mush_core::GOD),
            )
            .map_err(|error| error.to_string())?;
        ctx.out.pin_deferred();
    }
    Ok(())
}

/// Only the named public survey factor learns about a completed commission.
/// Affiliation alone is not grounds for making every employee personally trust
/// someone they have never dealt with. A removed or reassigned factor is omitted.
pub(crate) fn survey_representative(w: &World, faction: FactionId) -> Option<ObjId> {
    let affiliated = w
        .module_state::<mush_faction::FactionState>()?
        .factions
        .get(&faction)?;
    if affiliated.hidden {
        return None;
    }
    let npc = desk(w, "npc.qureshi")?;
    (mush_npc::public_faction_for(w, npc) == Some(faction)).then_some(npc)
}
