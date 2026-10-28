//! M8c F11: `org_actor` over a small world with the real org module: a live
//! grant acts; an actor GOING, an organisation or scope not Active, a scope
//! of another organisation, a revoked grant, a grant limited below the
//! amount and a missing Roll all refuse; `acting_as` re-stamps; staff never
//! substitutes for the grant.
use super::*;
use mush_core::{
    ArgShape, CmdError, CommandSpec, Ctx, Engine, GOD, GameModule, Inbound, ModError,
    ModuleBootData, ModuleCtx, ObjKind, Outbox, Registrar, Registry,
};
use mush_economy::Holder;
use mush_org::{
    CapabilityGrant, GrantLimit, OrgAction, OrgModule, OrgMutationOp, OrgMutationPlan,
    ScopeSelector,
};
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
type Build = Box<dyn FnOnce(&mut Ctx<'_>) -> OrgMutationPlan + Send>;
type Pending = Arc<Mutex<Option<Build>>>;

struct TestOrg {
    inner: OrgModule,
    pending: Pending,
}
impl GameModule for TestOrg {
    fn name(&self) -> &'static str {
        "org"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> std::result::Result<(), ModError> {
        self.inner.register(reg)?;
        let pending = self.pending.clone();
        reg.command(
            CommandSpec {
                name: "@holder-org-fixture".into(),
                shape: ArgShape::Raw,
                ..Default::default()
            },
            Arc::new(move |ctx: &mut Ctx<'_>| {
                if ctx.actor != GOD {
                    return Err(CmdError::msg("fixture operator"));
                }
                let build = pending
                    .lock()
                    .map_err(|_| CmdError::msg("fixture lock"))?
                    .take()
                    .ok_or_else(|| CmdError::msg("fixture plan"))?;
                let plan = build(ctx);
                mush_org::submit(
                    ctx,
                    &plan,
                    OrgAction {
                        action: "holder-fixture.grant".into(),
                        org: None,
                        detail: json!({"fixture":true}),
                    },
                )
                .map_err(|_| CmdError::msg("actual Org mutation refused"))
            }),
        )?;
        Ok(())
    }
    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        data: ModuleBootData,
    ) -> std::result::Result<(), ModError> {
        self.inner.on_world_loaded(ctx, data)
    }
}
/// The Roll's slot only: `org_actor` asks that the Roll's store is owned by
/// the Roll, nothing more.
struct SlotRoll;
impl GameModule for SlotRoll {
    fn name(&self) -> &'static str {
        crate::NS
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> std::result::Result<(), ModError> {
        reg.state(crate::RollStore::default());
        Ok(())
    }
}

fn command(e: &mut Engine, line: &str) -> Result {
    let mut out = Outbox::new();
    e.step(
        Inbound::Cmd {
            conn: 1,
            actor: GOD,
            line: line.into(),
        },
        &mut out,
        &mut mush_core::Effects::new(),
    );
    if out.events().iter().any(|(_, e)| {
        matches!(e, mush_proto::OutputEvent::Sys(text)
            if text.spans().iter().any(|s| s.tag.as_deref() == Some("error")))
    }) {
        return Err(format!("org command refused: {line}").into());
    }
    Ok(())
}

