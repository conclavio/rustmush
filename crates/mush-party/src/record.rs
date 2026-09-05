//! The party record, its caps, and the wire that re-checks every one of them
//! (E6a, 08f §3.1/§3.2/§9).
//!
//! ## What a party is
//!
//! A **roster with a name and a leader**, and nothing else. It holds no money, names
//! no account, carries no authority: [`Party::roles`] are *labels* — "face",
//! "decker" — and a label has never let anybody do anything. That is D-E6-5 written
//! into the record: the party is not an economic principal, so the only economic
//! thing it can hold is a [`ContractRef`] it cannot read.
//!
//! ## The fold rule (E5a A2/A3, applied here)
//!
//! [`Party`] is `Serialize` plus `#[serde(try_from = "PartyWire")]`, so **every**
//! persisted `party:{id}` document arrives through that `TryFrom` and therefore
//! through [`Party::validate`] — the same predicate the constructor runs. There is no
//! "the writer checked it" path: a hand-written document with nine members, a role
//! filed for somebody who is not on the roster, a duplicated member, an invitation
//! for a member, or invites on a disbanded party does not deserialize at all, and
//! boot drops it loudly ([`crate::boot`]) rather than loading it.
//!
//! ## Bounds
//!
//! Every collection and every string is capped (§9), and every cap is re-checked on
//! load. Arithmetic is `saturating_*`/`checked_*` throughout: this record is touched
//! from a tick handler, where a panic takes the world down.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::ObjId;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::error::PartyError;
use crate::ids::{ContractRef, PartyId};

// ---------------------------------------------------------------------------
// Constants (08f §9)
// ---------------------------------------------------------------------------

/// The most **accepted members** one party holds (§9).
pub const MAX_PARTY_SIZE: usize = 8;

/// The most **outstanding invitations** one party holds (§9). Independent of
/// [`MAX_PARTY_SIZE`]: an invitation costs a slot on this list, not on the roster,
/// and the roster cap is checked again when the invitee accepts.
pub const MAX_PARTY_INVITES: usize = 16;

/// The most UTF-8 bytes in a party name (§9).
pub const MAX_PARTY_NAME_BYTES: usize = 48;

/// The most UTF-8 bytes in a role label.
///
/// Not in §9 — added here, because §3.1 gives `roles` a `SmolStr` value and an
/// uncapped stored string is the one shape this crate is not allowed to have. 24
/// bytes fits every label the design names ("face", "decker", "rigger", "wheelman")
/// with room to spare, and a label is a display token, never a sentence.
pub const MAX_PARTY_ROLE_BYTES: usize = 24;

/// Idle expiry: seven days of no membership change and a `Forming` party disbands
/// (§9). Also the ceiling on how long one invitation may stand.
pub const PARTY_IDLE_TTL: i64 = 7 * 24 * 60 * 60;

/// The most **live** parties one player may lead at once.
///
/// Not in §9 — added here so `+party/create` is bounded per actor and not only
/// globally. Four, because the disambiguation rule for the party-less verbs
/// (`+party/leave`, `+party/kick <who>`, …) has to *list* the candidates in a refusal
/// line, and a list of four is a line a person reads; a list of forty is a wall. A
/// party is ephemeral by construction — it expires after
/// [`PARTY_IDLE_TTL`] of silence — so this is a cap on genuine concurrent crews,
/// never on how many runs a player may take in a month. Terminal (disbanded) parties
/// do not count.
pub const MAX_PARTIES_PER_LEADER: usize = 4;

/// The most **live** parties the whole world holds at once.
///
/// Not in §9 — added here so `+party/create` is bounded globally as well as per
/// actor. 4096 is `MAX_PARTIES_PER_LEADER` × 1024 concurrent leaders, comfortably
/// past any population this server expects, and it bounds the expiry tick's scan and
/// [`crate::parties_of`]'s answer at the same time. Terminal parties do not count
/// against it: they are the record of who ran with whom (the shipped
/// ended-tenancy precedent), and nothing periodic ever looks at one again.
pub const MAX_PARTIES: usize = 4096;

// ---------------------------------------------------------------------------
// Records
// ---------------------------------------------------------------------------

