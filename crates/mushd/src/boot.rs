//! The all-in-one composition root: everything `mushd` builds before it binds
//! a listener.
//!
//! Split out of the binary's `serve` (WS04-S0) so the composition the deployed
//! all-in-one runs is the composition the integration suite can boot and assert
//! on — every `crates/mushd/tests` case rides this exact roster.
//!
//! What it now *is*, though, is three calls. WS04-S0 closed four omissions this
//! root had drifted into against the forked server half's `build_engine` by
//! transcribing the missing steps into it, which left the tree carrying the
//! world boot twice: two module rosters, two composition orders, two sets of
//! post-composition state upgrades. Two transcriptions of one boot cannot
//! disagree loudly — they disagree by omission, which is exactly the failure
//! WS04-S0 was fixing. So WS04-S1 moved the store open into
//! [`mush_server::boot::open_store`], and WS04-S3 moved everything after it into
//! [`mush_server::boot::build_engine`] and deleted the transcription. The
//! all-in-one and the split half now build the same world through the same
//! code, and the only thing left here is the ORDER the all-in-one needs:
//! quarantine barrier, store, engine.
//!
//! The barrier stays first for the reason it always was: the pre-opened spool
//! owns the single-deployment lock, and any valid active records have to be
//! compared through the store's read-only connection before migrations, account
//! import, world seeding or module boot can write.

use mush_config::config::Config;
use mush_server::boot::{BootedEngine, OpenedStore};

use crate::quarantine::QuarantineSpool;

/// Boot the all-in-one world: the K0 quarantine barrier, then the shared root's
/// store open and engine build — everything before a listener exists.
///
/// The caller keeps the [`QuarantineSpool`] (it moves on into `ServerDeps`), so
/// it is borrowed here rather than consumed; `build_engine` borrows it too, as
/// the evidence that this cannot be called on a path that never opened one.
///
/// # Errors
///
/// Any boot step that refuses: a failed quarantine barrier, an unreachable or
/// non-durable store, unprovisionable key rings, a rejected config stanza, a
/// module registration clash, or a module boot pass that fails.
pub async fn boot_all_in_one(
    config: &Config,
    quarantine_spool: &mut QuarantineSpool,
) -> Result<(OpenedStore, BootedEngine), String> {
    mush_server::prepare_quarantine_for_writable_boot(config, quarantine_spool, None).await?;
    let opened = mush_server::boot::open_store(config).await?;
    let booted = mush_server::boot::build_engine(config, &opened.store, quarantine_spool).await?;
    Ok((opened, booted))
}
