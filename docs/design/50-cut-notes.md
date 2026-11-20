# 50. Law and the corridor — cut notes

One section a unit of [doc 50](50-law-and-the-corridor.md) §7. A unit's session replaces its own "Not built." line with
what it built, under these headings: **Survey**, **Built**, **Tests**, **Divergences**, **Not built**.
Each session edits only its own section, so two units' notes never touch the same lines.

The order of work, and which unit waits on which, is the lead's map of 2026-11-19 (in brief):
- **Wave 1, no unit waits:** CR, KC.0a, OP.0.
- **KC:** KC.0a, then KC.0, KC.1, KC.2 (which also waits on CR), then KC.3 and KC.4.
- **LW:** LW.1 waits on KC.2's terms schedule; LW.2 on KC.0; LW.3 on LW.1, LW.2 and CR; LW.4 on
  LW.3; LW.5 on CR and LW.2.
- **OP:** OP.1 on KC.0; OP.2 on OP.1 and KC.0a; OP.3 on OP.2 and CR; OP.4 on OP.3 and KC.2; OP.5 on
  OP.0, OP.4 and LW.1; OP.6 and OP.7 on OP.5; OP.8 on everything.
- **Lane bumps go one at a time:** KC.0, KC.2, KC.4 and OP.1 each change the lane pack.

## CR

The cargo reading, and two live defects beside it from doc 50 §4.3: the picket and the hold's
fill check count a bulk good held as a lot; a seizure can take cargo under a haul's hold and fails
the haul; the 35 km breach records the pilot, not the hull, and writes a ruling.

Not built.

## KC.0a

Not built.

## KC.0

Not built.

## KC.1

Not built.

## KC.2

Not built.

## KC.3

Not built.

## KC.4

Not built.

## LW.1

Not built.

## LW.2

Not built.

## LW.3

Not built.

## LW.4

Not built.

## LW.5

Not built.

## OP.0

The campaign engine's five additions and the seam at the composition root (Q-50-57, ruled at its
default; doc 50a A-2a to A-2h).

**Survey.** Each of the five held in code at `ecb96979`, at the anchors doc 50a gives:
- `CampaignStore::mint_operation` is `pub(crate)`, and only `@op/declare` and `@op/activate`
  declare. The tick's funding check ran on every live state, so a `Declared` operation carrying
  `@op/terms bond=` (no hold until activation) paused "unfunded" on the next fire, and
  `@op/activate` never cleared it (A-2g, live).
- `@op/terms` parsed sides, defenders and the forfeit beneficiary as `#<obj>` only;
  `notice::defenders` skipped every non-object; `render::acts_for` fails closed for an
  organization, so an organization's member could not read the notice either.
- `tick::objective_met` read a `CanonRecorded` objective every fire, met mid-window as the
  declarer's `success`.
- `settle_root` commits one kind, `campaign.operation.resolved`, with the outcome in `detail`
  only, and `WorldEvent::CanonRecorded` carries no detail. Pins: tacsim's 24 edges
  (`domain_arena/composition_edges.rs`), the campaign crate's five (`e7c9_p1`).
- `outcome_forfeits` was `== "forfeit"`: every other outcome released the bond.

**Built.**
- **Declare and activate in one step** (`crates/mush-campaign/src/declare.rs`):
  `pub fn declare_and_activate(ctx: &mut Ctx<'_>, declaration: Declaration) -> Result<OperationId,
  DeclareError>`, with `Declaration { declarer: PrincipalId, objective: ObjectiveSpec,
  legal_targets, stakes, bond: Option<BondTerms { account, amount, forfeit_to:
  Option<(PrincipalId, AccountId)> }>, sides: Vec<SideSpec>, eligible_defenders: AudienceSpec,
  notice_secs: i64, window_secs: i64, visibility: EconomicVisibility }`. It applies the per-issuer
  cap, mints, counts the notice from now, re-runs the terms constructor and the bond-account and
  outcome-stake checks `@op/terms` makes, places the hold first, and writes the operation
  `Activated`; the tick pushes the notice on its next fire. `DeclareError` is `NoCampaign`,
  `IssuerCap`, `Full`, `Window`, `Terms`, `Refused` or `Bond`. A refused call writes nothing but
  the allocator, which is persisted and never reissues an id.
- **The A-2g defect, fixed at its cause:** the tick's funding check skips a `Declared` operation
  (`tick.rs`), and `@op/activate` clears a stored `Unfunded` pause.
- **Organizations as declarer, side and defender.** The public call takes any `PrincipalId`, and
  `@op/terms` takes `org:<id>` beside `#<obj>` for `side`, `defender` and `bond.forfeit`
  (`org:<id>:<account>`). A defender organization's notice goes to its active members, at most
  `MAX_NOTICE_RECIPIENTS` (256) an operation, through the existing seam: an official notice,
  delivered at once (`mush_mail::post_notice`). `render::member_of` admits an active member to the
  notice lens, never to the plan and never to any authority.
