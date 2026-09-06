//! The `EconomyModule` (E2b) — the cut that makes the pure E2a engine LIVE: the
//! double-entry journal on the E0 `Ledger`, the full-bundle atomic `WriteBatch`, the
//! strict durable money ack-gate, and directional boot self-heal.
//!
//! ## Contracts (from the E2 design review)
//! - **The journal is the sole authority.** Balances, the idempotency index, the
//!   txid index, and the reversed set are ALL re-derived from the journal at boot
//!   ([`crate::boot`]); a projection that merely lags self-heals, and only real
//!   drift / a lost tail / an unreplayable journal freezes (R-B1/R-B2).
//! - **One post = one atomic bundle** ([`crate::post`]): the journal append + every
//!   touched account projection + the allocator `meta`, all on one `WriteBatch`,
//!   gated on the durable ack; the "posted" confirmation is emitted only on a durable
//!   `Ok(Ok)` via the strict gate (invariant 9 / R-R1).
//! - **No balance-mutation API outside validated posting** (§18-E2 exit criterion) —
//!   [`crate::post`]/[`crate::reverse`] are the only writers, and both go through the
//!   pure E2a `validate`.
//! - **Conservation tripwire** (R-R5): with a zero start and all mint/burn through a
//!   system source/sink, the sum of every account's balance per instrument is
//!   invariantly zero; a non-zero sum at boot is corruption → whole-economy freeze.

use std::collections::BTreeMap;
use std::sync::Arc;

use mush_core::module::{
    GameModule, ModError, ModuleBootData, ModuleCtx, Registrar, WorldEvent, WorldEventKind,
};
use mush_core::{Ctx, Ledger, ObjId};
use smol_str::SmolStr;

use crate::amount::Amount;
use crate::boot;
use crate::config::EconomyConfig;
use crate::custody::{ContainerProvider, CustodyProvider, CustodyTable};
use crate::escalation::Escalation;
use crate::ids::{
    AccountId, ApprovalRequestId, CadenceId, ContractId, EscalationId, HoldId, ObligationId,
    TransactionId, VaultId,
};
use crate::record::{Account, EconomicTransaction, IdempotencyKey, Vault};
use crate::snapshot::EconomySnapshot;

/// The cold-replay wall time past which the durability checkpoint becomes the next
/// cut's first item (08g §12.1 item 5, C17). Boot warns when it is crossed. The
/// replay itself, with the store local (doc 12 D-12-3); the fetch is logged beside
/// it, never inside it.
pub const CHECKPOINT_REVIEW_SECS: u64 = 5;
/// How many of the newest journal transactions the economy keeps in RAM as full
/// records by default (doc 12 §5.1, J2): about the last 300 chunks. Everything
/// older is reachable by id through [`EconomyStore::transaction_ref`] and by page
/// through the store-read seam (`@econ/history`, `@econ/reverse`); the sweep
/// evicts down to the limit, never the post path.
pub const DEFAULT_TX_TAIL_MAX: usize = 20_000;

/// The replayed-entry count past which the same review is due (08g §12.1 item 5).
/// Doc 12 §4 derives the entry line from the measured per-entry cost: after J1
/// (parse once, ~3 µs per entry) the 5 s line is ~1.5 million entries, and this
/// stays at the more conservative million — the first time the two lines agree.
/// The RAM lines of doc 12 D-12-4 bind before either.
pub const CHECKPOINT_REVIEW_ENTRIES: usize = 1_000_000;

/// The module id and persistence namespace.
pub const NS: &str = "economy";
/// The single journal log id (the world's double-entry ledger).
pub const JOURNAL_LOG: &str = "journal";

/// The in-RAM economy slot — the single source of truth at runtime. The
/// authoritative [`EconomySnapshot`] (instruments + account projections +
/// idempotency + reversed), the append-only journal [`Ledger`] (open tail in RAM),
/// the txid index (for cold reversal — journal-derived at boot), the monotonic
/// allocators, and the whole-economy freeze latch.
/// What the cold journal replay cost at the last boot (H3; the C17 log's
/// numbers as an accessor, so a harness can read them).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReplayStats {
    /// Journal transactions replayed.
    pub entries: usize,
    /// Wall time the replay took, milliseconds — the C17 timer: from the boot
    /// data being in hand to the store being rebuilt.
    pub elapsed_ms: u64,
    /// Wall time the store fetch of the namespace took before that, in
    /// milliseconds (doc 12 D-12-3: logged beside the replay, not inside it;
    /// zero when the loader did not measure — a test rig).
    pub fetch_ms: u64,
    /// J3: the covered sequence of the checkpoint the boot seeded from, or
    /// `None` for a full replay.
    pub from_checkpoint: Option<u64>,
    /// J3: how many journal entries the boot actually folded (the tail past
    /// the checkpoint, or every entry).
    pub replayed: usize,
    /// J3: wall time selecting, validating and seeding from the checkpoint
    /// documents took, milliseconds, inside `elapsed_ms` (zero with none).
    pub checkpoint_ms: u64,
}

/// What the journal-derived index keeps for **every** transaction ever posted
/// (J2): enough to find it (its sequence, hence its chunk), to answer "is this
/// id mine" for a settlement, and to date it — without its body, which lives
/// in the RAM tail while it is recent and in the journal always.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TxRef {
    /// The journal sequence the transaction was posted at.
    pub seq: u64,
    /// The idempotency key it was posted under.
    pub idempotency: IdempotencyKey,
    /// When it was posted (wall-clock seconds).
    pub posted_at: i64,
}

/// One cold journal read a command is waiting on (J2; the H1 pattern): the
/// correlation `Ctx::read_store` returned and what the page is for, so the
/// completion finishes the right command for the right actor. Transient world
/// RAM — a restart drops it, and a completion nobody is waiting on is dropped.
#[derive(Clone, Debug)]
pub struct PendingEconomyRead {
    /// The correlation the completion must carry.
    pub correlation: u64,
    /// When it was requested (world seconds); a request older than
    /// `STORE_READ_STALE_SECS` is superseded by the next one.
    pub requested_at: i64,
    /// What the read is for.
    pub kind: PendingEconomyReadKind,
}

/// The two commands that reach past the RAM tail (J2).
#[derive(Clone, Debug)]
pub enum PendingEconomyReadKind {
    /// `@econ/history`: older entries touching `account`, `wanted` more rows,
    /// paging down from chunk `next_hi` with `pages_left` pages allowed. Only
    /// entries below `below_seq` — the RAM tail's floor when the command ran —
    /// are rendered: the open chunk holds the tail's bodies too, and those
    /// rows went out with the command.
    History {
        account: AccountId,
        wanted: usize,
        staff: bool,
        next_hi: u64,
        pages_left: u8,
        below_seq: u64,
    },
    /// `@econ/reverse`: the body of `target`, which left the tail.
    Reverse {
        target: TransactionId,
        cause: crate::record::EconomicCause,
    },
}

