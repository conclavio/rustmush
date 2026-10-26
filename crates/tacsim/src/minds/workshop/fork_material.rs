//! Internal exact immutable source read for the closed Roll fork builder.
//! No public query, alternate actor or editable source parameter reaches it.

use mush_core::mechanics::{DefinitionKind, DefinitionValidationContext, OwnerId};
use mush_core::read_service::{OwnerReadContext, OwnerReadError, OwnerReadService};
use mush_core::{DefinitionRef, ModError, Registrar};
use mush_roll::commerce::fork_protocol::{ForkMaterial, ForkMaterialRequest, material_descriptor};
use std::collections::BTreeSet;

pub(crate) struct ForkMaterialReader;
fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
pub(crate) fn register(reg: &mut Registrar<'_>, manifest: &DefinitionRef) -> Result<(), ModError> {
    if reg.module() != crate::NS {
        return Err(ModError::msg("fork material is Tacsim-owned"));
    }
    let expected = material_descriptor(manifest)?;
    let definition = reg.definition(expected.clone())?;
    reg.definition_validator(
        DefinitionKind::new("owner_read").map_err(error)?,
        move |context: &DefinitionValidationContext<'_>| {
            let actual = context.definition();
            let reference = actual.reference();
            let mut references = expected.references().to_vec();
            references.sort();
            if reference.key().owner.as_str() == crate::NS
                && reference.key().kind == *expected.kind()
                && reference.key().id == *expected.id()
                && reference.version() == expected.version()
                && actual.payload() == expected.payload()
                && actual.references() == references
                && actual.is_current() == expected.is_current()
            {
                Ok(())
            } else {
                Err("fork source material differs from its closed configuration".into())
            }
        },
    )?;
    reg.owner_read_service(
        definition,
        BTreeSet::from([OwnerId::new("roll").map_err(error)?]),
        BTreeSet::new(),
        ForkMaterialReader,
    )
}
impl OwnerReadService for ForkMaterialReader {
    type State = crate::TacsimStore;
    type Request = ForkMaterialRequest;
    type Output = ForkMaterial;
    fn read(
        &self,
        sources: &crate::TacsimStore,
        ctx: &OwnerReadContext<'_>,
        request: ForkMaterialRequest,
    ) -> Result<ForkMaterial, OwnerReadError> {
        if ctx.requester().as_str() != "roll"
            || ctx.principal() != &(mush_core::action::PrincipalId::Object { id: ctx.actor() })
        {
            return Err(OwnerReadError::Authority);
        }
        // Compact identity in, bounded source bytes out. The builder repeats
        // the full immutable revision/envelope digest check before preparation.
        ForkMaterial::read_exact(request.identity(), sources).map_err(|_| OwnerReadError::Integrity)
    }
}
