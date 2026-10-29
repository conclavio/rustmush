//! **The seizure receipt family** (doc 17h cut M8d2, S1): every seize and forfeit derived
//! from its exact acknowledged journal entry, replay-verified at boot as the charter
//! transfer's are. The audit is a projection of the journal; it adds no persisted row.
use super::*;
pub(crate) const NAMESPACE: &str = "registry.core_seizure.v1";
const MAX_RECEIPTS: usize = 65_536;

/// What one act did, in the words the receipt keeps.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "step", rename_all = "snake_case", deny_unknown_fields)]
pub enum SeizureStepReceipt {
    /// The claim was placed on the proof of `hull`'s inspection `seq`; `reason` is its key.
    Seize {
        hull: ObjId,
        seq: u64,
        reason: String,
    },
    /// The live seizure `seizure` ended in the title moving to the claimant.
    Forfeit {
        seizure: CommitToken,
        reason: String,
    },
}
impl SeizureStepReceipt {
    #[must_use]
    pub fn reason(&self) -> &str {
        match self {
            Self::Seize { reason, .. } | Self::Forfeit { reason, .. } => reason,
        }
    }
}

/// **One act's receipt**: the commit, the journal entry, the operator who asked, the core and
/// the title it read, the claimant org and the post-title image.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeizureAuditSubject {
    pub commit: CommitToken,
    pub transaction: TransactionId,
    pub journal_seq: u64,
    pub posted_at_utc: i64,
    pub plan_hash: [u8; 32],
    pub operator: ObjId,
    /// The act's World IC second (the seizure's is what `minds.seized` reads).
    pub at_ic: i64,
    pub core: ObjId,
    pub title_revision: u64,
    pub claimant: OrgId,
    pub issuer: SystemPrincipalId,
    pub step: SeizureStepReceipt,
    pub title: EconomicAsset,
}
impl SeizureAuditSubject {
    pub fn validate(&self) -> Result<(), ReceiptAuditError> {
        let claimant = PrincipalId::Organization(self.claimant);
        let claim = disputed(&claimant, self.step.reason());
        let (revisions, owner_ok, claims_ok, step_ok) = match &self.step {
            SeizureStepReceipt::Seize { hull, seq, reason } => (
                1,
                self.title.title_owner != claimant,
                self.title.encumbrances.as_slice() == [claim],
                hull.0 != 0
                    && *hull != self.core
                    && *seq != 0
                    && reason.as_bytes() == inspection_key(*hull, *seq).as_bytes(),
            ),
            SeizureStepReceipt::Forfeit { seizure, .. } => (
                2,
                self.title.title_owner == claimant && self.title.scope.is_none(),
                self.title.encumbrances.is_empty(),
                *seizure != self.commit,
            ),
        };
        if self.operator.0 == 0
            || self.core.0 == 0
            || self.title_revision == 0
            || self.title_revision.checked_add(revisions) != Some(self.title.revision)
            || self.title.object != self.core
            || !owner_ok
            || !claims_ok
            || !step_ok
            || self.title.frozen
            || self.title.frozen_reason.is_some()
            || self.title.stolen.is_some()
            || self.title.lifecycle != AssetLifecycle::Active
            || self.title.last_journal_seq != self.journal_seq
        {
            return Err(ReceiptAuditError::Refused);
        }
        let value = serde_json::to_value(self).map_err(|_| ReceiptAuditError::Refused)?;
        mush_core::canonical_json_bytes_with_limit(&value, 16_384)
            .map_err(|_| ReceiptAuditError::Refused)?;
        Ok(())
    }
}