#[derive(Clone, Debug, Default)]
pub struct EconomyStore {
    /// The last boot's cold replay cost (H3); `None` until `on_world_loaded`.
    pub(crate) boot_replay: Option<ReplayStats>,
    /// Journal transactions posted since this boot (doc 12 D-12-1): with
    /// [`EconomyStore::last_boot`] it is the journal's live growth rate, the
    /// number the checkpoint decision keys on.
    pub(crate) posts_since_boot: u64,
    pub(crate) snapshot: EconomySnapshot,
    /// The double-entry journal (append-only; open tail in RAM, cold chunks paged
    /// from `data.kv` at boot).
    ///
    /// `pub(crate)` since the E4b1 verify fold (B7): `install_commit`'s reachability
    /// argument is *"the journal tail has exactly one mutator, and it is this
    /// function"* — an argument a `pub` field quietly voided, since any crate holding
    /// `&mut EconomyStore` could have appended. Nothing outside this crate ever used
    /// it; [`EconomyStore::journal`] is the read accessor.
    pub(crate) journal: Ledger,
    /// Every transaction by id — journal-derived at boot, so a cold reversal can read
    /// the original without a store round-trip.
    /// The **RAM tail** (J2): the newest [`EconomyConfig::tx_tail_max`]
    /// transactions as full records, in id order. Evicted oldest-first by the
    /// sweep ([`EconomyStore::evict_tail`]); a post only ever appends.
    pub(crate) transactions: BTreeMap<TransactionId, EconomicTransaction>,
    /// Every transaction ever posted, by id (J2): what the tail's eviction keeps.
    pub(crate) tx_index: BTreeMap<TransactionId, TxRef>,
    /// The journal-authority shadow (J3, [`crate::fold`]): every accumulator the
    /// boot replay derives from the journal, folded live by the same function on
    /// every post — balances, holds, titles, custody, terminal times, the title
    /// generation, the reversed set, the allocator high-water marks. What the
    /// checkpoint writer compares the projections against, and what it writes.
    pub(crate) fold: crate::fold::FoldState,
    /// The first journal sequence of every chunk (J2): chunk index → first seq,
    /// the map a cold read by sequence needs to name its chunk.
    pub(crate) chunk_firsts: BTreeMap<u64, u64>,
    /// In-flight cold journal reads, one per requesting actor (J2). Transient.
    pub(crate) pending_reads: std::collections::HashMap<ObjId, PendingEconomyRead>,
    /// J3: the checkpoint this store last wrote or booted from, if any.
    pub(crate) checkpoint: Option<crate::checkpoint::CheckpointMeta>,
    /// J3: every checkpoint document key the store knows (booted with or
    /// written), so a write can sweep what its head and the one it supersedes
    /// no longer name.
    pub(crate) checkpoint_keys: std::collections::BTreeSet<SmolStr>,
    /// J3: what boot reported — the checkpoint it seeded from and the entries
    /// it folded — for `ReplayStats`.
    pub(crate) boot_from_checkpoint: Option<u64>,
    pub(crate) boot_replayed: usize,
    pub(crate) boot_checkpoint_ms: u64,
    pub(crate) next_account: u64,
    pub(crate) next_transaction: u64,
    /// The vault-id allocator (E3b1); never reused, resumed from `meta` floored at
    /// one past every journal-derived custody effect.
    pub(crate) next_vault: u64,
    /// The approval-request-id allocator (E3b2); never reused, resumed from `meta`
    /// floored at one past every journal-derived `consumed_approval`.
    pub(crate) next_approval: u64,
    /// The hold-id allocator (E4a). E4b1 resumes it from `meta` floored at one past
    /// every **journal-derived** hold effect — never from surviving `hold:{id}` docs,
    /// because GC deletes terminal ones and re-minting a garbage-collected id under a
    /// stale `Contract.escrow` reference would attach live escrow to a dead contract.
    /// [`EconomyStore::peek_next_hold`] additionally floors it from the snapshot, so
    /// the pure path can never mint a colliding id even mid-plan.
    pub(crate) next_hold: u64,
    /// The obligation-id allocator (E4b1); never reused, resumed from `meta` floored
    /// past every parsable `obligation:` key — including a corrupt one, whose id was
    /// still minted.
    pub(crate) next_obligation: u64,
    /// The cadence-clock-id allocator (E4b1), floored the same way.
    pub(crate) next_cadence: u64,
    /// The escalation-id allocator (E4b1, §4.6), floored the same way — an
    /// unreadable escalation leaf must never have its id re-minted onto a different
    /// alert, or an acknowledgement would resume the wrong subject.
    pub(crate) next_escalation: u64,
    /// The contract-id allocator (E4b2, §5), floored past every parsable
    /// `contract:` key plus `meta`. Re-minting one would attach a live escrow hold
    /// (whose `purpose` names a `ContractId`) to a different agreement.
    pub(crate) next_contract: u64,
    /// The open + recently-acknowledged level-2/3 escalation records (§4.6), by id.
    /// Leaf docs (`escalation:{id}`): a corrupt one is skipped loudly at boot.
    pub(crate) escalations: BTreeMap<EscalationId, Escalation>,
    /// How many **consecutive** sweep fires have failed to expire each still-`Held`
    /// reservation whose deadline has passed (E4b1 verify fold, S6). RAM-only, like
    /// [`EconomyStore::hold_terminal_at`]: it is a ladder counter, not economic
    /// truth, and a restart is a fair place for the run to begin again.
    pub(crate) hold_expiry_failures: BTreeMap<HoldId, u32>,
    /// How many **consecutive** sweep fires have deferred each obligation's
    /// settlement as transient (S7). RAM-only for the same reason.
    pub(crate) obligation_defers: BTreeMap<ObligationId, u32>,
    /// Each account's recent **debits**, `(posted_at, amount)` oldest first — what
    /// an [`crate::ApprovalWindow`] is summed over (E6a step 1, D-E6-21). RAM-only
    /// and journal-derived exactly like [`EconomyStore::hold_terminal_at`]: it is a
    /// budget index, not economic truth, so it is rebuilt at boot from replay and
    /// never stored. Bounded by [`crate::MAX_APPROVAL_WINDOW_SECS`]: an entry older
    /// than the widest window any rule may declare is dropped at index time, so the
    /// tail is O(entries in that window) per account and a post reads only its own
    /// account's tail. One entry per transaction per account: the account's negative
    /// lines plus its placed holds, **less what the same transaction captured** — a
    /// capture spends a reservation the window already counted when it was placed,
    /// and counting it twice would make an escrowed run cost double. A refund never
    /// subtracts anything (conservative by design).
    ///
    /// **Only accounts whose rule actually carries a window are tracked** (E6a step 1
    /// verify fold, D4): the index existed for every account any post ever debited,
    /// which on a busy world is every account in it, held forever because the front
    /// was trimmed only when the *same* account was debited again. A rule that gains a
    /// window starts tracking from that moment, rebuilt once from the journal tail
    /// ([`EconomyStore::rebuild_debit_window`]); a rule that loses one stops.
    pub(crate) debit_window: BTreeMap<AccountId, WindowTail>,
    /// The notices queued for objects that were not connected when the economy had
    /// something to tell them (E6a step 1, DL-04): `econ:notice:{obj}` leaf docs,
    /// ≤ [`crate::MAX_PENDING_NOTICES`] per object, delivered and deleted at their
    /// next connect ([`crate::notice`]).
    pub(crate) notices: BTreeMap<ObjId, Vec<crate::notice::PendingNotice>>,
    /// Where the contract step starts scanning next fire (E4b2 verify fold, B4).
    ///
    /// The per-fire cap bounds *work*, and id order decided who got it: sixty-five
    /// contracts with something due meant the sixty-fifth was never reached, fire
    /// after fire, so a high-id contract could sit past its own deadline forever
    /// behind a wall of low-id ones. The cursor makes the scan a **rotation**, so
    /// every record is reached within one pass of the pool whatever its id. Durable in
    /// `meta` so a restart does not always resume from zero.
    pub(crate) sweep_contract_cursor: u64,
    /// The wall-clock instant of the last completed sweep fire — the evidence
    /// [`crate::classify_forgiveness`] reads to tell downtime from overload. Durable
    /// in `meta`; a sweep that did not run (a frozen economy) does not stamp it.
    pub(crate) last_sweep_real: i64,
    /// The IC instant of the last completed sweep fire (the time-jump evidence).
    pub(crate) last_sweep_ic: i64,
    /// The wall-clock instant this world's economy last finished booting (E4b1
    /// verify fold, S3). Durable in `meta`, stamped at the end of every boot: a gap
    /// between two sweeps that **contains** this instant provably contains a restart,
    /// which is downtime however short it was — the evidence that stops a 90-second
    /// restart classifying as `Overload` and paging a staffer on the second one.
    pub(crate) last_boot_real: i64,
    /// The accounts **this** recovery latch froze, and only those (E4b1 verify fold,
    /// B4a). Durable in `meta`, accumulated across every boot the latch survives.
    ///
    /// `@econ/unfreeze` clears exactly this list, so an account frozen for its own
    /// reason — a destroyed owner, a balance that would not fit `i64` — stays frozen
    /// and stays visible as a thing needing attention. Without it, recovery was
    /// either "unfreeze every account by hand" or "unfreeze them all and lose the
    /// list of which ones were really broken".
    pub(crate) boot_frozen: Vec<AccountId>,
    /// The whole-economy freeze latch (boot drift / conservation break / a dropped
    /// durable write). While set, [`crate::post`] refuses everything, and so do the
    /// non-recovery control verbs (`@account/open`, `@account/approval`,
    /// `@vault/create`).
    ///
    /// **Durable** (`meta.frozen`, written by `meta_doc`): the boot that
    /// detects drift also heals the drifted projections and persists them, so
    /// nothing in the docs would ever re-derive the freeze — only staff
    /// `@econ/unfreeze` clears it.
    ///
    /// `pub(crate)` since the E4b1 verify fold (B7): a latch any crate could clear by
    /// assignment is not a latch. [`EconomyStore::is_frozen`] reads it and
    /// [`crate::set_economy_frozen`] is the one durable writer.
    pub(crate) frozen: bool,
    /// The wiring-frozen custody provider table (injected after boot, the
    /// `OrgModule::with_predicates` shape). The built-in `ContainerProvider` is
    /// always last.
    pub(crate) providers: CustodyTable,
    /// The ruleset's configurable limits (M1), injected after boot the same way.
    /// Absent configuration is [`EconomyConfig::default`], so a server with no
    /// `[economy]` section behaves exactly as it did before the setting existed.
    pub(crate) config: EconomyConfig,
    /// The wiring-frozen external-fact provider table (E4c, §6.1 item 1). Empty is
    /// the E4b2 behaviour exactly: every `Fact` condition resolves unknown, and
    /// unknown is unsatisfied.
    pub(crate) facts: crate::facts::FactTable,
    /// The wiring-frozen observer table (E4c, §6.1 item 3). Empty means the
    /// compact events are computed and dropped, which is what every test rig that
    /// wires no observer wants.
    pub(crate) observers: crate::observer::ObserverTable,
    /// The system principals allowed to mint a title record through
    /// [`crate::issue_title`] (E4c, §6.1 item 6; D-E3-13). A **frozen table**, not a
    /// capability: a title issuer is a piece of server composition (E5's
    /// `sys:wod.housing`, E7's production output), and nothing a command or a
    /// document can add to. `@asset/title` remains the staff path and is unchanged.
    pub(crate) title_issuers: Arc<std::collections::BTreeSet<mush_org::SystemPrincipalId>>,
    /// The wiring-frozen canon kinds that count as a **producer-declared outcome**
    /// (E4c verify fold, E1). Empty refuses every `canon:` attestor at `Offer`, which
    /// is the fail-closed default until E6b ships `scene.outcome`.
    pub(crate) canon_outcome_kinds: Arc<crate::contract::CanonOutcomeKinds>,
    /// How deep the current observer-emission chain is (RAM-only, §6.1's
    /// re-entrancy note). See [`crate::observer::MAX_OBSERVER_DEPTH`].
    pub(crate) event_depth: u32,
    /// The **canon records this world has seen**, newest last, bounded to
    /// [`CANON_TAIL`] (RAM-only, E4c verify fold E4a).
    ///
    /// Canon evidence is edge-triggered: the observer runs once, when the record
    /// lands, and a contract that was not yet `Active` — or an economy that was
    /// frozen — missed the edge with no way back. This is the economy's own tail of
    /// what it saw, so `@econ/unfreeze` and a contract's activation can re-scan it.
    /// It is deliberately not the canon ledger (which this crate must not depend on)
    /// and it does not survive a restart; a record older than the tail is the
    /// documented limit, and `@contract/resolve <id>=settle` is the answer past it.
    pub(crate) canon_tail: std::collections::VecDeque<crate::attest::CanonRecord>,
    /// The wiring-frozen party capabilities (E6b, DL-11). Empty is the E6a
    /// behaviour exactly: no principal is programmatically acceptable.
    pub(crate) party_caps: Arc<PartyCapTable>,
    /// The wiring-frozen canon read seam (E6b, DL-09). `None` is the pre-E6b
    /// behaviour: the RAM tail alone, and nothing across a restart.
    pub(crate) canon_evidence: crate::attest::CanonEvidenceSeam,
    /// Whether the first sweep after boot has already run DL-09's source re-scan
    /// (RAM-only, once per boot).
    pub(crate) canon_boot_scan_done: bool,
    /// Whether a canon source re-scan is already in flight (E6b verify fold, R11) —
    /// the structural bound on re-entering it through a settlement's own observers.
    pub(crate) canon_rescanning: bool,
    /// E6b (C16): bounded per-actor refusal counters, RAM-only, surfaced by
    /// `@econ/alerts`. Telemetry, never authority.
    pub(crate) refusals: BTreeMap<ObjId, RefusalTally>,
}

