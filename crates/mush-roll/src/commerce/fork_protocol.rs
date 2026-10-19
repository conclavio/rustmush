//! Closed source-material composition contract for authored fork preparation.
//! The bytes are data, not a licence or a fork authority. Only the Roll builder
//! can request them after actual current-parent resolution. No DTO decodes them.

use super::RevisionIdentity;
use crate::{Revision, RollError, SourceAdapter, WorkingCopy};
use mush_core::mechanics::{DefinitionDescriptor, DefinitionId, DefinitionKind};
use mush_core::{DefinitionRef, ModError};
use serde::Serialize;
use serde_json::json;

#[derive(Serialize)]
pub struct ForkMaterialRequest {
    identity: RevisionIdentity,
}
impl ForkMaterialRequest {
    pub(crate) fn from_parent(parent: &super::parent_grant::CurrentParentGrant) -> Self {
        Self {
            identity: parent.holding().identity(),
        }
    }
    pub fn identity(&self) -> RevisionIdentity {
        self.identity
    }
}

/// Exact immutable bytes from the registered source owner. This type deliberately
/// conveys no grant; the full source/metadata hashes are repeated by the Roll owner.
#[derive(Serialize)]
pub struct ForkMaterial {
    identity: RevisionIdentity,
    source: String,
}
impl ForkMaterial {
    pub fn read_exact<S: SourceAdapter>(
        identity: RevisionIdentity,
        source: &S,
    ) -> Result<Self, RollError> {
        let source = source.read_source(identity.source())?;
        crate::bounded_source(source)?;
        if crate::source_digest(source) != identity.source() {
            return Err(RollError::Missing);
        }
        Ok(Self {
            identity,
            source: source.to_owned(),
        })
    }
    pub(crate) fn check(&self, revision: &Revision) -> Result<(), RollError> {
        crate::bounded_source(&self.source)?;
        if !self
            .identity
            .matches(revision)
            .map_err(|_| RollError::Missing)?
            || crate::source_digest(&self.source) != revision.source()
            || revision.envelope().digest(&self.source)? != revision.reference().digest
        {
            return Err(RollError::Missing);
        }
        Ok(())
    }
    pub(crate) fn working_copy(&self, revision: &Revision) -> Result<WorkingCopy, RollError> {
        self.check(revision)?;
        Ok(WorkingCopy {
            source: self.source.clone(),
            envelope: revision.envelope().clone(),
        })
    }
}

pub fn material_descriptor(manifest: &DefinitionRef) -> Result<DefinitionDescriptor, ModError> {
    // Locator and complete digest are also validated by the owning configured
    // Registry module; no caller selects a source algorithm or ABI override.
    if manifest.key().owner.as_str() != "tacsim"
        || manifest.key().kind.as_str() != "registry_manifest"
        || manifest.key().id.as_str() != "combat"
        || manifest.version() != 1
    {
        return Err(ModError::msg(
            "fork material needs the exact combat manifest",
        ));
    }
    Ok(DefinitionDescriptor::new(
        DefinitionKind::new("owner_read").map_err(|e| ModError::msg(e.to_string()))?,
        DefinitionId::new("authored_fork_material").map_err(|e| ModError::msg(e.to_string()))?,
        1,
        json!({"format":1,"purpose":"authored_fork","exact_source":true,
            "source_authority":false,"manifest":manifest}),
    )
    .map_err(|e| ModError::msg(e.to_string()))?
    .with_references(vec![manifest.clone()]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::{ObjId, mechanics::DefinitionDigest};
    struct Source(String);
    impl SourceAdapter for Source {
        type Token = ();
        fn prepare_source(&self, _: &str, _: &str) -> Result<crate::PreparedSource<()>, RollError> {
            Err(RollError::Missing)
        }
        fn read_source(&self, _: DefinitionDigest) -> Result<&str, RollError> {
            Ok(&self.0)
        }
    }
    #[test]
    fn large_valid_metadata_stays_out_of_request_and_material_is_reverified()
    -> Result<(), Box<dyn std::error::Error>> {
        // Structural schema test only; no grant or compilation is manufactured.
        let source = Source("s".repeat(crate::MAX_SOURCE_BYTES));
        let envelope = crate::Envelope {
            schema: json!({"a":"x".repeat(8192),"b":"y".repeat(8192),"c":"z".repeat(8192)}),
            presets: json!({}),
            profiles: json!({"line":{}}),
            rules: json!({"line":[]}),
            declared_class: "I".into(),
            mind_abi: "fixture".into(),
        };
        let revision = Revision {
            format: crate::FORMAT,
            provenance: None,
            reference: crate::RevisionRef {
                key: crate::RevisionKey {
                    character: ObjId(u32::MAX),
                    mind: u64::MAX,
                    number: u64::MAX,
                },
                digest: envelope.digest(&source.0)?,
            },
            source: crate::source_digest(&source.0),
            envelope,
            notes: String::new(),
        };
        let identity = RevisionIdentity::capture(&revision)?;
        assert!(serde_json::to_vec(&revision)?.len() > 16 * 1024);
        let request = ForkMaterialRequest { identity };
        assert!(
            serde_json::to_vec(&request)?.len() < 512,
            "exact identity fits without a larger core request cap"
        );
        let material = ForkMaterial::read_exact(identity, &source)?;
        material.check(&revision)?;
        assert_eq!(
            material.working_copy(&revision)?.envelope,
            revision.envelope
        );
        assert!(serde_json::to_vec(&material)?.len() < 1024 * 1024);
        assert!(ForkMaterial::read_exact(identity, &Source("changed".into())).is_err());
        let mut changed = revision.clone();
        changed.envelope.declared_class = "II".into();
        assert!(
            material.check(&changed).is_err(),
            "stored envelope must still hash to captured revision"
        );
        let mut crossed = revision;
        crossed.reference.key.number -= 1;
        assert!(
            material.check(&crossed).is_err(),
            "same source is not another immutable revision"
        );
        Ok(())
    }
}
