//! One closed Capability validator for Roll's explicit active services.
//! Each branch accepts only its own actual retained opaque intent. There is no
//! extensible validator list, generic service authority, or current-rights bypass.
use mush_core::action::{
    ActionError, AuthorityGuard, AuthorityGuardContext, AuthorityGuardKind, AuthorityGuardValidator,
};
use mush_core::{ModError, Registrar};
struct Services;
impl AuthorityGuardValidator for Services {
    fn is_current(
        &self,
        ctx: &AuthorityGuardContext<'_>,
        guard: &AuthorityGuard,
    ) -> Result<bool, ActionError> {
        Ok(super::facts::guard_current(ctx, guard)?
            || super::rating::reconformance::recall::guard_current(ctx, guard)?
            || super::charter_pending::service::guard_current(ctx, guard)?)
    }
}
pub(super) fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.authority_guard_validator(AuthorityGuardKind::Capability, Services)
}