/// How many canon records the economy remembers seeing (E4c verify fold, E4a).
pub const CANON_TAIL: usize = 64;

// ---- E6b: B5 — C16, bounded per-actor refusal telemetry -------------------

/// One actor's refusal tally (E6b, C16; 08g §11).
///
/// RAM-only telemetry, **never authority**: nothing reads it to decide anything,
/// and it is deliberately not persisted — a prober's count across a restart is not
/// worth a durable write, and a count that survived one would be a durable record
/// of who looked at what.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct RefusalTally {
    /// Refusals that rendered as **existence hiding** ("No such …"). This is the
    /// number that matters: it is the shape of a probe walking an id space.
    pub hidden: u32,
    /// Every other refusal — a real error the actor could act on.
    pub other: u32,
    /// When the first refusal in this tally happened (wall clock).
    pub first_at: i64,
    /// When the most recent one did.
    pub last_at: i64,
}

/// How many actors the refusal tally tracks at once (E6b, C16). At the cap the
/// **least recently refused** actor is dropped, so a burst from many actors cannot
/// grow the map and the ones still probing stay visible.
pub const MAX_REFUSAL_ACTORS: usize = 256;

/// How many rows `@econ/alerts` prints (E6b, C16).
pub const MAX_REFUSAL_ROWS: usize = 16;

/// How many entity ids one remembered canon record carries (E6b, DL-09). The
/// party-intersection gate can never match more than `MAX_PARTIES` of them, so this
/// is generous by an order of magnitude and still bounded.
pub const MAX_CANON_ENTITIES: usize = 64;

/// One object's last journaled custody movement (E4b2): which vault, which way, and
/// **which principal's post did it**. The depositor is the transaction's `actor`,
/// which is journal authority — never a claim on the contract record.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CustodyMark {
    /// The vault involved.
    pub vault: VaultId,
    /// Which way it moved.
    pub direction: crate::record::CustodyDirection,
    /// Who moved it (the transaction's actor).
    pub by: mush_org::PrincipalId,
    /// When (wall clock).
    pub at: i64,
}

/// Everything [`crate::boot`] derives, handed to [`EconomyStore::from_parts`] as one
/// value rather than a dozen positional arguments (E4b1 doubled the count).
pub(crate) struct BootParts {
    pub(crate) snapshot: EconomySnapshot,
    pub(crate) journal: Ledger,
    pub(crate) transactions: BTreeMap<TransactionId, EconomicTransaction>,
    pub(crate) tx_index: BTreeMap<TransactionId, TxRef>,
    pub(crate) fold: crate::fold::FoldState,
    pub(crate) chunk_firsts: BTreeMap<u64, u64>,
    pub(crate) escalations: BTreeMap<EscalationId, Escalation>,
    pub(crate) checkpoint: Option<crate::checkpoint::CheckpointMeta>,
    pub(crate) checkpoint_keys: std::collections::BTreeSet<SmolStr>,
    pub(crate) boot_from_checkpoint: Option<u64>,
    pub(crate) boot_replayed: usize,
    pub(crate) boot_checkpoint_ms: u64,
    pub(crate) next_account: u64,
    pub(crate) next_transaction: u64,
    pub(crate) next_vault: u64,
    pub(crate) next_approval: u64,
    pub(crate) next_hold: u64,
    pub(crate) next_obligation: u64,
    pub(crate) next_cadence: u64,
    pub(crate) next_escalation: u64,
    pub(crate) next_contract: u64,
    pub(crate) debit_window: BTreeMap<AccountId, WindowTail>,
    pub(crate) notices: BTreeMap<ObjId, Vec<crate::notice::PendingNotice>>,
    pub(crate) sweep_contract_cursor: u64,
    pub(crate) last_sweep_real: i64,
    pub(crate) last_sweep_ic: i64,
    pub(crate) last_boot_real: i64,
    pub(crate) boot_frozen: Vec<AccountId>,
    pub(crate) frozen: bool,
}

impl EconomyStore {
    /// Assemble a store from boot-derived parts (used by [`crate::boot`]). The
    /// provider table is empty here — the module injects the frozen one after boot.
    pub(crate) fn from_parts(p: BootParts) -> EconomyStore {
        EconomyStore {
            boot_replay: None,
            posts_since_boot: 0,
            snapshot: p.snapshot,
            journal: p.journal,
            transactions: p.transactions,
            tx_index: p.tx_index,
            fold: p.fold,
            chunk_firsts: p.chunk_firsts,
            pending_reads: std::collections::HashMap::new(),
            checkpoint: p.checkpoint,
            checkpoint_keys: p.checkpoint_keys,
            boot_from_checkpoint: p.boot_from_checkpoint,
            boot_replayed: p.boot_replayed,
            boot_checkpoint_ms: p.boot_checkpoint_ms,
            escalations: p.escalations,
            next_account: p.next_account,
            next_transaction: p.next_transaction,
            next_vault: p.next_vault,
            next_approval: p.next_approval,
            next_hold: p.next_hold,
            next_obligation: p.next_obligation,
            next_cadence: p.next_cadence,
            next_escalation: p.next_escalation,
            next_contract: p.next_contract,
            sweep_contract_cursor: p.sweep_contract_cursor,
            hold_expiry_failures: BTreeMap::new(),
            obligation_defers: BTreeMap::new(),
            debit_window: p.debit_window,
            notices: p.notices,
            last_sweep_real: p.last_sweep_real,
            last_sweep_ic: p.last_sweep_ic,
            last_boot_real: p.last_boot_real,
            boot_frozen: p.boot_frozen,
            frozen: p.frozen,
            providers: CustodyTable::default(),
            config: EconomyConfig::default(),
            facts: crate::facts::FactTable::default(),
            observers: crate::observer::ObserverTable::default(),
            title_issuers: Arc::default(),
            canon_outcome_kinds: Arc::default(),
            event_depth: 0,
            canon_tail: std::collections::VecDeque::new(),
            party_caps: Arc::default(),
            canon_evidence: None,
            canon_boot_scan_done: false,
            canon_rescanning: false,
            refusals: BTreeMap::new(),
        }
    }

    /// The authoritative snapshot (read seam).
    #[must_use]
    pub fn snapshot(&self) -> &EconomySnapshot {
        &self.snapshot
    }
    /// The double-entry journal (read seam, B7). The tail has exactly one mutator —
    /// `api::install_commit` — and that is the whole reachability argument its two
    /// post-move assertions rest on.
    #[must_use]
    pub fn journal(&self) -> &Ledger {
        &self.journal
    }
    /// Whether the economy is frozen.
    #[must_use]
    pub fn is_frozen(&self) -> bool {
        self.frozen
    }
    /// The ruleset's configurable limits (M1) — read **live**, never captured, so a
    /// lowered mint cap binds the very next settlement.
    #[must_use]
    pub fn config(&self) -> &EconomyConfig {
        &self.config
    }
    /// A transaction's **body** by id, while it is in the RAM tail (J2): a
    /// reversal target, a just-posted transaction's effects. `None` for an id
    /// that never existed *and* for one whose body left the tail — tell the two
    /// apart with [`EconomyStore::transaction_ref`].
    #[must_use]
    pub fn transaction(&self, id: TransactionId) -> Option<&EconomicTransaction> {
        self.transactions.get(&id)
    }
    /// The RAM tail in id order (staff audit; the newest
    /// [`EconomyConfig::tx_tail_max`] transactions).
    pub fn transactions(&self) -> impl Iterator<Item = &EconomicTransaction> {
        self.transactions.values()
    }
    /// What the index keeps for **every** transaction ever posted (J2), or
    /// `None` for an id the journal never had.
    #[must_use]
    pub fn transaction_ref(&self, id: TransactionId) -> Option<&TxRef> {
        self.tx_index.get(&id)
    }
    /// How many transactions the RAM tail holds.
    #[must_use]
    pub fn tail_len(&self) -> usize {
        self.transactions.len()
    }
    /// The journal sequence the RAM tail starts at, or `None` when it is empty.
    #[must_use]
    pub fn tail_floor_seq(&self) -> Option<u64> {
        self.transactions.values().next().map(|t| t.journal_seq)
    }
    /// J3: the checkpoint this store last wrote or booted from.
    #[must_use]
    pub fn checkpoint(&self) -> Option<&crate::checkpoint::CheckpointMeta> {
        self.checkpoint.as_ref()
    }
    /// The limit the sweep evicts the tail down to.
    #[must_use]
    pub fn tx_tail_max(&self) -> usize {
        if self.config.tx_tail_max == 0 {
            DEFAULT_TX_TAIL_MAX
        } else {
            self.config.tx_tail_max
        }
    }
    /// How many `Created` title effects the journal holds for `object` (J2):
    /// the generation the next issue mints under.
    #[must_use]
    pub fn created_count(&self, object: ObjId) -> u64 {
        self.fold.created_count.get(&object).copied().unwrap_or(0)
    }
    /// The chunk of the journal that holds sequence `seq`, from the first-seq
    /// map (J2); `None` before any entry.
    #[must_use]
    pub fn chunk_of_seq(&self, seq: u64) -> Option<u64> {
        self.chunk_firsts
            .iter()
            .rev()
            .find(|(_, first)| **first <= seq)
            .map(|(chunk, _)| *chunk)
    }
    /// Record that `seq` opened `chunk` (the first entry a chunk received): the
    /// post path calls it with the open chunk the append went into.
    pub(crate) fn note_chunk(&mut self, chunk: u64, seq: u64) {
        self.chunk_firsts.entry(chunk).or_insert(seq);
    }
    /// **Evict the RAM tail down to its limit**, oldest first (J2) — the sweep's
    /// job, never the post path's, so a burst of posts costs no eviction work
    /// in the step that posted them. The index keeps every id.
    pub(crate) fn evict_tail(&mut self) -> usize {
        let max = self.tx_tail_max();
        let mut evicted = 0usize;
        while self.transactions.len() > max {
            if self.transactions.pop_first().is_none() {
                break;
            }
            evicted += 1;
        }
        evicted
    }

