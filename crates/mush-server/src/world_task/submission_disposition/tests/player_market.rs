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
//! (deployment format 2, `launch_world`): the first boot of the named launch
//! through `game_modules`. C1c composes the player market in the launch's
//! Active branch and proves C1's gate here (plan §5): the census of the
//! composed launch, a restart, split parity through `build_engine`, and a
//! terms change across a reboot.
use super::paid_purchase::{
    auth_account, open_wallet,
    runtime::{Running, STEP},
    store::PurchaseFaultStore,
};
use mush_config::config::Config;
use mush_core::{GOD, ObjId, PerceivedRef};
use mush_economy::{
    AccountId as WalletId, Amount, CatalogRef, InstrumentId, VaultId,
    core_integration::ScopedCoreAccount,
    managed_entitlement::{GrossRecipientCut, Recipient},
};
use mush_industry::core_production::OriginAgent;
use mush_inventory::Grade;
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
use tacsim::{
    TacsimRuleset,
    composition::mind_commerce::MindCommerce,
    launch::{
        HardwareCoordinates, HouseDeployment, LaunchDeployment, LaunchMode, LaunchVenue,
        MarketTerms, NamedHouses, PlayerMarket, RegistryAuthority,
    },
};
use tokio::time::timeout;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

const DAY: i64 = 86_400;
/// 400cr: C1's provisional personal and House Pin price (plan §6).
const PIN_MINOR: i64 = 40_000;
/// The Compact levy and the House venue cut, per mille.
const LEVY_PERMILLE: u16 = 50;
const REGISTRY: &str = "roll.registry";
const VENUE: &str = "launch.venue";
pub(super) const CREDIT: &str = "tacsim.credit";

