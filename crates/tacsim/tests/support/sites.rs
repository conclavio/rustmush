//! **The mining sites stripped away** (doc 20 cut SM.2): helpers that rebuild the v8 lane shape
//! from the shipped v9 for the suites that rebuild an older lane — the six site ports, places and
//! twelve routes, Ceres's six site routes keys, the Freeport Registry org, the eight bodies'
//! surface figures and tables, and the Freeport's coordinates. Since doc 20 cut SM.6 also the
//! helpers that **open a site** for a keyed rig that hops to one (`open_site`, `open_sites`):
//! a keyed site's pad is closed until its clerk has opened every deposit.

use mush_core::ObjId;
use mush_testkit::TestWorld;
use serde_json::Value;

/// The six site port entries, in the lane's site order.
pub const SITE_PORTS: [&str; 6] = [
    "commons_port",
    "kerwan_port",
    "ahuna_port",
    "juling_port",
    "occator_port",
    "rheasilvia_port",
];
/// The six site places, in the same order.
pub const SITE_PLACES: [&str; 6] = [
    "freeport_commons",
    "kerwan_floor",
    "ahuna_flank",
    "juling_shadow",
    "occator_rim",
    "rheasilvia_commons",
];
/// The six venues, in the same order.
pub const SITE_VENUES: [&str; 6] = [
    "commons",
    "kerwan",
    "ahuna",
    "juling",
    "occator",
    "rheasilvia",
];
/// The twelve site routes, out and back per site.
pub const SITE_ROUTES: [&str; 12] = [
    "ceres_commons",
    "commons_ceres",
    "ceres_kerwan",
    "kerwan_ceres",
    "ceres_ahuna",
    "ahuna_ceres",
    "ceres_juling",
    "juling_ceres",
    "ceres_occator",
    "occator_ceres",
    "ceres_rheasilvia",
    "rheasilvia_ceres",
];
/// The bodies cut SM.2 gave surface figures.
pub const SURFACE_BODIES: [&str; 8] = [
    "ceres", "vesta", "pallas", "luna", "mars", "titan", "phobos", "deimos",
];

/// **Doc 32 cut S.1: the wide site port entries** (Urvara Reach, 512 a side).
pub const WIDE_SITE_PORTS: [&str; 1] = ["urvara_port"];
/// **Doc 32 cut S.1: the 512 site's rows** — Urvara Reach's place, its two routes and its port.
pub const WIDE_SITE_ROWS: [&str; 4] = [
    "urvara_reach",
    "ceres_urvara",
    "urvara_ceres",
    "urvara_port",
];

/// Remove what doc 32 cut S.1 added to the lane (lane v17): Urvara Reach's rows and Ceres's
/// `urvara` routes key — the lane v16 shape but its manifest.
pub fn strip_wide_sites(doc: &mut Value) {
    let entries = doc["entries"].as_object_mut().expect("the lane's entries");
    for id in WIDE_SITE_ROWS {
        entries.remove(id);
    }
    if let Some(routes) = entries
        .get_mut("ceres_port")
        .and_then(|ceres| ceres["def"]["routes"].as_object_mut())
    {
        routes.remove("urvara");
    }
}

/// Remove everything cut SM.2 added to the lane — the v8 shape (the manifest is the caller's) —
/// and, since lane v10, cut SM.3a's blocks (`super::vehicles::strip_vehicles`, first); since
/// lane v17, doc 32 cut S.1's 512 site ([`strip_wide_sites`]).
pub fn strip_sites(doc: &mut Value) {
    strip_wide_sites(doc);
    super::vehicles::strip_vehicles(doc);
    let entries = doc["entries"].as_object_mut().expect("the lane's entries");
    for id in SITE_PORTS
        .iter()
        .chain(SITE_PLACES.iter())
        .chain(SITE_ROUTES.iter())
    {
        entries.remove(*id);
    }
    for body in SURFACE_BODIES {
        if let Some(def) = entries
            .get_mut(body)
            .and_then(|entry| entry["def"].as_object_mut())
        {
            def.remove("surface_mg");
            def.remove("radius_km");
            def.remove("geology");
        }
    }
    if let Some(def) = entries
        .get_mut("ceres_freeport")
        .and_then(|entry| entry["def"].as_object_mut())
    {
        def.remove("lat_mdeg");
        def.remove("lon_mdeg");
    }
    if let Some(ceres) = entries.get_mut("ceres_port") {
        if let Some(routes) = ceres["def"]["routes"].as_object_mut() {
            for venue in SITE_VENUES {
                routes.remove(venue);
            }
        }
        if let Some(orgs) = ceres["def"]["orgs"].as_array_mut() {
            orgs.retain(|org| org["id"] != "freeport_registry");
        }
    }
}

