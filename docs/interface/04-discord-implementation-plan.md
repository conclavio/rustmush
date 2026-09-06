# 04 — Discord programme (D): implementation plan

**Draft 0.1 — 2026-09-06.** Scoped from [03-discord-owner-questions.md](03-discord-owner-questions.md)
(every question answered; 13A confirmed by spike) against the tree at `48eda83` (`origin/main`, branch
`z`). Every symbol named here was opened in that tree; line numbers are cited only where the survey
already cites them (08i's confirm-by-symbol discipline applies to the build).

**Owner's brief (2026-09-06).** Accept the survey's recommendations as written (1A–15A, 13A after the
spike) with three amendments: the doc 09 on-call service is built **immediately after** D, so D ships
the seams it needs (03 §7a); the Q12 fork is decided now — a process-agnostic REST sender — rather
than deferred; and `@discord/show` sits behind a **dedicated, explicitly granted privilege** that Wizard
does not imply, with the grant and every permitted use audited durably and the observed data never
logged (03 Q15). Go forward until a fork needs the owner.

**Notation.** Programme letter **D**; cuts **D1–D6**; decisions **D-D-n**; verify lenses are single
letters per cut (L leak, B bounds, S staleness/replay, P projection, A abuse). "Session" is the
interface plan's unit (`02` "Notation").

Companion docs: 03 (the survey and its code facts), 02-connectivity (SSH, session, web, protocol),
02-implementation-plan §3.10–3.12 (I10 Request/Respond, I11 Notice, I12 backlog), 09 §"On-call staff
paging" (the consumer of §7a), `docs/ops.md` (the operator surface D extends).

---

## 1. Scope — what D is, in the owner's terms

Discord as a fourth, deliberately lossy interface: a player who leaves the terminal can still
**follow** their scene from their character's perspective, still **act** (speech, poses, dice, answers
to server requests, the safety commands), staff paging lands in a Discord channel, and configured
comsys channels mirror both ways with the bot as the sole authority on who said what. The requirements
are 03 §1 R0–R4; the one load-bearing design choice is 03 §3: **a follow is a real connection of the
character's own session**, so every gate the terminal has — perception, `@ignore`, the turn gate,
flood control, `@boot` — applies unchanged and nothing in the world learns the connection is Discord.

D builds, in this order:

- **The identity layer** (D1) — the verified link, its store records and reverse index, the
  `@discord` and `@privilege` command families, the privileges audit ledger, the transport crate's
  skeleton with the REST client the on-call service will reuse, and the bot's DM slash commands.
- **The follow connection** (D2) — a third bridge in the gateway, the DM writer with its coalescing
  filter, DM lines as command lines, auto-follow on linkdead, inactivity detach.
- **Interaction** (D3) — `Request` as buttons and modals, `Notice` as DMs with a pre-filled modal,
  scene turn lines, the "your turn" notice, and the owner-namespaced interaction router with its
  server-side arm.
- **The relay** (D4) — staff-flagged comsys channels mirrored through webhooks and back through a
  typed relay-speak with an origin marker.
- **The staff channel and the on-call seams** (D5) — `+request` and staff notices onto a mapped
  channel; the durable DM adapter.
- **Consent** (D6) — the scene-side `Request` producers that make "agree with rulings" real on every
  client; the first real I10 producer.

**Exit criterion.** A linked player steps away from a timestop; their character stays in the scene; the
DM shows what that character perceives and nothing more; a pose typed in the DM lands in the room under
every gate a typed pose has; a facilitator's end-of-scene proposal arrives as buttons and the answer is
tallied; the OOC channel mirrors both ways under the right names; `+request` reaches `#staff` and a
staff reply comes back attributed; no test reaches Discord; the privileges audit answers "who looked at
whose Discord identity and why" without containing an identity; the on-call service's seams (§7a of 03)
exist with tests.

### 1.1 Explicitly out of D

- **The on-call incident service itself** (rotation, acknowledgement timeout, escalation, the
  encrypted operations store) — the next programme; D ships its seams (D1, D3, D5) and nothing more.
- **Discord OAuth2 on the web client** (03 Q3C) — a later link method writing the same record.
- **A shared scene thread in a guild** — only a public-scene projection could feed it; later, opt-in
  per scene (03 §7).
- **The bot in a voice call** — blocked by the C-toolchain rule for every library (03 Q13 spike:
  libopus via cmake; DAVE mandatory). `@discord/startcall` (create a channel, invite the room) is
  ordinary REST and can follow D6 as a small cut; it is not in D1–D6.
- **Account-level bans** in general — D1 adds `@discord/ban <player>` (a Discord-scoped refusal); the
  general primitive belongs to whoever owns accounts next.
- **Repairing `@set` for `Powers`** — noted in 03 Q15; a separate small item.
- **A second protocol.** Every wire addition is additive (`ClientKind::Discord`, two IPC arms in D4,
  one in D3); older peers ignore what they do not know.

---

## 2. What D consumes as landed (by symbol)

- **Bridges** — `GameBridge::{enter_game, line, respond, query, disconnected, arm_linkdead_reaper,
  rollback_local_attach, actor_of}` and `AuthBackend::characters` (`mush-gateway/src/bridge_common.rs`,
  `accounts.rs`); the SSH and web bridges as the two templates (`ssh_bridge.rs`, `web_bridge.rs`);
  `delivery::send_event_to_conn`, `char_key`; `WebHooks` / `SshHooks` as the dyn-compatible hook
  shape (`BoxFuture` methods, `is_ip_blocked`/`auth_failed`/`connection_opened` for transports with an
  IP — Discord has none).
- **Session** — `ConnectionHandle::new_structured`, `Sink::Structured` (`Arc<Envelope>`), the resume
  ring and `backlog_matching`, `DetachOutcome::Linkdead { grace }`, `SessionManager::{send_to_char,
  session_of_char, conns_of_session, update_caps}`.
- **Protocol** — `OutputEvent::{Narrative, Sys, Room, Channel, Notice, Request, Panel, Weather,
  SessionInfo, Disconnect}`, `RequestKind`, `RequestOption`, `RespondChoice`, `NoticeAction`,
  `ClientCaps`/`ClientKind`, `Envelope { seq, ts }`, `ClientCommand::Respond`; ts-rs export pinned by
  `cargo gen-ts`.
- **IPC** — `WireInbound::{Cmd, Query, ConnUp, ConnDown, SessionGone, Respond}`, `ToServer::Request
  { id, req: ServiceReq }` / `ToGateway::Reply`, `SendTarget::{Char, Conn, All}`, `IPC_VERSION = 4`
  (additive arms keep it), `GatewayLink::{to_server, request, try_reserve_connection}`.
