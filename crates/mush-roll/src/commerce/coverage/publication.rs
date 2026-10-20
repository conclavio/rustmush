//! Permanent publication obligations derived only from actual Conduct proofs.
//! These source-free rows do not authorize a discount or a new sortie.
use crate::commerce::{
    CharterId, FiledRevisionId, LineId, RevisionIdentity,
    conduct::{ConductRole, ConductTrialProof},
    families::CommerceFamilies,
    rating::RatingRef,
    records::{PublicationError, Record},
    roots::{RequiredRoot, RootAnchor},
};
use mush_core::{
    CommitToken, DefinitionDigest, DefinitionRef, ModuleCtx, ObjId,
    participant::OwnerPrepareContext,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

fn bad(_: impl std::fmt::Display) -> PublicationError {
    PublicationError::Mismatch
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageConductRef {
    trial: CommitToken,
    receipt: DefinitionDigest,
    charter: CharterId,
    line: LineId,
    filed: FiledRevisionId,
    revision: RevisionIdentity,
    rating: RatingRef,
    profile: String,
    role: ConductRole,
    manifest: DefinitionRef,
}
impl CoverageConductRef {
    fn from_proof(p: &ConductTrialProof) -> Self {
        Self {
            trial: p.operation(),
            receipt: p.digest(),
            charter: p.charter(),
            line: p.line(),
            filed: p.filed(),
            revision: p.revision(),
            rating: p.rating(),
            profile: p.profile().into(),
            role: p.role(),
            manifest: p.manifest().clone(),
        }
    }
    pub fn trial(&self) -> CommitToken {
        self.trial
    }
    pub fn receipt(&self) -> DefinitionDigest {
        self.receipt
    }
    pub fn charter(&self) -> CharterId {
        self.charter
    }
    pub fn line(&self) -> LineId {
        self.line
    }
    pub fn filed(&self) -> FiledRevisionId {
        self.filed
    }
    pub fn revision(&self) -> RevisionIdentity {
        self.revision
    }
    pub fn rating(&self) -> RatingRef {
        self.rating
    }
    pub fn profile(&self) -> &str {
        &self.profile
    }
    pub fn role(&self) -> ConductRole {
        self.role
    }
    pub fn manifest(&self) -> &DefinitionRef {
        &self.manifest
    }
}

fn bounded<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<CoverageConductRef>, D::Error> {
    struct Rows;
    impl<'de> serde::de::Visitor<'de> for Rows {
        type Value = Vec<CoverageConductRef>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("one to sixteen Conduct references")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut s: A,
        ) -> Result<Self::Value, A::Error> {
            if s.size_hint().is_some_and(|n| n > 16) {
                return Err(serde::de::Error::custom("Conduct reference bound"));
            }
            let mut rows = Vec::with_capacity(s.size_hint().unwrap_or(0).min(16));
            while let Some(row) = s.next_element()? {
                if rows.len() == 16 {
                    return Err(serde::de::Error::custom("Conduct reference bound"));
                }
                rows.push(row);
            }
            Ok(rows)
        }
    }
    d.deserialize_seq(Rows)
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoveragePublicationRetention {
    format: u16,
    operation: CommitToken,
    action: DefinitionRef,
    protocol: super::CoverageProtocol,
    publication: DefinitionDigest,
    product: DefinitionDigest,
    #[serde(deserialize_with = "bounded")]
    trials: Vec<CoverageConductRef>,
}
impl Record for CoveragePublicationRetention {
    const FAMILY: &'static str = "roll_coverage_publication";
    const MAX_BYTES: usize = 65536;
    fn key(&self) -> String {
        self.operation.to_hex()
    }
    fn validate(&self) -> Result<(), PublicationError> {
        self.protocol.validate()?;
        if self.format != 1
            || self.action != self.protocol.action(0)?
            || self.trials.is_empty()
            || self.trials.len() > 16
            || [self.publication, self.product]
                .iter()
                .any(|d| d.as_bytes() == &[0; 32])
            || self
                .trials
                .iter()
                .map(|r| r.trial)
                .collect::<BTreeSet<_>>()
                .len()
                != self.trials.len()
            || self.trials.iter().any(|r| {
                r.receipt.as_bytes() == &[0; 32]
                    || r.profile.is_empty()
                    || r.profile.len() > 64
                    || r.profile.trim() != r.profile
                    || r.profile.chars().any(char::is_control)
            })
        {
            return Err(PublicationError::Invalid);
        }
        mush_core::canonical_json_bytes_with_limit(
            &serde_json::to_value(self).map_err(bad)?,
            Self::MAX_BYTES,
        )
        .map_err(|_| PublicationError::Capacity)?;
        Ok(())
    }
}
impl CoveragePublicationRetention {
    pub fn operation(&self) -> CommitToken {
        self.operation
    }
    pub fn publication_digest(&self) -> DefinitionDigest {
        self.publication
    }
    pub fn product_digest(&self) -> DefinitionDigest {
        self.product
    }
    pub fn trials(&self) -> &[CoverageConductRef] {
        &self.trials
    }
    pub(super) fn new(
        operation: CommitToken,
        action: DefinitionRef,
        protocol: super::CoverageProtocol,
        publication: DefinitionDigest,
        product: DefinitionDigest,
        proofs: &[ConductTrialProof],
    ) -> Result<Self, PublicationError> {
        let row = Self {
            format: 1,
            operation,
            action,
            protocol,
            publication,
            product,
            trials: proofs.iter().map(CoverageConductRef::from_proof).collect(),
        };
        row.validate()?;
        Ok(row)
    }
    pub(super) fn roots(&self) -> Vec<RequiredRoot> {
        self.trials
            .iter()
            .map(|r| RequiredRoot {
                anchor: RootAnchor::CoveragePublication(self.operation, r.trial, self.publication),
                identity: r.revision,
            })
            .collect()
    }
    pub(super) fn audit(
        &self,
        ctx: &ModuleCtx<'_>,
        f: &CommerceFamilies,
    ) -> Result<(), PublicationError> {
        self.validate()?;
        for r in &self.trials {
            let proof = crate::commerce::conduct::metadata(ctx, f, r.trial)?;
            if r != &CoverageConductRef::from_proof(&proof) {
                return Err(PublicationError::Mismatch);
            }
        }
        Ok(())
    }
}

#[derive(Clone)]
pub(crate) struct PublicationFence {
    action: DefinitionRef,
    operation: CommitToken,
    actor: ObjId,
    utc: i64,
    proofs: Vec<ConductTrialProof>,
}
impl PublicationFence {
    pub(super) fn new(
        action: DefinitionRef,
        operation: CommitToken,
        actor: ObjId,
        utc: i64,
        proofs: Vec<ConductTrialProof>,
    ) -> Self {
        Self {
            action,
            operation,
            actor,
            utc,
            proofs,
        }
    }
    pub(super) fn operation(&self) -> CommitToken {
        self.operation
    }
    pub(super) fn check(&self, ctx: &OwnerPrepareContext<'_>) -> Result<(), PublicationError> {
        if ctx.action() != &self.action
            || ctx.commit_token() != self.operation
            || ctx.invocation().actor() != self.actor
            || ctx.invocation().principal()
                != &(mush_core::action::PrincipalId::Object { id: self.actor })
            || ctx.world().now() < self.utc
            || self.proofs.is_empty()
            || self.proofs.len() > 16
        {
            return Err(PublicationError::Stale);
        }
        for proof in &self.proofs {
            proof.check_history(ctx)?;
        }
        Ok(())
    }
}

pub(crate) fn all(
    ctx: &ModuleCtx<'_>,
    roll: &crate::RollStore,
    f: &CommerceFamilies,
    enabled: bool,
) -> Result<Vec<CoveragePublicationRetention>, PublicationError> {
    let mut rows = Vec::new();
    super::super::audit::scan(ctx, &f.coverage_publications, |row| {
        if !enabled || rows.len() >= 4096 {
            return Err(PublicationError::Unavailable);
        }
        row.audit(ctx, f)?;
        for root in row.roots() {
            roll.check_required_root(&root).map_err(bad)?;
        }
        rows.push(row.clone());
        Ok(())
    })?;
    Ok(rows)
}
pub(crate) fn root(
    ctx: &ModuleCtx<'_>,
    f: &CommerceFamilies,
    operation: CommitToken,
    trial: CommitToken,
    digest: DefinitionDigest,
) -> Result<RevisionIdentity, PublicationError> {
    let row = super::super::audit::read(ctx, &f.coverage_publications, operation.to_hex())?;
    row.audit(ctx, f)?;
    if row.publication != digest {
        return Err(PublicationError::Mismatch);
    }
    row.trials
        .iter()
        .find(|r| r.trial == trial)
        .map(|r| r.revision)
        .ok_or(PublicationError::Missing)
}