/// The C1 offer verbs (plan §7), re-grepped at C1a from every
/// `ActionPresentation::new` under `mush-roll/src` and `mush-market/src`,
/// including the verbs built from `match` arms and `format!`. The re-grep
/// added one: `roll-retire-authored-draft`, which `with_authored_forks`
/// composes (authored retirement, two offers). The personal filing verbs
/// (`roll-reserve-mind-charter`, `roll-file-original-mind`) predate C1.
///
/// 17g C2a (C2 plan §2.3) split the four shared verbs one per kind, so the
/// census grew from 36 to 45 (three shared verbs out, eleven per-kind verbs
/// in, one retirement verb added):
/// - `market-original-mind-offer` became `market-original-mind-list`,
///   `-quote`, `-retire-listing`, `-retire-quote` and `-expire`;
/// - `market-original-mind-purchase` became `market-original-mind-fund`,
///   `-grant` and `-refund`;
/// - `paid-mind-resale` became `paid-mind-resale-offer`, `-accept` and `-fund`;
/// - `roll-retire-authored-draft`'s version 1 became
///   `roll-retire-authored-draft-v1` (version 2 keeps the verb).
///
/// The classes and every principal's entry count are unchanged by the split.
const C1_VERBS: [&str; 45] = [
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
    "market-original-mind-list",
    "market-original-mind-quote",
    "market-original-mind-retire-listing",
    "market-original-mind-retire-quote",
    "market-original-mind-expire",
    "market-original-mind-fund",
    "market-original-mind-grant",
    "market-original-mind-refund",
    "roll-file-dependent-mind",
    "roll-update-original-mind",
    "roll-update-derived-mind",
    "roll-fork-paid-mind",
    "roll-retire-authored-draft",
    "roll-retire-authored-draft-v1",
    "roll-offer-charter-transfer",
    "roll-accept-charter-transfer",
    "roll-offer-pending-charter",
    "roll-fund-pending-charter",
    "roll-settle-pending-charter",
    "roll-refund-pending-charter",
    "paid-mind-resale-offer",
    "paid-mind-resale-accept",
    "paid-mind-resale-fund",
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
/// 17g C2a: the three shared verbs the split retired. No projection carries
/// one (`roll-retire-authored-draft` stays, as version 2's verb).
const SPLIT_AWAY: [&str; 3] = [
    "paid-mind-resale",
    "market-original-mind-offer",
    "market-original-mind-purchase",
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
/// Class C verbs of the v2 market (`mind_trade/provider.rs` List and Quote,
/// `mind_trade/purchase_provider.rs` Fund and Grant). Before 17g C2a these
/// were kinds of the two shared verbs, told apart by the action each offer
/// routes to; since the split each is its own verb.
const COUNTER_KINDS: [&str; 4] = [
    "market-original-mind-list",
    "market-original-mind-quote",
    "market-original-mind-fund",
    "market-original-mind-grant",
];
/// Class D: the v2 Expire, offered to GOD while an expired quote waits.
const GOD_KIND: &str = "market-original-mind-expire";
/// Class E verbs of the v2 market: each principal has exactly one offer of
/// each, wherever it stands (RetireListing, RetireQuote and Refund).
const GENERIC_KINDS: [&str; 3] = [
    "market-original-mind-retire-listing",
    "market-original-mind-retire-quote",
    "market-original-mind-refund",
];
/// Where the census principals stand (`Principals`).
const AT_THE_COUNTER: [&str; 3] = ["author", "buyer", "accountless"];

pub(super) fn config() -> Config {
    let mut config = Config::default();
    config.tacsim.enabled = true;
    config.store.store = "memory".into();
    config.game.ic_ratio = 24;
    // The genuine rating's pace in the two market stories (`MIND_PULSE_MS`).
    super::paid_purchase::mind_pulse(&mut config);
    config
}
fn system(name: &str) -> TestResult<SystemPrincipalId> {
    SystemPrincipalId::new(name).ok_or_else(|| format!("system principal {name}").into())
}

/// The census principals. Author, buyer and the accountless character stand
/// in the counter room (the `ceres` exchange); GOD, the clerk and the Registry
/// operator stand together outside it, in Limbo.
#[derive(Clone, Copy, Debug)]
pub(super) struct Principals {
    pub(super) god: ObjId,
    pub(super) author: ObjId,
    pub(super) buyer: ObjId,
    pub(super) clerk: ObjId,
    pub(super) operator: ObjId,
    pub(super) accountless: ObjId,
}
impl Principals {
    pub(super) fn all(&self) -> [(&'static str, ObjId); 6] {
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
pub(super) async fn character(
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
pub(super) async fn create(
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
/// The one number a staff command's acknowledgement names right after
/// `marker` (`Created org #5.`, `Created: Depot(#12).`, `(id 3).`).
pub(super) fn created(lines: &[String], marker: &str) -> TestResult<u64> {
    let ids = lines
        .iter()
        .filter_map(|line| line.split_once(marker))
        .map(|(_, tail)| {
            tail.chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse::<u64>()
        })
        .collect::<Result<Vec<_>, _>>()?;
    let [id]: [u64; 1] = ids
        .try_into()
        .map_err(|_| format!("one acknowledged {marker}"))?;
    Ok(id)
}
pub(super) async fn staff(server: &Running, line: String) -> TestResult<Vec<String>> {
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
/// (the same for every actor; before 17g C2a's split it told apart the kinds
/// of the shared verbs, and it still pins each verb's route), sorted. All verbs found are also printed per principal,
/// for the record of what an empty Active world offers.
type Census = BTreeMap<&'static str, Vec<Value>>;
/// One principal's projected legal actions. A read that lands while the world
/// commits another action (a boot-time service step) answers `Busy`; it is
/// retried within the step guard, never treated as an empty projection.
pub(super) async fn legal(
    server: &Running,
    actor: ObjId,
) -> TestResult<Vec<mush_core::ProjectedLegalAction>> {
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
            // 17g C2a: the shared verbs are split; none is projected again.
            if SPLIT_AWAY.contains(&verb.as_str()) {
                return Err(format!("{name} is projected the old verb {verb}").into());
            }
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
/// The offers of one verb.
fn kinds<'a>(entries: &'a [Value], kind: &str) -> Vec<&'a Value> {
    entries.iter().filter(|entry| verb(entry) == kind).collect()
}
/// Whether an entry is class C: a counter-only verb of the v3 and derived
/// purchases, or a counter verb of the v2 market.
fn counter_gated(entry: &Value) -> bool {
    ONLY_AT_THE_COUNTER.contains(&verb(entry)) || COUNTER_KINDS.contains(&verb(entry))
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
    // gains only the v2 Expire (`market-original-mind-expire`), and no
    // one else is offered it. Vacuous in an empty world: Expire is projected
    // only while an expired quote waits (`mind_trade/provider.rs`), so here
    // God equals the clerk.
    let god = of(census, "god")?;
    for entry in minus(god, clerk) {
        check_eq!(verb(entry), GOD_KIND, "God's extra {entry}");
    }
    for (name, entries) in census.iter().filter(|(name, _)| **name != "god") {
        check!(kinds(entries, GOD_KIND).is_empty(), "{GOD_KIND} for {name}");
    }
    // E, generic: every other C1 verb, and each generic v2 verb exactly
    // once, offered to every principal wherever it stands.
    for generic in C1_VERBS.iter().filter(|verb| {
        !OPERATOR_GATED.contains(verb)
            && !SERVICE.contains(verb)
            && !ONLY_AT_THE_COUNTER.contains(verb)
            && !COUNTER_KINDS.contains(verb)
            && **verb != GOD_KIND
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
    submit_initialize(&server, "roll-initialize-commerce").await?;
    server.stop().await?;
    let server = active(config, store, &seeded).await?;
    Ok((server, seeded))
}
/// GOD submits one parameterless initialize offer through the public path;
/// the harness resubmits it while the world answers `Busy` ([`Running::offer`]).
async fn submit_initialize(server: &Running, verb: &str) -> TestResult {
    server.offer(GOD, verb, json!({})).await?;
    Ok(())
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

// ---- 17g C1b: the named launch through `game_modules` ----------------------

/// The named launch's two staff login accounts: GOD's, and the one holding the
/// stock agent, the market clerk and the Registry operator.
const LAUNCH_STAFF: &str = "launch-staff";
pub(super) const LAUNCH_STAFF_TWO: &str = "launch-staff-two";
/// The named Houses: organization name and configured source name.
const LAUNCH_HOUSES: [(&str, &str); 3] = [
    ("Meridiani", "Meridiani Field Series"),
    ("Coprates", "Coprates Drill"),
    ("Tycho", "Safe Conduct"),
];

/// A staff command whose acknowledgement must contain `expect`. Legacy
/// commands answer a refusal in text too, so the acknowledgement is read.
pub(super) async fn acknowledged(
    server: &Running,
    line: String,
    expect: &str,
) -> TestResult<Vec<String>> {
    let lines = staff(server, line.clone()).await?;
    check!(
        lines.iter().any(|text| text.contains(expect)),
        "{line} is acknowledged with {expect:?}: {lines:?}"
    );
    Ok(lines)
}
/// One `@create`d thing, carried by GOD.
async fn made(server: &Running, name: &str) -> TestResult<ObjId> {
    let lines = acknowledged(server, format!("@create {name}"), "Created: ").await?;
    Ok(ObjId(u32::try_from(created(&lines, "(#")?)?))
}
/// One `@org/create`d organization.
async fn organization(server: &Running, name: &str) -> TestResult<OrgId> {
    let lines = acknowledged(server, format!("@org/create {name}"), "Created org #").await?;
    Ok(OrgId(created(&lines, "Created org #")?))
}
/// One `@org/scope create`d scope of `org`.
async fn org_scope(server: &Running, org: &str, name: &str) -> TestResult<OrgScopeId> {
    let lines = acknowledged(
        server,
        format!("@org/scope create {org}={name}"),
        "Created scope #",
    )
    .await?;
    Ok(OrgScopeId(created(&lines, "Created scope #")?))
}
/// One acknowledged scoped org account.
async fn org_account(server: &Running, org: &str, scope: OrgScopeId) -> TestResult<WalletId> {
    let lines = acknowledged(
        server,
        format!("@account/open org:{org}/{}={CREDIT}", scope.0),
        "Opened account #",
    )
    .await?;
    Ok(WalletId(created(&lines, "Opened account #")?))
}
/// One `@org/grant` as GOD, acknowledged.
pub(super) async fn grant(
    server: &Running,
    actor: ObjId,
    org: &str,
    capability: &str,
    scope: OrgScopeId,
) -> TestResult {
    acknowledged(
        server,
        format!(
            "@org/grant #{}={org} {capability} scope=#{}",
            actor.0, scope.0
        ),
        "Granted",
    )
    .await?;
    Ok(())
}

/// 17g C1b: every named-launch identity, made in a plain boot (no launch
/// path) by production paths only. These are the identities `setup_with(2)`
/// and `Rig::provision` make in tacsim's launch suite:
/// - characters through `create_player` and `link_character`. GOD, the House
///   publisher and first stock agent, is on the `launch-staff` login; the
///   caller creates both staff logins before the boot;
/// - the counter room by `@dig`; the factory, the output container and the
///   three stock containers by `@create`;
/// - the Republic, the Registry and venue wallets, and a mint faucet;
/// - each House's org, `publication` scope, treasury, three publisher grants,
///   and one filing fee minted into the treasury;
/// - the Basin importer's scopes, stock vaults, accounts, facility, title and
///   agent grants;
/// - the `Registry` org with its `review` scope, a clerk and an operator.
///
/// No `@instrument/register` (production already has `tacsim.credit`) and no
/// direct world call. Nothing is granted to the operator: that is the
/// Initialize step's. Returns the format-2 deployment in Initialize mode with
/// the provisional terms. `deploy/instance/launch.json.example` follows its
/// shape.
pub(super) async fn launch_world(
    server: &Running,
    store: &PurchaseFaultStore,
) -> TestResult<LaunchDeployment> {
    let staff_two = store
        .list_accounts()
        .await?
        .into_iter()
        .find(|account| account.name == LAUNCH_STAFF_TWO)
        .and_then(|account| account.id)
        .ok_or("the second staff login")?;
    let login = (LAUNCH_STAFF_TWO, staff_two);
    let agent = character(server, store, login, "LaunchAgent").await?;
    let clerk = character(server, store, login, "MarketClerk").await?;
    let operator = character(server, store, login, "RegistryOperator").await?;
    let dug = acknowledged(server, "@dig Named launch counter".into(), "room number").await?;
    let room = ObjId(u32::try_from(created(&dug, "room number #")?)?);
    let factory = made(server, "Basin fitting shop").await?;
    let output = made(server, "Basin completed work").await?;
    let mut containers = Vec::new();
    for name in [
        "Basin die depot",
        "Basin agent stock one",
        "Basin agent stock two",
    ] {
        containers.push(made(server, name).await?);
    }
    let faction =
        acknowledged(server, "@faction/create Republic".into(), "Created faction").await?;
    let republic = mush_faction::FactionId(created(&faction, "(id ")?);
    let registry = open_wallet(
        server,
        store,
        PrincipalId::System(system("launch.registry")?),
    )
    .await?;
    let venue = open_wallet(server, store, PrincipalId::System(system(VENUE)?)).await?;
    let faucet = open_wallet(server, store, PrincipalId::System(system("launch.faucet")?)).await?;
    let mut houses = Vec::new();
    for (org, source_name) in LAUNCH_HOUSES {
        let id = organization(server, org).await?;
        let scope = org_scope(server, org, "publication").await?;
        let treasury = org_account(server, org, scope).await?;
        for capability in [
            mush_economy::registry_filing::HOUSE_PUBLICATION_CAPABILITY,
            "asset.title.transfer",
            "economy.account.post",
        ] {
            grant(server, GOD, org, capability, scope).await?;
        }
        // Exactly one launch filing fee (100cr, in minor units).
        acknowledged(
            server,
            format!("@econ/mint {}={} 10000", faucet.0, treasury.0),
            "Minted",
        )
        .await?;
        houses.push(HouseDeployment {
            organization: id,
            scope,
            treasury,
            publisher: GOD,
            source_name: source_name.into(),
        });
    }
    let [meridiani, coprates, tycho]: [HouseDeployment; 3] =
        houses.try_into().map_err(|_| "three named houses")?;
    // The Basin importer, as `Rig::provision` builds it.
    let importer = organization(server, "Basin").await?;
    let stock_scope = org_scope(server, "Basin", "die-stock").await?;
    let factory_scope = org_scope(server, "Basin", "factory").await?;
    let mut vaults = Vec::new();
    for (i, container) in containers.iter().enumerate() {
        let lines = acknowledged(
            server,
            format!(
                "@vault/create HardwareStock{i}=container:#{} owner=org:Basin scope={}",
                container.0, stock_scope.0
            ),
            "Created vault #",
        )
        .await?;
        vaults.push(VaultId(created(&lines, "Created vault #")?));
    }
    let [depot, stock_one, stock_two]: [VaultId; 3] =
        vaults.try_into().map_err(|_| "three stock vaults")?;
    let supplier = org_account(server, "Basin", stock_scope).await?;
    let integrator = org_account(server, "Basin", factory_scope).await?;
    acknowledged(
        server,
        format!("@facility/open {}=2 core_integration core_refit", factory.0),
        "opened with",
    )
    .await?;
    acknowledged(
        server,
        format!("@facility/owner {}=org:Basin", factory.0),
        "re-assigned",
    )
    .await?;
    acknowledged(
        server,
        format!(
            "@asset/title #{}=org:Basin scope={}",
            factory.0, factory_scope.0
        ),
        "Title recorded",
    )
    .await?;
    for actor in [GOD, agent] {
        for capability in ["industry.core_die.issue", "vault.deposit", "vault.withdraw"] {
            grant(server, actor, "Basin", capability, stock_scope).await?;
        }
        for capability in ["industry.core.integrate", "industry.core.refit"] {
            grant(server, actor, "Basin", capability, factory_scope).await?;
        }
    }
    // The player market's Registry, as `setup_with(2)` makes it.
    let registry_org = organization(server, "Registry").await?;
    let review = org_scope(server, "Registry", "review").await?;
    let recipient = |owner: &str, account: WalletId| -> TestResult<Recipient> {
        Ok(Recipient {
            account,
            owner: PrincipalId::System(system(owner)?),
        })
    };
    Ok(LaunchDeployment {
        format: 2,
        mode: LaunchMode::Initialize,
        registry_issuer: system("hardware.factory")?,
        registry: recipient("launch.registry", registry)?,
        venue: LaunchVenue {
            name: MarketId::new("launch.counter").ok_or("counter venue")?,
            room,
            recipient: recipient(VENUE, venue)?,
        },
        houses: NamedHouses {
            meridiani,
            coprates,
            tycho,
        },
        republic,
        hardware: HardwareCoordinates {
            importer,
            stock_scope,
            depot,
            agents: [
                OriginAgent {
                    actor: GOD,
                    stock: stock_one,
                },
                OriginAgent {
                    actor: agent,
                    stock: stock_two,
                },
            ],
            grades: [Grade::new(10)?, Grade::new(20)?, Grade::new(30)?],
            supplier_account: supplier,
            integrator: ScopedCoreAccount {
                account: integrator,
                organization: importer,
                scope: factory_scope,
            },
            factory,
            output_container: output,
        },
        market: Some(PlayerMarket {
            clerk,
            registry_authority: RegistryAuthority {
                organization: registry_org,
                scope: review,
                operators: vec![operator],
            },
            terms: MarketTerms::PROVISIONAL,
        }),
    })
}
/// Write the deployment where `[tacsim.minds].launch_path` names it.
pub(super) fn write_launch(path: &std::path::Path, deployment: &LaunchDeployment) -> TestResult {
    std::fs::write(path, serde_json::to_vec_pretty(deployment)?)?;
    Ok(())
}

/// The census logins of the named launch: an author and a buyer, each its own
/// player account (C1 plan §4.4 test 6).
pub(super) const LAUNCH_AUTHOR: &str = "launch-author";
pub(super) const LAUNCH_BUYER: &str = "launch-buyer";
/// The four initialize offers of the named launch, all GOD's.
const LAUNCH_INITIALIZE: [&str; 4] = [
    "roll-initialize-commerce",
    "initialize-core-die-stock",
    "initialize-core-factory",
    "initialize-core-refit",
];
/// A fresh scratch directory for one test's data dir.
pub(super) fn scratch(prefix: &str) -> TestResult<std::path::PathBuf> {
    let dir = std::env::temp_dir().join(format!(
        "{prefix}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}
/// A named launch brought up by the runbook, ready for its Active boots.
pub(super) struct Launch {
    /// `[tacsim.minds].launch_path` names `path`.
    pub(super) config: Config,
    pub(super) store: Arc<PurchaseFaultStore>,
    pub(super) path: std::path::PathBuf,
    /// The document at `path`, in Active mode.
    pub(super) deployment: LaunchDeployment,
    /// The census principals. The counter is the deployment's venue room.
    pub(super) principals: Principals,
}
/// The runbook's bring-up through `game_modules` (`Running::boot(.., None)`
/// reads `[tacsim.minds].launch_path`):
/// - a plain boot: `launch_world`'s identities, then the census principals
///   (an author and a buyer on two more login accounts with their wallets,
///   and an accountless character, standing in the deployment's counter room;
///   God, the clerk and the operator in Limbo);
/// - an Initialize boot: the operators' two `@org/grant`s, then the four
///   initialize offers through the public offer path;
/// - the document rewritten in Active mode. Nothing is booted Active here.
pub(super) async fn initialized_launch(data_dir: &std::path::Path) -> TestResult<Launch> {
    initialized_launch_format(data_dir, 2).await
}
/// 17g C2a: the same bring-up on deployment format `format`. Format 1 is the
/// House-only launch: the document names no player market, so no operator
/// grant is made; the census principals are created exactly as on format 2.
pub(super) async fn initialized_launch_format(
    data_dir: &std::path::Path,
    format: u16,
) -> TestResult<Launch> {
    initialized_launch_with(data_dir, format, MarketTerms::PROVISIONAL).await
}
/// 17g C2a: the bring-up with the market's `terms` (the C2 story's 30 s quote).
pub(super) async fn initialized_launch_with(
    data_dir: &std::path::Path,
    format: u16,
    terms: MarketTerms,
) -> TestResult<Launch> {
    let mut config = config();
    config.server.data_dir = data_dir.to_path_buf();
    let store = Arc::new(PurchaseFaultStore::new());
    auth_account(&store, LAUNCH_STAFF, Some(GOD)).await?;
    auth_account(&store, LAUNCH_STAFF_TWO, None).await?;
    let author_account = auth_account(&store, LAUNCH_AUTHOR, None).await?;
    let buyer_account = auth_account(&store, LAUNCH_BUYER, None).await?;
    let mut server = Running::boot(&config, store.clone(), None).await?;
    server.connect(GOD).await?;
    let mut deployment = launch_world(&server, &store).await?;
    if let Some(market) = deployment.market.as_mut() {
        market.terms = terms;
    }
    let market = deployment.market.clone().ok_or("a format-2 market")?;
    let author = character(
        &server,
        &store,
        (LAUNCH_AUTHOR, author_account),
        "LaunchAuthor",
    )
    .await?;
    let buyer = character(
        &server,
        &store,
        (LAUNCH_BUYER, buyer_account),
        "LaunchBuyer",
    )
    .await?;
    let accountless = create(&server, "LaunchDrifter", None).await?;
    for actor in [author, buyer] {
        open_wallet(&server, &store, PrincipalId::Object(actor)).await?;
    }
    let operator = market
        .registry_authority
        .operators
        .first()
        .copied()
        .ok_or("one Registry operator")?;
    // The census's class C assertion proves these placements took.
    for actor in [author, buyer, accountless] {
        server
            .command(
                GOD,
                format!("@teleport #{}=#{}", actor.0, deployment.venue.room.0),
            )
            .await?;
    }
    for actor in [GOD, market.clerk, operator] {
        server
            .command(GOD, format!("@teleport #{}=#0", actor.0))
            .await?;
    }
    server.stop().await?;
    if format == 1 {
        deployment.format = 1;
        deployment.market = None;
    }
    let path = data_dir.join("launch.json");
    write_launch(&path, &deployment)?;
    config.tacsim.minds.launch_path = Some(path.display().to_string());
    // Initialize, through `game_modules`: the runbook's grants, then the
    // initialize offers.
    let mut server = Running::boot(&config, store.clone(), None).await?;
    server.connect(GOD).await?;
    let authority = &market.registry_authority;
    for operator in authority.operators.iter().filter(|_| format == 2) {
        for capability in ["roll.registry.recall", "roll.registry.revoke"] {
            grant(&server, *operator, "Registry", capability, authority.scope).await?;
        }
    }
    for verb in LAUNCH_INITIALIZE {
        submit_initialize(&server, verb).await?;
    }
    server.stop().await?;
    deployment.mode = LaunchMode::Active;
    write_launch(&path, &deployment)?;
    Ok(Launch {
        config,
        store,
        path,
        deployment,
        principals: Principals {
            god: GOD,
            author,
            buyer,
            clerk: market.clerk,
            operator,
            accountless,
        },
    })
}
/// One Active boot of the launch through `game_modules`, every census
/// principal connected. Each boot composes its owners afresh.
pub(super) async fn launch_active(launch: &Launch) -> TestResult<Running> {
    let server = Running::boot(&launch.config, launch.store.clone(), None).await?;
    for (_, actor) in launch.principals.all() {
        server.connect(actor).await?;
    }
    Ok(server)
}
/// The route of every C1 offer projected to each census principal.
async fn census_routes(
    server: &Running,
    principals: &Principals,
) -> TestResult<Vec<(String, mush_core::OfferRouteId)>> {
    let mut routes = Vec::new();
    for (_, actor) in principals.all() {
        for offer in legal(server, actor).await? {
            let verb = offer.presentation().verb.as_str();
            if C1_VERBS.contains(&verb) {
                routes.push((verb.to_owned(), offer.offer().route_id()));
            }
        }
    }
    Ok(routes)
}

/// 17g C1b test 5: the first boot of the named launch through `game_modules`,
/// the production composition (`Running::boot(.., None)` reads
/// `[tacsim.minds].launch_path`). A format-2 deployment, its identities made
/// by `launch_world`, boots Initialize. The operator's two runbook grants are
/// made, and `roll-initialize-commerce` and the three industry initialize
/// offers commit through the public offer path. Then Active boots, stops and
/// boots again. Since 17g C1c the Active launch composes the player market, so
/// the granted operator is offered both Registry actions.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn named_launch_format_two_initializes_and_activates_through_game_modules() -> TestResult {
    let data_dir = scratch("mush-named-launch")?;
    let result = timeout(STEP * 10, async {
        let launch = initialized_launch(&data_dir).await?;
        let deployment = &launch.deployment;
        check_eq!(deployment.format, 2);
        let market = deployment.market.clone().ok_or("a format-2 market")?;
        check_eq!(&market.terms, &MarketTerms::PROVISIONAL);
        eprintln!(
            "NAMED_LAUNCH_DEPLOYMENT {}",
            serde_json::to_string(deployment)?
        );
        let authority = &market.registry_authority;
        // Active, and a restart of Active: each a fresh composition.
        for _ in 0..2 {
            let mut server = Running::boot(&launch.config, launch.store.clone(), None).await?;
            server.connect(GOD).await?;
            // The Active launch composed its industry hardware, and the four
            // Initialize commits survived the restart: GOD, an origin agent
            // with the importer's grants, is offered the Active issue verb and
            // no initialize verb (`core_production/provider.rs` offers Issue
            // only to an Active, initialized origin).
            let verbs: Vec<String> = legal(&server, GOD)
                .await?
                .iter()
                .map(|offer| offer.presentation().verb.as_str().to_owned())
                .collect();
            check!(
                verbs.iter().any(|verb| verb == "issue-core-die"),
                "the Active launch offers GOD issue-core-die: {verbs:?}"
            );
            for initialize in LAUNCH_INITIALIZE {
                check!(
                    !verbs.iter().any(|verb| verb == initialize),
                    "the Active launch still offers {initialize}: {verbs:?}"
                );
            }
            // 17g C1c (replacing C1b's interim pin): the Active launch
            // composes the player market, so each granted operator is offered
            // both Registry actions.
            for operator in &authority.operators {
                server.connect(*operator).await?;
                let offered = legal(&server, *operator).await?;
                for gated in OPERATOR_GATED {
                    check!(
                        offered
                            .iter()
                            .any(|offer| offer.presentation().verb == gated),
                        "the granted operator is offered {gated}"
                    );
                }
            }
            server.stop().await?;
        }
        Ok(())
    })
    .await;
    let _ = std::fs::remove_dir_all(&data_dir);
    result?
}

/// 17g C1c test 6, C1's gate (plan §5). The launch's player market, composed
/// by the production format-2 deployment through `launch_path`, with the
/// operator granted at Initialize by `@org/grant`:
/// - **the census.** C1a's census of the C1 verbs for God, an author and a
///   buyer on two login accounts, the clerk, the operator and an accountless
///   character, the counter being the deployment's venue room, holds the five
///   classes of plan §7 and the privacy assertion (`projection_rules`);
/// - **a restart.** A second Active boot projects the identical census;
/// - **split and all-in-one parity.** The value is a real Active boot through
///   `boot::build_engine`, the split root, over the same store, with its own
///   durable key rings in a scratch data dir. Its offer route table has the
///   census server's length, and every census offer's route resolves to the
///   same `DefinitionRef` in both (a route id derives unkeyed from the
///   action's `DefinitionRef`, so this holds across key rings). mushd's
///   all-in-one delegates to that same root
///   (`crates/mushd/tests/durable_process/boot_composition.rs`,
///   `mushd_delegates_the_composition_root`), which is how the parity covers
///   both halves.
///
/// The market is empty here: no filing, listing, licence, charter or service
/// intent. Per-step restart of the new owners is C4's (plan §5).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn named_launch_composes_the_player_market_with_parity_and_restart() -> TestResult {
    let data_dir = scratch("mush-launch-market")?;
    let result = timeout(STEP * 10, async {
        let launch = initialized_launch(&data_dir).await?;
        let mut server = launch_active(&launch).await?;
        let first = census(&server, &launch.principals).await?;
        projection_rules(&first)?;
        let offered = census_routes(&server, &launch.principals).await?;
        check!(!offered.is_empty(), "the census offers C1 verbs");
        server.stop().await?;
        // A restart composes the same market and projects the same offers.
        let mut server = launch_active(&launch).await?;
        let again = census(&server, &launch.principals).await?;
        check_eq!(
            first,
            again,
            "the restarted launch projects the same offers"
        );
        server.stop().await?;
        // The split root over the same store, with its own key rings.
        let mut split = launch.config.clone();
        split.server.data_dir = data_dir.join("split");
        std::fs::create_dir_all(&split.server.data_dir)?;
        let store: Arc<dyn ObjectStore> = launch.store.clone();
        let booted = crate::boot::build_engine(
            &split,
            &store,
            &crate::quarantine::QuarantineSpool::in_memory(),
        )
        .await?;
        let routes = booted
            .engine
            .registry
            .action_providers()
            .ok_or("the split root's authoritative providers")?
            .offer_route_table();
        check_eq!(
            routes.len(),
            server.routes().len(),
            "the split root routes as many actions"
        );
        for (verb, route) in &offered {
            let ours = server
                .routes()
                .get(*route)
                .ok_or_else(|| format!("{verb} routes in the census server"))?;
            check_eq!(
                routes.get(*route),
                Some(ours),
                "{verb} routes to the same action in the split root"
            );
        }
        drop(booted);
        Ok(())
    })
    .await;
    let _ = std::fs::remove_dir_all(&data_dir);
    result?
}

/// 17g C1c test 7 (C1 plan §4.4, §8): what a terms change does to an
/// initialized format-2 world. After the census, the document's
/// `price_minor` becomes 45,000 and its `renewal_offer_ic_seconds` 172,800,
/// and the launch boots Active again. This records current behaviour; it is
/// not a defect to fix in C1, and no owner changes to make the reboot pass.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn named_launch_market_terms_change_across_a_reboot() -> TestResult {
    let data_dir = scratch("mush-launch-terms")?;
    let result =
        timeout(STEP * 10, async {
            let mut launch = initialized_launch(&data_dir).await?;
            let mut server = launch_active(&launch).await?;
            projection_rules(&census(&server, &launch.principals).await?)?;
            server.stop().await?;
            let market = launch
                .deployment
                .market
                .as_mut()
                .ok_or("a format-2 market")?;
            market.terms.price_minor = 45_000;
            market.terms.renewal_offer_ic_seconds = 172_800;
            write_launch(&launch.path, &launch.deployment)?;
            match launch_active(&launch).await {
                Ok(mut server) => {
                    eprintln!("NAMED_LAUNCH_TERMS_CHANGE booted");
                    let lines = server.command(GOD, "@mind/launch".into()).await?;
                    check!(
                        lines.iter().any(|line| line.contains("sells for 450cr")
                            && line.contains("open 2 IC days;")),
                        "the changed terms are the market line's: {lines:?}"
                    );
                    projection_rules(&census(&server, &launch.principals).await?)?;
                    server.stop().await?;
                }
                Err(error) => {
                    eprintln!("NAMED_LAUNCH_TERMS_CHANGE refused {error}");
                    return Err(error);
                }
            }
            Ok(())
        })
        .await;
    let _ = std::fs::remove_dir_all(&data_dir);
    result?
}