/// One outstanding invitation (§3.1).
///
/// `expires_at` is capped at `at + `[`PARTY_IDLE_TTL`]: an invitation may not outlive
/// the party it is an invitation to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invitation {
    /// Who was invited.
    pub who: ObjId,
    /// Who invited them (always the leader at the time).
    pub by: ObjId,
    /// Wall clock at the invitation.
    pub at: i64,
    /// Wall clock the invitation lapses at (≤ `at + `[`PARTY_IDLE_TTL`]).
    pub expires_at: i64,
}

impl Invitation {
    /// An invitation issued at `at`, standing for `PARTY_IDLE_TTL`.
    #[must_use]
    pub fn new(who: ObjId, by: ObjId, at: i64) -> Invitation {
        Invitation {
            who,
            by,
            at,
            expires_at: at.saturating_add(PARTY_IDLE_TTL),
        }
    }

    /// Whether this invitation has lapsed at `now`.
    #[must_use]
    pub fn is_expired(&self, now: i64) -> bool {
        self.expires_at <= now
    }
}

/// Why a party stopped existing (§3.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum DisbandCause {
    /// The leader ran `+party/disband`.
    Disbanded,
    /// The last member left.
    Empty,
    /// Nothing happened to it for [`PARTY_IDLE_TTL`].
    Expired,
    /// Its contract settled (the ruleset's [`crate::release`]).
    Settled,
    /// Its contract ended without settling — cancelled, refunded, expired (the
    /// ruleset's [`crate::release`]).
    Released,
}

impl DisbandCause {
    /// A short human word for a render.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            DisbandCause::Disbanded => "disbanded",
            DisbandCause::Empty => "empty",
            DisbandCause::Expired => "expired",
            DisbandCause::Settled => "settled",
            DisbandCause::Released => "released",
        }
    }
}

/// A party's lifecycle (§3.2).
///
/// `Locked` **freezes the roster**: every membership verb refuses while it holds, so
/// a substitution needs the ruleset to amend the contract and call [`crate::unlock`]
/// first. `Disbanded` is terminal and refuses everything, forever.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PartyStatus {
    /// Assembling: the only status a membership change is allowed in.
    #[default]
    Forming,
    /// Frozen against a contract.
    Locked {
        /// The opaque contract handle the ruleset supplied. Never interpreted here.
        by: ContractRef,
    },
    /// Terminal.
    Disbanded {
        /// Which way out it took.
        how: DisbandCause,
    },
}

impl PartyStatus {
    /// A short human word for a render.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            PartyStatus::Forming => "forming",
            PartyStatus::Locked { .. } => "locked",
            PartyStatus::Disbanded { .. } => "disbanded",
        }
    }
}

/// One party (§3.1).
///
/// Deserializes **only** through its private `PartyWire`, so every invariant below is
/// re-checked on the way in from the store:
///
/// - the name is non-empty, ≤ [`MAX_PARTY_NAME_BYTES`], and free of control characters;
/// - `members` are distinct and ≤ [`MAX_PARTY_SIZE`];
/// - `leader` is a member — required while the party is live; a `Disbanded` party may
///   have an **empty** roster, because that is exactly what `Disbanded { how: Empty }`
///   means (the last member left);
/// - `invited` are distinct by `who`, disjoint from `members`, ≤ [`MAX_PARTY_INVITES`],
///   and none stands longer than [`PARTY_IDLE_TTL`];
/// - `roles` keys ⊆ `members`, and every label is non-empty, control-free and
///   ≤ [`MAX_PARTY_ROLE_BYTES`];
/// - a `Disbanded` party holds **no** invitations.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "PartyWire")]
pub struct Party {
    /// This party's id. Must equal the id in its document key (the DL-07 identity
    /// rule, enforced at boot).
    pub id: PartyId,
    /// The display name.
    pub name: SmolStr,
    /// Who leads. Always a member while the party is live.
    pub leader: ObjId,
    /// Accepted members, ≤ [`MAX_PARTY_SIZE`]. Order is **display only** — the one
    /// place it is load-bearing is the leader hand-off, which takes the first
    /// remaining member.
    pub members: Vec<ObjId>,
    /// Outstanding invitations, ≤ [`MAX_PARTY_INVITES`].
    pub invited: Vec<Invitation>,
    /// Labels on members — "face", "decker". **Never authority**: nothing in this
    /// crate or any other reads a role to decide whether an act is allowed.
    pub roles: BTreeMap<ObjId, SmolStr>,
    /// Where it is in the state machine.
    pub status: PartyStatus,
    /// Idle expiry, refreshed by **any** membership change (create, invite,
    /// uninvite, accept, decline, leave, kick, role) and by [`crate::unlock`].
    pub expires_at: i64,
    /// Bumped by every persisted change.
    pub revision: u64,
}

