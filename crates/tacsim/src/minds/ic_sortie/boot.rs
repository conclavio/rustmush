use super::{
    records::{HEAD, Head, PREFIX, Record},
    registration::Registration,
    state::IcSorties,
};
use mush_core::{ModError, ModuleBootData, ModuleCtx, ModuleOp, ModuleWrite};
fn bad(_: impl std::fmt::Display) -> ModError {
    ModError::msg("IC execution history is incomplete or unavailable")
}

/// IC documents are read before the legacy loader can skip or heal anything.
/// No closure is inferred from a reboot, absence of a host, or a current base.
pub(crate) fn load(
    ctx: &mut ModuleCtx<'_>,
    data: &ModuleBootData,
    registration: Option<&Registration>,
) -> Result<(), ModError> {
    let mut head = None;
    let mut rows = Vec::new();
    for (key, value) in &data.kv {
        if key.as_str() == HEAD {
            if head.is_some() {
                return Err(bad("duplicate head"));
            }
            let parsed: Head = serde_json::from_value(value.clone()).map_err(|e| bad(e))?;
            parsed.validate().map_err(|e| bad(e))?;
            if serde_json::to_value(&parsed).map_err(|e| bad(e))? != *value {
                return Err(bad("head encoding"));
            }
            head = Some(parsed);
        } else if key.starts_with(PREFIX) {
            if rows.len() >= super::protocol::MAX_HISTORY {
                return Err(bad("history bound"));
            }
            let row = Record::decode(value).map_err(|e| bad(e))?;
            if key.as_str() != row.key() {
                return Err(bad("record key"));
            }
            rows.push(row);
        }
    }
    let Some(registration) = registration else {
        if head.is_some() || !rows.is_empty() {
            return Err(bad("disabled with retained IC"));
        }
        return Ok(());
    };
    if head.is_none() && !rows.is_empty() {
        return Err(bad("missing head"));
    }
    let fresh = head.is_none();
    let head = head.unwrap_or(Head {
        format: 1,
        next_transition: 1,
    });
    let state = IcSorties::reconstruct(head.clone(), rows).map_err(|e| bad(e))?;
    registration.loaded(ctx)?;
    *ctx.w
        .module_state_mut::<IcSorties>()
        .ok_or_else(|| bad("state"))? = state;
    if fresh {
        ctx.persist(ModuleOp {
            ns: crate::NS.into(),
            op: ModuleWrite::JsonSet {
                key: HEAD.into(),
                value: serde_json::to_value(head).map_err(|e| bad(e))?,
            },
        });
    }
    Ok(())
}
