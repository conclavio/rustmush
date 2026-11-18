//! **The kit doc 17f §9's stories 2 and 7 share on the combined M8 world** (doc 47 unit CO.6;
//! doc 44 unit MW.5, item 1): a world of several players, each a character on a gateway
//! account of their own; a typed line landed on the composed path with the commit tokens of
//! what it submitted; a contract with one `minds` fact condition, drafted, offered and accepted
//! by its two parties by their own `@contract` verbs; and the readings of the Roll's typed rows
//! the two stories assert on.
//!
//! Nothing here acts for a player. Staff act only where a helper's comment says so.

use std::collections::BTreeMap;

use mush_core::{CommitToken, GOD, ObjId};
use mush_economy::{ContractId, ContractState, EconomicContract};
use serde_json::Value;

use crate::excursions::{economy, run};
use crate::m8_world::{M8World, m8_world_manned};
use crate::mw21_completion::number;

/// The players' Dray stores, as MW.21's buyer has them (`excursions::docked_dray`).
const STORES: (u64, u64) = (53, 19);

/// **A player of the story**: the character, and the hull of their chartered Dray.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Player {
    pub(crate) who: ObjId,
    pub(crate) hull: ObjId,
}

/// **The combined world with `cast` chartered at Ceres** (each `(name, seed, ship)`): the
/// first is the M8 rig's pilot, docked and aboard (`docked_dray`); every other is chartered
/// and stocked the same way and aboard their own Dray. Each is a character on a gateway
/// account of their own, connected. Staff stow each Dray's tank and locker, as every ship
/// story's rig does.
pub(crate) fn world(cast: &[(&str, i64, &str)]) -> (M8World, Vec<Player>) {
    let mut players = Vec::new();
    let world = m8_world_manned(None, |rig| {
        let mut first = None;
        for (name, seed, ship) in cast {
            let (who, row) = crate::excursions::docked_dray(rig, (name, *seed, ship), STORES);
            players.push(Player {
                who,
                hull: ObjId(row.hull),
            });
            if first.is_none() {
                first = Some((who, row));
            }
        }
        let (pilot, row) = first.expect("a cast");
        (pilot, row, Vec::new())
    });
    let mut world = world;
    for player in &players {
        if !world.rig.tw.world().is_connected(player.who) {
            let _ = world.rig.tw.connect(player.who);
        }
    }
    (world, players)
}

/// **Every pair of the cast is two accounts, nobody is staff, and a staff verb is refused
/// each of them** (MW.21's clause 1, for more than two).
pub(crate) fn nobody_is_staff(world: &mut M8World, players: &[Player]) {
    for (i, a) in players.iter().enumerate() {
        assert_ne!(a.who, GOD);
        assert!(
            world.rig.tw.world().is_connected(a.who),
            "{a:?} is connected"
        );
        for b in &players[i + 1..] {
            assert!(
                crate::excursions::two_accounts(&world.rig, a.who, b.who),
                "{a:?} and {b:?} are characters of two accounts"
            );
        }
        assert_eq!(
            run(&mut world.rig.tw, a.who, "@econ/mint 1=2 100"),
            "Permission denied.",
            "a staff verb is refused {a:?}"
        );
    }
}

/// **Land every submission the engine holds**, each by its owner's path: a Roll or Economy
/// service's own submission (the minds-fact lapse recorder's, a renewal's) as a service's is
/// landed (`land_service`, its own idempotency key), and everything else as the world task
/// lands a tick's (`land`). The M8 rig's `fires` drains only the port's lanes and the market's
/// and drops the rest, which would strand a Roll service's submission (its observation never
/// ends under the rig's runtime), so the stories pump with [`fires`] here instead.
fn land_held(world: &mut M8World) {
    while let Some(submit) = world.rig.tw.engine_mut().take_authoritative_submit() {
        let owner = submit.action.key().owner.as_str();
        let landed = if matches!(owner, "roll" | "economy") {
            crate::excursions::land_service(&mut world.rig, &submit)
        } else {
            crate::excursions::land(&mut world.rig, &submit)
        };
        if let Err(refused) = landed {
            eprintln!(
                "C4S {} by #{} did not land: {refused}",
                submit.action.key().id,
                submit.actor.0
            );
        }
    }
}