    /// What the cold journal replay cost at the last boot (H3), or `None`
    /// before the module has booted.
    #[must_use]
    pub fn boot_replay(&self) -> Option<ReplayStats> {
        self.boot_replay
    }

    /// Journal transactions posted since this boot (doc 12 D-12-1).
    #[must_use]
    pub const fn posts_since_boot(&self) -> u64 {
        self.posts_since_boot
    }

    /// How many transactions the journal holds (its length, replayed plus
    /// posted).
    #[must_use]
    pub fn journal_len(&self) -> u64 {
        self.journal.next_seq(JOURNAL_LOG)
    }

    /// Mint the next account id, or `None` at the ceiling (E4b1 verify fold, B2).
    ///
    /// Every allocator here used to be `self.next += 1`, which **panics in a debug
    /// build and silently wraps in a release one** at `u64::MAX` — and one of them
    /// (`alloc_escalation`) is called from the sweep, where a panic aborts the world
    /// task. An exhausted allocator is not a reason to lose the world: the mint
    /// fails, the caller logs loudly and refuses that one operation, and everything
    /// else keeps running.
    pub fn alloc_account(&mut self) -> Option<AccountId> {
        let id = AccountId(self.next_account);
        self.next_account = self.next_account.checked_add(1)?;
        Some(id)
    }
    /// Mint the next vault id (E3b1), or `None` at the ceiling.
    pub fn alloc_vault(&mut self) -> Option<VaultId> {
        let id = VaultId(self.next_vault);
        self.next_vault = self.next_vault.checked_add(1)?;
        Some(id)
    }
    /// Mint the next approval-request id (E3b2), or `None` at the ceiling.
    pub fn alloc_approval(&mut self) -> Option<ApprovalRequestId> {
        let id = ApprovalRequestId(self.next_approval);
        self.next_approval = self.next_approval.checked_add(1)?;
        Some(id)
    }
    /// The wiring-frozen custody provider table (an `Arc` clone — cheap).
    #[must_use]
    pub(crate) fn providers(&self) -> CustodyTable {
        self.providers.clone()
    }
    /// The wiring-frozen fact provider table (an `Arc` clone — cheap), so a
    /// `ValidateEnv` can be built while the world is borrowed (E4c).
    #[must_use]
    pub(crate) fn facts(&self) -> crate::facts::FactTable {
        self.facts.clone()
    }
    /// The wiring-frozen observer table (an `Arc` clone — cheap), so observers can
    /// be driven with `&mut Ctx` after the store borrow ends.
    #[must_use]
    pub(crate) fn observers(&self) -> crate::observer::ObserverTable {
        self.observers.clone()
    }
    /// Whether `issuer` was registered as a title issuer at wiring (E4c).
    #[must_use]
    pub(crate) fn is_title_issuer(&self, issuer: &mush_org::SystemPrincipalId) -> bool {
        self.title_issuers.contains(issuer)
    }
    /// The wiring-frozen canon outcome kinds (an `Arc` clone — cheap), so a
    /// structural check can run while the world is borrowed (E4c verify fold, E1).
    #[must_use]
    pub(crate) fn canon_outcome_kinds(&self) -> Arc<crate::contract::CanonOutcomeKinds> {
        self.canon_outcome_kinds.clone()
    }
    /// Whether the wiring named `principal` as programmatically acceptable (E6b,
    /// DL-11). Composition data: nothing a command or a document can change.
    #[must_use]
    pub(crate) fn has_party_cap(
        &self,
        principal: &mush_org::PrincipalId,
        cap: crate::contract::PartyCap,
    ) -> bool {
        self.party_caps
            .get(&crate::contract::principal_token(principal))
            .is_some_and(|caps| caps.contains(&cap))
    }
    /// Count one refusal against `actor` (E6b, C16). Bounded by
    /// [`MAX_REFUSAL_ACTORS`]: at the cap the least recently refused actor is
    /// evicted, so the map is O(1) in the world's size and the actors still
    /// probing are the ones that stay.
    pub(crate) fn note_refusal(&mut self, actor: ObjId, hidden: bool, now: i64) {
        if !self.refusals.contains_key(&actor)
            && self.refusals.len() >= MAX_REFUSAL_ACTORS
            && let Some(stale) = self
                .refusals
                .iter()
                .min_by_key(|(id, t)| (t.last_at, id.0))
                .map(|(id, _)| *id)
        {
            self.refusals.remove(&stale);
        }
        let tally = self.refusals.entry(actor).or_insert(RefusalTally {
            hidden: 0,
            other: 0,
            first_at: now,
            last_at: now,
        });
        if hidden {
            tally.hidden = tally.hidden.saturating_add(1);
        } else {
            tally.other = tally.other.saturating_add(1);
        }
        tally.last_at = now;
    }

    /// The refusal tallies, busiest first (E6b, C16) — what `@econ/alerts` prints.
    #[must_use]
    pub fn refusal_tallies(&self) -> Vec<(ObjId, RefusalTally)> {
        let mut rows: Vec<(ObjId, RefusalTally)> =
            self.refusals.iter().map(|(id, t)| (*id, *t)).collect();
        rows.sort_by_key(|(id, t)| {
            (
                std::cmp::Reverse(t.hidden),
                std::cmp::Reverse(t.other),
                id.0,
            )
        });
        rows
    }