/// **The receipt audit**: every receipt by commit, and the live seizure of each core still
/// held (`held`, removed by its forfeit). A journal entry outside the family that touches a
/// held core's title marks the audit invalid: only this family may change it.
///
/// Boxed (one pointer in the fold), so the boot's frame does not grow with it (the M7
/// memory-size bound on `load_boot_with_coverage`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SeizureReceiptAudit(Box<AuditState>);
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AuditState {
    receipts: BTreeMap<CommitToken, SeizureAuditSubject>,
    held: BTreeMap<ObjId, CommitToken>,
    complete: bool,
    invalid: bool,
    #[serde(skip)]
    verified_this_boot: bool,
}
impl SeizureReceiptAudit {
    pub(crate) fn uninitialized(&self) -> bool {
        !self.0.complete && !self.0.invalid && self.0.receipts.is_empty() && self.0.held.is_empty()
    }
    pub fn require_complete(&self) -> Result<(), ReceiptAuditError> {
        if !self.0.complete || !self.0.verified_this_boot {
            return Err(ReceiptAuditError::Incomplete);
        }
        if self.0.invalid {
            return Err(ReceiptAuditError::Refused);
        }
        Ok(())
    }
    pub(crate) fn require_absent(&self) -> Result<(), ReceiptAuditError> {
        self.require_complete()?;
        if !self.0.receipts.is_empty() {
            return Err(ReceiptAuditError::Refused);
        }
        Ok(())
    }
    /// Every receipt, in commit order, once the audit is complete.
    ///
    /// # Errors
    /// The audit is not complete this boot, or is invalid.
    pub fn receipts(
        &self,
    ) -> Result<impl ExactSizeIterator<Item = &SeizureAuditSubject>, ReceiptAuditError> {
        self.require_complete()?;
        Ok(self.0.receipts.values())
    }
    /// The receipt of `commit`, once complete.
    ///
    /// # Errors
    /// As [`Self::receipts`].
    pub fn get(
        &self,
        commit: CommitToken,
    ) -> Result<Option<&SeizureAuditSubject>, ReceiptAuditError> {
        self.require_complete()?;
        Ok(self.0.receipts.get(&commit))
    }
    /// The live seizure holding `core`, if any. Not gated on completeness: the prepare
    /// gates on [`Self::may_admit`], which is.
    pub(crate) fn held(&self, core: ObjId) -> Option<&SeizureAuditSubject> {
        self.0.held.get(&core).and_then(|c| self.0.receipts.get(c))
    }
    /// Whether a forfeit of `core` moved its title to `owner` (the impounded read).
    pub(crate) fn forfeited_to(&self, core: ObjId, owner: &PrincipalId) -> bool {
        self.0.receipts.values().any(|r| {
            r.core == core
                && matches!(r.step, SeizureStepReceipt::Forfeit { .. })
                && *owner == PrincipalId::Organization(r.claimant)
        })
    }
    /// Whether `s` is the next legal receipt: a fresh commit; a Seize of a core no seizure
    /// holds; a Forfeit of exactly the live seizure, its claim and title unchanged since.
    pub(crate) fn may_admit(&self, s: &SeizureAuditSubject) -> bool {
        self.require_complete().is_ok() && self.admissible(s)
    }
    /// [`Self::may_admit`]'s rule without the completeness gate, for the replay itself.
    fn admissible(&self, s: &SeizureAuditSubject) -> bool {
        if self.0.receipts.len() >= MAX_RECEIPTS
            || self.0.receipts.contains_key(&s.commit)
            || s.validate().is_err()
        {
            return false;
        }
        match &s.step {
            SeizureStepReceipt::Seize { .. } => !self.0.held.contains_key(&s.core),
            SeizureStepReceipt::Forfeit { seizure, reason } => {
                self.held(s.core).is_some_and(|held| {
                    held.commit == *seizure
                        && held.claimant == s.claimant
                        && held.step.reason() == reason
                        && held.title.revision == s.title_revision
                        && matches!(held.step, SeizureStepReceipt::Seize { .. })
                })
            }
        }
    }
    /// Install an admitted receipt. Infallible: [`Self::may_admit`] held at prepare and the
    /// bind's fence held since.
    pub(super) fn install(&mut self, s: SeizureAuditSubject) {
        match &s.step {
            SeizureStepReceipt::Seize { .. } => {
                self.0.held.insert(s.core, s.commit);
            }
            SeizureStepReceipt::Forfeit { .. } => {
                self.0.held.remove(&s.core);
            }
        }
        self.0.receipts.insert(s.commit, s);
    }
    fn admit_replayed(&mut self, s: SeizureAuditSubject) {
        if !self.0.invalid && self.admissible(&s) {
            self.install(s);
        } else {
            self.0.invalid = true;
        }
    }
    /// Fold one journal entry (boot replay and the live fold call this alike).
    pub(crate) fn observe(&mut self, tx: &EconomicTransaction) {
        let ours = tx.idempotency.namespace == NAMESPACE
            || matches!(&tx.cause, EconomicCause::Ruleset { namespace, .. } if namespace == NAMESPACE);
        if !ours {
            if tx
                .asset_effects
                .iter()
                .any(|e| self.0.held.contains_key(&e.object))
            {
                self.0.invalid = true;
            }
            return;
        }
        match subject_of(tx) {
            Ok(subject) => self.admit_replayed(subject),
            Err(_) => self.0.invalid = true,
        }
    }
    pub(crate) fn finish_replay(&mut self) -> Result<(), ReceiptAuditError> {
        if self.0.invalid || self.0.receipts.len() > MAX_RECEIPTS || !self.held_match() {
            return Err(ReceiptAuditError::Refused);
        }
        self.0.complete = true;
        self.0.verified_this_boot = true;
        Ok(())
    }
    fn held_match(&self) -> bool {
        self.0.held.iter().all(|(core, commit)| {
            self.0.receipts.get(commit).is_some_and(|r| {
                r.core == *core && matches!(r.step, SeizureStepReceipt::Seize { .. })
            })
        })
    }
    pub(crate) fn checkpoint_complete(&self, rows: &[crate::checkpoint::IdemRow]) -> bool {
        // The checkpoint's complete key index proves an omitted inventory empty; a complete
        // flag alone proves neither direction (the charter transfer's rule).
        if self.uninitialized() {
            return !rows.iter().any(|r| r.ns.as_str() == NAMESPACE);
        }
        if !self.0.complete
            || self.0.invalid
            || self.0.receipts.len() > MAX_RECEIPTS
            || !self.held_match()
        {
            return false;
        }
        let mut expected = BTreeMap::new();
        for (key, receipt) in &self.0.receipts {
            if *key != receipt.commit || receipt.validate().is_err() {
                return false;
            }
            expected.insert(key.to_hex(), receipt);
        }
        for row in rows.iter().filter(|r| r.ns.as_str() == NAMESPACE) {
            let Some(receipt) = expected.remove(row.key.as_str()) else {
                return false;
            };
            let hash: String = receipt
                .plan_hash
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            if row.tx != receipt.transaction
                || row.seq != receipt.journal_seq
                || row.posted_at != receipt.posted_at_utc
                || row.plan_hash != hash
            {
                return false;
            }
        }
        expected.is_empty()
    }
    /// **The managed-title index's recovery** (S1): every receipt names a core the index
    /// knows as a managed core birth, and every held core's title is exactly its seizure's
    /// post-image (nothing else could have moved it).
    pub(crate) fn verify_titles(
        &self,
        index: &crate::managed_title::ManagedTitleIndex,
        assets: &BTreeMap<ObjId, EconomicAsset>,
    ) -> Result<(), ReceiptAuditError> {
        self.require_complete()?;
        for receipt in self.0.receipts.values() {
            if index.core(receipt.core).is_none() {
                return Err(ReceiptAuditError::Refused);
            }
        }
        for commit in self.0.held.values() {
            let receipt = self
                .0
                .receipts
                .get(commit)
                .ok_or(ReceiptAuditError::Refused)?;
            if assets.get(&receipt.core) != Some(&receipt.title) {
                return Err(ReceiptAuditError::Refused);
            }
        }
        Ok(())
    }
}

