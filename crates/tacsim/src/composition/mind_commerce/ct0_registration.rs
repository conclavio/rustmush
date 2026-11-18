#![cfg(test)]

//! Doc 46 unit CT.0: the composed registration of the mind-commerce families,
//! pinned as committed text (`ct0_registration.snap` beside this file).
//!
//! The family crates pin their own records, keys and digests over fixed
//! fixture manifests. What only a composed engine shows is the registration:
//! every definition the owners declared (binding descriptors, policies,
//! actions, outcomes, owner reads and participants, with each payload and
//! reference list), every state family, every action provider and its offer's
//! presentation, every command, tick and continuation delegation.
//!
//! It pins names and shapes, never a digest: every commerce digest inherits
//! the TacSim registry manifest, which other programmes change. A definition
//! reference is written `owner/kind/id@vN`, a `{key, version, digest}` object
//! inside a payload loses its `digest`, a string of 32 or more hex digits is
//! replaced, and the manifest's own payload (the TacSim catalogue) is not
//! written. The test fails if any written line still holds 32 hex digits.
//!
//! Each table is written as its difference from a commerce-free composition
//! of the same modules (Org, Market, Economy, Roll, TacSim), so a line names
//! what commerce registered and an unrelated module's change moves no line.
//! The widest chain, the House
//! chain followed by the player market in the launch's order
//! (`crate::launch`), is written in full detail; each narrower configuration
//! the neighbouring tests compose is written with its definition references
//! only and the same command, tick and state-family tables.

use super::*;
use mush_testkit::snapshot::Snapshot;
use std::collections::BTreeSet;

const FILE: &str = "src/composition/mind_commerce/ct0_registration.snap";

type Error = Box<dyn std::error::Error>;
/// One entry of a table: its labelled lines, the first one naming it.
type Record = Vec<(String, String)>;