/// **Fire the world's ticks `n` times**, six seconds apart, landing everything they submit
/// ([`land_held`]).
pub(crate) fn fires(world: &mut M8World, n: usize) {
    for _ in 0..n {
        let t = world.rig.tw.world().now().saturating_add(6);
        world.rig.tw.set_now(t);
        let _ = world.rig.tw.fire_due(t.saturating_mul(1_000));
        land_held(world);
    }
}

/// **Move the clock to the first real second whose IC instant is at or past `target`**
/// (`excursions::to_ic`), landing everything that fire submits ([`land_held`]).
pub(crate) fn to_ic(world: &mut M8World, target: i64) {
    let rate = i64::from(world.rig.tw.world().world_timeline().current().rate.max(1));
    let ahead = target
        .saturating_sub(crate::excursions::ic_now(&world.rig.tw))
        .max(0);
    let t = world
        .rig
        .tw
        .world()
        .now()
        .saturating_add(((ahead + rate - 1) / rate).max(1));
    world.rig.tw.set_now(t);
    let _ = world.rig.tw.fire_due(t.saturating_mul(1_000));
    land_held(world);
}

/// **`who` types `line` on their own connection** (the server's dispatch binds a connection
/// to its actor; the workshop's fork jobs answer on it): what `who` read.
fn typed(world: &mut M8World, who: ObjId, line: &str) -> String {
    let Some(conn) = (1..=256).find(|conn| world.rig.tw.engine().actor_of(*conn) == Some(who))
    else {
        return run(&mut world.rig.tw, who, line);
    };
    let mut out = mush_core::Outbox::new();
    world.rig.tw.engine_mut().step(
        mush_core::Inbound::Cmd {
            conn,
            actor: who,
            line: line.to_owned(),
        },
        &mut out,
        &mut mush_core::Effects::new(),
    );
    let events = out.take();
    crate::excursions::said(&world.rig.tw, &events, who)
}

/// **[`acts`] for a line that may be refused**: what `who` read, and each submission's commit
/// token or the provider's refusal (the server's driver prints that refusal to its typist).
pub(crate) fn attempts(
    world: &mut M8World,
    who: ObjId,
    line: &str,
) -> (String, Vec<Result<CommitToken, String>>) {
    // What a tick queued before the line is the tick's, not the typist's.
    land_held(world);
    let heard = typed(world, who, line);
    let mut staged = Vec::new();
    while let Some(submit) = world.rig.tw.engine_mut().take_authoritative_submit() {
        staged.push(*submit);
    }
    let landed: Vec<_> = staged
        .iter()
        .map(|submit| {
            assert_eq!(submit.actor, who, "{line}: its typist's: {submit:?}");
            crate::excursions::land_commit(&mut world.rig, submit).map(|(token, _)| token)
        })
        .collect();
    eprintln!("C4S #{} types `{line}` -> {heard} {landed:?}", who.0);
    (heard, landed)
}

/// **Type `line` as `who` and land what it submits** on the composed path (MW.21's `types`),
/// with the commit token of each submission landed, in order; a refused landing fails the
/// story.
pub(crate) fn acts(world: &mut M8World, who: ObjId, line: &str) -> (String, Vec<CommitToken>) {
    let (heard, landed) = attempts(world, who, line);
    let tokens = landed
        .into_iter()
        .map(|landed| landed.unwrap_or_else(|refused| panic!("{line}: {refused}")))
        .collect();
    (heard, tokens)
}

/// Whether nothing `line` submitted landed (it submitted nothing, or the provider refused
/// every submission).
pub(crate) fn nothing_landed(landed: &[Result<CommitToken, String>]) -> bool {
    landed.iter().all(Result::is_err)
}