- **A close-only objective with its own polarity:** `ObjectiveKind::ProbeAtClose { probe: SmolStr,
  met_favors: Favors }` with `Favors { Declarer, Defenders }`, also as `@op/terms
  <id>/objective.kind=probe:<name>:<declarer|defenders>`. The tick reads it only once the window
  has closed, through `CampaignModule::with_close_probe(Arc<dyn CloseProbe>)`, where `trait
  CloseProbe { fn read(&self, w: &World, probe: &str, operation: OperationId) -> Option<bool>; }`.
  `close_outcome(met, favors)` gives the declarer's word: a met probe that favors the defenders is
  `failure`. `None` is a wait: the operation keeps running past its close, pages once, and asks
  again each fire.
- **The bond is forfeit on the declarer's failure:** `outcome_forfeits` is `forfeit` or
  `failure`, and the bond goes to `bond.forfeit_to` (for a compiled declarer, the defenders'
  treasury).
- **The seam, TacSim's side** (`crates/tacsim/src/operations.rs`): `pub trait OperationFrame:
  Send + Sync { fn declare(&self, ctx: &mut Ctx<'_>, request: &OperationRequest) -> Result<u64,
  String>; fn status(&self, w: &World, operation: u64) -> Option<OperationStatus>; }`.
  `OperationRequest { declarer, summary, probe, met_favors: MetFavors, sides: Vec<(String,
  PrincipalId)>, defenders, legal_targets: Vec<String>, bond: Option<OperationBond>, notice_secs,
  window_secs, public: bool }`. `OperationStatus { phase: OperationPhase
  (Declared|Notice|Running|Settling|Settled), declarer, sides, defenders, legal_targets, window:
  (i64, i64), outcome: Option<OperationOutcome> }`, and `OperationOutcome { DeclarerSucceeded,
  DeclarerFailed, Partial, Abandoned }`. A TacSim arm finds it with
  `tacsim::operations::frame(w) -> Option<Arc<dyn OperationFrame>>` (the slot is
  `OperationFrameSlot`). The other direction is `tacsim::operations::close_probe(w, probe,
  operation) -> Option<bool>` over `CLOSE_PROBES: &[(&str, CloseProbeFn)]`, empty in OP.0.
- **The composition root** (`crates/mush-composition/src/campaign_operations.rs`):
  `CampaignOperationFrame` implements the trait over `declare_and_activate` and the
  `CampaignStore` reads; `TacsimCloseProbe` implements `CloseProbe` over `close_probe`;
  `CampaignLink` composes the campaign module and seats the frame beside it. `server_modules`
  composes `CampaignLink::new(CampaignModule::new()...with_close_probe(TacsimCloseProbe))`. No
  pinned edge moves: tacsim gains no campaign or mail edge, and the campaign no faction edge.

**Tests.** `crates/mush-campaign/tests/op0.rs` (10): the defect (`op0_a2g_…`); the public call as
an organization and its three refusals (`op0_a1_…`); the defender organization's notice and lens
(`op0_a2f_…`); the probe read only at the close, its four answers and the wait (`op0_a2a_…`);
forfeit on failure, release on success and on abandonment (`op0_a2c_…`).
`crates/mush-composition/tests/op0_frame.rs` (4): a fake owner that uses only TacSim's side
declares through the seated frame and reads who won both ways; unlinked, there is no frame; and the
production probe answers nothing. Mutations, each caught: the tick's `Declared` guard removed
(a2g), the probe read before the close (a2a), `failure` releasing (a2c), the organization's members
dropped from the notice (a2f); the polarity table flipped fails to compile (exhaustive match). Two
e7c9 tests have the issuer resolve `=failure`; they pass, since that bond names no beneficiary and
is released, and one message is reworded. All of mush-campaign passes.

**Divergences.**
- The frame reaches TacSim as a **world-state slot** registered beside the campaign module by
  `CampaignLink` (a wrapper carrying the campaign's own name), not as a builder on
  `TacsimRuleset`. The tacsim module is built in three places in `mush-server/src/boot.rs` and in
  tacsim's own helpers; the slot needs none of them and leaves the module roster (`boot_parity`)
  as it was.
- **A close probe is named, and answered from its owner's table.** TacSim names no probe in OP.0;
  OP.4 adds the tank farm's row to `CLOSE_PROBES`.
- **An unanswered probe is a wait, not a pause**, so it needs no `=retry`; staff may still decide
  it with `@op/resolve`.
- **Forfeit needs a named beneficiary.** A failing bond that names none is released, since the
  economy cannot capture a hold made to nobody. The issuer may still resolve their own operation
  `=failure`, conceding the bond to the beneficiary their own terms named; `forfeit` and `abort`
  stay staff's.
- **The canon record is unchanged** (one kind, the outcome in `detail`). Who won reaches TacSim
  through `OperationFrame::status`, the seam the ruling names. A faction clock bound to the kind
  still advances on any outcome (A-2d); Q-50-51's typed calls replace that path.
- An organization side or declarer still gets no plan lens (`acts_for` fails closed for one, as
  before); its members see the notice when it names them as defenders or audience. OP.7's
  `+operation` is the player page.

**Not built.** Any TacSim arm, Isidis, a front or a probe (OP.1 to OP.5). Organization members on
the resolution's canon entities (A-2e; Q-50-51 moves polity clocks by typed calls instead). A
capability for an organization's officers to read or move an operation's plan.

## OP.1

Not built.

## OP.2

Not built.

## OP.3

Not built.

## OP.4

Not built.

## OP.5

Not built.

## OP.6

Not built.

## OP.7

Not built.

## OP.8

Not built.
