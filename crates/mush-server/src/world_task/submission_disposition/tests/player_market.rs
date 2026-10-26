//! 17g §6 risk 1: the first boot of the full player-market roster through the
//! production composition root. Every server here boots through
//! `boot::game_modules_with_mind_commerce` (the `game_modules` root with the
//! real `MindCommerce` builder), exactly as `build_engine` composes. Its
//! identities are made by live public commands and accounts only: no direct
//! provider preparation, manual ACK, test certificate or world call.
//!
//! 17g C1a builds the roster C1c will compose (plan §2.2, the order of §2.3)
//! and reads the offers it projects in an empty Active world against the
//! five classes of plan §7. C1b extends this file with the launch path
//! (deployment format 2), and C1c with the composed launch market.
use super::paid_purchase::{
    auth_account, open_wallet,
    runtime::{Running, STEP},
    store::PurchaseFaultStore,
};
use mush_config::config::Config;
use mush_core::{GOD, ObjId, PerceivedRef};
use mush_economy::{
    AccountId as WalletId, Amount, CatalogRef, InstrumentId,
    managed_entitlement::{GrossRecipientCut, Recipient},
};
use mush_market::{
    MarketId,
    mind_trade::{OriginalDuration, OriginalPrice, OriginalQuotePolicy, records::QuoteVenue},
};
use mush_org::{OrgId, OrgScopeId, PrincipalId, SystemPrincipalId};
use mush_roll::commerce::{filing::InitialFilingTerms, module::CommerceMode};
use mush_store::ObjectStore;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use tacsim::{TacsimRuleset, composition::mind_commerce::MindCommerce};
use tokio::time::timeout;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

const DAY: i64 = 86_400;
/// 400cr: C1's provisional personal and House Pin price (plan §6).
const PIN_MINOR: i64 = 40_000;
/// The Compact levy and the House venue cut, per mille.
const LEVY_PERMILLE: u16 = 50;
const REGISTRY: &str = "roll.registry";
const VENUE: &str = "launch.venue";
const CREDIT: &str = "tacsim.credit";

/// The C1 offer verbs (plan §7), re-grepped at C1a from every
/// `ActionPresentation::new` under `mush-roll/src` and `mush-market/src`,
/// including the verbs built from `match` arms and `format!`. The re-grep
/// added one: `roll-retire-authored-draft`, which `with_authored_forks`
/// composes (authored retirement, two offers). The personal filing verbs
/// (`roll-reserve-mind-charter`, `roll-file-original-mind`) predate C1.
const C1_VERBS: [&str; 36] = [
    "original-list-v3",
    "original-quote-v3",
    "original-fund-v3",
    "original-grant-v3",
    "original-refund-v3",
    "derived-list-v1",
    "derived-quote-v1",
    "derived-fund-v1",
    "derived-grant-v1",
    "derived-refund-v1",
    "market-original-mind-offer",
    "market-original-mind-purchase",
    "roll-file-dependent-mind",
    "roll-update-original-mind",
    "roll-update-derived-mind",
    "roll-fork-paid-mind",
    "roll-retire-authored-draft",
    "roll-offer-charter-transfer",
    "roll-accept-charter-transfer",
    "roll-offer-pending-charter",
    "roll-fund-pending-charter",
    "roll-settle-pending-charter",
    "roll-refund-pending-charter",
    "paid-mind-resale",
    "paid-resale-service",
    "mind-renewal-draft",
    "mind-renewal-fund",
    "mind-renewal-grant",
    "mind-renewal-refund",
    "mind-renewal-close",
    "mind-renewal-service",
    "roll-withdraw-authored-mind",
    "roll-attest-owner-mind",
    "roll-replace-owner-parent",
    "roll-registry-recall",
    "roll-registry-revocation",
];
/// Class A: offered only to a policy operator holding a live grant.
const OPERATOR_GATED: [&str; 2] = ["roll-registry-recall", "roll-registry-revocation"];
/// Class B: offered only to their service actor while a live intent exists.
const SERVICE: [&str; 4] = [
    "mind-renewal-service",
    "paid-resale-service",
    "roll-settle-pending-charter",
    "roll-refund-pending-charter",
];
/// Class C verbs every one of whose offers needs the counter: the non-refund
/// steps of the original and derived purchases (their refunds are generic).
const ONLY_AT_THE_COUNTER: [&str; 8] = [
    "original-list-v3",
    "original-quote-v3",
    "original-fund-v3",
    "original-grant-v3",
    "derived-list-v1",
    "derived-quote-v1",
    "derived-fund-v1",
    "derived-grant-v1",
];
/// The two C1 verbs whose kinds are gated differently. One presentation
/// covers every kind, so the census tells the kinds apart by the action each
/// offer routes to (`mind_trade/mod.rs`, `mind_trade/purchase_records.rs`):
/// - `market-original-mind-offer` (`mind_trade/provider.rs`, `project_offer`):
///   List and Quote need the counter (C), Expire is God's (D), and
///   RetireListing and RetireQuote go to every live player (E);
/// - `market-original-mind-purchase` (`mind_trade/purchase_provider.rs`): Fund
///   and Grant need the counter (C), and Refund goes to every live player (E).
const SPLIT_BY_KIND: [&str; 2] = [
    "market-original-mind-offer",
    "market-original-mind-purchase",
];
/// Class C kinds of the two split verbs.
const COUNTER_KINDS: [&str; 4] = [
    "list_original_mind",
    "quote_original_mind",
    "purchase_original_mind",
    "settle_original_mind",
];
/// Class D kind: the v2 Expire, offered to GOD while an expired quote waits.
const GOD_KIND: &str = "expire_original_mind_quotes";
/// Class E kinds of the two split verbs: each principal has exactly one offer
/// of each, wherever it stands.
const GENERIC_KINDS: [&str; 3] = [
    "retire_original_mind_listing",
    "retire_original_mind_quote",
    "refund_original_mind",
];
/// Where the census principals stand (`Principals`).
const AT_THE_COUNTER: [&str; 3] = ["author", "buyer", "accountless"];