/// [`acts`], what `who` read.
pub(crate) fn says(world: &mut M8World, who: ObjId, line: &str) -> String {
    acts(world, who, line).0
}

/// `who`'s one personal credit account, its number.
pub(crate) fn account(world: &M8World, who: ObjId) -> u64 {
    crate::m8_end_user::credit(world, who).0.0
}

/// The contract `id` as the Economy holds it.
pub(crate) fn contract(world: &M8World, id: ContractId) -> EconomicContract {
    economy(&world.rig.tw)
        .snapshot()
        .contract(id)
        .cloned()
        .expect("the contract")
}

/// `@contract/draft <name>` typed by `who`: the new draft.
pub(crate) fn draft(world: &mut M8World, who: ObjId, name: &str) -> ContractId {
    let heard = says(world, who, &format!("@contract/draft {name}"));
    let issuer = mush_org::PrincipalId::Object(who);
    economy(&world.rig.tw)
        .snapshot()
        .contracts()
        .filter(|c| c.issuer == issuer && c.name == name)
        .map(|c| c.id)
        .max()
        .unwrap_or_else(|| panic!("{who}'s draft {name}: {heard}"))
}

/// A `minds` fact condition's text: `fact minds <kind> <id> revision=<revision>`.
pub(crate) fn fact(kind: &str, id: &str, revision: u64) -> String {
    format!("fact minds {kind} {id} revision={revision}")
}

/// A filed revision's fact id (`filed/` and sixteen hex digits).
pub(crate) fn filed_key(filed: u64) -> String {
    format!("filed/{filed:016x}")
}

/// A licence's fact id (`licence/` and sixteen hex digits).
pub(crate) fn licence_key(licence: u64) -> String {
    format!("licence/{licence:016x}")
}

/// **A one-condition contract between `issuer` and `party`, Active**: `issuer` drafts it, names
/// `party`, escrows `amount` minor units of their own credit for `party` on `condition` (a
/// [`fact`]), offers it, and both accept, each by their own `@contract` verbs. The id.
pub(crate) fn conditioned(
    world: &mut M8World,
    (issuer, party): (ObjId, ObjId),
    name: &str,
    condition: &str,
    amount: i64,
) -> ContractId {
    let id = draft(world, issuer, name);
    let (payer, payee) = (account(world, issuer), account(world, party));
    for line in [
        format!("@contract/party {id}=#{}", party.0),
        format!("@contract/pay {id}={payer} {amount}"),
        format!("@contract/split {id}={payee} {amount}"),
        format!("@contract/remainder {id}={payer}"),
    ] {
        let _ = says(world, issuer, &line);
    }
    let attached = says(
        world,
        issuer,
        &format!("@contract/condition {id}={condition}"),
    );
    assert_eq!(
        contract(world, id).consent.conditions.len(),
        1,
        "{issuer} attaches `{condition}`: {attached}"
    );
    for (who, verb) in [(issuer, "offer"), (issuer, "accept"), (party, "accept")] {
        let _ = says(world, who, &format!("@contract/{verb} {id}"));
    }
    assert_eq!(
        contract(world, id).state,
        ContractState::Active,
        "{name}: both parties accepted, and the fact is not known yet"
    );
    id
}

/// Whether contract `id` has settled.
pub(crate) fn settled(world: &M8World, id: ContractId) -> bool {
    matches!(contract(world, id).state, ContractState::Settled { .. })
}

/// **Fire the world's ticks until contract `id` settles** (the Economy settles a contract only
/// in its sweep, one bounded pass every thirty seconds; a fire is six), at most `bound` fires.
/// Whether it settled.
pub(crate) fn until_settled(world: &mut M8World, id: ContractId, bound: usize) -> bool {
    for _ in 0..bound {
        if settled(world, id) {
            return true;
        }
        fires(world, 1);
    }
    settled(world, id)
}