// ---------------------------------------------------------------------------
// Doc 20 cut SM.6: opening a site (reviewer F13)
// ---------------------------------------------------------------------------

/// The `Site` readiness line of `entry`: its state and its text.
pub fn site_line(tw: &TestWorld, entry: &str) -> Option<(tacsim::port::LineState, String)> {
    tacsim::port::readiness(tw.world())
        .lines
        .into_iter()
        .find(|line| {
            line.binding == tacsim::port::Binding::Site
                && line.port.as_ref().map(mush_core::CanonicalId::as_str) == Some(entry)
        })
        .map(|line| (line.state, line.text))
}

/// Whether `entry`'s `Site` line reads Ready.
pub fn site_open(tw: &TestWorld, entry: &str) -> bool {
    site_line(tw, entry).is_some_and(|(state, _)| state == tacsim::port::LineState::Ready)
}

/// Every live site clerk of the shipped lane's sites.
fn site_clerks(tw: &TestWorld) -> std::collections::BTreeSet<ObjId> {
    let Some(store) = tw.world().module_state::<tacsim::TacsimStore>() else {
        return std::collections::BTreeSet::new();
    };
    let lane =
        mush_core::CanonicalId::new(tacsim::definitions::content::LANE_PACK).expect("the lane id");
    // Doc 32 cut S.1: and the wide sites' clerks.
    SITE_PORTS
        .iter()
        .chain(WIDE_SITE_PORTS.iter())
        .filter_map(|entry| {
            let entry = mush_core::CanonicalId::new(*entry).ok()?;
            match store.port_map(&lane, &entry) {
                tacsim::port::PortMapState::Present(map) => {
                    map.site.as_ref().and_then(|row| row.clerk).map(ObjId)
                }
                _ => None,
            }
        })
        .collect()
}

/// A fresh request/commit token per landing, across the process (its own range).
fn site_token() -> [u8; 16] {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut bytes = [0x5e_u8; 16];
    bytes[..4].copy_from_slice(&n.to_be_bytes());
    bytes
}

/// Drive one queued submission the way the world task does (regenerate the actor's own
/// offer, resolve, prepare, install); the typed post-images go to `keep`.
pub fn land_one(
    tw: &mut TestWorld,
    submit: &mush_core::AuthoritativeSubmit,
    keep: &mut dyn FnMut(Vec<mush_core::ModuleOp>),
) -> Result<(), String> {
    let token = site_token();
    let (action, resolved) = {
        let engine = tw.engine();
        let query = engine
            .legal_action_query(
                mush_core::PrincipalId::Object { id: submit.actor },
                submit.actor,
                submit.subject.clone(),
            )
            .map_err(|error| error.to_string())?;
        let offers = engine
            .project_legal_actions(&query)
            .map_err(|error| error.to_string())?;
        let projected = offers
            .projected_for_action(&submit.action)
            .ok_or_else(|| "that action is not currently offered".to_owned())?;
        let submission = mush_core::OfferSubmission {
            offer: projected.offer(),
            subject: projected.subject().clone(),
            targets: Vec::new(),
            parameters: submit.parameters.clone(),
        };
        let execution = mush_core::OfferExecutionContext {
            request: mush_core::RequestUid::from_bytes(token),
            principal: mush_core::PrincipalId::Object { id: submit.actor },
            actor: submit.actor,
            idempotency: mush_core::ScopedIdempotencyKey::new(
                mush_core::IdempotencyScope {
                    principal: mush_core::PrincipalId::Object { id: submit.actor },
                    owner: submit.action.key().owner.clone(),
                    namespace: mush_core::CanonicalId::new(submit.action.key().id.as_str())
                        .map_err(|_| "namespace".to_owned())?,
                },
                1,
                mush_core::IdempotencyKey::Request(mush_core::RequestUid::from_bytes(token)),
            ),
            viewer_scope: query.viewer_scope(),
            now: None,
        };
        let resolved = engine
            .resolve_legal_action(&offers, execution, submission)
            .map_err(|error| format!("{error:?}"))?;
        let (action, resolved, _guards) = resolved.into_authoritative_parts();
        (action, resolved)
    };
    let (prepared, _) = tw
        .engine_mut()
        .prepare_registered_resolved_authoritative_action(
            &action,
            &resolved,
            mush_core::CommitToken::from_bytes(token),
        )
        .map_err(|error| format!("{error:?}"))?;
    let (ops, install) = prepared.into_parts();
    tw.engine_mut()
        .install_prepared_action(install)
        .map_err(|error| format!("{error:?}"))?;
    keep(ops);
    Ok(())
}