    /// Remember one canon record the world announced (E4c verify fold, E4a).
    pub(crate) fn note_canon(
        &mut self,
        id: u64,
        kind: &str,
        source: &str,
        entities: &[ObjId],
        at: i64,
    ) {
        if self.canon_tail.iter().any(|seen| seen.canon_id == id) {
            return;
        }
        while self.canon_tail.len() >= CANON_TAIL {
            self.canon_tail.pop_front();
        }
        // E6b (DL-09): the tail carries the record's **entities** too, so the
        // recovery path runs the same party-intersection gate the live path runs.
        // Bounded like everything else the economy remembers: a record naming an
        // implausible crowd contributes its first `MAX_CANON_ENTITIES` names, which
        // is far more than `MAX_PARTIES` can ever intersect.
        let mut entities: Vec<ObjId> = entities.to_vec();
        entities.truncate(MAX_CANON_ENTITIES);
        self.canon_tail.push_back(crate::attest::CanonRecord {
            canon_id: id,
            kind: SmolStr::new(kind),
            source: SmolStr::new(source),
            entities,
            at,
        });
    }
    /// The canon records this world has seen, oldest first (E4c verify fold, E4a).
    #[must_use]
    pub(crate) fn canon_tail(&self) -> Vec<crate::attest::CanonRecord> {
        self.canon_tail.iter().cloned().collect()
    }
    /// The wiring-frozen canon evidence seam (E6b, DL-09), or `None` when the
    /// composition wired none — which is the pre-E6b behaviour exactly.
    #[must_use]
    pub(crate) fn canon_evidence(
        &self,
    ) -> Option<std::sync::Arc<dyn crate::attest::CanonEvidence>> {
        self.canon_evidence.clone()
    }
    /// The **next contract id** without allocating it (E6b, B8).
    ///
    /// A canon-routed run has to name itself: the seal's source is
    /// `run:{contract id}`, and a `scene:{uid}` is unknowable when the run is
    /// offered. The condition is frozen at offer, so the poster must be able to
    /// write the id the offer is about to mint. The world is single-threaded, so
    /// this is exact; a mismatch would only ever mean the condition never matches,
    /// never that the wrong one does.
    #[must_use]
    pub fn peek_next_contract(&self) -> u64 {
        self.next_contract
    }
    /// The next transaction id WITHOUT allocating it (a unique idempotency-key seed
    /// for interactive commands).
    #[must_use]
    pub fn peek_next_transaction(&self) -> u64 {
        self.next_transaction
    }
    /// Mint the next obligation id (E4b1), or `None` at the ceiling.
    pub fn alloc_obligation(&mut self) -> Option<ObligationId> {
        let id = ObligationId(self.next_obligation);
        self.next_obligation = self.next_obligation.checked_add(1)?;
        Some(id)
    }
    /// Mint the next cadence-clock id (E4b1), or `None` at the ceiling.
    pub fn alloc_cadence(&mut self) -> Option<CadenceId> {
        let id = CadenceId(self.next_cadence);
        self.next_cadence = self.next_cadence.checked_add(1)?;
        Some(id)
    }
    /// Mint the next escalation id (E4b1, §4.6), or `None` at the ceiling.
    ///
    /// This is the one allocator the **sweep** calls, so it is the one whose
    /// overflow would have taken the world task down (B2). A `page` that cannot mint
    /// an id still suspends its subject, with a `Suspended { Staff }` reason and a
    /// loud log — the subject stops either way, which is the part that protects
    /// money.
    pub fn alloc_escalation(&mut self) -> Option<EscalationId> {
        let id = EscalationId(self.next_escalation);
        self.next_escalation = self.next_escalation.checked_add(1)?;
        Some(id)
    }
    /// Mint the next contract id (E4b2, §5), or `None` at the ceiling — the same
    /// shape as every other allocator since the E4b1 verify fold (B2): a caller at
    /// the ceiling logs loudly and refuses that one operation.
    pub fn alloc_contract(&mut self) -> Option<ContractId> {
        let id = ContractId(self.next_contract);
        self.next_contract = self.next_contract.checked_add(1)?;
        Some(id)
    }
    /// Where the journal last saw `object`'s custody, and who moved it (E4b2).
    #[must_use]
    pub(crate) fn last_custody(&self, object: ObjId) -> Option<&CustodyMark> {
        self.fold.last_custody.get(&object)
    }
    /// One escalation record.
    #[must_use]
    pub fn escalation(&self, id: EscalationId) -> Option<&Escalation> {
        self.escalations.get(&id)
    }
    /// Every escalation record, in id order.
    pub fn escalations(&self) -> impl Iterator<Item = &Escalation> {
        self.escalations.values()
    }
    /// Install (or replace) an escalation record in RAM. The doc write is
    /// [`crate::api::persist_escalation`]'s; both always happen together.
    pub(crate) fn put_escalation(&mut self, e: Escalation) {
        self.escalations.insert(e.id, e);
    }
    /// When a terminal hold became terminal, if this store knows (GC evidence).
    #[must_use]
    pub(crate) fn hold_terminal_at(&self, id: HoldId) -> Option<i64> {
        self.fold.hold_terminal_at.get(&id).copied()
    }
    /// The last completed sweep's two stamps (wall, IC).
    #[must_use]
    pub fn last_sweep(&self) -> (i64, i64) {
        (self.last_sweep_real, self.last_sweep_ic)
    }
    /// When this economy last finished booting (S3's classifier evidence).
    #[must_use]
    pub fn last_boot(&self) -> i64 {
        self.last_boot_real
    }
    /// Whether the gap since the last recorded sweep **contains a boot** — the S3
    /// evidence handed to [`crate::classify_forgiveness`].
    #[must_use]
    pub fn gap_spans_boot(&self) -> bool {
        self.last_boot_real > self.last_sweep_real
    }
    /// The accounts the recovery latch froze, in id order (B4a) — exactly what
    /// `@econ/unfreeze` restores.
    pub fn boot_frozen(&self) -> impl Iterator<Item = AccountId> + '_ {
        self.boot_frozen.iter().copied()
    }
    /// Forget the boot-freeze list (`@econ/unfreeze` has just restored it).
    pub(crate) fn clear_boot_frozen(&mut self) {
        self.boot_frozen.clear();
    }
    /// How many consecutive sweep fires have failed to expire this reservation (S6).
    #[must_use]
    pub fn expiry_refusals(&self, id: HoldId) -> u32 {
        self.hold_expiry_failures.get(&id).copied().unwrap_or(0)
    }
    /// How many consecutive sweep fires have deferred this obligation (S7).
    #[must_use]
    pub fn deferred_fires(&self, id: ObligationId) -> u32 {
        self.obligation_defers.get(&id).copied().unwrap_or(0)
    }
    /// The next hold id validation may mint. E4b1 resumes `next_hold` from the
    /// **journal's** hold effects at boot (never from surviving docs, which GC
    /// deletes); the snapshot floor stays as the belt, so a stale allocator can never
    /// aim a placement at a live reservation even mid-plan.
    #[must_use]
    pub fn peek_next_hold(&self) -> u64 {
        self.next_hold.max(self.snapshot.next_hold_floor())
    }
    /// The allocator resume document — high-water marks boot never lowers, plus the
    /// **durable** whole-economy freeze latch.
    ///
    /// `frozen` lives here because a latch that only exists in RAM is not a latch:
    /// the boot that detects drift freezes, heals the drifted docs to the journal,
    /// persists them — and the NEXT boot then reads a clean world and comes up
    /// spending. Boot ORs the stored value into what it derives (a read tolerant of
    /// the field's absence, for pre-E3 meta docs), and only staff `@econ/unfreeze`
    /// clears it.
    pub(crate) fn meta_doc(&self) -> serde_json::Value {
        serde_json::json!({
            "next_account_id": self.next_account,
            "next_transaction_id": self.next_transaction,
            "next_vault_id": self.next_vault,
            "next_approval_id": self.next_approval,
            "next_hold_id": self.next_hold,
            "next_obligation_id": self.next_obligation,
            "next_cadence_id": self.next_cadence,
            "next_escalation_id": self.next_escalation,
            "next_contract_id": self.next_contract,
            "sweep_contract_cursor": self.sweep_contract_cursor,
            "last_sweep_real": self.last_sweep_real,
            "last_sweep_ic": self.last_sweep_ic,
            "last_boot_real": self.last_boot_real,
            "boot_frozen": self.boot_frozen.iter().map(|a| a.0).collect::<Vec<u64>>(),
            "frozen": self.frozen,
        })
    }

    /// Record a posted transaction into the RAM indexes (called by `apply`-adjacent
    /// module code after the pure apply advances the snapshot).
    ///
    /// E4b1 also advances the hold allocator past every reservation this transaction
    /// **placed**, and stamps the GC clock of every one it made terminal. The
    /// allocator floor may never come from the surviving docs (GC deletes terminal
    /// ones), so it is maintained here and at boot from the journal alone.
    pub(crate) fn index_transaction(&mut self, tx: EconomicTransaction) {
        self.posts_since_boot = self.posts_since_boot.saturating_add(1);
        // J2: the index keeps what the tail's eviction must not lose.
        self.tx_index.insert(
            tx.id,
            TxRef {
                seq: tx.journal_seq,
                idempotency: tx.idempotency.clone(),
                posted_at: tx.posted_at,
            },
        );
        // J3: the journal-authority shadow, folded by the one function boot uses.
        self.fold.fold_entry(tx.journal_seq, &tx);
        // E6a step 1 (D-E6-21): the rolling-window tail, the same fold boot replays.
        // D4: only an account whose live rule carries a window is tracked at all —
        // every other one is dropped again immediately, so the index is O(windowed
        // accounts) and not O(accounts anything ever debited).
        for account in note_window_debits(&mut self.debit_window, &tx) {
            if !account_windowed(&self.snapshot, account) {
                self.debit_window.remove(&account);
            }
        }
        // E4b1: the hold allocator advances past every reservation placed; the
        // terminal times and the custody marks are the shadow's now.
        for effect in &tx.hold_effects {
            if effect.kind == crate::record::HoldEffectKind::Placed {
                self.next_hold = self.next_hold.max(effect.hold.id.0.saturating_add(1));
            }
        }
        self.transactions.insert(tx.id, tx);
    }

    /// Stamp the end of a sweep fire (§4.3) — the classifier's evidence.
    pub(crate) fn stamp_sweep(&mut self, real: i64, ic: i64) {
        self.last_sweep_real = real;
        self.last_sweep_ic = ic;
    }

    /// The notices queued for `who` while they were away (E6a step 1, DL-04), oldest
    /// first.
    #[must_use]
    pub fn pending_notices(&self, who: ObjId) -> Vec<crate::notice::PendingNotice> {
        self.notices.get(&who).cloned().unwrap_or_default()
    }

    /// What `account` has spent inside the trailing `period_secs` ending at `now`
    /// (E6a step 1, D-E6-21): the sum of its [`EconomyStore::debit_window`] entries
    /// with `posted_at > now − period_secs`. An unrepresentable sum saturates, which
    /// is over any window — fail closed.
    #[must_use]
    pub fn spent_in_window(&self, account: AccountId, now: i64, period_secs: i64) -> Amount {
        let Some(tail) = self.debit_window.get(&account) else {
            return Amount::ZERO;
        };
        // D4: a tail the per-account cap latched is no longer a sum. Unknown spend is
        // over every window — fail closed, so the next post asks a human.
        if tail.saturated {
            return Amount::from_minor(i64::MAX);
        }
        let since = now.saturating_sub(period_secs);
        let sum: i128 = tail
            .entries
            .iter()
            .filter(|(at, _)| *at > since)
            .map(|(_, amount)| amount.widen())
            .sum();
        Amount::from_widened(sum).unwrap_or(Amount::from_minor(i64::MAX))
    }

    /// One account's rolling-window tail (the tests and `@econ/audit`).
    #[must_use]
    pub fn debit_window_of(&self, account: AccountId) -> Option<&WindowTail> {
        self.debit_window.get(&account)
    }

    /// **Drop what no window can still cover, and forget the untracked** (D4) — the
    /// read-side half of the bound, run from the sweep's GC so a world that stops
    /// posting still stops holding tails. Pruning at index time alone left an account
    /// that was debited once and never again holding that entry forever.
    pub(crate) fn prune_debit_windows(&mut self, now: i64) {
        let snapshot = &self.snapshot;
        self.debit_window
            .retain(|account, _| account_windowed(snapshot, *account));
        let cutoff = now.saturating_sub(crate::record::MAX_APPROVAL_WINDOW_SECS);
        for tail in self.debit_window.values_mut() {
            // R3's rule again: never prune below the tail's own high-water mark, so a
            // clock that ran ahead and came back cannot forget a live spend.
            let high_water = tail.entries.back().map_or(now, |(at, _)| (*at).max(now));
            tail.prune(
                cutoff.min(high_water.saturating_sub(crate::record::MAX_APPROVAL_WINDOW_SECS)),
            );
        }
        self.debit_window.retain(|_, tail| !tail.entries.is_empty());
    }

    /// **Re-derive one account's tail from the journal** (D4): what
    /// `@account/approval <id>=… window=…` calls the moment a rule gains a window.
    ///
    /// The window is journal-derived, so a rule that starts carrying one must start
    /// carrying the spend the journal already records — otherwise a treasury could be
    /// emptied and then given a window, and the window would read the account as
    /// having spent nothing. The tail is folded from the transactions inside
    /// [`crate::MAX_APPROVAL_WINDOW_SECS`] of `now`, by the same
    /// [`note_window_debits`] live posting uses.
    pub(crate) fn rebuild_debit_window(&mut self, account: AccountId, now: i64) {
        self.debit_window.remove(&account);
        if !account_windowed(&self.snapshot, account) {
            return;
        }
        let floor = now.saturating_sub(crate::record::MAX_APPROVAL_WINDOW_SECS);
        // J2-D1 (doc 12): the fold runs over the RAM tail. When the tail starts
        // after the window floor, older spend is not counted — an approval window
        // is a spend control, not a conservation rule — and the gap is said.
        if let Some(oldest) = self.transactions.values().next()
            && oldest.posted_at > floor
        {
            tracing::warn!(
                account = account.0,
                tail_starts_at = oldest.posted_at,
                floor,
                "economy: the debit window was rebuilt from the RAM tail, which starts after \
                 the window floor; older spend is not counted (doc 12 J2-D1)"
            );
        }
        let mut tails: BTreeMap<AccountId, WindowTail> = BTreeMap::new();
        for tx in self.transactions.values() {
            if tx.posted_at < floor {
                continue;
            }
            let touches = tx.lines.iter().any(|l| l.account == account)
                || tx.hold_effects.iter().any(|e| e.hold.account == account);
            if !touches {
                continue;
            }
            note_window_debits(&mut tails, tx);
        }
        if let Some(tail) = tails.remove(&account) {
            self.debit_window.insert(account, tail);
        }
    }
}

