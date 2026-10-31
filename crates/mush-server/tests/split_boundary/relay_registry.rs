//! R1's production boot, content and persistence boundary. Live Redis/SSH evidence
//! is recorded separately in doc 15b; these tests require no external service.
use mush_config::config::Config;
use mush_core::{CanonicalId, GOD, LIMBO, ModuleOp, ModuleWrite};
use mush_server::{boot, persist, quarantine::QuarantineSpool};
use mush_sim::relay::{
    KEY, LinkState, MAX_COVERAGE, MAX_NODES, MAX_PAIRS, RelayRegistry, RelayStore,
};
use mush_store::{InMemoryStore, ObjectStore, WriteBatch};
use mush_testkit::TestWorld;
use std::{path::Path, sync::Arc};

struct Rig {
    cfg: Config,
    store: Arc<dyn ObjectStore>,
    spool: QuarantineSpool,
    tw: TestWorld,
    serial: u64,
}
impl Rig {
    fn channels(&self) -> &mush_comsys::ComsysState {
        self.tw.world().module_state().unwrap()
    }
    fn channel_setup(&mut self, people: &[mush_core::ObjId]) -> mush_comsys::ChannelId {
        self.run(GOD, "@channel/create Radio");
        for flag in ["ic", "logged", "persist", "loud", "relay"] {
            self.run(GOD, &format!("@channel/set Radio={flag}"));
        }
        for &p in people {
            assert!(self.run(p, "addcom radio=Radio").contains("added"));
        }
        self.channels().channel_by_name("Radio").unwrap().id
    }
    fn backlog(&self, actor: mush_core::ObjId) -> String {
        use mush_core::QueryHandler;
        serde_json::to_string(
            &mush_comsys::BacklogQuery
                .query(
                    self.tw.world(),
                    actor,
                    &serde_json::json!({"channel":"Radio","limit":200}),
                )
                .unwrap(),
        )
        .unwrap()
    }
    fn finish_channel(&mut self, channel: mush_comsys::ChannelId, seq: u64) {
        let prefix = format!("channel:v1:{}:{seq}:", channel.0);
        for _ in 0..100 {
            if !self.channels().ic.pending.contains(&(channel, seq)) {
                return;
            }
            self.mail_tick();
            let next = self
                .tw
                .world()
                .module_state::<mush_propagation::PropagationStore>()
                .unwrap()
                .envelopes()
                .filter(|e| e.dispatch_ref().is_some_and(|r| r.key.starts_with(&prefix)))
                .filter_map(|e| e.transit()?.flight.as_ref().map(|f| f.due.at))
                .min();
            if let Some(at) = next
                && at > self.tw.world().world_ic_now().at
            {
                self.advance_ic_to(at);
            }
        }
        panic!("channel line {seq} did not settle");
    }
}

#[tokio::test]
async fn r6b_channel_holes_offline_restart_and_shared_mirror_are_arrival_gated() {
    use mush_core::QueryHandler;
    let mut rig = Rig::new(true).await;
    let bob = rig.endpoints();
    let far = rig.tw.spawn_player("FarRadio");
    let phobos = rig
        .tw
        .world()
        .iter()
        .find(|o| o.name == "Townships Receiving")
        .unwrap()
        .id;
    rig.run(GOD, &format!("@teleport {far}={phobos}"));
    let chan = rig.channel_setup(&[GOD, bob, far]);
    rig.public_hub(false);
    let events = rig.tw.run(GOD, "radio first-secret RelayReceiver FarRadio");
    assert!(
        !events
            .iter()
            .any(|(a, _)| matches!(a, mush_core::Audience::Relay(_)))
    );
    rig.mail_tick();
    assert!(!rig.backlog(bob).contains("first-secret"));
    assert!(
        !rig.run(bob, "@channel/recall radio")
            .contains("first-secret")
    );
    let state = mush_comsys::StateQuery
        .query(rig.tw.world(), bob, &serde_json::json!({}))
        .unwrap();
    let json = serde_json::to_value(state).unwrap();
    assert!(!json.to_string().contains("first-secret"));
    assert_eq!(
        rig.channels().members[&(bob, chan)]
            .ic
            .as_ref()
            .unwrap()
            .sequence,
        0
    );
    rig.flush().await;
    rig.restart().await.unwrap();
    assert!(!rig.backlog(bob).contains("first-secret"));
    rig.public_hub(true);
    assert!(
        rig.run(GOD, "radio second-fast")
            .contains("queued for IC transit")
    );
    // Drive the fast covered copy alone; the uncovered recipient's physical
    // flight must not force this test to advance over the first letter's due.
    for _ in 0..20 {
        rig.mail_tick();
        if rig.backlog(bob).contains("second-fast") {
            break;
        }
    }
    assert!(rig.backlog(bob).contains("second-fast"));
    assert!(!rig.backlog(bob).contains("first-secret"));
    assert!(!rig.backlog(far).contains("second-fast"));
    let k = rig.channels().members[&(bob, chan)].ic.as_ref().unwrap();
    assert_eq!(k.sequence, 1);
    assert_eq!(k.received[0].message, 2);
    assert_eq!(k.received[0].seq, 1);
    rig.flush().await;
    rig.restart().await.unwrap();
    assert!(!rig.backlog(bob).contains("first-secret"));
    rig.finish_channel(chan, 1);
    rig.finish_channel(chan, 2);
    let k = rig.channels().members[&(bob, chan)].ic.as_ref().unwrap();
    assert_eq!(k.sequence, 2);
    assert_eq!(k.received[1].message, 1);
    assert_eq!(k.received[1].seq, 2);
    assert_eq!(
        rig.channels().members[&(bob, chan)].last_read,
        0,
        "offline arrivals remain unread"
    );
    assert!(rig.backlog(far).contains("first-secret"));
    assert!(rig.backlog(far).contains("second-fast"));
    let notice = rig.tw.connect(bob);
    assert!(
        rig.tw
            .seen_by(&notice, bob)
            .join(" ")
            .contains("2 messages since")
    );
    rig.flush().await;
    rig.restart().await.unwrap();
    assert_eq!(
        rig.channels().members[&(bob, chan)]
            .ic
            .as_ref()
            .unwrap()
            .sequence,
        2
    );
    assert!(
        rig.run(GOD, "@channel/set Radio=!ic")
            .contains("cannot change")
    );
    let events = rig.tw.run(GOD, "pub OOC still immediate");
    assert!(events.iter().any(|(_,e)|matches!(e,mush_proto::OutputEvent::Channel{text,..} if text.to_plain_text()=="OOC still immediate")));
}

#[tokio::test]
async fn r6b_membership_revocation_and_source_loss_preserve_transport_boundaries() {
    let mut rig = Rig::new(true).await;
    let bob = rig.endpoints();
    let alice = rig.tw.spawn_player("RadioAlice");
    let luna = rig.registry().nodes[&cid("malapert")].coverage[0];
    rig.run(GOD, &format!("@teleport {alice}={luna}"));
    let chan = rig.channel_setup(&[alice, bob]);
    rig.public_hub(false);
    rig.run(alice, "radio before-rejoin");
    rig.mail_tick();
    rig.run(bob, "delcom radio");
    rig.run(bob, "addcom radio=Radio");
    rig.flush().await;
    rig.restart().await.unwrap();
    rig.finish_channel(chan, 1);
    assert!(!rig.backlog(bob).contains("before-rejoin"));
    rig.run(alice, "radio revoked-in-flight");
    rig.mail_tick();
    rig.run(GOD, "@channel/perm Radio=see=#FALSE");
    rig.finish_channel(chan, 2);
    rig.run(GOD, "@channel/perm Radio=see=#TRUE");
    assert!(!rig.backlog(bob).contains("revoked-in-flight"));
    rig.run(alice, "radio source-lost-after-send");
    rig.mail_tick();
    // Controlled lifecycle fault after source acceptance. The frozen content
    // and physical origin must survive loss of the source object.
    rig.tw
        .world_mut()
        .set_flag(alice, mush_core::Flags::GOING, true)
        .unwrap();
    rig.flush().await;
    rig.restart().await.unwrap();
    rig.finish_channel(chan, 3);
    assert!(rig.backlog(bob).contains("source-lost-after-send"));
    assert_eq!(
        rig.channels().members[&(bob, chan)]
            .ic
            .as_ref()
            .unwrap()
            .sequence,
        1
    );
    assert!(rig.channels().ic.progress.visited <= mush_comsys::transit::MAX_COPIES);
}

#[tokio::test]
async fn r6b_identified_relay_retries_survive_restart_without_duplicate_fanout() {
    let mut rig = Rig::new(true).await;
    let bob = rig.endpoints();
    let chan = rig.channel_setup(&[GOD, bob]);
    let mut out = mush_core::Outbox::new();
    assert_eq!(
        rig.tw.engine_mut().relay_speak(
            GOD,
            "Radio",
            mush_proto::NarrativeKind::Say,
            "provider-private",
            &mut out
        ),
        Err(mush_core::RelayRefusal::RetryIdentity)
    );
    // The server's actual module entry point, with a stable provider ID.
    rig.tw
        .engine_mut()
        .relay_speak_identified(
            GOD,
            "Radio",
            mush_proto::NarrativeKind::Say,
            "provider-private",
            Some(400),
            &mut out,
        )
        .unwrap();
    // Engine direct service calls defer their module writes to the same
    // per-step drain as the shipping world loop.
    rig.tw
        .engine_mut()
        .finish_external_step(&mut out, &mut mush_core::Effects::new());
    assert!(
        !out.events()
            .iter()
            .any(|(a, _)| matches!(a, mush_core::Audience::Relay(_)))
    );
    rig.flush().await;
    rig.restart().await.unwrap();
    rig.tw
        .engine_mut()
        .relay_speak_identified(
            GOD,
            "Radio",
            mush_proto::NarrativeKind::Say,
            "provider-private",
            Some(400),
            &mut out,
        )
        .unwrap();
    assert_eq!(rig.channels().channels[&chan].seq, 1);
    assert_eq!(
        rig.tw.engine_mut().relay_speak_identified(
            GOD,
            "Radio",
            mush_proto::NarrativeKind::Say,
            "changed",
            Some(400),
            &mut out
        ),
        Err(mush_core::RelayRefusal::RetryIdentity)
    );
    rig.finish_channel(chan, 1);
    assert_eq!(rig.backlog(bob).matches("provider-private").count(), 1);
}
impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.cfg.server.data_dir);
    }
}
impl Rig {
    async fn set_mail_quota(&mut self, quota: usize) {
        // Controlled configuration fault through the owner's persisted schema;
        // do not write another module's live state slot from the server fixture.
        self.flush().await;
        self.store
            .write(WriteBatch {
                serial: self.serial,
                durable: true,
                module_ops: vec![ModuleOp {
                    ns: "mail".into(),
                    op: ModuleWrite::JsonSet {
                        key: "config".into(),
                        value: serde_json::json!({"quota":quota}),
                    },
                }],
                ..Default::default()
            })
            .await
            .unwrap();
        self.serial += 1;
        self.restart().await.unwrap();
    }
    fn mail(&self) -> &mush_mail::MailState {
        self.tw.world().module_state().unwrap()
    }
    fn mail_tick(&mut self) -> Vec<(mush_core::Audience, mush_proto::OutputEvent)> {
        let due = self.tw.engine_mut().sched.next_due().unwrap_or(0);
        self.tw
            .set_now((self.tw.world().now() + 1).max(due.div_euclid(1000) + 1));
        self.tw.fire_due(self.tw.world().now() * 1000)
    }
    fn mail_route(
        &self,
        id: u64,
        recipient: mush_core::ObjId,
        returning: bool,
    ) -> Option<mush_propagation::Envelope> {
        mush_propagation::dispatch::find(
            self.tw.world(),
            &format!(
                "mail:v1:{id}:{}:{}",
                recipient.0,
                if returning { "return" } else { "out" }
            ),
        )
        .cloned()
    }
    fn mail_hidden(&mut self, actor: mush_core::ObjId, secret: &str) {
        use mush_core::QueryHandler;
        let panel = mush_mail::ListQuery
            .query(self.tw.world(), actor, &serde_json::json!({}))
            .unwrap();
        assert!(!serde_json::to_string(&panel).unwrap().contains(secret));
        assert!(!self.run(actor, "@mail").contains(secret));
        assert!(!self.run(actor, "@mail 1").contains(secret));
        assert!(!self.run(actor, "+orders").contains("mail:v1"));
        assert_eq!(self.mail().unread_counts(actor).0, 0);
    }
    fn finish_mail(&mut self, id: u64) {
        // Advance only to actual frozen flight deadlines; never manufacture arrival.
        for _ in 0..80 {
            if mush_mail::transit::intents(self.tw.world())
                .unwrap()
                .iter()
                .all(|r| !r.key.starts_with(&format!("mail:v1:{id}:")))
            {
                return;
            }
            self.mail_tick();
            let next = self
                .tw
                .world()
                .module_state::<mush_propagation::PropagationStore>()
                .unwrap()
                .envelopes()
                .filter(|e| {
                    e.dispatch_ref()
                        .is_some_and(|r| r.key.starts_with(&format!("mail:v1:{id}:")))
                })
                .filter_map(|e| e.transit()?.flight.as_ref().map(|f| f.due.at))
                .min();
            if let Some(at) = next
                && at > self.tw.world().world_ic_now().at
            {
                self.advance_ic_to(at);
            }
        }
        panic!("mail {id} did not settle");
    }
    async fn new(enabled: bool) -> Self {
        static NEXT_DIR: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let mut cfg = Config::default();
        cfg.store.store = "memory".into();
        cfg.server.data_dir = std::env::temp_dir().join(format!(
            "mush-r1-{}-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        cfg.server.help_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/help");
        cfg.tacsim.enabled = enabled;
        cfg.tacsim.geology.key_dir = Some(cfg.server.data_dir.join("geology-keys"));
        let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
        let spool = QuarantineSpool::open(cfg.server.data_dir.join("quarantine")).unwrap();
        let engine = boot::build_engine(&cfg, &store, &spool)
            .await
            .unwrap()
            .engine;
        assert!(
            engine.world.refused_namespace_writes().is_empty(),
            "{:?}",
            engine.world.refused_namespace_writes()
        );
        if enabled {
            assert!(store.module_json_get("sim", KEY).await.unwrap().is_some());
        }
        let mut rig = Self {
            cfg,
            store,
            spool,
            tw: TestWorld::from_engine(engine),
            serial: 1,
        };
        rig.flush().await;
        rig
    }
    fn registry(&self) -> RelayRegistry {
        self.tw
            .world()
            .module_state::<RelayStore>()
            .unwrap()
            .registry()
            .unwrap()
            .clone()
    }
    fn run(&mut self, actor: mush_core::ObjId, command: &str) -> String {
        let events = self.tw.run(actor, command);
        self.tw.seen_by(&events, actor).join("\n")
    }
    async fn flush(&mut self) {
        let dirty = self.tw.world_mut().take_dirty();
        let mut batch = persist::batch_from_dirty(self.tw.world(), dirty, self.serial, true);
        batch.module_ops = self.tw.take_module_ops();
        self.store.write(batch).await.unwrap();
        self.serial += 1;
    }
    async fn flush_money(&mut self, quote: u64, phase: &str) {
        assert!(
            self.tw
                .engine_mut()
                .take_durable_gate()
                .is_some_and(|(_, strict)| strict)
        );
        let dirty = self.tw.world_mut().take_dirty();
        let mut batch = persist::batch_from_dirty(self.tw.world(), dirty, self.serial, true);
        batch.module_ops = self.tw.take_module_ops();
        assert!(batch.module_ops.iter().any(|o| o.ns == "economy"));
        assert!(batch.module_ops.iter().any(|o| o.ns == "propagation" && matches!(&o.op,
            ModuleWrite::JsonSet { key, value } if key == "relay_transport" && value["service"]["commerce"]["purchases"][quote.to_string()]["phase"] == phase)));
        self.store.write(batch).await.unwrap();
        self.serial += 1;
    }
    async fn restart(&mut self) -> Result<(), String> {
        let previous_now = self.tw.world().now();
        let engine = boot::build_engine(&self.cfg, &self.store, &self.spool)
            .await
            .map_err(|e| e.to_string())?
            .engine;
        self.tw = TestWorld::from_engine(engine);
        self.tw.set_now(previous_now);
        Ok(())
    }
    async fn replace_document(&mut self, value: serde_json::Value) {
        self.store
            .write(WriteBatch {
                serial: self.serial,
                durable: true,
                module_ops: vec![ModuleOp {
                    ns: "sim".into(),
                    op: ModuleWrite::JsonSet {
                        key: KEY.into(),
                        value,
                    },
                }],
                ..WriteBatch::default()
            })
            .await
            .unwrap();
        self.serial += 1;
    }
}

impl Rig {
    fn envelope(&self, id: u64) -> mush_propagation::Envelope {
        self.tw
            .world()
            .module_state::<mush_propagation::PropagationStore>()
            .unwrap()
            .envelope(id)
            .unwrap()
            .clone()
    }

    fn last_envelope(&self) -> mush_propagation::Envelope {
        self.tw
            .world()
            .module_state::<mush_propagation::PropagationStore>()
            .unwrap()
            .envelopes()
            .last()
            .unwrap()
            .clone()
    }

    fn endpoints(&mut self) -> mush_core::ObjId {
        let receiver = self.tw.spawn_player("RelayReceiver");
        let catalog = self.registry();
        let luna = catalog.nodes[&cid("malapert")].coverage[0];
        let ceres = catalog.nodes[&cid("ceres")].coverage[0];
        assert!(
            !self
                .run(GOD, &format!("@teleport me={luna}"))
                .contains("cannot")
        );
        assert!(
            !self
                .run(GOD, &format!("@teleport {receiver}={ceres}"))
                .contains("cannot")
        );
        // Earlier physical-route stories isolate class delay with explicit
        // no-charge operator grants. R3 stories select their own classes.
        for actor in [GOD, receiver] {
            for pair in ["luna_ceres", "luna_phobos", "luna_kirkwood", "luna_mercury"] {
                if catalog.pairs.contains_key(&cid(pair)) {
                    assert!(
                        self.run(GOD, &format!("+relay/grant {pair}/{actor}=priority"))
                            .contains("Class grant recorded")
                    );
                }
            }
        }
        receiver
    }

    fn pass(&mut self) {
        self.run(GOD, "@prop/pass");
    }