/// The persisted shape. Nothing constructs a [`Party`] from one except
/// `TryFrom`, which runs [`Party::validate`].
#[derive(Deserialize)]
struct PartyWire {
    id: PartyId,
    name: SmolStr,
    leader: ObjId,
    #[serde(default)]
    members: Vec<ObjId>,
    #[serde(default)]
    invited: Vec<Invitation>,
    #[serde(default)]
    roles: BTreeMap<ObjId, SmolStr>,
    #[serde(default)]
    status: PartyStatus,
    #[serde(default)]
    expires_at: i64,
    #[serde(default)]
    revision: u64,
}

impl TryFrom<PartyWire> for Party {
    type Error = String;

    fn try_from(w: PartyWire) -> Result<Party, String> {
        let p = Party {
            id: w.id,
            name: w.name,
            leader: w.leader,
            members: w.members,
            invited: w.invited,
            roles: w.roles,
            status: w.status,
            expires_at: w.expires_at,
            revision: w.revision,
        };
        p.validate()?;
        Ok(p)
    }
}

impl Party {
    /// A fresh `Forming` party led by, and consisting of, `leader`.
    ///
    /// # Errors
    ///
    /// The first invariant the name breaks, in words: empty, over
    /// [`MAX_PARTY_NAME_BYTES`], or carrying a control character.
    pub fn new(id: PartyId, name: &str, leader: ObjId, now: i64) -> Result<Party, String> {
        let p = Party {
            id,
            name: SmolStr::new(name.trim()),
            leader,
            members: vec![leader],
            invited: Vec::new(),
            roles: BTreeMap::new(),
            status: PartyStatus::Forming,
            expires_at: now.saturating_add(PARTY_IDLE_TTL),
            revision: 0,
        };
        p.validate()?;
        Ok(p)
    }

    /// Re-check every invariant of §3.1 — what the constructor runs and what every
    /// persisted document runs on the way in.
    ///
    /// # Errors
    ///
    /// The first invariant that fails, in words, naming the cap it broke.
    pub fn validate(&self) -> Result<(), String> {
        check_label(&self.name, MAX_PARTY_NAME_BYTES, "a party name")?;

        if self.members.len() > MAX_PARTY_SIZE {
            return Err(format!(
                "a party holds at most {MAX_PARTY_SIZE} members ({} given)",
                self.members.len()
            ));
        }
        let mut seen: BTreeSet<ObjId> = BTreeSet::new();
        for m in &self.members {
            if !seen.insert(*m) {
                return Err(format!("member {m} appears twice on the roster"));
            }
        }
        match self.status {
            PartyStatus::Forming | PartyStatus::Locked { .. } => {
                if !seen.contains(&self.leader) {
                    return Err(format!("leader {} is not on the roster", self.leader));
                }
            }
            // A `Disbanded { how: Empty }` party is precisely one whose roster
            // emptied, so a terminal party is allowed to have no members — and no
            // leader on the list. Any other terminal cause keeps the roster it had.
            PartyStatus::Disbanded { .. } => {
                if !self.members.is_empty() && !seen.contains(&self.leader) {
                    return Err(format!(
                        "leader {} is not on a non-empty disbanded roster",
                        self.leader
                    ));
                }
            }
        }

        if self.invited.len() > MAX_PARTY_INVITES {
            return Err(format!(
                "a party holds at most {MAX_PARTY_INVITES} invitations ({} given)",
                self.invited.len()
            ));
        }
        if matches!(self.status, PartyStatus::Disbanded { .. }) && !self.invited.is_empty() {
            return Err("a disbanded party holds no invitations".to_string());
        }
        let mut invitees: BTreeSet<ObjId> = BTreeSet::new();
        for inv in &self.invited {
            if !invitees.insert(inv.who) {
                return Err(format!("{} is invited twice", inv.who));
            }
            if seen.contains(&inv.who) {
                return Err(format!("member {} is also invited", inv.who));
            }
            if inv.expires_at > inv.at.saturating_add(PARTY_IDLE_TTL) {
                return Err(format!(
                    "an invitation stands at most {PARTY_IDLE_TTL} seconds ({} given)",
                    inv.expires_at.saturating_sub(inv.at)
                ));
            }
        }

        for (who, label) in &self.roles {
            if !seen.contains(who) {
                return Err(format!("{who} holds a role but is not a member"));
            }
            check_label(label, MAX_PARTY_ROLE_BYTES, "a role label")?;
        }
        Ok(())
    }