struct Rig {
    engine: Box<Engine>,
    pending: Pending,
    member: ObjId,
    other: ObjId,
    guild: OrgId,
    rival: OrgId,
}
impl Rig {
    fn new(roll: bool) -> Result<Self> {
        let pending = Pending::default();
        let mut modules: Vec<Box<dyn GameModule>> = vec![Box::new(TestOrg {
            inner: OrgModule::new(),
            pending: pending.clone(),
        })];
        if roll {
            modules.push(Box::new(SlotRoll));
        }
        let mut engine =
            Engine::with_modules(World::new_minimal(), Registry::new(), modules).map(Box::new)?;
        engine.world.set_now(100);
        let member = engine.world.create(ObjKind::Player, "Member", GOD, None)?;
        let other = engine.world.create(ObjKind::Player, "Other", GOD, None)?;
        engine.modules_loaded(
            HashMap::new(),
            &mut Outbox::new(),
            &mut mush_core::Effects::new(),
        )?;
        command(&mut engine, "@org/create Guild")?;
        command(&mut engine, "@org/create Rival")?;
        let state = engine
            .world
            .module_state::<OrgStore>()
            .ok_or("org")?
            .state();
        let named = |name: &str| {
            state
                .orgs()
                .find(|o| o.name.as_str() == name)
                .map(|o| o.id)
                .ok_or("actual org")
        };
        let (guild, rival) = (named("Guild")?, named("Rival")?);
        Ok(Self {
            engine,
            pending,
            member,
            other,
            guild,
            rival,
        })
    }
    fn world(&self) -> &World {
        &self.engine.world
    }
    fn scope(&mut self, org: OrgId, org_name: &str, name: &str) -> Result<OrgScopeId> {
        command(
            &mut self.engine,
            &format!("@org/scope create {org_name}={name}"),
        )?;
        let state = self
            .engine
            .world
            .module_state::<OrgStore>()
            .ok_or("org")?
            .state();
        Ok(state
            .scopes_in(org)
            .find(|s| s.name.as_str() == name)
            .ok_or("actual scope")?
            .id)
    }
    fn grant(&mut self, who: ObjId, org: &str, scope: Option<OrgScopeId>) -> Result {
        let scope = scope.map_or(String::new(), |s| format!(" scope={}", s.0));
        command(
            &mut self.engine,
            &format!("@org/grant #{}={org} {HOLDER_ORG_CAPABILITY}{scope}", who.0),
        )
    }
    fn mutate(&mut self, build: Build) -> Result {
        *self.pending.lock().map_err(|_| "fixture lock")? = Some(build);
        command(&mut self.engine, "@holder-org-fixture")
    }
}

#[test]
fn a_live_grant_acts_and_staff_never_substitutes() -> Result {
    let mut r = Rig::new(true)?;
    let (guild, member) = (r.guild, r.member);
    check_eq!(
        acts_for(r.world(), guild, member),
        Err(OrgActorError::NotForOrg)
    );
    r.grant(member, "Guild", None)?;
    check_eq!(acts_for(r.world(), guild, member), Ok(()));
    check_eq!(acts_anywhere(r.world(), guild, member), Ok(()));
    for nobody in [r.other, GOD, ObjId(0), ObjId(9_999)] {
        check_eq!(
            acts_for(r.world(), guild, nobody),
            Err(OrgActorError::NotForOrg),
            "{nobody:?}"
        );
    }
    // Organisation 0 is an organisation (F8): the first one minted.
    check_eq!(guild, OrgId(0));
    // The grant is the Guild's, never the Rival's.
    check_eq!(
        acts_for(r.world(), r.rival, member),
        Err(OrgActorError::NotForOrg)
    );
    Ok(())
}

#[test]
fn a_going_actor_does_not_act() -> Result {
    let mut r = Rig::new(true)?;
    let (guild, member) = (r.guild, r.member);
    r.grant(member, "Guild", None)?;
    r.engine
        .world
        .get_mut(member)
        .ok_or("member")?
        .flags
        .insert(Flags::GOING);
    check_eq!(
        acts_for(r.world(), guild, member),
        Err(OrgActorError::NotForOrg)
    );
    Ok(())
}

#[test]
fn an_inactive_organisation_does_not_act() -> Result {
    let mut r = Rig::new(true)?;
    let (guild, member) = (r.guild, r.member);
    r.grant(member, "Guild", None)?;
    command(&mut r.engine, "@org/status Guild=suspended")?;
    check_eq!(
        acts_for(r.world(), guild, member),
        Err(OrgActorError::NotForOrg)
    );
    check_eq!(
        acts_anywhere(r.world(), guild, member),
        Err(OrgActorError::NotForOrg)
    );
    Ok(())
}

#[test]
fn an_inactive_scope_and_another_organisations_scope_do_not_act() -> Result {
    let mut r = Rig::new(true)?;
    let (guild, rival, member) = (r.guild, r.rival, r.member);
    let wing = r.scope(guild, "Guild", "wing")?;
    let theirs = r.scope(rival, "Rival", "theirs")?;
    r.grant(member, "Guild", Some(wing))?;
    check_eq!(org_actor(r.world(), guild, wing, member), Ok(()));
    // A grant below root acts on its scope only: not the root (`acts_for`,
    // the paid family's rule), but on some Active scope (`acts_anywhere`,
    // the title-bearing pre-gate).
    check_eq!(
        acts_for(r.world(), guild, member),
        Err(OrgActorError::NotForOrg)
    );
    check_eq!(acts_anywhere(r.world(), guild, member), Ok(()));
    check_eq!(scoped_actor(r.world(), guild, Some(wing), member), Ok(()));
    check_eq!(
        org_actor(r.world(), guild, theirs, member),
        Err(OrgActorError::NotForOrg),
        "a scope of another organisation"
    );
    command(
        &mut r.engine,
        &format!("@org/scope retire Guild={}", wing.0),
    )?;
    check_eq!(
        org_actor(r.world(), guild, wing, member),
        Err(OrgActorError::NotForOrg),
        "a retired scope"
    );
    check_eq!(
        acts_anywhere(r.world(), guild, member),
        Err(OrgActorError::NotForOrg)
    );
    Ok(())
}