/// **The typed rows of `family`, by key**: what a restart boots from.
pub(crate) fn keyed(world: &M8World, family: &str) -> BTreeMap<String, Value> {
    world
        .rig
        .typed
        .iter()
        .filter(|(_, v)| v["family"]["id"].as_str() == Some(family))
        .map(|((ns, key), v)| (format!("{ns}/{key}"), v["value"].clone()))
        .collect()
}

/// The current pointer row of paid licence `licence` (`roll_mind_personal_paid_licence`).
pub(crate) fn paid_licence(world: &M8World, licence: u64) -> Value {
    world
        .rows("roll_mind_personal_paid_licence")
        .into_iter()
        .find(|row| number(row, "id") == licence)
        .unwrap_or_else(|| panic!("paid license {licence}'s row"))
}

/// Who holds paid licence `licence` now.
pub(crate) fn holder(world: &M8World, licence: u64) -> u64 {
    number(&paid_licence(world, licence), "holder")
}

/// Paid licence `licence`'s term of generation `generation`, if it has one.
pub(crate) fn paid_term(
    world: &M8World,
    licence: u64,
    generation: u64,
) -> Option<mush_roll::commerce::paid::PaidTermRecord> {
    world
        .rows("roll_mind_paid_term")
        .into_iter()
        .filter_map(|row| {
            serde_json::from_value::<mush_roll::commerce::paid::PaidTermRecord>(row).ok()
        })
        .find(|term| term.licence().get() == licence && term.generation().get() == generation)
}

/// The paid licences `who` holds now, by number.
pub(crate) fn held_by(world: &M8World, who: ObjId) -> Vec<u64> {
    world
        .rows("roll_mind_personal_paid_licence")
        .into_iter()
        .filter(|row| row["holder"] == serde_json::json!(who.0))
        .map(|row| number(&row, "id"))
        .collect()
}

/// **Seal `licence` aboard `player`'s Dray at her berth** (`+ship/mind filed:<n> go`, MW.13's
/// `seal_aboard` for any player): the line, and whether the Roll opened the claim.
pub(crate) fn seal(world: &mut M8World, player: Player, licence: u64) -> (String, bool) {
    let (heard, landed) = attempts(world, player.who, &format!("+ship/mind filed:{licence} go"));
    if nothing_landed(&landed) {
        return (format!("{heard} {landed:?}"), false);
    }
    for _ in 0..12 {
        fires(world, 1);
        if tacsim::port::ship_mind::aboard(world.rig.tw.world(), player.hull).is_some() {
            return (heard, true);
        }
    }
    (heard, false)
}

/// **Unseal `player`'s Dray at her berth** (`+ship/mind off go`) and fire until the claim the
/// seal opened has closed. Whether it closed.
pub(crate) fn unseal(world: &mut M8World, player: Player) -> bool {
    let (heard, tokens) = acts(world, player.who, "+ship/mind off go");
    assert_eq!(tokens.len(), 1, "one submission: {heard}");
    for _ in 0..24 {
        fires(world, 1);
        if tacsim::ic_claim_of(world.rig.tw.world(), player.hull) == tacsim::IcClaimRead::None {
            return true;
        }
    }
    false
}

/// **`who` leaves their Dray and staff carry them to the counter** (the M8 rig's counter is a
/// room staff made, with no door: MW.21's fiat 2).
pub(crate) fn to_the_counter(world: &mut M8World, who: ObjId) {
    let aboard = tacsim::space::hull_aboard_for_actor(world.rig.tw.world(), who).is_some();
    if aboard {
        let ashore = says(world, who, "+ship/leave");
        assert!(
            tacsim::space::hull_aboard_for_actor(world.rig.tw.world(), who).is_none(),
            "{who} is ashore: {ashore}"
        );
    }
    let counter = world.deployment.venue.room;
    crate::excursions::teleport(&mut world.rig.tw, who, counter);
}