/// One port fire (6 s on): every queued submission taken; the site clerks' landed (each must
/// land), every other dropped — every other arm verifies by pull and submits again on a later
/// fire (D-18-32). Returns how many site acts landed.
pub fn site_fire(tw: &mut TestWorld, keep: &mut dyn FnMut(Vec<mush_core::ModuleOp>)) -> u32 {
    let t = tw.world().now().saturating_add(6);
    tw.set_now(t);
    let _ = tw.fire_due(t.saturating_mul(1_000));
    let clerks = site_clerks(tw);
    let mut queued = Vec::new();
    while let Some(submit) = tw.engine_mut().take_authoritative_submit() {
        if submit.action.key().owner.as_str() == tacsim::NS {
            assert_ne!(
                submit.actor,
                mush_core::GOD,
                "GOD never submits from a tick"
            );
        }
        if clerks.contains(&submit.actor) {
            queued.push(*submit);
        }
    }
    let mut landed = 0_u32;
    for submit in &queued {
        land_one(tw, submit, keep).unwrap_or_else(|error| {
            panic!(
                "{} by site clerk #{} did not land: {error}",
                submit.action.key().id,
                submit.actor.0
            )
        });
        landed = landed.saturating_add(1);
    }
    landed
}

/// **Open one site** (doc 20 cut SM.6, reviewer F13): port fires — each also a surface fire,
/// which warms one wanted site — until **that** site's `Site` line reads Ready, landing the
/// site clerks' acts, bounded at 200 port fires (asserted); the port fires taken.
pub fn open_site(tw: &mut TestWorld, entry: &str) -> u32 {
    open_site_keeping(tw, entry, &mut |_| {})
}

/// [`open_site`], the landed bundles' typed post-images handed to `keep` (a rig that reboots
/// over its own typed store).
pub fn open_site_keeping(
    tw: &mut TestWorld,
    entry: &str,
    keep: &mut dyn FnMut(Vec<mush_core::ModuleOp>),
) -> u32 {
    for fires in 0..200_u32 {
        if site_open(tw, entry) {
            return fires;
        }
        let _ = site_fire(tw, keep);
    }
    assert!(
        site_open(tw, entry),
        "{entry} did not open within 200 port fires: {:?}",
        site_line(tw, entry)
    );
    200
}

/// **Open every lane site** (reviewer F13): as [`open_site`] for all six at once, bounded at
/// 300 port fires (asserted); the port fires each site took to read Ready.
pub fn open_sites(tw: &mut TestWorld) -> std::collections::BTreeMap<String, u32> {
    open_sites_keeping(tw, &mut |_| {})
}

/// [`open_sites`], the typed post-images handed to `keep`.
pub fn open_sites_keeping(
    tw: &mut TestWorld,
    keep: &mut dyn FnMut(Vec<mush_core::ModuleOp>),
) -> std::collections::BTreeMap<String, u32> {
    let mut taken: std::collections::BTreeMap<String, u32> = std::collections::BTreeMap::new();
    for fires in 0..=300_u32 {
        for entry in SITE_PORTS {
            if !taken.contains_key(entry) && site_open(tw, entry) {
                taken.insert(entry.to_owned(), fires);
            }
        }
        if taken.len() == SITE_PORTS.len() || fires == 300 {
            break;
        }
        let _ = site_fire(tw, keep);
    }
    for entry in SITE_PORTS {
        assert!(
            taken.contains_key(entry),
            "{entry} did not open within 300 port fires: {:?}",
            site_line(tw, entry)
        );
    }
    taken
}