#[test]
fn a_revoked_grant_stops_the_next_act() -> Result {
    let mut r = Rig::new(true)?;
    let (guild, member) = (r.guild, r.member);
    r.grant(member, "Guild", None)?;
    check_eq!(acts_for(r.world(), guild, member), Ok(()));
    let grant = r
        .world()
        .module_state::<OrgStore>()
        .ok_or("org")?
        .state()
        .grants_in(guild)
        .find(|g| g.grantee == member)
        .ok_or("actual grant")?
        .id;
    r.mutate(Box::new(move |_| {
        OrgMutationPlan::of(vec![OrgMutationOp::RevokeGrant { grant }])
    }))?;
    check_eq!(
        acts_for(r.world(), guild, member),
        Err(OrgActorError::NotForOrg)
    );
    check!(
        acting_as(
            r.world(),
            Holder::Organization {
                org: guild,
                acting: member
            },
            member
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn a_limited_grant_limits_the_amount() -> Result {
    let mut r = Rig::new(true)?;
    let (guild, member) = (r.guild, r.member);
    let root = root_scope(r.world(), guild).ok_or("root scope")?;
    let capability = CapabilityId::new(HOLDER_ORG_CAPABILITY).ok_or("capability")?;
    r.mutate(Box::new(move |ctx| {
        // A fresh id above the allocator's range, read without writing the
        // org module's slot (the state-slot gate).
        let id = mush_org::GrantId(
            1_000_000
                + ctx.w.module_state::<OrgStore>().map_or(0, |s| {
                    s.state()
                        .orgs()
                        .flat_map(|o| s.state().grants_in(o.id))
                        .map(|g| g.id.0)
                        .max()
                        .unwrap_or(0)
                }),
        );
        OrgMutationPlan::of(vec![OrgMutationOp::PutGrant {
            grant: CapabilityGrant {
                id,
                org: guild,
                grantee: member,
                capability,
                scope: ScopeSelector::SubtreeLive(root),
                limit: Some(GrantLimit {
                    max_amount: Some(100),
                }),
                delegable: false,
                valid_from: None,
                valid_until: None,
                revision: 1,
            },
        }])
    }))?;
    check_eq!(acts_for_amount(r.world(), guild, member, Some(100)), Ok(()));
    check_eq!(
        acts_for_amount(r.world(), guild, member, Some(101)),
        Err(OrgActorError::NotForOrg),
        "over the limit"
    );
    check_eq!(
        acts_for(r.world(), guild, member),
        Err(OrgActorError::NotForOrg),
        "a limited grant without an amount fails closed"
    );
    Ok(())
}

#[test]
fn acting_as_re_stamps_the_acting_character() -> Result {
    let mut r = Rig::new(true)?;
    let (guild, member, other) = (r.guild, r.member, r.other);
    r.grant(member, "Guild", None)?;
    let held = Holder::Organization {
        org: guild,
        acting: other,
    };
    check_eq!(
        acting_as(r.world(), held, member),
        Ok(Holder::Organization {
            org: guild,
            acting: member
        })
    );
    check_eq!(
        acting_as(r.world(), held, other),
        Err(OrgActorError::NotForOrg)
    );
    check_eq!(
        acting_as(r.world(), Holder::Character(member), member),
        Ok(Holder::Character(member))
    );
    check_eq!(
        acting_as(r.world(), Holder::Character(other), member),
        Err(OrgActorError::NotForOrg)
    );
    Ok(())
}

#[test]
fn without_the_roll_the_check_is_unavailable() -> Result {
    let mut r = Rig::new(false)?;
    let (guild, member) = (r.guild, r.member);
    r.grant(member, "Guild", None)?;
    check_eq!(
        acts_for(r.world(), guild, member),
        Err(OrgActorError::Unavailable)
    );
    Ok(())
}