/// **Staff carry `player` back to the berth gallery and they board their Dray** by their own
/// verb (MW.21's fiat 2, the way back).
pub(crate) fn back_aboard(world: &mut M8World, player: Player) {
    let gallery =
        crate::excursions::station_room(&world.rig.tw, "ceres_port", "room.c01_berth_gallery");
    crate::excursions::teleport(&mut world.rig.tw, player.who, gallery);
    let aboard = says(world, player.who, "+ship/board");
    assert_eq!(
        tacsim::space::hull_aboard_for_actor(world.rig.tw.world(), player.who),
        Some(player.hull),
        "{player:?} is aboard: {aboard}"
    );
}

/// **The listing code on the counter's board whose row holds every one of `needles`**, read by
/// `who` at the counter (`+minds`).
pub(crate) fn listing_code(world: &mut M8World, who: ObjId, needles: &[&str]) -> String {
    let board = says(world, who, "+minds");
    crate::mw21_completion::fits(&board);
    let row = board
        .lines()
        .find(|line| needles.iter().all(|needle| line.contains(needle)))
        .unwrap_or_else(|| panic!("a row with {needles:?}: {board}"));
    row.split_whitespace()
        .next()
        .expect("the row's code")
        .to_owned()
}

/// **`who` buys from listing `code` at the counter**: a quote, its payment into escrow, the
/// licence taken (`+minds/buy`, `/buy/pay`, `/buy/complete`). The paid licences `who` holds
/// that they did not before.
pub(crate) fn buys(world: &mut M8World, who: ObjId, code: &str) -> Vec<u64> {
    let before = held_by(world, who);
    let _ = says(world, who, &format!("+minds/buy {code}"));
    let _ = says(world, who, &format!("+minds/buy/pay {code}"));
    let _ = says(world, who, &format!("+minds/buy/complete {code}"));
    held_by(world, who)
        .into_iter()
        .filter(|licence| !before.contains(licence))
        .collect()
}