    // -- reads --------------------------------------------------------------

    /// Whether `who` is an accepted member.
    #[must_use]
    pub fn is_member(&self, who: ObjId) -> bool {
        self.members.contains(&who)
    }

    /// Whether `who` holds an invitation — **any** invitation record, lapsed or not.
    ///
    /// Deliberately expiry-blind, because this is the predicate [`crate::may_view`]
    /// is built on: an invitee who lets their invitation lapse keeps seeing the party
    /// until the expiry tick prunes the record, which is a second of visibility and
    /// never an escalation. `+party/accept` checks [`Invitation::is_expired`]
    /// separately, so a lapsed invitation can be *seen* and never *used*.
    #[must_use]
    pub fn is_invited(&self, who: ObjId) -> bool {
        self.invited.iter().any(|i| i.who == who)
    }

    /// `who`'s standing invitation, if they hold one.
    #[must_use]
    pub fn invitation_of(&self, who: ObjId) -> Option<&Invitation> {
        self.invited.iter().find(|i| i.who == who)
    }

    /// Whether `who` leads.
    #[must_use]
    pub fn is_leader(&self, who: ObjId) -> bool {
        self.leader == who
    }

    /// Whether the roster may still change (`Forming`).
    #[must_use]
    pub fn is_forming(&self) -> bool {
        matches!(self.status, PartyStatus::Forming)
    }

    /// Whether the roster is frozen against a contract.
    #[must_use]
    pub fn is_locked(&self) -> bool {
        matches!(self.status, PartyStatus::Locked { .. })
    }