- **World and dispatch** — `Inbound::Respond` handling and `respond <n>` line mode (I10 part 2);
  `Ctx::open_request` (producer API); `Outbox::notice_to_actor` and the `Ctx::notice_*` conveniences
  (I11); `resolve_classified_sends` as the per-recipient gate; `conn_up_inner` / `session_gone`;
  `perm::{has_role, has_power}`; `CommandSpec { min_role, power, lock, scene_class }` enforced in
  `cmd.rs` (`role_of < min_role`, then `has_power`); `Flags`/`Powers` with `impl_bits_serde!`
  (bits as integers, unknown bits retained); `Obj.powers` as the additive-field precedent.
- **Store** — `ObjectStore::{get_account, list_accounts, set_account_ssh_keys, token_put, token_get,
  token_touch, token_del, tokens_del_account, set_ban, get_bans}` and their in-memory twins;
  `AccountRec { id, name, argon2_hash, created, characters, ssh_keys }` with its hand-written redacting
  `Debug`; `TokenRecord`; Redis keys `account:{lowername}`, `token:{hash}` with `EXAT`; the shared
  behaviour suite (`mush-store/src/suite.rs`, gated on `REDIS_URL`); `WriteBatch.objects: Vec<Obj>`
  (whole-object snapshots — a new `Obj` field needs no store change).
- **Staff ops** — `StaffQueue`, `StaffOp { actor, kind }`, `StaffOpKind::{AddSshKey, ListSshKeys,
  DelSshKey, NewChar, …}`, `register_server_commands_with_cap` (`mush-server/src/staff.rs`); the
  world task's executor arms (`world_task.rs` "`@sshkey/add requested`" and the off-loop
  `run_newpassword` shape: `list_accounts` scan to find the owning account, `send_sys` back).
- **Ledger** — `mush_core::Ledger::{prepare_append, apply_prepared, open_entries, restore_from_kv}`,
  `archive_page_range`, `read_cold_range`, `StoreReadCompletion` (H1) as used by `mush-canon`
  (`data.rs` `LOG`, `commands.rs` `canon_archive` / `CanonArchiveRead`) and `mush-org`
  (`api.rs` `audit()`, `OrgAuditEntry`, `module.rs` `OrgStore.audit: Ledger`, `boot.rs`
  `restore_from_kv(&data.ledger)`).
- **Comsys** — `channel_speak`, `channel_notice`, `ChannelFlags`, `ChanMsg`, `Membership`,
  `ComsysState::{channel_by_name, members_of, membership_by_alias}`, the `AliasStage`.
- **Scenes** — `SceneRecord`, `ScenesState::active_scene_of`, `panel::{keyframe, publish, FEED}`,
  `SceneEventKind`, `record_scene_event`, the `+timestop/next|end|pause|resume` and
  `+scene/outcome` handlers.
- **Outbox** — `DurableOutboxAdapter { owner, adapter, deliver }`, `DurableDeliveryError::{Retryable,
  Permanent}`, `AuthoritativeOutboxRecord::intent().{owner, adapter, payload}`.
- **Config and secrets** — `Config` with `web: toml::Value` deserialized by the binaries into
  `WebConfig` (`mush-config/src/config.rs`, `mush-gateway/src/lib.rs` `serve`, `mushd/src/main.rs`);
  `mush_llm::config::resolve_key` (env var named in config, never logged); `dotenvy` in `mushd`.
- **Testkit** — `TestWorld::{with_modules, spawn_player, connect, run, seen_by, assert_saw,
  drain_write_batch}`, `WriteBatchView::{appended_to, asserts_atomic_with_ledger}`, `text_of`.
- **Build pins** — `crates/mush-contract-tests/legacy-handlers.toml` (every new verb needs a stanza
  with a non-baseline category and a note); `cargo gen-ts`; the workspace `rustls = { default-features
  = false, features = ["ring", …] }` pin.
- **Library** (03 Q13 spike) — `twilight-http`, `twilight-gateway`, `twilight-model`, `twilight-util`
  **0.17.1**, `default-features = false`, features `rustls-native-roots` (+ `decompression` on http;
  `twilight-http` + `zlib` on gateway; never `zstd`), pinned exactly; `Route` has no custom variant,
  so the raw path is `reqwest`.

---

## 3. Cut structure

| Cut | Builds | Sessions | Depends on | Touches |
|---|---|---|---|---|
| D1 | Identity: link record + reverse index + codes + bans in the store; `Privileges`, `has_privilege`, `CommandSpec.privilege`; the `mush-privileges` module (grant family + audit ledger); `@discord` family as staff ops; `mush-net-discord` skeleton (config, `DiscordApi` + fake, `DiscordRest`, gateway task, `/link /unlink /whoami`); wiring in both binaries; help + ops | 3 | nothing | mush-store, mush-core, mush-privileges (new), mush-server, mush-net-discord (new), mush-gateway, mushd, mush-config, mush-contract-tests, docs |
| D2 | Follow: `discord_bridge.rs`, `ClientKind::Discord`, the DM writer + coalescer + policy, DM lines, `/follow /detach`, `@discord/follow|unfollow`, auto-follow on linkdead, idle detach, `quiet` | 3 | D1 | mush-gateway, mush-net-discord, mush-proto (+gen-ts), mush-server (follow ops), docs |
| D3 | Interact: `Request` → components, `Notice` → DM + modal, scene turn lines, "your turn" notice, the interaction router + `ServiceReq::DiscordInteraction` | 2 | D2 | mush-net-discord, mush-gateway, mush-ipc, mush-server, mush-scenes, mush-core (`Registrar::discord_interaction`) |
| D4 | Relay: `ChannelFlags.relay`, `ChanMsg.origin`, `Audience::Relay`/`SendTarget::Relay`, `ServiceReq::RelaySpeak` + comsys apply, webhooks, `[discord.relay]` | 2 | D1 | mush-comsys, mush-core, mush-ipc, mush-server, mush-gateway, mushd (sink arm), mush-net-discord |
| D5 | Staff channel + on-call seams: relay producers for `+request` and staff notices, the reply path, `DiscordDmAdapter` | 1–2 | D4 | mush-jobs, mush-server (econ_alerts producer + adapter), mush-net-discord, docs/ops |
| D6 | Consent: scene `Request` producers (end, seal, rewind, turn nudge), the `Consent` scene event, the facilitator tally | 1–2 | I10 (built); D3 to matter on Discord | mush-scenes, docs/help/timestop.md |

Order: D1, then D2 → D3 and D4 → D5 as independent chains (D2 ∥ D4 after D1), D6 whenever. Roughly
**12–14 sessions**.

