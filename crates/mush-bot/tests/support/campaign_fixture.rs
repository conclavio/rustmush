//! Deliberately constrained owned world. It uses core objects and the public SSH
//! transport, with no engine modules, jobs, scheduler, RNG or external ingress.
use mush_bot::{
    Result,
    campaign::{ADAPTER, CAPABILITIES, Endpoint, FixtureStart, WorldSave},
    experiment::{load_json, save_json},
};
use mush_core::{GOD, Obj, ObjId, ObjKind, World};
use mush_net_ssh::{AuthResult, CharSummary, CreateErr, NewConn, SshConfig, SshHooks, SshServer};
use mush_proto::{OutputEvent, RichText};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    net::IpAddr,
    path::PathBuf,
    pin::Pin,
    sync::Arc,
};
use tokio::sync::{Mutex, mpsc};
type FutureBox<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Data {
    objects: Vec<Obj>,
    next_id: u32,
    players: BTreeMap<String, ObjId>,
    studio: ObjId,
    lounge: ObjId,
}
struct Connection {
    actor: String,
    output: mpsc::Sender<Arc<OutputEvent>>,
}
struct State {
    world: World,
    players: BTreeMap<String, ObjId>,
    studio: ObjId,
    lounge: ObjId,
    connections: BTreeMap<u64, Connection>,
    fenced: BTreeSet<String>,
    frozen: bool,
    invalid: bool,
    sequence: u64,
}
struct Fixture {
    state: Mutex<State>,
    start: FixtureStart,
    directory: PathBuf,
}
impl Fixture {
    async fn output(state: &mut State, ids: Vec<u64>, text: String) -> bool {
        let event = Arc::new(OutputEvent::Sys(RichText::plain(text)));
        for id in ids {
            if let Some(conn) = state.connections.get(&id)
                && conn.output.send(event.clone()).await.is_err()
            {
                return false;
            }
        }
        true
    }
    fn look(state: &State, actor: &str) -> String {
        let room = state
            .world
            .get(state.players[actor])
            .unwrap()
            .location
            .unwrap();
        let name = &state.world.get(room).unwrap().name;
        let contents = state
            .players
            .iter()
            .filter(|(_, id)| state.world.get(**id).unwrap().location == Some(room))
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>()
            .join("  ");
        let exits = if room == state.studio {
            "north (Lounge)  east (Lounge side door)"
        } else {
            "south (Rehearsal Room)  west (Rehearsal Room side door)"
        };
        format!(
            "{name}(#{})\nA rehearsal space for a band finding its voice.\nContents:\n{contents}\nObvious exits:\n{exits}",
            room.0
        )
    }
    async fn line_inner(&self, conn: u64, line: String) -> Result<()> {
        let mut state = self.state.lock().await;
        let actor = state
            .connections
            .get(&conn)
            .ok_or("unknown fixture connection")?
            .actor
            .clone();
        if line == format!("__campaign_fence {}", self.start.epoch)
            && !state.fenced.contains(&actor)
        {
            if state.invalid {
                return Err("fixture state invalidated before checkpoint".into());
            }
            state.fenced.insert(actor);
            if state.fenced.len() == self.start.actors.len() {
                state.frozen = true;
                let data = Data {
                    objects: state.world.iter().cloned().collect(),
                    next_id: state.world.next_id(),
                    players: state.players.clone(),
                    studio: state.studio,
                    lounge: state.lounge,
                };
                let save = WorldSave {
                    version: 1,
                    adapter: ADAPTER.into(),
                    capabilities: CAPABILITIES.into(),
                    epoch: self.start.epoch.clone(),
                    actors: self.start.actors.clone(),
                    frozen: true,
                    command_sequence: state.sequence,
                    logical_clock: state.world.now(),
                    data: serde_json::to_value(data)?,
                };
                save.validate()?;
                save_json(&self.directory.join("world-checkpoint.json"), &save)?;
                let ids = state.connections.keys().copied().collect();
                if !Self::output(
                    &mut state,
                    ids,
                    format!("CAMPAIGN FROZEN {}", self.start.epoch),
                )
                .await
                {
                    return Err("frozen output delivery failed".into());
                }
            }
            return Ok(());
        }
        if state.frozen || state.fenced.contains(&actor) {
            return Err("ingress after actor fence invalidates checkpoint".into());
        }
        state.sequence = state.sequence.checked_add(1).ok_or("sequence exhausted")?;
        let id = state.players[&actor];
        let location = state.world.get(id).unwrap().location;
        let (verb, arg) = line.trim().split_once(' ').unwrap_or((line.trim(), ""));
        let mut recipients = vec![conn];
        let text = match verb {
            "look" | "l" => Self::look(&state, &actor),
            "help" => "Owned rehearsal fixture: look, say <text>, pose <action>, page <actor>=<text>, north, south, time. No NPCs, jobs, RNG, builders or autonomous clock. This is a constrained test world.".into(),
            "time" => format!("Game clock: {} (fixed)", state.world.now()),
            "north" | "south" | "east" | "west" => {
                if verb=="north" && state.world.get(state.studio).unwrap().attrs.get("blocked").is_some() {
                    "The exit is blocked.".into()
                } else if (matches!(verb,"north"|"east") && location == Some(state.studio)) || (matches!(verb,"south"|"west") && location == Some(state.lounge)) {
                    let destination = if matches!(verb,"north"|"east") { state.lounge } else { state.studio };
                    state.world.move_to(id, destination)?;
                    Self::look(&state, &actor)
                } else { "There is no exit that way.".into() }
            }
            "close-door" => {
                let studio=state.studio;
                state.world.set_attr(studio,"blocked",mush_core::Attr::text("true",GOD))?;
                "The north door is now blocked.".into()
            }
            "walk-dogs" if location==Some(state.lounge) => {
                state.world.set_attr(id,"dog_walk_receipt",mush_core::Attr::text("returned",GOD))?;
                "Work receipt: the dogs have returned from their walk.".into()
            }
            "study" if location==Some(state.studio) => {
                state.world.set_attr(id,"study_receipt",mush_core::Attr::text("exercise",GOD))?;
                "Study receipt: the exercise has been submitted.".into()
            }
            "say" | "pose" => {
                recipients = state.connections.iter().filter(|(_, c)| state.world.get(state.players[&c.actor]).unwrap().location == location).map(|(id,_)| *id).collect();
                if verb == "say" { format!("{actor} says, \"{arg}\"") } else { format!("{actor} {arg}") }
            }
            "page" => {
                if let Some((target, speech)) = arg.split_once('=') {
                    if let Some((target_conn, _)) = state.connections.iter().find(|(_,c)| c.actor.eq_ignore_ascii_case(target)) {
                        recipients.push(*target_conn); recipients.sort(); recipients.dedup();
                        format!("{actor} pages: {speech}")
                    } else { "That player is not connected.".into() }
                } else { "Usage: page <actor>=<text>".into() }
            }
            _ => "Unsupported rehearsal-fixture command. Type help.".into(),
        };
        if !Self::output(&mut state, recipients, text).await {
            return Err("fixture output failed".into());
        }
        Ok(())
    }
}
impl SshHooks for Fixture {
    fn auth_account<'a>(
        &'a self,
        account: &'a str,
        password: &'a str,
    ) -> FutureBox<'a, AuthResult> {
        Box::pin(async move {
            let state = self.state.lock().await;
            if password == "local-fixture-only"
                && let Some((name, id)) = state
                    .players
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case(account))
            {
                AuthResult::Ok {
                    account: name.clone(),
                    characters: vec![CharSummary {
                        key: u64::from(id.0),
                        name: name.clone(),
                    }],
                }
            } else {
                AuthResult::BadCreds
            }
        })
    }
    fn create_account<'a>(
        &'a self,
        _: &'a str,
        _: &'a str,
    ) -> FutureBox<'a, std::result::Result<(), CreateErr>> {
        Box::pin(async { Err(CreateErr::BadName("Fixture cast is fixed".into())) })
    }
    fn connect_character<'a>(
        &'a self,
        conn: NewConn,
        account: &'a str,
        character: u64,
    ) -> FutureBox<'a, std::result::Result<(), String>> {
        Box::pin(async move {
            let mut state = self.state.lock().await;
            if state.frozen
                || state.invalid
                || state
                    .players
                    .get(account)
                    .is_none_or(|id| u64::from(id.0) != character)
                || state.connections.values().any(|c| c.actor == account)
            {
                return Err("Fixture membership is fixed and exclusive".into());
            }
            let id = conn.id;
            state.connections.insert(
                id,
                Connection {
                    actor: account.into(),
                    output: conn.tx,
                },
            );
            let text = Self::look(&state, account);
            if !Self::output(&mut state, vec![id], text).await {
                return Err("Fixture login output failed".into());
            }
            Ok(())
        })
    }
    fn line(&self, conn: u64, line: String) -> FutureBox<'_, ()> {
        Box::pin(async move {
            if let Err(error) = self.line_inner(conn, line).await {
                eprintln!("fixture invalid: {error}");
                let mut state = self.state.lock().await;
                state.invalid = true;
                let ids = state.connections.keys().copied().collect();
                Self::output(&mut state, ids, "CAMPAIGN INVALID".into()).await;
            }
        })
    }
    fn disconnected(&self, conn: u64) -> FutureBox<'_, ()> {
        Box::pin(async move {
            let mut state = self.state.lock().await;
            if state.connections.remove(&conn).is_some() && !state.frozen {
                state.invalid = true;
            }
        })
    }
    fn is_ip_blocked(&self, _: IpAddr) -> bool {
        false
    }
    fn auth_failed(&self, _: IpAddr) {}
}