/// The cause's correlation. A Forfeit carries the released claim's reason last (an
/// [`inspection_key`] has no colon), so the receipt names the claim without a pre-image.
pub(super) fn correlation(
    (commit, actor, at_ic): (CommitToken, ObjId, i64),
    claimant: OrgId,
    r: &RegistryCoreSeizure,
    reason: &str,
) -> String {
    match &r.step {
        SeizureStep::Seize(proof) => format!(
            "seize:{}:{}:{}:{}:{}:{}:{}:{}",
            commit, actor.0, r.core.0, r.title_revision, claimant.0, at_ic, proof.hull.0, proof.seq
        ),
        SeizureStep::Forfeit { seizure } => format!(
            "forfeit:{}:{}:{}:{}:{}:{}:{}:{}",
            commit, actor.0, r.core.0, r.title_revision, claimant.0, at_ic, seizure, reason
        ),
    }
}

pub(super) fn subject_of(
    tx: &EconomicTransaction,
) -> Result<SeizureAuditSubject, ReceiptAuditError> {
    let bad = || ReceiptAuditError::Refused;
    if tx.idempotency.namespace != NAMESPACE
        || !tx.lines.is_empty()
        || !tx.hold_effects.is_empty()
        || !tx.custody_effects.is_empty()
        || tx.consumed_approval.is_some()
        || tx.reverses.is_some()
    {
        return Err(bad());
    }
    let commit = CommitToken::from_hex(tx.idempotency.key.as_str()).map_err(|_| bad())?;
    if tx.idempotency.key.as_str() != commit.to_hex() {
        return Err(bad());
    }
    let EconomicCause::Ruleset {
        namespace,
        correlation: raw,
    } = &tx.cause
    else {
        return Err(bad());
    };
    if namespace != NAMESPACE || raw.len() > 512 {
        return Err(bad());
    }
    let mut f = raw.split(':');
    let kind = f.next().ok_or_else(bad)?;
    let raw_commit = f.next().ok_or_else(bad)?;
    let number = |s: Option<&str>| -> Result<u64, ReceiptAuditError> {
        s.ok_or_else(bad)?.parse().map_err(|_| bad())
    };
    let object = |n: u64| u32::try_from(n).map(ObjId).map_err(|_| bad());
    let operator = object(number(f.next())?)?;
    let core = object(number(f.next())?)?;
    let title_revision = number(f.next())?;
    let claimant = OrgId(number(f.next())?);
    let at_ic: i64 = f.next().ok_or_else(bad)?.parse().map_err(|_| bad())?;
    let (step, reason) = match kind {
        "seize" => {
            let hull = object(number(f.next())?)?;
            let seq = number(f.next())?;
            let key = inspection_key(hull, seq);
            (
                SeizureStep::Seize(InspectionProof {
                    key: key.clone(),
                    hull,
                    core,
                    seq,
                }),
                key,
            )
        }
        "forfeit" => (
            SeizureStep::Forfeit {
                seizure: CommitToken::from_hex(f.next().ok_or_else(bad)?).map_err(|_| bad())?,
            },
            f.next().ok_or_else(bad)?.to_owned(),
        ),
        _ => return Err(bad()),
    };
    let request = RegistryCoreSeizure {
        step,
        core,
        title_revision,
    };
    if f.next().is_some()
        || raw_commit != commit.to_hex()
        || raw.as_str() != correlation((commit, operator, at_ic), claimant, &request, &reason)
    {
        return Err(bad());
    }
    let PrincipalId::System(issuer) = &tx.actor else {
        return Err(bad());
    };
    let claim = PrincipalId::Organization(claimant);
    let (step, title) = match (&request.step, tx.asset_effects.as_slice()) {
        (SeizureStep::Seize(proof), [effect])
            if effect.object == core && effect.kind == crate::AssetEffectKind::Encumbered =>
        {
            (
                SeizureStepReceipt::Seize {
                    hull: proof.hull,
                    seq: proof.seq,
                    reason: proof.key.clone(),
                },
                effect.post.clone(),
            )
        }
        (SeizureStep::Forfeit { seizure }, [first, second])
            if first.object == core
                && second.object == core
                && first.kind == crate::AssetEffectKind::Released
                && second.kind == crate::AssetEffectKind::TitleTransferred =>
        {
            // The release's post-image carries no claim; the claim released is the one the
            // correlation names, which the audit matches against the live seizure.
            if first
                .post
                .encumbrances
                .iter()
                .any(|e| e == &disputed(&claim, &reason))
            {
                return Err(bad());
            }
            (
                SeizureStepReceipt::Forfeit {
                    seizure: *seizure,
                    reason: reason.clone(),
                },
                second.post.clone(),
            )
        }
        _ => return Err(bad()),
    };
    let row = SeizureAuditSubject {
        commit,
        transaction: tx.id,
        journal_seq: tx.journal_seq,
        posted_at_utc: tx.posted_at,
        plan_hash: tx.plan_hash,
        operator,
        at_ic,
        core,
        title_revision,
        claimant,
        issuer: issuer.clone(),
        step,
        title,
    };
    row.validate()?;
    Ok(row)
}