### 3.1 Cut D1 — Identity

**What it is.** Everything R0 needs, the privilege the owner amended in, the REST client the on-call
service will hold, and the bot's smallest possible surface (three DM slash commands). No follow yet:
after D1 a player can link, unlink and see their status; staff with the privilege can look; the bot
answers `/link`.

**Store (`mush-store`).**

- `DiscordLinkRec { user_id: u64, linked_at: i64, relay_character: Option<ObjId>, auto_follow: bool,
  dm_channel: Option<u64> }` — `AccountRec.discord: Option<DiscordLinkRec>` with `#[serde(default)]`
  (the `ssh_keys` precedent). `AccountRec`'s hand-written `Debug` prints `discord: <linked>` /
  `<none>`, never the id (the "never in a journal" rule applies to log lines too).
- `ObjectStore::set_account_discord(account, Option<&DiscordLinkRec>) -> Result<bool>` — atomic;
  Redis: one `MULTI` with `JSON.SET account:{name} $.discord` and the reverse index `SET
  discord:user:{id} {lowername}` (or `DEL` on unlink). **Uniqueness** is the reverse key: the link
  flow does `SET NX` first and refuses if it exists.
- `ObjectStore::account_by_discord_user(user_id) -> Result<Option<AccountRec>>` — `GET
  discord:user:{id}` then `get_account`.
- Link codes: `DiscordCodeRecord { account: String, expires: i64 }` under `discord:code:{sha256hex}`
  with `EXAT` (the `token:*` shape); `discord_code_put(hash, rec)`, `discord_code_take(hash) ->
  Result<Option<DiscordCodeRecord>>` (Redis `GETDEL`; expired = absent; in-memory prunes on read).
  Single use by construction.
- Bans: `DiscordBanRec { user_id, by_account, reason, at }` under `discord:ban:{user_id}`;
  `discord_ban_put/get/del`. Keyed by snowflake so it survives unlink.
- The shared suite gains one section (in-memory + Redis-gated): link, relink refused, unlink clears
  the index, code take-once and expiry, ban put/get/del.

**Core (`mush-core`).**

- `Privileges: u32` bitflags in `flags.rs` beside `Powers`, `impl_bits_serde!(Privileges, u32)`;
  `Privileges::DISCORD_IDENTITY = 1 << 0`. `Obj.privileges: Privileges` with `#[serde(default)]`
  (whole-object snapshots make it additive). `examine` lists set privilege names beside powers.
- `perm::has_privilege(w, who, p) -> bool` — **only** `w.get(who).is_some_and(|o|
  o.privileges.contains(p))`; no role branch; a doc comment says why. A test pins that God and Wizard
  without the bit are refused.
- `CommandSpec.privilege: Option<Privileges>` (defaulted), enforced in `cmd.rs` immediately after the
  `power` check with the same "Permission denied." path; `commands.list` (I1) filters on it the way it
  filters on `power`.

**The privileges module (`crates/mush-privileges`, new, `{core}` deps, module name `privileges`).**

- State `PrivilegesState { audit: Ledger, pending_reads }` (the `mush-canon` shape); NS `privileges`,
  log id `audit`; boot restores the ledger from `data.ledger` (`mush-org/src/boot.rs` shape); the
  archive pages through a `StoreReadCompletion` (H1) like `canon_archive`.
- `PrivilegeAuditEntry { at, actor: ObjId, action: SmolStr, privilege: Option<SmolStr>, subject:
  Option<ObjId>, reason: Option<String>, detail: Value }`; actions `grant | revoke | use | denied |
  unlink | ban | unban`. The public API `mush_privileges::audit(ctx, PrivilegeAction) ->
  Result<(), PrivilegeError>` is how `mush-server`'s `@discord` handlers append (cross-namespace writes
  go through the owning module's API). `detail` may never carry an identity: a `debug_assert` and a
  projection test grep the entry for a 17–20-digit integer.
- Commands: `@privilege/grant <player>=<privilege>=<reason>`, `@privilege/revoke
  <player>=<privilege>=<reason>` (reason required, ≤ 240 bytes), `@privilege/list [<player>]` (no
  argument: every holder), `@privilege/audit [<player>] [<page>]`. `min_role: Role::Wizard` on all
  four (§8 q1). A grant of a bit the actor already holds is a no-op that still audits.

**Server (`mush-server/src/staff.rs`, `world_task.rs`).**

- `StaffOpKind::{DiscordLinkMint { actor }, DiscordUnlinkSelf { actor }, DiscordUnlink { actor, target
  }, DiscordShow { actor, target }, DiscordSet { actor, relay_character: Option<ObjId>, auto_follow:
  Option<bool> }, DiscordBan { actor, target, reason }, DiscordUnban { actor, target }}` executed
  off-loop like `AddSshKey`: the owning account resolved store-side from the actor (self ops) or the
  target (staff ops), results delivered with `send_sys`.
- The `@discord` family (`ArgShape::EqPair`, `min_role: Player` for self ops): bare `@discord`
  (status: linked yes/no, follow state, `auto_follow`, `relay_character`; never the id — a player's own
  status shows the *account name the bot knows*, not the snowflake, to keep one rule everywhere),
  `@discord/link` (mint: reply "DM the bot `/link ABCD-EFGH` within 10 minutes"), `@discord/unlink`,
  `@discord/set relay=<character>`, `@discord/set autofollow=on|off`. Staff: `@discord/show <player>`
  (`privilege: Some(DISCORD_IDENTITY)`, `min_role: Player` — the privilege is the gate, not the role),
  `@discord/unlink <player>`, `@discord/ban <player>=<reason>`, `@discord/unban <player>` (all
  `min_role: Wizard`).
- **Audit at authorization, in the command's transaction.** `@discord/show` appends `use` (or, when
  the gate refuses, `denied` — the spec gate refuses before the handler runs, so `denied` is appended
  by a tiny wrapper: the handler is registered with `privilege: None` and checks `has_privilege`
  itself, appending `denied` and returning "Permission denied." — one place, one rule); the staff
  mutations append `unlink` / `ban` / `unban` with the reason. Every append precedes the `StaffOp`
  push, so the record means "an authorized request was made".
- The mint: 8 symbols from a 32-symbol alphabet (no `0 O 1 I`), formatted `ABCD-EFGH`, hashed with
  SHA-256 before `discord_code_put`, TTL `LINK_CODE_TTL_S`; the raw code goes only to the actor's own
  connections (`Audience::Actor`) and is never logged.

**Transport (`crates/mush-net-discord`, new).**

