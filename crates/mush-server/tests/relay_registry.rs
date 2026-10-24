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
        .find(|o| o.name == "Kasei Receiving Office")
        .unwrap()
        .id;
    rig.run(GOD, &format!("@teleport {far}={phobos}"));
    let chan = rig.channel_setup(&[GOD, bob, far]);
    rig.run(GOD, "@relay/disable luna_ceres.luna");
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
    rig.run(GOD, "@relay/enable luna_ceres.luna");
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
    rig.run(GOD, "@relay/disable luna_ceres.luna");
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
            assert!(
                self.run(GOD, &format!("+relay/grant luna_ceres/{actor}=priority"))
                    .contains("Class grant recorded")
            );
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
    rig.run(GOD, "@relay/disable luna_ceres.luna");
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
    rig.run(GOD, "@relay/enable luna_ceres.luna");
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
    for i in 0..mush_propagation::routing::MAX_LINK_QUEUE {
        let reply = rig.run(GOD, &format!("+order {receiver}=pending-{i}"));
        assert!(reply.contains("Queued"), "{i}: {reply}");
    }
    assert!(
        rig.run(GOD, &format!("+order {receiver}=overload"))
            .contains("link queue is full")
    );
    assert!(!rig.run(receiver, "+orders").contains("pending-"));
}

#[tokio::test]
async fn r2_moving_recipient_requires_another_physical_leg() {
    use mush_propagation::EnvelopeState;
    let mut rig = Rig::new(true).await;
    let receiver = rig.endpoints();
    rig.run(GOD, "@relay/disable luna_ceres.luna");
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
    for i in 0..MAX_PENDING_ROUTES {
        let reply = rig.run(GOD, &format!("+order {receiver}=waiting-{i}"));
        assert!(reply.contains("Queued"), "{reply}");
    }
    assert!(
        rig.run(GOD, &format!("+order {receiver}=overflow"))
            .contains("pending queue is full")
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
    assert_eq!((original.nodes.len(), original.pairs.len()), (2, 1));
    assert_eq!(original.pairs[&cid("luna_ceres")].state(), LinkState::Live);
    assert_eq!(original.nodes[&cid("malapert")].coverage.len(), 12);
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
    let stickney = rig
        .tw
        .world()
        .iter()
        .find(|o| o.name == "Kasei Receiving Office")
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
    rig.run(GOD, "@relay/disable luna_ceres.luna");
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
    for i in 0..4 {
        assert!(
            rig.run(GOD, &format!("@mail/quick {names}=waiting{i}/secret"))
                .contains("queued for IC transit")
        );
    }
    let next = rig.mail().next_id();
    assert!(
        rig.run(GOD, &format!("@mail/quick {names}=overload/x"))
            .contains("queue is full")
    );
    assert_eq!(rig.mail().next_id(), next, "refusal allocates nothing");
    assert_eq!(
        mush_mail::transit::intents(rig.tw.world()).unwrap().len(),
        128
    );
    let first = next - 4;
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
    for n in 0..64 {
        assert!(
            rig.run(GOD, &format!("radio bounded-{n}"))
                .contains("queued for IC transit")
        );
    }
    assert!(rig.run(GOD, "radio refused-65").contains("full"));
    assert_eq!(rig.channels().channels[&chan].seq, 64);
    assert_eq!(
        mush_comsys::transit::intents(rig.tw.world()).unwrap().len(),
        128
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
    assert!(rig.channels().ic.progress.expired >= 64);
    rig.run(GOD, "@relay/enable luna_ceres.luna");
    rig.run(GOD, "@channel/set Radio=cap=1");
    for n in 65..=66 {
        rig.run(GOD, &format!("radio retained-{n}"));
        rig.finish_channel(chan, n);
    }
    assert!(!rig.backlog(bob).contains("retained-65"));
    assert!(rig.backlog(bob).contains("retained-66"));
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
    // Restoring the default class releases the bounded entitlement slot.
    rig.run(GOD, &format!("+relay/grant luna_ceres/{receiver}=standard"));
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
            .any(|g| g.principal == receiver)
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
async fn r3_mixed_traffic_skips_a_busy_head_and_receipts_follow_membership() {
    use mush_propagation::record::DispatchKind;
    let mut rig = Rig::new(true).await;
    let bob = rig.endpoints();
    let channel = rig.channel_setup(&[GOD, bob]);
    // Seven mail units, then an eight-unit channel copy, then one order unit.
    // The channel must not consume the last unit or block the smaller order.
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
        1,
        "eligible small work passes a busy head"
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
async fn r3_upgrade_full_legacy_queue_preserves_capacity_and_refuses_partial_policy() {
    let mut rig = Rig::new(true).await;
    let bob = rig.endpoints();
    let exit = rig.registry().nodes[&cid("ceres")].place;
    rig.run(GOD, &format!("@prop/jam {exit}=100"));
    for n in 0..64 {
        assert!(
            rig.run(GOD, &format!("@mail/quick {bob}=legacy-{n}/payload"))
                .contains("queued for IC transit")
        );
    }
    // Convert a persisted, normally admitted full queue to its R2/R6 shape.
    // This models an upgrade; it never mutates another owner's live state.
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
    transport.as_object_mut().unwrap().remove("service");
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