fn config() -> Config {
    let mut config = Config::default();
    config.tacsim.enabled = true;
    config.store.store = "memory".into();
    config.game.ic_ratio = 24;
    config
}
fn system(name: &str) -> TestResult<SystemPrincipalId> {
    SystemPrincipalId::new(name).ok_or_else(|| format!("system principal {name}").into())
}

/// The census principals. Author, buyer and the accountless character stand
/// in the counter room (the `ceres` exchange); GOD, the clerk and the Registry
/// operator stand together outside it, in Limbo.
#[derive(Clone, Copy, Debug)]
struct Principals {
    god: ObjId,
    author: ObjId,
    buyer: ObjId,
    clerk: ObjId,
    operator: ObjId,
    accountless: ObjId,
}
impl Principals {
    fn all(&self) -> [(&'static str, ObjId); 6] {
        [
            ("god", self.god),
            ("author", self.author),
            ("buyer", self.buyer),
            ("clerk", self.clerk),
            ("operator", self.operator),
            ("accountless", self.accountless),
        ]
    }
}
/// What the plain boot's public commands created, for the roster and census.
#[derive(Clone, Debug)]
struct Seeded {
    principals: Principals,
    registry: WalletId,
    venue: WalletId,
    organization: OrgId,
    scope: OrgScopeId,
}

/// One more login account's character, made as the public account boundary
/// does: `create_player` then `link_character`.
async fn character(
    server: &Running,
    store: &PurchaseFaultStore,
    account: (&str, mush_core::AccountId),
    name: &str,
) -> TestResult<ObjId> {
    let id = create(server, name, Some(account.1)).await?;
    store.link_character(account.0, id, name).await?;
    Ok(id)
}
/// `create_player`, retried within the step guard while the world is `Busy`
/// committing another authoritative action (a boot-time service step). Busy
/// is a typed refusal before anything is created, so a retry is exact.
async fn create(
    server: &Running,
    name: &str,
    account: Option<mush_core::AccountId>,
) -> TestResult<ObjId> {
    let end = tokio::time::Instant::now() + STEP;
    loop {
        match timeout(STEP, server.handle()?.create_player(name, account)).await? {
            Ok(id) => return Ok(id),
            Err(crate::world_task::CreatePlayerErr::Busy) if tokio::time::Instant::now() < end => {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            Err(error) => return Err(error.into()),
        }
    }
}
/// The one `#<n>` a staff command's acknowledgement names after `marker`.
fn created(lines: &[String], marker: &str) -> TestResult<u64> {
    let ids = lines
        .iter()
        .filter_map(|line| line.split_once(marker))
        .filter_map(|(_, tail)| tail.split_whitespace().next())
        .map(|id| id.trim_end_matches([';', '.']).parse::<u64>())
        .collect::<Result<Vec<_>, _>>()?;
    let [id]: [u64; 1] = ids
        .try_into()
        .map_err(|_| format!("one acknowledged {marker}"))?;
    Ok(id)
}
async fn staff(server: &Running, line: String) -> TestResult<Vec<String>> {
    let lines = server.command(GOD, line.clone()).await?;
    if lines.is_empty() {
        return Err(format!("no acknowledgement for {line}").into());
    }
    Ok(lines)
}

/// The plain boot (no mind commerce): accounts, characters, wallets, the
/// Registry org with its `review` scope, the operator's two production grants,
/// and every principal's place, all through public commands. Then stop.
async fn seed(config: &Config, store: &Arc<PurchaseFaultStore>) -> TestResult<Seeded> {
    auth_account(store, "player-market-staff", Some(GOD)).await?;
    let staff_account = auth_account(store, "player-market-staff-two", None).await?;
    let author_account = auth_account(store, "player-market-author", None).await?;
    let buyer_account = auth_account(store, "player-market-buyer", None).await?;
    let mut server = Running::boot(config, store.clone(), None).await?;
    server.connect(GOD).await?;
    let author = character(
        &server,
        store,
        ("player-market-author", author_account),
        "MarketAuthor",
    )
    .await?;
    let buyer = character(
        &server,
        store,
        ("player-market-buyer", buyer_account),
        "MarketBuyer",
    )
    .await?;
    let clerk = character(
        &server,
        store,
        ("player-market-staff-two", staff_account),
        "MarketClerk",
    )
    .await?;
    let operator = character(
        &server,
        store,
        ("player-market-staff-two", staff_account),
        "RegistryOperator",
    )
    .await?;
    let accountless = create(&server, "MarketDrifter", None).await?;
    let registry = open_wallet(&server, store, PrincipalId::System(system(REGISTRY)?)).await?;
    let venue = open_wallet(&server, store, PrincipalId::System(system(VENUE)?)).await?;
    for actor in [author, buyer] {
        open_wallet(&server, store, PrincipalId::Object(actor)).await?;
    }
    let organization = OrgId(created(
        &staff(&server, "@org/create Registry".into()).await?,
        "Created org #",
    )?);
    let scope = OrgScopeId(created(
        &staff(&server, "@org/scope create Registry=review".into()).await?,
        "Created scope #",
    )?);
    for capability in ["roll.registry.recall", "roll.registry.revoke"] {
        let lines = staff(
            &server,
            format!(
                "@org/grant #{}=Registry {capability} scope=#{}",
                operator.0, scope.0
            ),
        )
        .await?;
        check!(
            lines.iter().any(|line| line.contains("Granted")),
            "the production grant is acknowledged: {lines:?}"
        );
    }
    // The census's class C assertion proves these placements took.
    for actor in [author, buyer, accountless] {
        server
            .command(
                GOD,
                format!("@teleport #{}=#{}", actor.0, server.exchange.0),
            )
            .await?;
    }
    for actor in [GOD, clerk, operator] {
        server
            .command(GOD, format!("@teleport #{}=#0", actor.0))
            .await?;
    }
    server.stop().await?;
    Ok(Seeded {
        principals: Principals {
            god: GOD,
            author,
            buyer,
            clerk,
            operator,
            accountless,
        },
        registry,
        venue,
        organization,
        scope,
    })
}

/// The full player-market roster, through the builder only, in the order C1c
/// composes (plan §2.2 and §2.3). Initialize composes `prepare` alone; Active
/// adds the House chain as the launch composes it (minus Conduct and the
/// Coprates admission, which are launch-only) and then the player market.
/// Every number is C1's provisional default (plan §6).
fn player_market_commerce(
    config: &Config,
    mode: CommerceMode,
    ids: &Seeded,
) -> TestResult<MindCommerce> {
    let cfg = crate::config_ext::tacsim_config(&config.tacsim)?;
    // No Conduct carrier, so the server layer `game_modules` builds matches.
    let layer = TacsimRuleset::with_game_content(cfg.clone(), cfg.enabled);
    let issuer = system(REGISTRY)?;
    let commerce = MindCommerce::prepare(
        &layer,
        mode,
        InitialFilingTerms::new(90 * DAY, 3 * DAY)?,
        mush_economy::registry_filing::InitialFilingPolicy::new(
            issuer.clone(),
            ids.registry,
            Amount::from_minor(10_000),
            CatalogRef {
                ruleset: "tacsim".into(),
                code: "mind.charter".into(),
            },
        )?,
    )?;
    if mode == CommerceMode::Initialize {
        return Ok(commerce);
    }
    let credit = || InstrumentId::new(CREDIT).ok_or("credit instrument");
    let ceres = || MarketId::new("ceres").ok_or("Ceres venue");
    let registry = Recipient {
        account: ids.registry,
        owner: PrincipalId::System(issuer.clone()),
    };
    let venue = Recipient {
        account: ids.venue,
        owner: PrincipalId::System(system(VENUE)?),
    };
    let levy = GrossRecipientCut {
        permille: LEVY_PERMILLE,
        recipient: Some(registry.clone()),
    };
    let venue_cut = GrossRecipientCut {
        permille: LEVY_PERMILLE,
        recipient: Some(venue.clone()),
    };
    let recall = mush_roll::commerce::registry_recall::RegistryRecallPolicy::new(
        issuer.clone(),
        ids.organization,
        ids.scope,
        vec![ids.principals.operator],
    )?;
    let revocation = mush_roll::commerce::registry_revocation::RegistryRevocationPolicy::new(
        issuer.clone(),
        ids.organization,
        ids.scope,
        vec![ids.principals.operator],
    )?;
    let clerk = ids.principals.clerk;
    Ok(commerce
        // 1. The House chain, as `LaunchComposition::prepare` composes it.
        .with_house_owner_terms(InitialFilingTerms::new(365 * DAY, 3 * DAY)?)?
        .with_house_filings()?
        .with_registry_reconformance()?
        .with_minds_facts()?
        .with_house_minds(mush_market::mind_house::HousePolicy {
            price: Amount::from_minor(PIN_MINOR),
            instrument: credit()?,
            registry: levy.clone(),
            venue_name: ceres()?,
            venue: venue_cut.clone(),
            transferable: true,
            term_seconds: 90 * DAY,
            grace_seconds: 3 * DAY,
            quote_seconds: 300,
            renewal_price: Amount::from_minor(PIN_MINOR),
            renewal_term_seconds: 90 * DAY,
            renewal_grace_seconds: 3 * DAY,
        })?
        .with_house_mind_renewals(mush_market::mind_house_renewal::RenewalPolicy {
            registry: levy.clone(),
            venue: Some(venue_cut),
            offer_ic_seconds: 259_200,
        })?
        .with_house_mind_renewal_service()?
        // 2. The player market. Guards force: source purchases before forks,
        // forks before the marked Registry, both before dependent filing,
        // that before entitlements, entitlements before renewals, charter
        // transfers before pending charters. Revocation's place before
        // pending charters and paid resale is a convention only.
        .with_original_source_purchases(OriginalQuotePolicy::new(
            OriginalPrice::new(
                Amount::from_minor(PIN_MINOR),
                credit()?,
                issuer.clone(),
                ids.registry,
                LEVY_PERMILLE,
            )?,
            QuoteVenue::new(ceres()?, venue.owner.clone(), venue.account, 0)?,
            OriginalDuration::new(true, 90 * DAY, 3 * DAY, 300)?,
        )?)?
        .with_authored_forks()?
        .with_marked_registry()?
        .with_dependent_owner_filings()?
        .with_mind_entitlements(mush_market::mind_entitlement::EntitlementPolicy {
            price: Amount::from_minor(PIN_MINOR),
            instrument: credit()?,
            registry: levy.clone(),
            venue_name: ceres()?,
            venue: GrossRecipientCut {
                permille: 0,
                recipient: None,
            },
            transferable: true,
            term_seconds: 90 * DAY,
            grace_seconds: 3 * DAY,
            quote_seconds: 300,
            renewal_price: Amount::from_minor(PIN_MINOR),
            renewal_term_seconds: 90 * DAY,
            renewal_grace_seconds: 3 * DAY,
        })?
        .with_compatible_updates()?
        .with_derived_updates()?
        .with_charter_transfers()?
        .with_registry_revocation(revocation)?
        .with_pending_charter_transfers(mush_roll::commerce::PendingCharterTransferConfig::new(
            clerk, 259_200,
        )?)?
        .with_paid_mind_resales(clerk, 86_400)?
        .with_registry_recall(recall)?
        .with_mind_renewals(mush_market::mind_renewal::RenewalPolicy {
            registry: levy,
            venue: None,
            offer_ic_seconds: 259_200,
        })?
        .with_mind_renewal_service()?
        .with_author_withdrawal()?
        .with_owner_attestations()?
        .with_owner_parent_qualification()?)
}

/// Each principal's projected C1 offers: every `ProjectedLegalAction` whose
/// verb is a C1 verb, serialized with its `offer` id (the only per-actor
/// part) removed and the id of the action its route names added as `action`
/// (the same for every actor; it tells apart the kinds of the two verbs in
/// `SPLIT_BY_KIND`), sorted. All verbs found are also printed per principal,
/// for the record of what an empty Active world offers.
type Census = BTreeMap<&'static str, Vec<Value>>;
/// One principal's projected legal actions. A read that lands while the world
/// commits another action (a boot-time service step) answers `Busy`; it is
/// retried within the step guard, never treated as an empty projection.
async fn legal(server: &Running, actor: ObjId) -> TestResult<Vec<mush_core::ProjectedLegalAction>> {
    let end = tokio::time::Instant::now() + STEP;
    loop {
        match timeout(
            STEP,
            server
                .handle()?
                .legal_actions(actor.0.into(), actor, PerceivedRef::SelfActor),
        )
        .await?
        {
            Ok(offers) => return Ok(offers),
            Err(crate::world_task::LegalActionReadError::Busy)
                if tokio::time::Instant::now() < end =>
            {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            Err(error) => return Err(error.into()),
        }
    }
}
async fn census(server: &Running, principals: &Principals) -> TestResult<Census> {
    let mut census = Census::new();
    for (name, actor) in principals.all() {
        let offers = legal(server, actor).await?;
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        let mut entries = Vec::new();
        for offer in offers {
            let verb = offer.presentation().verb.clone();
            *counts.entry(verb.clone()).or_default() += 1;
            if !C1_VERBS.contains(&verb.as_str()) {
                continue;
            }
            // Every listed C1 offer routes to an action this boot registered.
            let action = server
                .routes()
                .get(offer.offer().route_id())
                .ok_or_else(|| format!("{verb} routes to a registered action"))?
                .key()
                .id
                .as_str()
                .to_owned();
            let mut value = serde_json::to_value(&offer)?;
            let object = value
                .as_object_mut()
                .ok_or("a projected action is an object")?;
            object
                .remove("offer")
                .ok_or("a projected action names its offer")?;
            object.insert("action".into(), Value::String(action));
            entries.push(value);
        }
        entries.sort_by_key(Value::to_string);
        for (verb, count) in &counts {
            eprintln!("PLAYER_MARKET_CENSUS principal={name} verb={verb} entries={count}");
        }
        census.insert(name, entries);
    }
    Ok(census)
}
fn verb(entry: &Value) -> &str {
    entry
        .pointer("/presentation/verb")
        .and_then(Value::as_str)
        .unwrap_or_default()
}
fn verbs(entries: &[Value]) -> BTreeSet<&str> {
    entries.iter().map(verb).collect()
}
/// The id of the action an entry's offer routes to.
fn action(entry: &Value) -> &str {
    entry
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or_default()
}
/// The offers of one kind of the two split verbs. Action ids are not unique
/// across verbs (the v3 original steps reuse the same ids under their own
/// definitions), so the kind is read only within `SPLIT_BY_KIND`.
fn kinds<'a>(entries: &'a [Value], kind: &str) -> Vec<&'a Value> {
    entries
        .iter()
        .filter(|entry| SPLIT_BY_KIND.contains(&verb(entry)) && action(entry) == kind)
        .collect()
}
/// Whether an entry is class C: a counter-only verb, or a counter kind of a
/// split verb.
fn counter_gated(entry: &Value) -> bool {
    ONLY_AT_THE_COUNTER.contains(&verb(entry))
        || (SPLIT_BY_KIND.contains(&verb(entry)) && COUNTER_KINDS.contains(&action(entry)))
}
/// `left` minus `right`, as multisets of exact entries.
fn minus<'a>(left: &'a [Value], right: &[Value]) -> Vec<&'a Value> {
    let mut rest: Vec<&Value> = right.iter().collect();
    left.iter()
        .filter(|entry| match rest.iter().position(|other| other == entry) {
            Some(at) => {
                rest.swap_remove(at);
                false
            }
            None => true,
        })
        .collect()
}
fn without_operator_gated(entries: &[Value]) -> Vec<Value> {
    entries
        .iter()
        .filter(|entry| !OPERATOR_GATED.contains(&verb(entry)))
        .cloned()
        .collect()
}
fn of<'a>(census: &'a Census, name: &str) -> TestResult<&'a [Value]> {
    census
        .get(name)
        .map(Vec::as_slice)
        .ok_or_else(|| format!("census of {name}").into())
}