- `DiscordConfig` (`[discord]`, serde default, owned here like `WebConfig`): `enabled`, `token_env`,
  `application_id`, `guild_id`, `link_code_ttl_s`, `link_attempts_per_10m`, `dm_coalesce_ms`,
  `dm_max_chars`, `dm_queue_lines`, `follow_idle_detach_s`, `auto_follow_default`, `relay:
  Vec<RelayMap { channel, discord_channel_id, direction, webhook }>` (D4), `staff_channel_id` (D5).
  The token is `resolve_key(token_env)`; absent + `enabled` is a loud boot error.
- `DiscordApi` (async, dyn-compatible, `BoxFuture`): `create_dm(user) -> channel`,
  `send(channel, Outgoing) -> message_id`, `edit(channel, message, Outgoing)`,
  `execute_webhook(webhook, Outgoing)`, `respond(interaction_id, token, Response)`,
  `register_guild_commands(app, guild, &[CommandDef])`, `events() -> BoxStream<Event>`. Two impls:
  `TwilightApi` (the Gateway shard + `twilight_http::Client`) and `FakeApi` (a `testing` module:
  scripted events, recorded calls, optional 429/permanent failures).
- `DiscordRest` — the REST-only half (`create_dm`, `send`, `edit`, `execute_webhook`, `can_dm`), built
  from the token alone (`twilight_http::Client::new(token)` + a `reqwest` raw path for unmodelled
  routes), `Clone`, usable from either process (03 §7a.2). `can_dm` caches "closed DM" per user for
  `CAN_DM_CACHE_S`.
- `DiscordHooks` (the bridge implements it; mirrors `WebHooks`): `link_redeem(user_id, code) ->
  LinkOutcome`, `unlink(user_id) -> bool`, `whoami(user_id) -> Option<WhoAmI>`; D2 adds `follow`,
  `detach`, `line`, `respond`, `disconnected`; D3 `interaction`; D4 `relay_message`.
- `serve(hooks, api, config, shutdown)` — the bot task: registers the guild commands at start,
  consumes `events()`, dispatches `/link`, `/unlink`, `/whoami` (ephemeral replies; a `/link` from an
  author with 5 failed attempts in 10 minutes is refused without a lookup), reconnects the shard on its
  own (no bridge state depends on the shard being up), and never logs message content above `trace`.

**Gateway (`mush-gateway/src/discord_bridge.rs`, new).** `DiscordBridge<A: AuthBackend>` holding a
`GameBridge<A>`, the store, a `DiscordRest`. `link_redeem`: hash → `discord_code_take` → ban check →
`account_by_discord_user` must be `None` and the account's `discord` must be `None` → `SET NX` the
reverse key via `set_account_discord` → tell the actor in game through `sessions.send_to_char` (a `Sys`
line "Discord linked.") → `LinkOutcome::Linked { account }`. Every refusal is one uniform ephemeral
message (no oracle on which accounts exist). `unlink` and `whoami` are store reads/writes plus the same
in-game line.

**Wiring.** `mush-gateway` `serve()` and `mushd` `main` both: `DiscordConfig::deserialize(config.discord)`;
if `enabled`, build the bridge and spawn `mush_net_discord::serve` beside the web task; a failure is
loud and **non-fatal** (the web precedent, W4). `mush-config` gains `discord: toml::Value`
(`empty_toml_table`), `mushd.toml.example` a commented `[discord]` block.

**Docs.** `docs/help/discord.md` (linking, what is sent where, unlinking, privacy), `docs/help/admin.md`
(the `@privilege` family, the corrected sentence about `@set` and powers), `docs/ops.md` "Discord bot"
(portal checklist: application, bot token in `DISCORD_BOT_TOKEN`, intents incl. Message Content for
D4, the guild invite scopes/permissions, egress), `legacy-handlers.toml` stanzas for `@discord` and
`@privilege` (`legacy-post-image`, family `discord.link` / `privileges.grant`).

**Tests (~30).** Store suite (both backends); `has_privilege` ignores role; `CommandSpec.privilege`
refuses and `commands.list` filters; grant/revoke/list/audit round trip with `asserts_atomic_with_ledger`
on the grant; `@discord/show` appends `use` for a holder and `denied` for a Wizard without the bit;
the audit projection probe (no 17–20-digit integer anywhere in an entry); mint → redeem through
`FakeApi` → account linked → in-game confirmation; relink refused; ban refuses redeem; code expiry;
the transport's `/link` attempt limiter; boot with `[discord].enabled = false` touches nothing;
`AccountRec` `Debug` never prints the id.

**Verify lenses.** **L** — can any surface print the snowflake (a room, `examine`, WHO, a log, the
audit)? **A** — can an unlinked Discord user learn whether an account exists, or exhaust link codes?
**S** — can a code be redeemed twice, or after expiry, or after a restart?

**As built (2026-09-06, branch `z`, uncommitted — the owner commits).** Everything above landed, in
the shape described, with these deviations recorded rather than assumed:

- **`@discord/show` is gated in its handler, not through `CommandSpec.privilege`.** The spec field
  exists and is enforced generically in `gate_spec` (and filters `commands.list`), but `@discord` is
  one verb whose self-service forms any player may use, so the privilege check runs on the `/show`
  branch — which is also what lets a refusal be **recorded** (`denied`) before "Permission denied."
  is returned on the `Ok` path, so the entry persists. A missing privileges module fails closed
  ("the audit could not be recorded; nothing was done").
- **The uniqueness lock is `discord_user_reserve` (`SET NX`) + `set_account_discord`**, two atomic
  calls rather than one Lua script; `account_by_discord_user` treats a reservation whose account
  carries no link as stale, so a crash between the two steps leaves nothing a reader believes.