pub async fn serve(path: PathBuf) -> Result<()> {
    let start: FixtureStart = load_json(&path, 8_100_000)?;
    if start.version != 1
        || !mush_bot::organization::identifier(&start.epoch)
        || start.actors.is_empty()
        || start.actors.len() > 16
        || start
            .actors
            .iter()
            .any(|a| !mush_bot::organization::identifier(a))
        || start
            .actors
            .iter()
            .map(|a| a.to_lowercase())
            .collect::<BTreeSet<_>>()
            .len()
            != start.actors.len()
    {
        return Err("invalid fixture startup membership/epoch".into());
    }
    let directory = path
        .parent()
        .ok_or("fixture path needs parent")?
        .to_path_buf();
    let (world, players, studio, lounge, sequence) = if let Some(save) = &start.restore {
        save.validate()?;
        if save.actors != start.actors {
            return Err("fixture restore membership mismatch".into());
        }
        let data: Data = serde_json::from_value(save.data.clone())?;
        let mut world = World::from_objects(data.objects, data.next_id)?;
        world.set_now(save.logical_clock);
        if data.players.keys().cloned().collect::<Vec<_>>() != start.actors
            || world
                .get(data.studio)
                .is_none_or(|o| o.kind != ObjKind::Room)
            || world
                .get(data.lounge)
                .is_none_or(|o| o.kind != ObjKind::Room)
            || data.players.iter().any(|(name, id)| {
                world.get(*id).is_none_or(|o| {
                    o.kind != ObjKind::Player
                        || o.name.as_str() != name
                        || ![Some(data.studio), Some(data.lounge)].contains(&o.location)
                })
            })
        {
            return Err("fixture world schema does not match its declared capabilities".into());
        }
        (
            world,
            data.players,
            data.studio,
            data.lounge,
            save.command_sequence,
        )
    } else {
        let mut world = World::new_minimal();
        world.set_now(1_700_000_000);
        let studio = world.create(ObjKind::Room, "Rehearsal Room", GOD, None)?;
        let lounge = world.create(ObjKind::Room, "Lounge", GOD, None)?;
        let mut players = BTreeMap::new();
        for name in &start.actors {
            players.insert(
                name.clone(),
                world.create(ObjKind::Player, name, GOD, Some(studio))?,
            );
        }
        (world, players, studio, lounge, 0)
    };
    let fixture = Arc::new(Fixture {
        state: Mutex::new(State {
            world,
            players,
            studio,
            lounge,
            sequence,
            connections: BTreeMap::new(),
            fenced: BTreeSet::new(),
            frozen: false,
            invalid: false,
        }),
        start,
        directory: directory.clone(),
    });
    let host_key_path = directory.join("host-key");
    let server = SshServer::bind(
        SshConfig {
            listen: "127.0.0.1:0".parse()?,
            host_key_path: host_key_path.clone(),
            banner: "Owned campaign rehearsal fixture".into(),
        },
        fixture,
    )
    .await?;
    let key = russh::keys::load_secret_key(&host_key_path, None)?;
    let endpoint = Endpoint {
        version: 1,
        adapter: ADAPTER.into(),
        capabilities: CAPABILITIES.into(),
        port: server.local_addr().port(),
        host_key_sha256: key
            .public_key()
            .fingerprint(russh::keys::ssh_key::HashAlg::Sha256)
            .to_string(),
    };
    // Publish readiness atomically: the coordinator never reads a partially written endpoint.
    save_json(&directory.join("endpoint.pending.json"), &endpoint)?;
    std::fs::rename(
        directory.join("endpoint.pending.json"),
        directory.join("endpoint.json"),
    )?;
    server.run().await?;
    Ok(())
}