/// The five classes of plan §7, and the privacy rule, as the providers'
/// `project_offer` gates are built.
fn projection_rules(census: &Census) -> TestResult {
    // A, operator-gated: the granted operator, and no one else.
    for gated in OPERATOR_GATED {
        for (name, entries) in census {
            check_eq!(
                verbs(entries).contains(gated),
                *name == "operator",
                "{gated} for {name}"
            );
        }
    }
    // B, service: in no census. Vacuous in an empty world (no intent exists
    // to project); the fixture stories prove each service's actor binding.
    for service in SERVICE {
        for (name, entries) in census {
            check!(!verbs(entries).contains(service), "{service} for {name}");
        }
    }
    // Every offer of a split verb is one of its known kinds, so no kind
    // escapes the classes below unclassified.
    for (name, entries) in census {
        for entry in entries
            .iter()
            .filter(|entry| SPLIT_BY_KIND.contains(&verb(entry)))
        {
            let kind = action(entry);
            check!(
                COUNTER_KINDS.contains(&kind) || GENERIC_KINDS.contains(&kind) || kind == GOD_KIND,
                "a known kind of {} for {name}: {entry}",
                verb(entry)
            );
        }
    }
    // C, counter-gated: what the counter adds is only counter-gated offers,
    // every counter-gated verb and kind is offered at the counter, and none
    // is offered outside it.
    let author = of(census, "author")?;
    let clerk = of(census, "clerk")?;
    for entry in minus(author, clerk) {
        check!(
            counter_gated(entry),
            "the counter adds only counter-gated offers: {entry}"
        );
    }
    check!(
        minus(clerk, author).is_empty(),
        "the counter removes nothing"
    );
    for (name, entries) in census {
        let here = AT_THE_COUNTER.contains(name);
        for gated in ONLY_AT_THE_COUNTER {
            check_eq!(verbs(entries).contains(gated), here, "{gated} for {name}");
        }
        for kind in COUNTER_KINDS {
            check_eq!(!kinds(entries, kind).is_empty(), here, "{kind} for {name}");
        }
    }
    // D, God-only: beside the clerk (the same room, outside the counter) God
    // gains only the v2 Expire kind of `market-original-mind-offer`, and no
    // one else is offered it. Vacuous in an empty world: Expire is projected
    // only while an expired quote waits (`mind_trade/provider.rs`), so here
    // God equals the clerk.
    let god = of(census, "god")?;
    for entry in minus(god, clerk) {
        check_eq!(action(entry), GOD_KIND, "God's extra {entry}");
    }
    for (name, entries) in census.iter().filter(|(name, _)| **name != "god") {
        check!(kinds(entries, GOD_KIND).is_empty(), "{GOD_KIND} for {name}");
    }
    // E, generic: every other C1 verb, and each generic kind of the two split
    // verbs exactly once, offered to every principal wherever it stands.
    for generic in C1_VERBS.iter().filter(|verb| {
        !OPERATOR_GATED.contains(verb)
            && !SERVICE.contains(verb)
            && !ONLY_AT_THE_COUNTER.contains(verb)
            && !SPLIT_BY_KIND.contains(verb)
    }) {
        for (name, entries) in census {
            check!(verbs(entries).contains(generic), "{generic} for {name}");
        }
    }
    for kind in GENERIC_KINDS {
        for (name, entries) in census {
            check_eq!(kinds(entries, kind).len(), 1, "{kind} for {name}");
        }
    }
    // Privacy: the same place gives byte-identical offers, whoever holds what.
    check_eq!(author, of(census, "buyer")?, "author and buyer");
    check_eq!(author, of(census, "accountless")?, "author and accountless");
    check_eq!(
        without_operator_gated(clerk),
        without_operator_gated(of(census, "operator")?),
        "clerk and operator outside their grants"
    );
    Ok(())
}