- **A ban keeps the link.** `@discord/ban <player>` writes `discord:ban:{snowflake}` and leaves
  `$.discord` in place, so `@discord/unban <player>` can find the user; the bridge refuses a banned
  user's link/follow/relay. A user who unlinked while banned is unbanned by `@discord/unban
  <snowflake>` (typed, never displayed).
- **Two seams, not one:** `DiscordApi` (REST; `DiscordRest` is the twilight impl, `testing::FakeApi`
  the fake) and `EventSource` (the Gateway; `TwilightEvents` / `testing::FakeEvents`). The plan's
  `DiscordApi::events()` became the second trait so `DiscordRest` really is REST-only.
- **The link-code helpers live in `mush_config::discord`** (alphabet, normalization, SHA-256; the
  600 s TTL as a constant) because the server mints and the gateway redeems and neither should own
  the other's rule; `[discord].link_code_ttl_s` is therefore deferred until the server half reads
  `[discord]` (D2).
- **No `reqwest` raw path yet** — D1 uses no route twilight lacks — and `mushd` gains no new
  dependency: `mush_gateway::discord_bridge::spawn_bot(&Config, sessions, store)` is the one wiring
  call both binaries make, returning a `DiscordTask` they stop in their shutdown tails.
- `@privilege/archive [page]` rather than `@privilege/audit [player] [page]`; `@privilege/audit
  [player]` is the RAM tail filtered to entries where the player acted or was the subject.
- Owner default q1 taken: Wizard grants. q6 (`AccountIdentity` refresh) untouched — D4.

Landed: `Privileges` + `perm::has_privilege` + `CommandSpec.privilege` (core); `AccountRec.discord`,
`DiscordLinkRec`/`DiscordCodeRecord`/`DiscordBanRec`, nine `ObjectStore` methods in both backends and
the `discord:*` keys, `account_discord_link` in the shared suite (store); `mush-privileges` (module,
audit ledger, `@privilege` family, `audit()` API, archive read); `StaffOpKind::Discord*` + the
`@discord` family + executors (server); `mush-net-discord` (config, seams, twilight impls, fakes,
bot loop with the `/link` limiter); `discord_bridge.rs` + `spawn_bot` (gateway); `[discord]`
(config, `mushd.toml.example`); `help discord`, `help admin`, `docs/ops.md` "Discord bot" + keys;
`legacy-handlers.toml` stanzas for `@discord` and `@privilege`. Clippy `-D warnings` clean over all
eight touched crates, all targets; `cargo fmt --check` clean.

Tests: core + store + config + privileges + net-discord **637 passed, 0 failed, 53 ignored**
(the `REDIS_URL`-gated suite; `account_discord_link` is registered for it) across 13 suites; gateway
lib **36 passed** (the bridge's link/refusal tests); server staff **18 passed** (four new: the
privilege gate and denial audit, the staff forms' role/reason/audit rules, the self-service ops,
fail-closed without the module); the `legacy-handlers.toml` tripwire **2 passed**; `cargo check
--workspace --all-targets` clean (every crate's tests compile against the grown `Obj`, `CommandSpec`
and `AccountRec`). The full `cargo test --workspace` and the L/A/S adversarial verify (§4 step 3)
have **not** run yet — they are the next thing before D2 builds on this. A first self-review of the
lenses found one item to carry: `DiscordEvent`'s derived `Debug` prints the snowflake; nothing logs
an event today, and the bot's rule (no event above `trace`) holds, but D2's DM writer must keep it.

### 3.2 Cut D2 — Follow

**What it is.** R1 and the plain-line half of R2. The DM becomes a live, lossy view of the character.

- **The connection.** `ClientKind::Discord` (additive; `cargo gen-ts`; the TUI/web match sites are
  `Ssh`/`Web` literals, unaffected). `DiscordBridge::follow(user_id, character: Option<name>)`: resolve
  account → pick the character (the named one, else the single character, else the last connected —
  `relay_character` is the tiebreak; two or more with none named → an ephemeral list) → `ConnectionHandle
  ::new_structured(conn_id, Ipv4Addr::UNSPECIFIED, caps { kind: Discord, color: None, width: 80,
  height: 0, utf8: true, structured: true }, tx)` → `GameBridge::enter_game`. **Never** call
  `IpGuard::connection_opened` for a Discord connection; the per-user limiter is the bot's.
- **One attached character per Discord user** (D-D-6): a second `/follow` for another character
  detaches the first (`GameBridge::disconnected` with `Quit` semantics) and attaches the new one.
- **The DM writer** (`mush-net-discord/src/dm.rs`): owns the `Sink::Structured` receiver; **drains it
  eagerly** into a bounded coalescer (`DM_QUEUE_LINES`) so the 256-slot sink can never fill; a filter
  (03 §3 table) keeps `Narrative`, `Sys` tagged `scene|error|channel|system`, `Room` (compact),
  `Panel{feed:"scenes"}` (a one-line turn summary), `Notice`, `Request`, `Weather`, relay-flagged or
  mentioning `Channel`; drops the rest. Render: `mush_ansi` at `ColorDepth::None` (the existing text
  fallback), then Discord-markdown escaping. Batch on `DM_COALESCE_MS`, ≤ `DM_MAX_CHARS` per message.
  Under a 429 or a full queue drop the **oldest narrative** first and emit one `… N lines dropped
  (seq a–b) …` marker; `Notice` and `Request` are never dropped (they are held and sent first).
- **Inbound.** A DM message is a line: `DiscordHooks::line(conn, text)` → `GameBridge::line` (sanitize,
  `MAX_LINE_CHARS`, flood bucket, custody). `QUIT` in the DM detaches. A DM while unattached gets one
  ephemeral "use `/follow`" per `HINT_INTERVAL_S` and is not executed. Slash sugar: `/say`, `/pose`,
  `/look`, `/scene`, `/detach`, `/follow`, each producing the corresponding line.
- **Auto-follow on linkdead** (D-D-5). The bridge watches `DetachOutcome::Linkdead` for accounts whose
  link has `auto_follow = true` (a `GameBridge::disconnected` hook point, beside `arm_linkdead_reaper`):
  it attaches a Discord connection *before* the grace expires — the attach is `Rejoined {
  from_linkdead: true }` (one "has reconnected" announce, not two) — and the DM says "Your terminal
  dropped; following here. `/detach` to stop." If the DM cannot be created (closed DMs), nothing is
  attached and the account's `@discord` status shows "DMs closed".
- **Idle detach.** No Discord input for `FOLLOW_IDLE_DETACH_S` **and** not a member of an active scene
  (`ScenesQuery` through `GameBridge::query` is read-only and never stamps idle) → detach with a DM
  line. A Discord *platform* disconnect never detaches (the shard reconnects; the coalescer waits, then
  drops-oldest under its bound).
- **`quiet`** (D-D-7): `@discord/set quiet=on` reduces the filter to `Notice`, `Request`, and the
  actor's own scene (`Sys` tagged `scene`, the `scenes` panel line, and `Narrative` while
  `active_scene_of` is `Some`).
- `@discord/follow` / `@discord/unfollow` in game (staff ops that ask the gateway through the existing
  `ToGateway` session-control shape? No — the bridge is gateway-side; the command is answered by a
  `ServiceReq::DiscordFollow { actor, on }`… **not needed**: the in-game verbs are sugar the bridge
  already implements; they land as `StaffOpKind::DiscordFollow { actor, on }` whose executor sends a
  `ToGateway::DiscordFollow { char, on }` fire-and-forget arm — **one additive `ToGateway` arm**, the
  same shape as `SetFloodExempt`.)

**Tests (~25).** The policy table as a fixture-driven test (every `OutputEvent` variant → kept/dropped/
compacted); coalescing and the drop marker under a `FakeApi` returning 429; the sink never fills under a
10 000-line burst (the trip-wire test); a DM line runs under the turn gate (out-of-turn `+roll` refused
with the same text as SSH); `QUIT` detaches; auto-follow attaches once and announces once; idle detach
respects scene membership; `/follow` with two characters lists; the `ClientKind::Discord` round trip
and gen-ts.

**Verify lenses.** **L** — a DM never shows a line the same character's SSH connection would not (a
concealment probe with `+obfuscate`); **B** — the coalescer's bounds under a hostile burst; **S** — a
shard reconnect never duplicates or reorders (the `seq` is the order).

### 3.3 Cut D3 — Interact

**What it is.** The structured half of R2, and the interaction router the on-call service needs.

- **`Request` → components.** `Confirm` → two buttons; `Choice` with ≤ 25 options → buttons in rows
  of 5 (≤ 5 rows) else a string select (≤ 25) — `MAX_REQUEST_OPTIONS` is 32, so options 26–32 fall to
  a select with a "more" page; `Target`/`Text` → a button that opens a modal (one `TextInput` inside a
  `Label`, V2). A click is `GameBridge::respond(conn, id, choice)`; the message is edited to show the
  answer; at `deadline_ms` it is edited to "expired — the server applied its default" with components
  removed. `custom_id` = `d:req:<request id>:<option id>` (≤ 100 bytes; option ids are validated at
  the producer).
- **`Notice` → DM.** Severity marker (`ℹ` / `⚠` / `‼`), the text, and — when `action` is present — a
  button that opens a modal pre-filled with `action.prefill` (`value`, ≤ 4 000); submitting sends the
  line through `line()`. D-I-18 holds: no button sends a mutation directly.
- **Scene lines.** A `scenes` keyframe becomes one line on change: "Round 3 — Alice's turn" /
  "paused" / "resumed" / "ended". The **"your turn" `Notice`** producer lands in `mush-scenes`
  (`+timestop/next` and `resume`: `Ctx::notice_info(current_actor, "scene", "It is your turn (round
  N).")`) — client-neutral, I11's list.
