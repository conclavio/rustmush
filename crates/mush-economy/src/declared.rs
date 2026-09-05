//! Declared cadence clocks (SP1 — D-LBK-7 as decided in 08h §2 Q6).
//!
//! A cadence named in `[economy.cadences]` is **declared**: the economy registers a
//! [`ClockProvider`] for it at composition, under owner `economy` and id
//! `cadence.<name>`, so any consumer can read it through
//! `Engine::clock_now(ClockRef::Registered { owner: economy, id: cadence.<name>,
//! generation })` and get a kernel-built [`mush_core::ClockStamp`] whose `at` is the
//! cadence's `position`. The provider answers for the cadence's `generation` alone —
//! never for `revision`, which is a staleness stamp (08h Q6) — and refuses any other
//! generation as stale, exactly as K4a's world timeline does.
//!
//! A cadence created at runtime by `@econ/cadence create` is **not** declared: it
//! stays a legacy [`crate::EconomicClock::Cadence`] adapter and nothing registers
//! it. K's registry freezes at composition, so a runtime clock cannot be registered
//! after the fact by design (K answer 4: exact per-ID registration is right for
//! declared cadences; dynamic clocks are a separate owner-scoped provider).
//!
//! At boot the economy **adopts** an existing document with a declared name — stamps
//! it [`CADENCE_SCHEMA_DECLARED`] and leaves every parameter as persisted, because
//! configuration never silently rewrites a live clock; a period or source that
//! disagrees with the declaration is logged and paged **once** through the ladder
//! (subject [`EscalationSubject::DeclaredCadence`]) so a human decides which side is
//! right. A missing one is **created** as `@econ/cadence create` would, under the
//! boot's system authority, stamped declared.
//!
//! `EconomicClock` and the sweep are untouched: a declared cadence is still advanced
//! by the sweep from its own source and still resolves through the snapshot for
//! every economic schedule on it. The registration is additive.

use mush_core::module::{ModError, Registrar};
use mush_core::{
    ClockProvider, ClockProviderContext, ClockProviderFailure, ClockProviderKind, Ctx,
};
use smol_str::SmolStr;

use crate::api;
use crate::clock::{CADENCE_SCHEMA_DECLARED, CadenceClock};
use crate::config::{
    DeclaredCadence, DeclaredCadenceClock, EconomyConfig, declared_cadence_clock_id,
};
use crate::escalation::{EscalationKind, EscalationSubject};
use crate::module::EconomyStore;
use crate::snapshot::EconomySnapshot;

/// The composition-frozen provider for one declared cadence.
///
/// It carries the cadence **name**, not an id: the id is minted at boot (or was
/// minted by whoever created the clock first), and composition runs before any
/// document is read. The name is matched the way the cadence table matches names.
#[derive(Clone, Debug)]
pub struct DeclaredCadenceProvider {
    name: SmolStr,
}

impl DeclaredCadenceProvider {
    /// The provider for the declared cadence `name`.
    #[must_use]
    pub fn new(name: SmolStr) -> DeclaredCadenceProvider {
        DeclaredCadenceProvider { name }
    }

    /// The cadence name this provider answers for.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// The cadence record named `name`, matched case-insensitively (the cadence table's
/// own duplicate rule, `create_cadence`).
#[must_use]
pub fn cadence_by_name<'a>(snapshot: &'a EconomySnapshot, name: &str) -> Option<&'a CadenceClock> {
    snapshot
        .cadences()
        .find(|c| c.name.eq_ignore_ascii_case(name))
}

impl ClockProvider for DeclaredCadenceProvider {
    fn now(
        &self,
        context: &ClockProviderContext<'_>,
        generation: u64,
    ) -> Result<i64, ClockProviderFailure> {
        // The economy is not loaded, or the declared clock does not exist (yet) —
        // creation failed at boot, or the read is happening before boot. Unavailable,
        // never a fabricated zero: a missing clock must not make a deadline pass.
        let store = context
            .module_state::<EconomyStore>()
            .ok_or(ClockProviderFailure::Unavailable)?;
        let c = cadence_by_name(store.snapshot(), &self.name)
            .ok_or(ClockProviderFailure::Unavailable)?;
        let current = u64::from(c.generation);
        if generation != current {
            return Err(ClockProviderFailure::StaleGeneration {
                requested: generation,
                current,
            });
        }
        i64::try_from(c.position).map_err(|_| ClockProviderFailure::Overflow)
    }
}