/// Seed, boot Initialize and submit `roll-initialize-commerce` as GOD (it must
/// be offered to GOD), then boot Active with the full roster: the risk-1
/// proof. Returns the running Active server and the seeded identities.
async fn initialized_active(
    config: &Config,
    store: &Arc<PurchaseFaultStore>,
) -> TestResult<(Running, Seeded)> {
    let seeded = seed(config, store).await?;
    let initialize = player_market_commerce(config, CommerceMode::Initialize, &seeded)?;
    let mut server = Running::boot(config, store.clone(), Some(&initialize)).await?;
    server.connect(GOD).await?;
    let offered = legal(&server, GOD).await?;
    check!(
        offered
            .iter()
            .any(|offer| offer.presentation().verb == "roll-initialize-commerce"),
        "GOD is offered the commerce initialization"
    );
    // A `busy` disposition is the world refusing before custody (another
    // action awaits its receipt); the offer is resubmitted within the guard.
    let end = tokio::time::Instant::now() + STEP;
    loop {
        match server
            .offer(GOD, "roll-initialize-commerce", json!({}))
            .await
        {
            Ok(_) => break,
            Err(error)
                if error.to_string().ends_with("disposition=busy")
                    && tokio::time::Instant::now() < end =>
            {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            Err(error) => return Err(error),
        }
    }
    server.stop().await?;
    let server = active(config, store, &seeded).await?;
    Ok((server, seeded))
}
/// One Active boot of the full roster, every principal connected. A restart
/// composes its owners afresh, as a new process does.
async fn active(
    config: &Config,
    store: &Arc<PurchaseFaultStore>,
    seeded: &Seeded,
) -> TestResult<Running> {
    let roster = player_market_commerce(config, CommerceMode::Active, seeded)?;
    let server = Running::boot(config, store.clone(), Some(&roster)).await?;
    for (_, actor) in seeded.principals.all() {
        server.connect(actor).await?;
    }
    Ok(server)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn player_market_roster_boots_initializes_and_restarts_through_game_modules() -> TestResult {
    timeout(STEP * 10, async {
        let config = config();
        let store = Arc::new(PurchaseFaultStore::new());
        let (mut server, seeded) = initialized_active(&config, &store).await?;
        let first = census(&server, &seeded.principals).await?;
        server.stop().await?;
        let mut server = active(&config, &store, &seeded).await?;
        let again = census(&server, &seeded.principals).await?;
        check_eq!(
            first,
            again,
            "the restarted roster projects the same offers"
        );
        server.stop().await?;
        Ok(())
    })
    .await?
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn player_market_offers_follow_the_projection_rules() -> TestResult {
    timeout(STEP * 10, async {
        let config = config();
        let store = Arc::new(PurchaseFaultStore::new());
        let (mut server, seeded) = initialized_active(&config, &store).await?;
        let found = census(&server, &seeded.principals).await?;
        projection_rules(&found)?;
        server.stop().await?;
        Ok(())
    })
    .await?
}