    /// Whether this party is over. Terminal: nothing moves it again.
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        matches!(self.status, PartyStatus::Disbanded { .. })
    }

    /// Whether this party is still one somebody can be *in* — `Forming` or `Locked`.
    #[must_use]
    pub fn is_live(&self) -> bool {
        !self.is_terminal()
    }

    /// The contract this party is locked against, if it is locked.
    #[must_use]
    pub fn locked_by(&self) -> Option<ContractRef> {
        match self.status {
            PartyStatus::Locked { by } => Some(by),
            PartyStatus::Forming | PartyStatus::Disbanded { .. } => None,
        }
    }

    /// Why this party disbanded, if it did.
    #[must_use]
    pub fn disband_cause(&self) -> Option<DisbandCause> {
        match self.status {
            PartyStatus::Disbanded { how } => Some(how),
            PartyStatus::Forming | PartyStatus::Locked { .. } => None,
        }
    }

    /// `who`'s role label, if they hold one.
    #[must_use]
    pub fn role_of(&self, who: ObjId) -> Option<&SmolStr> {
        self.roles.get(&who)
    }

    /// Whether this party currently admits a **membership change** — the single
    /// predicate `/invite`, `/uninvite`, `/accept`, `/decline`, `/leave`, `/kick`,
    /// `/role` and `/disband` all ask (§3.2).
    ///
    /// # Errors
    ///
    /// - [`PartyError::Locked`] while `Locked`: a locked roster is frozen, and a
    ///   substitution needs the ruleset to amend the contract and call
    ///   [`crate::unlock`] first.
    /// - [`PartyError::State`] once `Disbanded`: terminal refuses everything.
    pub fn check_open(&self) -> Result<(), PartyError> {
        match self.status {
            PartyStatus::Forming => Ok(()),
            PartyStatus::Locked { .. } => Err(PartyError::Locked),
            PartyStatus::Disbanded { .. } => Err(PartyError::State),
        }
    }

    /// Whether one more member fits.
    ///
    /// # Errors
    ///
    /// [`PartyError::TooLarge`] naming the roster cap.
    pub fn check_room_for_member(&self) -> Result<(), PartyError> {
        if self.members.len() >= MAX_PARTY_SIZE {
            return Err(PartyError::TooLarge("party"));
        }
        Ok(())
    }

    /// Whether one more invitation fits.
    ///
    /// # Errors
    ///
    /// [`PartyError::TooLarge`] naming the invitation cap.
    pub fn check_room_for_invite(&self) -> Result<(), PartyError> {
        if self.invited.len() >= MAX_PARTY_INVITES {
            return Err(PartyError::TooLarge("invitations"));
        }
        Ok(())
    }

    // -- writes -------------------------------------------------------------

    /// Refresh the idle clock and bump the revision — what **every** membership
    /// change does, `/role` included.
    ///
    /// `/role` counts as activity on purpose: a leader labelling the crew is a leader
    /// working on the crew, and a party being actively organised is exactly the one
    /// idle expiry must not collect.
    pub fn touch(&mut self, now: i64) {
        self.expires_at = now.saturating_add(PARTY_IDLE_TTL);
        self.revision = self.revision.saturating_add(1);
    }

    /// Bump the revision without touching the idle clock — for changes that are
    /// housekeeping rather than activity (the expiry tick's invitation prune).
    pub fn bump(&mut self) {
        self.revision = self.revision.saturating_add(1);
    }

    /// Take this party terminal, clearing the invitations a disbanded party may not
    /// hold. Idempotent in effect: a second call re-stamps the cause.
    pub fn disband(&mut self, how: DisbandCause) {
        self.status = PartyStatus::Disbanded { how };
        self.invited.clear();
        self.bump();
    }

    /// Drop `who` from the roster, their role label with them, handing the lead on
    /// when they held it and disbanding `Empty` when they were the last.
    ///
    /// Returns `false` when `who` was not a member (nothing changed). The caller
    /// still owns the [`Party::touch`], because a no-op is not activity.
    pub fn remove_member(&mut self, who: ObjId) -> bool {
        let before = self.members.len();
        self.members.retain(|m| *m != who);
        if self.members.len() == before {
            return false;
        }
        self.roles.remove(&who);
        match self.members.first().copied() {
            // The lead passes to the next member in roster order.
            Some(next) => {
                if self.leader == who {
                    self.leader = next;
                }
            }
            // The last member left: the party is over, and the roster is empty by
            // design (§3.2's "last member leaves → Disbanded{Empty}").
            None => self.disband(DisbandCause::Empty),
        }
        true
    }

    /// Drop every lapsed invitation. Returns how many went.
    pub fn prune_invitations(&mut self, now: i64) -> usize {
        let before = self.invited.len();
        self.invited.retain(|i| !i.is_expired(now));
        before.saturating_sub(self.invited.len())
    }
}