/// Register one [`DeclaredCadenceProvider`] per `[economy.cadences]` stanza
/// (`EconomyModule::register`, composition time).
///
/// Validates the whole list first, so a bad stanza fails composition with the
/// reason and no partial registration.
///
/// # Errors
///
/// The list is not acceptable ([`EconomyConfig::check_declared_cadences`]), or the
/// kernel refused a registration (a duplicate, the owner/global provider limits).
pub(crate) fn register(reg: &mut Registrar<'_>, config: &EconomyConfig) -> Result<(), ModError> {
    config
        .check_declared_cadences()
        .map_err(|e| ModError::msg(format!("economy: {e}")))?;
    for name in config.cadences.keys() {
        let id = declared_cadence_clock_id(name)
            .map_err(|e| ModError::msg(format!("economy: [economy.cadences.{name:?}]: {e}")))?;
        reg.clock_provider(
            ClockProviderKind::Registered,
            id,
            DeclaredCadenceProvider::new(name.clone()),
        )?;
    }
    Ok(())
}

/// Adopt or create every declared cadence (`EconomyModule::on_world_loaded`, after
/// the store is installed and before any layered module boots — housing and the
/// WoD layer look their campaign clock up by name, and find the declared one).
pub(crate) fn reconcile(ctx: &mut Ctx<'_>, config: &EconomyConfig) {
    for (name, decl) in &config.cadences {
        let existing = ctx
            .w
            .module_state::<EconomyStore>()
            .and_then(|s| cadence_by_name(s.snapshot(), name).cloned());
        match existing {
            Some(c) => adopt(ctx, name, decl, c),
            None => create(ctx, name, decl),
        }
    }
}

fn adopt(ctx: &mut Ctx<'_>, name: &str, decl: &DeclaredCadence, mut c: CadenceClock) {
    let id = c.id;
    if c.schema != CADENCE_SCHEMA_DECLARED {
        tracing::info!(
            cadence = %name, id = id.0, from_schema = c.schema,
            "economy: adopted an existing cadence clock named in [economy.cadences] \
             (parameters left as persisted)"
        );
        c.schema = CADENCE_SCHEMA_DECLARED;
        api::put_cadence(ctx, c.clone());
    }
    let persisted_clock = DeclaredCadenceClock::of_source(c.source);
    let clock_matches = persisted_clock == Some(decl.clock);
    let period_matches = c.period == decl.period_secs;
    if c.catch_up_cap != decl.catch_up {
        tracing::warn!(
            cadence = %name, id = id.0, persisted = c.catch_up_cap, declared = decl.catch_up,
            "economy: [economy.cadences] declares a different catch-up than the live clock \
             carries; the live value stands"
        );
    }
    if clock_matches && period_matches {
        return;
    }
    let persisted_word = persisted_clock.map_or("manual", DeclaredCadenceClock::word);
    let context = format!(
        "economy.cadence.declared:{name}: the configuration declares clock = {:?}, period_secs = \
         {}, but cadence #{} is persisted as clock = {:?}, period = {}. The live clock was adopted \
         as declared and left exactly as it is: configuration never rewrites a running clock. \
         Decide which side is right.",
        decl.clock.word(),
        decl.period_secs,
        id.0,
        persisted_word,
        c.period,
    );
    tracing::warn!(cadence = %name, id = id.0, %context, "economy: declared cadence mismatch");
    let now = ctx.w.now();
    let recommended = vec![
        format!("@econ/cadence show #{}", id.0),
        format!(
            "edit [economy.cadences.{name:?}] to clock = {:?}, period_secs = {} (the live clock's \
             values), then restart",
            persisted_word, c.period
        ),
    ];
    let _ = crate::sweep::page(
        ctx,
        EscalationKind::BootReconciliation,
        EscalationSubject::DeclaredCadence(id),
        None,
        context,
        recommended,
        now,
    );
}

fn create(ctx: &mut Ctx<'_>, name: &str, decl: &DeclaredCadence) {
    match api::create_cadence_with_schema(
        ctx,
        name,
        decl.clock.source(),
        decl.period_secs,
        Some(decl.catch_up),
        CADENCE_SCHEMA_DECLARED,
    ) {
        Ok(id) => tracing::info!(
            cadence = %name, id = id.0, clock = decl.clock.word(), period_secs = decl.period_secs,
            "economy: created the cadence clock declared in [economy.cadences]"
        ),
        // A world already carrying `MAX_CADENCES` runtime clocks, or an allocator at
        // its ceiling. The provider is registered and answers `Unavailable` until a
        // clock of that name exists; nothing on it can fall due meanwhile (a missing
        // cadence never makes a deadline pass), so the world boots and the log says
        // why the declared clock is not there.
        Err(error) => tracing::error!(
            cadence = %name, %error,
            "economy: could not create the cadence clock declared in [economy.cadences]; its \
             registered provider answers unavailable until one exists"
        ),
    }
}