/// **Save `who`'s draft `name`, have the Registry rate it and file it** by the workshop's and
/// the Registry's verbs (`@mind/save`, `@mind/rate`, `@mind/file`, twice: the charter's shell,
/// then the filing), from `who`'s own wallet: MW.21's `writes_rates_and_files` for a draft that
/// already exists (a fork's), after typing `edits` into it (a filing whose source is another
/// filing's is a duplicate, so a fork is filed only once its author has changed it). The filed
/// number and the owner's licence.
pub(crate) async fn saves_rates_and_files(
    world: &mut M8World,
    who: ObjId,
    name: &str,
    edits: &[&str],
) -> (u64, u64) {
    use mush_roll::commerce::rating::{RatingJob, RatingJobState, RatingReceipt};
    use std::time::{Duration, Instant};
    // A fork's working copy is installed by the author service, a pulse or two after the fork.
    let deadline = Instant::now() + Duration::from_secs(120);
    while world
        .rig
        .tw
        .world()
        .module_state::<mush_roll::RollStore>()
        .is_none_or(|store| store.mind(who, name).is_err())
    {
        assert!(
            Instant::now() < deadline,
            "{name} is not in {who}'s library: {:?}",
            world.rows("roll_mind_draft_parent")
        );
        world.pulse();
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    for line in edits {
        let _ = says(world, who, line);
        world.pulse();
    }
    let _ = says(world, who, &format!("@mind/save {name}=first"));
    let saved = |world: &M8World| -> Option<mush_roll::Revision> {
        let store = world
            .rig
            .tw
            .world()
            .module_state::<mush_roll::RollStore>()?;
        let mind = store.mind(who, name).ok()?;
        mind.latest()
            .and_then(|reference| store.revision(who, reference).ok())
            .cloned()
    };
    let deadline = Instant::now() + Duration::from_secs(300);
    let revision = loop {
        world.pulse();
        if let Some(revision) = saved(world) {
            break revision;
        }
        assert!(Instant::now() < deadline, "{name} was never saved");
        tokio::time::sleep(Duration::from_millis(2)).await;
    };
    // The Registry releases an earlier job a pulse after its receipt: type until it takes this.
    let deadline = Instant::now() + Duration::from_secs(300);
    loop {
        let heard = says(world, who, &format!("@mind/rate {name}@1"));
        if heard.contains("Registry preparation queued") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the Registry never took {name}: {heard}"
        );
        world.pulse();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let identity =
        mush_roll::commerce::RevisionIdentity::capture(&revision).expect("the revision identity");
    let started = Instant::now();
    let deadline = started + Duration::from_secs(7_200);
    loop {
        world.pulse();
        let done = world.rows("rating_receipts").into_iter().any(|wire| {
            serde_json::from_value::<RatingReceipt>(wire)
                .is_ok_and(|receipt| receipt.subject().revision() == identity)
        });
        if done {
            break;
        }
        for job in world
            .rows("rating_jobs")
            .into_iter()
            .filter_map(|row| serde_json::from_value::<RatingJob>(row).ok())
            .filter(|job| job.subject().revision() == identity)
        {
            match job.state() {
                RatingJobState::Queued {}
                | RatingJobState::Running { .. }
                | RatingJobState::Completed { .. } => {}
                other => panic!("the Registry ended {name}'s rating without a receipt: {other:?}"),
            }
        }
        assert!(
            Instant::now() < deadline,
            "the Registry did not rate {name}"
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    eprintln!("C4S rated {name} in {} s", started.elapsed().as_secs());
    let before = world.rows("roll_mind_initial_filing");
    let deadline = Instant::now() + Duration::from_secs(300);
    loop {
        let (heard, staged) = acts(world, who, &format!("@mind/file {name}@1"));
        if heard.contains("Filing ") && !staged.is_empty() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the filing never landed: {heard}"
        );
        world.pulse();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let filings: Vec<_> = world
        .rows("roll_mind_initial_filing")
        .into_iter()
        .filter(|row| !before.contains(row))
        .collect();
    let [filing] = filings.as_slice() else {
        panic!("one new filing: {filings:?}")
    };
    assert_eq!(
        filing["requester"],
        serde_json::json!(who.0),
        "the filing is its author's: {filing}"
    );
    (number(filing, "filed"), number(filing, "licence"))
}

/// **`who` asks for the source of the filed mind under `licence`** on their typed client
/// (`MindRequest::FiledSource`, the surface the web and TUI clients read source through), on
/// their own connection: the source's SHA-256, or the refusal.
pub(crate) fn filed_source(
    world: &mut M8World,
    who: ObjId,
    licence: u64,
) -> Result<String, String> {
    use mush_proto::{InvokeRequest, InvokeResult, MindRequest, MindResponse, OutputEvent};
    let conn = (1..=64)
        .find(|conn| world.rig.tw.engine().actor_of(*conn) == Some(who))
        .unwrap_or_else(|| panic!("{who} holds a connection"));
    let mut out = mush_core::Outbox::new();
    world.rig.tw.engine_mut().step(
        mush_core::Inbound::Invoke {
            conn,
            actor: who,
            id: 4_207,
            request: InvokeRequest::Mind(MindRequest::FiledSource {
                licence: licence.to_string(),
                first: 1,
                last: 200,
            }),
        },
        &mut out,
        &mut mush_core::Effects::new(),
    );
    let answers: Vec<MindResponse> = out
        .take()
        .into_iter()
        .filter_map(|(_, event)| match event {
            OutputEvent::InvokeResult {
                result: InvokeResult::Mind(response),
                ..
            } => Some(response),
            _ => None,
        })
        .collect();
    let [answer] = answers.as_slice() else {
        panic!("one answer to the source read: {answers:?}")
    };
    match answer {
        MindResponse::FiledSource { source_sha256, .. } => Ok(source_sha256.clone()),
        other => Err(format!("{other:?}")),
    }
}