- **The interaction router** (03 §7a.4). The bot task hands every component/modal interaction to
  `DiscordHooks::interaction(user_id, custom_id, values, reply)`. The bridge routes by namespace:
  `d:req:` → `respond`; `d:call:` → the bridge (later); `d:oncall:` and anything else →
  `ServiceReq::DiscordInteraction { user_id, account: Option<String>, custom_id, values }` →
  `ServiceResp::DiscordInteraction(Result<InteractionReply, InteractionError>)` — **one additive
  `ServiceReq`/`ServiceResp` pair**. The server dispatches by prefix to handlers registered at wiring
  (`Registrar::discord_interaction(prefix, Arc<dyn DiscordInteractionHandler>)`), each receiving the
  authenticated `(user_id, account)` and returning bounded reply text. The bridge answers Discord with
  a deferred update within 3 s and edits with the reply. A gateway-owned namespace never crosses the
  boundary.

**Tests (~20).** Each `RequestKind` renders to the expected components (snapshot); a click answers once
and a second click is ignored (the engine's duplicate rule, observed through the reply); deadline edit;
the prefill modal submits the exact line; the router: `d:req:` never crosses the boundary, an unknown
prefix does, an unregistered prefix is a bounded refusal; the "your turn" notice fires on `next` and on
`resume` and never on `pause`.

**Verify lenses.** **S** — a stale or forged `custom_id` (wrong request id, another user's) is refused
by the engine's existing rules; **B** — 25/5/40 component limits; **P** — the reply text is bounded and
carries no cross-viewer content.

### 3.4 Cut D4 — Relay

**What it is.** R4. Staff-flagged comsys channels mirrored into Discord channels and back.

- **Comsys.** `ChannelFlags.relay: bool` (serde default; `@channel/set <chan>=relay|!relay`,
  `COMSYS_ADMIN`); `ChanMsg.origin: Option<SmolStr>` (serde default; `"discord"` for relayed lines);
  `channel_speak` gains an `origin` parameter (the alias stage passes `None`) and, when the channel is
  `relay` and `origin != Some("discord")`, pushes the same `OutputEvent::Channel` to
  `Audience::Relay(channel_name)`.
- **Core / IPC.** `Audience::Relay(SmolStr)` → `SendTarget::Relay(SmolStr)` in `resolve_classified_sends`
  (no recipient gate — the world is publishing to a named relay sink, not to a character); the gateway
  routes `Relay` to the Discord bridge's channel mapper (`transmit_and_police` gains the arm; the
  all-in-one `InProcessSink` too). `ServiceReq::RelaySpeak { user_id, account, actor, channel, kind,
  text }` / `ServiceResp::RelaySpeak(Result<(), RelayError>)`: the server re-validates `actor` ∈
  `account` through the fail-closed `AccountIdentity` oracle, then re-enters comsys as a typed apply
  (`AsyncCompletion { module: "comsys", payload }`) whose handler runs membership, gag and speak-lock
  exactly as the alias stage does and calls `channel_speak(…, origin: Some("discord"))`.
- **Discord side.** `[discord.relay]` maps channel name → Discord channel id, `direction`
  (`both | to_discord | to_game`), `webhook: bool`. Game → Discord posts through a per-channel webhook
  created at first use (id stored in `mod:discord:webhook:{channel}` via a small gateway-side KV — or
  simply re-fetched from `GET /channels/{id}/webhooks` at boot; default the latter, no new store
  surface) with `username = character name`, `avatar_url = none`. Discord → game: a `MESSAGE_CREATE`
  in a mapped channel from a non-bot author → `account_by_discord_user` → `relay_character` (or the
  attached character if one is followed) → `RelaySpeak`; an unlinked author gets one ephemeral hint per
  `HINT_INTERVAL_S` and is otherwise ignored; a banned author is ignored silently. Per-author limiter
  `RELAY_PER_USER` (governor).
- **Loop guard.** `origin == "discord"` never re-enters the relay tap; a followed user's own relayed
  line reaches their DM *once* (through their membership, as any channel line) — the test pins it.
- **The AccountIdentity staleness** (03 §6): `RelaySpeak` for a character created since boot fails
  closed with a bounded "relink after the next restart" hint until the oracle learns to refresh on
  `link_character` — a one-line follow-up in `accounts_identity.rs` if the owner wants it now (§8 q6).

**Tests (~20).** Flag round trip and `@channel/set`; a relay-flagged channel emits `Relay` once per
line and an unflagged one never; `RelaySpeak` refuses a non-member, a gagged member, a speak-lock
failure, and a `(user, account, actor)` mismatch; the origin loop guard; webhook name = character name;
unlinked and banned authors; direction `to_discord` never accepts inbound.

**Verify lenses.** **L** — the `see` lock and `@ignore` leak is *documented*, and an IC channel is never
relay by default; **A** — a Discord user cannot speak as anyone but their own linked character, cannot
speak while gagged, cannot exceed the limiter; **S** — a replayed `RelaySpeak` (the gateway's custody
replay) does not double-post.

### 3.5 Cut D5 — Staff channel and the on-call seams

- **Producers.** `+request` creation (`mush-jobs`) and staff `Notice`s with category `alert` /
  `escalation` (`econ_alerts.rs`) also emit to `Audience::Relay("staff")` when `[discord].staff_channel_id`
  is set; the bridge posts them to that channel as the bot (no webhook impersonation for system lines).
  Staff reply in Discord on a `Staff` comsys channel mapped with `direction = both` (D4 machinery).
- **`DiscordDmAdapter`** (`mush-server`): `DurableOutboxAdapter { owner: "oncall", adapter:
  "discord.dm" }` over a `DiscordRest`; payload `{ user_id, text, components, incident, attempt }`;
  `Permanent` on a closed DM / unknown user / 4xx that is not 429, `Retryable` on 429 / 5xx / network;
  the message text carries `incident #… · attempt n`; the item's result records the message id. Wired
  only when `[discord].enabled`; a fake-REST test exercises retry, permanent failure and the duplicate
  label. Nothing produces an `oncall` intent yet — that is the on-call programme.
- `docs/ops.md` gains the on-call section's Discord paragraph: what D provides (03 §7a), what O owns.

**Tests (~12).** Producer emission gated on config; the adapter's three outcomes; the label; the
message id in the result; a staff reply attributed to the staff character.

**Verify lenses.** **P** — a staff line never reaches a player relay channel; **B** — bounded payloads.

### 3.6 Cut D6 — Consent

- `mush-scenes` opens a `Request` (`Ctx::open_request`) to every participant on: a facilitator's
  `+timestop/end` (Confirm: "End the scene?"), `+scene/outcome` (Choice over the outcome names),
  `+scene/rewind` (a new facilitator verb: Confirm), and — when the facilitator has set
  `+timestop/nudge <minutes>` — a turn-pass nudge to the current actor after that idle interval
  (Choice: pass / hold). Deadlines: `CONSENT_DEADLINE_S`; silence = **abstain**. Each answer appends a
  `SceneEventKind::Consent { about, choice }` to the scene log; the facilitator sees the tally in
  `+timestop/who` and the `scenes` keyframe; **nothing blocks** — the facilitator's command proceeds
  after the tally or the deadline, whichever first, and `+scene/pause` remains the immediate tool.
- `docs/help/timestop.md` documents the flow.

**Tests (~14).** Each producer opens exactly one request per participant; abstain on deadline; a
disagree is recorded and does not block; the tally in the keyframe; an observer is never asked.

**Verify lenses.** **S** — a late answer after the facilitator proceeded is ignored; **P** — the tally
names no one but to the facilitator.

---

## 4. Build discipline

02 §4's rhythm applies per cut: confirm the slice by symbol; build; `cargo clippy --workspace
--all-targets -- -D warnings`, `cargo test --workspace`, `cargo fmt --all --check`; a probe-confirming
adversarial verify with the cut's lenses; fold findings with `vf_<lens><n>_…` regression tests;
commit with the verify outcome; **push only on the owner's word**. Added for D:

- **No test reaches Discord.** `FakeApi` is the only `DiscordApi` in tests; CI sets no token; a test
  that constructs `TwilightApi` is a bug.
- **Secrets.** The token comes from the env var named in `[discord].token_env`; it is never in
  `mushd.toml`, never logged, never in a `Debug`. `DiscordConfig`'s `Debug` prints `token_env`, not a
  token (the config holds none).
- **Content never logs.** DM text, relay text and interaction values log only at `trace`; `info` lines
  carry ids and counts.
- **Identities never render.** The snowflake appears only in `@discord/show`'s reply; a projection
  probe (`assert_projection_text_clean`-style) runs over every audit entry, every `Notice`, every
  `Sys` line D emits, and `AccountRec`'s `Debug`.
- **Every new verb** lands on `legacy-handlers.toml` in the same commit; every `mush-proto` change runs
  `cargo gen-ts`; help pages change with the behaviour; `docs/ops.md` changes with the config.
- **Feature pins.** `twilight-*` pinned to `=0.17.1`; `default-features = false`; `rustls-native-roots`;
  never `zstd`; a workspace test asserts `aws-lc-sys` is absent from the lockfile.
- **Drain first, buffer second, send third** — the DM writer's invariant, with the burst test.

Standing rules from 02 §4 that D re-asserts: additive wire evolution; no new idle-stamping path
(`follow`, the writer, `ScenesQuery`, and `whoami` never touch `last_input`; a DM *line* does, as any
input must); degrade, never crowd (the drop marker, never truncated nonsense).

---

## 5. Coordination

- **With I.** D3's "your turn" notice is I11's producer list; D6 is I10's first real producer. D
  touches `mush-proto` only for `ClientKind::Discord`; I owns the client renders of `Request` and
  `Notice`, which D does not change.
- **With H.** The privileges archive reader rides H1's store-read seam exactly as canon does; nothing
  new is asked of H.
- **With the on-call programme (O, next).** D1 ships `DiscordRest` and the link lookups, D3 the
  interaction router and its server arm, D5 the adapter. O owns `OnCallMember`, the rotation, the
  operations store, and the first `oncall` outbox intent. The `d:oncall:` namespace is reserved now.
- **Shared files.** `mush-core/src/{flags.rs, perm.rs, cmd.rs, obj.rs}` (D1 adds; nothing else in
  flight touches them), `mush-server/src/{staff.rs, world_task.rs}` (arms added at the end of each
  match), both bridges' wiring in `mush-gateway/src/lib.rs` and `mushd/src/main.rs`. Whoever touches a
  shared file rebases on the other's last commit the same day.

---

## 6. Decisions

- **D-D-1** Discord is a **fourth transport bridge in the gateway**; a follow is a real connection of
  the character's session (03 Q1A, Q4A). Nothing in the world knows the transport.
- **D-D-2** The link is **one account ↔ one Discord user**, recorded as the immutable snowflake on the
  account record with a reverse index; the handle is never stored (Q2A).
- **D-D-3** The link flow is **in-game mint, Discord redeem**: a hashed, single-use, 10-minute code
  (Q3A). OAuth is a later second method writing the same record.
- **D-D-4** A follow **opens** explicitly or by opt-in auto-follow on linkdead, **closes** on
  `/detach`, `@discord/unfollow`, `QUIT`, `@boot`, unlink, or idle-and-not-in-a-scene — never on a
  Discord platform disconnect (Q5A).
- **D-D-5** **One attached character per Discord user**; `relay_character` speaks in relayed channels
  when nothing is attached (Q6A).
- **D-D-6** The DM is an **allow-list** rendered through `mush-ansi` plain text, coalesced, bounded,
  drop-oldest-narrative with a visible marker; `Notice` and `Request` are never dropped (Q7A).
- **D-D-7** A DM line is a **command line verbatim**; slash commands are sugar; buttons answer
  `Request`s; a `Notice` prefill opens a pre-filled modal — never a bare mutation (Q8A, D-I-18).
- **D-D-8** Consent is **advisory**: silence is abstain, disagreement is recorded and never a veto;
  `+scene/pause` stays the blocking tool (Q9A).
- **D-D-9** The relay is **staff-flagged, config-mapped, both directions, webhook-impersonated**, with
  a typed relay-speak re-validated fail-closed and an origin marker as the loop guard; the `see`/
  `@ignore` leak on the Discord side is documented, and IC channels are never relay by default (Q10A).
- **D-D-10** Staff paging in D is a **relayed staff channel**; the on-call service is the next
  programme and reuses D's seams (Q11A, 03 §7a).
- **D-D-11** The Discord client runs in the **gateway**; a **REST-only `DiscordRest`** is usable from
  either process and is the outbox adapter's transport (Q12A as amended; the Q12 fork → option (i)).
- **D-D-12** The library is **twilight 0.17.1** with defaults off, `rustls-native-roots`, `zlib`,
  behind `DiscordApi`; `reqwest` is the raw path (Q13A, spike).
- **D-D-13** The Message Content intent is **enabled** (Q14A); slash-only relay is the documented
  fallback.
- **D-D-14** `@discord/show` is gated by **`Privileges::DISCORD_IDENTITY`**, a new explicit-only type
  with no role short-circuit; grants, revocations, uses, denials and the Discord staff mutations are
  appended to a durable **audit ledger** that never records an identity; `@discord/ban` takes a player
  name (Q15A as amended).
- **D-D-15** Interactions arrive over the **Gateway** (never the HTTP endpoint — Discord makes them
  mutually exclusive and D needs the Gateway); the bridge routes by **`custom_id` namespace** and
  forwards unknown namespaces to the server through one additive `ServiceReq` (03 §7a.4).
- **D-D-16** Every D addition to the wire and the store is **additive and defaulted**; `IPC_VERSION`
  stays 4.

---

## 7. Constants

| Constant | Value | Where |
|---|---|---|
| `LINK_CODE_LEN` / alphabet | 8 symbols, `ABCD-EFGH`, 32-symbol set without `0 O 1 I` | mush-server (mint) |
| `LINK_CODE_TTL_S` | 600 | `[discord].link_code_ttl_s` |
| `LINK_ATTEMPTS_PER_10M` | 5 per Discord user | mush-net-discord |
| `PRIVILEGE_REASON_MAX_BYTES` | 240 | mush-privileges |
| `DM_COALESCE_MS` | 1 500 | `[discord].dm_coalesce_ms` |
| `DM_MAX_CHARS` | 1 900 (headroom under 2 000) | `[discord].dm_max_chars` |
| `DM_QUEUE_LINES` | 2 000 | `[discord].dm_queue_lines` |
| `FOLLOW_IDLE_DETACH_S` | 21 600 | `[discord].follow_idle_detach_s` |
| `HINT_INTERVAL_S` | 600 | bridge |
| `CAN_DM_CACHE_S` | 3 600 | `DiscordRest` |
| `RELAY_PER_USER` | 5 per 10 s | bridge (governor) |
| `MAX_CUSTOM_ID_BYTES` | 100 (Discord) | mush-net-discord |
| Buttons per row / rows / select options / components per message | 5 / 5 / 25 / 40 (Discord) | mush-net-discord |
| `CONSENT_DEADLINE_S` | 900 | mush-scenes |
| Audit chunk target | `Ledger` default | mush-privileges |

---

## 8. Open questions (defaults chosen; ★ marks an owner decision)

1. ★ **Who may `@privilege/grant`.** Wizard (default) or God only. The audit makes either safe; God-only
   is stricter and slower.
2. **Follow announce wording.** The plain "has reconnected" (default, D2); "… is following via
   Discord" needs a kind on `ConnUp` (03 Q4C) — a later additive field.
3. **Relay default direction** for a newly mapped channel: `both` (default) or `to_discord`.
4. **`quiet` default** for a new link: off (default).
5. **Idle detach** default 6 h (default) — a scene member is never idle-detached either way.
6. ★ **`AccountIdentity` refresh** on `link_character` so a character created since boot can relay
   without a restart: do it in D4 (default: yes, one small change in `accounts_identity.rs` guarded by
   a test) or leave the documented limitation.
7. **Webhook storage**: re-fetch per boot (default) versus a gateway-side KV entry.
8. **The privilege's name**: `DISCORD_IDENTITY` (default).

None blocks D1; q1 and q6 are the owner's, the rest are defaults the build takes.

---

## 9. What D hands back

- **To O (on-call).** `DiscordRest` (D1), `account_by_discord_user` and `AccountRec.discord` (D1), the
  `d:oncall:` interaction namespace and `ServiceReq::DiscordInteraction` (D3), `DiscordDmAdapter` (D5),
  and the rule that O references the *account*, never a copy of the snowflake (03 §7a.1).
- **To I.** The "your turn" notice (I11), D6 as I10's first producer, and the observation that
  line-mode `+backlog` (I12, deferred) is what a player returning from Discord to SSH will miss first.
- **To scenes.** `SceneEventKind::Consent` and the tally; `+scene/rewind` as a new facilitator verb.
- **To accounts.** The general account-ban primitive (03 Q15) and the `@set`-for-`Powers` repair.
- **To ops.** The Discord section of `docs/ops.md`, and the reminder that the gateway now makes
  outbound connections (Discord's gateway websocket and REST) — egress rules must allow them.

---

## 10. Changelog

- **Draft 0.1 — 2026-09-06.** First plan: six cuts, sixteen decisions, the constants, eight open
  questions with defaults.
- **Draft 0.2 — 2026-09-06.** D1 built (§3.1 "As built"): the identity layer, the privilege and its
  audit ledger, the transport crate and bridge, docs and pins; deviations recorded; the L/A/S verify
  still owed.