fn json<T: serde::Serialize + ?Sized>(value: &T) -> Result<String, Error> {
    Ok(serde_json::to_string(value)?)
}
fn one(label: &str, value: String) -> Record {
    vec![(label.to_owned(), value)]
}
/// A definition reference by name: `owner/kind/id@vN`, no digest.
fn name(reference: &DefinitionRef) -> String {
    let key = reference.key();
    format!(
        "{}/{}/{}@v{}",
        key.owner.as_str(),
        key.kind.as_str(),
        key.id.as_str(),
        reference.version()
    )
}
fn names(references: &[DefinitionRef]) -> String {
    let all: Vec<String> = references.iter().map(name).collect();
    format!("[{}]", all.join(", "))
}
fn some_name(reference: Option<&DefinitionRef>) -> String {
    reference.map_or_else(|| "none".to_owned(), name)
}
/// The placeholder a stripped digest-shaped string is written as.
const STRIPPED: &str = "<digest stripped>";
/// A JSON value with every embedded `DefinitionRef` reduced to its key and
/// version and every all-hex string of 32 or more digits replaced: both are
/// digests the composed registry derives from the TacSim manifest.
fn strip(value: &serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match value {
        Value::Object(map) => {
            let reference = map.len() == 3
                && map.contains_key("key")
                && map.contains_key("version")
                && map.contains_key("digest");
            Value::Object(
                map.iter()
                    .filter(|(field, _)| !(reference && field.as_str() == "digest"))
                    .map(|(field, inner)| (field.clone(), strip(inner)))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.iter().map(strip).collect()),
        Value::String(text) if text.len() >= 32 && text.chars().all(|c| c.is_ascii_hexdigit()) => {
            Value::String(STRIPPED.to_owned())
        }
        other => other.clone(),
    }
}
fn stripped<T: serde::Serialize + ?Sized>(value: &T) -> Result<String, Error> {
    json(&strip(&serde_json::to_value(value)?))
}
/// The first run of 32 or more hex digits in `text`, if any.
fn hex_run(text: &str) -> Option<&str> {
    let mut start = None;
    for (at, c) in text.char_indices().chain([(text.len(), ' ')]) {
        match (c.is_ascii_hexdigit(), start) {
            (true, None) => start = Some(at),
            (false, Some(from)) => {
                if at - from >= 32 {
                    return text.get(from..at);
                }
                start = None;
            }
            _ => {}
        }
    }
    None
}

/// The commerce-free composition every table is compared with: the five
/// modules `house_engine` boots, or (`five` false) the three `engine` boots.
#[inline(never)]
fn plain(five: bool) -> Result<Box<Engine>, ModError> {
    let mut modules: Vec<Box<dyn GameModule>> = vec![
        Box::new(EconomyModule::new()),
        Box::new(RollModule::with_writers([crate::NS])),
        Box::new(layer()),
    ];
    if five {
        modules.insert(0, Box::new(mush_org::OrgModule::new()));
        modules.insert(
            1,
            Box::new(
                MarketModule::new()
                    .with_venue_gate(std::sync::Arc::new(crate::port::market::PortVenueGate)),
            ),
        );
    }
    compose_boxed(World::new_minimal(), Registry::new(), modules)
}

/// The widest chain: the House chain, then the player market, in
/// `LaunchComposition::prepare`'s order (Conduct, the Registry inspection
/// and IC claims left out: they need the hardware catalogue).
fn widest() -> Result<MindCommerce, ModError> {
    prepare(CommerceMode::Active)?
        .with_house_owner_terms(InitialFilingTerms::new(365 * 86_400, 3 * 86_400).map_err(error)?)?
        .with_house_filings()?
        .with_registry_reconformance()?
        .with_minds_facts()?
        .with_house_minds(house_policy()?)?
        .with_two_seat_house_pins(house_policy()?)?
        .with_house_mind_renewals(house_renewal_policy()?)?
        .with_house_mind_renewal_service()?
        .with_owner_attestations()?
        .with_original_source_purchases(quotes()?)?
        .with_authored_forks()?
        .with_marked_registry()?
        .with_dependent_owner_filings()?
        .with_mind_entitlements(entitlement_tests::policy()?)?
        .with_compatible_updates()?
        .with_derived_updates()?
        .with_charter_transfers()?
        .with_registry_revocation(revocation_policy()?)?
        .with_pending_charter_transfers(pending_policy()?)?
        .with_paid_mind_resales(mush_core::ObjId(7), 86_400)?
        .with_registry_recall(recall_policy()?)?
        .with_mind_renewals(renewal_policy()?)?
        .with_mind_renewal_service()?
        .with_author_withdrawal()?
        .with_owner_parent_qualification()
}

/// Every enumerable registration table of a composed engine, by name.
/// Tables whose order is registration order (commands, ticks, panel feeds,
/// job queues) keep it; the rest are sorted.
fn tables(engine: &Engine, detail: bool) -> Result<Vec<(&'static str, Vec<Record>)>, Error> {
    let r = &engine.registry;
    let definitions = r.definitions().ok_or("definitions")?;
    let providers = r.action_providers().ok_or("action providers")?;
    let mut entries: Vec<_> = definitions.iter().collect();
    entries.sort_by(|a, b| a.reference().cmp(b.reference()));
    let mut defs = Vec::with_capacity(entries.len());
    for entry in entries {
        let reference = entry.reference();
        let mut record = one("definition", name(reference));
        if detail {
            record.push(("  current".into(), entry.is_current().to_string()));
            // The manifest's payload and references are the TacSim catalogue
            // and configuration, which other programmes change.
            if reference.key().kind.as_str() == "registry_manifest" {
                let omitted = "<the TacSim catalogue: not written>".to_owned();
                record.push(("  payload".into(), omitted.clone()));
                record.push(("  references".into(), omitted));
            } else {
                record.push(("  payload".into(), stripped(entry.payload())?));
                record.push(("  references".into(), names(entry.references())));
            }
        }
        if reference.key().kind.as_str() == "action" {
            let owner = providers
                .get_exact(reference)
                .map_or_else(|_| "none".to_owned(), |p| p.owner().as_str().to_owned());
            record.push(("  provider".into(), owner));
        }
        defs.push(record);
    }

    let families = r.state_families().ok_or("state families")?;
    // The registry exposes its family locators through `Debug` only:
    // `families: [(OwnerId("market"), StateFamilyId("…")), …]`.
    let debug = format!("{families:?}");
    let inner = debug
        .split_once("families: [")
        .and_then(|(_, rest)| rest.split_once("], fingerprint"))
        .map(|(inner, _)| inner)
        .ok_or("state family Debug shape")?;
    let quoted: Vec<&str> = inner.split('"').skip(1).step_by(2).collect();
    check!(
        quoted.len().is_multiple_of(2),
        "state family Debug pairs: {inner}"
    );
    let mut family_rows: Vec<Record> = quoted
        .chunks(2)
        .map(|pair| one("family", pair.join(" / ")))
        .collect();
    check_eq!(family_rows.len(), families.len(), "every family is listed");
    family_rows.sort();
    let triples = |rows: Vec<(
        mush_core::mechanics::OwnerId,
        mush_core::StateFamilyId,
        mush_core::mechanics::OwnerId,
    )>,
                   label: &str|
     -> Vec<Record> {
        let mut out: Vec<Record> = rows
            .iter()
            .map(|(owner, family, other)| {
                one(
                    label,
                    format!(
                        "{} / {} -> {}",
                        owner.as_str(),
                        family.as_str(),
                        other.as_str()
                    ),
                )
            })
            .collect();
        out.sort();
        out
    };
    let mut delegations: Vec<Record> = families
        .delegations()
        .iter()
        .map(|(owner, grantee)| {
            one(
                "delegation",
                format!("{} -> {}", owner.as_str(), grantee.as_str()),
            )
        })
        .collect();
    delegations.sort();

    let mut bridges = Vec::new();
    for bridge in r.bridges().ok_or("bridges")?.iter() {
        bridges.push(vec![
            (
                "bridge provider".to_owned(),
                bridge.provider().as_str().to_owned(),
            ),
            ("  key".to_owned(), stripped(bridge.key())?),
            ("  definition".to_owned(), name(bridge.definition())),
        ]);
    }
    bridges.sort();
    let mut migrations = Vec::new();
    for (definition, from, to) in r.definition_migrations().ok_or("migrations")?.iter() {
        migrations.push(vec![
            ("migration".to_owned(), name(definition)),
            ("  from".to_owned(), name(from)),
            ("  to".to_owned(), name(to)),
        ]);
    }
    migrations.sort();
    let mut wakes = Vec::new();
    for (owner, id) in r
        .decision_sources()
        .ok_or("decision sources")?
        .wake_adapters()
    {
        wakes.push(one(
            "wake adapter",
            format!("{} / {}", owner.as_str(), id.as_str()),
        ));
    }
    wakes.sort();

    let mut commands = Vec::new();
    for (spec, module, compatibility) in r.command_registrations() {
        let mut record = one("command", spec.name.to_string());
        record.push(("  module".into(), format!("{module:?}")));
        record.push(("  compatibility".into(), format!("{compatibility:?}")));
        record.push(("  aliases".into(), format!("{:?}", spec.aliases)));
        record.push(("  switches".into(), format!("{:?}", spec.switches)));
        record.push(("  shape".into(), format!("{:?}", spec.shape)));
        record.push((
            "  gates".into(),
            format!(
                "role={:?} power={:?} privilege={:?} lock={} scene={:?}",
                spec.min_role,
                spec.power,
                spec.privilege,
                spec.lock.is_some(),
                spec.scene_class
            ),
        ));
        for (number, line) in spec.help.split('\n').enumerate() {
            record.push((format!("  help | {:02}", number + 1), line.to_owned()));
        }
        commands.push(record);
    }
    let ticks = r
        .module_ticks()
        .iter()
        .map(|tick| {
            one(
                "tick",
                format!("{}:{} {:?}", tick.module, tick.tag, tick.cadence),
            )
        })
        .collect();
    let feeds = r
        .panel_feeds()
        .iter()
        .map(|(module, feed)| one("panel feed", format!("{module}:{feed}")))
        .collect();
    let queues = r
        .job_queues()
        .iter()
        .map(|queue| one("job queue", format!("{queue:?}")))
        .collect();
    let mut apis: Vec<Record> = r
        .script_apis()
        .iter()
        .map(|(module, api)| one("script api", format!("{module}:{api}")))
        .collect();
    apis.sort();
    let mut persists: Vec<Record> = r
        .persist_delegations()
        .iter()
        .map(|d| {
            one(
                "persist delegation",
                format!(
                    "{} -> {} (by {}): {}",
                    d.writer, d.ns, d.declared_by, d.reason
                ),
            )
        })
        .collect();
    persists.sort();
    let mut namespaces: Vec<Record> = r
        .namespace_owners()
        .into_iter()
        .map(|(ns, owner)| one("namespace", format!("{ns} -> {owner}")))
        .collect();
    namespaces.sort();
    let mut continuations: Vec<Record> = r
        .continuation_delegations()
        .into_iter()
        .map(|(prefix, grantee, reason)| {
            one(
                "continuation delegation",
                format!("{prefix} -> {}: {reason}", grantee.as_str()),
            )
        })
        .collect();
    continuations.sort();

    Ok(vec![
        ("definitions", defs),
        ("state families", family_rows),
        ("state family delegations", delegations),
        (
            "state family scoped delegations",
            triples(families.family_delegations(), "scoped delegation"),
        ),
        (
            "state family read grants",
            triples(families.family_read_grants(), "read grant"),
        ),
        ("bridges", bridges),
        ("definition migrations", migrations),
        ("decision wake adapters", wakes),
        ("commands", commands),
        ("ticks", ticks),
        ("panel feeds", feeds),
        ("job queues", queues),
        ("script apis", apis),
        ("persist delegations", persists),
        ("namespace owners", namespaces),
        ("continuation delegations", continuations),
    ])
}

/// Writes one configuration: each table's records the commerce-free
/// composition does not have (`+`), and those it has that this one lost (`-`).
fn write(
    snap: &mut Snapshot,
    config: &str,
    engine: &Engine,
    base: &[(&'static str, Vec<Record>)],
    detail: bool,
) -> Result<Vec<DefinitionRef>, Error> {
    let mut added_actions = Vec::new();
    for ((table, full), (base_table, base_rows)) in tables(engine, detail)?.iter().zip(base) {
        check_eq!(table, base_table);
        let before: BTreeSet<&Record> = base_rows.iter().collect();
        let after: BTreeSet<&Record> = full.iter().collect();
        let added: Vec<&Record> = full.iter().filter(|r| !before.contains(r)).collect();
        let removed: Vec<&Record> = base_rows.iter().filter(|r| !after.contains(r)).collect();
        if added.is_empty() && removed.is_empty() {
            continue;
        }
        snap.section(&format!(
            "{config}: {table} (+{} -{})",
            added.len(),
            removed.len()
        ));
        for record in added {
            for (label, value) in record {
                snap.line(&format!("+ {label}"), value);
            }
        }
        for record in removed {
            for (label, value) in record {
                snap.line(&format!("- {label}"), value);
            }
        }
    }
    let base_refs = plain_refs(base);
    for entry in engine.registry.definitions().ok_or("definitions")?.iter() {
        let reference = entry.reference();
        if reference.key().kind.as_str() == "action" && !base_refs.contains(&name(reference)) {
            added_actions.push(reference.clone());
        }
    }
    added_actions.sort();
    Ok(added_actions)
}

/// The commerce-free composition's definition names, read back from its
/// table (the first line of each record is the reference's name).
fn plain_refs(base: &[(&'static str, Vec<Record>)]) -> BTreeSet<String> {
    base.iter()
        .filter(|(table, _)| *table == "definitions")
        .flat_map(|(_, rows)| rows.iter())
        .filter_map(|record| record.first())
        .map(|(_, value)| value.clone())
        .collect()
}

/// Every offer a player sees for `actions` on a freshly composed engine (no
/// licence, no hydration), with its presentation. Providers that need a held
/// licence or a live record offer nothing here.
fn offers(
    snap: &mut Snapshot,
    engine: &mut Engine,
    actions: &[DefinitionRef],
) -> Result<(), Error> {
    use mush_core::action::{PerceivedRef, PrincipalId as Principal};
    engine.set_offer_keys(mush_core::OfferKeyRing::new(
        mush_core::OfferSecret::new(1, [0x46; 32])?,
        [],
    )?);
    let viewer = engine.world.create(
        mush_core::ObjKind::Player,
        "Ct0Viewer",
        mush_core::GOD,
        Some(mush_core::LIMBO),
    )?;
    snap.section("widest chain: legal-action offers to a player with no holding");
    let query = engine.legal_action_query(
        Principal::Object { id: viewer },
        viewer,
        PerceivedRef::SelfActor,
    )?;
    match engine.project_legal_actions(&query) {
        Err(refusal) => snap.line("projection refused", &refusal.to_string()),
        Ok(sink) => {
            let mut none = 0usize;
            for action in actions {
                let Some(offer) = sink.projected_for_action(action) else {
                    none += 1;
                    continue;
                };
                snap.line("offer for", &name(action));
                snap.line("  presentation", &stripped(offer.presentation())?);
                snap.line("  parameters", &stripped(offer.parameters())?);
                snap.line("  targets", &stripped(offer.targets())?);
                snap.line("  required channels", &stripped(offer.required_channels())?);
                snap.line("  expires", &stripped(&offer.expires())?);
            }
            snap.line("commerce actions offering nothing here", &none.to_string());
        }
    }
    Ok(())
}

/// The player market's command, `+minds`, which every composition registers
/// (its slot opens with the market): the delta tables cannot show it, so its
/// whole registration is written here. `@mind`, `+registry` and `mind` are
/// shared too but mostly belong to other programmes, so they are left out.
fn family_commands(
    snap: &mut Snapshot,
    engine: &Engine,
    base: &[(&'static str, Vec<Record>)],
) -> Result<(), Error> {
    snap.section(
        "widest chain: the player market command, shared with the commerce-free composition",
    );
    let shared: BTreeSet<&Record> = base
        .iter()
        .filter(|(table, _)| *table == "commands")
        .flat_map(|(_, rows)| rows.iter())
        .collect();
    for (table, rows) in tables(engine, false)? {
        if table != "commands" {
            continue;
        }
        for record in rows.iter().filter(|r| shared.contains(r)) {
            let named = record.first().is_some_and(|(_, name)| name == "+minds");
            if named {
                for (label, value) in record {
                    snap.line(label, value);
                }
            }
        }
    }
    Ok(())
}

/// The chain's own references in builder order (the order the composition
/// configures the owners with them), as `full_player_market_chain_…` asserts.
fn builder_order(snap: &mut Snapshot, chain: &MindCommerce) -> Result<(), Error> {
    use mush_roll::commerce::house_purchase::HouseProtocol::{HouseV1, HouseV2};
    let s = &chain.state;
    snap.section("widest chain: references in builder order");
    snap.line("registry manifest", &name(chain.manifest()));
    snap.line("initial filing", &name(&s.filing_action));
    snap.line("house filing", &some_name(s.house_filing_action.as_ref()));
    let house = s.house_purchases.as_ref().ok_or("House purchases")?;
    snap.line(
        "house participant economy",
        &name(house.participants().economy()),
    );
    snap.line(
        "house participant retention",
        &name(house.participants().retention()),
    );
    for (version, protocol) in [("v1", HouseV1), ("v2", HouseV2)] {
        snap.line(
            &format!("house {version} policy"),
            &name(&house.policy_ref_for_protocol(protocol)?),
        );
        snap.line(
            &format!("house {version} quote"),
            &name(&house.quote_action(protocol)?),
        );
        for reference in house.owner_actions(protocol)? {
            snap.line(&format!("house {version} owner action"), &name(&reference));
        }
    }
    for reference in s.house_purchase_actions.iter().flatten() {
        snap.line("house purchase action", &name(reference));
    }
    for reference in s.two_seat_house_purchase_actions.iter().flatten() {
        snap.line("house two-seat purchase action", &name(reference));
    }
    let house_renewals = s.house_renewals.as_ref().ok_or("House renewals")?;
    snap.line("house renewal manifest", &name(house_renewals.manifest()));
    for reference in s.house_renewal_actions.iter().flatten() {
        snap.line("house renewal action", &name(reference));
    }
    snap.line(
        "house renewal service",
        &some_name(s.house_renewal_service.as_ref()),
    );
    for reference in s.purchase_actions.iter().flatten() {
        snap.line("original purchase action", &name(reference));
    }
    for (protocol, set) in ["original v3", "derived v1"]
        .into_iter()
        .zip(s.entitlement_actions.iter().flatten())
    {
        for reference in set {
            snap.line(&format!("entitlement {protocol} action"), &name(reference));
        }
    }
    for (label, reference) in [
        ("dependent filing", &s.dependent_filing_action),
        ("compatible update", &s.compatible_update_action),
        ("derived update", &s.derived_update_action),
        ("charter transfer accept", &s.charter_transfer_action),
        ("registry revocation", &s.registry_revocation_action),
    ] {
        snap.line(label, &some_name(reference.as_ref()));
    }
    snap.line("registry recall", &name(&recall_policy()?.action()?));
    for reference in pending_policy()?.actions()? {
        snap.line("pending charter action", &name(&reference));
    }
    for reference in mush_roll::commerce::paid_resale::protocol::actions()? {
        snap.line("paid resale action", &name(&reference));
    }
    let renewals = s.renewals.as_ref().ok_or("personal renewals")?;
    snap.line("personal renewal manifest", &name(renewals.manifest()));
    snap.line(
        "personal renewal participant economy",
        &name(renewals.participants().economy()),
    );
    snap.line(
        "personal renewal participant retention",
        &name(renewals.participants().retention()),
    );
    for reference in s.renewal_actions.iter().flatten() {
        snap.line("personal renewal action", &name(reference));
    }
    snap.line(
        "personal renewal service",
        &some_name(s.renewal_service.as_ref()),
    );
    snap.line(
        "personal renewal player front",
        &format!("{:?}", chain.player_front().map(|o| o.as_str().to_owned())),
    );
    Ok(())
}

/// How a narrower configuration is booted: as its neighbouring test boots it.
#[derive(Clone, Copy)]
enum Boot {
    /// `engine`: Economy, Roll, TacSim.
    Plain,
    /// `house_engine(.., true)`: Org, Market, Economy, Roll, TacSim.
    House,
}

type Configure = fn() -> Result<MindCommerce, ModError>;

fn narrower() -> [(&'static str, Boot, Configure); 21] {
    [
        ("initialize", Boot::Plain, || {
            prepare(CommerceMode::Initialize)
        }),
        ("active, no option", Boot::Plain, || {
            prepare(CommerceMode::Active)
        }),
        ("registry recall", Boot::Plain, || {
            prepare(CommerceMode::Active)?.with_registry_recall(recall_policy()?)
        }),
        ("registry revocation", Boot::Plain, || {
            prepare(CommerceMode::Active)?.with_registry_revocation(revocation_policy()?)
        }),
        ("minds facts", Boot::Plain, || {
            prepare(CommerceMode::Active)?.with_minds_facts()
        }),
        ("registry reconformance", Boot::Plain, || {
            prepare(CommerceMode::Active)?.with_registry_reconformance()
        }),
        ("original purchases (sealed)", Boot::House, || {
            prepare(CommerceMode::Active)?.with_original_purchases(quotes()?)
        }),
        ("original purchases (source forms)", Boot::House, || {
            prepare(CommerceMode::Active)?.with_original_source_purchases(quotes()?)
        }),
        ("original purchases and paid resale", Boot::House, || {
            prepare(CommerceMode::Active)?
                .with_original_purchases(quotes()?)?
                .with_paid_mind_resales(mush_core::GOD, 60)
        }),
        ("house purchases", Boot::House, || {
            prepare(CommerceMode::Active)?
                .with_house_filings()?
                .with_house_minds(house_policy()?)
        }),
        ("house and original purchases", Boot::House, || {
            prepare(CommerceMode::Active)?
                .with_original_purchases(quotes()?)?
                .with_house_filings()?
                .with_house_minds(house_policy()?)
        }),
        ("house purchases, two-seat", Boot::House, || {
            prepare(CommerceMode::Active)?
                .with_house_filings()?
                .with_house_minds(house_policy()?)?
                .with_two_seat_house_pins(house_policy()?)
        }),
        ("house renewals without the service", Boot::House, || {
            prepare(CommerceMode::Active)?
                .with_house_filings()?
                .with_house_minds(house_policy()?)?
                .with_house_mind_renewals(house_renewal_policy()?)
        }),
        ("house renewals and the service", Boot::House, || {
            prepare(CommerceMode::Active)?
                .with_house_filings()?
                .with_house_minds(house_policy()?)?
                .with_house_mind_renewals(house_renewal_policy()?)?
                .with_house_mind_renewal_service()
        }),
        (
            "house two-seat, renewals and the service",
            Boot::House,
            || {
                prepare(CommerceMode::Active)?
                    .with_house_filings()?
                    .with_house_minds(house_policy()?)?
                    .with_two_seat_house_pins(house_policy()?)?
                    .with_house_mind_renewals(house_renewal_policy()?)?
                    .with_house_mind_renewal_service()
            },
        ),
        ("dependent owners", Boot::House, entitlement_tests::prepared),
        ("entitlements", Boot::House, entitled),
        ("personal renewals without the service", Boot::House, || {
            entitled()?.with_mind_renewals(renewal_policy()?)
        }),
        ("personal renewals and the service", Boot::House, || {
            entitled()?
                .with_mind_renewals(renewal_policy()?)?
                .with_mind_renewal_service()
        }),
        ("player market, revocation first", Boot::House, || {
            player_market(true)
        }),
        ("player market, revocation last", Boot::House, || {
            player_market(false)
        }),
    ]
}

#[test]
fn ct0_composed_registration_snapshot() -> TestResult {
    let five = plain(true)?;
    let base = tables(&five, true)?;
    let base_five = tables(&five, false)?;
    let base_three = tables(&*plain(false)?, false)?;
    let mut snap = Snapshot::new();

    let chain = widest()?;
    let mut full = house_engine(&chain, true)?;
    let actions = write(&mut snap, "widest chain", &full, &base, true)?;
    builder_order(&mut snap, &chain)?;
    offers(&mut snap, &mut full, &actions)?;
    family_commands(&mut snap, &full, &base)?;
    // The House and player chains each register what the narrower boots
    // assert (a sample: the full refs are the lines above).
    for id in [
        "draft_mind_renewal",
        "close_mind_renewal_service",
        "draft_house_mind_renewal",
        "close_house_mind_renewal_service",
        "purchase_house_mind",
        "purchase_original_mind",
    ] {
        check!(has(&full, "market", "action", id), "{id}");
    }

    for (name, boot, configure) in narrower() {
        let config = configure()?;
        let (engine, base) = match boot {
            Boot::Plain => (engine(&config)?, &base_three),
            Boot::House => (house_engine(&config, true)?, &base_five),
        };
        write(&mut snap, name, &engine, base, false)?;
    }

    // Proof that no registry-derived digest is left: no written line holds a
    // run of 32 hex digits. The allow-list is empty: the one fixed hex input,
    // the offer key, is never written.
    let rendered = snap.render(FILE);
    for line in rendered.lines() {
        if let Some(run) = hex_run(line) {
            return Err(format!("a digest-shaped run {run} in: {line}").into());
        }
    }
    snap.check(env!("CARGO_MANIFEST_DIR"), FILE)?;
    Ok(())
}