    fn advance_ic_to(&mut self, at: i64) {
        assert_eq!(self.tw.world().world_timeline().current().rate, 1);
        let delta = at - self.tw.world().world_ic_now().at;
        assert!(delta >= 0);
        self.tw.set_now(self.tw.world().now() + delta);
    }
}

#[tokio::test]
async fn r2_pair_delivery_and_ack_are_durable_and_arrival_gated() {
    use mush_propagation::EnvelopeState;
    let mut rig = Rig::new(true).await;
    let receiver = rig.endpoints();
    let reply = rig.run(GOD, &format!("+order {receiver}=R2-private-order"));
    assert!(reply.contains("Queued revision"), "{reply}");
    let issued = rig.last_envelope();
    assert_eq!(issued.transit().unwrap().legs.len(), 3);
    assert!(!rig.run(receiver, "+orders").contains("R2-private-order"));
    assert!(
        !rig.run(receiver, &format!("@prop/envelopes {receiver}"))
            .contains("R2-private-order")
    );
    assert!(
        rig.run(receiver, &format!("@prop/route {}", issued.id()))
            .contains("Permission denied")
    );
    rig.pass();
    assert!(
        rig.envelope(issued.id())
            .transit()
            .unwrap()
            .queued_at
            .is_some()
    );
    rig.flush().await;
    let queued = rig.envelope(issued.id());
    rig.restart().await.unwrap();
    assert_eq!(rig.envelope(issued.id()), queued);
    rig.pass();
    let delivered = rig.envelope(issued.id());
    assert_eq!(delivered.state(), EnvelopeState::Delivered);
    assert_eq!(delivered.path().len(), 1);
    assert_eq!(delivered.transit().unwrap().history.len(), 2);
    assert!(
        rig.run(receiver, "+orders")
            .contains("Standing order: revision 1")
    );
    assert!(!rig.run(GOD, "+orders").contains("Delivered"));
    for _ in 0..3 {
        rig.pass();
    }
    assert_eq!(
        rig.envelope(issued.id()),
        delivered,
        "arrival must not run again"
    );
    assert!(
        rig.run(receiver, &format!("+orders/ack {}", issued.id()))
            .contains("Acknowledgement queued")
    );
    let ack = rig.last_envelope();
    rig.pass();
    assert_eq!(rig.envelope(issued.id()).state(), EnvelopeState::Delivered);
    rig.flush().await;
    rig.restart().await.unwrap();
    rig.pass();
    assert_eq!(
        rig.envelope(issued.id()).state(),
        EnvelopeState::Acknowledged
    );
    assert_eq!(rig.envelope(ack.id()).path().len(), 1);
    assert!(rig.run(GOD, "+orders").contains("Acknowledged"));
    rig.flush().await;
    rig.restart().await.unwrap();
    assert_eq!(
        rig.envelope(issued.id()).state(),
        EnvelopeState::Acknowledged
    );
}

#[tokio::test]
async fn r2_conventional_fallback_freezes_sky_lag_across_restart_and_link_repair() {
    use mush_propagation::EnvelopeState;
    let mut rig = Rig::new(true).await;
    let receiver = rig.endpoints();
    rig.public_hub(false);
    assert!(
        rig.run(GOD, &format!("+order {receiver}=R2-timed-order"))
            .contains("Queued")
    );
    let id = rig.last_envelope().id();
    rig.pass();
    let in_flight = rig.envelope(id);
    let flight = in_flight.transit().unwrap().flight.clone().unwrap();
    assert!(flight.seconds > 60, "astronomical lag, not abstract range");
    assert!(flight.basis.starts_with("sky.v1:"));
    rig.public_hub(true);
    rig.flush().await;
    rig.restart().await.unwrap();
    assert_eq!(rig.envelope(id), in_flight);
    rig.advance_ic_to(flight.due.at - 1);
    rig.pass();
    assert_eq!(rig.envelope(id).state(), EnvelopeState::Transmitted);
    assert!(!rig.run(receiver, "+orders").contains("R2-timed-order"));
    rig.advance_ic_to(flight.due.at);
    rig.pass();
    assert_eq!(rig.envelope(id).state(), EnvelopeState::Delivered);
    let after = rig.envelope(id);
    assert_eq!(after.transit().unwrap().history[0].flight, flight);
    assert!(
        after.path().is_empty(),
        "repair cannot replay an already transmitted leg through a relay"
    );
    assert!(rig.run(receiver, "+orders").contains("R2-timed-order"));
}

#[tokio::test]
async fn r2_fifo_bandwidth_survives_restart_and_rejects_full_link_queue() {
    use mush_propagation::EnvelopeState;
    let mut rig = Rig::new(true).await;
    let receiver = rig.endpoints();
    let mut ids = vec![];
    for _ in 0..3 {
        assert!(
            rig.run(GOD, &format!("+order {receiver}={}", "x".repeat(512)))
                .contains("Queued")
        );
        ids.push(rig.last_envelope().id());
    }
    rig.pass();
    rig.pass();
    assert_eq!(rig.envelope(ids[0]).path().len(), 1);
    assert_eq!(rig.envelope(ids[1]).path().len(), 1);
    assert_eq!(rig.envelope(ids[2]).path().len(), 0);
    assert_eq!(rig.envelope(ids[2]).state(), EnvelopeState::Transmitted);
    rig.flush().await;
    rig.restart().await.unwrap();
    rig.pass();
    assert_eq!(
        rig.envelope(ids[2]).path().len(),
        0,
        "same second cannot spend the link budget twice"
    );
    rig.advance_ic_to(rig.tw.world().world_ic_now().at + 1);
    rig.pass();
    assert_eq!(rig.envelope(ids[2]).state(), EnvelopeState::Delivered);
    let luna = rig.registry().nodes[&cid("malapert")].place;
    let other = rig.tw.spawn_player("OtherOperator");
    let extra = rig.tw.spawn_player("ExtraOperator");
    for who in [other, extra] {
        rig.tw
            .world_mut()
            .set_flag(who, mush_core::Flags::ROYALTY, true)
            .unwrap();
        rig.run(GOD, &format!("@teleport {who}={luna}"));
    }
    for i in 0..mush_propagation::routing::MAX_LINK_QUEUE {
        let who = if i < 32 { GOD } else { other };
        let reply = rig.run(who, &format!("+order {receiver}=pending-{i}"));
        assert!(reply.contains("Queued"), "{i}: {reply}");
    }
    assert!(
        rig.run(extra, &format!("+order {receiver}=overload"))
            .contains("link queue is full")
    );
    assert!(!rig.run(receiver, "+orders").contains("pending-"));
}

#[tokio::test]
async fn r2_moving_recipient_requires_another_physical_leg() {
    use mush_propagation::EnvelopeState;
    let mut rig = Rig::new(true).await;
    let receiver = rig.endpoints();
    rig.public_hub(false);
    rig.run(GOD, &format!("+order {receiver}=forward-this"));
    let id = rig.last_envelope().id();
    rig.pass();
    let first = rig.envelope(id).transit().unwrap().flight.clone().unwrap();
    let luna = rig.registry().nodes[&cid("malapert")].place;
    rig.run(GOD, &format!("@teleport {receiver}={luna}"));
    rig.advance_ic_to(first.due.at);
    rig.pass();
    let forwarded = rig.envelope(id);
    assert_eq!(forwarded.state(), EnvelopeState::Transmitted);
    let transit = forwarded.transit().unwrap();
    assert_eq!(transit.forwardings, 1);
    let due = transit.flight.as_ref().unwrap().due.at;
    assert!(due > first.due.at);
    assert!(!rig.run(receiver, "+orders").contains("forward-this"));
    rig.advance_ic_to(due);
    rig.pass();
    assert_eq!(rig.envelope(id).state(), EnvelopeState::Delivered);
}

#[tokio::test]
async fn r2_unknown_endpoints_wait_with_bounded_admission_and_expiry() {
    use mush_propagation::{EnvelopeState, record::ENVELOPE_TTL_SECS, routing::MAX_PENDING_ROUTES};
    let mut rig = Rig::new(true).await;
    let receiver = rig.endpoints();
    rig.run(GOD, &format!("@teleport {receiver}={LIMBO}"));
    let senders: Vec<_> = (0..4)
        .map(|i| rig.tw.spawn_player(&format!("QueueSender{i}")))
        .collect();
    for &sender in &senders {
        rig.tw
            .world_mut()
            .set_flag(sender, mush_core::Flags::ROYALTY, true)
            .unwrap();
    }
    for i in 0..MAX_PENDING_ROUTES - mush_propagation::admission::CONTROL_RESERVE {
        let sender = senders[i / 32];
        let reply = rig.run(sender, &format!("+order {receiver}=waiting-{i}"));
        assert!(reply.contains("Queued"), "{reply}");
    }
    assert!(
        rig.run(GOD, &format!("+order {receiver}=overflow"))
            .contains("reserved for acknowledgements")
    );
    let last = rig.last_envelope();
    rig.pass();
    rig.pass();
    assert_eq!(rig.envelope(last.id()).state(), EnvelopeState::Queued);
    assert!(rig.envelope(last.id()).transit().is_none());
    assert!(!rig.run(receiver, "+orders").contains("waiting-"));
    rig.advance_ic_to(last.issued().at + ENVELOPE_TTL_SECS);
    rig.pass();
    rig.pass();
    assert_eq!(rig.envelope(last.id()).state(), EnvelopeState::Expired);
}
fn cid(s: &str) -> CanonicalId {
    CanonicalId::new(s).unwrap()
}

#[tokio::test]
async fn r1_real_seed_availability_and_noop_plan_survive_restart() {
    let mut rig = Rig::new(true).await;
    let original = rig.registry();
    rig.restart().await.unwrap();
    assert_eq!(rig.registry(), original);
    assert_eq!((original.nodes.len(), original.pairs.len()), (5, 4));
    assert_eq!(original.pairs[&cid("luna_ceres")].state(), LinkState::Live);
    assert_eq!(original.nodes[&cid("malapert")].coverage.len(), 15);
    assert_eq!(original.nodes[&cid("ceres")].coverage.len(), 13);
    assert!(rig.run(GOD, "@relay/check").contains("Bindings OK"));
    assert!(
        rig.run(GOD, "@relay/show luna_ceres")
            .contains("luna_ceres.ceres")
    );
    assert!(
        rig.run(GOD, "@relay/disable luna_ceres.luna")
            .contains("revision 2")
    );
    assert_eq!(
        rig.registry().pairs[&cid("luna_ceres")].state(),
        LinkState::Degraded
    );
    assert!(
        rig.run(GOD, "@relay/disable luna_ceres.luna")
            .contains("no change")
    );
    rig.flush().await;
    let disabled = rig.registry();
    rig.restart().await.unwrap();
    assert_eq!(rig.registry(), disabled);
    let reply = rig.run(GOD, "@atlas/pack plan tacsim.relay.luna_ceres");
    assert!(reply.contains("Nothing to do."), "{reply}");
    assert_eq!(rig.registry(), disabled);
    assert!(
        rig.run(GOD, "@relay/enable luna_ceres.luna")
            .contains("revision 3")
    );
    assert!(
        rig.run(GOD, "@relay/power luna_ceres.ceres=off")
            .contains("revision 4")
    );
    assert!(rig.run(GOD, "@relay/check").contains("Degraded"));
    assert!(
        rig.run(GOD, "@relay/power luna_ceres.ceres=on")
            .contains("revision 5")
    );
    assert!(
        rig.run(GOD, "@relay/staff luna_ceres.ceres=off")
            .contains("revision 6")
    );
    assert!(rig.run(GOD, "@relay/check").contains("Degraded"));
    assert!(
        rig.run(GOD, "@relay/staff luna_ceres.ceres=on")
            .contains("revision 7")
    );
    rig.flush().await;
    rig.restart().await.unwrap();
    let mut restored = rig.registry();
    assert_eq!(restored.pairs[&cid("luna_ceres")].state(), LinkState::Live);
    restored.revision = original.revision;
    assert_eq!(restored, original);
    assert!(rig.tw.world().refused_namespace_writes().is_empty());
}

#[tokio::test]
async fn r1_existing_world_acquires_seed_once() {
    let mut rig = Rig::new(false).await;
    let old_ids: Vec<_> = rig.tw.world().iter().map(|o| o.id).collect();
    assert!(
        rig.tw
            .world()
            .module_state::<RelayStore>()
            .unwrap()
            .registry()
            .is_none()
    );
    rig.cfg.tacsim.enabled = true;
    rig.restart().await.unwrap();
    let original = rig.registry();
    for id in old_ids {
        assert!(rig.tw.world().get(id).is_some());
    }
    rig.flush().await;
    rig.restart().await.unwrap();
    assert_eq!(rig.registry(), original);
}

#[tokio::test]
async fn r1_permissions_and_installed_object_lifecycle() {
    let mut rig = Rig::new(true).await;
    let player = rig.tw.spawn_player("RelayVisitor");
    let original = rig.registry();
    let denied = rig.run(player, "@relay/show luna_ceres");
    assert!(denied.contains("Permission denied"), "{denied}");
    assert_eq!(denied, rig.run(player, "@relay/show nonexistent"));
    assert_eq!(denied, rig.run(player, "@relay/disable luna_ceres.luna"));
    assert_eq!(rig.registry(), original);
    let half = original.pairs[&cid("luna_ceres")].halves[0].object;
    let reply = rig.run(GOD, &format!("@teleport {half}={LIMBO}"));
    assert!(reply.contains("cannot be moved"), "{reply}");
    let reply = rig.run(GOD, &format!("@behavior/remove {half}=sim.relay_half"));
    assert!(reply.contains("still referenced"), "{reply}");
    let reply = rig.run(GOD, &format!("@destroy {half}"));
    assert!(
        reply.contains("refus") || reply.contains("behavior"),
        "{reply}"
    );
    original.reconcile(rig.tw.world()).unwrap();
}

#[tokio::test]
async fn r1_bad_documents_are_held_not_reseeded() {
    let mut rig = Rig::new(true).await;
    let original = rig.registry();
    let good = serde_json::to_value(&original).unwrap();
    let mut bad_cases = Vec::new();
    bad_cases.push(serde_json::json!({"version":999}));
    let mut duplicate = good.clone();
    duplicate["pairs"]["luna_ceres"]["halves"][1]["id"] =
        duplicate["pairs"]["luna_ceres"]["halves"][0]["id"].clone();
    bad_cases.push(duplicate);
    let mut conflict = good.clone();
    conflict["pairs"]["luna_ceres"]["halves"][0]["object"] =
        good["nodes"]["malapert"]["object"].clone();
    bad_cases.push(conflict);
    let mut location = good.clone();
    location["nodes"]["malapert"]["place"] = serde_json::json!(LIMBO.0);
    location["nodes"]["malapert"]["coverage"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!(LIMBO.0));
    bad_cases.push(location);
    let mut huge = good.clone();
    huge["nodes"]["malapert"]["label"] = serde_json::json!("x".repeat(mush_sim::relay::MAX_BYTES));
    bad_cases.push(huge);
    for bad in bad_cases {
        rig.replace_document(bad.clone()).await;
        let error = rig.restart().await.unwrap_err();
        assert!(error.contains("relay:registry"), "{error}");
        assert_eq!(
            rig.store.module_json_get("sim", KEY).await.unwrap(),
            Some(bad)
        );
        rig.replace_document(good.clone()).await;
        rig.restart().await.unwrap();
        assert_eq!(rig.registry(), original);
    }
    rig.store
        .write(WriteBatch {
            serial: rig.serial,
            module_ops: vec![ModuleOp {
                ns: "sim".into(),
                op: ModuleWrite::Del { key: KEY.into() },
            }],
            ..WriteBatch::default()
        })
        .await
        .unwrap();
    let error = rig.restart().await.unwrap_err();
    assert!(error.contains("unrecorded binding"), "{error}");
    assert!(
        rig.store
            .module_json_get("sim", KEY)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn r1_bounds_and_terminal_state_are_enforced() {
    let rig = Rig::new(true).await;
    let original = rig.registry();
    for bound in ["nodes", "pairs", "coverage"] {
        let mut bad = original.clone();
        match bound {
            "nodes" => {
                for i in 0..MAX_NODES {
                    bad.nodes.insert(
                        cid(&format!("extra{i}")),
                        original.nodes[&cid("ceres")].clone(),
                    );
                }
            }
            "pairs" => {
                for i in 0..MAX_PAIRS {
                    bad.pairs.insert(
                        cid(&format!("extra{i}")),
                        original.pairs[&cid("luna_ceres")].clone(),
                    );
                }
            }
            _ => bad.nodes.get_mut(&cid("ceres")).unwrap().coverage = vec![LIMBO; MAX_COVERAGE + 1],
        }
        assert!(bad.validate().is_err(), "{bound}");
    }
    let mut pair = original.pairs[&cid("luna_ceres")].clone();
    pair.halves[0].destroyed = true;
    assert_eq!(pair.state(), LinkState::Dead);
    pair.halves[1].powered = false;
    assert_eq!(pair.state(), LinkState::Dead);
}

#[tokio::test]
async fn r6a_mail_handoffs_gate_visibility_and_survive_each_restart() {
    use mush_propagation::EnvelopeState;
    let mut rig = Rig::new(true).await;
    let bob = rig.endpoints();
    rig.public_hub(false);
    let stickney = rig
        .tw
        .world()
        .iter()
        .find(|o| o.name == "Townships Receiving")
        .unwrap()
        .id;
    rig.run(GOD, &format!("@teleport {bob}={stickney}"));
    // Source intent and Sent copy are durable; no destination entry exists yet.
    assert!(
        rig.run(GOD, &format!("@mail/quick {bob}=R6a-secret/first body"))
            .contains("queued for IC transit")
    );
    let id = rig.mail().next_id() - 1;
    assert!(rig.mail_route(id, bob, false).is_none());
    rig.mail_hidden(bob, "R6a-secret");
    rig.flush().await;
    rig.restart().await.unwrap();
    rig.mail_hidden(bob, "R6a-secret");
    rig.mail_tick();
    let outbound = rig.mail_route(id, bob, false).unwrap();
    let flight = outbound.transit().unwrap().flight.clone().unwrap();
    assert!(flight.seconds > 0);
    // Transport acceptance/in-flight crash: exact frozen itinerary survives.
    rig.flush().await;
    rig.restart().await.unwrap();
    assert_eq!(rig.mail_route(id, bob, false).unwrap(), outbound);
    rig.mail_hidden(bob, "R6a-secret");
    // Sender deletion cannot reap a payload still needed by its recipient.
    rig.run(GOD, "@mail/folder sent");
    rig.run(GOD, "@mail/delete 1");
    rig.run(GOD, "@mail/purge");
    assert!(rig.mail().message(id).is_some());
    rig.advance_ic_to(flight.due.at - 1);
    rig.pass();
    rig.mail_hidden(bob, "R6a-secret");
    rig.advance_ic_to(flight.due.at);
    rig.pass();
    assert_eq!(
        rig.mail_route(id, bob, false).unwrap().state(),
        EnvelopeState::Delivered
    );
    // Receipt has committed, but the mail owner has not applied it yet.
    rig.mail_hidden(bob, "R6a-secret");
    rig.flush().await;
    rig.restart().await.unwrap();
    let events = rig.mail_tick();
    assert_eq!(
        events
            .iter()
            .filter(
                |(a, e)| matches!(a,mush_core::Audience::Actor(x) if *x==bob)
                    && matches!(e,mush_proto::OutputEvent::MailNotice{ id: n,.. } if *n==id)
            )
            .count(),
        1
    );
    assert_eq!(rig.mail().unread_counts(bob).0, 1);
    assert!(
        rig.mail()
            .message(id)
            .unwrap()
            .delivery
            .as_ref()
            .unwrap()
            .is_empty(),
        "no instant success knowledge"
    );
    rig.flush().await;
    rig.restart().await.unwrap();
    assert_eq!(rig.mail().unread_counts(bob).0, 1);
    for _ in 0..3 {
        let events = rig.mail_tick();
        assert!(
            !events
                .iter()
                .any(|(_, e)| matches!(e,mush_proto::OutputEvent::MailNotice{ id:n,..} if *n==id))
        );
    }
    let returning = rig.mail_route(id, bob, true).unwrap();
    let return_flight = returning.transit().unwrap().flight.as_ref().unwrap();
    assert!(return_flight.seconds > 0);
    assert!(
        rig.mail()
            .message(id)
            .unwrap()
            .delivery
            .as_ref()
            .unwrap()
            .is_empty()
    );
    rig.finish_mail(id);
    assert_eq!(
        rig.mail().message(id).unwrap().delivery.as_ref().unwrap()[&bob],
        mush_mail::transit::Outcome::Delivered
    );
    assert!(rig.run(bob, "@mail 1").contains("first body"));
    rig.flush().await;
    rig.restart().await.unwrap();
    assert_eq!(rig.mail().folder_ids(bob, "inbox"), vec![id]);
    assert!(rig.tw.world().refused_namespace_writes().is_empty());
}

#[tokio::test]
async fn r6a_all_send_paths_bulk_self_and_old_mail_use_the_right_boundary() {
    let mut rig = Rig::new(true).await;
    let bob = rig.endpoints();
    // Legacy records are already delivered and never acquire new dispatches.
    rig.flush().await;
    let old = serde_json::json!({"id":99,"from":GOD,"to":[bob],"subject":"legacy","body":"old body","sent_at":42});
    let entry = serde_json::json!({"owner":bob,"msg":99,"folder":"inbox","status":"read"});
    rig.store
        .write(WriteBatch {
            serial: rig.serial,
            durable: true,
            module_ops: vec![
                ModuleOp {
                    ns: "mail".into(),
                    op: ModuleWrite::JsonSet {
                        key: "msg:99".into(),
                        value: old,
                    },
                },
                ModuleOp {
                    ns: "mail".into(),
                    op: ModuleWrite::JsonSet {
                        key: format!("box:{}:99", bob.0).into(),
                        value: entry,
                    },
                },
            ],
            ..Default::default()
        })
        .await
        .unwrap();
    rig.serial += 1;
    rig.restart().await.unwrap();
    assert!(rig.run(bob, "@mail 1").contains("old body"));
    assert!(
        mush_mail::transit::intents(rig.tw.world())
            .unwrap()
            .is_empty()
    );
    // Reply compose, reply-all and forward each generate a new source dispatch.
    for command in ["@mail/reply 1".to_string(), "@mail/replyall 1".to_string()] {
        rig.run(bob, &command);
        rig.run(bob, "reply body");
        assert!(rig.run(bob, "--").contains("queued for IC transit"));
        let id = rig.mail().next_id() - 1;
        assert!(rig.mail().entry(GOD, id).is_none());
        rig.finish_mail(id);
        assert!(rig.mail().entry(GOD, id).is_some());
    }
    assert!(
        rig.run(bob, "@mail/fwd 1=#1")
            .contains("queued for IC transit")
    );
    let id = rig.mail().next_id() - 1;
    rig.finish_mail(id);
    assert!(rig.mail().message(id).unwrap().body.contains("old body"));
    assert!(
        rig.run(bob, &format!("@mail/quick {bob}=self/local note"))
            .contains("queued for IC transit")
    );
    let id = rig.mail().next_id() - 1;
    assert_eq!(rig.mail().entry(bob, id).unwrap().folder.as_str(), "sent");
    rig.finish_mail(id);
    assert_eq!(rig.mail().entry(bob, id).unwrap().folder.as_str(), "inbox");
    // The full supported body stays with mail. Actual bytes select bulk fallback.
    let body = "x".repeat(mush_mail::MAX_MAIL_BODY_BYTES);
    // Programmatic/compose ingress still reaches the same bounded submit function;
    // use 512 valid compose lines to avoid the connection's individual line cap.
    rig.run(GOD, &format!("@mail/send {bob}=bulk"));
    for _ in 0..511 {
        rig.run(GOD, &"x".repeat(127));
    }
    rig.run(GOD, &"x".repeat(128));
    assert!(rig.run(GOD, "--").contains("queued for IC transit"));
    let id = rig.mail().next_id() - 1;
    assert_eq!(rig.mail().message(id).unwrap().body.len(), body.len());
    rig.mail_tick();
    let outbound = rig.mail_route(id, bob, false).unwrap();
    assert_eq!(outbound.dispatch_ref().unwrap().bytes, 65_540);
    assert!(
        outbound
            .transit()
            .unwrap()
            .legs
            .iter()
            .all(|l| l.pair.is_none())
    );
    assert!(rig.mail().entry(bob, id).is_none());
    rig.finish_mail(id);
    assert_eq!(rig.mail().message(id).unwrap().body.len(), 65_536);
    assert!(rig.mail().entry(bob, id).is_some());
}

#[tokio::test]
async fn r6a_refusals_wait_for_arrival_and_a_physical_return() {
    use mush_mail::transit::Outcome;
    let mut rig = Rig::new(true).await;
    let bob = rig.endpoints();
    rig.public_hub(false);
    let alice = rig.tw.spawn_player("MailAlice");
    let luna = rig.registry().nodes[&cid("malapert")].coverage[0];
    rig.run(GOD, &format!("@teleport {alice}={luna}"));
    rig.tw.world_mut().set_connected(bob, false);
    for expected in [Outcome::Refused, Outcome::Full, Outcome::Unavailable] {
        rig.set_mail_quota(300).await;
        rig.run(alice, &format!("@mail/quick {bob}=policy/private"));
        let id = rig.mail().next_id() - 1;
        rig.mail_tick();
        let outbound = rig.mail_route(id, bob, false).unwrap();
        assert!(
            rig.mail()
                .message(id)
                .unwrap()
                .delivery
                .as_ref()
                .unwrap()
                .is_empty()
        );
        match expected {
            Outcome::Refused => {
                assert!(rig.run(bob, "@lock/mail me=#FALSE").contains("Locked."));
            }
            Outcome::Full => {
                assert!(rig.run(bob, "@unlock/mail me").contains("Unlocked."));
                rig.set_mail_quota(0).await;
            }
            Outcome::Unavailable => {
                rig.set_mail_quota(300).await;
                rig.tw
                    .world_mut()
                    .set_flag(bob, mush_core::Flags::GOING, true)
                    .unwrap();
            }
            _ => unreachable!(),
        }
        let due = outbound.transit().unwrap().flight.as_ref().unwrap().due.at;
        rig.advance_ic_to(due);
        rig.pass();
        rig.mail_tick();
        assert!(rig.mail().entry(bob, id).is_none());
        assert!(
            rig.mail()
                .message(id)
                .unwrap()
                .delivery
                .as_ref()
                .unwrap()
                .is_empty(),
            "failure must return physically"
        );
        rig.finish_mail(id);
        assert_eq!(
            rig.mail().message(id).unwrap().delivery.as_ref().unwrap()[&bob],
            expected
        );
    }
}

#[tokio::test]
async fn r6a_pending_capacity_expiry_and_deleted_sent_payload_are_bounded() {
    let mut rig = Rig::new(true).await;
    rig.endpoints();
    let targets: Vec<_> = (0..32)
        .map(|i| rig.tw.spawn_player(&format!("UnplacedMail{i}")))
        .collect();
    let names = targets
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",");
    for i in 0..1 {
        assert!(
            rig.run(GOD, &format!("@mail/quick {names}=waiting{i}/secret"))
                .contains("queued for IC transit")
        );
    }
    let next = rig.mail().next_id();
    assert!(
        rig.run(GOD, &format!("@mail/quick {names}=overload/x"))
            .contains("allowance is full")
    );
    assert_eq!(rig.mail().next_id(), next, "refusal allocates nothing");
    assert_eq!(
        mush_mail::transit::intents(rig.tw.world()).unwrap().len(),
        32
    );
    let first = next - 1;
    for _ in 0..10 {
        rig.mail_tick();
    }
    assert!(
        rig.mail_route(first, targets[0], false)
            .unwrap()
            .transit()
            .is_none()
    );
    for target in &targets {
        assert_eq!(rig.mail().unread_counts(*target).0, 0);
    }
    rig.run(GOD, "@mail/folder sent");
    for _ in 0..4 {
        rig.run(GOD, "@mail/delete 1");
    }
    rig.run(GOD, "@mail/purge");
    assert!(
        rig.mail().message(first).is_some(),
        "pending copies retain their payload"
    );
    rig.flush().await;
    rig.restart().await.unwrap();
    let expiry = rig
        .mail_route(first, targets[0], false)
        .unwrap()
        .issued()
        .at
        + mush_propagation::ENVELOPE_TTL_SECS;
    rig.advance_ic_to(expiry);
    rig.pass();
    rig.pass();
    for id in first..next {
        rig.finish_mail(id);
    }
    for _ in 0..3 {
        rig.mail_tick();
    }
    assert!(
        rig.mail().message(first).is_none(),
        "completed orphan payload is reaped"
    );
    assert!(
        mush_mail::transit::intents(rig.tw.world())
            .unwrap()
            .is_empty()
    );
    assert!(
        rig.tw
            .world()
            .module_state::<mush_propagation::PropagationStore>()
            .unwrap()
            .envelopes()
            .all(|e| e.dispatch_ref().is_none())
    );
    assert!(
        rig.run(
            GOD,
            &format!("@mail/quick {}=capacity returned/x", targets[0])
        )
        .contains("queued for IC transit")
    );
}

#[tokio::test]
async fn r6b_bounds_expiry_and_retention_release_real_transport_work() {
    let mut rig = Rig::new(true).await;
    let bob = rig.endpoints();
    let chan = rig.channel_setup(&[GOD, bob]);
    rig.run(GOD, "@relay/disable luna_ceres.luna");
    for n in 0..16 {
        assert!(
            rig.run(GOD, &format!("radio bounded-{n}"))
                .contains("queued for IC transit")
        );
    }
    assert!(rig.run(GOD, "radio refused-17").contains("full"));
    assert_eq!(rig.channels().channels[&chan].seq, 16);
    assert_eq!(
        mush_comsys::transit::intents(rig.tw.world()).unwrap().len(),
        32
    );
    rig.flush().await;
    rig.restart().await.unwrap();
    rig.mail_tick();
    rig.advance_ic_to(rig.tw.world().world_ic_now().at + 31 * 86400);
    for _ in 0..40 {
        rig.mail_tick();
    }
    assert!(rig.channels().ic.pending.is_empty());
    assert!(!rig.backlog(bob).contains("bounded-"));
    assert!(rig.channels().ic.progress.expired >= 16);
    rig.run(GOD, "@relay/enable luna_ceres.luna");
    rig.run(GOD, "@channel/set Radio=cap=1");
    for n in 17..=18 {
        rig.run(GOD, &format!("radio retained-{n}"));
        rig.finish_channel(chan, n);
    }
    assert!(!rig.backlog(bob).contains("retained-17"));
    assert!(rig.backlog(bob).contains("retained-18"));
    assert_eq!(rig.channels().ic.messages.len(), 1);
    rig.run(GOD, "delcom radio");
    rig.run(bob, "delcom radio");
    assert!(rig.channels().ic.messages.is_empty());
    rig.flush().await;
    rig.restart().await.unwrap();
    assert!(rig.channels().ic.messages.is_empty());
}

#[tokio::test]
async fn r6b_membership_bound_and_corrupt_knowledge_fail_closed() {
    let mut rig = Rig::new(true).await;
    let luna = rig.registry().nodes[&cid("malapert")].coverage[0];
    rig.run(GOD, &format!("@teleport me={luna}"));
    let chan = rig.channel_setup(&[GOD]);
    for n in 0..31 {
        let p = rig.tw.spawn_player(&format!("RadioMember{n}"));
        rig.run(GOD, &format!("@teleport {p}={luna}"));
        assert!(rig.run(p, "addcom r=Radio").contains("added"));
    }
    let excess = rig.tw.spawn_player("ExcessMember");
    assert!(!rig.run(excess, "addcom r=Radio").contains("added"));
    assert_eq!(rig.channels().members_of(chan).count(), 32);
    rig.run(GOD, "radio bounded fanout");
    rig.finish_channel(chan, 1);
    let epoch = rig.channels().members[&(GOD, chan)]
        .ic
        .as_ref()
        .unwrap()
        .epoch
        .clone();
    rig.flush().await;
    rig.restart().await.unwrap();
    assert_eq!(
        rig.channels().members[&(GOD, chan)]
            .ic
            .as_ref()
            .unwrap()
            .epoch,
        epoch
    );
    let mut m = rig.channels().members[&(GOD, chan)].clone();
    m.ic.as_mut().unwrap().epoch.clear();
    rig.store
        .write(WriteBatch {
            serial: rig.serial,
            durable: true,
            module_ops: vec![ModuleOp {
                ns: "comsys".into(),
                op: ModuleWrite::JsonSet {
                    key: format!("member:{}:{}", GOD.0, chan.0).into(),
                    value: serde_json::to_value(m).unwrap(),
                },
            }],
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(
        rig.restart().await.is_err(),
        "malformed receipt identity must not silently expose legacy history"
    );
}

#[tokio::test]
async fn r3_four_classes_restart_capacity_receipts_and_scandal() {
    use mush_core::QueryHandler;
    use mush_propagation::{
        EnvelopeState,
        service::{BoardQuery, Class},
    };
    let mut rig = Rig::new(true).await;
    let receiver = rig.endpoints();
    let luna = rig.registry().nodes[&cid("malapert")].coverage[0];
    let mut senders = vec![];
    for (name, class) in [
        ("StandardSender", "standard"),
        ("DeferredSender", "deferred"),
        ("PrioritySender", "priority"),
        ("SovereignSender", "sovereign"),
    ] {
        let who = rig.tw.spawn_player(name);
        rig.run(GOD, &format!("@teleport {who}={luna}"));
        assert!(
            rig.run(GOD, &format!("+relay/grant luna_ceres/{who}={class}"))
                .contains("Class grant recorded")
        );
        senders.push(who);
        assert!(
            rig.run(
                who,
                &format!("@mail/quick {receiver}=R3-{class}/{}", "x".repeat(490))
            )
            .contains("queued")
        );
    }
    rig.mail_tick();
    let policy = rig
        .tw
        .world()
        .module_state::<mush_propagation::PropagationStore>()
        .unwrap()
        .relay_transport()
        .service
        .as_ref()
        .unwrap()
        .clone();
    let envelopes: Vec<_> = rig
        .tw
        .world()
        .module_state::<mush_propagation::PropagationStore>()
        .unwrap()
        .envelopes()
        .filter(|e| {
            e.kind() == mush_propagation::record::DispatchKind::Mail
                && e.transit().is_some_and(|t| t.queued_at.is_some())
        })
        .cloned()
        .collect();
    assert_eq!(envelopes.len(), 4, "all four durably queued before service");
    assert!(
        envelopes
            .iter()
            .all(|e| e.transit().unwrap().service.as_ref().unwrap().units == 4)
    );
    rig.flush().await;
    rig.restart().await.unwrap();
    assert_eq!(
        rig.tw
            .world()
            .module_state::<mush_propagation::PropagationStore>()
            .unwrap()
            .relay_transport()
            .service
            .as_ref()
            .unwrap(),
        &policy
    );
    for e in &envelopes {
        assert_eq!(rig.envelope(e.id()), *e);
    }
    rig.pass();
    rig.pass();
    let receipts = rig
        .tw
        .world()
        .module_state::<mush_propagation::PropagationStore>()
        .unwrap()
        .relay_transport()
        .service
        .as_ref()
        .unwrap()
        .receipts
        .clone();
    assert_eq!(receipts.len(), 2);
    assert!(
        receipts
            .iter()
            .all(|r| matches!(r.ticket.class, Class::Priority | Class::Sovereign))
    );
    assert_eq!(receipts.iter().map(|r| r.ticket.units).sum::<u32>(), 8);
    for _ in 0..3 {
        rig.pass();
    }
    assert_eq!(
        rig.tw
            .world()
            .module_state::<mush_propagation::PropagationStore>()
            .unwrap()
            .relay_transport()
            .service
            .as_ref()
            .unwrap()
            .receipts
            .len(),
        2,
        "retry does not spend twice"
    );
    let standard = envelopes.iter().find(|e| e.issuer() == senders[0]).unwrap();
    let eligible = standard
        .transit()
        .unwrap()
        .service
        .as_ref()
        .unwrap()
        .eligible_at;
    rig.advance_ic_to(eligible - 1);
    rig.pass();
    assert_ne!(
        rig.envelope(standard.id()).state(),
        EnvelopeState::Delivered
    );
    rig.advance_ic_to(eligible);
    rig.pass();
    assert_eq!(
        rig.envelope(standard.id()).state(),
        EnvelopeState::Delivered
    );
    let deferred = envelopes.iter().find(|e| e.issuer() == senders[1]).unwrap();
    let earliest = deferred
        .transit()
        .unwrap()
        .service
        .as_ref()
        .unwrap()
        .eligible_at;
    rig.advance_ic_to(earliest);
    rig.pass();
    let cfg = policy.config;
    if !cfg.window_open(earliest) {
        assert_ne!(
            rig.envelope(deferred.id()).state(),
            EnvelopeState::Delivered
        );
    }
    let next = earliest
        + (cfg.window_period_s - earliest.rem_euclid(cfg.window_period_s)) % cfg.window_period_s;
    rig.advance_ic_to(next);
    rig.pass();
    assert_eq!(
        rig.envelope(deferred.id()).state(),
        EnvelopeState::Delivered
    );
    rig.mail_tick();
    assert!(rig.run(receiver, "+relay/receipts").contains("Standard"));
    assert!(
        !rig.run(senders[0], "+relay/receipts")
            .contains("transmitted"),
        "sender has no remote receipt shortcut"
    );
    let public = rig.run(receiver, "+relay");
    assert!(!public.contains("actual queued"));
    let panel = BoardQuery
        .query(
            rig.tw.world(),
            receiver,
            &serde_json::json!({"actor":1,"actual":true}),
        )
        .unwrap();
    if let mush_proto::OutputEvent::Panel { data, .. } = panel {
        assert_eq!(
            data["lines"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect::<Vec<_>>()
                .join("\n"),
            public
        );
    } else {
        panic!("board panel");
    }
    assert!(
        rig.run(receiver, "+relay/load luna_ceres")
            .contains("Permission denied")
    );
    assert!(
        rig.run(receiver, "+relay/publish luna_ceres")
            .contains("Permission denied")
    );
    let operator = rig.registry().operator;
    let org = rig
        .tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .unwrap()
        .state()
        .org(operator)
        .unwrap()
        .id
        .0;
    rig.run(
        GOD,
        &format!("@org/grant {}={org} relay.publish", senders[0]),
    );
    for command in [
        "+relay/load/publish luna_ceres",
        "+relay/audit/publish luna_ceres",
        "+relay/discover/publish luna_ceres",
    ] {
        let output = rig.run(senders[0], command);
        assert!(
            output.contains("Choose one relay command switch"),
            "{output}"
        );
        assert!(!output.contains("actual queued"));
    }
    rig.run(GOD, &format!("@org/grant {receiver}={org} relay.inspect"));
    let discovered = rig.run(receiver, "+relay/discover luna_ceres");
    assert!(discovered.contains("Evidence filed"), "{discovered}");
    assert!(rig.run(receiver, "+intel").contains("Standard queued"));
    assert!(!rig.run(senders[0], "+intel").contains("Standard queued"));
    assert!(
        rig.run(receiver, "+relay/publish luna_ceres")
            .contains("Permission denied")
    );
    rig.run(GOD, &format!("@org/grant {receiver}={org} relay.publish"));
    // Knowledge is bounded independently of retained transport evidence. A
    // rolled-off or cleared observation can be filed again through its owner.
    rig.flush().await;
    rig.store
        .write(WriteBatch {
            serial: rig.serial,
            durable: true,
            module_ops: vec![ModuleOp {
                ns: "npc".into(),
                op: ModuleWrite::JsonSet {
                    key: mush_npc::intel_key(mush_faction::Principal::Character(receiver)),
                    value: serde_json::json!([]),
                },
            }],
            ..Default::default()
        })
        .await
        .unwrap();
    rig.serial += 1;
    rig.restart().await.unwrap();
    assert!(
        rig.run(receiver, "+relay/publish luna_ceres")
            .contains("no longer")
    );
    assert!(
        rig.run(receiver, "+relay/discover luna_ceres")
            .contains("knowledge refreshed")
    );
    assert!(rig.run(receiver, "+intel").contains("Standard queued"));
    rig.run(receiver, "+relay/discover luna_ceres");
    assert_eq!(
        rig.tw
            .world()
            .module_state::<mush_npc::NpcHostState>()
            .unwrap()
            .intel
            .for_principal(mush_faction::Principal::Character(receiver))
            .len(),
        1
    );
    // R8 makes Deferred the free default; restoring it releases the courtesy slot.
    rig.run(GOD, &format!("+relay/grant luna_ceres/{receiver}=deferred"));
    assert!(
        !rig.tw
            .world()
            .module_state::<mush_propagation::PropagationStore>()
            .unwrap()
            .relay_transport()
            .service
            .as_ref()
            .unwrap()
            .grants
            .iter()
            .any(|g| g.principal == receiver && g.pair == cid("luna_ceres"))
    );
    let published = rig.run(receiver, "+relay/publish luna_ceres");
    assert!(published.contains("scandal front"), "{published}");
    rig.flush().await;
    rig.restart().await.unwrap();
    assert!(
        rig.run(receiver, "+relay/publish luna_ceres")
            .contains("already published")
    );
    assert_eq!(
        rig.tw
            .world()
            .module_state::<mush_faction::SimState>()
            .unwrap()
            .fronts
            .values()
            .filter(|f| f.title == "The Exchange congestion board")
            .count(),
        1
    );
    assert!(
        rig.tw.world().refused_namespace_writes().is_empty(),
        "{:?}",
        rig.tw.world().refused_namespace_writes()
    );
}

#[tokio::test]
async fn r3_mixed_traffic_serializes_fifo_and_receipts_follow_membership() {
    use mush_propagation::record::DispatchKind;
    let mut rig = Rig::new(true).await;
    let bob = rig.endpoints();
    let channel = rig.channel_setup(&[GOD, bob]);
    // Seven mail units, then an eight-unit channel copy, then one order unit.
    // The channel uses the remaining unit and retains FIFO rank on restart.
    assert!(
        rig.run(GOD, &format!("@mail/quick {bob}=seven/{}", "m".repeat(840)))
            .contains("queued")
    );
    let mail_id = rig.mail().next_id() - 1;
    rig.mail_tick();
    assert!(
        rig.run(GOD, &format!("radio {}", "c".repeat(980)))
            .contains("queued")
    );
    rig.mail_tick();
    assert!(
        rig.run(GOD, &format!("+order {bob}=small"))
            .contains("Queued")
    );
    let order = rig.last_envelope().id();
    rig.pass();
    rig.pass();
    rig.pass();
    assert_eq!(
        rig.envelope(order).path().len(),
        0,
        "older partial work retains FIFO rank"
    );
    let state = rig
        .tw
        .world()
        .module_state::<mush_propagation::PropagationStore>()
        .unwrap()
        .relay_transport()
        .clone();
    let channel_receipts = state
        .service
        .as_ref()
        .unwrap()
        .receipts
        .iter()
        .filter(|r| r.kind == DispatchKind::Channel)
        .count();
    assert_eq!(channel_receipts, 0);
    let partial = rig
        .tw
        .world()
        .module_state::<mush_propagation::PropagationStore>()
        .unwrap()
        .envelopes()
        .find(|e| e.kind() == DispatchKind::Channel && e.recipient() == bob)
        .unwrap()
        .transit()
        .unwrap()
        .serialization
        .clone()
        .unwrap();
    assert_eq!((partial.sent, partial.total), (1, 8));
    assert_eq!(state.capacity[&cid("luna_ceres")].units, 8);
    rig.flush().await;
    rig.restart().await.unwrap();
    rig.pass();
    assert_eq!(
        rig.tw
            .world()
            .module_state::<mush_propagation::PropagationStore>()
            .unwrap()
            .relay_transport()
            .capacity,
        state.capacity
    );
    let now = rig.tw.world().world_ic_now().at;
    rig.advance_ic_to(now + 1);
    rig.finish_channel(channel, 1);
    rig.finish_mail(mail_id);
    rig.pb_passes();
    assert_eq!(rig.envelope(order).path().len(), 1);
    assert!(rig.run(bob, "+relay/receipts").contains("Channel"));
    rig.run(bob, "delcom radio");
    assert!(!rig.run(bob, "+relay/receipts").contains("Channel"));
    rig.run(bob, "addcom radio=Radio");
    assert!(!rig.run(bob, "+relay/receipts").contains("Channel"));
}

#[tokio::test]
async fn r3_aged_standard_work_precedes_new_high_class_traffic() {
    let mut rig = Rig::new(true).await;
    let bob = rig.endpoints();
    rig.run(GOD, "+relay/grant luna_ceres/#1=standard");
    rig.run(GOD, &format!("+order {bob}={}", "s".repeat(512)));
    let old = rig.last_envelope().id();
    rig.pass();
    let queued = rig
        .envelope(old)
        .transit()
        .unwrap()
        .queued_at
        .as_ref()
        .unwrap()
        .at;
    rig.advance_ic_to(queued + 600);
    rig.run(GOD, "+relay/grant luna_ceres/#1=sovereign");
    let mut young = vec![];
    for _ in 0..3 {
        rig.run(GOD, &format!("+order {bob}={}", "y".repeat(512)));
        young.push(rig.last_envelope().id());
    }
    rig.pass();
    rig.pass();
    rig.pass();
    let state = rig
        .tw
        .world()
        .module_state::<mush_propagation::PropagationStore>()
        .unwrap()
        .relay_transport();
    assert_eq!(
        state
            .service
            .as_ref()
            .unwrap()
            .receipts
            .first()
            .unwrap()
            .envelope,
        old
    );
    assert_eq!(rig.envelope(old).path().len(), 1);
    assert_eq!(
        young
            .iter()
            .filter(|id| rig.envelope(**id).path().len() == 1)
            .count(),
        1
    );
    assert_eq!(state.capacity[&cid("luna_ceres")].units, 8);
}

#[tokio::test]
async fn r3_full_legacy_tickets_preserve_capacity_and_refuse_partial_policy() {
    let mut rig = Rig::new(true).await;
    let bob = rig.endpoints();
    let exit = rig.registry().nodes[&cid("ceres")].place;
    rig.run(GOD, &format!("@prop/jam {exit}=100"));
    let other = rig.tw.spawn_player("LegacyOperator");
    rig.tw
        .world_mut()
        .set_flag(other, mush_core::Flags::ROYALTY, true)
        .unwrap();
    let luna = rig.registry().nodes[&cid("malapert")].place;
    rig.run(GOD, &format!("@teleport {other}={luna}"));
    // Fill the shared link through independent producers/principals. New mail
    // alone cannot monopolize it; old accepted tickets still migrate in place.
    for n in 0..32 {
        assert!(
            rig.run(GOD, &format!("@mail/quick {bob}=legacy-{n}/payload"))
                .contains("queued for IC transit")
        );
        assert!(
            rig.run(other, &format!("+order {bob}=legacy-{n}"))
                .contains("Queued")
        );
    }
    // Convert admitted tickets to their R2/R6 shape while preserving the
    // separately funded R8 market. Dropping the whole service record now loses
    // real purchase authority and correctly refuses boot, tested below.
    for _ in 0..4 {
        rig.mail_tick();
    }
    rig.flush().await;
    let book = rig
        .tw
        .world()
        .module_state::<mush_propagation::PropagationStore>()
        .unwrap();
    assert_eq!(
        book.envelopes()
            .filter_map(|e| e.transit())
            .filter(|t| t.queued_at.is_some())
            .count(),
        64
    );
    let mut transport = serde_json::to_value(book.relay_transport()).unwrap();
    let saved = transport.clone();
    let mut legacy = mush_propagation::service::State::new(
        book.relay_transport()
            .service
            .as_ref()
            .unwrap()
            .config
            .clone(),
    );
    legacy.commerce = book
        .relay_transport()
        .service
        .as_ref()
        .unwrap()
        .commerce
        .clone();
    transport["service"] = serde_json::to_value(legacy).unwrap();
    transport["capacity"] =
        serde_json::json!({"luna_ceres":{"at":rig.tw.world().world_ic_now().at,"units":8}});
    let mut ops = vec![ModuleOp {
        ns: "propagation".into(),
        op: ModuleWrite::JsonSet {
            key: "relay_transport".into(),
            value: transport.clone(),
        },
    }];
    for e in book.envelopes() {
        let mut row = serde_json::to_value(e).unwrap();
        if let Some(t) = row.get_mut("transit").and_then(|t| t.as_object_mut()) {
            t.remove("service");
        }
        ops.push(ModuleOp {
            ns: "propagation".into(),
            op: ModuleWrite::JsonSet {
                key: format!("envelope:{}", e.id()).into(),
                value: row,
            },
        });
    }
    rig.store
        .write(WriteBatch {
            serial: rig.serial,
            durable: true,
            module_ops: ops,
            ..Default::default()
        })
        .await
        .unwrap();
    rig.serial += 1;
    rig.restart().await.unwrap();
    let upgraded = rig
        .store
        .module_json_get("propagation", "relay_transport")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(upgraded["capacity"], transport["capacity"]);
    assert_eq!(upgraded["service"]["config"]["version"], 1);
    rig.mail_tick();
    let book = rig
        .tw
        .world()
        .module_state::<mush_propagation::PropagationStore>()
        .unwrap();
    assert!(
        book.envelopes()
            .filter_map(|e| e.transit())
            .filter(|t| t.queued_at.is_some())
            .all(|t| t.service.is_some()),
        "a full legacy queue can acquire tickets without readmission"
    );
    rig.flush().await;
    // Losing the policy beside issued tickets is corruption, not a migration.
    let mut missing = rig
        .store
        .module_json_get("propagation", "relay_transport")
        .await
        .unwrap()
        .unwrap();
    missing.as_object_mut().unwrap().remove("service");
    rig.store
        .write(WriteBatch {
            serial: rig.serial,
            durable: true,
            module_ops: vec![ModuleOp {
                ns: "propagation".into(),
                op: ModuleWrite::JsonSet {
                    key: "relay_transport".into(),
                    value: missing,
                },
            }],
            ..Default::default()
        })
        .await
        .unwrap();
    rig.serial += 1;
    assert!(rig.restart().await.unwrap_err().contains("policy missing"));
    let mut invalid = saved;
    invalid["service"]["config"]["window_period_s"] = 0.into();
    rig.store
        .write(WriteBatch {
            serial: rig.serial,
            durable: true,
            module_ops: vec![ModuleOp {
                ns: "propagation".into(),
                op: ModuleWrite::JsonSet {
                    key: "relay_transport".into(),
                    value: invalid,
                },
            }],
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(rig.restart().await.is_err());
}

#[tokio::test]
async fn r3_expired_queue_tickets_do_not_inflate_private_load() {
    let mut rig = Rig::new(true).await;
    let bob = rig.endpoints();
    let exit = rig.registry().nodes[&cid("ceres")].place;
    rig.run(GOD, &format!("@prop/jam {exit}=100"));
    rig.run(GOD, &format!("+order {bob}=will expire at jammed exit"));
    rig.pass();
    let id = rig.last_envelope().id();
    assert!(
        rig.run(GOD, "+relay/load luna_ceres")
            .contains("actual queued 1")
    );
    let expires =
        rig.tw.world().world_ic_now().at + mush_propagation::record::ENVELOPE_TTL_SECS + 1;
    rig.advance_ic_to(expires);
    rig.pass();
    assert_eq!(
        rig.envelope(id).state(),
        mush_propagation::EnvelopeState::Expired
    );
    assert!(
        rig.run(GOD, "+relay/load luna_ceres")
            .contains("actual queued 0")
    );
}

#[tokio::test]
async fn r4_sender_receiver_policy_revalidates_queue_and_preserves_consumed_service() {
    use mush_propagation::{EnvelopeState, service::Class};
    let mut rig = Rig::new(true).await;
    let bob = rig.endpoints();
    let ceres = rig.registry().nodes[&cid("ceres")].place;
    rig.run(GOD, &format!("@prop/jam {ceres}=100"));
    rig.run(GOD, &format!("+order {bob}=queued-before-throttle"));
    let id = rig.last_envelope().id();
    rig.pass();
    let original = rig.envelope(id).transit().unwrap().service.clone().unwrap();
    assert_eq!(original.class, Class::Priority);
    assert!(
        rig.run(
            bob,
            &format!("+relay/policy luna_ceres/{bob}=suspended,operator.test,unauthorized")
        )
        .contains("Permission denied")
    );
    assert!(
        rig.run(
            GOD,
            "+relay/policy luna_ceres/#1=throttled,operator.incident,review pending"
        )
        .contains("policy recorded")
    );
    rig.pass();
    let ticket = rig.envelope(id).transit().unwrap().service.clone().unwrap();
    assert_eq!(ticket.class, Class::Standard);
    assert_eq!(ticket.base_class, Some(Class::Priority));
    assert!(ticket.throttled);
    assert_eq!(ticket.eligible_at, original.eligible_at + 60);
    rig.flush().await;
    rig.restart().await.unwrap();
    assert!(
        rig.run(GOD, "+relay/status luna_ceres")
            .contains("review pending")
    );
    rig.run(GOD, &format!("@prop/jam {ceres}=0"));
    rig.pass();
    assert_ne!(rig.envelope(id).state(), EnvelopeState::Delivered);
    rig.advance_ic_to(ticket.eligible_at);
    rig.pass();
    assert_eq!(rig.envelope(id).state(), EnvelopeState::Delivered);
    let receipts = rig
        .tw
        .world()
        .module_state::<mush_propagation::PropagationStore>()
        .unwrap()
        .relay_transport()
        .service
        .as_ref()
        .unwrap()
        .receipts
        .clone();
    assert_eq!(receipts.len(), 1);
    assert!(
        !receipts[0].proof(),
        "an honestly imposed throttle is not congestion fraud evidence"
    );
    rig.run(
        GOD,
        "+relay/policy luna_ceres/#1=suspended,operator.incident,closed pending review",
    );
    rig.pass();
    assert_eq!(rig.envelope(id).state(), EnvelopeState::Delivered);
    assert_eq!(
        rig.tw
            .world()
            .module_state::<mush_propagation::PropagationStore>()
            .unwrap()
            .relay_transport()
            .service
            .as_ref()
            .unwrap()
            .receipts,
        receipts
    );
    rig.run(
        GOD,
        "+relay/policy luna_ceres/#1=served,operator.incident,review complete",
    );
    // A receiving principal's suspension overrides even a Sovereign sender grant.
    rig.run(GOD, "+relay/grant luna_ceres/#1=sovereign");
    rig.run(GOD, &format!("@prop/jam {ceres}=100"));
    rig.run(GOD, &format!("+order {bob}=receiver-denial"));
    let second = rig.last_envelope().id();
    rig.pass();
    assert!(rig.envelope(second).transit().unwrap().service.is_some());
    rig.run(
        GOD,
        &format!("+relay/policy luna_ceres/{bob}=suspended,operator.incident,receiver incident"),
    );
    rig.flush().await;
    rig.restart().await.unwrap();
    rig.pass();
    let fallback = rig.envelope(second).transit().unwrap().clone();
    assert!(fallback.legs.iter().all(|l| l.pair.is_none()));
    assert!(fallback.service.is_none());
    assert!(fallback.flight.as_ref().unwrap().seconds > 1);
    // Restoring after transmission cannot teleport the conventional flight.
    rig.run(
        GOD,
        &format!("+relay/policy luna_ceres/{bob}=served,operator.incident,cleared"),
    );
    rig.run(GOD, &format!("@prop/jam {ceres}=0"));
    rig.pass();
    assert_eq!(
        rig.envelope(second).transit().unwrap().flight,
        fallback.flight
    );
    rig.advance_ic_to(fallback.flight.unwrap().due.at);
    rig.pass();
    assert_eq!(rig.envelope(second).state(), EnvelopeState::Delivered);
    assert!(rig.envelope(second).path().is_empty());
}

#[tokio::test]
async fn r4_receiver_throttle_sources_status_privacy_and_restore() {
    use mush_propagation::service::Class;
    let mut rig = Rig::new(true).await;
    let bob = rig.endpoints();
    for reason in ["incident", "review"] {
        assert!(
            rig.run(
                GOD,
                &format!(
                    "+relay/policy luna_ceres/{bob}=throttled,operator.{reason},private {reason}"
                )
            )
            .contains("policy recorded")
        );
    }
    rig.run(GOD, &format!("+order {bob}=receiver-throttle"));
    let id = rig.last_envelope().id();
    rig.pass();
    let ticket = rig.envelope(id).transit().unwrap().service.clone().unwrap();
    assert_eq!(
        ticket.class,
        Class::Standard,
        "two throttles do not stack demotion"
    );
    assert!(
        rig.run(bob, "+relay")
            .contains("your class Standard; service Throttled")
    );
    assert!(
        !rig.run(GOD, "+relay/status luna_ceres")
            .contains("private incident")
    );
    assert!(
        rig.run(GOD, &format!("+relay/status luna_ceres/{bob}"))
            .contains("your own service only")
    );
    rig.run(
        GOD,
        &format!("+relay/policy luna_ceres/{bob}=served,operator.incident,cured one"),
    );
    assert!(
        rig.run(bob, "+relay/status luna_ceres")
            .contains("Throttled")
    );
    rig.run(
        GOD,
        &format!("+relay/policy luna_ceres/{bob}=served,operator.review,cured two"),
    );
    rig.pass();
    rig.pass();
    assert_eq!(
        rig.envelope(id).state(),
        mush_propagation::EnvelopeState::Delivered
    );
    assert!(rig.run(bob, "+relay/status luna_ceres").contains("Served"));
    assert!(
        rig.run(
            GOD,
            &format!("+relay/policy luna_ceres/{bob}=suspended,arrears,spoofed bill")
        )
        .contains("must start with operator.")
    );
    assert!(
        rig.run(
            GOD,
            &format!("+relay/policy luna_ceres/{bob}=suspended,operator.bad,")
        )
        .contains("plain reason")
    );
    rig.flush().await;
    rig.restart().await.unwrap();
    assert!(
        rig.run(GOD, "+relay/audit luna_ceres")
            .contains("cured two")
    );
}

#[tokio::test]
async fn r4_org_titled_endpoints_and_independent_operator_link() {
    let mut rig = Rig::new(true).await;
    let bob = rig.endpoints();
    let operator = rig.registry().operator;
    // Normal economic title identifies an org-owned receiving unit, not a payer.
    rig.run(GOD, "@create PolicyReceiverUnit");
    let unit = rig
        .tw
        .world()
        .iter()
        .find(|o| o.name == "PolicyReceiverUnit")
        .unwrap()
        .id;
    let ceres = rig.registry().nodes[&cid("ceres")].place;
    rig.run(GOD, &format!("@teleport {unit}={ceres}"));
    let title = rig.run(GOD, &format!("@asset/title {unit}=org:{operator}"));
    assert!(title.contains("Title recorded"), "{title}");
    rig.run(
        GOD,
        &format!(
            "+relay/policy luna_ceres/org:{operator}=suspended,operator.org,organization denied"
        ),
    );
    rig.run(GOD, &format!("+order {unit}=organization receiver"));
    assert!(
        rig.last_envelope()
            .transit()
            .unwrap()
            .legs
            .iter()
            .all(|l| l.pair.is_none())
    );
    let npc = rig
        .tw
        .world_mut()
        .create(mush_core::ObjKind::Npc, "Policy NPC", GOD, Some(ceres))
        .unwrap();
    assert!(
        rig.run(
            GOD,
            &format!(
                "+relay/policy luna_ceres/{npc}=suspended,operator.npc,Personal NPC restriction"
            )
        )
        .contains("policy recorded")
    );
    rig.run(GOD, &format!("+order {npc}=npc receiver"));
    assert!(
        rig.last_envelope()
            .transit()
            .unwrap()
            .legs
            .iter()
            .all(|l| l.pair.is_none())
    );
    // Personal membership/physical location cannot accidentally become org identity.
    rig.run(GOD, &format!("+order {bob}=personal receiver"));
    assert!(
        rig.last_envelope()
            .transit()
            .unwrap()
            .legs
            .iter()
            .any(|l| l.pair.is_some())
    );
    rig.run(
        GOD,
        &format!("+relay/policy luna_ceres/{bob}=suspended,operator.person,personal denial"),
    );
    rig.run(GOD, &format!("+order {bob}=denied on first operator"));
    assert!(
        rig.last_envelope()
            .transit()
            .unwrap()
            .legs
            .iter()
            .all(|l| l.pair.is_none())
    );
    // Controlled persisted topology fixture with a second, independently owned
    // pair. Objects/behavior bindings use normal APIs; only the topology document
    // is fixture input. R9 supplies manufacture/deployment, not this test fixture.
    let other = rig
        .tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .unwrap()
        .state()
        .orgs()
        .find(|o| o.id != operator)
        .unwrap()
        .id;
    let mut catalog = rig.registry();
    let mut pair = catalog.pairs[&cid("luna_ceres")].clone();
    for (i, key) in ["malapert", "ceres"].iter().enumerate() {
        let mut node = catalog.nodes[&cid(key)].clone();
        let node_name = format!("R4IndependentNode{i}");
        rig.run(GOD, &format!("@create {node_name}"));
        node.object = rig
            .tw
            .world()
            .iter()
            .find(|o| o.name == node_name)
            .unwrap()
            .id;
        rig.run(GOD, &format!("@teleport {}={}", node.object, node.place));
        rig.run(
            GOD,
            &format!("@behavior/add {}=sim.relay_node", node.object),
        );
        rig.run(
            GOD,
            &format!("@unit/create {}=tacsim.unit.relay_station", node.object),
        );
        let half_name = format!("R4IndependentHalf{i}");
        let half = rig
            .tw
            .world_mut()
            .create(
                mush_core::ObjKind::Thing,
                &half_name,
                GOD,
                Some(node.object),
            )
            .unwrap();
        rig.run(GOD, &format!("@behavior/add {half}=sim.relay_half"));
        node.operator = other;
        let key = cid(&format!("independent_{key}"));
        pair.halves[i].node = key.clone();
        pair.halves[i].object = half;
        pair.halves[i].id = cid(&format!("independent.half{i}"));
        catalog.nodes.insert(key, node);
    }
    pair.class_policy = mush_sim::relay::ClassPolicy::Private;
    catalog.pairs.insert(cid("independent"), pair);
    catalog.revision += 1;
    catalog.reconcile(rig.tw.world()).unwrap();
    rig.flush().await;
    rig.replace_document(serde_json::to_value(&catalog).unwrap())
        .await;
    rig.restart().await.unwrap();
    assert!(
        rig.run(GOD, "@atlas/pack plan tacsim.relay.luna_ceres")
            .contains("Nothing to do.")
    );
    rig.run(
        GOD,
        &format!("+relay/access independent/{bob}=discover,use"),
    );
    rig.run(GOD, &format!("@org/grant {bob}={operator} relay.manage"));
    assert!(
        rig.run(
            bob,
            "+relay/policy independent/#1=suspended,operator.test,wrong operator"
        )
        .contains("Permission denied")
    );
    assert!(
        rig.run(
            bob,
            "+relay/policy luna_ceres/#1=suspended,operator.test,correct operator"
        )
        .contains("policy recorded")
    );
    rig.run(
        GOD,
        &format!("+order {bob}=private default remains independent"),
    );
    let e = rig.last_envelope();
    let state = rig
        .tw
        .world()
        .module_state::<mush_propagation::PropagationStore>()
        .unwrap();
    assert_eq!(
        mush_propagation::commerce::base(
            state.relay_transport().service.as_ref().unwrap(),
            rig.tw.world(),
            &cid("independent"),
            &e
        ),
        mush_propagation::service::Class::Standard
    );
    for (class, stored) in [("deferred", true), ("standard", false)] {
        rig.run(GOD, &format!("+relay/grant independent/#1={class}"));
        let book = rig
            .tw
            .world()
            .module_state::<mush_propagation::PropagationStore>()
            .unwrap();
        assert_eq!(
            book.relay_transport()
                .service
                .as_ref()
                .unwrap()
                .grants
                .iter()
                .any(|g| g.pair == cid("independent") && g.principal == GOD),
            stored
        );
    }
    rig.run(GOD, "+relay/grant independent/#1=priority");
    rig.run(GOD, &format!("+order {bob}=independent private link"));
    let id = rig.last_envelope().id();
    assert!(
        rig.envelope(id)
            .transit()
            .unwrap()
            .legs
            .iter()
            .any(|l| l.pair == Some(cid("independent")))
    );
    rig.pass();
    rig.pass();
    assert_eq!(
        rig.envelope(id).state(),
        mush_propagation::EnvelopeState::Delivered
    );
    assert_eq!(rig.envelope(id).path().len(), 1);
}

fn r7_plan(section: &str, value: i64) -> mush_tactics::UnitMutationPlan {
    serde_json::from_value(serde_json::json!({"ops":[{"SetCell":{"addr":{"Section":{"section":section,"cell":"internals"}},"value":value}}]})).unwrap()
}
fn r7_damage(rig: &mut Rig, node: mush_core::ObjId, section: &str, value: i64) {
    let result = rig.run(
        GOD,
        &format!(
            "@unit/damage {node}={}",
            serde_json::to_string(&r7_plan(section, value)).unwrap()
        ),
    );
    assert!(result.contains("Applied"), "{result}");
    assert!(rig.tw.world().refused_namespace_writes().is_empty());
}

#[tokio::test]
async fn r7_damage_permanent_loss_consequences_repair_and_restart() {
    use mush_campaign::{RecoveryConfig, RecoveryGrant, RecoveryRefusal};
    let mut rig = Rig::new(true).await;
    let bob = rig.endpoints();
    let node = rig.registry().nodes[&cid("ceres")].object;
    assert_eq!(rig.registry().structural_version, 1);
    let unit = mush_tactics::module::unit_of(rig.tw.world(), node).unwrap();
    assert_eq!(
        tacsim::definitions::UnitClass::read(unit),
        tacsim::definitions::UnitClass::Station
    );
    // A controlled notoriety-seeking faction is opt-in through normal commands.
    rig.run(GOD, "@faction/create R7Cell");
    let name = rig.tw.world().get(GOD).unwrap().name.clone();
    rig.run(GOD, &format!("@faction/add R7Cell={name}"));
    rig.run(GOD, "@faction/resource R7Cell=attention/999/test opt-in");
    rig.run(GOD, "@faction/resource R7Cell=legitimacy/5/test baseline");
    rig.run(GOD, "@front/create R7Cell=Legitimacy crisis");
    let front = rig
        .tw
        .world()
        .module_state::<mush_faction::SimState>()
        .unwrap()
        .fronts
        .values()
        .find(|f| f.title == "Legitimacy crisis")
        .unwrap()
        .id;
    rig.run(GOD, &format!("@clock/create {front}=Public condemnation/4"));
    let clock = rig
        .tw
        .world()
        .module_state::<mush_faction::SimState>()
        .unwrap()
        .clocks
        .values()
        .find(|c| c.front == front)
        .unwrap()
        .id;
    rig.run(GOD, &format!("@clock/set {clock}=on:relay.node_destroyed"));
    let now = rig.tw.world().world_ic_now().at;
    let grant =
        RecoveryGrant::with_cover(GOD, 1, bob, "scout".into(), now - 100, Some(now - 1), true)
            .unwrap();
    let cfg = RecoveryConfig {
        enabled: true,
        ..Default::default()
    };
    assert!(!matches!(
        mush_campaign::eligible(rig.tw.world(), &cfg, GOD, Some(&grant), now),
        Err(RecoveryRefusal::Cooldown { .. })
    ));
    // Power loss disables, but causes no terminal incident or sanctions.
    r7_damage(&mut rig, node, "power", 0);
    assert_eq!(
        rig.registry().pairs[&cid("luna_ceres")].state(),
        mush_sim::relay::LinkState::Degraded
    );
    assert!(rig.registry().losses.is_empty());
    rig.run(GOD, "@relay/power luna_ceres.ceres=on");
    assert_eq!(
        rig.registry().pairs[&cid("luna_ceres")].state(),
        mush_sim::relay::LinkState::Degraded
    );
    r7_damage(&mut rig, node, "power", 200);
    assert_eq!(
        rig.registry().pairs[&cid("luna_ceres")].state(),
        mush_sim::relay::LinkState::Live
    );
    // A Standard message is genuinely queued when structure is destroyed.
    rig.run(GOD, "+relay/grant luna_ceres/#1=standard");
    rig.run(GOD, &format!("+order {bob}=r7 queued order"));
    let order = rig.last_envelope().id();
    assert!(
        rig.envelope(order)
            .transit()
            .unwrap()
            .legs
            .iter()
            .any(|l| l.pair.is_some())
    );
    r7_damage(&mut rig, node, "core", 0);
    let loss = rig.registry().losses[&cid("ceres")].clone();
    assert_eq!(loss.responsible, Some(GOD));
    assert!(loss.canon.is_some());
    assert!(
        rig.run(GOD, "@relay/show luna_ceres")
            .contains("orphaned=true")
    );
    assert!(
        rig.run(GOD, "+relay/status luna_ceres")
            .contains("permanent Exchange anathema")
    );
    rig.run(
        GOD,
        "+relay/policy luna_ceres/#1=served,operator.test,cure cannot revoke anathema",
    );
    assert!(
        rig.run(GOD, "+relay/status luna_ceres")
            .contains("Suspended")
    );
    assert!(matches!(
        mush_campaign::eligible(rig.tw.world(), &cfg, GOD, Some(&grant), now),
        Err(RecoveryRefusal::Cooldown { .. })
    ));
    assert!(mush_sim::relay::cover_voided(rig.tw.world(), GOD));
    rig.pass();
    assert!(
        rig.envelope(order)
            .transit()
            .unwrap()
            .legs
            .iter()
            .all(|l| l.pair.is_none())
    );
    let due = rig.envelope(order).transit().unwrap().flight.clone();
    for _ in 0..2 {
        r7_damage(&mut rig, node, "core", 1000);
        r7_damage(&mut rig, node, "core", 0);
    }
    r7_damage(&mut rig, node, "core", 1000);
    assert_eq!(rig.registry().losses[&cid("ceres")], loss);
    let state = rig
        .tw
        .world()
        .module_state::<mush_faction::FactionState>()
        .unwrap();
    let faction = state.faction_by_name("R7Cell").unwrap();
    assert_eq!(faction.resources["attention"], 1000);
    assert_eq!(faction.resources["legitimacy"], 4);
    assert!(
        state
            .factions
            .values()
            .all(|f| f.relay_condemnations.len() == 1)
    );
    assert_eq!(
        rig.tw
            .world()
            .module_state::<mush_faction::SimState>()
            .unwrap()
            .clocks[&clock]
            .filled,
        1
    );
    rig.flush().await;
    rig.restart().await.unwrap();
    assert_eq!(rig.registry().losses[&cid("ceres")], loss);
    assert_eq!(
        rig.registry().pairs[&cid("luna_ceres")].state(),
        mush_sim::relay::LinkState::Dead
    );
    assert_eq!(rig.envelope(order).transit().unwrap().flight, due);
    assert!(
        rig.run(GOD, "+relay/status luna_ceres")
            .contains("Suspended")
    );
}

#[tokio::test]
async fn r7_custody_void_and_checked_return_use_the_same_loss_writer() {
    let mut rig = Rig::new(true).await;
    let node = rig.registry().nodes[&cid("malapert")].object;
    let mut out = mush_core::Outbox::new();
    {
        let engine = rig.tw.engine_mut();
        let mut ctx = mush_core::Ctx {
            w: &mut engine.world,
            actor: GOD,
            switches: Default::default(),
            args: Default::default(),
            out: &mut out,
            reg: &engine.registry,
            module: Some("tactics".into()),
            typed_state: mush_core::ModuleActionState::unavailable(),
        };
        let mark = mush_sim::deployed::DeployedMark {
            match_id: 777,
            arena: "test.custody".into(),
            field: None,
        };
        mush_tactics::custody::check_out(&mut ctx, node, mark.clone()).unwrap();
        let back = mush_tactics::custody::check_in(&mut ctx, node, None).unwrap();
        assert!(back.restored);
        assert!(
            ctx.w
                .module_state::<RelayStore>()
                .unwrap()
                .registry()
                .unwrap()
                .losses
                .is_empty()
        );
        mush_tactics::custody::check_out(&mut ctx, node, mark).unwrap();
        mush_tactics::custody::check_in(&mut ctx, node, Some(&r7_plan("core", 0))).unwrap();
    }
    let ops = out
        .take_deferred()
        .into_iter()
        .filter_map(|d| match d {
            mush_core::Deferred::Persist(op) => Some(op),
            _ => None,
        })
        .collect();
    rig.store
        .write(WriteBatch {
            serial: rig.serial,
            durable: true,
            module_ops: ops,
            ..Default::default()
        })
        .await
        .unwrap();
    rig.serial += 1;
    let loss = rig.registry().losses[&cid("malapert")].clone();
    assert_eq!(
        loss.responsible, None,
        "arena ownership does not establish an attacker"
    );
    assert!(!mush_sim::relay::cover_voided(rig.tw.world(), GOD));
    assert!(loss.canon.is_some());
    rig.flush().await;
    rig.restart().await.unwrap();
    assert_eq!(rig.registry().losses[&cid("malapert")], loss);
    assert_eq!(
        rig.registry().pairs[&cid("luna_ceres")].state(),
        mush_sim::relay::LinkState::Dead
    );
}

#[test]
fn r7_station_pack_digest_and_bounded_attention() {
    let pack = tacsim::pack::load_pack(include_str!(
        "../../../tacsim/content/units/relay_station.json"
    ))
    .unwrap();
    assert_eq!(
        pack.manifest().digest().to_hex(),
        "9db7f4ad577e1071c124369289f9fdd6d5ff395b78160b9362c4718e29a7827e"
    );
    let mut f = mush_faction::Faction::new(mush_faction::FactionId(0), "Controlled attention");
    f.apply_resource("attention", i64::MAX, "test", GOD, 0);
    assert_eq!(f.resources["attention"], 1000);
    assert_eq!(f.resource_log.last().unwrap().delta, 1000);
    f.apply_resource("attention", i64::MIN, "test", GOD, 0);
    assert_eq!(f.resources["attention"], 0);
    assert_eq!(f.resource_log.last().unwrap().delta, -1000);
}

#[tokio::test]
async fn r7_missing_migrated_structure_refuses_boot_and_old_seed_migrates_once() {
    let mut rig = Rig::new(true).await;
    let node = rig.registry().nodes[&cid("ceres")].object;
    rig.run(GOD, &format!("@unit/remove {node}"));
    assert!(mush_tactics::module::unit_of(rig.tw.world(), node).is_none());
    rig.flush().await;
    let error = rig.restart().await.unwrap_err();
    assert!(
        error.contains("migrated structure missing or held"),
        "{error}"
    );
    // Model the R4 shape: no unit/binding and no R7 migration latch.
    let mut doc = rig
        .store
        .module_json_get("sim", KEY)
        .await
        .unwrap()
        .unwrap();
    doc.as_object_mut().unwrap().remove("structural_version");
    rig.replace_document(doc).await;
    rig.restart().await.unwrap();
    assert!(mush_tactics::module::unit_of(rig.tw.world(), node).is_some());
    assert_eq!(rig.registry().structural_version, 1);
    let before = mush_tactics::module::unit_of(rig.tw.world(), node)
        .unwrap()
        .clone();
    rig.flush().await;
    rig.restart().await.unwrap();
    assert_eq!(
        mush_tactics::module::unit_of(rig.tw.world(), node).unwrap(),
        &before
    );
}

impl Rig {
    fn retail(&self) -> mush_propagation::commerce::Market {
        mush_propagation::commerce::current(self.tw.world())
            .unwrap()
            .clone()
    }
    fn account(&self, id: u64) -> mush_economy::Account {
        self.tw
            .world()
            .module_state::<mush_economy::EconomyStore>()
            .unwrap()
            .snapshot()
            .account(mush_economy::AccountId(id))
            .unwrap()
            .clone()
    }
    fn money_total(&self) -> i128 {
        self.tw
            .world()
            .module_state::<mush_economy::EconomyStore>()
            .unwrap()
            .snapshot()
            .accounts()
            .filter(|a| a.instrument.as_str() == "tacsim.credit")
            .map(|a| a.posted_balance.widen())
            .sum()
    }
    fn retail_ready(&mut self) {
        for _ in 0..20 {
            if self.retail().capital_transaction.is_some() {
                return;
            }
            self.mail_tick();
        }
        panic!(
            "Exchange capitalization did not settle: {:?}",
            self.retail()
        );
    }
    fn retail_sender(&mut self, name: &str, fund: bool) -> (mush_core::ObjId, u64) {
        let who = self.tw.spawn_player(name);
        let luna = self.registry().nodes[&cid("malapert")].coverage[0];
        self.run(GOD, &format!("@teleport {who}={luna}"));
        self.run(GOD, &format!("@account/open {who}=tacsim.credit"));
        let account = self
            .tw
            .world()
            .module_state::<mush_economy::EconomyStore>()
            .unwrap()
            .snapshot()
            .accounts_of(&mush_org::PrincipalId::Object(who))
            .next()
            .unwrap()
            .id
            .0;
        if fund {
            let reply = self.run(GOD, &format!("@econ/pay 1={account} 10000"));
            assert_eq!(
                self.account(account).posted_balance.minor(),
                10_000,
                "{reply}"
            );
        }
        (who, account)
    }
    fn retail_mail(
        &mut self,
        who: mush_core::ObjId,
        to: mush_core::ObjId,
        body: &str,
    ) -> (u64, u64) {
        assert!(
            self.run(who, &format!("@mail/quick {to}=Paid/{body}"))
                .contains("queued")
        );
        let mail = self.mail().next_id() - 1;
        for _ in 0..5 {
            self.mail_tick();
            if let Some(e) = self.mail_route(mail, to, false) {
                return (mail, e.id());
            }
        }
        panic!("mail not accepted");
    }
    fn retail_quote(
        &mut self,
        who: mush_core::ObjId,
        envelope: u64,
        class: &str,
        payer: u64,
    ) -> u64 {
        let id = self.retail().next;
        let reply = self.run(who, &format!("+relay/quote {envelope}={class},{payer}"));
        assert!(reply.contains(&format!("Quote {id}:")), "{reply}");
        id
    }
}
#[tokio::test]
async fn r8a_purchase_reservation_atomic_transmission_restart_and_conservation() {
    use mush_propagation::commerce::Phase;
    let mut rig = Rig::new(true).await;
    let bob = rig.endpoints();
    rig.retail_ready();
    let (alice, payer) = rig.retail_sender("RetailAlice", true);
    let (mail, envelope) = rig.retail_mail(alice, bob, &"x".repeat(490));
    assert!(rig.run(alice, "+relay").contains("free Deferred"));
    assert!(
        rig.run(alice, "+relay/outbox")
            .contains(&format!("Envelope {envelope}"))
    );
    let quote = rig.retail_quote(alice, envelope, "priority", payer);
    let p = rig.retail().purchases[&quote].clone();
    assert_eq!(p.units, 4);
    assert_eq!(p.total_minor, 2000);
    let before = (rig.account(payer), rig.account(p.payee));
    let total = rig.money_total();
    rig.flush().await;
    rig.restart().await.unwrap();
    let reply = rig.run(alice, &format!("+relay/buy {quote}"));
    assert!(
        reply.contains("Reserved"),
        "{reply}; account={:?}",
        rig.account(payer)
    );
    assert_eq!(rig.account(payer).posted_balance, before.0.posted_balance);
    assert_eq!(rig.account(payer).held_amount.minor(), 2000);
    let p = rig.retail().purchases[&quote].clone();
    let hold = p.hold.unwrap();
    rig.flush_money(quote, "reserved").await;
    rig.run(alice, &format!("@hold/release {hold}"));
    assert_eq!(rig.account(payer).held_amount.minor(), 2000);
    rig.run(alice, &format!("+relay/buy {quote}"));
    assert_eq!(rig.account(payer).held_amount.minor(), 2000);
    rig.flush().await;
    rig.restart().await.unwrap();
    assert_eq!(rig.retail().purchases[&quote], p);
    // Staff pass is the second production ingress; it must settle the same money.
    for _ in 0..12 {
        rig.pass();
        if rig.retail().purchases[&quote].phase == Phase::Consumed {
            break;
        }
    }
    assert_eq!(rig.retail().purchases[&quote].phase, Phase::Consumed);
    assert_eq!(rig.account(payer).held_amount.minor(), 0);
    assert_eq!(
        rig.account(payer).posted_balance.minor(),
        before.0.posted_balance.minor() - 2000
    );
    assert_eq!(
        rig.account(p.payee).posted_balance.minor(),
        before.1.posted_balance.minor() + 2000
    );
    assert_eq!(rig.money_total(), total);
    let purchased = rig.retail().purchases[&quote].clone();
    rig.flush_money(quote, "consumed").await;
    rig.restart().await.unwrap();
    rig.run(alice, &format!("+relay/buy {quote}"));
    rig.pass();
    assert_eq!(rig.retail().purchases[&quote], purchased);
    assert!(
        rig.run(alice, &format!("+relay/cancel {quote}"))
            .contains("no refund")
    );
    rig.finish_mail(mail);
    assert!(rig.mail().entry(bob, mail).is_some());
    assert_eq!(rig.money_total(), total);
    assert!(rig.tw.world().refused_namespace_writes().is_empty());
}
#[tokio::test]
async fn r8a_cancel_expiry_no_funds_and_no_payer_alias_bypass() {
    use mush_propagation::commerce::Phase;
    let mut rig = Rig::new(true).await;
    let bob = rig.endpoints();
    rig.retail_ready();
    let (alice, payer) = rig.retail_sender("Buyer", true);
    let (poor, empty) = rig.retail_sender("PoorBuyer", false);
    let (_, ep) = rig.retail_mail(poor, bob, "no funds");
    assert!(
        rig.run(poor, &format!("+relay/quote {ep}=priority,{payer}"))
            .contains("No usable payer")
    );
    let q = rig.retail_quote(poor, ep, "priority", empty);
    let reply = rig.run(poor, &format!("+relay/buy {q}"));
    assert!(reply.contains("Reservation refused"), "{reply}");
    assert_eq!(rig.retail().purchases[&q].phase, Phase::Quoted);
    assert_eq!(rig.account(empty).held_amount.minor(), 0);
    let (_, env) = rig.retail_mail(alice, bob, "cancel and upgrade");
    let q = rig.retail_quote(alice, env, "standard", payer);
    assert!(
        rig.run(alice, &format!("+relay/buy {q}"))
            .contains("Reserved")
    );
    let before = rig.account(payer).posted_balance;
    rig.run(alice, &format!("+relay/cancel {q}"));
    assert_eq!(rig.retail().purchases[&q].phase, Phase::Released);
    assert_eq!(rig.account(payer).held_amount.minor(), 0);
    let upgrade = rig.retail_quote(alice, env, "priority", payer);
    rig.run(alice, &format!("+relay/buy {upgrade}"));
    let p = rig.retail().purchases[&upgrade].clone();
    assert!(p.total_minor > rig.retail().purchases[&q].total_minor);
    // Receiving endpoint denial wins over a solvent buyer's purchased Priority.
    rig.run(
        GOD,
        &format!("+relay/policy luna_ceres/{bob}=suspended,operator.billingtest,receiver denied"),
    );
    rig.flush().await;
    rig.restart().await.unwrap();
    rig.advance_ic_to(p.service_until);
    rig.pass();
    assert_eq!(rig.retail().purchases[&upgrade].phase, Phase::Released);
    assert_eq!(rig.account(payer).held_amount.minor(), 0);
    assert_eq!(rig.account(payer).posted_balance, before);
    rig.run(alice, &format!("+relay/cancel {upgrade}"));
    assert_eq!(rig.account(payer).held_amount.minor(), 0);
}

#[tokio::test]
async fn r8a_standard_fanout_prices_real_utf8_content_and_default_deferred_stays_free() {
    use mush_propagation::{commerce::Phase, service::Class};
    let mut rig = Rig::new(true).await;
    let bob = rig.endpoints();
    rig.retail_ready();
    let (alice, payer) = rig.retail_sender("FanoutBuyer", true);
    let carol = rig.tw.spawn_player("RetailCarol");
    let room = rig.registry().nodes[&cid("ceres")].coverage[0];
    rig.run(GOD, &format!("@teleport {carol}={room}"));
    rig.run(GOD, &format!("+relay/grant luna_ceres/{carol}=priority"));
    assert!(
        rig.run(
            alice,
            &format!("@mail/quick {bob},{carol}=Paid/{}", "é".repeat(65))
        )
        .contains("queued")
    );
    let mail = rig.mail().next_id() - 1;
    rig.mail_tick();
    let copies: Vec<_> = [bob, carol]
        .into_iter()
        .map(|who| rig.mail_route(mail, who, false).unwrap())
        .collect();
    assert!(
        copies
            .iter()
            .all(|e| e.dispatch_ref().unwrap().bytes == 134)
    );
    let before = (
        rig.account(payer).posted_balance,
        rig.account(rig.retail().payee).posted_balance,
    );
    let total = rig.money_total();
    let mut quotes = vec![];
    for e in &copies {
        let q = rig.retail_quote(alice, e.id(), "standard", payer);
        let reply = rig.run(alice, &format!("+relay/buy {q}"));
        assert!(reply.contains("Reserved"), "{reply}");
        quotes.push(q);
    }
    assert_eq!(rig.account(payer).held_amount.minor(), 400);
    for _ in 0..4 {
        rig.pass();
    }
    for q in &quotes {
        assert_eq!(rig.retail().purchases[q].phase, Phase::Reserved);
    }
    let due = copies
        .iter()
        .map(|e| {
            rig.envelope(e.id())
                .transit()
                .unwrap()
                .service
                .as_ref()
                .unwrap()
                .eligible_at
        })
        .max()
        .unwrap();
    rig.advance_ic_to(due);
    for _ in 0..12 {
        rig.mail_tick();
        if quotes
            .iter()
            .all(|q| rig.retail().purchases[q].phase == Phase::Consumed)
        {
            break;
        }
    }
    assert!(
        quotes
            .iter()
            .all(|q| rig.retail().purchases[q].phase == Phase::Consumed)
    );
    assert_eq!(
        rig.account(payer).posted_balance.minor(),
        before.0.minor() - 400
    );
    assert_eq!(
        rig.account(rig.retail().payee).posted_balance.minor(),
        before.1.minor() + 400
    );
    assert_eq!(rig.money_total(), total);
    let (free, _) = rig.retail_mail(alice, bob, "free copy");
    let e = rig.mail_route(free, bob, false).unwrap();
    assert_eq!(
        e.transit().unwrap().service.as_ref().unwrap().class,
        Class::Deferred
    );
    let policy = rig
        .tw
        .world()
        .module_state::<mush_propagation::PropagationStore>()
        .unwrap()
        .relay_transport()
        .service
        .as_ref()
        .unwrap()
        .config
        .clone();
    let earliest = e.transit().unwrap().service.as_ref().unwrap().eligible_at;
    let phase = earliest.rem_euclid(policy.window_period_s);
    let window = if policy.window_open(earliest) {
        earliest
    } else {
        earliest + policy.window_period_s - phase + policy.window_start_s
    };
    rig.advance_ic_to(window);
    for _ in 0..12 {
        rig.mail_tick();
        if rig.mail().entry(bob, free).is_some() {
            break;
        }
    }
    assert!(rig.mail().entry(bob, free).is_some());
    assert_eq!(
        rig.account(payer).posted_balance.minor(),
        before.0.minor() - 400
    );
}

#[tokio::test]
async fn r8a_pre_retail_upgrade_preserves_accepted_terms_and_missing_market_refuses_boot() {
    use mush_propagation::service::Class;
    let mut rig = Rig::new(true).await;
    let bob = rig.endpoints();
    let market = rig.retail();
    assert!(market.capital_transaction.is_none());
    assert_eq!(rig.account(market.payee).posted_balance.minor(), 0);
    assert!(
        rig.run(GOD, &format!("+order {bob}=pre-retail"))
            .contains("Queued revision")
    );
    let old = rig.last_envelope().id();
    rig.flush().await;
    let mut transport = rig
        .store
        .module_json_get("propagation", "relay_transport")
        .await
        .unwrap()
        .unwrap();
    transport["service"]
        .as_object_mut()
        .unwrap()
        .remove("commerce");
    transport["service"]["grants"] = serde_json::json!([]);
    // No capitalization, holds or payments occurred. Remove only the zero-value
    // R8 account to model the real pre-retail schema; never delete funded money.
    rig.store
        .write(WriteBatch {
            serial: rig.serial,
            durable: true,
            module_ops: vec![
                ModuleOp {
                    ns: "economy".into(),
                    op: ModuleWrite::Del {
                        key: format!("account:{}", market.payee).into(),
                    },
                },
                ModuleOp {
                    ns: "propagation".into(),
                    op: ModuleWrite::JsonSet {
                        key: "relay_transport".into(),
                        value: transport,
                    },
                },
            ],
            ..Default::default()
        })
        .await
        .unwrap();
    rig.serial += 1;
    rig.restart().await.unwrap();
    assert!(rig.retail().legacy_before > old);
    rig.pass();
    assert_eq!(
        rig.envelope(old)
            .transit()
            .unwrap()
            .service
            .as_ref()
            .unwrap()
            .class,
        Class::Standard
    );
    assert!(
        rig.run(GOD, &format!("+order {bob}=post-retail"))
            .contains("Queued revision")
    );
    let new = rig.last_envelope().id();
    rig.pass();
    assert_eq!(
        rig.envelope(new)
            .transit()
            .unwrap()
            .service
            .as_ref()
            .unwrap()
            .class,
        Class::Deferred
    );
    rig.retail_ready();
    let funded = rig.account(rig.retail().payee).posted_balance;
    rig.flush().await;
    rig.restart().await.unwrap();
    assert_eq!(rig.account(rig.retail().payee).posted_balance, funded);
    let mut broken = rig
        .store
        .module_json_get("propagation", "relay_transport")
        .await
        .unwrap()
        .unwrap();
    broken["service"]
        .as_object_mut()
        .unwrap()
        .remove("commerce");
    rig.store
        .write(WriteBatch {
            serial: rig.serial,
            durable: true,
            module_ops: vec![ModuleOp {
                ns: "propagation".into(),
                op: ModuleWrite::JsonSet {
                    key: "relay_transport".into(),
                    value: broken,
                },
            }],
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(
        rig.restart()
            .await
            .unwrap_err()
            .contains("without its market")
    );
}

#[tokio::test]
async fn r8a_orphaned_reservation_and_forged_quote_refuse_boot() {
    let mut rig = Rig::new(true).await;
    let bob = rig.endpoints();
    rig.retail_ready();
    let (alice, payer) = rig.retail_sender("AuditBuyer", true);
    let (_, env) = rig.retail_mail(alice, bob, "audit");
    let q = rig.retail_quote(alice, env, "priority", payer);
    rig.run(alice, &format!("+relay/buy {q}"));
    rig.flush().await;
    let original = rig
        .store
        .module_json_get("propagation", "relay_transport")
        .await
        .unwrap()
        .unwrap();
    for missing in [true, false] {
        let mut broken = original.clone();
        if missing {
            broken["service"]["commerce"]["purchases"]
                .as_object_mut()
                .unwrap()
                .remove(&q.to_string());
        } else {
            broken["service"]["commerce"]["purchases"][q.to_string()]["total_minor"] =
                serde_json::json!(1);
        }
        rig.store
            .write(WriteBatch {
                serial: rig.serial,
                durable: true,
                module_ops: vec![ModuleOp {
                    ns: "propagation".into(),
                    op: ModuleWrite::JsonSet {
                        key: "relay_transport".into(),
                        value: broken,
                    },
                }],
                ..Default::default()
            })
            .await
            .unwrap();
        rig.serial += 1;
        let error = rig.restart().await.unwrap_err();
        assert!(
            error.contains(if missing {
                "orphaned relay reservation"
            } else {
                "invalid relay purchases"
            }),
            "{error}"
        );
    }
    rig.store
        .write(WriteBatch {
            serial: rig.serial,
            durable: true,
            module_ops: vec![ModuleOp {
                ns: "propagation".into(),
                op: ModuleWrite::JsonSet {
                    key: "relay_transport".into(),
                    value: original,
                },
            }],
            ..Default::default()
        })
        .await
        .unwrap();
    rig.serial += 1;
    rig.restart().await.unwrap();
    assert_eq!(rig.account(payer).held_amount.minor(), 500);
    rig.run(alice, &format!("+relay/cancel {q}"));
    assert_eq!(rig.account(payer).held_amount.minor(), 0);
}

#[tokio::test]
async fn r8a_failed_capture_keeps_money_capacity_receipt_and_message_unspent() {
    use mush_propagation::commerce::Phase;
    let mut rig = Rig::new(true).await;
    let bob = rig.endpoints();
    rig.retail_ready();
    let (alice, payer) = rig.retail_sender("FrozenPayeeBuyer", true);
    let (_, env) = rig.retail_mail(alice, bob, "unspent");
    let q = rig.retail_quote(alice, env, "priority", payer);
    rig.run(alice, &format!("+relay/buy {q}"));
    rig.flush().await;
    let payee = rig.retail().payee;
    let key = format!("account:{payee}");
    let mut account = rig
        .store
        .module_json_get("economy", &key)
        .await
        .unwrap()
        .unwrap();
    // Controlled fault in the account's documented status authority, not money.
    account["status"] = serde_json::json!("Frozen");
    rig.store
        .write(WriteBatch {
            serial: rig.serial,
            durable: true,
            module_ops: vec![ModuleOp {
                ns: "economy".into(),
                op: ModuleWrite::JsonSet {
                    key: key.into(),
                    value: account,
                },
            }],
            ..Default::default()
        })
        .await
        .unwrap();
    rig.serial += 1;
    rig.restart().await.unwrap();
    let total = rig.money_total();
    for _ in 0..4 {
        rig.pass();
    }
    assert_eq!(rig.retail().purchases[&q].phase, Phase::Reserved);
    assert_eq!(rig.account(payer).held_amount.minor(), 500);
    assert!(rig.envelope(env).path().is_empty());
    assert!(
        !rig.tw
            .world()
            .module_state::<mush_propagation::PropagationStore>()
            .unwrap()
            .relay_transport()
            .service
            .as_ref()
            .unwrap()
            .receipts
            .iter()
            .any(|r| r.envelope == env)
    );
    assert_eq!(rig.money_total(), total);
    let result = rig.run(GOD, &format!("@account/unfreeze {payee}"));
    assert!(result.contains("unfrozen"), "{result}");
    for _ in 0..8 {
        rig.pass();
    }
    assert_eq!(rig.retail().purchases[&q].phase, Phase::Consumed);
    assert_eq!(rig.account(payer).held_amount.minor(), 0);
    assert_eq!(rig.money_total(), total);
}

#[tokio::test]
async fn r8a_expired_hold_waiting_in_release_backlog_cannot_be_captured() {
    use mush_propagation::commerce::Phase;
    let mut rig = Rig::new(true).await;
    let bob = rig.endpoints();
    rig.retail_ready();
    let (alice, payer) = rig.retail_sender("ExpiryBacklogBuyer", true);
    let (_, env) = rig.retail_mail(alice, bob, "expiry backlog");
    // The first eight expired quotes fill this pass's bounded release budget.
    for _ in 0..mush_propagation::commerce::MAX_SETTLEMENTS {
        rig.retail_quote(alice, env, "standard", payer);
    }
    let paid = rig.retail_quote(alice, env, "priority", payer);
    rig.run(alice, &format!("+relay/buy {paid}"));
    let p = rig.retail().purchases[&paid].clone();
    rig.advance_ic_to(p.service_until);
    let before = rig.account(payer).posted_balance;
    rig.pass();
    assert_eq!(rig.retail().purchases[&paid].phase, Phase::Reserved);
    assert!(rig.envelope(env).path().is_empty());
    assert_eq!(rig.account(payer).posted_balance, before);
    rig.pass();
    assert_eq!(rig.retail().purchases[&paid].phase, Phase::Released);
    assert_eq!(rig.account(payer).held_amount.minor(), 0);
    assert_eq!(rig.account(payer).posted_balance, before);
}

#[tokio::test]
async fn r8a_two_exchange_hops_use_one_purchase_and_survive_restart_between_hops() {
    use mush_propagation::commerce::Phase;
    let mut rig = Rig::new(true).await;
    let bob = rig.endpoints();
    rig.retail_ready();
    let catalog = rig.registry();
    let phobos = catalog.nodes[&cid("phobos")].place;
    let ceres = catalog.nodes[&cid("ceres")].coverage[0];
    rig.run(GOD, &format!("@teleport {bob}={phobos}"));
    rig.run(GOD, &format!("@prop/jam {phobos}=100"));
    let (alice, payer) = rig.retail_sender("TwoHopBuyer", true);
    rig.run(GOD, &format!("@teleport {alice}={ceres}"));
    let (mail, env) = rig.retail_mail(alice, bob, "two hops");
    let q = rig.retail_quote(alice, env, "priority", payer);
    rig.run(alice, &format!("+relay/buy {q}"));
    for _ in 0..12 {
        rig.pass();
    }
    assert_eq!(rig.retail().purchases[&q].phase, Phase::Consumed);
    let purchase = rig.retail().purchases[&q].clone();
    assert_eq!(rig.envelope(env).path().len(), 1);
    let balances = (
        rig.account(payer).posted_balance,
        rig.account(purchase.payee).posted_balance,
    );
    rig.flush().await;
    rig.restart().await.unwrap();
    rig.run(GOD, &format!("@prop/jam {phobos}=0"));
    // The outbound purchased copy arrives now. Its unpaid return notice is
    // independently Deferred and is not part of this two-hop charge assertion.
    for _ in 0..24 {
        rig.mail_tick();
        if rig.mail().entry(bob, mail).is_some() {
            break;
        }
    }
    assert!(
        rig.mail().entry(bob, mail).is_some(),
        "{:?}",
        rig.envelope(env)
    );
    assert_eq!(rig.retail().purchases[&q], purchase);
    assert_eq!(
        (
            rig.account(payer).posted_balance,
            rig.account(purchase.payee).posted_balance
        ),
        balances
    );
    let book = rig
        .tw
        .world()
        .module_state::<mush_propagation::PropagationStore>()
        .unwrap();
    let receipts: Vec<_> = book
        .relay_transport()
        .service
        .as_ref()
        .unwrap()
        .receipts
        .iter()
        .filter(|r| r.envelope == env)
        .collect();
    assert_eq!(receipts.len(), 2);
    assert!(
        receipts
            .iter()
            .all(|r| r.ticket.class == mush_propagation::service::Class::Priority)
    );
}

impl Rig {
    fn billing(&self) -> mush_propagation::billing::Book {
        mush_propagation::billing::current(self.tw.world())
            .unwrap()
            .clone()
    }
    fn subscription_obligation(&self, id: u64) -> mush_economy::Obligation {
        let oid = self.billing().subscriptions[&id].obligation.unwrap();
        self.tw
            .world()
            .module_state::<mush_economy::EconomyStore>()
            .unwrap()
            .snapshot()
            .obligation(mush_economy::ObligationId(oid))
            .unwrap()
            .clone()
    }
    fn subscribe(
        &mut self,
        actor: mush_core::ObjId,
        principal: &str,
        class: &str,
        payer: u64,
    ) -> u64 {
        let id = self.billing().next;
        let reply = self.run(
            actor,
            &format!("+relay/subscribe {principal}={class},{payer}"),
        );
        assert!(reply.contains(&format!("Subscription {id} for")), "{reply}");
        let reply = self.run(actor, &format!("+relay/enroll {id}"));
        assert!(reply.contains("Active"), "{reply}");
        id
    }
    fn bill(&mut self) {
        let now = self.tw.world().now() * 1000;
        self.tw.engine_mut().sched.insert(
            now,
            mush_core::ScheduledKind::ModuleTick {
                module: "economy".into(),
                tag: "economy.sweep".into(),
            },
        );
        self.tw.fire_due(now);
        self.pass();
    }
    fn next_bill(&mut self, id: u64) {
        let due = self.subscription_obligation(id).schedule.next_due;
        self.advance_ic_to(due);
        self.bill();
    }
    fn receipt_class(&self, env: u64) -> Option<mush_propagation::service::Class> {
        self.tw
            .world()
            .module_state::<mush_propagation::PropagationStore>()
            .unwrap()
            .relay_transport()
            .service
            .as_ref()
            .unwrap()
            .receipts
            .iter()
            .find(|r| r.envelope == env)
            .map(|r| r.ticket.class)
    }
    async fn flush_subscription(&mut self, id: u64) {
        assert!(
            self.tw
                .engine_mut()
                .take_durable_gate()
                .is_some_and(|(_, strict)| strict)
        );
        let dirty = self.tw.world_mut().take_dirty();
        let mut batch = persist::batch_from_dirty(self.tw.world(), dirty, self.serial, true);
        batch.module_ops = self.tw.take_module_ops();
        assert!(batch.module_ops.iter().any(|o| o.ns == "economy"));
        assert!(batch.module_ops.iter().any(|o|o.ns=="propagation" && matches!(&o.op,ModuleWrite::JsonSet{key,value} if key=="relay_transport" && value["service"]["billing"]["subscriptions"][id.to_string()]["obligation"].is_u64())));
        self.store.write(batch).await.unwrap();
        self.serial += 1;
    }
}
#[tokio::test]
async fn r8b_person_subscription_real_bills_queued_delivery_arrears_cure_restart_and_sanctions() {
    use mush_propagation::service::Class;
    let mut rig = Rig::new(true).await;
    let bob = rig.endpoints();
    rig.retail_ready();
    let (alice, payer) = rig.retail_sender("RecurringAlice", true);
    let payee = rig.retail().payee;
    let capital = rig.account(payee).posted_balance.minor();
    let id = rig.subscribe(alice, &alice.to_string(), "priority", payer);
    assert!(rig.run(alice, "+relay").contains("your class Deferred"));
    rig.flush_subscription(id).await;
    rig.restart().await.unwrap();
    rig.bill();
    assert_eq!(rig.subscription_obligation(id).settled_total, 1);
    assert_eq!(rig.account(payer).posted_balance.minor(), 0);
    assert_eq!(rig.account(payee).posted_balance.minor(), capital + 10000);
    assert!(rig.run(alice, "+relay").contains("your class Priority"));
    let (_, env) = rig.retail_mail(alice, bob, "subscription paid crossing");
    for _ in 0..8 {
        rig.pass();
    }
    assert_eq!(rig.receipt_class(env), Some(Class::Priority));
    rig.flush().await;
    rig.restart().await.unwrap();
    for _ in 0..3 {
        rig.bill();
    }
    assert_eq!(rig.subscription_obligation(id).settled_total, 1);
    assert_eq!(rig.account(payee).posted_balance.minor(), capital + 10000);
    rig.next_bill(id);
    rig.pass();
    let terms = tacsim::relay_world::current(rig.tw.world()).unwrap();
    assert_eq!(terms.progress(), 2);
    assert_eq!(
        terms.facts[0].restriction.principal,
        mush_propagation::service::Principal::Object(alice)
    );

    assert!(
        rig.run(alice, "+relay/status luna_ceres")
            .contains("Throttled")
    );
    let (_, waiting) = rig.retail_mail(alice, bob, "held by a missed bill");
    rig.pass();
    assert_eq!(
        rig.envelope(waiting)
            .transit()
            .unwrap()
            .service
            .as_ref()
            .unwrap()
            .class,
        Class::Standard
    );
    rig.next_bill(id);
    rig.pass();
    assert_eq!(
        tacsim::relay_world::current(rig.tw.world())
            .unwrap()
            .progress(),
        4
    );

    assert!(
        rig.run(alice, "+relay/status luna_ceres")
            .contains("Suspended")
    );
    rig.flush().await;
    rig.restart().await.unwrap();
    rig.run(
        GOD,
        &format!("+relay/policy luna_ceres/{alice}=suspended,operator.incident,compound incident"),
    );
    assert!(
        rig.run(alice, &format!("+relay/cure {id}"))
            .contains("200.00 cr")
    );
    assert!(
        rig.run(bob, &format!("+relay/cure {id}=2,0"))
            .contains("No such represented")
    );
    assert!(
        rig.run(alice, &format!("+relay/cure {id}=2,0"))
            .contains("Cure refused")
    );
    rig.run(GOD, &format!("@econ/pay 1={payer} 30000"));
    assert!(
        rig.run(alice, &format!("+relay/cure {id}=2,0"))
            .contains("Arrears paid")
    );
    rig.flush_subscription(id).await;
    let cured = rig.subscription_obligation(id);
    rig.restart().await.unwrap();
    assert_eq!(rig.subscription_obligation(id), cured);
    assert_eq!(rig.account(payer).posted_balance.minor(), 10000);
    assert!(
        rig.run(alice, &format!("+relay/cure {id}=2,0"))
            .contains("Arrears paid")
    );
    assert_eq!(rig.account(payer).posted_balance.minor(), 10000);
    assert!(
        rig.run(alice, "+relay/status luna_ceres")
            .contains("operator.incident Suspended")
    );
    rig.run(
        GOD,
        &format!("+relay/policy luna_ceres/{alice}=served,operator.incident,resolved incident"),
    );
    assert!(
        rig.run(alice, "+relay/status luna_ceres")
            .contains("luna_ceres: Served")
    );
    assert!(rig.run(alice, "+relay").contains("your class Priority"));
    rig.next_bill(id);
    assert_eq!(rig.subscription_obligation(id).settled_total, 2);
    assert_eq!(rig.account(payer).posted_balance.minor(), 0);
    assert_eq!(rig.account(payee).posted_balance.minor(), capital + 40000);
}
#[tokio::test]
async fn r8b_later_payment_does_not_erase_old_debt_and_cancellation_keeps_arrears() {
    use mush_economy::ObligationStatus;
    let mut rig = Rig::new(true).await;
    rig.endpoints();
    rig.retail_ready();
    let (alice, payer) = rig.retail_sender("EarlyCure", false);
    let id = rig.subscribe(alice, &alice.to_string(), "standard", payer);
    rig.bill();
    assert!(rig.run(alice, "+relay").contains("your class Deferred"));
    assert_eq!(rig.subscription_obligation(id).missed_total, 1);
    rig.run(GOD, &format!("@econ/pay 1={payer} 5000"));
    rig.next_bill(id);
    assert_eq!(
        rig.subscription_obligation(id).status,
        ObligationStatus::Active
    );
    assert!(
        rig.run(alice, "+relay/status luna_ceres")
            .contains("Throttled")
    );
    assert!(
        rig.run(alice, &format!("+relay/cure {id}=1,0"))
            .contains("Arrears paid")
    );
    assert_eq!(rig.account(payer).posted_balance.minor(), 0);
    rig.next_bill(id);
    assert!(matches!(
        rig.subscription_obligation(id).status,
        ObligationStatus::Grace { missed: 1, .. }
    ));
    rig.run(alice, &format!("+relay/unsubscribe {id}"));
    assert_eq!(
        rig.subscription_obligation(id).status,
        ObligationStatus::Cancelled
    );
    assert!(
        rig.run(alice, &format!("+relay/subscribe {alice}=priority,{payer}"))
            .contains("existing subscription first")
    );
    assert!(
        rig.run(alice, &format!("+relay/cure {id}=1,0"))
            .contains("Cure refused")
    );
    rig.run(GOD, &format!("@econ/pay 1={payer} 2500"));
    assert!(
        rig.run(alice, &format!("+relay/cure {id}=2,1"))
            .contains("Arrears paid")
    );
    rig.flush_subscription(id).await;
    rig.restart().await.unwrap();
    assert_eq!(
        rig.subscription_obligation(id).status,
        ObligationStatus::Cancelled
    );
    assert!(rig.run(alice, "+relay").contains("your class Deferred"));
    let before = rig.account(payer).posted_balance;
    rig.advance_ic_to(rig.tw.world().world_ic_now().at + 3 * 86400);
    rig.bill();
    assert_eq!(rig.account(payer).posted_balance, before);
}
#[tokio::test]
async fn r8b_sovereign_real_institution_treaty_expires_before_billing_and_never_aliases_payer() {
    use mush_propagation::service::Class;
    let mut rig = Rig::new(true).await;
    let bob = rig.endpoints();
    rig.retail_ready();
    let (officer, personal) = rig.retail_sender("TreatyOfficer", true);
    let org = rig
        .tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .unwrap()
        .state()
        .orgs()
        .find(|o| o.name == "Freeport Registry")
        .unwrap()
        .id;
    let payer = rig
        .tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .unwrap()
        .snapshot()
        .accounts_of(&mush_org::PrincipalId::Organization(org))
        .next()
        .unwrap()
        .id
        .0;
    assert!(
        rig.run(
            officer,
            &format!("+relay/subscribe org:{org}=sovereign,{payer}")
        )
        .contains("No such represented")
    );
    rig.run(
        GOD,
        &format!("@org/grant {officer}={org} economy.account.post"),
    );
    rig.run(
        GOD,
        &format!("@org/grant {officer}={org} economy.account.view"),
    );
    assert!(
        rig.run(
            officer,
            &format!("+relay/subscribe org:{org}=sovereign,{payer}")
        )
        .contains("No treaty")
    );
    assert!(
        rig.run(
            officer,
            &format!("+relay/treaty org:{org}=2,government agreement")
        )
        .contains("Permission denied")
    );
    let exchange = rig.registry().operator;
    assert!(
        rig.run(
            GOD,
            &format!("+relay/treaty org:{exchange}=2,not a government")
        )
        .contains("No recognized")
    );
    assert!(
        rig.run(
            GOD,
            &format!("+relay/treaty org:{org}=2,Registry public service")
        )
        .contains("Treaty offered")
    );
    let id = rig.subscribe(officer, &format!("org:{org}"), "sovereign", payer);
    let before = rig.account(payer).posted_balance.minor();
    rig.bill();
    assert_eq!(rig.account(payer).posted_balance.minor(), before - 50000);
    // A member using an organization's payer remains a personal sender.
    assert!(rig.run(officer, "+relay").contains("your class Deferred"));
    rig.run(GOD, "@create RegistryDispatchUnit");
    let unit = rig
        .tw
        .world()
        .iter()
        .find(|o| o.name == "RegistryDispatchUnit")
        .unwrap()
        .id;
    let luna = rig.registry().nodes[&cid("malapert")].coverage[0];
    rig.run(GOD, &format!("@teleport {unit}={luna}"));
    assert!(
        rig.run(GOD, &format!("@asset/title {unit}=org:{org}"))
            .contains("Title recorded")
    );
    // Ordinary unit ingress can address a co-located recipient. The addressed
    // character then moves before the queued order is processed: normal forwarding
    // supplies the cross-system leg without a staff addressing bypass.
    rig.run(GOD, &format!("@teleport {bob}={luna}"));
    let reply = rig.run(unit, &format!("+order {bob}=treaty dispatch"));
    assert!(reply.contains("Queued"), "{reply}");
    let ceres = rig.registry().nodes[&cid("ceres")].coverage[0];
    rig.run(GOD, &format!("@teleport {bob}={ceres}"));
    let env = rig.last_envelope().id();
    for _ in 0..8 {
        rig.pass();
    }
    assert_eq!(rig.receipt_class(env), Some(Class::Sovereign));
    rig.flush().await;
    rig.restart().await.unwrap();
    let end = rig.billing().subscriptions[&id].until_ic.unwrap();
    rig.advance_ic_to(end);
    let before = rig.account(payer).posted_balance;
    rig.bill();
    rig.bill();
    assert_eq!(rig.account(payer).posted_balance, before);
    assert_eq!(
        rig.subscription_obligation(id).status,
        mush_economy::ObligationStatus::Cancelled
    );
    assert_eq!(
        mush_propagation::billing::class(rig.tw.world(), exchange.0, unit),
        None
    );
    assert_eq!(rig.account(personal).posted_balance.minor(), 10000);
    rig.flush().await;
    rig.restart().await.unwrap();
    rig.bill();
    assert_eq!(rig.account(payer).posted_balance, before);
}
#[tokio::test]
async fn r8b_cure_preserves_economic_suspension_and_rate_changes_do_not_rebill() {
    let mut rig = Rig::new(true).await;
    rig.endpoints();
    rig.retail_ready();
    let (alice, payer) = rig.retail_sender("PausedSubscriber", false);
    let id = rig.subscribe(alice, &alice.to_string(), "standard", payer);
    rig.bill();
    let oid = rig.subscription_obligation(id).id;
    rig.run(
        GOD,
        &format!("@obligation/suspend {oid}=independent account review"),
    );
    let held = rig.subscription_obligation(id).holders();
    assert!(!held.is_empty());
    rig.run(GOD, &format!("@econ/pay 1={payer} 2500"));
    let payee = rig.retail().payee;
    // Account freeze is a recovery fault, not a gameplay command. Change only
    // that documented status authority and re-enter through production boot.
    rig.flush().await;
    let key = format!("account:{payee}");
    let mut account = rig
        .store
        .module_json_get("economy", &key)
        .await
        .unwrap()
        .unwrap();
    account["status"] = serde_json::json!("Frozen");
    rig.store
        .write(WriteBatch {
            serial: rig.serial,
            durable: true,
            module_ops: vec![ModuleOp {
                ns: "economy".into(),
                op: ModuleWrite::JsonSet {
                    key: key.into(),
                    value: account,
                },
            }],
            ..Default::default()
        })
        .await
        .unwrap();
    rig.serial += 1;
    rig.restart().await.unwrap();
    assert!(
        rig.run(alice, &format!("+relay/cure {id}=1,0"))
            .contains("Cure refused")
    );
    assert_eq!(rig.account(payer).posted_balance.minor(), 2500);
    rig.run(GOD, &format!("@account/unfreeze {payee}"));
    assert!(
        rig.run(alice, &format!("+relay/cure {id}=1,0"))
            .contains("Arrears paid")
    );
    assert_eq!(rig.subscription_obligation(id).holders(), held);
    assert!(
        rig.run(alice, "+relay/status luna_ceres")
            .contains("Suspended")
    );
    rig.run(GOD, &format!("@obligation/resume {oid}"));
    assert!(
        rig.run(alice, "+relay/status luna_ceres")
            .contains("Served")
    );
    let before = rig.subscription_obligation(id);
    rig.run(GOD, "@tacsim/time rate=24");
    rig.bill();
    rig.run(GOD, "@tacsim/time rate=1");
    rig.bill();
    assert_eq!(
        rig.subscription_obligation(id).periods_settled,
        before.periods_settled
    );
    rig.flush().await;
    rig.restart().await.unwrap();
    assert_eq!(
        rig.subscription_obligation(id).service_terms,
        before.service_terms
    );
}
#[tokio::test]
async fn r8b_lost_subscription_owner_and_forged_terms_refuse_boot() {
    let mut rig = Rig::new(true).await;
    rig.endpoints();
    rig.retail_ready();
    let (alice, payer) = rig.retail_sender("BillingIntegrity", true);
    let id = rig.subscribe(alice, &alice.to_string(), "standard", payer);
    rig.bill();
    rig.flush_subscription(id).await;
    let key = "relay_transport";
    let original = rig
        .store
        .module_json_get("propagation", key)
        .await
        .unwrap()
        .unwrap();
    for forged in [false, true] {
        let mut doc = original.clone();
        if forged {
            doc["service"]["billing"]["subscriptions"][id.to_string()]["payer"] =
                serde_json::json!(1);
        } else {
            doc["service"]["billing"]["subscriptions"]
                .as_object_mut()
                .unwrap()
                .clear();
        }
        rig.store
            .write(WriteBatch {
                serial: rig.serial,
                durable: true,
                module_ops: vec![ModuleOp {
                    ns: "propagation".into(),
                    op: ModuleWrite::JsonSet {
                        key: key.into(),
                        value: doc,
                    },
                }],
                ..Default::default()
            })
            .await
            .unwrap();
        rig.serial += 1;
        let error = rig.restart().await.unwrap_err();
        assert!(
            error.contains(if forged {
                "obligation/terms mismatch"
            } else {
                "Orphaned relay subscription"
            }),
            "{error}"
        );
    }
    rig.store
        .write(WriteBatch {
            serial: rig.serial,
            durable: true,
            module_ops: vec![ModuleOp {
                ns: "propagation".into(),
                op: ModuleWrite::JsonSet {
                    key: key.into(),
                    value: original,
                },
            }],
            ..Default::default()
        })
        .await
        .unwrap();
    rig.serial += 1;
    rig.restart().await.unwrap();
    assert_eq!(rig.subscription_obligation(id).settled_total, 1);
}

impl Rig {
    fn public_hub(&mut self, enabled: bool) {
        for pair in ["luna_ceres", "luna_phobos", "luna_kirkwood", "luna_mercury"] {
            self.run(
                GOD,
                &format!(
                    "@relay/{} {pair}.luna",
                    if enabled { "enable" } else { "disable" }
                ),
            );
        }
    }
    async fn apply_content(&mut self, packs: Vec<mush_content::Pack>) {
        self.flush().await;
        let mut out = mush_core::Outbox::new();
        {
            let engine = self.tw.engine_mut();
            let mut ctx = mush_core::Ctx {
                w: &mut engine.world,
                actor: GOD,
                switches: Default::default(),
                args: Default::default(),
                out: &mut out,
                reg: &engine.registry,
                module: Some("content".into()),
                typed_state: mush_core::ModuleActionState::unavailable(),
            };
            for pack in packs {
                let (latch, _) = mush_content::apply_pack(&mut ctx, &pack).unwrap();
                assert!(
                    latch.anomaly().is_none(),
                    "{} {:?}",
                    pack.manifest().id(),
                    latch.anomaly()
                );
            }
        }
        let ops = out
            .take_deferred()
            .into_iter()
            .filter_map(|d| {
                if let mush_core::Deferred::Persist(op) = d {
                    Some(op)
                } else {
                    None
                }
            })
            .collect();
        let dirty = self.tw.world_mut().take_dirty();
        let mut batch = persist::batch_from_dirty(self.tw.world(), dirty, self.serial, true);
        batch.module_ops = ops;
        self.store.write(batch).await.unwrap();
        self.serial += 1;
    }
    async fn old_star() -> Self {
        let mut rig = Self::new(false).await;
        let mut packs = tacsim::composition::boot_pack_list(false, true).unwrap();
        // This historical Ceres world predates passengers and their Ceres v4 dependency.
        packs.retain(|p| {
            !matches!(
                p.manifest().id().as_str(),
                tacsim::definitions::content::CAST_CERES_PACK
                    | "tacsim.passengers.stickney"
                    | "tacsim.passengers.ceres"
                    | "tacsim.crew.ceres"
                    | "tacsim.quests.first_steps"
                    | "tacsim.quests.leila_followup"
                    | "tacsim.quests.port_connections"
                    | "tacsim.quests.safe_passage"
                    | "tacsim.quests.survey_work"
            )
        });
        let new_ids: Vec<_> = tacsim::relay_content::packs()
            .unwrap()
            .iter()
            .map(|p| p.manifest().id().clone())
            .collect();
        packs.retain(|p| !new_ids.contains(p.manifest().id()));
        for pack in &mut packs {
            if pack.manifest().id().as_str() == tacsim::definitions::content::CERES_PACK {
                *pack = ceres_v2();
            }
        }
        packs.push(mush_content::load_pack(tacsim::relay_content::HISTORICAL_JSON).unwrap());
        rig.apply_content(packs).await;
        rig.cfg.tacsim.enabled = true;
        rig.restart().await.unwrap();
        assert_eq!(rig.registry().pairs.len(), 1);
        assert!(tacsim::relay_world::current(rig.tw.world()).is_none());
        assert_eq!(
            rig.latched(tacsim::definitions::content::CERES_PACK),
            Some(2)
        );
        // The current relay's exact dependency upgrades explicitly before the
        // historical topology migration exercised by these stories.
        rig.apply_content(vec![
            mush_content::load_pack(tacsim::definitions::content::CERES_JSON).unwrap(),
            mush_content::load_pack(tacsim::definitions::content::CAST_CERES_JSON).unwrap(),
        ])
        .await;
        rig.flush().await;
        rig
    }
}
#[tokio::test]
async fn r10_historical_pack_upgrade_preserves_identity_money_and_inflight_due() {
    let mut rig = Rig::old_star().await;
    let bob = rig.endpoints();
    rig.run(GOD, "@relay/disable luna_ceres.luna");
    rig.run(GOD, &format!("+order {bob}=old-world-inflight"));
    rig.pass();
    let envelope = rig.last_envelope();
    let flight = envelope.transit().unwrap().flight.clone().unwrap();
    let original = rig.registry();
    let money = rig.money_total();
    let npc = tacsim::relay_mobile::current(rig.tw.world())
        .unwrap()
        .postmaster;
    rig.apply_content(vec![
        mush_content::load_pack(tacsim::relay_content::JSON).unwrap(),
    ])
    .await;
    rig.restart().await.unwrap();
    let expanded = rig.registry();
    assert_eq!((expanded.nodes.len(), expanded.pairs.len()), (5, 4));
    assert_eq!(expanded.operator, original.operator);
    assert_eq!(
        expanded.pairs[&cid("luna_ceres")],
        original.pairs[&cid("luna_ceres")]
    );
    for n in ["malapert", "ceres"] {
        assert_eq!(
            expanded.nodes[&cid(n)].object,
            original.nodes[&cid(n)].object
        );
    }
    assert_ne!(
        expanded.nodes[&cid("malapert")].place,
        original.nodes[&cid("malapert")].place
    );
    assert_eq!(rig.money_total(), money);
    assert_eq!(
        tacsim::relay_world::current(rig.tw.world())
            .unwrap()
            .postmaster,
        npc
    );
    assert_eq!(
        rig.envelope(envelope.id()).transit().unwrap().flight,
        Some(flight.clone())
    );
    rig.advance_ic_to(flight.due.at);
    rig.pass();
    assert_eq!(
        rig.envelope(envelope.id()).state(),
        mush_propagation::EnvelopeState::Delivered
    );
    rig.flush().await;
    rig.restart().await.unwrap();
    assert_eq!(rig.registry(), expanded);
    assert_eq!(rig.money_total(), money);
    assert!(rig.tw.world().refused_namespace_writes().is_empty());
}

/// Re-sign a pack edited as JSON with the shipping canonical encoder, as
/// `crates/mush-content/examples/pack_digest.rs` does.
fn resigned(mut value: serde_json::Value) -> mush_content::Pack {
    let digest = mush_content::parse_pack(&value.to_string())
        .unwrap()
        .computed_digest()
        .unwrap()
        .to_hex();
    value["manifest"]["digest"] = digest.into();
    mush_content::load_pack(&value.to_string()).unwrap()
}
/// Published Ceres v2, before Oduya's role-aware guidance profile.
fn ceres_v2() -> mush_content::Pack {
    let mut ceres = tacsim::definitions::content::station_with_cast_value(
        tacsim::definitions::content::CERES_JSON,
        tacsim::definitions::content::CAST_CERES_JSON,
    )
    .unwrap();
    ceres["manifest"]["version"] = 2.into();
    ceres["entries"]["npc.oduya"]["def"]["profile"] = serde_json::from_str(include_str!(
        "../../../tacsim/tests/support/oduya_v2_profile.json"
    ))
    .unwrap();
    ceres["entries"]["npc.qureshi"]["def"] = serde_json::from_str(include_str!(
        "../../../tacsim/tests/support/qureshi_v3_def.json"
    ))
    .unwrap();
    ceres["entries"]
        .as_object_mut()
        .unwrap()
        .remove("lore.qureshi_personal");
    let pack = resigned(ceres);
    assert_eq!(
        pack.manifest().digest().to_hex(),
        "641e1a3983447cca1a05bed6a839f98fe68c2afa683e0deee0c5e4e2904d8538",
        "the rebuild is the published Ceres v2"
    );
    pack
}
/// Published relay v4: lane v15 and Ceres v2, with the same full star.
fn relay_v4() -> mush_content::Pack {
    let mut relay: serde_json::Value = serde_json::from_str(tacsim::relay_content::JSON).unwrap();
    relay["manifest"]["version"] = 4.into();
    for requirement in relay["manifest"]["requires"].as_array_mut().unwrap() {
        if requirement["id"] == tacsim::definitions::content::CERES_PACK {
            requirement["version"] = 2.into();
        }
    }
    let pack = resigned(relay);
    assert_eq!(
        pack.manifest().digest().to_hex(),
        "2ef60c5168baf6dc621bbc9ea5576a853df358d6789a7b541bd3739b9f971eb9",
        "the rebuild is the published relay v4"
    );
    pack
}
/// The published lane v13 (relay cut R9b): the shipped v15 less doc 20 cut SM.12b's yard and
/// SM.12a's two `mill` rows, at version 13, re-signed to its published digest.
fn lane_v13() -> mush_content::Pack {
    let mut lane: serde_json::Value =
        serde_json::from_str(tacsim::definitions::content::LANE_JSON).unwrap();
    // Doc 20 cut SM.12b: Occator Rim's yard first (a yard names a mill row).
    lane["entries"]["occator_port"]["def"]["site"]
        .as_object_mut()
        .unwrap()
        .remove("yard");
    for vendor in lane["entries"]["ceres_port"]["def"]["vendors"]
        .as_array_mut()
        .unwrap()
    {
        vendor.as_object_mut().unwrap().remove("mill");
    }
    lane["manifest"]["version"] = 13.into();
    let pack = resigned(lane);
    assert_eq!(
        pack.manifest().digest().to_hex(),
        "34d47907dd42dbcd5192aae05d6eaf76c89dd40abd877845ac83ec35968e47c1",
        "the rebuild is the published lane v13"
    );
    pack
}
/// The published relay v2 (relay cut R10): the shipped v5 at version 2, requiring lane
/// v13 and Ceres v2, re-signed to its published digest. No entry byte differs.
fn relay_v2() -> mush_content::Pack {
    let mut relay: serde_json::Value = serde_json::from_str(tacsim::relay_content::JSON).unwrap();
    relay["manifest"]["version"] = 2.into();
    for requirement in relay["manifest"]["requires"].as_array_mut().unwrap() {
        if requirement["id"] == tacsim::definitions::content::LANE_PACK {
            requirement["version"] = 13.into();
        } else if requirement["id"] == tacsim::definitions::content::CERES_PACK {
            requirement["version"] = 2.into();
        }
    }
    let pack = resigned(relay);
    assert_eq!(
        pack.manifest().digest().to_hex(),
        "6d99bd6f999c88285404f05242100945416191a2dc67e18b4838514124d61401",
        "the rebuild is the published relay v2"
    );
    pack
}
impl Rig {
    /// A world kept from relay cut R10: lane v13 and relay v2 applied, the full star
    /// live, booted by today's binary (lane v15, relay v6), whose boot pass upgrades
    /// neither latch.
    async fn v13_star() -> Self {
        let mut rig = Self::new(false).await;
        let mut packs = tacsim::composition::boot_pack_list(false, true).unwrap();
        packs.retain(|p| {
            !matches!(
                p.manifest().id().as_str(),
                tacsim::definitions::content::CAST_CERES_PACK
                    | "tacsim.passengers.stickney"
                    | "tacsim.passengers.ceres"
                    | "tacsim.crew.ceres"
                    | "tacsim.quests.first_steps"
                    | "tacsim.quests.leila_followup"
                    | "tacsim.quests.port_connections"
                    | "tacsim.quests.safe_passage"
                    | "tacsim.quests.survey_work"
            )
        });
        for pack in &mut packs {
            match pack.manifest().id().as_str() {
                tacsim::definitions::content::LANE_PACK => *pack = lane_v13(),
                tacsim::definitions::content::CERES_PACK => *pack = ceres_v2(),
                "tacsim.relay.luna_ceres" => *pack = relay_v2(),
                _ => {}
            }
        }
        rig.apply_content(packs).await;
        rig.cfg.tacsim.enabled = true;
        rig.restart().await.unwrap();
        rig.flush().await;
        rig
    }
    fn latched(&self, pack: &str) -> Option<u32> {
        self.tw
            .world()
            .module_state::<mush_content::ContentState>()
            .unwrap()
            .latches
            .applied(&cid(pack))
            .map(mush_content::Applied::version)
    }
    fn operator_org(&self) -> mush_org::Organization {
        let operator = self.registry().operator;
        self.tw
            .world()
            .module_state::<mush_org::OrgStore>()
            .unwrap()
            .state()
            .org(operator)
            .unwrap()
            .clone()
    }
    fn story(&self) -> serde_json::Value {
        serde_json::to_value(
            tacsim::relay_world::current(self.tw.world())
                .expect("the full star seeds the Terms story"),
        )
        .unwrap()
    }
}
/// **Doc 20 cut SM.12a: the relay seed follows the lane** (since doc 20 cut SM.12b, at lane v15
/// and relay v4). Relay v6 is v2's star re-signed to require lane v15 and Ceres v4 (`luna_ceres.json`;
/// `LANE_VERSION`'s doc). A world kept at lane v13 and relay v2 with the full star live boots
/// today's binary unchanged, takes lane v15 — SM.12a's mills and SM.12b's yard — by the lane's
/// own path (`@atlas/pack plan`/`apply`), and keeps the star through relay v6: staff's plan
/// reads "Nothing to do." (no entry differs, so there is nothing to apply and the latch may stay
/// at 2, which `full_star` still reads as the star), and the operator's apply of the shipped pack
/// (the v1 → v2 tests' driver) latches 6. The catalog — its operator, nodes, pairs and halves —
/// the operator org, the bindings, the Terms story and the money never move, across two
/// restarts.
#[tokio::test]
async fn sm12b_a_kept_v13_star_takes_lane_v15_and_relay_v6() {
    let lane = tacsim::definitions::content::LANE_PACK;
    let relay = "tacsim.relay.luna_ceres";
    let mut rig = Rig::v13_star().await;
    assert_eq!(rig.latched(lane), Some(13), "the boot pass never upgrades");
    assert_eq!(rig.latched(relay), Some(2), "the boot pass never upgrades");
    let original = rig.registry();
    assert_eq!((original.nodes.len(), original.pairs.len()), (5, 4));
    let org = rig.operator_org();
    let story = rig.story();
    let money = rig.money_total();
    assert!(rig.run(GOD, "@relay/check").contains("Bindings OK"));

    rig.apply_content(vec![
        mush_content::load_pack(tacsim::definitions::content::CERES_JSON).unwrap(),
        mush_content::load_pack(tacsim::definitions::content::CAST_CERES_JSON).unwrap(),
    ])
    .await;

    // The lane's own upgrade path: staff plan and apply.
    let plan = rig.run(GOD, &format!("@atlas/pack plan {lane}"));
    let fingerprint = plan
        .split("@atlas/pack apply ")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap_or_else(|| panic!("no fingerprint:\n{plan}"))
        .to_owned();
    let applied = rig.run(GOD, &format!("@atlas/pack apply {fingerprint}"));
    assert!(
        applied.contains(&format!("Applied {lane}@15: ")),
        "{plan}\n{applied}"
    );
    assert!(!applied.contains("STOPPED"), "{applied}");
    assert_eq!(rig.latched(lane), Some(15));

    // Relay v6 differs from v2 only in its manifest: staff have nothing to apply.
    let plan = rig.run(GOD, &format!("@atlas/pack plan {relay}"));
    assert!(plan.contains("Nothing to do."), "{plan}");
    rig.flush().await;
    rig.restart().await.unwrap();
    assert_eq!((rig.latched(lane), rig.latched(relay)), (Some(15), Some(2)));
    assert_eq!(rig.registry(), original);
    assert_eq!(
        rig.story(),
        story,
        "relay v2 beside lane v15 is still the full star"
    );

    // The operator's apply of the shipped seed latches v6 and changes nothing else.
    rig.apply_content(vec![
        mush_content::load_pack(tacsim::relay_content::JSON).unwrap(),
    ])
    .await;
    assert_eq!(rig.latched(relay), Some(6));
    assert_eq!(rig.registry(), original);
    rig.flush().await;
    rig.restart().await.unwrap();
    assert_eq!((rig.latched(lane), rig.latched(relay)), (Some(15), Some(6)));
    assert_eq!(rig.registry(), original);
    assert_eq!(rig.operator_org(), org);
    assert_eq!(
        rig.story(),
        story,
        "relay v6 is the full star: the Terms story boots unchanged"
    );
    assert!(rig.run(GOD, "@relay/check").contains("Bindings OK"));
    assert_eq!(rig.money_total(), money);
    assert!(rig.tw.world().refused_namespace_writes().is_empty());
}
#[tokio::test]
async fn r10_named_terms_case_deduplicates_unrelated_principals_and_cure() {
    let mut rig = Rig::new(true).await;
    let bob = rig.endpoints();
    let initial = tacsim::relay_world::current(rig.tw.world())
        .unwrap()
        .clone();
    assert_eq!(initial.progress(), 0);
    let office = rig
        .tw
        .world()
        .get(initial.postmaster)
        .unwrap()
        .location
        .unwrap();
    assert_eq!(
        rig.tw.world().get(office).unwrap().name,
        "Postmaster's Office"
    );
    assert!(
        rig.run(
            bob,
            &format!("+relay/policy luna_ceres/{bob}=throttled,operator.case,unauthorized")
        )
        .contains("Permission denied")
    );
    assert_eq!(
        tacsim::relay_world::current(rig.tw.world())
            .unwrap()
            .progress(),
        0
    );
    for _ in 0..2 {
        assert!(
            rig.run(
                GOD,
                &format!(
                    "+relay/policy luna_ceres/{bob}=throttled,operator.case,compound incident"
                )
            )
            .contains("policy recorded")
        );
    }
    assert_eq!(
        tacsim::relay_world::current(rig.tw.world())
            .unwrap()
            .facts
            .len(),
        1
    );
    rig.run(
        GOD,
        "+relay/policy luna_ceres/#1=suspended,operator.other,another principal",
    );
    assert_eq!(
        tacsim::relay_world::current(rig.tw.world())
            .unwrap()
            .progress(),
        2
    );
    rig.flush().await;
    rig.restart().await.unwrap();
    rig.run(
        GOD,
        &format!("+relay/policy luna_phobos/{bob}=suspended,operator.case,confirmed incident"),
    );
    let complete =
        serde_json::to_value(tacsim::relay_world::current(rig.tw.world()).unwrap()).unwrap();
    assert_eq!(complete["facts"].as_array().unwrap().len(), 2);
    rig.run(
        GOD,
        &format!("+relay/policy luna_ceres/{bob}=served,operator.case,settled"),
    );
    rig.flush().await;
    rig.restart().await.unwrap();
    rig.pass();
    assert_eq!(
        serde_json::to_value(tacsim::relay_world::current(rig.tw.world()).unwrap()).unwrap(),
        complete
    );
    assert!(rig.tw.world().refused_namespace_writes().is_empty());
}

#[tokio::test]
async fn r10_expansion_inherits_terminal_hub_loss_without_a_healthy_window() {
    let mut rig = Rig::old_star().await;
    let node = rig.registry().nodes[&cid("malapert")].object;
    r7_damage(&mut rig, node, "core", 0);
    let original = rig.registry();
    rig.apply_content(vec![
        mush_content::load_pack(tacsim::relay_content::JSON).unwrap(),
    ])
    .await;
    let expanded = rig.registry();
    assert_eq!(expanded.losses, original.losses);
    assert_eq!(
        expanded.pairs[&cid("luna_ceres")],
        original.pairs[&cid("luna_ceres")]
    );
    assert!(
        expanded
            .pairs
            .values()
            .all(|p| p.state() == LinkState::Dead)
    );
    rig.restart().await.unwrap();
    assert_eq!(rig.registry().losses, original.losses);
    r7_damage(&mut rig, node, "core", 1000);
    assert!(
        rig.registry()
            .pairs
            .values()
            .all(|p| p.state() == LinkState::Dead)
    );
}
#[tokio::test]
async fn oduya_kept_ceres_v2_and_relay_v4_upgrade_without_replacing_the_live_star() {
    let ceres = tacsim::definitions::content::CERES_PACK;
    let relay = "tacsim.relay.luna_ceres";
    let mut rig = Rig::new(false).await;
    let mut packs = tacsim::composition::boot_pack_list(false, true).unwrap();
    packs.retain(|p| {
        !matches!(
            p.manifest().id().as_str(),
            tacsim::definitions::content::CAST_CERES_PACK
                | "tacsim.passengers.stickney"
                | "tacsim.passengers.ceres"
                | "tacsim.crew.ceres"
                | "tacsim.quests.first_steps"
                | "tacsim.quests.leila_followup"
                | "tacsim.quests.port_connections"
                | "tacsim.quests.safe_passage"
                | "tacsim.quests.survey_work"
        )
    });
    for pack in &mut packs {
        match pack.manifest().id().as_str() {
            tacsim::definitions::content::CERES_PACK => *pack = ceres_v2(),
            "tacsim.relay.luna_ceres" => *pack = relay_v4(),
            _ => {}
        }
    }
    rig.apply_content(packs).await;
    rig.run(GOD, "@relay/disable luna_ceres.luna");
    let original = rig.registry();
    rig.flush().await;
    rig.cfg.tacsim.enabled = true;
    rig.restart().await.unwrap();
    let org = rig.operator_org();
    let money = rig.money_total();
    let story = rig.story();
    assert_eq!((rig.latched(ceres), rig.latched(relay)), (Some(2), Some(4)));
    assert_eq!(rig.registry(), original, "boot never upgrades the old star");

    let plan = rig.run(GOD, &format!("@atlas/pack plan {ceres}"));
    let fingerprint = plan
        .split("@atlas/pack apply ")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap_or_else(|| panic!("no Ceres upgrade fingerprint:\n{plan}"))
        .to_owned();
    let applied = rig.run(GOD, &format!("@atlas/pack apply {fingerprint}"));
    assert!(
        applied.contains(&format!("Applied {ceres}@4: ")),
        "{applied}"
    );
    assert!(!applied.contains("STOPPED"), "{applied}");
    assert_eq!((rig.latched(ceres), rig.latched(relay)), (Some(4), Some(4)));
    assert_eq!(rig.registry(), original);
    let plan = rig.run(GOD, &format!("@atlas/pack plan {relay}"));
    assert!(plan.contains("Nothing to do."), "{plan}");
    rig.flush().await;
    rig.restart().await.unwrap();
    assert_eq!(rig.registry(), original);
    assert_eq!(rig.story(), story);

    // Only the manifest changed; applying the checked seed advances its latch
    // without replacing the operator, node/half objects or disabled link state.
    rig.apply_content(vec![
        mush_content::load_pack(tacsim::relay_content::JSON).unwrap(),
    ])
    .await;
    assert_eq!((rig.latched(ceres), rig.latched(relay)), (Some(4), Some(6)));
    assert_eq!(rig.registry(), original);
    rig.flush().await;
    rig.restart().await.unwrap();
    assert_eq!(rig.registry(), original);
    assert_eq!(rig.operator_org(), org);
    assert_eq!(rig.story(), story);
    assert_eq!(rig.money_total(), money);
    assert!(rig.run(GOD, "@relay/check").contains("Bindings OK"));
    assert!(rig.tw.world().refused_namespace_writes().is_empty());
}

#[test]
fn r10_public_star_content_digests_are_pinned() {
    let expected = [
        (
            "region.kirkwood_depot",
            "0c4187a8757fe7f7fed9179f09f0a62e83db606ea25f2a9cdffebca6ccd76627",
        ),
        (
            "region.mercury_relay",
            "a8732cda91b8e10a9845ff87d879c9cf45983659b98064c8b583d1388b2ff725",
        ),
        (
            "neighborhood.malapert_exchange",
            "acad4209e0ded034b989bfbe0ff7311df8e02734d579bd6bcdd6a5abbf0b626c",
        ),
        (
            "neighborhood.phobos_exchange",
            "c186a5bc336ed6572d54e6caf47d471b0bb7081c745152fb270b55137d220b70",
        ),
        (
            "neighborhood.kirkwood_depot",
            "fe585584e7631ed5904c4402d5042f451ff487a417aaf5b259b27db0fcc4e328",
        ),
        (
            "neighborhood.mercury_relay",
            "511d9a2997a9c1a05e9249639a680496fdf1439a8777124b7187150bf823ac49",
        ),
        (
            "tacsim.relay.luna_ceres",
            // Doc 20 cut SM.12a: v3, v2's star re-signed to require lane v14 (was
            // `6d99bd6f…` at v2, requiring lane v13). Doc 20 cut SM.12b: v4, re-signed to
            // require lane v15 (was `a9bf71ef…` at v3).
            // Qureshi's trust service: v6 requires Ceres v4, with no entry changes.
            "b8e247be72aa912b0e30767e0dd5efb1cf6f47faab5fc7a64e5ae507b9c8e277",
        ),
    ];
    let packs = tacsim::relay_content::packs().unwrap();
    assert_eq!(packs.len(), expected.len());
    for (p, (id, digest)) in packs.iter().zip(expected) {
        assert_eq!(p.manifest().id().as_str(), id);
        assert_eq!(p.manifest().digest().to_hex(), digest);
    }
}

#[tokio::test]
async fn r10_upgrade_observes_cancelled_debt_without_inventing_an_active_subscription() {
    let mut rig = Rig::old_star().await;
    rig.endpoints();
    rig.retail_ready();
    let (alice, payer) = rig.retail_sender("OldDebt", false);
    let id = rig.subscribe(alice, &alice.to_string(), "priority", payer);
    rig.bill();
    rig.next_bill(id);
    assert_eq!(rig.subscription_obligation(id).missed_total, 2);
    assert!(
        rig.run(alice, &format!("+relay/unsubscribe {id}"))
            .contains("Cancelled")
    );
    assert!(tacsim::relay_world::current(rig.tw.world()).is_none());
    rig.apply_content(vec![
        mush_content::load_pack(tacsim::relay_content::JSON).unwrap(),
    ])
    .await;
    rig.restart().await.unwrap();
    rig.pass();
    let story = tacsim::relay_world::current(rig.tw.world()).unwrap();
    assert_eq!(story.progress(), 4);
    assert_eq!(story.facts.len(), 1);
    assert_eq!(
        story.facts[0].restriction.principal,
        mush_propagation::service::Principal::Object(alice)
    );
    assert_eq!(
        rig.billing().subscriptions[&id].phase,
        mush_propagation::billing::Phase::Cancelled
    );
    let facts = serde_json::to_value(story).unwrap();
    rig.flush().await;
    rig.restart().await.unwrap();
    rig.pass();
    assert_eq!(
        serde_json::to_value(tacsim::relay_world::current(rig.tw.world()).unwrap()).unwrap(),
        facts
    );
}

// Availability/privacy repair: production handlers, source reservations and reboot.
async fn ap_private_fixture(rig: &mut Rig) {
    let operator = rig.registry().operator;
    let other = rig
        .tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .unwrap()
        .state()
        .orgs()
        .find(|o| o.id != operator)
        .unwrap()
        .id;
    let mut catalog = rig.registry();
    let mut pair = catalog.pairs[&cid("luna_ceres")].clone();
    for (i, key) in ["malapert", "ceres"].iter().enumerate() {
        let mut node = catalog.nodes[&cid(key)].clone();
        let node_name = format!("R4IndependentNode{i}");
        rig.run(GOD, &format!("@create {node_name}"));
        node.object = rig
            .tw
            .world()
            .iter()
            .find(|o| o.name == node_name)
            .unwrap()
            .id;
        rig.run(GOD, &format!("@teleport {}={}", node.object, node.place));
        rig.run(
            GOD,
            &format!("@behavior/add {}=sim.relay_node", node.object),
        );
        rig.run(
            GOD,
            &format!("@unit/create {}=tacsim.unit.relay_station", node.object),
        );
        let half_name = format!("R4IndependentHalf{i}");
        let half = rig
            .tw
            .world_mut()
            .create(
                mush_core::ObjKind::Thing,
                &half_name,
                GOD,
                Some(node.object),
            )
            .unwrap();
        rig.run(GOD, &format!("@behavior/add {half}=sim.relay_half"));
        node.operator = other;
        let key = cid(&format!("independent_{key}"));
        pair.halves[i].node = key.clone();
        pair.halves[i].object = half;
        pair.halves[i].id = cid(&format!("independent.half{i}"));
        catalog.nodes.insert(key, node);
    }
    pair.class_policy = mush_sim::relay::ClassPolicy::Private;
    catalog.pairs.insert(cid("independent"), pair);
    catalog.revision += 1;
    catalog.reconcile(rig.tw.world()).unwrap();
    rig.flush().await;
    rig.replace_document(serde_json::to_value(&catalog).unwrap())
        .await;
    rig.restart().await.unwrap();
}

#[tokio::test]
async fn ap_one_sender_cannot_fill_shared_admission_before_or_after_handoff() {
    let mut rig = Rig::new(true).await;
    let receiver = rig.endpoints();
    let sender = rig.tw.spawn_player("APsender");
    let a = rig.tw.spawn_player("APunknownA");
    let b = rig.tw.spawn_player("APunknownB");
    for n in 0..16 {
        let out = rig.run(sender, &format!("@mail/quick {a} {b}=fill{n}/body"));
        assert!(out.contains("queued for IC transit"), "{out}");
    }
    let before = rig.mail().next_id();
    let out = rig.run(sender, &format!("@mail/quick {a}=refused/body"));
    assert!(
        out.contains("Your communication allowance is full"),
        "{out}"
    );
    assert_eq!(rig.mail().next_id(), before);
    assert!(
        rig.run(GOD, &format!("+order {receiver}=unrelated before handoff"))
            .contains("Queued revision")
    );
    for _ in 0..5 {
        rig.mail_tick();
    }
    rig.flush().await;
    rig.restart().await.unwrap();
    let out = rig.run(sender, &format!("@mail/quick {a}=still refused/body"));
    assert!(
        out.contains("Your communication allowance is full"),
        "{out}"
    );
    assert!(
        rig.run(GOD, &format!("+order {receiver}=unrelated after restart"))
            .contains("Queued revision")
    );
    let id = rig.last_envelope().id();
    for _ in 0..4 {
        rig.pass();
    }
    assert_eq!(
        rig.envelope(id).state(),
        mush_propagation::EnvelopeState::Delivered
    );
    let out = rig.run(receiver, &format!("+orders/ack {id}"));
    assert!(out.contains("Acknowledgement queued"), "{out}");
}

#[tokio::test]
async fn ap_multiple_mail_senders_leave_channel_and_tactical_headroom() {
    let mut rig = Rig::new(true).await;
    let receiver = rig.endpoints();
    let a = rig.tw.spawn_player("APunknown");
    for (who, count) in [("APfirst", 32), ("APsecond", 16)] {
        let sender = rig.tw.spawn_player(who);
        for n in 0..count {
            let out = rig.run(sender, &format!("@mail/quick {a}=fill{n}/body"));
            assert!(out.contains("queued for IC transit"), "{out}");
        }
    }
    let third = rig.tw.spawn_player("APthird");
    let out = rig.run(third, &format!("@mail/quick {a}=no/body"));
    assert!(out.contains("producer's allowance"), "{out}");
    rig.channel_setup(&[third, a]);
    assert!(
        rig.run(third, "radio independent channel")
            .contains("queued for IC transit")
    );
    assert!(
        rig.run(GOD, &format!("+order {receiver}=order remains usable"))
            .contains("Queued revision")
    );
}

#[tokio::test]
async fn ap_private_discovery_use_revocation_and_public_carriage_are_separate() {
    let mut rig = Rig::new(true).await;
    let bob = rig.endpoints();
    ap_private_fixture(&mut rig).await;
    let outsider = rig.tw.spawn_player("APoutsider");
    let luna = rig.registry().nodes[&cid("malapert")].place;
    rig.run(GOD, &format!("@teleport {outsider}={luna}"));
    rig.public_hub(false);
    for switch in ["status", "load", "audit", "discover", "publish"] {
        assert_eq!(
            rig.run(outsider, &format!("+relay/{switch} independent")),
            rig.run(outsider, &format!("+relay/{switch} no_such_pair"))
        );
    }
    rig.run(
        GOD,
        &format!("+relay/grant independent/{outsider}=priority"),
    );
    let plan = |rig: &Rig| {
        let w = rig.tw.world();
        let at = w.world_ic_now();
        let svc = w
            .module_state::<mush_propagation::routing::RoutingServices>()
            .unwrap();
        let metric = svc.0.snapshot(w, &at);
        mush_propagation::routing::plan(
            w,
            w.module_state().unwrap(),
            metric.as_ref(),
            luna,
            rig.registry().nodes[&cid("ceres")].place,
            &at,
            0,
            1,
            Some((outsider, bob)),
        )
        .0
        .unwrap()
    };
    assert!(plan(&rig).legs.iter().all(|l| l.pair.is_none()));
    assert!(
        rig.run(
            GOD,
            &format!("+relay/access independent/{outsider}=discover")
        )
        .contains("policy recorded")
    );
    assert!(
        rig.run(outsider, "+relay/status independent")
            .contains("not granted")
    );
    assert!(plan(&rig).legs.iter().all(|l| l.pair.is_none()));
    for actor in [outsider, bob] {
        rig.run(
            GOD,
            &format!("+relay/access independent/{actor}=discover,use"),
        );
    }
    assert!(
        plan(&rig)
            .legs
            .iter()
            .any(|l| l.pair == Some(cid("independent")))
    );
    rig.flush().await;
    rig.restart().await.unwrap();
    assert!(
        plan(&rig)
            .legs
            .iter()
            .any(|l| l.pair == Some(cid("independent")))
    );
    rig.run(GOD, "+relay/grant independent/#1=standard");
    rig.run(GOD, &format!("+order {bob}=queued before revoke"));
    let id = rig.last_envelope().id();
    rig.pass();
    assert!(rig.envelope(id).transit().unwrap().queued_at.is_some());
    rig.run(GOD, &format!("+relay/access independent/{bob}=discover"));
    rig.pass();
    assert!(
        rig.envelope(id)
            .transit()
            .unwrap()
            .legs
            .iter()
            .all(|l| l.pair.is_none())
    );
    rig.run(GOD, &format!("+relay/access independent/{outsider}=none"));
    assert_eq!(
        rig.run(outsider, "+relay/status independent"),
        rig.run(outsider, "+relay/status missing")
    );
    assert!(
        rig.run(GOD, "+relay/network independent=public")
            .contains("policy recorded")
    );
    assert!(
        rig.run(outsider, "+relay/status independent")
            .contains("allowed")
    );
    assert!(
        plan(&rig)
            .legs
            .iter()
            .any(|l| l.pair == Some(cid("independent")))
    );
    assert!(
        rig.run(GOD, "+relay/network independent=private")
            .contains("policy recorded")
    );
    assert!(plan(&rig).legs.iter().all(|l| l.pair.is_none()));
}

#[tokio::test]
async fn ap_organization_speaking_is_explicit_frozen_scoped_and_revocable() {
    let mut rig = Rig::new(true).await;
    let bob = rig.endpoints();
    ap_private_fixture(&mut rig).await;
    rig.public_hub(false);
    let official = rig.tw.spawn_player("APofficial");
    let luna = rig.registry().nodes[&cid("malapert")].place;
    rig.run(GOD, &format!("@teleport {official}={luna}"));
    let org = rig.registry().operator; // A different principal than the private operator.
    rig.run(
        GOD,
        &format!("@org/grant {official}={org} economy.account.post"),
    );
    assert!(
        rig.run(official, &format!("+relay/represent org:{org}"))
            .contains("Permission denied")
    );
    rig.run(GOD, &format!("@org/scope create {org}=local_comms"));
    let child = rig
        .tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .unwrap()
        .state()
        .scopes_in(org)
        .find(|s| s.name == "local_comms")
        .unwrap()
        .id;
    rig.run(
        GOD,
        &format!("@org/grant {official}={org} relay.speak scope={child}"),
    );
    assert!(
        rig.run(official, &format!("+relay/represent org:{org}"))
            .contains("Permission denied"),
        "local authority cannot claim institution-wide identity"
    );
    rig.run(GOD, &format!("@org/grant {official}={org} relay.speak"));
    assert!(
        rig.run(official, &format!("+relay/represent org:{org}"))
            .contains("principal selected")
    );
    rig.run(
        GOD,
        &format!("+relay/access independent/org:{org}=discover,use"),
    );
    rig.run(
        GOD,
        &format!("+relay/access independent/{bob}=discover,use"),
    );
    rig.run(official, &format!("@mail/quick {bob}=official/body"));
    let mail_id = rig.mail().next_id() - 1;
    assert_eq!(
        mush_mail::transit::intents(rig.tw.world())
            .unwrap()
            .into_iter()
            .find(|i| i.representation.is_some())
            .unwrap()
            .representation
            .unwrap()
            .organization,
        org.0
    );
    rig.run(official, "+relay/represent personal");
    rig.mail_tick();
    let envelope = rig.mail_route(mail_id, bob, false).unwrap();
    assert_eq!(envelope.representation().unwrap().organization, org.0);
    assert!(
        envelope
            .transit()
            .unwrap()
            .legs
            .iter()
            .any(|l| l.pair == Some(cid("independent")))
    );
    rig.flush().await;
    rig.restart().await.unwrap();
    assert_eq!(
        rig.envelope(envelope.id()).representation(),
        envelope.representation()
    );
    // Suspending the represented institution removes live speaking authority.
    let out = rig.run(GOD, &format!("@org/status {org}=suspended"));
    assert!(out.contains("now suspended"), "{out}");
    rig.pass();
    assert!(
        rig.envelope(envelope.id())
            .transit()
            .unwrap()
            .legs
            .iter()
            .all(|l| l.pair.is_none())
    );
    assert!(
        rig.run(official, &format!("+relay/represent org:{org}"))
            .contains("Permission denied")
    );
}

#[tokio::test]
async fn ap_control_reserve_survives_data_saturation_and_restart() {
    let mut rig = Rig::new(true).await;
    let receiver = rig.endpoints();
    let mut orders = Vec::new();
    let luna = rig.registry().nodes[&cid("malapert")].place;
    for i in 0..17 {
        let issuer = rig.tw.spawn_player(&format!("APcommander{i}"));
        rig.tw
            .world_mut()
            .set_flag(issuer, mush_core::Flags::ROYALTY, true)
            .unwrap();
        rig.run(GOD, &format!("@teleport {issuer}={luna}"));
        rig.run(GOD, &format!("+relay/grant luna_ceres/{issuer}=priority"));
        let out = rig.run(issuer, &format!("+order {receiver}=ack source {i}"));
        assert!(out.contains("Queued"), "{out}");
        orders.push(rig.last_envelope().id());
    }
    for _ in 0..6 {
        rig.pass();
        rig.advance_ic_to(rig.tw.world().world_ic_now().at + 1);
    }
    assert!(
        orders
            .iter()
            .all(|id| rig.envelope(*id).state() == mush_propagation::EnvelopeState::Delivered)
    );
    let unknown = rig.tw.spawn_player("APnoLocation");
    for (n, count) in [32, 32, 32, 16].into_iter().enumerate() {
        let sender = rig.tw.spawn_player(&format!("APdata{n}"));
        rig.tw
            .world_mut()
            .set_flag(sender, mush_core::Flags::ROYALTY, true)
            .unwrap();
        for i in 0..count {
            let out = rig.run(sender, &format!("+order {unknown}=pending {i}"));
            assert!(out.contains("Queued"), "{out}");
        }
    }
    let out = rig.run(
        GOD,
        &format!("+order {unknown}=data cannot consume reserve"),
    );
    assert!(out.contains("reserved for acknowledgements"), "{out}");
    rig.flush().await;
    rig.restart().await.unwrap();
    for id in &orders[..16] {
        let out = rig.run(receiver, &format!("+orders/ack {id}"));
        assert!(out.contains("Acknowledgement queued"), "{out}");
    }
    let out = rig.run(receiver, &format!("+orders/ack {}", orders[16]));
    assert!(out.contains("pending queue is full"), "{out}");
    assert_eq!(
        rig.tw
            .world()
            .module_state::<mush_propagation::PropagationStore>()
            .unwrap()
            .envelopes()
            .filter(|e| matches!(
                e.state(),
                mush_propagation::EnvelopeState::Queued
                    | mush_propagation::EnvelopeState::Transmitted
            ))
            .count(),
        128
    );
}

#[tokio::test]
async fn ap_organization_quota_is_shared_and_switching_identity_cannot_evade_actor_quota() {
    let mut rig = Rig::new(true).await;
    rig.endpoints();
    let org = rig.registry().operator;
    let unknown = rig.tw.spawn_player("APunlocated");

    let mut officials = Vec::new();
    for i in 0..3 {
        let who = rig.tw.spawn_player(&format!("APagent{i}"));
        // Royalty permits addressing an unknown-position test recipient, but
        // does not bypass relay.speak (only Wizard does).
        rig.tw
            .world_mut()
            .set_flag(who, mush_core::Flags::ROYALTY, true)
            .unwrap();
        rig.run(GOD, &format!("@org/grant {who}={org} relay.speak"));
        assert!(
            rig.run(who, &format!("+relay/represent org:{org}"))
                .contains("principal selected")
        );
        officials.push(who);
    }
    for who in &officials[..2] {
        for i in 0..16 {
            assert!(
                rig.run(*who, &format!("+order {unknown}=official {i}"))
                    .contains("Queued")
            );
        }
    }
    assert!(
        rig.run(officials[2], &format!("+order {unknown}=shared quota"))
            .contains("allowance is full")
    );
    rig.run(officials[0], "+relay/represent personal");
    for i in 0..16 {
        assert!(
            rig.run(officials[0], &format!("+order {unknown}=personal {i}"))
                .contains("Queued")
        );
    }
    assert!(
        rig.run(officials[0], &format!("+order {unknown}=actor quota"))
            .contains("allowance is full")
    );
    rig.flush().await;
    rig.restart().await.unwrap();
    assert!(
        rig.run(
            officials[2],
            &format!("+order {unknown}=persistent organization quota")
        )
        .contains("allowance is full")
    );
    assert!(
        rig.run(
            officials[0],
            &format!("+order {unknown}=persistent actor quota")
        )
        .contains("allowance is full")
    );
}

#[tokio::test]
async fn ap_mail_reservation_stays_with_original_sender_until_return_is_consumed() {
    let mut rig = Rig::new(true).await;
    let bob = rig.endpoints();
    let ceres = rig.registry().nodes[&cid("ceres")].place;
    rig.run(GOD, &format!("@prop/jam {ceres}=100"));
    for n in 0..32 {
        let out = rig.run(GOD, &format!("@mail/quick {bob}=roundtrip{n}/body"));
        assert!(out.contains("queued for IC transit"), "{out}");
    }
    for _ in 0..4 {
        rig.mail_tick();
    }
    rig.run(GOD, &format!("@prop/jam {ceres}=0"));
    for _ in 0..6 {
        rig.pass();
        rig.advance_ic_to(rig.tw.world().world_ic_now().at + 1);
    }
    // All outbound arrivals are durable; source receipt application has not run.
    assert_eq!(
        rig.tw
            .world()
            .module_state::<mush_propagation::PropagationStore>()
            .unwrap()
            .envelopes()
            .filter(|e| e.state() == mush_propagation::EnvelopeState::Delivered)
            .count(),
        32
    );
    assert!(
        rig.run(GOD, &format!("@mail/quick {bob}=between owners/body"))
            .contains("allowance is full")
    );
    rig.public_hub(false);
    for _ in 0..4 {
        rig.mail_tick();
    }
    let requests = mush_mail::transit::intents(rig.tw.world()).unwrap();
    assert_eq!(requests.len(), 32);
    assert!(
        requests
            .iter()
            .all(|r| r.key.ends_with(":return") && r.reservation_actor == GOD)
    );
    let out = rig.run(GOD, &format!("@mail/quick {bob}=returning control/body"));
    assert!(out.contains("allowance is full"), "{out}");
    rig.flush().await;
    rig.restart().await.unwrap();
    assert!(
        rig.run(
            GOD,
            &format!("@mail/quick {bob}=restart cannot refill/body")
        )
        .contains("allowance is full")
    );
}

#[tokio::test]
async fn ap_zero_based_organization_and_scope_survive_source_handoff_and_restart() {
    let mut rig = Rig::new(true).await;
    let bob = rig.endpoints();
    ap_private_fixture(&mut rig).await;
    rig.public_hub(false);
    assert_eq!(
        rig.tw
            .world()
            .module_state::<mush_org::OrgStore>()
            .unwrap()
            .state()
            .org(mush_org::OrgId(0))
            .unwrap()
            .root_scope
            .0,
        0
    );
    let official = rig.tw.spawn_player("APzeroOfficial");
    let luna = rig.registry().nodes[&cid("malapert")].place;
    rig.run(GOD, &format!("@teleport {official}={luna}"));
    rig.run(GOD, &format!("@org/grant {official}=0 relay.speak"));
    let out = rig.run(official, "+relay/represent org:0");
    assert!(out.contains("principal selected"), "{out}");
    let out = rig.run(GOD, "+relay/access independent/org:0=discover,use");
    assert!(out.contains("policy recorded"), "{out}");
    rig.run(
        GOD,
        &format!("+relay/access independent/{bob}=discover,use"),
    );
    let out = rig.run(official, &format!("@mail/quick {bob}=zero IDs/body"));
    assert!(out.contains("queued for IC transit"), "{out}");
    let id = rig.mail().next_id() - 1;
    rig.flush().await;
    rig.restart().await.unwrap();
    rig.mail_tick();
    let e = rig.mail_route(id, bob, false).unwrap();
    assert_eq!(
        e.representation(),
        Some(mush_core::RelayRepresentation {
            organization: 0,
            scope: 0
        })
    );
    assert!(
        e.transit()
            .unwrap()
            .legs
            .iter()
            .any(|l| l.pair == Some(cid("independent")))
    );
    rig.flush().await;
    rig.restart().await.unwrap();
    assert_eq!(rig.envelope(e.id()).representation(), e.representation());
    assert!(
        rig.run(official, "+relay/represent")
            .contains("org:0; scope 0")
    );
}

// RR-5..7: checked schedules, once-per-operator admission and persistent serialization.
impl Rig {
    fn pb_state(&self) -> &mush_propagation::service::State {
        self.tw
            .world()
            .module_state::<mush_propagation::PropagationStore>()
            .unwrap()
            .relay_transport()
            .service
            .as_ref()
            .unwrap()
    }
    fn pb_schedule(&mut self, pair: &str, fields: &str) {
        let reply = self.run(
            GOD,
            &format!("+relay/schedule {pair}={fields};integration gate"),
        );
        assert!(reply.contains("recorded"), "{reply}");
    }
    fn pb_passes(&mut self) {
        for _ in 0..6 {
            self.pass();
        }
    }
}
#[tokio::test]
async fn pb_schedule_authority_revision_bounds_and_frozen_admission() {
    let mut rig = Rig::new(true).await;
    let bob = rig.endpoints();
    rig.run(GOD, "@tacsim/time rate=1");
    assert!(
        rig.run(bob, "+relay/schedule luna_ceres=standard:120;unauthorized")
            .contains("Permission denied")
    );
    rig.pb_schedule("luna_ceres", "standard:120,rate:2,burst:2");
    let first = rig.pb_state().schedules[&cid("luna_ceres")].clone();
    assert_eq!(first.revision, 1);
    assert_eq!(first.actor, GOD);
    assert!(
        rig.pb_state()
            .audit
            .last()
            .unwrap()
            .action
            .contains("rate=2 burst=2")
    );
    for settings in [
        "expect:0,rate:1",
        "rate:9",
        "burst:0",
        "max:129",
        "period:10,start:9,window:2",
    ] {
        assert!(
            !rig.run(
                GOD,
                &format!("+relay/schedule luna_ceres={settings};refuse")
            )
            .contains("recorded")
        );
        assert_eq!(rig.pb_state().schedules[&cid("luna_ceres")], first);
    }
    rig.run(GOD, "+relay/grant luna_ceres/#1=standard");
    rig.run(GOD, &format!("+order {bob}=frozen-schedule"));
    let id = rig.last_envelope().id();
    rig.pb_passes();
    let ticket = rig.envelope(id).transit().unwrap().service.clone().unwrap();
    assert_eq!(ticket.policy.as_ref().unwrap().revision, 1);
    assert_eq!(ticket.declared_s, 120);
    rig.pb_schedule("luna_ceres", "expect:1,standard:300,rate:1,burst:1");
    rig.flush().await;
    rig.restart().await.unwrap();
    assert_eq!(
        rig.envelope(id).transit().unwrap().service.as_ref(),
        Some(&ticket)
    );
    assert_eq!(rig.pb_state().schedules[&cid("luna_ceres")].revision, 2);
    rig.advance_ic_to(ticket.eligible_at - 1);
    rig.pb_passes();
    assert!(!rig.run(bob, "+orders").contains("frozen-schedule"));
    rig.advance_ic_to(ticket.eligible_at);
    rig.pb_passes();
    assert!(rig.run(bob, "+orders").contains("frozen-schedule"));
    rig.run(GOD, &format!("+order {bob}=new-schedule"));
    rig.pb_passes();
    assert_eq!(
        rig.last_envelope()
            .transit()
            .unwrap()
            .service
            .as_ref()
            .unwrap()
            .declared_s,
        300
    );
    assert!(
        rig.run(GOD, "+relay/audit luna_ceres")
            .contains("revision 2")
    );
}
#[tokio::test]
async fn pb_two_spokes_pay_one_operator_admission_across_restart() {
    let mut rig = Rig::new(true).await;
    let bob = rig.endpoints();
    rig.run(GOD, "@tacsim/time rate=1");
    let registry = rig.registry();
    rig.run(
        GOD,
        &format!("@teleport me={}", registry.nodes[&cid("ceres")].coverage[0]),
    );
    rig.run(
        GOD,
        &format!(
            "@teleport {bob}={}",
            registry.nodes[&cid("mercury")].coverage[0]
        ),
    );
    for pair in ["luna_ceres", "luna_mercury"] {
        rig.run(GOD, &format!("+relay/grant {pair}/#1=standard"));
    }
    rig.pb_schedule("luna_ceres", "standard:60");
    rig.pb_schedule("luna_mercury", "standard:180");
    rig.run(GOD, &format!("+order {bob}=single-network-admission"));
    let id = rig.last_envelope().id();
    rig.pb_passes();
    let first = rig.envelope(id).transit().unwrap().service.clone().unwrap();
    assert!(!first.repeat_operator);
    assert_eq!(first.declared_s, 60);
    rig.advance_ic_to(first.eligible_at);
    rig.pass();
    let middle = rig.envelope(id);
    assert_eq!(middle.path().len(), 1);
    let second = middle.transit().unwrap().service.as_ref().unwrap();
    assert!(second.repeat_operator);
    assert_eq!(second.declared_s, 0);
    assert_eq!(
        second.policy.as_ref().unwrap().settings.standard_delay_s,
        180
    );
    assert_eq!(second.eligible_at, first.eligible_at);
    rig.flush().await;
    rig.restart().await.unwrap();
    rig.pb_passes();
    assert_eq!(rig.envelope(id).path().len(), 2);
    assert!(rig.run(bob, "+orders").contains("single-network-admission"));
    let receipts: Vec<_> = rig
        .pb_state()
        .receipts
        .iter()
        .filter(|r| r.envelope == id)
        .collect();
    assert_eq!(receipts.len(), 2);
    assert!(receipts.iter().all(|r| r.transmitted == first.eligible_at));
}
#[tokio::test]
async fn pb_paid_serialization_captures_first_unit_once_and_hides_incomplete_mail() {
    use mush_propagation::commerce::Phase;
    let mut rig = Rig::new(true).await;
    let bob = rig.endpoints();
    rig.run(GOD, "@tacsim/time rate=1");
    rig.retail_ready();
    rig.pb_schedule("luna_ceres", "rate:1,burst:1");
    let (alice, payer) = rig.retail_sender("SerializationBuyer", true);
    let (mail, id) = rig.retail_mail(alice, bob, &"x".repeat(490));
    let quote = rig.retail_quote(alice, id, "priority", payer);
    let price = rig.retail().purchases[&quote].total_minor;
    let total = rig.money_total();
    rig.run(alice, &format!("+relay/buy {quote}"));
    rig.flush_money(quote, "reserved").await;
    rig.pb_passes();
    let first = rig.envelope(id);
    let partial = first.transit().unwrap().serialization.clone().unwrap();
    assert_eq!((partial.sent, partial.total), (1, 4));
    assert!(first.path().is_empty());
    assert_eq!(rig.retail().purchases[&quote].phase, Phase::Consumed);
    assert_eq!(rig.account(payer).posted_balance.minor(), 10_000 - price);
    assert!(rig.mail().entry(bob, mail).is_none());
    assert!(!rig.pb_state().receipts.iter().any(|r| r.envelope == id));
    rig.flush_money(quote, "consumed").await;
    rig.restart().await.unwrap();
    let purchase = rig.retail().purchases[&quote].clone();
    rig.pb_passes();
    assert_eq!(
        rig.envelope(id).transit().unwrap().serialization.as_ref(),
        Some(&partial)
    );
    // Policy changes do not mint a fresh burst, change the paid price, or discard progress.
    rig.pb_schedule("luna_ceres", "burst:8,max:128");
    rig.pb_passes();
    assert_eq!(
        rig.envelope(id).transit().unwrap().serialization.as_ref(),
        Some(&partial)
    );
    for n in 1..4 {
        rig.advance_ic_to(partial.started + n);
        rig.pb_passes();
        if n < 3 {
            assert_eq!(
                rig.envelope(id)
                    .transit()
                    .unwrap()
                    .serialization
                    .as_ref()
                    .unwrap()
                    .sent,
                n as u32 + 1
            );
            assert!(rig.mail().entry(bob, mail).is_none());
        }
    }
    assert_eq!(
        rig.envelope(id).state(),
        mush_propagation::EnvelopeState::Delivered
    );
    let receipt = rig
        .pb_state()
        .receipts
        .iter()
        .find(|r| r.envelope == id)
        .unwrap();
    assert_eq!(receipt.serialized_from, Some(partial.started));
    assert_eq!(receipt.transmitted, partial.started + 3);
    assert_eq!(
        receipt.ticket.policy.as_ref().unwrap().settings.max_bytes,
        1024
    );
    rig.mail_tick();
    assert!(rig.mail().entry(bob, mail).is_some());
    rig.flush().await;
    rig.restart().await.unwrap();
    rig.mail_tick();
    assert!(rig.mail().entry(bob, mail).is_some());
    assert_eq!(rig.retail().purchases[&quote], purchase);
    assert_eq!(rig.money_total(), total);
}
#[tokio::test]
async fn pb_deferred_window_keeps_partial_progress_and_accepted_window() {
    let mut rig = Rig::new(true).await;
    let bob = rig.endpoints();
    rig.run(GOD, "@tacsim/time rate=1");
    rig.pb_schedule(
        "luna_ceres",
        "deferred:0,period:10,start:0,window:2,rate:1,burst:1",
    );
    rig.run(GOD, "+relay/grant luna_ceres/#1=deferred");
    rig.run(GOD, &format!("+order {bob}={}", "d".repeat(400)));
    let id = rig.last_envelope().id();
    rig.pass();
    let at = rig.tw.world().world_ic_now().at;
    let start = (at.div_euclid(10) + 1) * 10;
    rig.advance_ic_to(start);
    rig.pb_passes();
    assert_eq!(
        rig.envelope(id)
            .transit()
            .unwrap()
            .serialization
            .as_ref()
            .unwrap()
            .sent,
        1
    );
    rig.advance_ic_to(start + 1);
    rig.pb_passes();
    let progress = rig
        .envelope(id)
        .transit()
        .unwrap()
        .serialization
        .clone()
        .unwrap();
    assert_eq!(progress.sent, 2);
    rig.pb_schedule("luna_ceres", "period:1,start:0,window:1,max:128");
    rig.advance_ic_to(start + 2);
    rig.pb_passes();
    assert_eq!(
        rig.envelope(id).transit().unwrap().serialization.as_ref(),
        Some(&progress)
    );
    rig.flush().await;
    rig.restart().await.unwrap();
    rig.advance_ic_to(start + 9);
    rig.pb_passes();
    assert_eq!(
        rig.envelope(id).transit().unwrap().serialization.as_ref(),
        Some(&progress)
    );
    assert!(!rig.run(bob, "+orders").contains(&"d".repeat(400)));
    rig.advance_ic_to(start + 10);
    rig.pb_passes();
    rig.advance_ic_to(start + 11);
    rig.pb_passes();
    assert!(rig.run(bob, "+orders").contains(&"d".repeat(400)));
    // The new maximum is independent of rate and applies to later traffic.
    rig.run(GOD, &format!("+order {bob}={}", "n".repeat(200)));
    assert!(
        rig.last_envelope()
            .transit()
            .unwrap()
            .legs
            .iter()
            .all(|l| l.pair.as_ref() != Some(&cid("luna_ceres")))
    );
}
#[tokio::test]
async fn pb_one_unit_private_hardware_serializes_and_revocation_discards_partial() {
    let mut rig = Rig::new(true).await;
    let bob = rig.endpoints();
    rig.run(GOD, "@tacsim/time rate=1");
    ap_private_fixture(&mut rig).await;
    let mut registry = rig.registry();
    registry
        .pairs
        .get_mut(&cid("independent"))
        .unwrap()
        .capacity_units_per_ic_second = 1;
    registry.revision += 1;
    rig.replace_document(serde_json::to_value(registry).unwrap())
        .await;
    rig.restart().await.unwrap();
    rig.public_hub(false);
    rig.pb_schedule("independent", "standard:0,rate:1,burst:1");
    assert!(
        rig.run(GOD, "+relay/schedule independent=rate:2;too fast")
            .contains("installed link capacity")
    );
    rig.run(GOD, &format!("+relay/access independent/{bob}=use"));
    assert_eq!(
        rig.run(bob, "+relay/schedule independent"),
        rig.run(bob, "+relay/schedule missing")
    );
    rig.run(GOD, &format!("+order {bob}={}", "p".repeat(300)));
    let id = rig.last_envelope().id();
    rig.pb_passes();
    let first = rig.envelope(id);
    assert_eq!(
        first
            .transit()
            .unwrap()
            .serialization
            .as_ref()
            .unwrap()
            .sent,
        1
    );
    assert!(
        first
            .transit()
            .unwrap()
            .legs
            .iter()
            .any(|l| l.pair == Some(cid("independent")))
    );
    rig.flush().await;
    rig.restart().await.unwrap();
    rig.run(GOD, &format!("+relay/access independent/{bob}=none"));
    rig.pb_passes();
    let rerouted = rig.envelope(id);
    let r = rerouted.transit().unwrap();
    assert!(r.serialization.is_none());
    assert!(r.legs.iter().all(|l| l.pair.is_none()));
    assert!(r.flight.as_ref().is_some_and(|f| f.due.at > f.started.at));
    assert!(rerouted.path().is_empty());
    assert!(!rig.run(bob, "+orders").contains(&"p".repeat(300)));
}
#[tokio::test]
async fn pb_corrupt_policy_and_serialization_refuse_boot_without_resetting_spend() {
    let mut rig = Rig::new(true).await;
    let bob = rig.endpoints();
    rig.run(GOD, "@tacsim/time rate=1");
    rig.pb_schedule("luna_ceres", "rate:1,burst:1");
    rig.retail_ready();
    let (_, id) = rig.retail_mail(GOD, bob, &"z".repeat(490));
    rig.pb_passes();
    assert!(rig.envelope(id).transit().unwrap().serialization.is_some());
    assert!(
        rig.run(GOD, &format!("+relay/quote {id}=priority,1"))
            .contains("No such unconsumed outgoing copy")
    );
    assert!(
        !rig.run(GOD, "+relay/outbox")
            .contains(&format!("Envelope {id} "))
    );
    rig.flush().await;
    let key = format!("envelope:{id}");
    let original = rig
        .store
        .module_json_get("propagation", &key)
        .await
        .unwrap()
        .unwrap();
    let transport = rig
        .store
        .module_json_get("propagation", "relay_transport")
        .await
        .unwrap()
        .unwrap();
    for (record, good, bad) in [
        (key.as_str(), original.clone(), {
            let mut bad = original.clone();
            bad["transit"]["serialization"]["sent"] = 0.into();
            bad
        }),
        ("relay_transport", transport.clone(), {
            let mut bad = transport.clone();
            bad["service"]["schedules"]["luna_ceres"]["settings"]["rate"] = 9.into();
            bad
        }),
    ] {
        rig.store
            .write(WriteBatch {
                serial: rig.serial,
                durable: true,
                module_ops: vec![ModuleOp {
                    ns: "propagation".into(),
                    op: ModuleWrite::JsonSet {
                        key: record.into(),
                        value: bad.clone(),
                    },
                }],
                ..Default::default()
            })
            .await
            .unwrap();
        rig.serial += 1;
        assert!(rig.restart().await.is_err());
        assert_eq!(
            rig.store
                .module_json_get("propagation", record)
                .await
                .unwrap(),
            Some(bad)
        );
        rig.store
            .write(WriteBatch {
                serial: rig.serial,
                durable: true,
                module_ops: vec![ModuleOp {
                    ns: "propagation".into(),
                    op: ModuleWrite::JsonSet {
                        key: record.into(),
                        value: good,
                    },
                }],
                ..Default::default()
            })
            .await
            .unwrap();
        rig.serial += 1;
        rig.restart().await.unwrap();
    }
    assert_eq!(serde_json::to_value(rig.envelope(id)).unwrap(), original);
}