/// The most entries one account's rolling-window tail keeps (E6a step 1 verify fold,
/// D4). Four thousand posts inside one account's window is far past any legitimate
/// wallet and well short of anything that costs memory; past it the tail stops being
/// a sum and starts being a **latch** — see [`WindowTail::saturated`].
pub const MAX_WINDOW_ENTRIES: usize = 4_096;

/// One account's rolling-window tail: `(posted_at, debit)` oldest first, plus the
/// fail-closed latch the per-account cap sets.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WindowTail {
    /// The entries, oldest first, each `(posted_at, debit)`.
    pub(crate) entries: std::collections::VecDeque<(i64, Amount)>,
    /// Set when [`MAX_WINDOW_ENTRIES`] forced an entry out that the window may still
    /// have covered. The sum is then no longer knowable, and an unknown spend is
    /// **over** every window: [`EconomyStore::spent_in_window`] answers `i64::MAX`,
    /// so the next post needs an approval a human gives. Cleared the moment the tail
    /// drains back under the cap on its own (every retained entry is then real).
    pub(crate) saturated: bool,
}

impl WindowTail {
    /// Drop entries at or before `cutoff` from the front, clearing the latch once the
    /// tail is honest again.
    fn prune(&mut self, cutoff: i64) {
        while self.entries.front().is_some_and(|(at, _)| *at < cutoff) {
            self.entries.pop_front();
        }
        if self.saturated && self.entries.len() < MAX_WINDOW_ENTRIES {
            self.saturated = false;
        }
    }

    /// How many entries it holds (for the tests and `@econ/audit`).
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether it holds nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Whether the cap latched this tail closed.
    #[must_use]
    pub fn is_saturated(&self) -> bool {
        self.saturated
    }
}

/// Whether `account`'s live rule carries a rolling window at all (D4) — the one
/// predicate that decides who is tracked.
#[must_use]
pub(crate) fn account_windowed(snapshot: &EconomySnapshot, account: AccountId) -> bool {
    snapshot
        .account(account)
        .and_then(|a| a.approval.as_ref())
        .is_some_and(|rule| rule.window.is_some())
}

/// Fold one transaction into the rolling-window tails (E6a step 1, D-E6-21) — the
/// ONE place the per-account debit an [`crate::ApprovalWindow`] counts is defined,
/// shared by [`EconomyStore::index_transaction`] and boot's replay so the two can
/// never disagree. Returns the accounts it touched, so the caller can drop the ones
/// whose rule carries no window (D4).
///
/// Per account: the negative lines plus the placed holds, **less what the same
/// transaction captured** (the placement already counted; a settlement that captures
/// escrow is not a second spend). Releases and expiries subtract nothing — a refund
/// never re-opens the window.
///
/// Two bounds, both from the E6a step 1 verify fold:
///
/// - **R3: nothing is dropped at index time.** The front used to be trimmed against
///   *this post's* `posted_at − 90 d`, so a single post carrying a wall clock far in
///   the future erased the account's whole history permanently — and replay repeated
///   it, so a reboot did not heal it either. Age is a question about **now**, and a
///   post's own clock is not now: the tail is bounded by *length* here and pruned by
///   age on the read side ([`EconomyStore::prune_debit_windows`], against the world's
///   clock and never below the tail's own high-water mark).
/// - **D4, the length cap.** Past [`MAX_WINDOW_ENTRIES`] the oldest entry is dropped
///   and the tail latches [`WindowTail::saturated`]: the sum is unknown, and unknown
///   reads as over every window.
pub(crate) fn note_window_debits(
    window: &mut BTreeMap<AccountId, WindowTail>,
    tx: &EconomicTransaction,
) -> Vec<AccountId> {
    let mut debits: BTreeMap<AccountId, i128> = BTreeMap::new();
    for line in &tx.lines {
        if line.delta.is_negative() {
            *debits.entry(line.account).or_insert(0) -= line.delta.widen();
        }
    }
    for effect in &tx.hold_effects {
        match effect.kind {
            crate::record::HoldEffectKind::Placed => {
                *debits.entry(effect.hold.account).or_insert(0) += effect.hold.amount.widen();
            }
            crate::record::HoldEffectKind::Captured => {
                if let crate::record::HoldStatus::Consumed { amount, .. } = &effect.hold.status {
                    *debits.entry(effect.hold.account).or_insert(0) -= amount.widen();
                }
            }
            crate::record::HoldEffectKind::Released | crate::record::HoldEffectKind::Expired => {}
        }
    }
    let mut touched: Vec<AccountId> = Vec::with_capacity(debits.len());
    for (account, debit) in debits {
        touched.push(account);
        if debit <= 0 {
            continue;
        }
        let amount = Amount::from_widened(debit).unwrap_or(Amount::from_minor(i64::MAX));
        let tail = window.entry(account).or_default();
        while tail.entries.len() >= MAX_WINDOW_ENTRIES {
            tail.entries.pop_front();
            tail.saturated = true;
        }
        tail.entries.push_back((tx.posted_at, amount));
    }
    touched
}

/// The economy [`GameModule`]. Hand one to `Engine::with_modules` (after the org
/// module — economy authorizes through it).
#[derive(Debug, Default)]
pub struct EconomyModule {
    providers: CustodyTable,
    config: EconomyConfig,
    facts: crate::facts::FactTable,
    observers: crate::observer::ObserverTable,
    title_issuers: Arc<std::collections::BTreeSet<mush_org::SystemPrincipalId>>,
    canon_outcome_kinds: Arc<crate::contract::CanonOutcomeKinds>,
    /// E6b (DL-09): the bounded canon read seam, so a seal committed while the
    /// economy was frozen — or before a restart — is still found.
    canon_evidence: crate::attest::CanonEvidenceSeam,
    /// E6b (DL-11): the seventh wiring-frozen table — which organization
    /// principals may have a contract acceptance performed programmatically.
    party_caps: Arc<PartyCapTable>,
}

impl EconomyModule {
    /// A fresh module with only the built-in container custody provider.
    #[must_use]
    pub fn new() -> EconomyModule {
        EconomyModule::with_custody(Vec::new())
    }

    /// A module with a wiring-frozen custody provider table (E3c hands in
    /// `mush-sim`'s interior provider). The built-in [`ContainerProvider`] is
    /// **always appended last**, so a wired provider wins the routing race for
    /// backings it handles and the container fallback is the last resort.
    #[must_use]
    pub fn with_custody(mut providers: Vec<Arc<dyn CustodyProvider>>) -> EconomyModule {
        providers.push(Arc::new(ContainerProvider));
        EconomyModule {
            providers: Arc::new(providers),
            ..EconomyModule::default()
        }
    }

    /// Wire the ruleset's configurable limits (M1). Omitting this is
    /// [`EconomyConfig::default`] — the pre-M1 behaviour, with the default mint cap.
    #[must_use]
    pub fn with_config(mut self, config: EconomyConfig) -> EconomyModule {
        self.config = config;
        self
    }

    /// Wire the external-fact providers (E4c, §6.1 item 1) — `mush-jobs` today,
    /// `mush-sim`/`mush-tactics` for doc 10 §15's boundary facts. Omitting this
    /// leaves every [`crate::TypedCondition::Fact`] resolving *unknown*, which is
    /// unsatisfied: a `Fact`-gated contract simply never matures.
    ///
    /// # Panics
    ///
    /// If two providers claim the same namespace (E4c verify fold, E8). The table is
    /// routed first-match-wins, so a duplicate silently shadows: the second `jobs`
    /// provider would answer nothing, every `Fact` condition it owns would resolve
    /// *unknown*, and the contracts gated on them would wait forever with no error
    /// anywhere. This is composition-root data — a wiring mistake a developer makes
    /// once, at startup, with the fix in the panic message — so it is loud.
    #[must_use]
    pub fn with_facts(
        mut self,
        providers: Vec<Arc<dyn crate::facts::FactProvider>>,
    ) -> EconomyModule {
        {
            let mut seen: std::collections::BTreeSet<SmolStr> = std::collections::BTreeSet::new();
            for p in &providers {
                assert!(
                    seen.insert(SmolStr::new(p.provider())),
                    "economy: two fact providers claim the namespace {:?}; the table routes \
                     first-match-wins, so the second would silently answer nothing",
                    p.provider()
                );
            }
        }
        self.facts = Arc::new(providers);
        self
    }

    /// Wire the canon kinds that count as a **producer-declared outcome** (E4c
    /// verify fold, E1). Omitting this refuses every `canon:` attestor set at
    /// `Offer`, which is the fail-closed default; E6b registers exactly
    /// `scene.outcome`, which `mush-scenes` writes only under a **staff-conferred
    /// sealer** — never a self-granted facilitator (M14, D-E6-7).
    #[must_use]
    pub fn with_canon_outcome_kinds(mut self, kinds: Vec<SmolStr>) -> EconomyModule {
        self.canon_outcome_kinds = Arc::new(kinds.into_iter().collect());
        self
    }

    /// Wire the post-commit observers (E4c, §6.1 item 3) — `mush-server`'s canon
    /// evidence bridge today. Omitting this computes no events at all.
    #[must_use]
    pub fn with_observers(
        mut self,
        observers: Vec<Arc<dyn crate::observer::EconomyObserver>>,
    ) -> EconomyModule {
        self.observers = Arc::new(observers);
        self
    }

    /// Wire the system principals allowed to mint title through
    /// [`crate::issue_title`] (E4c, §6.1 item 6). Omitting this refuses every
    /// issuer, which is the fail-closed default D-E3-13 asked for.
    #[must_use]
    pub fn with_title_issuers(
        mut self,
        issuers: Vec<mush_org::SystemPrincipalId>,
    ) -> EconomyModule {
        self.title_issuers = Arc::new(issuers.into_iter().collect());
        self
    }

    /// Wire the **canon read seam** (E6b, DL-09). Omitting it leaves the RAM tail
    /// as the only recovery, which loses every record older than
    /// [`CANON_TAIL`] and every record at all across a restart.
    #[must_use]
    pub fn with_canon_evidence(
        mut self,
        evidence: Arc<dyn crate::attest::CanonEvidence>,
    ) -> EconomyModule {
        self.canon_evidence = Some(evidence);
        self
    }

    // ---- E6b: B4 — DL-11's vendor accept -------------------------------

