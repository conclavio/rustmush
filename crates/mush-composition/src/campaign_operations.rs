//! **The operation frame's two links** (doc 50 cut OP.0; Q-50-57 item 4), joined here where both
//! `tacsim` and `mush-campaign` are in reach and neither crate's pinned edges move.
//!
//! - **TacSim to the campaign:** [`CampaignOperationFrame`] implements TacSim's
//!   `tacsim::operations::OperationFrame` over the campaign crate's public API
//!   (`mush_campaign::declare_and_activate` and the `CampaignStore` reads). [`CampaignLink`]
//!   composes the campaign module and seats the frame in world state beside it, so wherever the
//!   campaign is composed TacSim finds the frame through `tacsim::operations::frame`, and nowhere
//!   else.
//! - **The campaign to TacSim:** [`TacsimCloseProbe`] implements `mush_campaign::CloseProbe` over
//!   `tacsim::operations::close_probe`, which `mush_campaign::CampaignModule::with_close_probe`
//!   takes in, so a close-only objective is answered by the TacSim probe it names.

use std::sync::Arc;

use mush_campaign::{
    AudienceSpec, BondTerms, CampaignModule, CampaignStore, CloseProbe, Declaration, Favors,
    ObjectiveKind, ObjectiveSpec, OperationId, OperationState, SideSpec,
};
use mush_core::module::{GameModule, ModError, ModuleBootData, ModuleCtx, Registrar};
use mush_core::{Ctx, World};
use mush_economy::EconomicVisibility;
use smol_str::SmolStr;
use tacsim::operations::{
    MetFavors, OperationFrame, OperationFrameSlot, OperationOutcome, OperationPhase,
    OperationRequest, OperationStatus,
};

/// **TacSim's operation frame, over `mush-campaign`** (this module's head).
#[derive(Clone, Copy, Debug, Default)]
pub struct CampaignOperationFrame;

/// The campaign's declaration for a TacSim request.
#[must_use]
pub fn declaration(request: &OperationRequest) -> Declaration {
    let defenders: Vec<mush_org::PrincipalId> = request.defenders.clone();
    Declaration {
        declarer: request.declarer.clone(),
        objective: ObjectiveSpec {
            kind: ObjectiveKind::ProbeAtClose {
                probe: SmolStr::new(&request.probe),
                met_favors: match request.met_favors {
                    MetFavors::Declarer => Favors::Declarer,
                    MetFavors::Defenders => Favors::Defenders,
                },
            },
            summary: SmolStr::new(&request.summary),
        },
        legal_targets: request.legal_targets.iter().map(SmolStr::new).collect(),
        stakes: Vec::new(),
        bond: request.bond.as_ref().map(|bond| BondTerms {
            account: bond.account,
            amount: bond.amount,
            forfeit_to: bond.forfeit_to.clone(),
        }),
        sides: request
            .sides
            .iter()
            .map(|(name, principal)| SideSpec {
                name: SmolStr::new(name),
                principal: principal.clone(),
            })
            .collect(),
        eligible_defenders: AudienceSpec {
            members: defenders.clone(),
        },
        notice_secs: request.notice_secs,
        window_secs: request.window_secs,
        visibility: if request.public {
            EconomicVisibility::Public
        } else {
            EconomicVisibility::Audience {
                members: defenders,
                announce_issuer: false,
            }
        },
    }
}

/// The outcome TacSim reads from the campaign's recorded word.
#[must_use]
pub fn outcome_of(recorded: &str, abandoned: bool) -> Option<OperationOutcome> {
    if abandoned {
        return Some(OperationOutcome::Abandoned);
    }
    match recorded.trim() {
        "success" => Some(OperationOutcome::DeclarerSucceeded),
        "failure" | "forfeit" => Some(OperationOutcome::DeclarerFailed),
        "partial" => Some(OperationOutcome::Partial),
        "abort" => Some(OperationOutcome::Abandoned),
        _ => None,
    }
}

impl OperationFrame for CampaignOperationFrame {
    fn declare(&self, ctx: &mut Ctx<'_>, request: &OperationRequest) -> Result<u64, String> {
        mush_campaign::declare_and_activate(ctx, declaration(request))
            .map(|id| id.0)
            .map_err(|error| error.to_string())
    }

    fn status(&self, w: &World, operation: u64) -> Option<OperationStatus> {
        let doc = w
            .module_state::<CampaignStore>()?
            .operation(OperationId(operation))?;
        let phase = match doc.runtime.state {
            OperationState::Declared => OperationPhase::Declared,
            OperationState::Activated => OperationPhase::Notice,
            OperationState::Running => OperationPhase::Running,
            OperationState::Resolving { .. } => OperationPhase::Settling,
            OperationState::Resolved | OperationState::Aborted => OperationPhase::Settled,
        };
        let outcome = doc
            .runtime
            .root
            .as_ref()
            .and_then(|root| outcome_of(&root.outcome, doc.runtime.abandoned));
        Some(OperationStatus {
            phase,
            declarer: doc.terms.attacker.clone(),
            sides: doc
                .terms
                .sides
                .iter()
                .map(|side| (side.name.to_string(), side.principal.clone()))
                .collect(),
            defenders: doc.terms.eligible_defenders.members.clone(),
            legal_targets: doc
                .terms
                .legal_targets
                .iter()
                .map(ToString::to_string)
                .collect(),
            window: (doc.terms.window.0.at, doc.terms.window.1.at),
            outcome,
        })
    }
}

/// **The campaign's close probe, over TacSim's** (this module's head).
#[derive(Clone, Copy, Debug, Default)]
pub struct TacsimCloseProbe;

impl CloseProbe for TacsimCloseProbe {
    fn read(&self, w: &World, probe: &str, operation: OperationId) -> Option<bool> {
        tacsim::operations::close_probe(w, probe, operation.0)
    }
}

/// **The campaign module with TacSim's frame seated beside it** (this module's head). It is the
/// campaign module in every respect (its name, its registration, its boot) and registers one more
/// state slot, `tacsim::operations::OperationFrameSlot`, holding [`CampaignOperationFrame`]. The
/// roster is unchanged: there is no second module.
pub struct CampaignLink(CampaignModule);

impl CampaignLink {
    /// Seat the frame beside `module`.
    #[must_use]
    pub fn new(module: CampaignModule) -> CampaignLink {
        CampaignLink(module)
    }
}

impl GameModule for CampaignLink {
    fn name(&self) -> &'static str {
        self.0.name()
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        self.0.register(reg)?;
        reg.state(OperationFrameSlot::new(Arc::new(CampaignOperationFrame)));
        Ok(())
    }

    fn audit_typed_history(&self, ctx: &ModuleCtx<'_>) -> Result<(), ModError> {
        self.0.audit_typed_history(ctx)
    }

    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        data: ModuleBootData,
    ) -> Result<(), ModError> {
        self.0.on_world_loaded(ctx, data)
    }

    fn on_all_loaded(&mut self, ctx: &mut ModuleCtx<'_>) -> Result<(), ModError> {
        self.0.on_all_loaded(ctx)
    }

    fn finalize_owner_composition(&mut self, ctx: &mut ModuleCtx<'_>) -> Result<(), ModError> {
        self.0.finalize_owner_composition(ctx)
    }

    fn boot_stream_requests(
        &self,
        kv: &[(SmolStr, serde_json::Value)],
    ) -> Result<Vec<(SmolStr, u32)>, ModError> {
        self.0.boot_stream_requests(kv)
    }
}