/// Whether `s` is acceptable as a stored one-line string: non-empty after trimming,
/// within `cap` UTF-8 bytes, and free of control characters (a name carrying `\n`
/// writes two lines into every render that ever quotes it).
///
/// The one bounded-string check this crate has, run by [`Party::validate`] on the
/// name and every role label, and by `+party/create` / `+party/role` before they
/// build anything — so the refusal a player reads and the refusal a document gets
/// are the same predicate.
///
/// # Errors
///
/// The first rule broken, in words, ready to render to the actor.
pub fn check_label(s: &str, cap: usize, what: &str) -> Result<(), String> {
    if s.trim().is_empty() {
        return Err(format!("{what} may not be empty"));
    }
    if s.len() > cap {
        return Err(format!("{what} is at most {cap} bytes ({} given)", s.len()));
    }
    if s.chars().any(char::is_control) {
        return Err(format!("{what} may not contain control characters"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        DisbandCause, Invitation, MAX_PARTY_INVITES, MAX_PARTY_NAME_BYTES, MAX_PARTY_ROLE_BYTES,
        MAX_PARTY_SIZE, PARTY_IDLE_TTL, Party, PartyStatus,
    };
    use crate::error::PartyError;
    use crate::ids::{ContractRef, PartyId};
    use mush_core::ObjId;
    use smol_str::SmolStr;

    fn party() -> Party {
        Party::new(PartyId(1), "Wrecking Crew", ObjId(10), 1_000).expect("valid")
    }

    fn wire(p: &Party) -> serde_json::Value {
        serde_json::to_value(p).expect("serializes")
    }

    fn reload(v: &serde_json::Value) -> Result<Party, serde_json::Error> {
        serde_json::from_value::<Party>(v.clone())
    }

    #[test]
    fn a_fresh_party_is_forming_with_the_leader_aboard() {
        let p = party();
        assert_eq!(p.status, PartyStatus::Forming);
        assert_eq!(p.members, vec![ObjId(10)]);
        assert!(p.is_member(ObjId(10)));
        assert!(p.is_leader(ObjId(10)));
        assert_eq!(p.expires_at, 1_000 + PARTY_IDLE_TTL);
        assert_eq!(p.revision, 0);
    }

    #[test]
    fn a_valid_party_round_trips_through_the_wire() {
        let mut p = party();
        p.members.push(ObjId(11));
        p.invited.push(Invitation::new(ObjId(12), ObjId(10), 1_000));
        p.roles.insert(ObjId(11), SmolStr::new("decker"));
        p.status = PartyStatus::Locked {
            by: ContractRef(77),
        };
        assert_eq!(reload(&wire(&p)).expect("valid doc"), p);
    }

    #[test]
    fn the_wire_refuses_an_empty_or_oversized_name() {
        let p = party();
        let mut v = wire(&p);
        v["name"] = serde_json::json!("");
        assert!(reload(&v).is_err());
        v["name"] = serde_json::json!("x".repeat(MAX_PARTY_NAME_BYTES + 1));
        assert!(reload(&v).is_err());
        v["name"] = serde_json::json!("two\nlines");
        assert!(reload(&v).is_err());
    }

    #[test]
    fn the_wire_refuses_a_ninth_member() {
        let mut p = party();
        for n in 11..=(10 + u32::try_from(MAX_PARTY_SIZE).expect("small")) {
            p.members.push(ObjId(n));
        }
        // Nine members: the constructor cap re-checked on the way in.
        assert_eq!(p.members.len(), MAX_PARTY_SIZE + 1);
        assert!(reload(&wire(&p)).is_err());
    }

    #[test]
    fn the_wire_refuses_a_duplicate_member() {
        let mut p = party();
        p.members.push(ObjId(10));
        assert!(reload(&wire(&p)).is_err());
    }

    #[test]
    fn the_wire_refuses_a_leader_off_the_roster() {
        let mut p = party();
        p.leader = ObjId(99);
        assert!(reload(&wire(&p)).is_err());
    }

    #[test]
    fn the_wire_refuses_a_role_for_a_non_member() {
        let mut p = party();
        p.roles.insert(ObjId(44), SmolStr::new("face"));
        assert!(reload(&wire(&p)).is_err());
    }

    #[test]
    fn the_wire_refuses_an_oversized_or_empty_role_label() {
        let mut p = party();
        p.roles.insert(
            ObjId(10),
            SmolStr::new("x".repeat(MAX_PARTY_ROLE_BYTES + 1)),
        );
        assert!(reload(&wire(&p)).is_err());
        p.roles.insert(ObjId(10), SmolStr::new(""));
        assert!(reload(&wire(&p)).is_err());
    }

    #[test]
    fn the_wire_refuses_a_seventeenth_invitation_and_a_duplicate_one() {
        let mut p = party();
        for n in 100..=(100 + u32::try_from(MAX_PARTY_INVITES).expect("small")) {
            p.invited.push(Invitation::new(ObjId(n), ObjId(10), 1_000));
        }
        assert_eq!(p.invited.len(), MAX_PARTY_INVITES + 1);
        assert!(reload(&wire(&p)).is_err());

        let mut p = party();
        p.invited.push(Invitation::new(ObjId(12), ObjId(10), 1_000));
        p.invited.push(Invitation::new(ObjId(12), ObjId(10), 1_000));
        assert!(reload(&wire(&p)).is_err());
    }

    #[test]
    fn the_wire_refuses_an_invitation_for_a_member() {
        let mut p = party();
        p.invited.push(Invitation::new(ObjId(10), ObjId(10), 1_000));
        assert!(reload(&wire(&p)).is_err());
    }

    #[test]
    fn the_wire_refuses_an_invitation_that_outlives_the_ttl() {
        let mut p = party();
        let mut inv = Invitation::new(ObjId(12), ObjId(10), 1_000);
        inv.expires_at = inv.expires_at.saturating_add(1);
        p.invited.push(inv);
        assert!(reload(&wire(&p)).is_err());
    }

    #[test]
    fn the_wire_refuses_invitations_on_a_disbanded_party() {
        let mut p = party();
        p.invited.push(Invitation::new(ObjId(12), ObjId(10), 1_000));
        p.status = PartyStatus::Disbanded {
            how: DisbandCause::Disbanded,
        };
        assert!(reload(&wire(&p)).is_err());
    }

    #[test]
    fn an_empty_roster_is_legal_only_once_disbanded() {
        let mut p = party();
        p.members.clear();
        assert!(reload(&wire(&p)).is_err(), "a live party needs its leader");
        p.status = PartyStatus::Disbanded {
            how: DisbandCause::Empty,
        };
        assert!(
            reload(&wire(&p)).is_ok(),
            "Disbanded{{Empty}} has no roster"
        );
    }

    #[test]
    fn removing_the_last_member_disbands_empty_and_removing_the_leader_hands_on() {
        let mut p = party();
        p.members.push(ObjId(11));
        p.roles.insert(ObjId(10), SmolStr::new("face"));
        assert!(p.remove_member(ObjId(10)));
        assert_eq!(p.leader, ObjId(11));
        assert!(p.role_of(ObjId(10)).is_none());
        assert!(p.is_forming());
        assert!(p.remove_member(ObjId(11)));
        assert_eq!(p.disband_cause(), Some(DisbandCause::Empty));
        assert!(p.members.is_empty());
        assert!(!p.remove_member(ObjId(11)), "a no-op reports itself");
    }

    #[test]
    fn check_open_names_locked_and_disbanded_apart() {
        let mut p = party();
        assert_eq!(p.check_open(), Ok(()));
        p.status = PartyStatus::Locked { by: ContractRef(1) };
        assert_eq!(p.check_open(), Err(PartyError::Locked));
        p.disband(DisbandCause::Settled);
        assert_eq!(p.check_open(), Err(PartyError::State));
    }

    #[test]
    fn pruning_drops_only_lapsed_invitations() {
        let mut p = party();
        p.invited.push(Invitation::new(ObjId(12), ObjId(10), 0));
        p.invited.push(Invitation::new(ObjId(13), ObjId(10), 1_000));
        assert_eq!(p.prune_invitations(PARTY_IDLE_TTL), 1);
        assert_eq!(p.invited.len(), 1);
        assert!(p.is_invited(ObjId(13)));
    }

    #[test]
    fn touch_refreshes_the_idle_clock_and_bump_does_not() {
        let mut p = party();
        p.touch(5_000);
        assert_eq!(p.expires_at, 5_000 + PARTY_IDLE_TTL);
        assert_eq!(p.revision, 1);
        p.bump();
        assert_eq!(p.expires_at, 5_000 + PARTY_IDLE_TTL);
        assert_eq!(p.revision, 2);
    }

    #[test]
    fn saturating_arithmetic_never_overflows_the_idle_clock() {
        let mut p = party();
        p.touch(i64::MAX);
        assert_eq!(p.expires_at, i64::MAX);
        let inv = Invitation::new(ObjId(12), ObjId(10), i64::MAX);
        assert_eq!(inv.expires_at, i64::MAX);
    }
}