    /// Wire the **party capabilities** (E6b, DL-11 / 08f §2 item 7, M15): which
    /// organization principals may have a contract **acceptance** performed
    /// programmatically, because nobody is ever connected to do it by hand.
    ///
    /// A staff-built NPC fence org has no `acts_for` holder at all — no member, no
    /// grantee — and `acting_party` takes the `acts_for` route for an organization
    /// and nothing else, which is what would otherwise put a staffer in every
    /// single deal. This table is the exception, and it is exactly as narrow as it
    /// sounds:
    ///
    /// - it is **composition-frozen** — ids only, set at wiring, and nothing a
    ///   command, a document or a grant can add to;
    /// - it grants **no money authority whatever**. The accept still runs
    ///   `accept_core`, still places escrow under the org account's own debit
    ///   authority, and still refuses on `InsufficientAvailable` — which is exactly
    ///   why "a fence that runs out stops quoting" needs no code at all (M19,
    ///   D-E6-15);
    /// - it is reachable only from [`crate::accept_contract_as`], never from
    ///   `@contract/accept`, so no player acquires the power to bind an org.
    ///
    /// Omitting it is the fail-closed default: no principal is programmatically
    /// acceptable, which is exactly E6a's behaviour.
    #[must_use]
    pub fn with_party_caps(
        mut self,
        caps: Vec<(mush_org::PrincipalId, crate::contract::PartyCap)>,
    ) -> EconomyModule {
        let mut table: PartyCapTable = std::collections::BTreeMap::new();
        for (principal, cap) in caps.into_iter().take(MAX_PARTY_CAPS) {
            table
                .entry(crate::contract::principal_token(&principal))
                .or_default()
                .insert(cap);
        }
        self.party_caps = Arc::new(table);
        self
    }
}

/// The wiring-frozen party-capability table (E6b, DL-11), keyed by
/// [`crate::contract::principal_key`] because `PrincipalId` is `Hash`-but-not-`Ord`.
pub type PartyCapTable =
    std::collections::BTreeMap<SmolStr, std::collections::BTreeSet<crate::contract::PartyCap>>;

/// How many principals the wiring may name (E6b, DL-11). A composition-frozen
/// table is small by construction; the cap says so out loud.
pub const MAX_PARTY_CAPS: usize = 64;

impl GameModule for EconomyModule {
    fn name(&self) -> &'static str {
        NS
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.classify_commands(mush_core::CommandCompatibility::EconomyCarveOut);
        reg.state(EconomyStore::default());
        // J2: `@econ/history` and `@econ/reverse` reach past the RAM tail through
        // the store-read seam (H1); this is where the page comes back.
        reg.store_read_completion(Arc::new(crate::commands::EconomyArchiveRead))?;
        reg.subscribe(WorldEventKind::ObjDestroyed, Arc::new(on_obj_destroyed));
        // E4c §6.1 item 2: the canon attestation observer. The economy subscribes
        // to **core's** event, never to `mush-canon` — the dependency runs the other
        // way, and canon's own RAM-tail-only queryability does not matter because
        // the attestation this records is durable on the contract document.
        reg.subscribe(
            WorldEventKind::CanonRecorded,
            Arc::new(crate::attest::on_canon_recorded),
        );
        reg.destroy_gate(Arc::new(EconomicDestroyGate));
        // E6a step 1 (DL-04): the notices an org's officers missed while away are
        // rendered at their next connect (the housing `notice.rs` pattern).
        reg.hook(
            mush_core::HookPoint::Connected,
            crate::notice::reconnect_hook(),
        );
        // SP1 (D-LBK-7 as decided): one composition-frozen `ClockRef::Registered`
        // provider per `[economy.cadences]` stanza, under `cadence.<name>`. The list is
        // validated whole first; a runtime cadence registers nothing.
        crate::declared::register(reg, &self.config)?;
        // The ONE economy tick (D-E4-3): holds, obligations and GC on a single
        // bounded `Coarse` sweep. The core scheduler is in-RAM and re-armed from
        // scratch at boot, so there are deliberately no per-record timers — every
        // catch-up is computed from schedule arithmetic, never from a tick count.
        reg.tick(
            crate::sweep::SWEEP_TAG,
            mush_core::module::TickCadence::Coarse(std::time::Duration::from_secs(
                crate::SWEEP_PERIOD_SECS.unsigned_abs(),
            )),
            Arc::new(crate::sweep::sweep),
        )?;
        crate::commands::register(reg)?;
        Ok(())
    }

    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        data: ModuleBootData,
    ) -> Result<(), ModError> {
        let fetch_ms = data.fetch_ms;
        let started = std::time::Instant::now();
        let tail_max = if self.config.tx_tail_max == 0 {
            DEFAULT_TX_TAIL_MAX
        } else {
            self.config.tx_tail_max
        };
        let mut store = boot::load_boot(
            ctx,
            data,
            &self.providers,
            &self.canon_outcome_kinds,
            tail_max,
        )?;
        // C17 (08g §12.1 item 5): the cold replay is the whole boot cost until a
        // durability checkpoint exists. Say what it cost, every boot, and say loudly
        // when it crosses the line the owner drew.
        let elapsed = started.elapsed();
        // J2-F2: the journal's length, not the RAM tail's — the tail is bounded
        // now, and the C17 line, the per-entry cost and the harness all count
        // what was replayed.
        let entries = usize::try_from(store.journal_len()).unwrap_or(usize::MAX);
        let elapsed_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
        let from_checkpoint = store.boot_from_checkpoint;
        let replayed = store.boot_replayed;
        let checkpoint_ms = store.boot_checkpoint_ms;
        store.boot_replay = Some(ReplayStats {
            entries,
            elapsed_ms,
            fetch_ms,
            from_checkpoint,
            replayed,
            checkpoint_ms,
        });
        tracing::info!(
            entries,
            elapsed_ms,
            fetch_ms,
            from_checkpoint = ?from_checkpoint,
            replayed,
            checkpoint_ms,
            "economy: journal replayed at boot"
        );
        if elapsed.as_secs() > CHECKPOINT_REVIEW_SECS || entries > CHECKPOINT_REVIEW_ENTRIES {
            tracing::warn!(
                entries,
                elapsed_ms,
                fetch_ms,
                "economy: the cold journal replay crossed the checkpoint-review threshold \
                 ({CHECKPOINT_REVIEW_SECS} s or {CHECKPOINT_REVIEW_ENTRIES} entries): per 08g \
                 §12.1 item 5 the durability checkpoint becomes the next cut's first item"
            );
        }
        // Inject the wiring-frozen tables and config into the freshly booted slot.
        store.providers = self.providers.clone();
        store.config = self.config.clone();
        store.facts = self.facts.clone();
        store.observers = self.observers.clone();
        store.title_issuers = self.title_issuers.clone();
        store.canon_outcome_kinds = self.canon_outcome_kinds.clone();
        store.party_caps = self.party_caps.clone();
        store.canon_evidence = self.canon_evidence.clone();
        *ctx.w
            .module_state_mut::<EconomyStore>()
            .expect("economy store slot") = store;
        // SP1 (D-LBK-7): adopt or create every declared cadence, now that the store is
        // live and before any layered module (housing, the WoD layer) looks its
        // campaign clock up by name.
        crate::declared::reconcile(ctx, &self.config);
        // E6a step 1 verify fold (R5): boot opens its own level-2 records — a
        // quarantined contract, an obligation whose account went away — and announced
        // none of them, because only the ladder's `page` announces and boot does not
        // call it. Announce the open set now that the observers are wired; no staffer
        // is connected at boot, so the server's fan-out records nothing and R4's
        // connect-time re-announcement is what actually delivers them.
        crate::sweep::announce_open_escalations(ctx, crate::escalation::MAX_BOOT_ESCALATIONS);
        Ok(())
    }
}

// ---- the destroy gate (§4.9) ----------------------------------------------

/// The pre-destroy veto: an object carrying (or containing) live economic weight
/// is not destroyable. Refusing here — at the command layer, before
/// `World::destroy` marks the husk `GOING` and disperses its contents — is the
/// only place that can still see what a vault holds; the `ObjDestroyed` backstop
/// and the boot passes are for destruction that bypasses `@destroy`.
///
/// The message is **neutral for non-staff** (§4.5 existence hiding) and detailed
/// for staff.
#[derive(Debug)]
struct EconomicDestroyGate;

impl mush_core::behavior::DestroyGate for EconomicDestroyGate {
    fn check(
        &self,
        w: &mush_core::World,
        _reg: &mush_core::Registry,
        actor: ObjId,
        target: ObjId,
    ) -> Result<(), String> {
        let Some(store) = w.module_state::<EconomyStore>() else {
            return Ok(());
        };
        let mut reasons: Vec<String> = Vec::new();
        for id in crate::api::subtree(w, target) {
            reasons.extend(economic_holds(w, store, id));
        }
        if reasons.is_empty() {
            return Ok(());
        }
        if crate::api::is_staff(w, actor) {
            Err(format!(
                "Economic records block that destruction: {}",
                reasons.join("; ")
            ))
        } else {
            Err("You cannot destroy that right now.".to_string())
        }
    }
}

/// Every reason `id` may not be destroyed: its own title, a vault it backs, and
/// the accounts/titles it holds as a principal (§15.3's gates).
fn economic_holds(w: &mush_core::World, store: &EconomyStore, id: ObjId) -> Vec<String> {
    use crate::record::{AccountStatus, AssetLifecycle, CustodyRef, VaultStatus};
    let mut out = Vec::new();
    let snap = store.snapshot();
    // (a) it IS a titled asset.
    if snap
        .asset(id)
        .is_some_and(|a| a.lifecycle != AssetLifecycle::WrittenOff)
    {
        out.push(format!("#{} holds a live title record", id.0));
    }
    // (b) it backs a vault.
    for backing in [CustodyRef::Container(id), CustodyRef::SpatialInterior(id)] {
        let Some(v) = snap.vault_by_backing(&backing) else {
            continue;
        };
        match v.status {
            VaultStatus::Active => out.push(format!("#{} backs active vault #{}", id.0, v.id)),
            VaultStatus::Frozen => {
                let held = crate::custody::resolve(&store.providers, w, &v.backing, None)
                    .ok()
                    .and_then(|p| p.contents(w, &v.backing))
                    .is_none_or(|c| !c.is_empty());
                if held {
                    out.push(format!(
                        "#{} backs frozen vault #{}, which still holds things",
                        id.0, v.id
                    ));
                }
            }
            VaultStatus::Closed => {}
        }
    }
    // (c) it is a principal with money or titles.
    let principal = mush_org::PrincipalId::Object(id);
    for a in snap.accounts() {
        if a.owner == principal && !a.posted_balance.is_zero() && a.status != AccountStatus::Closed
        {
            out.push(format!(
                "#{} owns account #{} with a non-zero balance",
                id.0, a.id
            ));
        }
    }
    let titled: Vec<String> = snap
        .assets_of(&principal)
        .filter(|a| a.lifecycle != AssetLifecycle::WrittenOff)
        .map(|a| format!("#{}", a.object.0))
        .collect();
    if !titled.is_empty() {
        out.push(format!("#{} holds title to {}", id.0, titled.join(", ")));
    }
    out
}

/// `ObjDestroyed` observer: the programmatic-destruction backstop (§4.9).
///
/// FREEZE (never delete) every account a destroyed object owned — money is
/// preserved for staff resolution; a frozen account takes no new posting. (ObjIds
/// are never reused, so no impostor can inherit it.) E3b1 extends that to title:
/// every asset **on** the dead object, **owned by** it, **assigned to** it, or
/// **naming** it as creditor/claimant latches `frozen`, and every vault it backed
/// goes `Frozen`. These are **doc writes, not journal effects** — a reduction of
/// authority, never a title move. The observer runs after `World::destroy` has
/// already dispersed contents, so what a vault *lost* is caught by the boot
/// custody reconciliation instead.
fn on_obj_destroyed(ctx: &mut Ctx<'_>, ev: &WorldEvent) {
    use crate::record::{AccountStatus, CustodyRef, Encumbrance, VaultStatus};
    let WorldEvent::ObjDestroyed { obj } = ev else {
        return;
    };
    let obj = *obj;
    let principal = mush_org::PrincipalId::Object(obj);
    let (accounts, assets, vaults): (Vec<Account>, Vec<crate::record::EconomicAsset>, Vec<Vault>) = {
        let Some(store) = ctx.w.module_state_mut::<EconomyStore>() else {
            return;
        };
        let ids: Vec<AccountId> = store
            .snapshot
            .accounts()
            .filter(|a| a.owner == principal)
            .filter(|a| a.status == AccountStatus::Active)
            .map(|a| a.id)
            .collect();
        let mut accounts = Vec::new();
        for id in ids {
            if let Some(a) = store.snapshot.account_mut(id) {
                // E3's split, applied here too (E4b1 verify fold, B8): `status` is
                // doc authority and `revision` is JOURNAL authority (`apply` bumps it
                // once per touched account per entry, and boot re-derives it as
                // `1 + touches`). Bumping it from a doc-authority write made the
                // healthy doc claim a revision its own `last_journal_seq` never
                // produced — the exact shape `set_account_status` and
                // `set_asset_frozen` already refuse, and the asset arm below is the
                // model. Boot's `max` hid the damage; an `expected_revisions` guard
                // written against the pre-freeze value did not.
                a.status = AccountStatus::Frozen;
                accounts.push(a.clone());
            }
        }
        let names = |e: &Encumbrance| match e {
            Encumbrance::Lien { creditor, .. } => *creditor == principal,
            Encumbrance::Disputed { claimant, .. } => *claimant == principal,
            Encumbrance::Lease { lessor, .. } => *lessor == principal,
            // Collateral names no principal of its own — the contract does, and a
            // destroyed party is E4b2's contract reconciliation, not an asset freeze.
            Encumbrance::Collateral { .. } => false,
        };
        let touched: Vec<ObjId> = store
            .snapshot
            .assets()
            .filter(|a| !a.frozen)
            .filter(|a| {
                a.object == obj
                    || a.title_owner == principal
                    || a.assignment
                        .as_ref()
                        .is_some_and(|x| x.assigned_to == principal)
                    || a.encumbrances.iter().any(names)
            })
            .map(|a| a.object)
            .collect();
        let mut assets = Vec::new();
        for id in touched {
            if let Some(a) = store.snapshot.asset_mut(id) {
                // The latch is doc authority: it must NOT disturb a journal-authority
                // field. Bumping `revision` here would make the healthy doc disagree
                // with the journal at its own claimed sequence, and the next boot
                // would read the freeze itself as drift (D-E3-1 / C2).
                a.frozen = true;
                a.frozen_reason = Some(SmolStr::new_static("object-destroyed"));
                assets.push(a.clone());
            }
        }
        let backed: Vec<VaultId> = [CustodyRef::Container(obj), CustodyRef::SpatialInterior(obj)]
            .iter()
            .filter_map(|b| store.snapshot.vault_by_backing(b))
            .filter(|v| v.status == VaultStatus::Active)
            .map(|v| v.id)
            .collect();
        let mut vaults = Vec::new();
        for id in backed {
            if let Some(v) = store.snapshot.vault_mut(id) {
                v.status = VaultStatus::Frozen;
                v.revision += 1;
                vaults.push(v.clone());
            }
        }
        (accounts, assets, vaults)
    };
    for a in &accounts {
        crate::api::persist_account(ctx.out, a);
    }
    for a in &assets {
        crate::api::persist_asset(ctx.out, a);
    }
    for v in &vaults {
        crate::api::persist_vault(ctx.out, v);
    }
}

// ---- the org scope-use binder (§4.8.2) ------------------------------------

/// Answers `mush-org`'s `@org/scope retire` question — *does the economy still bind
/// this compartment?* — through the wiring-frozen [`mush_org::ScopeUseProvider`]
/// table.
///
/// The dependency runs economy → org, so org cannot look for an account itself; it
/// asks, exactly as an org `AccessPolicy` asks faction about standing. Retiring a
/// bound scope would strand the records pointing at it: after E3b2 a retired scope
/// authorizes nobody, so a live org account under it becomes unspendable by anyone
/// but staff.
#[derive(Debug)]
pub struct EconomyScopeUse;

impl mush_org::ScopeUseProvider for EconomyScopeUse {
    fn binder(&self) -> &str {
        "the economy"
    }

    fn scope_in_use(&self, w: &mush_core::World, scope: mush_org::OrgScopeId) -> Option<String> {
        use crate::record::{AccountStatus, AssetLifecycle, VaultStatus};
        let store = w.module_state::<EconomyStore>()?;
        let snap = store.snapshot();
        if let Some(a) = snap
            .accounts()
            .find(|a| a.scope == Some(scope) && a.status != AccountStatus::Closed)
        {
            return Some(format!("account #{} is still open on it", a.id));
        }
        if let Some(x) = snap
            .assets()
            .find(|x| x.scope == Some(scope) && x.lifecycle != AssetLifecycle::WrittenOff)
        {
            return Some(format!("asset #{} is still titled to it", x.object.0));
        }
        if let Some(v) = snap
            .vaults()
            .find(|v| v.scope == Some(scope) && v.status != VaultStatus::Closed)
        {
            return Some(format!("vault #{} is still bound to it", v.id));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{InstrumentId, TransactionId};
    use crate::record::{EconomicCause, EconomicVisibility, JournalLine};

    /// One posted transaction debiting `account` by `amount` at `at`.
    fn debit(id: u64, account: AccountId, amount: i64, at: i64) -> EconomicTransaction {
        EconomicTransaction {
            id: TransactionId(id),
            journal_seq: id,
            idempotency: crate::IdempotencyKey::new("test", format!("k{id}")),
            plan_hash: [0u8; 32],
            actor: mush_org::PrincipalId::Object(ObjId(1)),
            description: String::new(),
            cause: EconomicCause::Command { serial: id },
            visibility: EconomicVisibility::default(),
            posted_at: at,
            lines: vec![JournalLine {
                account,
                instrument: InstrumentId::new("test").expect("instrument"),
                delta: Amount::from_minor(-amount),
                memo: smol_str::SmolStr::new_static("t"),
            }],
            asset_effects: Vec::new(),
            custody_effects: Vec::new(),
            hold_effects: Vec::new(),
            reverses: None,
            consumed_approval: None,
        }
    }

    /// E6a step 1 verify fold, D4 — the per-account length cap **fails closed**.
    ///
    /// Four thousand posts inside one account's window is far past any legitimate
    /// wallet, and the tail has to stay bounded whatever a world does. Past the cap
    /// the sum is no longer knowable, and an unknown spend must read as *over* every
    /// window rather than under it: the alternative is a silent budget hole that opens
    /// exactly on the busiest account in the game.
    #[test]
    fn vf_d4_the_window_length_cap_saturates_fail_closed() {
        let account = AccountId(7);
        let mut window: BTreeMap<AccountId, WindowTail> = BTreeMap::new();
        for i in 0..u64::try_from(MAX_WINDOW_ENTRIES).unwrap_or(u64::MAX) {
            let tx = debit(i, account, 1, 1_000 + i64::try_from(i).unwrap_or(0));
            let touched = note_window_debits(&mut window, &tx);
            assert_eq!(touched, vec![account]);
        }
        let tail = window.get(&account).expect("the tail");
        assert_eq!(tail.len(), MAX_WINDOW_ENTRIES);
        assert!(!tail.is_saturated(), "the cap latched early");

        // One past the cap: the oldest entry goes and the tail latches.
        let tx = debit(9_999, account, 1, 1_000_000);
        note_window_debits(&mut window, &tx);
        let tail = window.get(&account).expect("the tail");
        assert_eq!(tail.len(), MAX_WINDOW_ENTRIES, "the tail grew past its cap");
        assert!(tail.is_saturated(), "the cap did not latch");

        // A saturated tail reads as over every window — the post needs a human.
        let mut store = EconomyStore {
            debit_window: window,
            ..EconomyStore::default()
        };
        assert_eq!(
            store.spent_in_window(account, 1_000_000, 3_600).minor(),
            i64::MAX,
            "a tail the cap truncated read as a knowable sum"
        );
        // ...and it un-latches once age has drained it back under the cap on its own,
        // because every entry left is then real.
        store.snapshot = crate::snapshot::EconomySnapshot::new();
        store.prune_debit_windows(1_000_000);
        assert!(
            store.debit_window_of(account).is_none(),
            "an account with no window at all stayed indexed"
        );
    }
}
